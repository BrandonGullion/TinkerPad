import { LSPPlugin } from "@codemirror/lsp-client";
import { Prec, type Extension } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import type * as lsp from "vscode-languageserver-protocol";

import { pick } from "./picker";
import type { TabManager } from "./tabs";
/**
 * Go to definition (work item 02-09).
 *
 * `F12` (or `Mod`+click) asks the server where something is defined. A target
 * in this playground moves the cursor; one in someone else's file — a crate in
 * `~/.cargo/registry`, a `.d.ts` in `node_modules` — opens read-only in a peek
 * tab. `Alt+Left` goes back to where you were, however many jumps deep.
 *
 * `Mod+F12` does the same for the *type* of the thing under the cursor, which
 * is what you want when the definition is a variable and the interesting part
 * is its type.
 */
interface Jump {
  /** The tab that was showing, and where in it. */
  tab: string;
  line: number;
  col: number;
}

/** Where to go: a file, and a place in it. */
interface Target {
  uri: string;
  line: number;
  col: number;
}

/** Where we have been, most recent last. Shared by every tab. */
const history: Jump[] = [];

/** How many jumps back are remembered. */
const HISTORY = 50;

/** LSP is zero-based; the editor counts lines from one. */
function target(location: lsp.Location | lsp.LocationLink): Target {
  const uri = "uri" in location ? location.uri : location.targetUri;
  const range = "range" in location ? location.range : location.targetSelectionRange;
  return { uri, line: range.start.line + 1, col: range.start.character + 1 };
}

/** A short label for a target, for when there is more than one. */
function label(target: Target, tabs: TabManager): string {
  const playground = playgroundOf(target.uri);
  const file = playground
    ? (tabs.summary(playground)?.name ?? "this playground")
    : decodeURIComponent(target.uri).replace(/^.*[\\/]/, "");
  return `${file}:${target.line}`;
}

/**
 * Ask the server for a definition and go there. `method` is
 * `textDocument/definition` or `textDocument/typeDefinition`.
 */
async function jumpTo(view: EditorView, method: string, tabs: TabManager): Promise<boolean> {
  const plugin = LSPPlugin.get(view);
  if (!plugin) return false;
  const pos = view.state.selection.main.head;
  plugin.client.sync();

  let result: lsp.Location | lsp.Location[] | lsp.LocationLink[] | null;
  try {
    result = await plugin.client.request<lsp.DefinitionParams, typeof result>(method, {
      textDocument: { uri: plugin.uri },
      position: plugin.toPosition(pos),
    });
  } catch {
    // A server that is restarting answers with an error; the next keystroke
    // will work (02-05).
    return true;
  }

  const found = (Array.isArray(result) ? result : result ? [result] : []).map(target);
  if (found.length === 0) {
    tabs.flashNote("No definition found.");
    return true;
  }

  let going = found[0]!;
  if (found.length > 1) {
    const chosen = await pick(
      "Go to definition",
      found.map((t, i) => ({ id: String(i), label: label(t, tabs), detail: where(t.uri) })),
      "Which one",
    );
    if (!chosen) return true;
    going = found[Number(chosen.id)]!;
  }

  remember(view, tabs);
  await go(going, tabs);
  return true;
}

/** Where a target lives, for telling two `mod.rs`es apart. */
function where(uri: string): string {
  if (playgroundOf(uri)) return "in this playground";
  return decodeURIComponent(uri.replace(/^file:\/\/\/?/, "")).replace(/[\\/][^\\/]*$/, "");
}

/** Note where the cursor is, so `Alt+Left` can bring it back. */
function remember(view: EditorView, tabs: TabManager): void {
  const tab = tabs.activeTab;
  if (!tab) return;
  const line = view.state.doc.lineAt(view.state.selection.main.head);
  history.push({
    tab: tab.id,
    line: line.number,
    col: view.state.selection.main.head - line.from + 1,
  });
  if (history.length > HISTORY) history.shift();
}

/** Open a target: this playground, another one, or someone else's file. */
async function go(target: Target, tabs: TabManager): Promise<void> {
  const playground = playgroundOf(target.uri);
  if (playground) {
    await tabs.open(playground);
    tabs.goTo(target.line, target.col);
    return;
  }
  const path = pathOf(target.uri);
  if (!path) {
    tabs.flashNote(`Can't open ${target.uri}.`);
    return;
  }
  const opened = await tabs.openReadOnly(path, target.line, target.col);
  if (!opened) tabs.flashNote(`Can't open ${path}.`);
}

/** The playground a `tinkerpad://` URI names, if it is one. */
function playgroundOf(uri: string): string | null {
  const match = /^tinkerpad:\/\/playground\/(.+)$/.exec(uri);
  return match ? (match[1] ?? null) : null;
}

/** The path a `file://` URI names, in the form this platform uses. */
function pathOf(uri: string): string | null {
  if (!uri.startsWith("file:")) return null;
  let path = decodeURIComponent(uri.replace(/^file:\/\//, ""));
  // `file:///C:/x` → `C:/x`; `file:///home/x` stays absolute.
  if (/^\/[A-Za-z]:/.test(path)) path = path.slice(1);
  return path;
}

/**
 * Go back to where the last jump started.
 *
 * Bound globally rather than in the editor, because half the point is getting
 * out of a read-only tab, and those have no language server behind them.
 */
export function jumpBack(tabs: TabManager): boolean {
  const back = history.pop();
  if (!back) {
    tabs.flashNote("Nowhere to go back to.");
    return true;
  }
  const tab = tabs.find(back.tab);
  if (!tab) {
    // The tab was closed while we were away; try the next one back.
    return jumpBack(tabs);
  }
  tabs.activate(tab);
  tabs.goTo(back.line, back.col);
  return true;
}

/** `F12`, `Mod+F12`, `Mod`+click, and `Alt+Left`. */
export function definitionKeymap(tabs: TabManager): Extension {
  return [
    Prec.high(
      keymap.of([
        {
          key: "F12",
          run: (view) => {
            void jumpTo(view, "textDocument/definition", tabs);
            return true;
          },
        },
        {
          key: "Mod-F12",
          run: (view) => {
            void jumpTo(view, "textDocument/typeDefinition", tabs);
            return true;
          },
        },
      ]),
    ),
    EditorView.domEventHandlers({
      mousedown: (event, view) => {
        if (!(event.metaKey || event.ctrlKey) || event.button !== 0) return false;
        const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
        if (pos === null) return false;
        view.dispatch({ selection: { anchor: pos } });
        void jumpTo(view, "textDocument/definition", tabs);
        event.preventDefault();
        return true;
      },
    }),
  ];
}
