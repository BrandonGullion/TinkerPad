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

- [x] Run flow: save the playground (01-12 handles dirty state; for now always
      write), build `RunSpec` from `[run]` with placeholders substituted, spawn via
      `tinkerpad-process` with cwd = `{dir}`.
- [x] Program resolution order: settings override → `PATH` (with `PATHEXT` on
      Windows). Not found → `RunEvent::Failed` carrying the `install_hint`.
- [x] `{scratch}` resolves to `<app-data>/scratch/<plugin-id>/`, created on demand.
- [x] Env from `[run].env` is layered over the process defaults from 00-04. Values
      support placeholders too.
- [ ] One run per playground at a time; runs in different tabs may overlap. The UI
      shows per-tab run state.
- [x] Exit code and wall time reported. Non-zero exit is shown distinctly in the
      status bar.
- [x] No run timeout exists, by default or otherwise. A silent process runs until
      it exits or the user cancels. (DEBUGGING.md D12.)
- [x] `RunSpec` is built by one pure function from the manifest and playground
      and returned to the caller before spawn, so a later debug launch can add
      args and env to it. (DEBUGGING.md D13.) Leave a comment noting that `[run]`
      may later split into `[build]` + `[run]` (D14); don't design anything that
      assumes run is a single opaque step.
- [x] Test with a fixture plugin whose `[run]` invokes the tiny echo binary from
      00-04's `tests/bin/` (no shell, no `cfg`) to prove placeholder substitution
      end to end on all three platforms in CI.

## Out of scope

Helper runners (03-03), Dump parsing (01-07; it plugs into the stdout stream this
item produces), stdin (04-05).

## Gotchas

Don't pass the source text to the runner. It runs the file on disk. That's what
makes per-run and helper runners interchangeable later.

## Results

Implemented 2026-09-15.

**Pieces**
- `tinkerpad-runner` `spec.rs`: `run_spec(plugin, playground, ctx)` is a pure
  function that returns a `RunSpec`:
  - fills all eight placeholders in the command, args, cwd, and env;
  - resolves the program (settings override, then `PATH`) or returns `NotFound`
    with the plugin's install hint;
  - sets the child's `PATH` to the discovered one, then layers `[run].env` on top;
  - defaults the working directory to the playground folder.

  Its doc comment notes the possible future `[build]` + `[run]` split
  (DEBUGGING.md D14). There's no timeout anywhere.
- `tinkerpad-app` `runs.rs`: a run slot per playground.
  - Starting a run replaces that playground's previous run, waiting for its final
    event first.
  - It saves the text, waits up to 10 s if the toolchain check is still running,
    builds the spec, and creates `<app-data>/scratch/<plugin-id>/`.
  - Then it runs the file and streams events.
  - A missing toolchain or program becomes a `failed` event with the reason and
    install hint.
  - The hard-coded Node run is gone; nothing in `tinkerpad-runner` or `runs.rs`
    knows about any language.
- Commands `run` and `cancel` now take a playground id.
- UI: the run controller takes the open playground and cancels that playground's
  run. At startup the app opens the most recently modified playground whose
  plugin is loaded. With none open, the editor explains why and Mod+Enter shows
  "Open a playground to run it."

**Tests**
- 5 integration tests in `tinkerpad-runner/tests/spec.rs` use
  `tinkerpad-echoargs`, a small binary that prints its arguments, working
  directory, and chosen environment variables:
  - all eight placeholders plus a literal `{{…}}` escape, end to end through a
    real process;
  - `{toolchain}` as the command, with `{scratch}` as the working directory;
  - a settings override choosing the program;
  - a missing program carrying the install hint;
  - the spec being plain data a debug launch can extend.
- In-app `run` self-test (Windows release build), now through the node-check
  test plugin's `[run]`. All earlier checks still pass (exit codes, stderr, UTF-8,
  cancel, run-while-running), plus four new ones:
  - the run saves the playground first;
  - runs in two playgrounds overlap;
  - cancel stops only that playground's run;
  - a plugin with a missing toolchain fails with "Mini isn't ready: `mini` was
    not found."

  Mod+Enter to first output was 42 ms. All six scenarios (run, layout, locked,
  library, toolchains, plugins) pass together; CI runs them on macOS and Linux.
- A screenshot confirmed normal startup reopens the most recent playground.

## Checklist status

**Ticked with a note**
- *Test with the echo binary from 00-04's `tests/bin/`.* That binary belongs to
  `tinkerpad-process`, and Cargo only exposes a test binary's path to its own
  crate. `tinkerpad-runner` has its own small `tinkerpad-echoargs` for the same
  purpose.
- *Non-zero exit shown distinctly.* Already true since 00-06 and still checked
  by the `run` self-test.

**Open**
- *The UI shows per-tab run state.* The core keeps a separate run per playground
  and the self-test proves two can overlap. The UI still has one editor and
  follows only the latest run until tabs arrive (01-12).

**Consequence to be aware of.** Removing the hard-coded Node run means the app
can only run a playground whose plugin is loaded. Until bundled plugins (01-09 to
01-11) and the new-playground UI (01-12) exist, running code by hand needs a
user plugin and an existing playground. The node-check test plugin works for that.
