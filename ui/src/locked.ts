// Locked lines: a kind's prelude and postlude, shown in the editor but not
// editable (docs/PLUGINS.md §3).
//
// The locked regions are part of the document, exactly as on disk, so the
// language server, the runner, and a future debugger see the same text the
// editor shows. Nothing is hidden and nothing is transformed on save.

import {
  EditorSelection,
  EditorState,
  type Extension,
  Prec,
  RangeSetBuilder,
  StateField,
  type Transaction,
} from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  gutter,
  GutterMarker,
  keymap,
} from "@codemirror/view";

/**
 * Locked character ranges: the head is `[0, head)` and the tail is
 * `[tail, length)`. Everything between is editable.
 */
export interface LockedRanges {
  head: number;
  tail: number;
}

/**
 * Where the prelude and postlude are in `text`, or `null` if the text doesn't
 * start and end with them. Offsets are JavaScript string offsets, which is
 * what CodeMirror uses (not bytes, which is what the Rust side counts).
 */
export function findLocked(text: string, prelude: string, postlude: string): LockedRanges | null {
  if (!prelude && !postlude) return null;
  if (text.length < prelude.length + postlude.length) return null;
  if (!text.startsWith(prelude) || !text.endsWith(postlude)) return null;
  return { head: prelude.length, tail: text.length - postlude.length };
}

/** Locked ranges, kept correct as the editable middle changes size. */
const lockedField = StateField.define<LockedRanges | null>({
  create: () => null,
  update(value, tr) {
    if (!value || !tr.docChanged) return value;
    // Edits only ever land in the middle, so the head never moves and the
    // tail moves by however much the middle grew or shrank. Associating
    // right keeps an insertion at the tail boundary on the editable side.
    return { head: value.head, tail: tr.changes.mapPos(value.tail, 1) };
  },
});

/** The editable middle of the document, or the whole document if nothing is locked. */
export function editableRange(state: EditorState): { from: number; to: number } {
  const locked = state.field(lockedField, false);
  return locked ? { from: locked.head, to: locked.tail } : { from: 0, to: state.doc.length };
}

/** Whether a change `[from, to)` (an insertion when equal) reaches into a locked range. */
function reachesLocked(from: number, to: number, locked: LockedRanges): boolean {
  if (from === to) {
    // Insertions at the middle's edges are fine; anywhere outside isn't.
    return from < locked.head || from > locked.tail;
  }
  return from < locked.head || to > locked.tail;
}

function transactionReachesLocked(tr: Transaction, locked: LockedRanges): boolean {
  let reaches = false;
  tr.changes.iterChangedRanges((fromA, toA) => {
    if (reachesLocked(fromA, toA, locked)) reaches = true;
  });
  return reaches;
}

class LockMarker extends GutterMarker {
  override toDOM() {
    const el = document.createElement("span");
    el.className = "cm-lock-marker";
    el.textContent = "🔒";
    el.title = "Locked: part of this playground's kind";
    return el;
  }
}
const lockMarker = new LockMarker();
const lockedLine = Decoration.line({ class: "cm-locked-line" });

/** Start positions of lines that lie entirely inside a locked range. */
export function lockedLineStarts(state: EditorState): number[] {
  const locked = state.field(lockedField, false);
  if (!locked) return [];
  const doc = state.doc;
  const starts: number[] = [];
  for (let n = 1; n <= doc.lines; n++) {
    const line = doc.line(n);
    // A line is locked if all of its text, and its line break, are locked.
    // The prelude usually ends in a newline, so the line after it starts at
    // `head` and is editable; the postlude usually starts with one, so the
    // line before it ends at `tail` and is editable.
    const lineEnd = n < doc.lines ? line.to + 1 : line.to;
    const inHead = locked.head > 0 && lineEnd <= locked.head;
    const inTail = locked.tail < doc.length && line.from > locked.tail;
    if (inHead || inTail) starts.push(line.from);
  }
  return starts;
}

const decorations = EditorView.decorations.compute([lockedField, "doc"], (state): DecorationSet => {
  const builder = new RangeSetBuilder<Decoration>();
  for (const from of lockedLineStarts(state)) builder.add(from, from, lockedLine);
  return builder.finish();
});

const lockGutter = gutter({
  class: "cm-lock-gutter",
  markers: (view) => {
    const builder = new RangeSetBuilder<GutterMarker>();
    for (const from of lockedLineStarts(view.state)) builder.add(from, from, lockMarker);
    return builder.finish();
  },
});

/**
 * The editor extension for a document with locked regions (none if `ranges`
 * is null). `onBlocked` is called when the user's edit is refused, so the UI
 * can show a hint.
 */
export function lockedLinesExtension(ranges: LockedRanges | null, onBlocked: () => void): Extension {
  if (!ranges) return [];
  return [
    lockedField.init(() => ranges),

    EditorState.changeFilter.of((tr) => {
      const locked = tr.startState.field(lockedField, false);
      if (!locked) return true;
      if (!transactionReachesLocked(tr, locked)) return true;

      if (tr.isUserEvent("input") || tr.isUserEvent("delete") || tr.isUserEvent("move")) {
        queueMicrotask(onBlocked);
      }

      // An insertion at the very start or end of the document sits on the
      // edge of the suppressed ranges below and would slip through, so refuse
      // the whole transaction in that case.
      const length = tr.startState.doc.length;
      let edgeInsert = false;
      tr.changes.iterChangedRanges((fromA, toA) => {
        if (fromA !== toA) return;
        if ((fromA === 0 && locked.head > 0) || (fromA === length && locked.tail < length)) {
          edgeInsert = true;
        }
      });
      if (edgeInsert) return false;

      // Otherwise drop only the parts inside locked ranges. A select-all
      // delete then clears just the middle.
      const suppressed: number[] = [];
      if (locked.head > 0) suppressed.push(0, locked.head);
      if (locked.tail < length) suppressed.push(locked.tail, length);
      return suppressed;
    }),

    // Mod+A selects only the editable middle, so typing replaces just that.
    Prec.high(
      keymap.of([
        {
          key: "Mod-a",
          run: (view) => {
            const { from, to } = editableRange(view.state);
            view.dispatch({ selection: EditorSelection.single(from, to), userEvent: "select" });
            return true;
          },
        },
      ]),
    ),

    decorations,
    lockGutter,
    EditorView.theme({
      ".cm-locked-line": { backgroundColor: "rgb(255 255 255 / 0.04)" },
      ".cm-locked-line, .cm-locked-line *": { color: "var(--muted)" },
      ".cm-lock-gutter .cm-gutterElement": { width: "1.4em", textAlign: "center" },
      ".cm-lock-marker": { fontSize: "0.7em", opacity: "0.7" },
    }),
  ];
}
