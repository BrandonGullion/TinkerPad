// `tree` dumps, and any plain JSON structure: a collapsible tree.
//
// Children are created the first time a node opens, and then only a page at a
// time, so a 50,000-element array costs one page of rows until asked for more.

import type { Json } from "../protocol";
import { el, isContainer, jsonSummary, primitive, type JsonObject } from "./json";

/** Children rendered per page when a node opens. */
export const TREE_PAGE = 200;

export interface TreeOptions {
  /** Nodes shallower than this start open. 2 opens the root and its children. */
  openDepth: number;
  /** Shown as a badge on the root row. */
  type?: string;
}

export function renderTree(value: Json, options: TreeOptions = { openDepth: 2 }): HTMLElement {
  const tree = el("div", "dump-tree");
  tree.setAttribute("role", "tree");
  tree.append(treeNode(null, value, 0, options));
  return tree;
}

function treeNode(key: string | null, value: Json, depth: number, options: TreeOptions): HTMLElement {
  const item = el("div", "tree-node");
  item.setAttribute("role", "treeitem");
  const row = el("div", "tree-row");
  row.dataset.nav = "";
  row.tabIndex = -1;
  const toggle = el("span", "tree-toggle");
  row.append(toggle);
  if (key !== null) row.append(el("span", "tree-key", key), el("span", "tree-colon", ": "));
  item.append(row);

  if (!isContainer(value)) {
    row.append(primitive(value, true));
    if (depth === 0 && options.type) row.append(el("span", "dump-type", options.type));
    return item;
  }

  const count = Array.isArray(value) ? value.length : Object.keys(value).length;
  const summary = el("span", "tree-summary");
  row.append(summary);
  if (depth === 0 && options.type) row.append(el("span", "dump-type", options.type));
  if (count === 0) {
    summary.textContent = Array.isArray(value) ? "[]" : "{}";
    return item;
  }

  const children = el("div", "tree-children");
  children.setAttribute("role", "group");
  item.append(children);

  let keys: string[] | null = null;
  let rendered = 0;
  let open = false;
  let more: HTMLElement | null = null;

  const renderPage = () => {
    keys ??= Array.isArray(value) ? null : Object.keys(value);
    const end = Math.min(count, rendered + TREE_PAGE);
    const fragment = document.createDocumentFragment();
    for (let i = rendered; i < end; i++) {
      const k = keys ? keys[i]! : String(i);
      const child = keys ? (value as JsonObject)[k]! : (value as Json[])[i]!;
      fragment.append(treeNode(k, child, depth + 1, options));
    }
    rendered = end;
    more?.remove();
    more = null;
    if (rendered < count) {
      more = el("div", "tree-more", `… ${(count - rendered).toLocaleString()} more`);
      more.dataset.nav = "";
      more.tabIndex = -1;
      more.setAttribute("role", "button");
      more.addEventListener("click", (e) => {
        e.stopPropagation();
        const focused = document.activeElement === more;
        const firstNew = rendered;
        renderPage();
        // Keep the keyboard where the list continues.
        if (focused) children.querySelectorAll<HTMLElement>(":scope > .tree-node > .tree-row")[firstNew]?.focus();
      });
      fragment.append(more);
    }
    children.append(fragment);
  };

  const setOpen = (next: boolean) => {
    open = next;
    if (open && rendered === 0) renderPage();
    children.hidden = !open;
    toggle.textContent = open ? "▾" : "▸";
    row.setAttribute("aria-expanded", String(open));
    // Arrays always show their length; objects show a key count when closed
    // (and at the root, where an open object would otherwise say nothing).
    summary.textContent = Array.isArray(value) || !open || depth === 0 ? jsonSummary(value) : "";
  };

  row.addEventListener("click", (e) => {
    e.stopPropagation();
    // Selecting text inside a row shouldn't collapse it.
    if (!(window.getSelection()?.isCollapsed ?? true)) return;
    setOpen(!open);
  });
  setOpen(depth < options.openDepth);
  return item;
}
