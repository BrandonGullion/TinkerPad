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

- [x] Library root defaults to `~/TinkerPad/`, overridable in settings. Created on
      first launch with a `README.md` explaining the layout.
- [x] `Playground` in `tinkerpad-core` per ARCHITECTURE.md §3, with a ULID `id`.
- [x] `create(name, language, kind) -> Playground` makes the folder, writes
      `playground.toml` and the scaffold files (01-05). Name collision appends `-2`,
      `-3`. Folder name is the slugified name; the display name keeps its case and
      spaces.
- [x] `rename` renames the folder and updates the index; `id` never changes.
- [x] `delete` moves the folder to the OS recycle bin (`trash` crate), never a hard
      delete.
- [x] `list()` returns playgrounds sorted by last modified; each carries name,
      language, kind, tags, modified time, and the first non-empty code line as a
      preview.
- [x] `read_source(id)` / `write_source(id, text)`; writes are atomic (write to
      temp, rename).
- [x] `notify` watcher on the root: a folder added or removed outside the app shows
      up within a second; edits to `main.*` outside the app while it's open in a tab
      raise a `SourceChangedOnDisk` event (the UI handles it in 01-12).
- [ ] Folders without a `playground.toml` are ignored silently. A malformed
      `playground.toml` is logged and the folder shown with a warning badge.
- [x] Tests run against a temp dir: create/list/rename/delete round-trip; watcher
      picks up an external create.

## Out of scope

Search, tags UI, recents (04-01). Templates content (01-05). Multi-file playgrounds.

## Gotchas

`notify` on Windows can fire several events for one save. Debounce 100 ms and
coalesce by path. macOS FSEvents coalesces on its own but can arrive late (up to
a second); Linux inotify has a per-user watch limit, so watch the root
recursively with one watcher rather than one per playground. Run the watcher test
in CI on all three.

## Results

Implemented 2026-09-14.

**Pieces**
- `tinkerpad-core` `playground.rs`: `PlaygroundId` (ULID text), `KindId`,
  `Playground`, `PlaygroundSummary`, `BrokenPlayground`. Events
  `libraryChanged` and `sourceChangedOnDisk`.
- `tinkerpad-library`:
  - `meta.rs` reads and writes `playground.toml`. It adds `name` (the display
    name) and `main` (the main file's path) to the format in ARCHITECTURE.md §6,
    so the library never needs plugin knowledge.
  - `slug.rs` makes folder names: letters and digits in any script, hyphens for
    the rest, 60 characters at most, Windows reserved names avoided, and `-2`,
    `-3` for collisions.
  - `library.rs` holds the index, create, rename, delete, list, read, write, and
    the watcher.
  - `platform.rs` sends folders to the trash, using `NSFileManager` on macOS.
- `tinkerpad-app`: `library.rs` picks the root (`TINKERPAD_LIBRARY`, then
  `[library].root` in settings, then `~/TinkerPad`), falls back to
  `<app-data>/library` with a toast if the root can't be created, and forwards
  watcher events. Commands: `list_playgrounds`, `create_playground`,
  `rename_playground`, `delete_playground`, `read_source`, `write_source`.

**How the watcher tells our saves from someone else's.** The library stores a
hash of the main file's contents whenever it writes or creates one. When a file
event arrives, the file is hashed again: a match is our own save, a difference is
an outside edit. This avoids relying on timestamps, which some filesystems store
to the second.

**Tests.** 13 in `tinkerpad-library` (3 unit, 10 integration against a temp
folder):
- Open creates the root and README.
- Create writes metadata and the main file; given files, nested ones included,
  are written; a failed create leaves nothing behind.
- Name collisions get `-2`, `-3`.
- List is newest first, with previews cut at 120 characters.
- Rename moves the folder, keeps the id, numbers collisions, and survives a
  rescan.
- 20 atomic writes leave no temp files.
- Non-playground folders are ignored; malformed metadata and duplicated ids are
  reported as broken.
- Watcher: an outside create, an outside edit, and an outside delete are all
  seen. The app's own create, rename, and save produce no events.
- Delete moves to the real trash (only when `TINKERPAD_TEST_TRASH=1`, which CI
  sets).

The watcher test passed 8 runs in a row locally. Its first CI run on Windows still
caught a race: on a slow disk the watcher could compare a folder with the index
after our files were written but before the index was updated, and report a
spurious `Changed`. Create, rename, and delete now hold the index lock for the
whole operation, and the watcher takes the same lock before comparing, so it only
ever sees the finished state. It uses 10 s timeouts for slow
CI machines and FSEvents latency; locally it finishes in about 3 s.

**In the app** (`library` self-test, Windows release build, throwaway library
folder): create makes `self-test` and `self-test-2`; an unknown language is
rejected with `invalidInput`; write and read round-trip; list shows the preview
with the newest playground first; rename moves the folder to `renamed-test` and
keeps the id and source; no `sourceChangedOnDisk` for the app's own writes. CI
runs this plus `library-trash` on macOS and Linux.

## Checklist status

**Ticked with a note**
- *`delete` moves the folder to the recycle bin.* Verified in CI, where
  `TINKERPAD_TEST_TRASH=1` lets the test use the real trash: the Rust test passed
  on Windows, macOS, and Linux, and the in-app `library-trash` self-test on macOS
  and Linux. Locally the test skips itself so it doesn't fill the recycle bin.
- *`Playground` per ARCHITECTURE.md §3.* `created` is `created_ms` (milliseconds
  since the epoch) rather than a `DateTime`, because `tinkerpad-core` has no date
  library. `playground.toml` still stores an RFC 3339 datetime.
- *`create` writes the scaffold files (01-05).* `create` takes a list of files to
  write. The app passes none yet, so the main file starts empty; 01-05 fills in
  templates and scaffold files.
- *Library root overridable in settings.* `[library].root`, plus a
  `TINKERPAD_LIBRARY` environment variable for tests. The default `~/TinkerPad`
  comes from Tauri's home folder lookup; tests and self-tests always set an
  override, so the default location itself hasn't been exercised.
- *Edits while open in a tab raise `SourceChangedOnDisk`.* The event is raised
  for any outside edit to a main file; the UI decides which tabs care (01-12).

**Open**
- *Malformed `playground.toml` shown with a warning badge.* It is logged and
  returned in `list_playgrounds` under `broken`, but no library UI exists to show
  a badge until 01-12 and 04-01.
