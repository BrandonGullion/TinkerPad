// The command palette (work item 04-03).
//
// Everything the window can do, in one list, with its category and its keys.
// What you used last is at the top, because the next thing you want is usually
// the last thing you wanted.

import { pick } from "../picker";
import { commands, format, runCommand } from "./registry";

/** Ids in the order they were last used, most recent first. */
const RECENT_KEY = "tinkerpad.recentCommands";
const RECENT_KEPT = 8;

function recent(): string[] {
  try {
    const raw = localStorage.getItem(RECENT_KEY);
    const list: unknown = raw ? JSON.parse(raw) : [];
    return Array.isArray(list) ? list.filter((id): id is string => typeof id === "string") : [];
  } catch {
    // No storage, or nonsense in it: an empty history costs nothing.
    return [];
  }
}

function remember(id: string): void {
  const list = [id, ...recent().filter((known) => known !== id)].slice(0, RECENT_KEPT);
  try {
    localStorage.setItem(RECENT_KEY, JSON.stringify(list));
  } catch {
    // The palette still works; it just won't remember across restarts.
  }
}

/** Show the palette and run what was chosen. */
export async function openCommandPalette(): Promise<void> {
  const lately = recent();
  const available = commands().filter((command) => !command.hidden && command.enabled);
  const rank = (id: string) => {
    const at = lately.indexOf(id);
    return at === -1 ? lately.length : at;
  };
  const ordered = [...available].sort((a, b) => rank(a.id) - rank(b.id));

  const choice = await pick(
    "Commands",
    ordered.map((command) => ({
      id: command.id,
      label: command.title,
      // The category is matched by the filter too, so "run" finds the run
      // commands whatever they are called.
      detail: [command.category, format(command.binding)].filter(Boolean).join(" · "),
    })),
    "Type a command",
  );
  if (!choice) return;
  remember(choice.id);
  await runCommand(choice.id);
}
