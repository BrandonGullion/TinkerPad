# 01-05 · Kinds, templates, scaffold, locked lines

**Phase:** 1 · **Size:** M · **Crate/area:** `tinkerpad-library`, `tinkerpad-plugin`, `ui/` · **Depends on:** 01-01, 01-04, 00-03
**Design:** PLUGINS.md §2 (`[[kind]]`, `[scaffold]`), §3

## What

When a playground is created, write the kind's template (with `{{name}}` filled)
as `main.<ext>` plus any `[scaffold].files`. If the kind has `prelude`/`postlude`,
they are part of the file, and the editor renders those lines locked and dimmed.

## Why

Kinds are how the app gives LINQPad's "expression / statements / program" feel
without any hidden code, which is what keeps the LSP honest (PLUGINS.md §3).

## Expectations

- [x] Template substitution supports `{{name}}` and `{{date}}` only. Anything else
      is left verbatim. Templates are read from the plugin folder at create time,
      not cached, so editing a bundled template during dev takes effect immediately.
- [x] `[scaffold].files` entries `{ path, template }` are written relative to the
      playground folder; nested dirs are created. Existing files are never
      overwritten (matters for re-scaffold later).
- [x] `playground.toml` records `kind`. The locked-line ranges are *derived* from
      the kind's `prelude`/`postlude` at open time by matching the file's head and
      tail, not stored. If the file no longer starts/ends with them (user edited
      outside the app), no lines are locked and a status-bar note says so.
- [x] Editor: locked ranges are read-only (CodeMirror `EditorState.changeFilter` or
      a read-only range extension), rendered with a dimmed background and a gutter
      marker. Cursor can enter them; typing there is rejected with a brief hint.
- [x] Select-all + delete leaves the locked lines. Undo history behaves.
- [x] Mod+A then typing replaces only the editable region.
- [x] Test: create each bundled kind, assert file contents, assert the derived
      locked ranges.

- [x] The file the runner and LSP see is byte-identical to the editor buffer
      after save. No transformation step exists between editor and disk, and
      none may be added later. (DEBUGGING.md D1: breakpoints map 1:1.)

## Out of scope

Switching a playground's kind after creation (add later if wanted). Snippets.

## Gotchas

CodeMirror positions are character offsets, not lines. Compute locked ranges as
offsets once at open and adjust them through the `changeFilter` so they stay right
as the user edits above/below.

## Results

Implemented 2026-09-15.

**Pieces**
- `tinkerpad-plugin` `scaffold.rs`:
  - `render` fills `{{name}}` and `{{date}}`.
  - `today_utc` gives the date with no date library.
  - `main_file` is prelude + rendered template + postlude.
  - `playground_files` returns the main file plus rendered `[scaffold].files`.
  - `locked` reports `None`, `Regions { head, tail }` (byte lengths), or
    `Mismatch`.
- `tinkerpad-library`: file writes during create never overwrite (the first entry
  for a path wins).
- `tinkerpad-app`: `create_playground` writes the real files. The new
  `open_playground` returns the text, and the prelude and postlude when they're
  locked, with a `lock` state of `none`, `locked`, `mismatch`, or `unknownKind`.
- UI `locked.ts`: a CodeMirror extension. A state field tracks the locked ranges
  as the middle grows and shrinks. A `changeFilter` drops edits inside them and
  refuses insertions at the document's very start or end, which the filter's
  ranges alone would let through. Mod+A selects the middle. Locked lines get a
  line decoration and a gutter marker. A refusal calls back to show a hint.
- UI `playground.ts`: `openPlayground` loads a playground into the editor with
  locking and sets the status bar's language and note. The status bar gained
  sticky and flash notes.
- `docs/PLUGINS.md` §3 now spells out how files are made and how locked lines are
  found.

**Why offsets are computed twice.** The Rust side counts bytes; CodeMirror counts
JavaScript string positions (UTF-16). So Rust only decides *whether* the file is
locked, and the editor measures the prelude and postlude itself.

**How `changeFilter` behaves at the edges.** Checked with a small Node script
against `@codemirror/state` before relying on it. Insertions exactly at the
middle's edges are allowed, which is what we want. Insertions at position 0 or at
the end of the document slip past the suppressed ranges, so the extension refuses
those explicitly.

**Tests**
- 3 unit tests in `scaffold.rs`: rendering only `{{name}}` and `{{date}}`, calendar
  dates including a leap day, and locked regions (match, prelude removed,
  postlude removed, CRLF, overlapping, prelude only).
- 4 integration tests in `tinkerpad-plugin/tests/scaffold.rs`, through the real
  library:
  - every kind of the four test plugins, checking file contents and locked
    regions on disk;
  - the expression kind, byte for byte, including the mismatch after an outside
    edit;
  - nested, rendered scaffold files;
  - templates re-read on every create.
- 1 new library test: create never overwrites and the first entry wins.
- In-app `locked` self-test (Windows release build), 16 checks, all passing:

  | Check | Result |
  |---|---|
  | Template renders `{{name}}` and `{{date}}` | pass |
  | Plain kind has no locked lines | pass |
  | Expression file is prelude + template + postlude | pass |
  | Locked lines dimmed and marked (3 lines, 3 markers) | pass |
  | Cursor can enter a locked line | pass |
  | Typing in a locked line is refused, with a hint | pass |
  | Typing in the middle works | pass |
  | Mod+A selects only the middle; typing replaces just that | pass |
  | Select-all + delete leaves the locked lines | pass |
  | Undo restores the middle | pass |
  | File on disk is byte-identical to the editor after saving | pass |
  | File without its wrapper opens unlocked, and the status bar says why | pass |

  CI runs it on macOS and Linux. A screenshot confirmed the look: locked lines
  dimmed with a lock in the gutter, and the hint in the status bar.

## Checklist status

**Ticked with a note**
- *Test: create each bundled kind.* No plugins are bundled yet (they arrive in
  01-09 to 01-11). The test covers every kind of the four test plugins instead,
  and its comment says to add the bundled plugins when they exist.
- *`{{date}}`.* The date is in UTC, which avoids a time-zone library. Just after
  midnight, local and UTC dates can differ.
- *Status-bar note when the file no longer matches.* Shown by `openPlayground`.
  Tabs (01-12) will call it when opening playgrounds; today only the self-test
  opens one.
- *`[scaffold].files` rendering.* Scaffold templates get `{{name}}` and `{{date}}`
  only. The Rust plugin (01-11) will need more, such as the folder slug and
  absolute paths, and will extend this.
