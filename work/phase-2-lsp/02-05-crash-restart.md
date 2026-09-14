# 02-05 · Crash restart and replay

**Phase:** 2 · **Size:** S · **Crate/area:** `tinkerpad-lsp` · **Depends on:** 02-02, 02-03
**Design:** ARCHITECTURE.md §4.1

## What

When a server exits unexpectedly, mark it `Crashed`, wait with exponential backoff,
respawn, re-`initialize`, replay `didOpen` for every tracked document, and carry on.
In-flight requests get a synthesised error response so the editor doesn't hang.

## Why

rust-analyzer and Roslyn both crash occasionally. A scratchpad that needs a restart
when they do is not "kept alive".

## Expectations

- [ ] Unexpected exit (not after `shutdown`) → `ServerState::Crashed { restarts }`
      event, toast "typescript-language-server crashed, restarting…".
- [ ] Backoff 1 s, 2 s, 4 s, 8 s, 16 s, cap 30 s; after 5 consecutive crashes within
      10 minutes, stop and show "gave up; Restart in the status strip". A manual
      restart resets the counter.
- [ ] Pending requests receive a JSON-RPC error response (`code: -32000`, message
      "server restarted") routed to the right tab.
- [ ] Replay: `didOpen` with the *current* text and a fresh version for each open
      doc; then diagnostics reappear on their own.
- [ ] The editor client (02-04) treats the error responses as transient (no red
      banner) and retries completion on the next keystroke.
- [ ] Tests with the fake server's `test/crash` request: crash → restart → echo
      works again; pending request gets the error; five crashes → gave up state.

## Out of scope

Detecting a hung (alive but unresponsive) server. Add a health ping later if it
happens in practice.

## Gotchas

Don't replay `didChange` history. The current text is the whole state a server
needs.
