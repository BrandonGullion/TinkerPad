/**
 * Show `value` in TinkerPad's output pane as a table, tree, or value, and
 * return it unchanged, so `dump(xs).length` works.
 *
 * - An array of objects becomes a sortable table (at most 1,000 rows).
 * - Other objects, arrays, `Map`s, and `Set`s become a collapsible tree.
 * - Everything else becomes a single value with its type.
 *
 * @param title A caption shown above the value.
 */
export function dump<T>(value: T, title?: string): T;

/** The dump node `dump` would write for `value`, without writing it. */
export function toDumpNode(value: unknown, title?: string): object;
