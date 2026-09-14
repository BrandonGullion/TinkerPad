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

- [ ] `LspPool::attach(playground) -> Result<Attachment>`: finds or spawns the
      server for the playground's language and workspace root (per the plugin's
      `[lsp]`), runs `initialize`/`initialized` once per server, sends
      `textDocument/didOpen`. `Attachment` is dropped on tab close → `didClose`.
      The server stays running.
- [ ] `ServerHandle` tracks `ServerState` (`Stopped`, `Starting`, `Ready`,
      `Crashed`, `Stopping`) in a `watch` channel and emits `CoreEvent::ServerState`
      changes for the UI.
- [ ] `initialize` params come from the plugin: root URI from the workspace
      strategy (02-07; for now `playground_dir` and `library_root` are enough),
      `initializationOptions` from `[lsp.init_options]`, and a fixed client
      capabilities object that matches what the editor client (02-04) supports.
- [ ] Requests made while `Starting` are queued and flushed on `Ready`, with a
      10 s cap after which they fail with a clear error.
- [ ] `open_docs` is tracked per server (URI, version, text) for replay in 02-05.
- [ ] App exit: all servers get `shutdown` → `exit` concurrently, 2 s grace, then
      kill. Exit is never blocked more than 3 s total.
- [ ] Spawn failure (server binary missing) yields `ServerState::Stopped` with a
      reason and the plugin's `install_hint`; the editor keeps working without LSP.
- [ ] Tests with the fake server: two attachments share one process; dropping both
      leaves it running; pool shutdown terminates it; missing binary path is a
      typed error.

## Out of scope

Proxying to the editor (02-03), crash restart (02-05), idle/pre-warm (02-06),
scratch workspaces (02-07).

## Gotchas

`textDocument/didChange` needs monotonically increasing versions per document.
Own the version counter in `open_docs`; never trust the editor's number.
