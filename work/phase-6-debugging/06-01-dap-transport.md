# 06-01 · DAP transport and client

**Phase:** 6 · **Size:** M · **Crate/area:** `tinkerpad-dap` (new) · **Depends on:** 02-01
**Design:** DEBUGGING.md §1, D4

## What

A client that speaks the Debug Adapter Protocol to a child process: the
`Content-Length` framing from 02-01, DAP's `request`/`response`/`event` message
shapes, and the sequence numbering that pairs a response with its request.

No pool, no UI, no adapter: one type that can be handed a spawned process and
asked things.

## Why

DAP is LSP's sibling — the same framing with a different envelope — and D4 split
that framing into its own module so this is a new envelope rather than a new
transport. Doing it alone, against a fake adapter, is the half that can be
tested without any language being involved.

## Expectations

- [ ] The framing module from `tinkerpad-lsp` is reused rather than copied. If
      it has to move somewhere both crates can depend on, that move is part of
      this item.
- [ ] `request` returns a future resolving to the matching response, paired on
      `seq`/`request_seq`. A response nobody is waiting for is logged and
      dropped, not panicked on.
- [ ] Events are a broadcast stream. `stopped`, `continued`, `terminated`,
      `exited`, `output` and `initialized` are typed; anything else arrives as
      its raw payload rather than being discarded.
- [ ] A request that is never answered times out and says which request it was
      — except `launch`, `attach` and `configurationDone`, which wait as long as
      the program takes (D12).
- [ ] Reverse requests (`runInTerminal`, `startDebugging`) are answered with a
      refusal rather than ignored, so an adapter that asks does not hang.
- [ ] A fake adapter in the crate's own `bin/`, like `tinkerpad-fakeprobe`, so
      every case runs on every OS: a handshake, a stop, an unanswered request, a
      malformed frame, and an adapter that exits mid-session.

## Out of scope

Supervision, restarts and idle policy (06-02). Anything a language needs (06-07
onward). The UI.

## Gotchas

DAP's `seq` is per-direction and starts at 1; a response carries both its own
`seq` and the `request_seq` it answers. Getting that wrong produces a client
that works until two requests are in flight.

`initialize` is answered *before* `initialized` is emitted, and the capabilities
in that response decide which later requests are even legal. Keep them.
