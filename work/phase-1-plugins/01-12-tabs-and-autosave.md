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

- [ ] `Mod+N` creates a new playground in the *last used* language and default
      kind, names it `Untitled N`, opens it in a new tab, and focuses the editor.
      Zero prompts. `Mod+Shift+N` opens a small picker (language, then kind) with
      type-to-filter and Enter.
- [ ] The tab strip shows name, language icon or short code, a dirty dot, and a
      running indicator. Middle-click or `Mod+W` closes. `Ctrl+Tab` cycles (Ctrl on every platform, matching browsers).
- [ ] One `EditorView`; switching tabs swaps `EditorState` so undo history, cursor,
      and scroll are preserved per tab.
- [ ] Autosave: 500 ms after the last keystroke, and always before a run. Dirty dot
      clears on save. Save is atomic (01-04).
- [ ] Rename by double-clicking the tab or `F2`; renames the folder via 01-04.
- [ ] `SourceChangedOnDisk` (01-04): if the tab is clean, reload silently; if dirty,
      show a bar with "Reload" / "Keep mine".
- [ ] Session restore: open tabs, active tab, and per-tab scroll position are saved
      to `<app-data>/session.json` on change and restored at launch. A missing
      playground is skipped with a toast.
- [ ] Closing the last tab leaves an empty state with a hint: "Mod+N for a new
      playground, Mod+O to open one". `Mod+O` opens a quick-open list from 01-04's
      index (name + language, type-to-filter). This is the minimal library UI until
      04-01.
- [ ] Output pane is per tab (each tab keeps its last run's output).

## Out of scope

Search, tags, pinning (04-01). Split views. Drag to reorder tabs (nice, later).

## Gotchas

Keep tab state in one store on the UI side and treat the core as the source of
truth for playground metadata. The UI should never cache a playground's name; it
reads it from the library list events.
