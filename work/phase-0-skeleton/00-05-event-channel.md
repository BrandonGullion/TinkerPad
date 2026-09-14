# 00-05 · Core to UI event channel and command surface

**Phase:** 0 · **Size:** S · **Crate/area:** `tinkerpad-app`, `tinkerpad-core`, `ui/` · **Depends on:** 00-02
**Design:** ARCHITECTURE.md §3, §8

## What

The two pipes between the Rust core and the webview. Commands go UI → core as Tauri
commands with typed request/response. Events go core → UI through one broadcast
channel forwarded by a single task, so ordering is preserved. Types live in
`tinkerpad-core` and are mirrored in `ui/src/protocol.ts`.

## Why

Every feature after this is "add a command, add an event". Having one pattern,
typed on both ends, keeps the UI boundary narrow enough that the shell could be
swapped later (PLAN.md §3.1).

## Expectations

- [x] `tinkerpad-core` defines `Command`-side request/response structs and a
      `CoreEvent` enum, all `serde` serialisable, with `#[serde(tag = "type")]`.
- [x] `AppState` in `tinkerpad-app` holds an `Arc` of everything and a
      `broadcast::Sender<CoreEvent>`. Exactly one task drains it and calls Tauri
      `emit`. Nothing else calls `emit`.
- [x] UI has a single `subscribe(handler)` and a typed `invoke<T>()` wrapper.
      `protocol.ts` is hand-written now; note in the file that generating it (e.g.
      with `ts-rs` or `specta`) is an option if it drifts.
- [x] Events carry a scope id (playground id, server id, or run id) so the UI can
      route without inspecting payloads.
- [x] A burst of 10,000 events does not reorder and does not drop under normal
      load; lagging receivers are logged, not silently ignored.
- [ ] Errors from commands reach the UI as a structured `{ code, message }`, never
      as a stringified `anyhow` chain. The UI shows them in a non-blocking toast.

## Out of scope

Any specific command beyond `ping` and the run/cancel pair added in 00-06.

## Gotchas

Tauri `Channel` (per-call streaming) is the better fit for high-volume streams like
LSP traffic in Phase 2. Use the broadcast + emit path for app-wide events, and note
in the code where a Channel should be used instead.

## Results

Implemented 2026-09-14.

- Types: `tinkerpad-core` `events.rs` (`Envelope`, `Scope`, `CoreEvent`,
  `RunEvent`) and `error.rs` (`CommandError`, `ErrorCode`), with wire-format
  tests. Mirror in `ui/src/protocol.ts`.
- Bus: `tinkerpad-app` `events.rs`. Sequence numbers are assigned under the same
  lock as the send, so order is total. One forwarder task calls Tauri `emit`.
  Lag is logged.
- Test: four tasks publish 10,000 events concurrently; all arrive, strictly in
  sequence order.
- UI: `ipc.ts` has `subscribe`, `ready`, and a typed `invoke` that normalises
  failures to `CommandError`, shows a toast, and rethrows. `toast.ts` renders
  them.
- Verified in the release app: the `pong` event now travels through the bus and
  renders.
- Not yet exercised visually: a command error producing a toast. No command can
  fail yet; 00-06's `run` command is the first that can.

## Checklist status

Checked 2026-09-14.

**Open**
- *Command errors shown as a toast.* The structured `{ code, message }` wire
  format is tested in Rust, and `ipc.ts` normalises failures and calls `toast`.
  No command has failed in a test, so the toast has never been seen.

**Ticked with a note**
- *`Command`-side request/response structs.* Commands take plain arguments;
  the shared types are `CommandError` and `ErrorCode`. Request structs will be
  added when a command needs more than a couple of arguments.
- *`broadcast::Sender<CoreEvent>`.* The channel carries `Envelope` (sequence
  number, scope, event) rather than bare `CoreEvent`.
