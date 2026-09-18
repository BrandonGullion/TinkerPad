// The command palette (Mod+Shift+P): the actions that aren't on a key you'd
// guess, in one searchable list. Work item 04-01 grows it; for now it exists
// so "Add package…" has a home.

import { invoke } from "./ipc";
import { pick } from "./picker";
import type { RunController } from "./run";
import type { TabManager } from "./tabs";
import { toast } from "./toast";

const isMac = navigator.platform.toLowerCase().includes("mac");
const MOD = isMac ? "⌘" : "Ctrl+";

export interface Command {
  id: string;
  label: string;
  /** The shortcut, if it has one. */
  shortcut?: string;
  run: () => void | Promise<void>;
}

/** Every command, in the order they're offered. */
export function commands(tabs: TabManager, runs: RunController): Command[] {
  const text = () => tabs.activeTab ? tabs.textOf(tabs.activeTab) : "";
  return [
    { id: "add-package", label: "Add package…", run: () => tabs.addPackage() },
    { id: "run", label: "Run", shortcut: `${MOD}Enter`, run: () => runs.run(tabs.activeId, text()) },
    { id: "cancel", label: "Cancel run", shortcut: `${MOD}.`, run: () => runs.cancel() },
    { id: "clear-output", label: "Clear output", shortcut: `${MOD}L`, run: () => tabs.activeOutput.clear() },
    { id: "new", label: "New playground", shortcut: `${MOD}N`, run: () => tabs.newPlayground() },
    { id: "new-pick", label: "New playground…", shortcut: `${MOD}⇧N`, run: () => tabs.pickAndCreate() },
    { id: "open", label: "Open playground…", shortcut: `${MOD}O`, run: () => tabs.quickOpen() },
    { id: "rename", label: "Rename playground", shortcut: "F2", run: () => tabs.startRename() },
    { id: "close", label: "Close tab", shortcut: `${MOD}W`, run: () => tabs.closeActive() },
    { id: "force-per-run", label: "Force per-run execution", run: () => forcePerRun() },
    { id: "next-error", label: "Next error or warning", shortcut: "F8", run: () => tabs.cycleAnnotation(1) },
    { id: "previous-error", label: "Previous error or warning", shortcut: "⇧F8", run: () => tabs.cycleAnnotation(-1) },
  ];
}

/**
 * Keep runs off plugin helpers for the rest of the session, or let them back
 * on (work item 03-03). It's a toggle, so it says which way it went.
 */
async function forcePerRun(): Promise<void> {
  const now = await invoke<boolean>("force_per_run");
  const forced = await invoke<boolean>("force_per_run", { forced: !now });
  toast(
    "info",
    forced
      ? "Runs will start a new process each time, ignoring plugin helpers."
      : "Runs will use plugin helpers again.",
  );
}

/** Show the palette and run what was chosen. */
export async function openCommandPalette(tabs: TabManager, runs: RunController): Promise<void> {
  const list = commands(tabs, runs);
  const choice = await pick(
    "Commands",
    list.map((c) => ({ id: c.id, label: c.label, detail: c.shortcut })),
    "Type a command",
  );
  if (choice) await list.find((c) => c.id === choice.id)?.run();
}
