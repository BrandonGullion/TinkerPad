# 02-01 · JSON-RPC transport

**Phase:** 2 · **Size:** S · **Crate/area:** `tinkerpad-lsp` · **Depends on:** 00-04
**Design:** ARCHITECTURE.md §4.3

## What

LSP's wire format over a child process's stdio: `Content-Length` framing, a reader
task that yields `RawMessage`s, a writer task fed by an `mpsc`, and just enough
message classification (request / response / notification, by presence of `id` and
`method`) to route. Messages stay as `serde_json::Value` at this layer.

## Why

Small, self-contained, and the first thing that talks to a real server. Getting it
right in isolation, with a fake server, avoids debugging framing bugs through the
editor.

## Expectations

- [ ] Two modules: `framing` (Content-Length codec over a byte stream: `encode`,
      a streaming `decode` yielding `Vec<u8>` bodies) and `jsonrpc` (parses bodies
      into `RawMessage`, classifies, routes). `framing` has no knowledge of
      JSON-RPC. (DEBUGGING.md D4: DAP reuses `framing` with its own message
      shapes.)
- [ ] `Transport::spawn(SpawnSpec) -> (Transport, Supervised)` on top of
      `tinkerpad-process`. `send(RawMessage)` and an `incoming()` stream.
- [ ] Reader handles headers in any order and case, `Content-Type` present or
      absent, a body split across chunks, and multiple messages in one chunk.
      Malformed header → log, resync by scanning for the next `Content-Length:`.
- [ ] Writer serialises with exactly one `Content-Length` header and `\r\n\r\n`.
- [ ] `RawMessage` classification helpers: `is_request()`, `is_response()`,
      `is_notification()`, `id()`, `method()`.
- [ ] Server stderr is logged line by line under the server's span (from 00-07).
- [ ] A fake LSP server binary in `tests/` (Rust, ~100 lines) that answers
      `initialize`, echoes a custom `test/echo` request, sends a
      `window/logMessage` notification on demand, and exits on `exit`. Used here
      and by 02-02, 02-05.
- [ ] Tests: initialize handshake; 1,000 echo round-trips in order; a 2 MB message
      body; server closes stdout mid-message → incoming stream ends with an error,
      no panic.

## Out of scope

Typed `lsp-types` deserialisation (done per method where needed in 02-03),
pooling, restart.

## Gotchas

Use `lsp-types` only for the messages the proxy actually inspects. Deserialising
every message into typed structs costs time and breaks on servers that send
non-standard extensions.
