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

- [x] Unexpected exit (not after `shutdown`) → `ServerState::Crashed { restarts }`
      event, toast "typescript-language-server crashed, restarting…".
- [x] Backoff 1 s, 2 s, 4 s, 8 s, 16 s, cap 30 s; after 5 consecutive crashes within
      10 minutes, stop and show "gave up; Restart in the status strip". A manual
      restart resets the counter.
- [x] Pending requests receive a JSON-RPC error response (`code: -32000`, message
      "server restarted") routed to the right tab.
- [x] Replay: `didOpen` with the *current* text and a fresh version for each open
      doc; then diagnostics reappear on their own.
- [x] The editor client (02-04) treats the error responses as transient (no red
      banner) and retries completion on the next keystroke.
- [x] Tests with the fake server's `test/crash` request: crash → restart → echo
      works again; pending request gets the error; five crashes → gave up state.

## Out of scope

Detecting a hung (alive but unresponsive) server. Add a health ping later if it
happens in practice.

## Gotchas

Don't replay `didChange` history. The current text is the whole state a server
needs.

## Results

Implemented 2026-09-16.

**The handle outlives the process.** A `ServerHandle` now holds its pipe in a
`Connection` that is replaced wholesale on a restart, so tabs, attachments, and
the proxy keep the same `Arc` and never learn that the process changed. Only the
state says so.

**What a crash does**

- An exit nobody asked for (not after `shutdown`, not from our own kill) drops
  the connection, wakes every request waiting on it, counts the crash, and
  schedules a restart: 1 s, 2 s, 4 s, 8 s, 16 s, then 30 s. The state carries the
  count — `Crashed { reason, restarts }` — and the pool turns each one into a
  toast ("fake crashed, restarting… (attempt 2)").
- Crashes further apart than ten minutes are not "in a row": a server that has
  been fine all afternoon and then dies gets the full budget again.
- Six in a row and the pool stops trying. The server sits in `Stopped` with
  "it crashed 6 times in a row: …" and the hint "Restart it from the status
  strip when you have fixed the cause". Requests then fail immediately instead
  of waiting for a server nobody is bringing back.
- Opening a tab in that language starts it again, because that is a person
  asking for it; so does `LspPool::restart`, which is the status strip's button
  (02-08). Either resets the count.

**What the editor sees**

- A request in flight when the server goes gets a JSON-RPC error with code
  `-32000` and "the language server restarted before answering …", routed to the
  tab that asked. `@codemirror/lsp-client` swallows failures for completion and
  hover, so nothing is shown to the user; only explicitly invoked actions
  (format, rename) report, and those arrive in 02-09.
- After the handshake, every open document is sent again as `didOpen` with the
  text and version the editor last had. Never the change history: the text is
  the whole state a server needs. Diagnostics come back on their own.

**Configurable for tests.** `ServerConfig::with_backoff` shortens the ladder, so
the restart tests run in 0.9 s instead of sitting through 31 s of real waits; the
ladder itself is checked by a unit test.

**Measurements** (Windows 11, release, the fake server)

| What | Result |
|---|---|
| Crash to answering again, minus the backoff | 41 ms (61 ms measured with the tests’ 20 ms base) |
| So, with the default ladder | ~1.04 s for the first crash, ~2.04 s for the second |
| Restart tests, whole file | 1.18 s |
| Restart under a live tab, rust-analyzer, to completions again | within the 2 s the scenario allows per attempt |

**Tests** — 5 against real processes, 1 unit.

- A crashed server comes back with a new pid and answers again, with the same
  handle throughout.
- The documents are open again afterwards, at the version and text the editor
  last sent — not the text they were opened with.
- A request in flight when the process dies gets `-32000` back at the tab that
  asked, and nothing is left waiting.
- Six crashes in a row: the state says it gave up, requests fail rather than
  hang, the toasts were published, and asking for the server again (opening a
  tab) starts it and resets the count.
- A manual restart replaces the process and reopens the documents on the new
  one.
- The backoff ladder: 1, 2, 4, 8, 16, 30, 30, 30.

In the app, the `lsp` scenario gained three checks: a language server can be
restarted under a live tab, completions come back without the editor reopening
anything, and no error banner is shown for the requests the restart lost.

**Docs** — ARCHITECTURE.md §4.1's crash-restart rule now describes what was
built: the counts, the give-up, what survives, and what the editor is told.

## Checklist status

**Deviation: five restarts, not five crashes.** The item says "after 5
consecutive crashes … stop". As built, five crashes are *restarted* and the
sixth is where the pool gives up, so `MAX_RESTARTS = 5` is a budget of restarts
rather than of crashes. The difference is one attempt; naming it after what the
pool does seemed clearer than after what it refuses.

**Note: no in-app crash test.** rust-analyzer has no "please crash" request, and
killing it through the app's own handle is a *stop*, not a crash, so the in-app
scenario verifies the restart path (which shares everything after the decision
to restart) and the Rust tests verify the crash path against a process that
really does die.
