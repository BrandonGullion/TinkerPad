// `table` dumps: a hand-rolled grid.
//
// Up to LAZY_ROWS rows render at once. Bigger tables are virtualised in
// chunks: each chunk is a <tbody> placeholder of the right height until it
// comes near the visible area, and goes back to a placeholder when it moves
// far away (unless something inside it is open or focused).

import type { Json, RichNode, RichValue } from "../protocol";
import { csvField } from "./copy";
import { asRichValue, el, isContainer, jsonSummary, plural, primitive, richSummary } from "./json";
import { renderTree } from "./tree";

type TableNode = Extract<RichNode, { kind: "table" }>;

/** Tables with more rows than this are virtualised. */
export const LAZY_ROWS = 2000;
/** Rows per virtualised chunk. */
export const CHUNK_ROWS = 200;
/** Strings longer than this are clipped with an ellipsis. */
const CLIP_CHARS = 48;
/** Placeholder height per row before any row has been measured. */
const ESTIMATED_ROW_PX = 24;

export interface TableContext {
  /** The scrolling ancestor, which virtualised chunks watch. */
  scrollRoot: HTMLElement | null;
  /** Renders a nested dump value found in a cell. */
  renderNested: (value: RichValue) => HTMLElement;
}

export interface RenderedTable {
  element: HTMLElement;
  /** The table as CSV, in the order currently shown. */
  toCsv: () => string;
}

const disposers = new WeakMap<Element, () => void>();

/** Stop the observers of any virtualised tables inside `root`. */
export function disposeTables(root: Element): void {
  const own = disposers.get(root);
  if (own) own();
  for (const table of root.querySelectorAll(".dump-table")) disposers.get(table)?.();
}

export function renderTable(node: TableNode, ctx: TableContext): RenderedTable {
  const { columns, rows } = node;
  const width = rows.reduce((w, r) => Math.max(w, r.length), columns.length);

  const element = el("div", "dump-table-block");
  const scroller = el("div", "dump-table-wrap");
  const table = el("table", "dump-table");
  const head = el("thead");
  const headRow = el("tr");
  const headers: HTMLTableCellElement[] = [];
  for (let c = 0; c < width; c++) {
    const column = columns[c];
    const th = el("th");
    th.dataset.nav = "";
    th.tabIndex = -1;
    th.title = "Sort";
    th.setAttribute("aria-sort", "none");
    th.append(el("span", "col-name", column?.name ?? ""));
    if (column?.type) th.append(el("span", "dump-type", column.type));
    th.append(el("span", "sort-mark"));
    th.addEventListener("click", () => sortBy(c));
    headers.push(th);
    headRow.append(th);
  }
  head.append(headRow);
  table.append(head);
  scroller.append(table);

  const foot = el("div", "dump-table-foot", plural(rows.length, "row"));
  if (node.truncated) foot.append(el("span", "dump-truncated", " · truncated by the dump library"));
  element.append(scroller, foot);

  let order = rows.map((_, i) => i);
  let sort: { col: number; dir: 1 | -1 } | null = null;
  let observer: IntersectionObserver | null = null;
  let rowHeight = ESTIMATED_ROW_PX;

  const renderRow = (index: number): HTMLTableRowElement => {
    const tr = el("tr");
    const row = rows[index]!;
    for (let c = 0; c < width; c++) tr.append(cell(row[c], ctx));
    return tr;
  };

  const renderRows = (start: number, end: number): DocumentFragment => {
    const fragment = document.createDocumentFragment();
    for (let i = start; i < end; i++) fragment.append(renderRow(order[i]!));
    return fragment;
  };

  interface Chunk {
    body: HTMLTableSectionElement;
    start: number;
    end: number;
    filled: boolean;
    height: number;
  }
  const chunks = new Map<Element, Chunk>();

  const placeholder = (chunk: Chunk) => {
    const tr = el("tr", "dump-placeholder");
    const td = el("td");
    td.colSpan = Math.max(width, 1);
    td.style.height = `${chunk.height}px`;
    tr.append(td);
    chunk.body.replaceChildren(tr);
    chunk.filled = false;
  };

  const materialize = (chunk: Chunk) => {
    if (chunk.filled) return;
    chunk.body.replaceChildren(renderRows(chunk.start, chunk.end));
    chunk.filled = true;
  };

  /** Size placeholders from the first real chunk once it's laid out. */
  let measured = false;
  const measure = () => {
    const first = [...chunks.values()][0];
    const height = first?.filled ? first.body.getBoundingClientRect().height : 0;
    if (!first || height === 0) return;
    measured = true;
    rowHeight = height / (first.end - first.start);
    for (const chunk of chunks.values()) {
      if (chunk.filled) continue;
      chunk.height = (chunk.end - chunk.start) * rowHeight;
      placeholder(chunk);
    }
  };

  const release = (chunk: Chunk) => {
    if (!chunk.filled) return;
    if (chunk.body.contains(document.activeElement)) return;
    if (chunk.body.querySelector(".dump-cell-open, td.expanded")) return;
    const height = chunk.body.getBoundingClientRect().height;
    if (height === 0) return; // not laid out (hidden); keep it
    chunk.height = height;
    placeholder(chunk);
  };

  const fill = () => {
    observer?.disconnect();
    observer = null;
    chunks.clear();
    for (const body of table.querySelectorAll("tbody")) body.remove();

    if (rows.length <= LAZY_ROWS) {
      const body = el("tbody");
      body.append(renderRows(0, rows.length));
      table.append(body);
      return;
    }
    element.classList.add("virtual");
    observer = new IntersectionObserver(
      (entries) => {
        if (!table.isConnected) return;
        if (!measured) measure();
        for (const entry of entries) {
          const chunk = chunks.get(entry.target);
          if (!chunk) continue;
          if (entry.isIntersecting) materialize(chunk);
          else release(chunk);
        }
      },
      { root: ctx.scrollRoot, rootMargin: "1500px 0px" },
    );
    for (let start = 0; start < rows.length; start += CHUNK_ROWS) {
      const end = Math.min(rows.length, start + CHUNK_ROWS);
      const body = el("tbody", "dump-chunk");
      const chunk: Chunk = { body, start, end, filled: false, height: (end - start) * rowHeight };
      chunks.set(body, chunk);
      if (start === 0) {
        // The first chunk is almost always visible; don't wait a frame for it.
        body.append(renderRows(start, end));
        chunk.filled = true;
      } else {
        placeholder(chunk);
      }
      table.append(body);
      observer.observe(body);
    }
  };

  const sortBy = (col: number) => {
    if (sort?.col === col) sort = sort.dir === 1 ? { col, dir: -1 } : null;
    else sort = { col, dir: 1 };
    order = rows.map((_, i) => i);
    if (sort) {
      const { dir } = sort;
      const keys = rows.map((r) => sortKey(r[col]));
      // Array.prototype.sort is stable, so equal keys keep the dump's order.
      order.sort((a, b) => compareKeys(keys[a]!, keys[b]!, dir));
    }
    headers.forEach((th, i) => {
      const state = sort?.col === i ? (sort.dir === 1 ? "ascending" : "descending") : "none";
      th.setAttribute("aria-sort", state);
      th.querySelector(".sort-mark")!.textContent = state === "ascending" ? " ▲" : state === "descending" ? " ▼" : "";
    });
    fill();
  };

  fill();
  disposers.set(table, () => observer?.disconnect());

  const toCsv = () => {
    const lines = [headers.map((_, c) => csvField(columns[c]?.name ?? "")).join(",")];
    for (const i of order) {
      const row = rows[i]!;
      const fields: string[] = [];
      for (let c = 0; c < width; c++) fields.push(csvField(csvText(row[c])));
      lines.push(fields.join(","));
    }
    return `${lines.join("\n")}\n`;
  };

  return { element, toCsv };
}

function cell(x: Json | undefined, ctx: TableContext): HTMLTableCellElement {
  const td = el("td");
  if (x === undefined) {
    td.className = "dump-missing";
    return td;
  }
  if (typeof x === "number") td.className = "num";
  if (isContainer(x)) {
    const rich = asRichValue(x);
    td.append(
      rich
        ? expander(richSummary(rich), () => ctx.renderNested(rich))
        : expander(jsonSummary(x), () => renderTree(x, { openDepth: 1 })),
    );
  } else if (typeof x === "string" && x.length > CLIP_CHARS) {
    const text = el("span", "dump-clip", x);
    text.title = x.length > 2000 ? `${x.slice(0, 2000)}…` : x;
    text.dataset.nav = "";
    text.tabIndex = -1;
    text.setAttribute("aria-expanded", "false");
    text.addEventListener("click", () => {
      const open = td.classList.toggle("expanded");
      text.setAttribute("aria-expanded", String(open));
    });
    td.append(text);
  } else {
    td.append(primitive(x));
  }
  return td;
}

/** A collapsed summary that renders its content the first time it opens. */
function expander(label: string, render: () => HTMLElement): HTMLElement {
  const box = el("div", "dump-cell");
  const toggle = el("span", "dump-toggle", `▸ ${label}`);
  toggle.dataset.nav = "";
  toggle.tabIndex = -1;
  toggle.setAttribute("role", "button");
  toggle.setAttribute("aria-expanded", "false");
  let content: HTMLElement | null = null;
  toggle.addEventListener("click", (e) => {
    e.stopPropagation();
    const open = toggle.getAttribute("aria-expanded") !== "true";
    if (open && !content) {
      content = el("div", "dump-cell-content");
      content.append(render());
      box.append(content);
    }
    if (content) content.hidden = !open;
    toggle.setAttribute("aria-expanded", String(open));
    toggle.textContent = `${open ? "▾" : "▸"} ${label}`;
    box.classList.toggle("dump-cell-open", open);
  });
  box.append(toggle);
  return box;
}

interface SortKey {
  /** Numbers, then booleans, then strings, then structures; missing last. */
  rank: number;
  value: number | string;
}

const MISSING_RANK = 9;
const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

export function sortKey(x: Json | undefined): SortKey {
  if (x === undefined || x === null) return { rank: MISSING_RANK, value: 0 };
  if (typeof x === "number") return { rank: 0, value: x };
  if (typeof x === "boolean") return { rank: 1, value: x ? 1 : 0 };
  if (typeof x === "string") return { rank: 2, value: x };
  const rich = asRichValue(x);
  if (rich?.kind === "value" && !isContainer(rich.value)) return sortKey(rich.value);
  if (rich?.kind === "timing") return { rank: 0, value: rich.ms };
  return { rank: 3, value: JSON.stringify(x) };
}

export function compareKeys(a: SortKey, b: SortKey, dir: 1 | -1): number {
  // Missing values stay at the bottom in both directions.
  if (a.rank !== b.rank) {
    if (a.rank === MISSING_RANK) return 1;
    if (b.rank === MISSING_RANK) return -1;
    return (a.rank - b.rank) * dir;
  }
  if (typeof a.value === "number" && typeof b.value === "number") return (a.value - b.value) * dir;
  return collator.compare(String(a.value), String(b.value)) * dir;
}

/** A cell as CSV text: scalars as themselves, structures as JSON. */
export function csvText(x: Json | undefined): string {
  if (x === undefined || x === null) return "";
  if (!isContainer(x)) return String(x);
  const rich = asRichValue(x);
  if (rich?.kind === "value" && !isContainer(rich.value)) return rich.value === null ? "" : String(rich.value);
  return JSON.stringify(x);
}
