// The code editor: one CodeMirror view for the lifetime of the window.
//
// Tabs (work item 01-12) will swap `EditorState` objects in and out of this
// single view, which keeps undo history, selection, and scroll per tab. So
// this module owns the view and accepts states; it never recreates the view.

import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { bracketMatching, indentOnInput } from "@codemirror/language";
import { EditorState, type Extension, Prec } from "@codemirror/state";
import {
  drawSelection,
  EditorView,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
} from "@codemirror/view";

export interface EditorActions {
  run(): void;
  cancel(): void;
  clearOutput(): void;
}

const theme = EditorView.theme(
  {
    "&": {
      height: "100%",
      backgroundColor: "var(--bg)",
      color: "var(--fg)",
      fontSize: "var(--editor-font-size)",
    },
    ".cm-scroller": { fontFamily: "var(--font-mono)", lineHeight: "1.5" },
    ".cm-content": { caretColor: "var(--accent)" },
    ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--accent)" },
    "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection": {
      backgroundColor: "#264f78 !important",
    },
    ".cm-gutters": {
      backgroundColor: "var(--bg)",
      color: "var(--muted)",
      border: "none",
    },
    ".cm-activeLine": { backgroundColor: "rgb(255 255 255 / 0.04)" },
    ".cm-activeLineGutter": { backgroundColor: "transparent", color: "var(--fg)" },
    "&.cm-focused .cm-matchingBracket": {
      backgroundColor: "rgb(59 130 246 / 0.25)",
      outline: "1px solid rgb(59 130 246 / 0.5)",
    },
  },
  { dark: true },
);

export class Editor {
  readonly view: EditorView;
  private readonly base: Extension;

  constructor(parent: HTMLElement, actions: EditorActions, doc = "") {
    // App shortcuts take precedence over CodeMirror defaults (Mod-l would
    // otherwise select the line). `Mod` is Cmd on macOS and Ctrl elsewhere.
    const appKeys = Prec.highest(
      keymap.of([
        { key: "Mod-Enter", run: () => (actions.run(), true) },
        { key: "Mod-.", run: () => (actions.cancel(), true) },
        { key: "Mod-l", run: () => (actions.clearOutput(), true) },
      ]),
    );
    this.base = [
      appKeys,
      lineNumbers(),
      highlightActiveLineGutter(),
      highlightActiveLine(),
      drawSelection(),
      history(),
      indentOnInput(),
      bracketMatching(),
      keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
      theme,
    ];
    this.view = new EditorView({ parent, state: this.createState(doc) });
  }

  /** A fresh state with the editor's standard extensions. */
  createState(doc: string): EditorState {
    return EditorState.create({ doc, extensions: this.base });
  }

  /** Swap in another state (another tab's document). */
  setState(state: EditorState): void {
    this.view.setState(state);
  }

  get text(): string {
    return this.view.state.doc.toString();
  }

  focus(): void {
    this.view.focus();
  }
}
