# 03-01 · Helper protocol and fake helper

**Phase:** 3 · **Size:** M · **Crate/area:** `tinkerpad-plugin` · **Depends on:** 00-04, 01-01
**Design:** PLUGINS.md §5

## What

The Rust side of `tinkerpad-plugin-v1`: message types, a JSON-lines codec over a
child's stdio, a `HelperClient` with typed `run`, `cancel`, and hook calls, and a
fake helper binary for tests. Finalise the protocol text in PLUGINS.md §5 as part
of this item; it becomes public API when a third party implements it.

## Why

One protocol serves warm execution and hooks for every language. Getting the
message shapes and timeouts right once, with a fake, means each real helper
(Python, C#, Node) is a straightforward port.

## Expectations

- [ ] `HelperMessage` enums (core → helper, helper → core) with
      `#[serde(tag = "type")]`, matching PLUGINS.md §5 field for field. Unknown
      message types from the helper are logged and ignored, not fatal.
- [ ] Forward-compatibility rule written into PLUGINS.md §5 and tested with the
      fake helper: helpers must ignore unknown fields in any message, and the core
      ignores unknown fields from helpers. Adding an optional field (e.g. `debug`
      on `run`) is not a protocol version bump. (DEBUGGING.md D7.)
- [ ] `run` carries no timeout field. (DEBUGGING.md D12.)
- [ ] Codec: one JSON object per line, `\n` terminated; tolerate `\r\n`; a line
      that fails to parse is logged with its first 200 bytes and skipped.
- [ ] `HelperClient::start(spec) -> Result<HelperClient>`: spawn, send `hello`
      with `plugin_dir` and `scratch_dir`, wait for `ready` with a 10 s timeout.
- [ ] `run(RunRequest) -> RunHandle` producing the same `RunEvent`s as the per-run
      runner (stdout bytes decoded from base64 and passed through the dump parser
      from 01-07). `cancel()` sends `cancel` and, after 2 s without `exited`,
      kills the helper and marks the run cancelled.
- [ ] `hook(HookRequest) -> Result<HookResult>` with a 5 s timeout, returning
      `files` and `lsp_config`. Hooks not listed in the manifest's `hooks` are
      never sent; the caller gets `Ok(HookResult::default())`.
- [ ] `shutdown()`: send `shutdown`, 2 s grace, kill.
- [ ] Fake helper in `tests/` (Rust, standalone binary): implements everything,
      with test switches (`crash_on_run`, `hang_on_cancel`, `slow_ready`).
- [ ] Tests: hello/ready; run streaming stdout with a dump sentinel inside; cancel
      that works; cancel that hangs → kill path; hook round-trip; `ready` timeout.
- [ ] PLUGINS.md §5 updated to match exactly what shipped, including the base64
      chunk convention and every timeout.

## Out of scope

Pooling and lifecycle (03-02), wiring into the run path (03-03), any real helper.

## Gotchas

Helpers may print to stdout before `ready` (a runtime banner). Treat anything
that isn't a valid message before `ready` as log noise, not a protocol error.
