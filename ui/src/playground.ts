// Opening a playground: what the core returns, and the notes shown about
// locked lines. Tabs (tabs.ts) do the opening; the helpers here keep the
// older one-editor call sites (the self-tests) working through them.

import type { Editor } from "./editor";
import type { StatusBar } from "./layout";
import type { Playground } from "./protocol";
import type { TabManager } from "./tabs";

/** Reply to `open_playground`. */
export interface OpenedPlayground {
  playground: Playground;
  /** The main file exactly as on disk. */
  text: string;
  prelude: string | null;
  postlude: string | null;
  lock: "none" | "locked" | "mismatch" | "unknownKind";
}

export const LOCKED_HINT = "Those lines are locked: they're part of this playground's kind.";
export const MISMATCH_NOTE =
  "Locked lines are off: the file no longer starts and ends with its kind's wrapper.";

let tabs: TabManager | null = null;

export function setTabManager(manager: TabManager): void {
  tabs = manager;
}

/** The id of the playground in the active tab, if any. */
export function currentPlayground(): string | null {
  return tabs?.activeId ?? null;
}

/** Open a playground in a tab and switch to it. */
export async function openPlayground(_editor: Editor, _status: StatusBar, id: string): Promise<OpenedPlayground> {
  if (!tabs) throw new Error("tabs aren't ready");
  return tabs.open(id);
}
