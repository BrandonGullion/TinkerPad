# 03-02 · Helper pool and build-on-first-use

**Phase:** 3 · **Size:** M · **Crate/area:** `tinkerpad-runner`, `tinkerpad-plugin` · **Depends on:** 03-01, 02-02, 02-05, 02-06
**Design:** ARCHITECTURE.md §1, §5.1; PLUGINS.md §5, §8

## What

`HelperPool`: one helper per plugin, lazily started, kept alive, restarted on
crash, idle-shutdown per `[helper].idle_shutdown`. Before the first start, run
`[helper].build` if present and the helper binary is missing or the helper source
folder has changed.

## Why

The helper is the warm runner. It must have exactly the lifecycle guarantees the
LSP pool has, and those already exist; this item reuses them rather than
reimplementing.

## Expectations

- [x] Extract the generic supervision pieces from `tinkerpad-lsp` (state machine,
      backoff, idle timer, RSS sampling) into `tinkerpad-process::supervise` so
      both pools share them. LSP tests still pass after the move. The generic
      `Pool<K, P>` is parameterised over the protocol client type so a third pool
      (debug adapters, Phase 6) is a type instantiation, not a copy.
      (DEBUGGING.md D5.)
- [x] `HelperPool::get(plugin_id) -> Result<Arc<HelperClient>>` starts on first
      call; concurrent first calls wait on the same start rather than starting two.
- [x] Build step: if `[helper].build` exists, compute a hash of the helper source
      folder (file names + mtimes + sizes); if it differs from the last recorded
      hash in `<app-data>/helpers/<plugin-id>.build`, run the build command with
      output streamed to the output pane under a "Building C# helper…" header, then
      record the hash. Build failure → `HelperState::Failed { reason }`; runs fall
      back to `[run]` (03-03).
- [x] Crash restart with backoff and the same give-up rule as 02-05. A run in
      progress at crash time ends with `RunEvent::Failed("helper crashed")`.
- [x] Idle shutdown after `[helper].idle_shutdown` with no runs (default `"never"`
      per the manifest, user-overridable in settings under `[helpers.<id>]`).
- [x] App exit: shut all helpers down alongside the LSP servers, same grace rules.
- [x] Status strip (02-08) shows helpers as chips with the same states.
- [x] Tests with the fake helper: shared instance across two calls; crash →
      restart; build hash unchanged → no rebuild; source touched → rebuild.

## Out of scope

Using helpers to run code (03-03). Hooks (03-07).

## Gotchas

The build command runs with the plugin's toolchain, so its program name must go
through the same override/`PATH` resolution as `[run]` (01-06). Reuse that function.

## Results

Implemented 2026-09-18.

**`tinkerpad_process::supervise`** is now the one place a long-lived child is
kept alive. `Lifecycle` holds the state, the crash budget, the idle clock, the
uptime and the last few hundred things the child said; `Crash` says whether to
wait and restart or to give up; `Rss` samples memory; `Pool<K, P>` holds many,
hands out `Arc`s, and stops them all at exit inside a budget; `watch` is the
one tick loop that stops an idle child and keeps its memory figure fresh. A
protocol client becomes one of these by implementing `Supervised`: a lifecycle,
a pid, a polite stop and a kill.

**The LSP side moved onto it** rather than keeping a copy. `ServerHandle` lost
its state channel, restart counter, idle clock, uptime, log ring, and its own
`watch`, `backoff` and `resident_memory` functions — 134 lines out of
`server.rs` and the whole map-and-shutdown half of `pool.rs` — and gained one
`Lifecycle` field. Its public API is unchanged, and all **47 LSP integration
tests and 33 unit tests still pass**, which is the point: the refactor is
supposed to be invisible from outside.

**`HelperPool`** (in `tinkerpad-runner`) is what the item is really for. One
helper per plugin, started on first call, shared by everything that asks, and
restarted when it dies of its own accord. `HelperHandle::client()` holds an
async lock across the start, so callers racing for a cold helper wait for one
start instead of beginning three. A helper that keeps crashing is given up on
with the same rule servers get, and comes back when someone asks for it by name.

**Build on first use.** If a plugin ships its helper as source
(`[helper.build]`), the pool hashes the plugin folder — names, sizes and
modified times, skipping `bin`, `obj`, `target`, `node_modules` and `.git` —
and compares it with `<app-data>/helpers/<plugin>.build`. Different or missing
means build; the same means don't. Build output goes to the helper's log, where
the status strip's **Log** button shows it, and to a broadcast channel a run
waiting on the build can subscribe to (03-03 puts it in the output pane). A
build that fails leaves `Stopped` with the reason, nothing is recorded, and the
run path falls back to `[run]`.

**Idle is runs, not documents.** A server's clock runs when it has no open
documents; a helper's runs when nothing is using it. `HelperHandle::using()`
hands out a `Use` guard that stops the clock while it lives, which is what the
run path will hold for the length of a run.

**Settings** for both live in one reader now: `lsp_settings.rs` became
`service_settings.rs`, and the same parser serves `[lsp.<id>]` and
`[helpers.<id>]`, because the two things have the same shape and the same life.

**The status strip shows both.** Servers and helpers are normalised into one
`Child` in `servers.ts`, so a helper gets the same chip, dot, memory figure,
popover and Restart/Stop/Log buttons, with two rows of its own (what is using
it, and how long it has been idle) in place of the server's documents and
percentiles. A new `CoreEvent::Helper` refreshes the strip the way `Server`
does.

**Measured** (Windows, release build, the test plugin's Node helper): the
build's first run **46–48 ms**, a helper **ready** and answering with a pid
straight after, and **35 MB** resident — sampled by the same watcher that
samples rust-analyzer. Quitting stops servers and helpers concurrently, each
under its own 3 s budget, so neither queues behind the other.

**Tests** — 8 unit tests on the shared supervision (backoff doubling to the cap,
giving up and starting the count over, the shorter limit for something never
used, the idle clock, the log ring, states that wake nobody, memory shown only
against a running child) and 3 on the folder hash; **10 integration tests**
against real processes with the fake helper from 03-01: one helper shared by
everything that asks, three callers racing to be first starting one between
them, a helper that dies coming back warm with a new pid, one that will not
start reported rather than hammered, the build running before the first use and
not again, a changed source building again, a failed build leaving it
unavailable and not recorded, an idle helper stopping on its own, one in use
not stopping, every helper stopping together inside the budget, and Restart
replacing the process.

**End to end in the app**, as a `helpers` self-test scenario and in CI: the test
plugin now ships a real `tinkerpad-plugin-v1` helper (`helper.js`) and a
stand-in build step (`build.js`), so the whole path — manifest to config to
build to process to chip — runs on any machine with Node. Eight checks: it
starts on demand, gets a chip of its own, reports a pid and a name, shows the
right facts in its popover, builds again when its source changes (with the
build's own output kept), doesn't build when nothing changed, stops, and comes
back as a new process. A screenshot shows both chips side by side:
`● Node check 35 MB` and `● Node check helper 35 MB`.

## Checklist status

**Note: `Pool<K, P>` is parameterised over the supervised child, not the
protocol client.** The item asks for the pool to be "parameterised over the
protocol client type"; what ships takes the type that *owns* the protocol
client (`ServerHandle`, `HelperHandle`) and requires it to implement
`Supervised`. That is what makes a debug adapter pool in Phase 6 a type
instantiation: the protocol client itself has no lifecycle to supervise.

**Deviation: build output reaches the output pane in 03-03, not here.** The item
says the build streams "to the output pane under a Building C# helper… header".
In 03-02 nothing has started a run yet — the pool builds because something asked
for a helper — so the header and every line go to the helper's log and to a
broadcast channel. 03-03 subscribes to it before asking for the helper and puts
the lines in the pane of the run that caused the build. The header text is here
and tested.

**Note: the crashed-run message is 03-01's.** "A run in progress at crash time
ends with `RunEvent::Failed("helper crashed")`" is already implemented, as
`HelperEvent::Failed { message: "the <plugin> helper stopped while the program
was running" }`, and tested there. 03-03 maps it to the UI's `RunEvent::Failed`.
What is new here is that the helper comes back afterwards.

**Note: no plugin ships a helper yet.** The bundled Python, C# and TypeScript
helpers are 03-04, 03-05 and 03-06, so the only `[helper]` section in the
repository is the test plugin's. Everything above is exercised through it.
