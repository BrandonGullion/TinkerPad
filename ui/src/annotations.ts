// Compiler and runtime positions marked in the editor (work item 01-13).
//
// The runner finds them in a run's output with the plugin's `[output]` pattern
// and sends them as `annotation` events. Only those pointing at the tab's own
// file are marked here; the rest stay clickable in the output pane. They live
// in the editor state, so each tab keeps its own, and they're replaced at the
// start of the next run.

import { type EditorState, type Extension, StateEffect, StateField } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView } from "@codemirror/view";

export interface EditorAnnotation {
  /** One-based, as tools report it. */
  line: number;
  /** One-based, or null when the pattern captured no column. */
  col: number | null;
  message: string;
  severity: "error" | "warning";
  /** Added to the hover text, for example when the line is a locked one. */
  note?: string;
}

/** Replace the editor's annotations. */
export const setAnnotations = StateEffect.define<EditorAnnotation[]>();

const field = StateField.define<{ list: EditorAnnotation[]; marks: DecorationSet }>({
  create: () => ({ list: [], marks: Decoration.none }),
  update(value, tr) {
    for (const effect of tr.effects) {
      if (effect.is(setAnnotations)) return { list: effect.value, marks: build(effect.value, tr.state) };
    }
    if (!tr.docChanged) return value;
    // Keep them roughly in place while the document changes; the next run
    // replaces them anyway.
    return { list: value.list, marks: value.marks.map(tr.changes) };
  },
  provide: (f) => EditorView.decorations.from(f, (v) => v.marks),
});

function build(list: EditorAnnotation[], state: EditorState): DecorationSet {
  const ranges = [];
  for (const annotation of [...list].sort((a, b) => a.line - b.line)) {
    const lineNumber = Math.min(Math.max(annotation.line, 1), state.doc.lines);
    const line = state.doc.line(lineNumber);
    const indent = line.text.length - line.text.trimStart().length;
    let from = annotation.col
      ? Math.min(line.from + annotation.col - 1, line.to)
      : line.from + indent;
    const to = line.to;
    // A compiler reporting something *missing* points one past the last
    // character — `; expected` on a line of 22 characters is column 23. There
    // is nothing there to underline, so underline what it followed.
    if (from >= to) from = Math.max(line.from + indent, to - 1);
    if (to <= from) continue; // A blank line: nothing to mark.
    const title = annotation.note ? `${annotation.note}\n${annotation.message}` : annotation.message;
    ranges.push(
      Decoration.mark({
        class: `cm-annotation cm-annotation-${annotation.severity}`,
        attributes: { title },
      }).range(from, to),
    );
  }
  return Decoration.set(ranges, true);
}

/** The annotations currently in an editor state, in line order. */
export function annotationsOf(state: EditorState): EditorAnnotation[] {
  return [...(state.field(field, false)?.list ?? [])].sort((a, b) => a.line - b.line || (a.col ?? 0) - (b.col ?? 0));
}

export function annotationsExtension(): Extension {
  return [
    field,
    EditorView.theme({
      ".cm-annotation": {
        // Longhands rather than the shorthand, which older WebKit (the
        // webview on macOS and Linux) only partly supports.
        textDecorationLine: "underline",
        textDecorationStyle: "wavy",
        textDecorationColor: "var(--error)",
        textDecorationSkipInk: "none",
        textUnderlineOffset: "3px",
      },
      ".cm-annotation-warning": { textDecorationColor: "var(--warning)" },
    }),
  ];
}
