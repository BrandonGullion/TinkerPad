# 01-12 · Tabs, new-tab flow, autosave

**Phase:** 1 · **Size:** M · **Crate/area:** `ui/`, `tinkerpad-app` · **Depends on:** 01-04, 01-05, 00-03
**Design:** ARCHITECTURE.md §7, PLAN.md §1 (ease of use)

## What

Multiple open playgrounds as tabs over a single editor view. A new-tab flow that
takes one keystroke and no dialog. Autosave on a debounce and on run. Reopen the
last session's tabs at launch.

## Why

"Easy to use" lives here. LINQPad's magic is that a new query is instant and never
asks a question. Get this flow right and the app feels finished long before it is.

## Expectations

- [x] `Mod+N` creates a new playground in the *last used* language and default
      kind, names it `Untitled N`, opens it in a new tab, and focuses the editor.
      Zero prompts. `Mod+Shift+N` opens a small picker (language, then kind) with
      type-to-filter and Enter.
- [x] The tab strip shows name, language icon or short code, a dirty dot, and a
      running indicator. Middle-click or `Mod+W` closes. `Ctrl+Tab` cycles (Ctrl on every platform, matching browsers).
- [x] One `EditorView`; switching tabs swaps `EditorState` so undo history, cursor,
      and scroll are preserved per tab.
- [x] Autosave: 500 ms after the last keystroke, and always before a run. Dirty dot
      clears on save. Save is atomic (01-04).
- [x] Rename by double-clicking the tab or `F2`; renames the folder via 01-04.
- [x] `SourceChangedOnDisk` (01-04): if the tab is clean, reload silently; if dirty,
      show a bar with "Reload" / "Keep mine".
- [x] Session restore: open tabs, active tab, and per-tab scroll position are saved
      to `<app-data>/session.json` on change and restored at launch. A missing
      playground is skipped with a toast.
- [x] Closing the last tab leaves an empty state with a hint: "Mod+N for a new
      playground, Mod+O to open one". `Mod+O` opens a quick-open list from 01-04's
      index (name + language, type-to-filter). This is the minimal library UI until
      04-01.
- [x] Output pane is per tab (each tab keeps its last run's output).

## Out of scope

Search, tags, pinning (04-01). Split views. Drag to reorder tabs (nice, later).

## Gotchas

Keep tab state in one store on the UI side and treat the core as the source of
truth for playground metadata. The UI should never cache a playground's name; it
reads it from the library list events.

## Results

Implemented 2026-09-15.

**UI** (`ui/src/`)
- `tabs.ts`, the `TabManager`, is the one store for tab state.
  - Each tab has an `EditorState`, a scroll position, an `OutputPane`, the text
    last saved, a dirty flag, a conflict flag, and a pending autosave. Switching
    swaps the state into the single `EditorView`.
  - Names, languages, and kinds are always read from the library listing, which
    refreshes on `libraryChanged` and after a create or rename.
  - Autosave writes 500 ms after the last edit, and a run marks the text as
    saved, because the run command writes it.
  - `sourceChanged` reloads a clean tab. A dirty tab gets a bar with **Reload**
    and **Keep mine**, and autosave pauses until the user chooses.
  - When a playground disappears from the library, its tab closes with a toast.
  - The session is saved 400 ms after any change (open, close, switch, scroll)
    and restored at launch. Missing playgrounds are skipped with one toast.
- The tab strip shows a language badge (the plugin name if it's three
  characters or fewer, otherwise the extension), the name, a dirty dot, a
  pulsing running dot, and a close button (×). A dirty tab shows the dot in
  place of × until hovered. A **+** button makes a new playground.
- `picker.ts`: a keyboard-first filter list, used by `Mod+Shift+N` (language,
  then kind, skipping the kind step when there's only one) and by `Mod+O`
  (quick open, showing name, language, and preview).
- `run.ts`: `RunController` tracks runs per playground. It maps run ids to
  playgrounds, buffers events that arrive before the id is known, and sends
  output and state to that playground's tab. Runs in different tabs overlap, and
  the status bar follows the active tab.
- The empty state reads "Ctrl+N for a new playground, Ctrl+O to open one", with
  ⌘N and ⌘O on macOS. It shows its own empty output pane.
- `main.ts`: global shortcuts are handled before CodeMirror. On window close,
  unsaved tabs and the session are written.
- A `[hidden]` rule now hides elements even when their class sets `display`.
  Screenshots caught the conflict bar and empty state showing while hidden.

**Core** (`tinkerpad-app`, `tinkerpad-library`)
- `load_session` and `save_session` read and write `<app-data>/session.json`
  (`session.rs`): open tabs with scroll, the active tab, and the last language.
  - A missing or corrupt file is an empty session. The write is atomic.
  - `TINKERPAD_SESSION` overrides the path, and self-test runs default to a temp
    file, so tests never touch the real session.
- `list_plugins` now includes each plugin's kinds, for the picker.
- `Library::write_source` skips a write when the file already holds that text,
  and returns whether it wrote. A file's modification time now changes only when
  its content does. That fixed the rebuild-on-every-run cost noted in 01-10 and
  01-11:

  | Rerun, unchanged | Before | Now |
  |---|---|---|
  | C# | 1,931 ms | 176 ms |
  | Rust | 422 ms | 77 ms |

- `selftest_touch_source`, available only while self-testing, writes a
  playground's file behind the library's back, so the real watcher path can be
  tested.
- ARCHITECTURE.md §7 now describes the single view, per-tab state, saving,
  conflicts, the session, and the shortcuts. It previously described one
  CodeMirror instance per tab.

**Tests**
- Rust:
  - session round trip, overwrite, and missing, corrupt, or partial files;
  - `write_source` leaves identical text and the modification time alone.
- New in-app `tabs` scenario, 19 checks:
  - closing the last tab shows the hint;
  - `Mod+N` makes "Untitled 1" in node-check with its default kind, no picker,
    and the editor focused;
  - the tab shows its name and badge;
  - the dirty dot appears, and autosave lands 506 ms after the edit and clears
    it, with the file matching;
  - switching tabs keeps text, cursor (line 150), and scroll (1500 px);
  - undo still works after switching;
  - `Ctrl+Tab` and `Ctrl+Shift+Tab` cycle;
  - output and run state stay per tab: a running tab keeps its output and its
    running dot while another tab runs, and the dot clears after cancel;
  - `F2` renames the playground and its folder;
  - double-click starts a rename, and Escape cancels it;
  - a clean tab reloads on a real external change;
  - a dirty tab shows the bar, and Keep mine saves;
  - Reload takes the file;
  - `Mod+Shift+N` picks node-check, then Expression;
  - `Mod+W` and middle-click close;
  - `session.json` matches the tabs, the active tab, and the last language;
  - `Mod+O` opens by name from the empty state;
  - restoring reopens two tabs, skips a missing one, and shows a toast.
- New `session-prepare` and `session-restore` scenarios, run as two real
  launches. The second launch reopened "Session A" and "Session B" with the
  active tab and its 1,200 px scroll. CI runs both on macOS and Linux.
- Existing scenarios now read the active tab's output pane wherever they switch
  tabs. The locked-lines scenario now saves through the tab and uses a real
  external edit.
- All 141 checks in 12 scenarios pass on Windows (release build).
- Screenshots showed the tab strip with running and dirty dots, the conflict
  bar, the quick-open picker, the empty state, and a restored session.

## Checklist status

**Ticked with a note**
- *Keyboard shortcuts.* `Mod+N`, `Mod+Shift+N`, `Mod+O`, `Mod+W`, `Ctrl+Tab`,
  and `F2` were tested by dispatching key events through the app's real document
  listeners. They were not pressed on a physical keyboard, because tests don't
  send OS keystrokes. The webview could claim some of these before the page sees
  them; WebView2's browser accelerators are the main suspect for `Ctrl+N` and
  `Ctrl+W`. That is worth a manual check on each OS.
- *Language icon or short code.* A short code.
- *Save is atomic (01-04).* Unchanged: `write_source` still writes atomically,
  and now also skips identical text.
- *Closing a tab while it runs.* Not specified. Closing cancels the run, since
  its output would have nowhere to go.
