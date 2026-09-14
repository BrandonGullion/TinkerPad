# 01-14 · Dependencies: add package

**Phase:** 1 · **Size:** S · **Crate/area:** `tinkerpad-plugin`, `tinkerpad-app`, `ui/` · **Depends on:** 01-01, 01-06, 01-12
**Design:** PLUGINS.md §2 (`[deps]`)

## What

A minimal "add package" command that does what the plugin's `[deps]` says: either
runs a command (`cargo add`, `npm install`) in the playground folder, or inserts an
inline directive (`#:package Foo@1.0`) at the top of the main file.

## Why

A scratchpad without packages is a toy. The full UI can wait (04-01), but the
mechanism should exist so the manifest field is proven with three real plugins.

## Expectations

- [ ] `[deps] kind = "command"` with `add = [...]` (placeholders `{name}`,
      `{version}`; `{version}` may be empty → strip a trailing `@`).
- [ ] `[deps] kind = "inline-directive"` with `directive = "..."` inserted after
      any existing directives at the top of the file (after locked lines if the
      prelude is first), one per line, no duplicates by name.
- [ ] Command palette / `Mod+Shift+P` → "Add package…" prompts for `name` and
      optional `version`. Command output streams into the output pane like a run.
- [ ] Failure (network, unknown package) shows the tool's stderr; nothing is
      half-applied for the inline-directive kind.
- [ ] After adding, the next run picks the package up with no extra step in all
      three bundled plugins. Verify with `serde`, `zod`, and `Newtonsoft.Json`.

## Out of scope

Package search/autocomplete, version pickers, removal UI, lockfile handling.

## Gotchas

`npm install` needs a `package.json`; the TS plugin's `[scaffold]` should write a
minimal one (`{"type":"module","private":true}`) so this works on a fresh playground.
