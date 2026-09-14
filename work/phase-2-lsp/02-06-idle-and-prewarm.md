# 02-06 · Idle policy and pre-warm

**Phase:** 2 · **Size:** S · **Crate/area:** `tinkerpad-lsp`, `tinkerpad-app` · **Depends on:** 02-02
**Design:** ARCHITECTURE.md §4.1, PLAN.md §6 (rust-analyzer memory)

## What

Two policies on the pool: shut a server down after it has had no open documents
for a configurable time (per plugin `idle_shutdown`, user-overridable), and spawn
the user's pinned languages at app launch in the background so the first tab is
already warm.

## Why

Keep-alive without an idle policy means rust-analyzer eating a gigabyte forever.
Pre-warm is the flip side: the first completion of the day should be as fast as
the hundredth.

## Expectations

- [ ] `idle_shutdown` values: `"never"` or a duration string (`"30m"`, `"2h"`).
      Plugin default from the manifest; user override in settings
      `[lsp.<plugin-id>] idle_shutdown = "..."`.
- [ ] Idle clock starts when the last document closes and resets when one opens.
      Shutdown is the clean `shutdown`/`exit` sequence, state `Stopped`, no toast.
- [ ] Pre-warm: settings `[lsp] prewarm = ["typescript", "csharp"]`. At launch,
      after the window is visible (never before), spawn those servers with a root
      per their workspace strategy. Launch-to-window time is unchanged; verify by
      measuring with and without pre-warm.
- [ ] A pre-warmed server with no documents is still subject to idle shutdown, so
      an unused pinned language doesn't sit forever. Default idle for pre-warmed
      servers with zero docs: 1 h.
- [ ] Memory: RSS per server sampled every 10 s (via `sysinfo` or the `windows`
      crate) and included in `ServerState` events for 02-08.
- [ ] Tests: idle timer fires and stops the fake server; reopening a doc after
      shutdown respawns transparently; pre-warm list with an unknown plugin id logs
      and continues.

## Out of scope

Memory-based eviction (`MemoryAbove`). Keep the enum variant reserved and add it
if RA proves to be a problem in practice.

## Gotchas

Idle shutdown must be cancelled if a `didOpen` arrives during the shutdown
sequence; simplest is to mark `Stopping` and have `attach` wait for `Stopped` then
respawn.
