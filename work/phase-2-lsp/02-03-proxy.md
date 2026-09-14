# 02-03 · Proxy: URI rewriting and id ownership

**Phase:** 2 · **Size:** M · **Crate/area:** `tinkerpad-lsp`, `tinkerpad-app` · **Depends on:** 02-02, 00-05
**Design:** ARCHITECTURE.md §4.2

## What

The bridge between the editor's LSP client and the pooled server. Editor-side
messages arrive over a Tauri `Channel`, get their URIs rewritten from
`tinkerpad://playground/<id>` to the real `file:///` path and their request ids
remapped to core-owned ids, and go to the server. Responses and server-initiated
messages come back the other way.

## Why

Sitting in the middle is what lets one server serve many tabs, lets the core
synthesise error responses when a server dies, and keeps the editor ignorant of
where files live.

## Expectations

- [ ] One `Channel` per open tab from the UI; the proxy tags each with its
      playground id and attachment.
- [ ] URI rewriting table covers: `textDocument.uri` in all `textDocument/*`
      requests and notifications, `publishDiagnostics.uri`, `Location`/
      `LocationLink` in definition/references/implementation results,
      `WorkspaceEdit.changes` keys and `documentChanges[].textDocument.uri`,
      `workspace/symbol` results. Implemented as a small recursive walk over known
      paths per method, with a test that lists every rewritten method.
- [ ] Reverse mapping for server → editor: a real path that isn't an open
      playground (go-to-definition into a dependency) is passed through as a
      `file:///` URI so 02-09 can open it read-only.
- [ ] Request ids: editor ids are per tab; the core allocates global ids and keeps
      `(server, core_id) → (tab, editor_id)`. Responses are routed to the right tab.
- [ ] Server → editor requests (`workspace/configuration`, `client/registerCapability`,
      `window/workDoneProgress/create`) are answered by the core, not forwarded,
      with sensible defaults. `window/showMessage` becomes a toast; `logMessage`
      goes to the log.
- [ ] Dropping a tab's channel cancels its in-flight requests (`$/cancelRequest`
      to the server) and drops their pending entries.
- [ ] Every message is logged at `trace` with direction, method, and size; a
      per-server counter of messages and p50/p95 latency for requests is kept for
      02-08.
- [ ] Tests: URI rewrite round-trips for each method in the table; id remap with
      two tabs interleaving; a tab dropped mid-request produces no stray response.

## Out of scope

The editor side (02-04). Position rewriting (none, by design).

## Gotchas

Diagnostics arrive as one `publishDiagnostics` per document; fan them out to only
the tab that owns that playground.
