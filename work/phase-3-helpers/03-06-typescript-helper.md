# 03-06 · TypeScript Node helper

**Phase:** 3 · **Size:** S · **Crate/area:** `plugins/typescript/helper/` · **Depends on:** 03-03, 01-09
**Design:** PLUGINS.md §7 (TypeScript), §5

## What

A Node script that stays resident and runs playgrounds by dynamic `import()` with
a cache-busting query, so Node's ~40 ms startup and the type-strip pass happen
once. Small, and mostly here to prove a second helper against the protocol.

## Why

Node is already fast per-run. The helper wins ~40 ms per Mod+Enter and, more
importantly, gives the protocol a second real implementation before contributors
write theirs.

## Expectations

- [x] `helper/helper.mjs`, no build step (`[helper]` without `build`).
- [x] On `run`: `await import(pathToFileURL(main) + "?t=" + Date.now())`, with
      `process.stdout.write` and `process.stderr.write` monkey-patched for the
      duration to emit base64 chunks; unhandled rejections and thrown errors →
      `stderr` with the stack plus `exited` code 1.
- [x] Cancel: no safe way to abort a running module in-process; on `cancel` the
      helper replies nothing and lets the core kill and respawn it (03-01's 2 s
      rule). Document this trade-off in the plugin README.
- [x] Module cache growth: the query-string trick leaks one module entry per run.
      Restart the helper automatically after 500 runs (helper-side counter,
      reply `exited` then exit 0; the pool restarts it on next use without
      counting it as a crash — add a `{"type":"retiring"}` message to the
      protocol if the pool needs to distinguish this, and update PLUGINS.md §5).
- [x] `--experimental-transform-types` passed in `[helper].command` args so
      `enum` works like the per-run path.
- [x] The helper's Node args come entirely from the manifest (`[helper].command`
      + args), never hard-coded in the script, so Phase 6 can add
      `--inspect=0` via a `[debug]` section. Confirm by hand that starting the
      helper with `--inspect=0` prints the inspector URL on stderr and the helper
      still speaks the protocol on stdout. (DEBUGGING.md D10.)

## Out of scope

Worker-thread isolation (would allow cancel; consider if the respawn cost annoys).

## Gotchas

A playground that starts a server (`http.createServer`) keeps the helper busy
after `exited`. Track open handles with `process.getActiveResourcesInfo()` and
warn on `stderr` if any remain.

## Results

Implemented 2026-09-19, against **Node 24.14.0**.

**`plugins/typescript/helper/helper.mjs`**, 250 lines and no build step. A run
is `await import(file + "?t=…")` with `process.stdout.write` and
`process.stderr.write` borrowed for the duration, so Node's start-up and its
first type-strip pass are paid for once a session.

**Measured**, the same file each way:

| | Per-run (`node …  main.ts`) | On the helper |
|---|---|---|
| Whole run | 55–64 ms | **5 ms** (2 ms of it the run itself) |
| Helper start to `ready` | — | 32 ms |

In the app, Mod+Enter to a rendered `dump()` table: 98 ms on the session's first
run (which starts the helper), **20 ms** warm.

**A run ends when the program would have ended.** The first version reported
`exited` as soon as the module's top-level code finished — so
`setInterval(…)` "ended" instantly while still ticking inside the helper, and
`setTimeout(() => console.log("later"), 700)` lost its output entirely. Node
keeps going until the event loop is empty, and so does this now: after the
import resolves it waits for anything the run put on the loop, watching
`process.getActiveResourcesInfo()` against a baseline taken when the run
started. A delayed `console.log` arrives (measured at 710 ms for a 700 ms
timer); a `setInterval` keeps the run going, exactly as `node main.ts` would,
until the user cancels.

That took two goes to get right. `setInterval` and the helper's own polling
timer are both reported as `Timeout`, so filtering the name out filtered out the
thing being looked for; the poll now uses an `unref`'d timer, which isn't on the
loop at all. And the baseline has to be taken when a run starts, not at startup:
stdin's pipe attaches after the module's top level, so everything looked like a
leftover.

**Cancel is a restart, and says so.** There is no safe way to stop a module
running on this thread. The helper answers `cancel` with silence and the core
kills it after two seconds and starts another (03-01's rule) — measured at
2312 ms from the user pressing Mod+. to the run being reported cancelled. The
plugin README explains the trade and points at the worker-thread alternative.

**Retirement is a new protocol message.** The cache-busting query means Node
keeps every module the helper has ever loaded, so after 500 runs it says
`{"type":"retiring","reason":…}` and exits 0. Without this the pool would count
that as a crash, back off, and tell the user their helper had died. Now it
starts a fresh one immediately — no backoff, no crash counted, nothing said.
PLUGINS.md §5 documents the message, `FromHelper::Retiring` reads it,
`HelperClient::is_retiring` exposes it, and the pool acts on it.

**D10 confirmed by hand.** Every Node flag comes from `[helper].args`, none from
inside the script. Started with `--inspect=0` added, Node prints
`Debugger listening on ws://127.0.0.1:59500/…` on **stderr** and the helper
speaks the protocol on stdout as usual — which is what Phase 6 needs.

**Tests.** A pool test with the fake helper (which now has a
`FAKEHELPER_RETIRE_AFTER` switch): a helper that retires is replaced, its
`restarts()` stays 0, and the pool hands out the new one. In the app, the
TypeScript scenario now runs entirely on the helper and passes unchanged, plus
three checks of its own: the run says "warm", a run that won't stop is cancelled
anyway, and the helper comes back as a new process. The `dump`, `annotations`,
`packages`, `run` and `tabs` scenarios — all of which use this plugin — pass
unchanged.

## Checklist status

**Deviation: a run waits for the event loop.** The item says `import()` then
report; that would have made `setTimeout` output vanish and `setInterval` runs
end while still running. Waiting matches Node, and matches the per-run path the
helper is supposed to be indistinguishable from. The gotcha the item raises — a
playground that starts a server holding the helper — is still true and still
warned about; it is now visible as a run that hasn't finished, which is what it
is.

**Note: the "retiring" message was added, as the item allowed.** It is the only
way the pool can tell a planned exit from a crash, and a crash toast for a
routine replacement would be a lie.

**Note: 500 runs is not reachable in a test.** The Rust test uses one run via
the fake helper's switch. The number itself is a judgement, not a measurement.
