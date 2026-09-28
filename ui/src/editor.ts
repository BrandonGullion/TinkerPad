// The code editor: one CodeMirror view for the lifetime of the window.
//
// Tabs (work item 01-12) will swap `EditorState` objects in and out of this
// single view, which keeps undo history, selection, and scroll per tab. So
// this module owns the view and accepts states; it never recreates the view.

import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { bracketMatching, indentOnInput, syntaxHighlighting } from "@codemirror/language";
import { Compartment, EditorState, type Extension } from "@codemirror/state";
import { indentUnit } from "@codemirror/language";

import { editorKeymap } from "./commands/editor";
import { currentTheme, highlightStyle } from "./theme";
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

/** Everything about the editor that a theme decides. */
function themeExtensions(theme: "light" | "dark"): Extension {
  return [editorTheme(theme === "dark"), syntaxHighlighting(highlightStyle(theme), { fallback: true })];
}

/** Tab width and what pressing Tab inserts, from one number. */
function indentation(tabSize: number): Extension {
  return [EditorState.tabSize.of(tabSize), indentUnit.of(" ".repeat(tabSize))];
}

/**
 * The editor's own colours, all of them from the theme's variables (04-04).
 * `dark` tells CodeMirror which way its built-in styles should lean, so it is
 * built once per theme and swapped rather than being fixed.
 */
function editorTheme(dark: boolean): Extension {
  return EditorView.theme(
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
        backgroundColor: "var(--selection) !important",
      },
      ".cm-gutters": {
        backgroundColor: "var(--bg)",
        color: "var(--muted)",
        border: "none",
      },
      ".cm-activeLine": { backgroundColor: "var(--active-line)" },
      ".cm-activeLineGutter": { backgroundColor: "transparent", color: "var(--fg)" },
      "&.cm-focused .cm-matchingBracket": {
        backgroundColor: "var(--bracket-match)",
        outline: "1px solid var(--bracket-match-outline)",
      },
      ".cm-tooltip": {
        backgroundColor: "var(--panel)",
        border: "1px solid var(--border)",
        color: "var(--fg)",
      },
      ".cm-tooltip-autocomplete ul li[aria-selected]": {
        backgroundColor: "var(--accent)",
        color: "var(--accent-fg)",
      },
    },
    { dark },
  );
}

export class Editor {
  readonly view: EditorView;
  private readonly base: Extension;
  /** Tab size and indent unit, so settings can change them in place (04-02). */
  private readonly indent = new Compartment();
  private tabSize = 2;

  /** The app's own keys, rebuilt when `keybindings.toml` changes (04-03). */
  private readonly appKeys = new Compartment();
  /** The theme's colours and highlighting, swapped on a theme change (04-04). */
  private readonly colours = new Compartment();

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
      // Colours the tokens a language grammar produces, in this theme's
      // colours (04-04). Plain text without a grammar is just text.
      this.colours.of(themeExtensions(currentTheme())),
      keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
      this.indent.of(indentation(this.tabSize)),
    ];
    this.view = new EditorView({ parent, state: this.createState(doc) });
  }

  /** A fresh state with the editor's standard extensions, plus `extra`. */
  createState(doc: string, extra: Extension = []): EditorState {
    return EditorState.create({ doc, extensions: [this.base, extra] });
  }

  /** Paint in another theme's colours (work item 04-04). */
  applyTheme(theme: "light" | "dark"): void {
    this.view.dispatch({ effects: this.colours.reconfigure(themeExtensions(theme)) });
  }

  /** The app's keys changed: rebuild the editor's copy of them (04-03). */
  applyBindings(): void {
    this.view.dispatch({ effects: this.appKeys.reconfigure(editorKeymap()) });
  }

  /** Swap in another state (another tab's document). */
  setState(state: EditorState): void {
    this.view.setState(state);
    this.view.dispatch({
      effects: [
        this.appKeys.reconfigure(editorKeymap()),
        this.colours.reconfigure(themeExtensions(currentTheme())),
      ],
    });
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
