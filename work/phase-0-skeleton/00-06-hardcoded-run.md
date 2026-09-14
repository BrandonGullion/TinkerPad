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
