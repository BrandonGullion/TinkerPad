# 02-02 · Server pool: spawn, share, keep alive, shutdown

**Phase:** 2 · **Size:** M · **Crate/area:** `tinkerpad-lsp` · **Depends on:** 02-01, 01-01, 01-03
**Design:** ARCHITECTURE.md §4.1, PLAN.md §3.3

## What

`LspPool`: a map from `(language, workspace_root)` to a `ServerHandle`. Servers are
spawned lazily on first use, shared by every playground of that language, kept
alive when tabs close, and shut down cleanly on app exit.

## Why

This is the "keep language servers alive" requirement in its purest form. Once this
exists, the second tab in a language is instant.

## Expectations

- [x] `LspPool::attach(playground) -> Result<Attachment>`: finds or spawns the
      server for the playground's language and workspace root (per the plugin's
      `[lsp]`), runs `initialize`/`initialized` once per server, sends
      `textDocument/didOpen`. `Attachment` is dropped on tab close → `didClose`.
      The server stays running.
- [x] `ServerHandle` tracks `ServerState` (`Stopped`, `Starting`, `Ready`,
      `Crashed`, `Stopping`) in a `watch` channel and emits `CoreEvent::ServerState`
      changes for the UI.
- [x] `initialize` params come from the plugin: root URI from the workspace
      strategy (02-07; for now `playground_dir` and `library_root` are enough),
      `initializationOptions` from `[lsp.init_options]`, and a fixed client
      capabilities object that matches what the editor client (02-04) supports.
- [x] Requests made while `Starting` are queued and flushed on `Ready`, with a
      10 s cap after which they fail with a clear error.
- [x] `open_docs` is tracked per server (URI, version, text) for replay in 02-05.
- [x] App exit: all servers get `shutdown` → `exit` concurrently, 2 s grace, then
      kill. Exit is never blocked more than 3 s total.
- [x] Spawn failure (server binary missing) yields `ServerState::Stopped` with a
      reason and the plugin's `install_hint`; the editor keeps working without LSP.
- [x] Tests with the fake server: two attachments share one process; dropping both
      leaves it running; pool shutdown terminates it; missing binary path is a
      typed error.

## Out of scope

Proxying to the editor (02-03), crash restart (02-05), idle/pre-warm (02-06),
scratch workspaces (02-07).

## Gotchas

`textDocument/didChange` needs monotonically increasing versions per document.
Own the version counter in `open_docs`; never trust the editor's number.

## Results

Implemented 2026-09-16.

**Core**
- `ServerState`: `Stopped { reason, hint }`, `Starting`, `Ready { pid, name,
  version }`, `Crashed { reason }`, `Stopping`, and `CoreEvent::Server
  { language, root, state }` so the UI can show it (02-08 renders it).

**`server.rs` — one language server**
- `ServerConfig` is plain data: language, root, a `SpawnSpec`,
  `initializationOptions`, an install hint, and a start timeout. The app builds
  it from the plugin's `[lsp]` section, so `tinkerpad-lsp` never reads
  `plugin.toml` and the dependency direction in the root `Cargo.toml` holds.
- `ServerHandle::start` spawns the transport and runs `initialize` /
  `initialized` in the background: the state goes `Starting` → `Ready`, carrying
  the pid and whatever the server called itself.
- `request()` waits for the handshake if one is in progress, so a request made
  the instant a tab opens is answered rather than refused. The wait is capped
  (10 s by default, configurable per server) and fails with "did not finish
  starting in time".
- One router task owns the wire: responses go to the `oneshot` that is waiting,
  everything else is broadcast for the proxy (02-03). When the process exits,
  every waiting request is woken rather than left hanging.
- `open_docs` holds each URI's language, version, and text. The core owns the
  version counter — LSP requires versions to increase per document, and the
  editor's numbering can't be trusted across a reload — and the text is what
  02-05 will replay after a restart.
- `did_open` on a URI that is already open is a `didChange`, not a second open.
- `stop()` is `shutdown` → `exit` → wait → kill, each step under its own
  deadline.

**`pool.rs` — the pool**
- `LspPool::attach(config, uri, text)` finds or starts the server for
  `(language, root)`, waits for the handshake, sends `didOpen`, and returns an
  `Attachment`. Dropping the attachment sends `didClose` and nothing else: the
  server stays up, which is the whole point of the pool.
- A server that fails to spawn is remembered in the `Stopped` state with the
  reason and the plugin's `install_hint`, and every request to it fails
  immediately, so a missing language server never stops the editor working.
- `shutdown()` stops every server at once, under a 3 s budget for the lot, then
  kills whatever is left.
- State changes are forwarded to the UI as `CoreEvent::Server` until the server
  stops.

**App**
- `lsp.rs` builds the pool, forwards its events to the event bus, and the Tauri
  `RunEvent::Exit` hook stops every server before the process goes away. Nothing
  attaches yet — that is the editor client, 02-04 — so the pool is empty until
  then.

**`tinkerpad-fakelsp`** gained document tracking (`didOpen`/`didChange`/
`didClose`, complaining on stderr about a double open or a version that doesn't
increase), a `test/docs` request that reports its pid and what it has open, and
`TINKERPAD_FAKELSP_INIT_DELAY_MS` to hold up the handshake the way a real server
loading a workspace does.

**Measurements** (Windows 11, release build, against the fake server)

| What | Result |
|---|---|
| First attach: spawn + handshake + `didOpen` | 10.4 ms |
| Second playground attaching to the same server | 4.5 µs |
| Stopping three servers at app exit | 0.9 ms |

The second tab costs about two thousandths of the first, which is the point of
the item: a shared server makes it a `didOpen` and nothing more.

**Tests** — 9 against real processes.

- Two playgrounds share one process; closing one tab closes only its document;
  closing the last leaves the server running; a third playground reuses it.
- A different root is a different server; the same root twice is one.
- Versions increase by one per change whatever the caller does, the text is kept
  for replay, and opening an open URI is a change rather than a second open.
- A request made while the server is still starting waits for the handshake and
  succeeds; one that never finishes fails with a clear error.
- A missing server binary reports the reason and the `install_hint`, the pool
  remembers it, and the UI hears about it.
- State changes reach the UI in order (`Starting` → `Ready` → `Stopped`).
- `shutdown()` stops every server well inside the budget, and a server whose
  stdin is gone is killed rather than waited for.

**Docs** — ARCHITECTURE.md §4.1 now carries the real `LspPool` and
`ServerHandle` shapes, the `ServerConfig` boundary, what `attach` does, and the
3 s exit budget.

## Checklist status

**Deviation: `attach` takes a config, not a playground.** The item wrote
`LspPool::attach(playground)`. A playground's language, kind, and `[lsp]`
section live in `tinkerpad-plugin`, which `tinkerpad-lsp` must not depend on
(root `Cargo.toml`, ARCHITECTURE.md §2). So the pool takes a `ServerConfig` and
a document, and the app builds that config from the plugin. The knowledge stays
in the app, where 02-04 and 02-07 will need it anyway.

**Open: the workspace strategy is not applied yet.** `ServerConfig.root` takes
whatever root it is given, and `playground_dir` / `library_root` are simply two
different roots as far as the pool is concerned. Turning a plugin's
`[lsp].workspace` into a root is 02-07, and nothing attaches before 02-04, so
there is nothing to apply it to yet.

**Open: no in-app self-test.** The pool has no bundled plugin to drive it —
none of the three plugins has an `[lsp]` section — and the fake server binary
isn't shipped with the app. The Rust tests cover the behaviour; the in-app
scenario arrives with 02-04, when a real server is attached to a real tab.
