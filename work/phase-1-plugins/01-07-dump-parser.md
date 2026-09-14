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

- [ ] `RichValue` in `tinkerpad-core` with `v`, `kind` (value, table, tree, text,
      html, image, timing), optional `title`/`id`, and per-kind fields per
      PLUGINS.md §6. `serde` with `#[serde(tag = "kind")]`. Unknown `kind` parses
      into a `RichValue::Unknown { raw }` rather than failing.
- [ ] Parser handles: a sentinel split across chunk boundaries at any byte; text
      before and after a sentinel on the same line; multiple sentinels per chunk;
      a sentinel with malformed JSON (emit the raw payload as stderr-styled text
      plus a `Warning` event); an unterminated sentinel at process exit (flush as
      text).
- [ ] Payload cap: 8 MiB default. Beyond it, stop buffering, emit a
      `RichValue::Truncated { bytes }` marker, resume scanning for BEL.
- [ ] Also accept `ST` (`ESC \`) as the terminator, not only `BEL`, since some
      libraries emit it.
- [ ] Throughput: 100 MB of plain stdout with no sentinels passes through with
      negligible overhead (benchmark, record the number).
- [ ] Table-driven tests from a `tests/dump_cases/*.txt` fixture set: input bytes,
      chunk split points, expected event sequence. Include one captured from each
      bundled dump library once they exist (01-09..11 add theirs).
- [ ] A `tinkerpad-core::dump::examples()` returning one sample of every kind, used
      by the UI renderer tests and the docs.

## Out of scope

Rendering (01-08). The dump libraries themselves (per plugin). Lazy expansion via `id`.

## Gotchas

Write the parser over `&[u8]`, not `&str`; user output may contain invalid UTF-8
and must still pass through. Convert to text lossily only at the UI boundary.
