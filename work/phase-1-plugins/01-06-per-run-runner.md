# 01-06 · Per-run runner from manifest

**Phase:** 1 · **Size:** S · **Crate/area:** `tinkerpad-runner` · **Depends on:** 00-06, 01-01, 01-03, 01-04
**Design:** ARCHITECTURE.md §5.1, PLUGINS.md §2 (`[run]`)

## What

Replace the hard-coded `node <file>` from 00-06 with the plugin's `[run]` section:
substitute placeholders, apply env, resolve the program through toolchain overrides,
save the playground, and run it in the playground folder.

## Why

This is the moment the app becomes language-agnostic. After this item, nothing in
`tinkerpad-runner` knows what Node is.

## Expectations

- [ ] Run flow: save the playground (01-12 handles dirty state; for now always
      write), build `RunSpec` from `[run]` with placeholders substituted, spawn via
      `tinkerpad-process` with cwd = `{dir}`.
- [ ] Program resolution order: settings override → `PATH` (with `PATHEXT` on
      Windows). Not found → `RunEvent::Failed` carrying the `install_hint`.
- [ ] `{scratch}` resolves to `<app-data>/scratch/<plugin-id>/`, created on demand.
- [ ] Env from `[run].env` is layered over the process defaults from 00-04. Values
      support placeholders too.
- [ ] One run per playground at a time; runs in different tabs may overlap. The UI
      shows per-tab run state.
- [ ] Exit code and wall time reported. Non-zero exit is shown distinctly in the
      status bar.
- [ ] No run timeout exists, by default or otherwise. A silent process runs until
      it exits or the user cancels. (DEBUGGING.md D12.)
- [ ] `RunSpec` is built by one pure function from the manifest and playground
      and returned to the caller before spawn, so a later debug launch can add
      args and env to it. (DEBUGGING.md D13.) Leave a comment noting that `[run]`
      may later split into `[build]` + `[run]` (D14); don't design anything that
      assumes run is a single opaque step.
- [ ] Test with a fixture plugin whose `[run]` invokes the tiny echo binary from
      00-04's `tests/bin/` (no shell, no `cfg`) to prove placeholder substitution
      end to end on all three platforms in CI.

## Out of scope

Helper runners (03-03), Dump parsing (01-07; it plugs into the stdout stream this
item produces), stdin (04-05).

## Gotchas

Don't pass the source text to the runner. It runs the file on disk. That's what
makes per-run and helper runners interchangeable later.
