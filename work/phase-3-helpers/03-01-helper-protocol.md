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

- [x] `HelperMessage` enums (core → helper, helper → core) with
      `#[serde(tag = "type")]`, matching PLUGINS.md §5 field for field. Unknown
      message types from the helper are logged and ignored, not fatal.
- [x] Forward-compatibility rule written into PLUGINS.md §5 and tested with the
      fake helper: helpers must ignore unknown fields in any message, and the core
      ignores unknown fields from helpers. Adding an optional field (e.g. `debug`
      on `run`) is not a protocol version bump. (DEBUGGING.md D7.)
- [x] `run` carries no timeout field. (DEBUGGING.md D12.)
- [x] Codec: one JSON object per line, `\n` terminated; tolerate `\r\n`; a line
      that fails to parse is logged with its first 200 bytes and skipped.
- [x] `HelperClient::start(spec) -> Result<HelperClient>`: spawn, send `hello`
      with `plugin_dir` and `scratch_dir`, wait for `ready` with a 10 s timeout.
- [x] `run(RunRequest) -> RunHandle` producing the same `RunEvent`s as the per-run
      runner (stdout bytes decoded from base64 and passed through the dump parser
      from 01-07). `cancel()` sends `cancel` and, after 2 s without `exited`,
      kills the helper and marks the run cancelled.
- [x] `hook(HookRequest) -> Result<HookResult>` with a 5 s timeout, returning
      `files` and `lsp_config`. Hooks not listed in the manifest's `hooks` are
      never sent; the caller gets `Ok(HookResult::default())`.
- [x] `shutdown()`: send `shutdown`, 2 s grace, kill.
- [x] Fake helper in `tests/` (Rust, standalone binary): implements everything,
      with test switches (`crash_on_run`, `hang_on_cancel`, `slow_ready`).
- [x] Tests: hello/ready; run streaming stdout with a dump sentinel inside; cancel
      that works; cancel that hangs → kill path; hook round-trip; `ready` timeout.
- [x] PLUGINS.md §5 updated to match exactly what shipped, including the base64
      chunk convention and every timeout.

## Out of scope

Pooling and lifecycle (03-02), wiring into the run path (03-03), any real helper.

## Gotchas

Helpers may print to stdout before `ready` (a runtime banner). Treat anything
that isn't a valid message before `ready` as log noise, not a protocol error.

## Results

Implemented 2026-09-17.

**`tinkerpad_plugin::helper`**, in three parts. `protocol` is the wire format:
`ToHelper` and `FromHelper`, both `#[serde(tag = "type")]`, field for field with
PLUGINS.md §5. `codec` is the JSON-lines decoder. `client` is one running
helper: spawn, `hello`, `ready`, then runs and hooks until something stops it.

**The codec** takes whatever chunks arrive and gives back whole messages. `\r\n`
is tolerated, blank lines are nothing, a line that isn't a message comes back as
a `Garbled` with its first 200 bytes for the log, and a line past 16 MiB is
dropped as it arrives rather than buffered — a helper that loses its mind can't
take the app with it. Unparseable lines are logged at debug before `ready` (a
runtime banner is not an error) and as a warning after it.

**One router task** owns the child and fans its messages out: a run's output to
whoever holds that `HelperRun`, a hook's `result` to the call waiting on it, and
anything else to the log. The helper's own stderr is log-only; a *run's* stderr
arrives as a `stderr` message, because only one of those two belongs in the
results pane.

**Cancel means cancelled.** `HelperRun::cancel()` sends `cancel` and waits 2 s;
if nothing comes back it kills the helper, waits for the process to actually be
gone, and ends the run as cancelled either way. A caller that cancels and then
asks whether the helper is alive gets the truth, which is what the pool (03-02)
will need before it respawns. A second `run` cancels the first by the same path,
and the first run is reported cancelled rather than abandoned.

**Hooks never block the app.** A hook the manifest doesn't declare is never
sent, and one that doesn't answer in 5 s is treated as having returned nothing;
both give `Ok(HookResult::default())`. Only a dead helper is an error.

**Measured** (debug build, Windows, fake helper): spawn to `ready` **14.8 ms**;
a warm run, from `run` to the last event of the reply, **205 µs** averaged over
50 runs. The per-run runner's cheapest possible equivalent is a process spawn,
which 01-06 measured at 42 ms for a trivial program — which is the whole argument
for helpers in one line.

**The fake helper** (`src/bin/fakehelper.rs`) is a working `tinkerpad-plugin-v1`
implementation in 200 lines, and deliberately an ill-mannered one: it prints a
banner before `ready`, puts fields in `ready` that no version of the core
defines, and sends a `telemetry` message that is not a message type at all. What
a run does is chosen by the playground `kind` — `echo` streams the main file
back in 7-byte chunks, `stderr` sends it the other way, `slow` starts and waits
to be cancelled, `fail` answers with a diagnostic, `crash` exits mid-run — and
four environment switches cover the paths a well-behaved helper never takes:
`FAKEHELPER_SLOW_READY_MS`, `_CRASH_ON_RUN`, `_HANG_ON_CANCEL`,
`_IGNORE_SHUTDOWN`.

**Tests** — 11 unit tests on the codec (both line endings, a message split
across three chunks, a banner skipped, the 200-byte log cut kept off a character
boundary, the 16 MiB cap, JSON that isn't a message, a message type from the
future, unknown fields, a last line with no newline) and 14 against a real
process: hello/ready through the banner, with what `hello` carried read back off
disk; a run streaming its output with a dump sentinel and an emoji intact byte
for byte; a run's stderr; a cancel the helper honours, leaving it warm enough to
run again; a cancel it ignores, which kills it; a helper that dies mid-run
failing the run instead of hanging it; a second run cancelling the first; a
compile error arriving as `failed` with somewhere to point; all four hooks,
including the one that isn't declared and so is never sent; the `ready` timeout;
`shutdown` honoured, and `shutdown` ignored and killed after the grace; and
forward compatibility in both directions, driven by writing raw JSON with
invented fields straight into the helper's stdin.

**PLUGINS.md §5** now says what shipped: the transport rules, base64 chunking
and what not to do to program output, the playground object as a helper actually
receives it (camelCase, absolute), which diagnostic fields are optional, the
`exited`-or-`failed` rule, and every timeout in one table.

## Checklist status

**Deviation: `HelperEvent`, not `RunEvent`.** The item asks for "the same
`RunEvent`s as the per-run runner … passed through the dump parser from 01-07".
`RunEvent`, `OutputDecoder`, and `DumpParser` live in `tinkerpad-runner`, which
depends on this crate; a client here cannot use them without inverting the
dependency direction. `HelperEvent` is the same shape minus `Started` (a
helper's run has no pid of its own), and the client hands over the program's
bytes exactly as written, sentinels and all. 03-03 feeds them to the same
`OutputDecoder` the per-run path uses, so the UI sees one kind of run. The test
asserts the bytes survive byte for byte, sentinel included, which is the part
this layer is responsible for.

**Deviation: the fake helper is in `src/bin/`, not `tests/`.** `CARGO_BIN_EXE_…`
only finds binary targets, and this matches `tinkerpad-fakelsp` from 02-01.

**Note: no test for the 5 s hook timeout.** It would add five seconds to the
suite to prove a `tokio::time::timeout` call. The two other grace periods that
end in a kill (`cancel`, `shutdown`) are both tested, and the "hook that is never
sent" test covers the other way of getting an empty result.
