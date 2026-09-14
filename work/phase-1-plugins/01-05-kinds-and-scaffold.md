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

- [ ] Template substitution supports `{{name}}` and `{{date}}` only. Anything else
      is left verbatim. Templates are read from the plugin folder at create time,
      not cached, so editing a bundled template during dev takes effect immediately.
- [ ] `[scaffold].files` entries `{ path, template }` are written relative to the
      playground folder; nested dirs are created. Existing files are never
      overwritten (matters for re-scaffold later).
- [ ] `playground.toml` records `kind`. The locked-line ranges are *derived* from
      the kind's `prelude`/`postlude` at open time by matching the file's head and
      tail, not stored. If the file no longer starts/ends with them (user edited
      outside the app), no lines are locked and a status-bar note says so.
- [ ] Editor: locked ranges are read-only (CodeMirror `EditorState.changeFilter` or
      a read-only range extension), rendered with a dimmed background and a gutter
      marker. Cursor can enter them; typing there is rejected with a brief hint.
- [ ] Select-all + delete leaves the locked lines. Undo history behaves.
- [ ] Mod+A then typing replaces only the editable region.
- [ ] Test: create each bundled kind, assert file contents, assert the derived
      locked ranges.

- [ ] The file the runner and LSP see is byte-identical to the editor buffer
      after save. No transformation step exists between editor and disk, and
      none may be added later. (DEBUGGING.md D1: breakpoints map 1:1.)

## Out of scope

Switching a playground's kind after creation (add later if wanted). Snippets.

## Gotchas

CodeMirror positions are character offsets, not lines. Compute locked ranges as
offsets once at open and adjust them through the `changeFilter` so they stay right
as the user edits above/below.
