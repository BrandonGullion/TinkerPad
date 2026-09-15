// Rendering dumped values (`tinkerpad-dump-v1`, docs/PLUGINS.md §6).
//
// `renderRich` turns one RichValue into an element for the output pane. Every
// node gets a "Copy JSON" action (tables also "Copy CSV") and, when it has a
// title, a caption above it. Scalars and timings without a title render
// inline, in the flow of the surrounding text.

import type { RichNode, RichValue } from "../protocol";
import { copyButton } from "./copy";
import { renderHtml } from "./html";
import { highlightInto } from "./highlight";
import { el, formatMs, isContainer, primitive, typeName } from "./json";
import { renderTable } from "./table";
import { renderTree } from "./tree";
import "./dump.css";

export interface RenderContext {
  /** The scrolling element the value is shown in. Big tables watch it. */
  scrollRoot: HTMLElement | null;
}

const INLINE_KINDS = new Set<RichNode["kind"]>(["value", "timing"]);

export function renderRich(value: RichValue, ctx: RenderContext): HTMLElement {
  const inline = INLINE_KINDS.has(value.kind) && !value.title;
  const node = el(inline ? "span" : "div", `dump dump-kind-${value.kind}${inline ? " dump-inline" : ""}`);
  node.dataset.kind = value.kind;

  const actions = el("span", "dump-actions");
  actions.append(copyButton("JSON", () => JSON.stringify(value, null, 2)));

  let body: HTMLElement;
  switch (value.kind) {
    case "value":
      body = renderValue(value);
      break;
    case "timing":
      body = el("span", "dump-timing");
      body.append(el("span", "timing-label", value.label), el("span", "timing-ms", formatMs(value.ms)));
      break;
    case "table": {
      const table = renderTable(value, {
        scrollRoot: ctx.scrollRoot,
        renderNested: (nested) => renderRich(nested, ctx),
      });
      actions.append(copyButton("CSV", table.toCsv));
      body = table.element;
      break;
    }
    case "tree":
      body = renderTree(value.value, { openDepth: 2, type: value.type });
      break;
    case "text": {
      body = el("pre", "dump-text");
      if (value.lang) body.dataset.lang = value.lang;
      highlightInto(body, value.text, value.lang);
      break;
    }
    case "html":
      body = renderHtml(value.html);
      break;
    case "image":
      body = renderImage(value.mime, value.base64);
      break;
    case "unknown": {
      body = el("div");
      const kind = typeof (value.raw as { kind?: unknown })?.kind === "string" ? (value.raw as { kind: string }).kind : "?";
      body.append(
        el("div", "dump-note", `This app doesn't know how to show a "${kind}" dump yet. Here's the raw value:`),
        renderTree(value.raw, { openDepth: 1 }),
      );
      break;
    }
    case "truncated":
      body = el("div", "dump-note", `This dump was too large to show (${value.bytes.toLocaleString()} bytes).`);
      break;
  }

  if (inline) {
    node.append(body, actions);
  } else {
    const head = el("div", value.title ? "dump-head" : "dump-head untitled");
    if (value.title) head.append(el("span", "dump-title", value.title));
    head.append(actions);
    node.append(head, body);
  }
  return node;
}

function renderValue(value: Extract<RichValue, { kind: "value" }>): HTMLElement {
  const body = el("span", "dump-value");
  if (isContainer(value.value)) {
    // Not a scalar, though the schema expects one: show it as a tree.
    body.append(renderTree(value.value, { openDepth: 1 }));
  } else {
    body.append(primitive(value.value));
  }
  body.append(el("span", "dump-type", value.type ?? typeName(value.value)));
  return body;
}

function renderImage(mime: string, base64: string): HTMLElement {
  if (!/^image\/[\w.+-]+$/i.test(mime) || !/^[A-Za-z0-9+/=\s]*$/.test(base64)) {
    return el("div", "dump-note", `Can't show an image of type "${mime}".`);
  }
  const box = el("div", "dump-image-box");
  const img = el("img", "dump-image");
  img.alt = "Dumped image";
  img.dataset.nav = "";
  img.tabIndex = -1;
  img.title = "Click to expand";
  img.setAttribute("aria-expanded", "false");
  img.addEventListener("click", () => {
    const open = img.classList.toggle("expanded");
    img.setAttribute("aria-expanded", String(open));
    img.title = open ? "Click to shrink" : "Click to expand";
  });
  img.addEventListener("error", () => box.replaceChildren(el("div", "dump-note", "This image couldn't be decoded.")));
  img.src = `data:${mime};base64,${base64.replace(/\s/g, "")}`;
  box.append(img);
  return box;
}
