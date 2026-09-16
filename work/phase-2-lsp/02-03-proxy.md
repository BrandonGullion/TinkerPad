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

- [x] One `Channel` per open tab from the UI; the proxy tags each with its
      playground id and attachment.
- [x] URI rewriting table covers: `textDocument.uri` in all `textDocument/*`
      requests and notifications, `publishDiagnostics.uri`, `Location`/
      `LocationLink` in definition/references/implementation results,
      `WorkspaceEdit.changes` keys and `documentChanges[].textDocument.uri`,
      `workspace/symbol` results. Implemented as a small recursive walk over known
      paths per method, with a test that lists every rewritten method.
- [x] Reverse mapping for server → editor: a real path that isn't an open
      playground (go-to-definition into a dependency) is passed through as a
      `file:///` URI so 02-09 can open it read-only.
- [x] Request ids: editor ids are per tab; the core allocates global ids and keeps
      `(server, core_id) → (tab, editor_id)`. Responses are routed to the right tab.
- [x] Server → editor requests (`workspace/configuration`, `client/registerCapability`,
      `window/workDoneProgress/create`) are answered by the core, not forwarded,
      with sensible defaults. `window/showMessage` becomes a toast; `logMessage`
      goes to the log.
- [x] Dropping a tab's channel cancels its in-flight requests (`$/cancelRequest`
      to the server) and drops their pending entries.
- [x] Every message is logged at `trace` with direction, method, and size; a
      per-server counter of messages and p50/p95 latency for requests is kept for
      02-08.
- [x] Tests: URI rewrite round-trips for each method in the table; id remap with
      two tabs interleaving; a tab dropped mid-request produces no stray response.

## Out of scope

The editor side (02-04). Position rewriting (none, by design).

## Gotchas

Diagnostics arrive as one `publishDiagnostics` per document; fan them out to only
the tab that owns that playground.

## Results

Implemented 2026-09-16.

**`uri.rs` — two namings of one file**
- `UriMap` maps `tinkerpad://playground/<id>` ↔ `file:///…/main.rs` for every
  open playground, comparing file URIs on a normalised form so a server that
  echoes a Windows path back in another case or slash direction still matches.
- Rewriting walks a message for the keys that hold URIs — `uri`, `targetUri`,
  `newUri`, `oldUri`, `documentUri`, `externalUri`, and the keys of a
  `WorkspaceEdit.changes` object — instead of following a path per method.
  Everything else passes through byte for byte.
- A file URI that is not an open playground is left alone, so a
  go-to-definition into a dependency arrives at the editor as a `file:///` URI
  for 02-09 to open read-only.

**`proxy.rs` — the core in the middle**
- `open_tab(playground, main_file, attachment, sink)` connects one tab; the
  `EditorSink` is where its messages go (a Tauri channel in the app, a vector in
  tests). `close_tab` cancels what the tab was waiting for, drops the
  attachment (so `didClose` goes out), and leaves the server up.
- Requests get a core-owned id, with `core_id → (tab, editor_id, method,
  started)` kept until the answer comes back; the answer is handed to the tab
  that asked, under the id it used. Two tabs can both use id 1, which is the
  case the ownership exists for.
- A request to a server that isn't running is answered with an error, never
  silence: the editor must not be left waiting.
- `workspace/configuration` (one `null` per item), `client/registerCapability`,
  `client/unregisterCapability`, and `window/workDoneProgress/create` are
  answered by the core; anything else the server asks gets a proper
  "method not found". None of it reaches the editor.
- `window/showMessage` becomes a toast naming the server, `window/logMessage` a
  log line, `textDocument/publishDiagnostics` goes to the one tab that shows
  that document, and anything else is fanned out to every tab on that server
  with its URIs translated.
- Every message is logged at `trace` with direction, method, and size; per-server
  counts and p50/p95 request latency are kept for the status strip (02-08).

**`server.rs`** gained `begin_request` (hands back the core id and the answer to
come), `cancel_request` (`$/cancelRequest`, and forget it), and `send` for a
message the caller has already built. `request` is now a thin wrapper over
`begin_request`, so there is one path to the wire.

**App**
- `lsp.rs` builds a `ServerConfig` from the plugin's `[lsp]` section: the
  command resolved through the same settings overrides and `PATH` a run uses,
  `{dir} {main} {plugin_dir} {root} {name}` filled in, and the root chosen by
  the workspace strategy (`none`, `playground_dir`, `library_root`;
  `scratch_workspace` warns and uses the playground folder until 02-07).
- Commands `lsp_open` (takes the tab's `Channel`, returns the document URI),
  `lsp_send`, and `lsp_close`. The editor side of them is 02-04.

**`tinkerpad-fakelsp`** gained `test/wait`, `test/publish`, `test/tell`, and
`test/ask` — which asks the client the three questions real servers ask, plus
one it shouldn't, and reports what it was told.

**Measurements** (Windows 11, release)

| What | Result |
|---|---|
| Request round trip, editor → proxy → server → editor | p50 98 µs, p95 125 µs |

**Tests** — 10 unit (7 URI, 3 proxy), 10 against real processes.

- Every method in the rewrite table round-trips editor → server → editor
  unchanged, URIs inside results (`Location`, `LocationLink`) are rewritten, a
  dependency keeps its file URI, a Windows path matches in any case or slash
  direction, and nothing that is not a URI is touched.
- The editor names a playground and the server sees a file, with the answer
  coming back in the editor's naming under the editor's id.
- Two tabs interleaving 20 requests each, both using id 1: every answer reaches
  the right tab, in order, and neither sees the other's.
- A tab closed mid-request: the pending entry goes, `$/cancelRequest` is sent,
  and the late answer produces no stray response and no panic.
- Diagnostics reach only the tab that owns the document.
- The core answers the server's questions itself, and the editor never sees
  them.
- `window/showMessage` becomes a toast of the right level, naming the server.
- Traffic is counted and requests are timed.

**Docs** — ARCHITECTURE.md §4.2 now describes the walk-by-key rule and why, id
ownership and cancellation, and which server-to-client requests the core
answers.

## Checklist status

**Deviation: a walk by key, not a table of paths per method.** The item asked
for "a small recursive walk over known paths per method". Servers send fields
this core has never heard of — `rust-analyzer/…` extensions, custom code
actions — and those carry URIs too; a path table would hand the editor URIs it
cannot name. Walking for URI-shaped keys covers those as well, and
`uri::REWRITTEN_METHODS` keeps the per-method list as the checklist the tests
run through, which is what the criterion was really asking for.

**Open: nothing drives this from the UI yet.** The commands exist
(`lsp_open`/`lsp_send`/`lsp_close`), but the editor client that opens a channel
is 02-04, and no bundled plugin has an `[lsp]` section yet, so there is no
in-app self-test for this item. The Rust tests cover the behaviour against real
processes.
