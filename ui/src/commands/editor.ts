// The commands CodeMirror runs itself (work item 04-03).
//
// The editor has its own keymap, so a shortcut works the same whether focus is
// in the text or not — and so the text can have the key first when that is
// what the user means. `Mod+Enter` with a completion popup open accepts the
// completion rather than running the program; everything else goes straight to
// the registry.
//
// The keymap is rebuilt when bindings change, so `keybindings.toml` reaches
// the editor without the view being recreated.

import { acceptCompletion, completionStatus } from "@codemirror/autocomplete";
import { type Extension, Prec } from "@codemirror/state";
import { type EditorView, keymap } from "@codemirror/view";

import { bindingsFor, runCommand } from "./registry";

/** Commands the editor binds itself; the window doesn't run them again. */
export const EDITOR_COMMANDS = ["run.start", "run.cancel", "run.clearOutput"];

/** The editor's keymap, at a precedence above CodeMirror's own defaults. */
export function editorKeymap(): Extension {
  const bindings = EDITOR_COMMANDS.flatMap((id) =>
    bindingsFor(id).flatMap((binding) => {
      const key = codeMirrorKey(binding);
      if (!key) return [];
      // A completion popup owns Enter while it is open: accepting what you
      // were choosing is what Mod+Enter means there (04-03's gotcha). F5
      // can't mean that, so it runs whatever is showing.
      const acceptsFirst = id === "run.start" && /(^|-)Enter$/.test(key);
      return [
        {
          key,
          run: (view: EditorView) => {
            if (acceptsFirst && completionStatus(view.state) === "active") {
              return acceptCompletion(view);
            }
            void runCommand(id);
            return true;
          },
        },
      ];
    }),
  );
  return Prec.highest(keymap.of(bindings));
}

/** A binding in CodeMirror's spelling: `mod+enter` becomes `Mod-Enter`. */
export function codeMirrorKey(binding: string | undefined): string | null {
  if (!binding) return null;
  const parts = binding.split("+").map((part) => part.trim().toLowerCase());
  const key = parts.pop();
  if (!key) return null;
  const names: Record<string, string> = {
    mod: "Mod",
    ctrl: "Ctrl",
    cmd: "Cmd",
    meta: "Cmd",
    alt: "Alt",
    option: "Alt",
    shift: "Shift",
  };
  const mods = parts.map((part) => names[part]).filter((name): name is string => Boolean(name));
  return [...mods, keyName(key)].join("-");
}

/** CodeMirror names keys as the browser does: `Enter`, `F8`, `l`, `.`. */
function keyName(key: string): string {
  if (key.length === 1) return key;
  return key.charAt(0).toUpperCase() + key.slice(1);
}
