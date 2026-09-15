// Shared helpers for dumped JSON: recognising nested dump values, rendering
// primitives, and one-line summaries for collapsed values.

import type { Json, RichValue } from "../protocol";

export type JsonObject = { [key: string]: Json };

export function isObject(x: Json | undefined): x is JsonObject {
  return typeof x === "object" && x !== null && !Array.isArray(x);
}

export function isContainer(x: Json | undefined): x is Json[] | JsonObject {
  return typeof x === "object" && x !== null;
}

/** Create an element with a class and optional text. */
export function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  className = "",
  text?: string,
): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (className) e.className = className;
  if (text !== undefined) e.textContent = text;
  return e;
}

/**
 * Read a JSON object as a dump value, if it is one (`{"v": 1, "kind": ...}`).
 * Table cells can hold these. The core validated the top-level value but not
 * what's nested inside it, so a node with the wrong fields for its kind comes
 * back as `unknown` and renders as raw JSON.
 */
export function asRichValue(x: Json | undefined): RichValue | null {
  if (!isObject(x) || typeof x.kind !== "string" || typeof x.v !== "number") return null;
  if (valid(x)) return x as unknown as RichValue;
  const title = typeof x.title === "string" ? x.title : undefined;
  return { v: x.v, title, kind: "unknown", raw: x };
}

function valid(x: JsonObject): boolean {
  if (x.title !== undefined && typeof x.title !== "string") return false;
  switch (x.kind) {
    case "value":
    case "tree":
      return "value" in x;
    case "table":
      return (
        Array.isArray(x.columns) &&
        x.columns.every((c) => isObject(c) && typeof c.name === "string") &&
        Array.isArray(x.rows) &&
        x.rows.every((r) => Array.isArray(r))
      );
    case "text":
      return typeof x.text === "string";
    case "html":
      return typeof x.html === "string";
    case "image":
      return typeof x.mime === "string" && typeof x.base64 === "string";
    case "timing":
      return typeof x.label === "string" && typeof x.ms === "number";
    case "truncated":
      return typeof x.bytes === "number";
    case "unknown":
      return "raw" in x;
    default:
      return false;
  }
}

/** The JSON type name shown in badges when the dump didn't give one. */
export function typeName(x: Json): string {
  if (x === null) return "null";
  if (Array.isArray(x)) return "array";
  return typeof x;
}

/**
 * A primitive as a coloured span. Strings are quoted in trees, where they
 * sit next to numbers and keys, and plain in tables and values.
 */
export function primitive(x: Json, quote = false): HTMLElement {
  if (x === null) return el("span", "j-null", "null");
  switch (typeof x) {
    case "string":
      return el("span", "j-str", quote ? JSON.stringify(x) : x);
    case "number":
      return el("span", "j-num", String(x));
    case "boolean":
      return el("span", "j-bool", String(x));
    default:
      // Containers aren't primitives; show them compactly rather than fail.
      return el("span", "j-other", JSON.stringify(x));
  }
}

export function plural(n: number, word: string): string {
  return `${n.toLocaleString()} ${word}${n === 1 ? "" : "s"}`;
}

/** `[3]` for arrays, `{2 keys}` for objects. */
export function jsonSummary(x: Json[] | JsonObject): string {
  return Array.isArray(x) ? `[${x.length.toLocaleString()}]` : `{${plural(Object.keys(x).length, "key")}}`;
}

/** A one-line description of a collapsed dump value. */
export function richSummary(v: RichValue): string {
  const clip = (s: string) => (s.length > 40 ? `${s.slice(0, 40)}…` : s);
  const title = v.title ? `${v.title} · ` : "";
  switch (v.kind) {
    case "value":
      return title + clip(isContainer(v.value) ? JSON.stringify(v.value) : String(v.value));
    case "tree":
      return title + (isContainer(v.value) ? jsonSummary(v.value) : clip(String(v.value)));
    case "table":
      return `${title}table · ${plural(v.rows.length, "row")}`;
    case "text":
      return title + clip(v.text.split("\n", 1)[0] ?? "");
    case "html":
      return `${title}html`;
    case "image":
      return `${title}${v.mime}`;
    case "timing":
      return `${title}${v.label} ${formatMs(v.ms)}`;
    case "unknown":
      return `${title}unknown kind`;
    case "truncated":
      return `${title}too large`;
  }
}

export function formatMs(ms: number): string {
  if (ms >= 1000) return `${(ms / 1000).toFixed(ms >= 10_000 ? 1 : 2)} s`;
  if (ms >= 100) return `${Math.round(ms)} ms`;
  return `${Number(ms.toPrecision(3))} ms`;
}
