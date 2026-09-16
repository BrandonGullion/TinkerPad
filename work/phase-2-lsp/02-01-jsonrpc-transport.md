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

- [x] Two modules: `framing` (Content-Length codec over a byte stream: `encode`,
      a streaming `decode` yielding `Vec<u8>` bodies) and `jsonrpc` (parses bodies
      into `RawMessage`, classifies, routes). `framing` has no knowledge of
      JSON-RPC. (DEBUGGING.md D4: DAP reuses `framing` with its own message
      shapes.)
- [x] `Transport::spawn(SpawnSpec) -> (Transport, Supervised)` on top of
      `tinkerpad-process`. `send(RawMessage)` and an `incoming()` stream.
- [x] Reader handles headers in any order and case, `Content-Type` present or
      absent, a body split across chunks, and multiple messages in one chunk.
      Malformed header → log, resync by scanning for the next `Content-Length:`.
- [x] Writer serialises with exactly one `Content-Length` header and `\r\n\r\n`.
- [x] `RawMessage` classification helpers: `is_request()`, `is_response()`,
      `is_notification()`, `id()`, `method()`.
- [x] Server stderr is logged line by line under the server's span (from 00-07).
- [x] A fake LSP server binary in `tests/` (Rust, ~100 lines) that answers
      `initialize`, echoes a custom `test/echo` request, sends a
      `window/logMessage` notification on demand, and exits on `exit`. Used here
      and by 02-02, 02-05.
- [x] Tests: initialize handshake; 1,000 echo round-trips in order; a 2 MB message
      body; server closes stdout mid-message → incoming stream ends with an error,
      no panic.

## Out of scope

Typed `lsp-types` deserialisation (done per method where needed in 02-03),
pooling, restart.

## Gotchas

Use `lsp-types` only for the messages the proxy actually inspects. Deserialising
every message into typed structs costs time and breaks on servers that send
non-standard extensions.

## Results

Implemented 2026-09-16.

**`framing`** — the `Content-Length` codec, with no idea what a message means.

- `encode` / `encode_into` write exactly one `Content-Length` header and
  `\r\n\r\n`. `Decoder` is a two-state machine: `push` bytes as they arrive,
  `take` bodies as they complete.
- Forgiving in every way except the length: header names case-insensitive,
  headers in any order, `Content-Type` optional, `\n` accepted as a line
  ending, a body split across any number of chunks, several messages in one
  chunk, and a zero-length body.
- A header block with no usable `Content-Length` is reported and skipped: the
  decoder scans for the next `Content-Length` in what is left. That is the only
  way back from a server that writes something unframed (a panic message) into
  its stdout.
- `MAX_BODY` (64 MiB) is refused rather than buffered, and `end()` turns a body
  cut in half by the end of the stream into `FrameError::Truncated`.

**`jsonrpc`** — `RawMessage`, a `serde_json::Value` that knows what it is.

- `kind()`, `is_request()`, `is_response()`, `is_notification()`, `id()`,
  `method()`, `params()`, `result()`, `error_object()`, and constructors for
  each shape. A null `id` counts as no id, since nothing can be routed by it.
- Fields the core has never heard of survive a round trip byte for byte, which
  is what lets the proxy pass server extensions through untouched.
- `Display` writes one line for the log ("request initialize id=1"), never the
  body.

**`transport`** — a server on the other end of a pipe.

- `Transport::spawn(name, spec)` over `tinkerpad-process`, forcing a stdin pipe.
  `send`/`sender()` queue outgoing messages; `recv()`/`incoming()` yield
  `Message`, `Broken`, and a final `Exited`.
- The reader task unframes stdout and keeps going after a `Broken`: only the
  process ending ends the stream. The writer task frames and flushes one message
  at a time. Both queues hold 256 messages, so a stalled server slows its sender
  rather than growing a buffer.
- stderr is logged line by line under an `lsp{server, pid}` span, handling lines
  split across chunks, CRLF, and a last line with no newline.

**`tinkerpad-fakelsp`** — a stand-in server (140 lines) that speaks real
framing: `initialize` with capabilities and its pid, `test/echo`, `test/log`
(sends N `window/logMessage` notifications), `test/big`, `test/stderr`,
`test/halt` (writes half a frame and closes stdout), `shutdown`, and `exit`
(0 after a shutdown, 1 otherwise). Used here, and by 02-02 and 02-05.

**Measurements** (Windows 11, release build)

| What | Result |
|---|---|
| 1,000 `test/echo` round trips | 9.1 ms total, 9 µs each, all in order |
| A 2 MiB response | arrives whole, and the connection keeps working |

**Tests** — 21 unit, 8 against the fake server.

- Framing: round trip, header order and case, bare newlines, several messages
  per chunk, every possible split point of one frame, zero-length body, resync
  after a missing and after a non-numeric length, over-size refusal, truncation
  at end of stream, and a 2 MiB body delivered in 8 KiB chunks.
- JSON-RPC: classification of all four shapes, string ids, null ids, byte-exact
  round trip, unknown fields kept, non-objects refused, log line.
- Transport: the initialize handshake (including that the answering process is
  the pid the transport reports), 1,000 ordered round trips, a 2 MiB message,
  notifications arriving before the response they precede, stdout closing
  mid-message (a framing error, then the exit, then nothing, no panic and no
  hang), `kill()`, an error response leaving the session usable, and stderr
  chatter not disturbing the message stream.

**Docs** — ARCHITECTURE.md §4.3 now describes the three modules, the resync
rule, the bounded queues, and the fake server.

## Checklist status

**Deviation: `spawn` returns one value.** The item asked for
`Transport::spawn(SpawnSpec) -> (Transport, Supervised)`. `Supervised` owns the
single output stream the reader task has to consume, so handing it back would
mean two owners of one stream. `Transport` therefore keeps it and exposes what
the pair was for: `pid()`, `kill()`, `kill_handle()`, and the `Exited` event
that ends the incoming stream. `Transport::over(name, child, stdin)` takes a
child someone else spawned, which is how 02-02 will build specs its own way.
