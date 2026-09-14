# 01-04 · Playground library on disk

**Phase:** 1 · **Size:** M · **Crate/area:** `tinkerpad-library`, `tinkerpad-core` · **Depends on:** 01-02
**Design:** PLAN.md §3.5, ARCHITECTURE.md §6

## What

The folder-per-playground store: create, open, rename, delete, list, and read/write
the main file. `playground.toml` is the metadata. An in-memory index is built at
startup and kept fresh by a file watcher.

## Why

Plain folders are the storage decision. Making them the source of truth from the
start means no migration later and means users can `git init` their library.

## Expectations

- [ ] Library root defaults to `~/TinkerPad/`, overridable in settings. Created on
      first launch with a `README.md` explaining the layout.
- [ ] `Playground` in `tinkerpad-core` per ARCHITECTURE.md §3, with a ULID `id`.
- [ ] `create(name, language, kind) -> Playground` makes the folder, writes
      `playground.toml` and the scaffold files (01-05). Name collision appends `-2`,
      `-3`. Folder name is the slugified name; the display name keeps its case and
      spaces.
- [ ] `rename` renames the folder and updates the index; `id` never changes.
- [ ] `delete` moves the folder to the OS recycle bin (`trash` crate), never a hard
      delete.
- [ ] `list()` returns playgrounds sorted by last modified; each carries name,
      language, kind, tags, modified time, and the first non-empty code line as a
      preview.
- [ ] `read_source(id)` / `write_source(id, text)`; writes are atomic (write to
      temp, rename).
- [ ] `notify` watcher on the root: a folder added or removed outside the app shows
      up within a second; edits to `main.*` outside the app while it's open in a tab
      raise a `SourceChangedOnDisk` event (the UI handles it in 01-12).
- [ ] Folders without a `playground.toml` are ignored silently. A malformed
      `playground.toml` is logged and the folder shown with a warning badge.
- [ ] Tests run against a temp dir: create/list/rename/delete round-trip; watcher
      picks up an external create.

## Out of scope

Search, tags UI, recents (04-01). Templates content (01-05). Multi-file playgrounds.

## Gotchas

`notify` on Windows can fire several events for one save. Debounce 100 ms and
coalesce by path. macOS FSEvents coalesces on its own but can arrive late (up to
a second); Linux inotify has a per-user watch limit, so watch the root
recursively with one watcher rather than one per playground. Run the watcher test
in CI on all three.
