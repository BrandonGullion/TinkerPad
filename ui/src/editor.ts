// The code editor: one CodeMirror view for the lifetime of the window.
//
// Tabs (work item 01-12) will swap `EditorState` objects in and out of this
// single view, which keeps undo history, selection, and scroll per tab. So
// this module owns the view and accepts states; it never recreates the view.

import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import {
  bracketMatching,
  defaultHighlightStyle,
  indentOnInput,
  syntaxHighlighting,
} from "@codemirror/language";
import { Compartment, EditorState, type Extension } from "@codemirror/state";
import { indentUnit } from "@codemirror/language";

import { editorKeymap } from "./commands/editor";
import {
  drawSelection,
  EditorView,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
} from "@codemirror/view";

/** What a font family falls back to when the one asked for isn't installed. */
const MONO_FALLBACK = '"Cascadia Code", "JetBrains Mono", "SF Mono", Menlo, Consolas, monospace';

/** Tab width and what pressing Tab inserts, from one number. */
function indentation(tabSize: number): Extension {
  return [EditorState.tabSize.of(tabSize), indentUnit.of(" ".repeat(tabSize))];
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
  /** Tab size and indent unit, so settings can change them in place (04-02). */
  private readonly indent = new Compartment();
  private tabSize = 2;

  /** The app's own keys, rebuilt when `keybindings.toml` changes (04-03). */
  private readonly appKeys = new Compartment();

  constructor(parent: HTMLElement, doc = "") {
    this.base = [
      this.appKeys.of(editorKeymap()),
      lineNumbers(),
      highlightActiveLineGutter(),
      highlightActiveLine(),
      drawSelection(),
      history(),
      indentOnInput(),
      bracketMatching(),
      // Colours the tokens a language grammar produces. Plain text without a
      // grammar looks the same as before (02-04).
      syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
      keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
      this.indent.of(indentation(this.tabSize)),
      theme,
    ];
    this.view = new EditorView({ parent, state: this.createState(doc) });
  }

  /** A fresh state with the editor's standard extensions, plus `extra`. */
  createState(doc: string, extra: Extension = []): EditorState {
    return EditorState.create({ doc, extensions: [this.base, extra] });
  }

  /** The app's keys changed: rebuild the editor's copy of them (04-03). */
  applyBindings(): void {
    this.view.dispatch({ effects: this.appKeys.reconfigure(editorKeymap()) });
  }

  /** Swap in another state (another tab's document). */
  setState(state: EditorState): void {
    this.view.setState(state);
    this.view.dispatch({ effects: this.appKeys.reconfigure(editorKeymap()) });
    // A state made before the last settings change carries the old indent.
    this.view.dispatch({ effects: this.indent.reconfigure(indentation(this.tabSize)) });
  }

  /**
   * Apply the editor settings (work item 04-02). The fonts are CSS variables,
   * so they cost a property each; the tab size is state, and goes in through
   * a compartment rather than a new view.
   */
  applySettings(fontSize: number, fontFamily: string | null, tabSize: number): void {
    const root = document.documentElement;
    root.style.setProperty("--editor-font-size", `${fontSize}px`);
    if (fontFamily) root.style.setProperty("--font-mono", `${fontFamily}, ${MONO_FALLBACK}`);
    else root.style.removeProperty("--font-mono");
    if (tabSize !== this.tabSize) {
      this.tabSize = tabSize;
      this.view.dispatch({ effects: this.indent.reconfigure(indentation(tabSize)) });
    }
  }

  get text(): string {
    return this.view.state.doc.toString();
  }

  focus(): void {
    this.view.focus();
  }
}
