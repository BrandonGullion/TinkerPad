// The command registry (work item 04-03).
//
// Every action the window can take is a command with an id. The palette lists
// them, the keyboard dispatches to them, `keybindings.toml` renames their
// keys, and `docs/KEYBINDINGS.md` is generated from the same list. Nothing
// else in the UI decides what a key does.
//
// A binding is a string: lowercase parts joined by `+`, modifiers first, the
// key last — `mod+shift+p`, `f8`, `alt+arrowleft`. `mod` is Cmd on macOS and
// Ctrl elsewhere; `ctrl` and `cmd` mean themselves, for the few bindings that
// really are platform-specific. A command can have several bindings; the
// first is the one the palette and tooltips name.

import { CATALOG, CATEGORIES, defaultBindings, type CommandInfo } from "./catalog.js";

export type { CommandInfo };
export { CATALOG, CATEGORIES };

export const isMac = navigator.platform.toLowerCase().includes("mac");

/** What a command does, and whether it can do it now. */
export interface Handler {
  run: () => void | Promise<void>;
  /** When false, the palette dims it and its key does nothing. */
  enabled?: () => boolean;
}

/** A command as the palette sees it: what it is, and what it is bound to. */
export interface Command extends CommandInfo {
  /** The first of `bindings`: the one worth naming in a tooltip. */
  binding?: string;
  bindings: string[];
  enabled: boolean;
}

const handlers = new Map<string, Handler>();
const conditions = new Map<string, () => boolean>();
/** Bindings from `keybindings.toml`, over the defaults. An empty list unbinds. */
let overrides: Record<string, string[]> = {};

/** Give a command its behaviour. Ids not in the catalog are refused. */
export function registerCommand(id: string, handler: Handler): void {
  if (!CATALOG.some((command) => command.id === id)) {
    throw new Error(`no command ${id} in the catalog`);
  }
  handlers.set(id, handler);
}

export function registerCommands(map: Record<string, Handler | (() => void)>): void {
  for (const [id, handler] of Object.entries(map)) {
    registerCommand(id, typeof handler === "function" ? { run: handler } : handler);
  }
}

/** Define what a `when` in the catalog means, e.g. `playground`. */
export function registerCondition(name: string, test: () => boolean): void {
  conditions.set(name, test);
}

/** Whether a command can run right now. */
export function enabled(command: CommandInfo): boolean {
  if (!handlers.has(command.id)) return false;
  const handler = handlers.get(command.id);
  if (handler?.enabled && !handler.enabled()) return false;
  if (!command.when) return true;
  return conditions.get(command.when)?.() ?? true;
}

/** Every command that has a handler, with its binding and availability. */
export function commands(): Command[] {
  return CATALOG.filter((command) => handlers.has(command.id)).map((command) => {
    const bindings = bindingsFor(command.id);
    return { ...command, binding: bindings[0], bindings, enabled: enabled(command) };
  });
}

/** Everything `id` is bound to now: the user's bindings, else the catalog's. */
export function bindingsFor(id: string): string[] {
  return overrides[id] ?? defaultBindings()[id] ?? [];
}

/** The binding to name for `id`: the first of `bindingsFor`. */
export function bindingFor(id: string): string | undefined {
  return bindingsFor(id)[0];
}

/** Run a command by id. Unknown or unavailable ones do nothing. */
export async function runCommand(id: string): Promise<void> {
  const command = CATALOG.find((c) => c.id === id);
  const handler = handlers.get(id);
  if (!command || !handler || !enabled(command)) return;
  await handler.run();
}

/**
 * Replace the user's bindings. Returns the conflicts: two commands bound to
 * the same keys, which the caller reports — the later one wins, because
 * refusing both would leave the user with neither.
 */
export function applyBindings(user: Record<string, string[]>): string[] {
  overrides = { ...user };
  const seen = new Map<string, string>();
  const conflicts: string[] = [];
  for (const command of CATALOG) {
    for (const binding of bindingsFor(command.id)) {
      const key = normalise(binding);
      const other = seen.get(key);
      if (other && other !== command.id) {
        conflicts.push(`${format(binding)} is bound to both ${other} and ${command.id}`);
      }
      seen.set(key, command.id);
    }
  }
  return conflicts;
}

/** The command a key event should run, if any. */
export function commandFor(event: KeyboardEvent): Command | undefined {
  const pressed = normaliseEvent(event);
  if (!pressed) return undefined;
  return commands().find(
    (command) => command.enabled && command.bindings.some((binding) => normalise(binding) === pressed),
  );
}

/** Every binding, in a platform's spelling: `Ctrl+Enter / F5`. */
export function formatAll(bindings: readonly string[], mac = isMac): string {
  return bindings.map((binding) => format(binding, mac)).join(" / ");
}

/**
 * How a binding is written for a platform: `⌘⇧P` on macOS, `Ctrl+Shift+P`
 * elsewhere. `mac` is this machine unless the caller says otherwise — the docs
 * generator renders both from one process.
 */
export function format(binding: string | undefined, mac = isMac): string {
  if (!binding) return "";
  const parts = binding.split("+").map((part) => part.trim().toLowerCase());
  const key = parts.pop() ?? "";
  const has = (name: string) => parts.includes(name);
  const mod = has("mod");
  const pieces: string[] = [];
  if (mac) {
    if (has("ctrl")) pieces.push("⌃");
    if (has("alt")) pieces.push("⌥");
    if (has("shift")) pieces.push("⇧");
    if (mod || has("cmd")) pieces.push("⌘");
    return `${pieces.join("")}${keyName(key, mac)}`;
  }
  if (mod || has("ctrl")) pieces.push("Ctrl");
  if (has("cmd")) pieces.push("Win");
  if (has("alt")) pieces.push("Alt");
  if (has("shift")) pieces.push("Shift");
  pieces.push(keyName(key, mac));
  return pieces.join("+");
}

/** The key part, as people write it. */
function keyName(key: string, mac: boolean): string {
  switch (key) {
    case "arrowleft":
      return mac ? "←" : "Left";
    case "arrowright":
      return mac ? "→" : "Right";
    case "arrowup":
      return mac ? "↑" : "Up";
    case "arrowdown":
      return mac ? "↓" : "Down";
    case "enter":
      return mac ? "↩" : "Enter";
    case "tab":
      return mac ? "⇥" : "Tab";
    case "escape":
      return "Esc";
    case ",":
      return ",";
    case ".":
      return ".";
    default:
      return key.length === 1 ? key.toUpperCase() : key.charAt(0).toUpperCase() + key.slice(1);
  }
}

/** A binding in a form two spellings of the same keys agree on. */
export function normalise(binding: string): string {
  const parts = binding
    .split("+")
    .map((part) => part.trim().toLowerCase())
    .filter((part) => part !== "");
  const key = parts.pop() ?? "";
  const mod = parts.includes("mod");
  const ctrl = parts.includes("ctrl") || (mod && !isMac);
  const meta = parts.includes("cmd") || parts.includes("meta") || (mod && isMac);
  const flags = [
    ctrl ? "ctrl" : "",
    meta ? "meta" : "",
    parts.includes("alt") || parts.includes("option") ? "alt" : "",
    parts.includes("shift") ? "shift" : "",
  ].filter(Boolean);
  return [...flags, keyOf(key)].join("+");
}

/** What a key event presses, in the same form as `normalise`. */
function normaliseEvent(event: KeyboardEvent): string | undefined {
  const key = keyOf(event.key);
  if (!key || ["control", "shift", "alt", "meta"].includes(key)) return undefined;
  const flags = [
    event.ctrlKey ? "ctrl" : "",
    event.metaKey ? "meta" : "",
    event.altKey ? "alt" : "",
    event.shiftKey ? "shift" : "",
  ].filter(Boolean);
  return [...flags, key].join("+");
}

/** One spelling for a key: lowercase, and the same for every layout quirk. */
function keyOf(key: string): string {
  const lower = key.toLowerCase();
  switch (lower) {
    case " ":
    case "spacebar":
      return "space";
    case "esc":
      return "escape";
    case "del":
      return "delete";
    default:
      return lower;
  }
}

/**
 * The registry as Markdown, for `docs/KEYBINDINGS.md` and for the palette's
 * "copy" command. Pure: the bindings are passed in, so the docs generator can
 * render them from Node, for either platform.
 */
export function asMarkdown(
  bindings: Record<string, string[]>,
  mac = isMac,
): string {
  const lines = [
    "# Keybindings",
    "",
    "Generated from the command registry (`ui/src/commands/catalog.js`) by",
    "`npm run docs:keys`. Don't edit this file; edit the catalog.",
    "",
    "`Mod` is Cmd on macOS and Ctrl everywhere else. Any of these can be",
    "changed in `<app-data>/keybindings.toml`:",
    "",
    "```toml",
    '"run.start" = "mod+shift+r"',
    '"tab.close" = ""            # unbound',
    "```",
    "",
  ];
  for (const category of CATEGORIES) {
    const rows = CATALOG.filter((command) => command.category === category);
    if (rows.length === 0) continue;
    lines.push(`## ${category}`, "", "| Command | Keys | Id |", "|---|---|---|");
    for (const command of rows) {
      const keys = bindings[command.id] ?? [];
      lines.push(`| ${command.title} | ${keys.length ? formatAll(keys, mac) : "—"} | \`${command.id}\` |`);
    }
    lines.push("");
  }
  return lines.join("\n");
}
