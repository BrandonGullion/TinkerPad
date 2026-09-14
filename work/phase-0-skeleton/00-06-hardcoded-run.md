# 00-06 · Hard-coded run, end to end, with cancel

**Phase:** 0 · **Size:** S · **Crate/area:** `tinkerpad-runner`, `tinkerpad-app`, `ui/` · **Depends on:** 00-03, 00-04, 00-05
**Design:** PLAN.md §5 Phase 0, ARCHITECTURE.md §5.1

## What

Mod+Enter writes the editor buffer to a temp file and runs a hard-coded `node
<file>` through `tinkerpad-process`, streaming output into the pane. A Cancel button
(and `Mod+.`) kills it. This is the whole app, in miniature, with no plugins.

## Why

It closes the loop. Everything from here on replaces a hard-coded piece with a
configurable one. It also produces the first real latency number for "Mod+Enter to
first byte".

## Expectations

- [ ] `tinkerpad-runner` exposes `run(RunSpec) -> RunHandle` producing `RunEvent`s
      (`Started`, `Stdout`, `Stderr`, `Exited { code, duration }`, `Failed`), built on
      `tinkerpad-process`. `RunSpec` is program/args/cwd/env for now.
- [ ] `run` and `cancel` Tauri commands; `RunEvent`s flow to the UI via 00-05.
- [ ] Typing `console.log(1)` and pressing Mod+Enter shows `1` in the output pane
      with exit code 0 and elapsed time in the status bar.
- [ ] `while(true){}` can be cancelled; the process is gone (check Task Manager);
      the UI shows "cancelled" rather than an exit code.
- [ ] Pressing Run while a run is active cancels the previous run first, then starts
      the new one. No interleaved output.
- [ ] Output pane is cleared at the start of each run (with a setting stub to keep
      it, defaulting to clear).
- [ ] Measured and recorded here: Mod+Enter to first output byte for
      `console.log(1)`. Expect ~40–80 ms of which Node startup is most.
- [ ] Node not installed → a clear error in the output pane, not a panic.

## Out of scope

Plugins, playground files, Dump. The temp file location is fine for now.

## Gotchas

Elapsed time in the status bar should update at ~10 Hz from the UI's own clock,
not from events, so it stays smooth during a silent long run.

## Results

Implemented 2026-09-14.

**Pieces**
- `tinkerpad-runner`: `RunSpec` (plain data, no timeout), `run()` streaming
  `RunEvent`s, and `Utf8Stream`, an incremental decoder so characters split
  across chunks aren't mangled.
- `tinkerpad-app` `runs.rs`: writes the buffer to `<app-data>/scratch/scratch.js`,
  runs `node`, publishes events scoped to the run id. Starting a run cancels the
  active one and waits for its final event before spawning, so output never
  interleaves.
- UI `run.ts`: buffers events that arrive before the `run` command returns,
  ignores superseded runs, drives the status bar and output pane. Cancel button
  and `Mod+.`.
- The app now configures the macOS pid registry and sweeps orphans at startup
  (00-04's force-quit path).

**Timing**, Windows release build, `console.log(1)`:

| Measurement | Time |
|---|---|
| Mod+Enter to first output, first run | 34.6 ms |
| Mod+Enter to first output, second run | 35.2 ms |
| Runner alone, spawn to first byte (`cargo test`) | 36.8 ms |

Nearly all of it is Node startup, as expected.

**Self-test `run` scenario** (in-app, through the real keymap), all pass:
`console.log(1)` prints `1` with exit 0 and the status bar says so; stderr is
shown and exit code 2 is reported; UTF-8 survives; the Cancel button appears
while running; `Mod+.` cancels `while (true) {}`; running again while a
`setInterval` run is active replaces it with no stray output. CI runs this on
macOS and Linux.

**Process cleanup:** `tinkerpad-runner` tests assert the cancelled `node`
process is gone at the OS level.

**Not verified in the app:** the "Node not installed" message. The runner test
covers the not-found error, and the app maps it to an install hint in the output
pane, but no test runs the app without Node on `PATH`.

**CI results** (debug builds on shared runners), in-app `run` self-test passes
on both:

| Platform | Mod+Enter to first output, first run | Second run |
|---|---|---|
| macOS arm64 | 48 ms | 35 ms |
| Ubuntu under Xvfb | 49 ms | 50 ms |
