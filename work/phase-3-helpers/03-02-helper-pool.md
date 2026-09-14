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

- [ ] Extract the generic supervision pieces from `tinkerpad-lsp` (state machine,
      backoff, idle timer, RSS sampling) into `tinkerpad-process::supervise` so
      both pools share them. LSP tests still pass after the move. The generic
      `Pool<K, P>` is parameterised over the protocol client type so a third pool
      (debug adapters, Phase 6) is a type instantiation, not a copy.
      (DEBUGGING.md D5.)
- [ ] `HelperPool::get(plugin_id) -> Result<Arc<HelperClient>>` starts on first
      call; concurrent first calls wait on the same start rather than starting two.
- [ ] Build step: if `[helper].build` exists, compute a hash of the helper source
      folder (file names + mtimes + sizes); if it differs from the last recorded
      hash in `<app-data>/helpers/<plugin-id>.build`, run the build command with
      output streamed to the output pane under a "Building C# helper…" header, then
      record the hash. Build failure → `HelperState::Failed { reason }`; runs fall
      back to `[run]` (03-03).
- [ ] Crash restart with backoff and the same give-up rule as 02-05. A run in
      progress at crash time ends with `RunEvent::Failed("helper crashed")`.
- [ ] Idle shutdown after `[helper].idle_shutdown` with no runs (default `"never"`
      per the manifest, user-overridable in settings under `[helpers.<id>]`).
- [ ] App exit: shut all helpers down alongside the LSP servers, same grace rules.
- [ ] Status strip (02-08) shows helpers as chips with the same states.
- [ ] Tests with the fake helper: shared instance across two calls; crash →
      restart; build hash unchanged → no rebuild; source touched → rebuild.

## Out of scope

Using helpers to run code (03-03). Hooks (03-07).

## Gotchas

The build command runs with the plugin's toolchain, so its program name must go
through the same override/`PATH` resolution as `[run]` (01-06). Reuse that function.
