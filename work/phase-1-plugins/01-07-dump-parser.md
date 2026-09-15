# 01-07 · Dump sentinel parser

**Phase:** 1 · **Size:** M · **Crate/area:** `tinkerpad-runner`, `tinkerpad-core` · **Depends on:** 00-04, 01-06
**Design:** ARCHITECTURE.md §5.3, PLUGINS.md §6

## What

A state machine over the stdout byte stream that finds `ESC ] 1337 ; TinkerPad=
<json> BEL` sequences, parses the JSON into `RichValue`, and emits
`RunEvent::Rich(RichValue)` in order with the surrounding plain text. Plus the
`RichValue` types themselves.

## Why

`Dump()` is a core promise (PLAN.md §1). The parser is the only piece of it the
core owns, and it must be bulletproof: a broken helper library in someone's plugin
must degrade to visible text, never to lost output or a frozen UI.

## Expectations

- [x] `RichValue` in `tinkerpad-core` with `v`, `kind` (value, table, tree, text,
      html, image, timing), optional `title`/`id`, and per-kind fields per
      PLUGINS.md §6. `serde` with `#[serde(tag = "kind")]`. Unknown `kind` parses
      into a `RichValue::Unknown { raw }` rather than failing.
- [x] Parser handles: a sentinel split across chunk boundaries at any byte; text
      before and after a sentinel on the same line; multiple sentinels per chunk;
      a sentinel with malformed JSON (emit the raw payload as stderr-styled text
      plus a `Warning` event); an unterminated sentinel at process exit (flush as
      text).
- [x] Payload cap: 8 MiB default. Beyond it, stop buffering, emit a
      `RichValue::Truncated { bytes }` marker, resume scanning for BEL.
- [x] Also accept `ST` (`ESC \`) as the terminator, not only `BEL`, since some
      libraries emit it.
- [x] Throughput: 100 MB of plain stdout with no sentinels passes through with
      negligible overhead (benchmark, record the number).
- [ ] Table-driven tests from a `tests/dump_cases/*.txt` fixture set: input bytes,
      chunk split points, expected event sequence. Include one captured from each
      bundled dump library once they exist (01-09..11 add theirs).
- [x] A `tinkerpad-core::dump::examples()` returning one sample of every kind, used
      by the UI renderer tests and the docs.

## Out of scope

Rendering (01-08). The dump libraries themselves (per plugin). Lazy expansion via `id`.

## Gotchas

Write the parser over `&[u8]`, not `&str`; user output may contain invalid UTF-8
and must still pass through. Convert to text lossily only at the UI boundary.

## Results

Implemented 2026-09-15.

**Pieces**
- `tinkerpad-core` `dump.rs`: `RichValue { v, title, id, node }`, where `node` is
  a `RichNode` tagged by `kind`:
  - the seven kinds from PLUGINS.md §6, plus `Unknown { raw }` and
    `Truncated { bytes }`;
  - a missing `v` means 1, and an unknown `kind` keeps the whole payload raw;
  - a known kind with missing fields is an error, so it shows as malformed;
  - `dump::examples()` returns one of each kind, and a test round-trips them.
- `tinkerpad-runner` `dump.rs`: `DumpParser`, a byte-level state machine.
  - It finds the prefix with a SIMD substring search and keeps a partial prefix
    at a chunk end for the next chunk.
  - Payloads end at `BEL` or `ESC \`.
  - Text pieces borrow from the input, so plain output isn't copied.
  - Over the cap, the payload buffer is freed and only its size is counted.
- `tinkerpad-runner` `output.rs`: `OutputDecoder` turns stdout and stderr bytes
  into UI run events.
  - Stdout goes through the parser, then the UTF-8 decoder.
  - A malformed payload becomes stderr text plus a `warning` event.
  - Only stdout is scanned.
- `Utf8Stream` gained `push_into`, which no longer copies each chunk.
- Core `RunEvent` gained `rich { value }` and `warning { message }`, mirrored in
  `ui/src/protocol.ts` along with the `RichValue` types.
- The app's run forwarder uses `OutputDecoder` and logs dump and warning counts
  per run.
- UI: until 01-08 renders values, the output pane shows a one-line marker such as
  `[dump: table "users"]` where each value was, and `⚠ …` for warnings.
- PLUGINS.md §6 now says what the core does with odd input: `ST`, unknown kinds,
  malformed payloads, the cap, unterminated sentinels, and stderr.

**Tests**
- 12 case files in `tinkerpad-runner/tests/dump_cases/`, covering:
  - plain text, including ANSI and other OSC codes;
  - text around a sentinel, and several sentinels in one chunk;
  - `ST`, and mixing `ST` with `BEL`;
  - malformed payloads: bad JSON, missing fields, empty, a lone ESC;
  - a sentinel unterminated at exit;
  - false and partial prefixes;
  - the cap: over, exactly at, ended by `ST`, and still open at exit;
  - an unknown kind, invalid UTF-8, and one of every kind.

  Each case runs as one chunk, one byte at a time, split in two at every
  position, and at listed split points. Breaking an expectation on purpose made
  the test fail with an expected-versus-actual listing.
- The real 8 MiB cap: a payload of exactly 8 MiB parses, and one byte more
  becomes a marker.
- Unit tests for the schema, the decoder, and split UTF-8.
- New in-app `dump` self-test scenario, run by a real Node program. Six checks
  pass on Windows (release build), and CI runs the scenario on macOS and Linux:
  - events arrive in order with the text around them;
  - value fields are kept;
  - a 20,000-row table spanning many pipe reads parses whole;
  - malformed JSON shows as text with a warning;
  - `ST` works;
  - the output pane marks each dump.

  All other scenarios still pass.

**Throughput** (`cargo run --release -p tinkerpad-runner --example dump_throughput`,
Windows 11, best of 5, 100 MB with no sentinels)

| Input | Chunk | Decoder only | With parser |
|---|---|---|---|
| Plain log lines | 8 KiB | 7,516 MB/s | 6,093 MB/s |
| Plain log lines | 64 KiB | 6,762 MB/s | 6,625 MB/s |
| Lines with ANSI colours | 8 KiB | 9,770 MB/s | 7,674 MB/s |
| Lines with ANSI colours | 64 KiB | 9,343 MB/s | 8,690 MB/s |

100 MB takes 11 to 16 ms with the parser. The first version scanned for every
ESC byte and ran at 1,600 MB/s on coloured output. Searching for the whole prefix
fixed that.

## Checklist status

**Ticked with a note**
- *`RichValue::Unknown { raw }` and `RichValue::Truncated`.* These are
  `RichNode::Unknown` and `RichNode::Truncated` inside `RichValue`. That's because
  `v`, `title`, and `id` belong to every kind. On the wire it's the same flat
  object with a `kind` tag.

**Open**
- *Include a captured case from each bundled dump library.* Those libraries don't
  exist yet. 01-09 to 01-11 add their captures to `tests/dump_cases/`.
