// The command registry, as the rest of the window sees it (work item 04-03).
//
// `catalog.js` says what exists, `registry.ts` says what it is bound to and
// runs it, `palette.ts` lists it, `editor.ts` gives CodeMirror the same keys.
// This re-exports the parts other modules need, and owns the two commands that
// are about the registry itself.

import { invoke, subscribe } from "../ipc";
import { toast } from "../toast";

import { defaultBindings } from "./catalog.js";
import { applyBindings, asMarkdown, bindingsFor, commands } from "./registry";

export { CATALOG, CATEGORIES, bindingFor, bindingsFor, commandFor, commands, format, formatAll, registerCommand, registerCommands, registerCondition, runCommand } from "./registry";
export type { Command, CommandInfo, Handler } from "./registry";
export { openCommandPalette } from "./palette";
export { EDITOR_COMMANDS, editorKeymap } from "./editor";

/** Called after bindings change, so the editor can rebuild its keymap. */
let onChange: () => void = () => {};

export function onBindingsChanged(listener: () => void): void {
  onChange = listener;
}

/**
 * Read `keybindings.toml` and keep it applied. Conflicts are reported once,
 * when they arrive: the later command wins, since refusing both would leave
 * the user with neither.
 */
export async function watchKeybindings(): Promise<void> {
  subscribe(({ event }) => {
    if (event.type === "keybindings") apply(event.bindings);
  });
  const bindings = await invoke<Record<string, string[]>>("get_keybindings").catch(() => ({}));
  apply(bindings);
}

function apply(bindings: Record<string, string[]>): void {
  const conflicts = applyBindings(bindings);
  onChange();
  if (conflicts.length) {
    toast("warning", `Two commands share a shortcut:\n${conflicts.join("\n")}`);
  }
}

/** The whole keymap as Markdown, for pasting into an issue or a note. */
export async function copyKeybindings(): Promise<void> {
  const bindings: Record<string, string[]> = {};
  for (const command of commands()) {
    const keys = bindingsFor(command.id);
    if (keys.length) bindings[command.id] = keys;
  }
  const markdown = asMarkdown({ ...defaultBindings(), ...bindings });
  try {
    await navigator.clipboard.writeText(markdown);
    toast("info", "The keybindings are on the clipboard.");
  } catch {
    toast("warning", "Couldn't reach the clipboard.");
  }
}
