// TinkerPad's dump library for TypeScript and JavaScript on Node.
//
// `dump(value, title?)` writes one `tinkerpad-dump-v1` value to stdout as
// `ESC ] 1337 ; TinkerPad= <json> BEL` (docs/PLUGINS.md §6) and returns
// `value`. This file decides the shape; the app only renders it:
//
// - a non-empty array whose items are all objects -> `table`, columns from the
//   union of their keys in first-seen order, at most MAX_ROWS rows;
// - any other object, array, Map, or Set          -> `tree`;
// - everything else                               -> `value`.
//
// Nesting deeper than MAX_DEPTH becomes a short summary string, a reference
// back to an enclosing object becomes "[circular]", and values JSON can't hold
// (undefined, NaN, bigint, Date, functions, symbols) become sensible scalars.

const ESC = String.fromCharCode(0x1b);
const BEL = String.fromCharCode(0x07);
const PREFIX = `${ESC}]1337;TinkerPad=`;

/** Levels of nesting shown; containers below that are summarised. */
export const MAX_DEPTH = 5;
/** Rows kept in a table, and items kept in an array, Map, Set, or object. */
export const MAX_ROWS = 1000;

/**
 * Show `value` in the output pane and return it unchanged.
 * @template T
 * @param {T} value
 * @param {string} [title]
 * @returns {T}
 */
export function dump(value, title) {
  process.stdout.write(PREFIX + JSON.stringify(toDumpNode(value, title)) + BEL);
  return value;
}

/**
 * The dump node for `value`, without writing it.
 * @param {unknown} value
 * @param {string} [title]
 */
export function toDumpNode(value, title) {
  const node = describe(value, 0, new Set());
  if (title !== undefined && title !== null) node.title = String(title);
  return node;
}

/** A rich node for `value` at `depth`, with `ancestors` for cycle detection. */
function describe(value, depth, ancestors) {
  if (isTable(value)) return table(value, depth, ancestors);
  if (isContainer(value)) {
    return { v: 1, kind: "tree", value: toJson(value, depth, ancestors), type: typeName(value) };
  }
  return { v: 1, kind: "value", value: scalar(value), type: typeName(value) };
}

function table(rows, depth, ancestors) {
  ancestors.add(rows);
  const kept = rows.slice(0, MAX_ROWS);
  const columns = [];
  const index = new Map();
  for (const row of kept) {
    for (const key of Object.keys(row)) {
      if (!index.has(key)) {
        index.set(key, columns.length);
        columns.push(key);
      }
    }
  }
  const types = columns.map(() => new Set());
  const body = kept.map((row) => {
    if (ancestors.has(row)) return columns.map(() => "[circular]");
    ancestors.add(row);
    const cells = columns.map((key, c) => {
      if (!Object.prototype.hasOwnProperty.call(row, key)) return null;
      const cell = row[key];
      if (cell !== null && cell !== undefined) types[c].add(columnType(cell));
      return tableCell(cell, depth + 2, ancestors);
    });
    ancestors.delete(row);
    return cells;
  });
  ancestors.delete(rows);
  return {
    v: 1,
    kind: "table",
    columns: columns.map((name, c) => {
      const seen = [...types[c]];
      return seen.length === 1 ? { name, type: seen[0] } : seen.length === 0 ? { name } : { name, type: "mixed" };
    }),
    rows: body,
    truncated: rows.length > kept.length,
  };
}

/** A table cell: scalars inline, structures as nested nodes the app can open. */
function tableCell(value, depth, ancestors) {
  if (!isContainer(value)) return scalar(value);
  if (ancestors.has(value)) return "[circular]";
  if (depth >= MAX_DEPTH) return summary(value);
  return describe(value, depth, ancestors);
}

/** `value` as plain JSON for a tree. */
function toJson(value, depth, ancestors) {
  if (!isContainer(value)) return scalar(value);
  if (ancestors.has(value)) return "[circular]";
  if (depth >= MAX_DEPTH) return summary(value);
  ancestors.add(value);
  try {
    const next = (v) => toJson(v, depth + 1, ancestors);
    if (value instanceof Map) {
      const entries = [...value.entries()];
      const kept = entries.slice(0, MAX_ROWS);
      if (kept.every(([k]) => typeof k === "string" || typeof k === "number")) {
        const out = {};
        for (const [k, v] of kept) out[String(k)] = next(v);
        if (entries.length > kept.length) out["…"] = more(entries.length - kept.length);
        return out;
      }
      const out = kept.map(([k, v]) => ({ key: next(k), value: next(v) }));
      if (entries.length > kept.length) out.push(more(entries.length - kept.length));
      return out;
    }
    if (Array.isArray(value) || value instanceof Set || ArrayBuffer.isView(value)) {
      const items = Array.isArray(value) ? value : Array.from(value);
      const out = items.slice(0, MAX_ROWS).map(next);
      if (items.length > MAX_ROWS) out.push(more(items.length - MAX_ROWS));
      return out;
    }
    if (value instanceof Error) {
      const out = { name: value.name, message: value.message };
      if (value.stack) out.stack = value.stack;
      for (const key of Object.keys(value)) out[key] = next(value[key]);
      return out;
    }
    const keys = Object.keys(value);
    const out = {};
    for (const key of keys.slice(0, MAX_ROWS)) out[key] = next(value[key]);
    if (keys.length > MAX_ROWS) out["…"] = more(keys.length - MAX_ROWS);
    return out;
  } finally {
    ancestors.delete(value);
  }
}

/** A value JSON can hold, standing in for one it can't. */
function scalar(value) {
  switch (typeof value) {
    case "string":
    case "boolean":
      return value;
    case "number":
      return Number.isFinite(value) ? value : String(value);
    case "bigint":
      return `${value}n`;
    case "undefined":
      return null;
    case "symbol":
      return value.toString();
    case "function":
      return /^class\b/.test(Function.prototype.toString.call(value))
        ? `[class ${value.name || "(anonymous)"}]`
        : `[Function: ${value.name || "(anonymous)"}]`;
    default:
      if (value === null) return null;
      if (value instanceof Date) return Number.isNaN(value.getTime()) ? "Invalid Date" : value.toISOString();
      if (value instanceof RegExp) return String(value);
      if (value instanceof Promise) return "[Promise]";
      if (value instanceof WeakMap) return "[WeakMap]";
      if (value instanceof WeakSet) return "[WeakSet]";
      return summary(value);
  }
}

/** Objects the tree shows structurally; the rest (Date, RegExp, ...) are scalars. */
function isContainer(value) {
  if (typeof value !== "object" || value === null) return false;
  return !(
    value instanceof Date ||
    value instanceof RegExp ||
    value instanceof Promise ||
    value instanceof WeakMap ||
    value instanceof WeakSet
  );
}

function isRecord(value) {
  return (
    isContainer(value) &&
    !Array.isArray(value) &&
    !(value instanceof Map) &&
    !(value instanceof Set) &&
    !ArrayBuffer.isView(value)
  );
}

function isTable(value) {
  return Array.isArray(value) && value.length > 0 && value.every(isRecord);
}

function typeName(value) {
  if (value === null) return "null";
  if (Array.isArray(value)) return `Array(${value.length})`;
  if (value instanceof Map) return `Map(${value.size})`;
  if (value instanceof Set) return `Set(${value.size})`;
  if (typeof value === "function") return "function";
  if (typeof value !== "object") return typeof value;
  const name = value.constructor?.name;
  return name && name !== "Object" ? name : "object";
}

function summary(value) {
  if (Array.isArray(value)) return `[Array(${value.length})]`;
  if (value instanceof Map) return `[Map(${value.size})]`;
  if (value instanceof Set) return `[Set(${value.size})]`;
  const name = value?.constructor?.name;
  return `[${name && name !== "Object" ? name : "Object"}]`;
}

function more(n) {
  return `… ${n} more`;
}

/** A column's type hint: like typeName, without sizes. */
function columnType(value) {
  if (Array.isArray(value)) return "array";
  if (value instanceof Map) return "Map";
  if (value instanceof Set) return "Set";
  return typeName(value);
}
