// Opening a playground in the editor.
//
// Tabs (work item 01-12) will build on this; for now there is one editor and
// the self-tests are the only caller.

import type { Editor } from "./editor";
import { invoke } from "./ipc";
import type { StatusBar } from "./layout";
import { findLocked, lockedLinesExtension } from "./locked";
import type { Listing, Playground, PluginsStatus } from "./protocol";

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

let current: string | null = null;

/** The id of the playground in the editor, if any. */
export function currentPlayground(): string | null {
  return current;
}

/**
 * Load a playground into the editor, locking its prelude and postlude when
 * the file still has them.
 */
export async function openPlayground(
  editor: Editor,
  status: StatusBar,
  id: string,
): Promise<OpenedPlayground> {
  const opened = await invoke<OpenedPlayground>("open_playground", { id });
  const ranges =
    opened.lock === "locked"
      ? findLocked(opened.text, opened.prelude ?? "", opened.postlude ?? "")
      : null;

  const extension = lockedLinesExtension(ranges, () => status.flashNote(LOCKED_HINT));
  editor.setState(editor.createState(opened.text, extension));
  current = opened.playground.id;

  status.setLanguage(`${opened.playground.language} · ${opened.playground.kind}`);
  status.setNote(
    opened.lock === "mismatch" || (opened.lock === "locked" && !ranges) ? MISMATCH_NOTE : null,
  );
  return opened;
}

/**
 * Open the most recently modified playground whose plugin is loaded, if there
 * is one. Stands in for session restore until tabs arrive (01-12).
 */
export async function openMostRecent(editor: Editor, status: StatusBar): Promise<boolean> {
  const [listing, plugins] = await Promise.all([
    invoke<Listing>("list_playgrounds"),
    invoke<PluginsStatus>("list_plugins"),
  ]);
  const loaded = new Set(plugins.plugins.map((p) => p.id));
  const recent = listing.playgrounds.find((p) => loaded.has(p.language));
  if (!recent) return false;
  await openPlayground(editor, status, recent.id);
  return true;
}
