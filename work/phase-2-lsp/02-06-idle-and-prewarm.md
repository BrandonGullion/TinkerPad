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

- [x] `idle_shutdown` values: `"never"` or a duration string (`"30m"`, `"2h"`).
      Plugin default from the manifest; user override in settings
      `[lsp.<plugin-id>] idle_shutdown = "..."`.
- [x] Idle clock starts when the last document closes and resets when one opens.
      Shutdown is the clean `shutdown`/`exit` sequence, state `Stopped`, no toast.
- [x] Pre-warm: settings `[lsp] prewarm = ["typescript", "csharp"]`. At launch,
      after the window is visible (never before), spawn those servers with a root
      per their workspace strategy. Launch-to-window time is unchanged; verify by
      measuring with and without pre-warm.
- [x] A pre-warmed server with no documents is still subject to idle shutdown, so
      an unused pinned language doesn't sit forever. Default idle for pre-warmed
      servers with zero docs: 1 h.
- [x] Memory: RSS per server sampled every 10 s (via `sysinfo` or the `windows`
      crate) and included in `ServerState` events for 02-08.
- [x] Tests: idle timer fires and stops the fake server; reopening a doc after
      shutdown respawns transparently; pre-warm list with an unknown plugin id logs
      and continues.

## Out of scope

Memory-based eviction (`MemoryAbove`). Keep the enum variant reserved and add it
if RA proves to be a problem in practice.

## Gotchas

Idle shutdown must be cancelled if a `didOpen` arrives during the shutdown
sequence; simplest is to mark `Stopping` and have `attach` wait for `Stopped` then
respawn.

## Results

Implemented 2026-09-16.

**Idle.** Each server has a clock that runs only while it has no open
documents: it starts when the last one closes and stops when one opens. How
long it may run comes from the plugin's `idle_shutdown`, overridable per
language in settings; `"never"` means never. The exception is a server that has
never been given a document at all — a pre-warmed language nobody opened —
which is held to an hour whatever the setting says, so pinning a language you
don't use that day costs you an hour of memory, not a day of it.

Reaching the limit is the ordinary `shutdown`/`exit` sequence and a `Stopped`
state with no reason and no toast: nobody asked for it, so nobody is told. The
next tab starts the server again on the same handle, and any request made in
the meantime waits for it rather than failing.

**Settings** (`crates/tinkerpad-app/src/lsp_settings.rs`)

```toml
[lsp]
prewarm = ["typescript"]

[lsp.rust]
idle_shutdown = "30m"     # or "never", "90s", "2h"
```

Anything wrong in there is a warning and the setting is ignored — a typo in a
duration should cost you the setting, not the app. `TINKERPAD_LSP_PREWARM`
overrides the list, which is how the self-test and CI pin a server.

**Pre-warm.** The pinned servers start when the UI reports it is up, never
before, so the window can't be held up by one. A language whose server lives in
each playground's folder (Rust today) can't be pre-warmed — there is no
playground yet — and is logged and skipped, as is an id with no plugin.

**Memory.** Resident memory is sampled every 10 s per server (`sysinfo`) and
carried in `ServerState::Ready { rss_bytes }`, which the status strip will show
in 02-08. A new `server_status` command lists every server the pool holds with
its state, root, and memory.

**Waiting, revisited.** A request used to be refused the moment a server was
anything but `Ready`. Now `Starting`, `Stopping`, and `Crashed` all mean "it is
coming back, wait for it", and only a server stopped for good says no. That is
what makes an idle stop invisible: the tab that wakes the server doesn't have
to know it was asleep.

**A test server.** The `node-check` fixture plugin gained `server.js`, a ~130
line language server that answers `initialize`, completion, hover, and
diagnostics. It gives the self-tests (and CI, on every OS) a real server with
nothing to install, and it is what the pre-warm scenario pins.

**Measurements** (Windows 11, release build)

| What | Result |
|---|---|
| Launch to window, no pre-warm | 398, 399, 426 ms (median 399) |
| Launch to window, pre-warming a server | 407, 400, 400 ms (median 400) |
| Memory of the test server, sampled | 48.3 MiB |
| Idle tests, whole file | 3.4 s (real timers, in milliseconds) |

Pre-warm costs the window nothing, which is the point: it starts after the UI
says it is up.

**Tests** — 8 against real processes, 4 unit.

- A server with no documents left stops on its own, and the next tab brings it
  back on the same handle with a new process.
- Opening a document again resets the clock; a server with a document open
  never idles.
- A server that was never given a document stops even when its plugin says
  "never"; one that has been used and says "never" is kept.
- An idle stop is clean and quiet: no reason, no hint, no toast.
- A request after an idle stop starts the server again and is answered.
- Memory is sampled and looks like bytes.
- Settings: `prewarm` and per-language `idle_shutdown` are read, `"never"`
  overrides a plugin's duration, a language with no setting keeps the plugin's,
  and nonsense costs only the setting.

In the app, a new `prewarm` scenario (4 checks) confirms a pinned server is up
with no playground open, is a real process rooted at the library, that an
unknown language id is skipped rather than fatal, and that its memory is
sampled. CI runs it on macOS and Linux.

**Docs** — ARCHITECTURE.md §4.1's idle and pre-warm rules now describe what was
built, including the launch measurements; `settings.toml`'s template documents
the `[lsp]` section.

## Checklist status

**Open: `MemoryAbove` eviction** is out of scope by the item's own text; the
`rss_bytes` sample is there for when it is wanted.

**Note: Rust can't be pre-warmed yet.** Its server is rooted at each
playground's folder, so there is nothing to start before a tab is open. 02-07
gives Rust one shared scratch workspace, which is what will make it warmable —
at which point it becomes the language that most wants pre-warming.
