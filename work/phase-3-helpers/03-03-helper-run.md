# 03-03 · Helper-run path with fallback

**Phase:** 3 · **Size:** S · **Crate/area:** `tinkerpad-runner` · **Depends on:** 03-02, 01-06
**Design:** ARCHITECTURE.md §5.1

## What

When a plugin declares `[helper]` with `"run"` in its `hooks`, Mod+Enter goes to
the helper. If the helper is unavailable (build failed, gave up restarting, missing
binary), fall back to `[run]` and tell the user once.

## Why

This is the switch that turns the warm runner on. It must be invisible when it
works and honest when it doesn't.

## Expectations

- [ ] `RunnerSpec` resolution: `Helper` if `[helper]` exists and lists `run`;
      otherwise `PerRun`. If `Helper` is chosen but the pool reports `Failed` or
      `GaveUp`, use `PerRun` and emit a one-time toast per session: "C# helper
      unavailable (reason); using dotnet run".
- [ ] `RunEvent` stream is identical from either path; the UI cannot tell them
      apart except for a small "warm" badge in the status bar for helper runs.
- [ ] Cancel goes to the right place: helper `cancel` or process kill.
- [ ] One run per playground still holds; two playgrounds of the same language
      queue on the single helper (protocol allows one run at a time). The queued
      run shows "waiting for helper" in its tab.
- [ ] A `Force per-run` command palette entry for debugging, sticky per session.
- [ ] Test with the fake helper: run routes to helper; with `crash_on_run` and
      give-up reached, next run routes to per-run with the toast event emitted once.

## Out of scope

Real helpers (03-04..06).

## Gotchas

Save-before-run still applies: the helper reads the file from disk, same as the
per-run path. Don't pass source text over the protocol.
