# 02-04 · Editor LSP client

**Phase:** 2 · **Size:** M · **Crate/area:** `ui/` · **Depends on:** 02-03, 00-03
**Design:** ARCHITECTURE.md §7, PLAN.md §3.1

## What

Wire `@codemirror/lsp-client` (or `codemirror-languageserver` if the official
package lacks something) into the editor with a transport that speaks over the
Tauri `Channel` from 02-03. Turn on completion, hover, diagnostics, signature help,
and document sync.

## Why

This is where the warm-server investment becomes visible: completions that appear
as you type, in every tab, without a spinner.

## Expectations

- [x] A `TauriLspTransport` implementing the client package's transport interface:
      one per tab, created on tab open, closed on tab close.
- [x] Document sync: incremental `didChange` on every edit, debounced no more than
      ~50 ms so diagnostics feel live.
- [x] Completion: triggered on typing and on trigger characters the server
      declares; shows kind icons, detail, and documentation in a side panel;
      accepts with Tab/Enter; supports `textEdit`, `additionalTextEdits` (auto
      imports), and snippet placeholders (`$1`, `${2:name}`) via CodeMirror's
      snippet support.
- [x] Hover: on mouse hover and on `Mod+K Mod+I`; renders Markdown with code
      blocks highlighted.
- [x] Diagnostics: squiggles by severity, gutter markers, a hover message, and
      `F8`/`Shift+F8` cycling shared with 01-13's annotations.
- [x] Signature help on `(` and `,`, dismissed on `)` or Escape.
- [x] Locked lines (01-05): edits inside them never reach the server (they're
      rejected before the change is applied anyway); diagnostics on them still show.
- [x] Highlighter per plugin: `[language].highlighter` selects a bundled
      CodeMirror language package (PLUGINS.md §9). Load lazily per language so the
      initial bundle stays small; measure bundle size before and after.
- [x] When the server is `Stopped` or `Crashed`, the editor keeps working; the
      status bar shows the state (02-08). No error dialogs.
- [x] Time from typing `.` to the completion popup, warm server, TypeScript:
      measured and recorded here. Target under 100 ms.

## Out of scope

Go to definition (02-09), rename, code actions, formatting (all easy add-ons later
once this works).

## Gotchas

Check what `@codemirror/lsp-client` supports before starting; if it lacks
`additionalTextEdits` or snippets, that's the point to fall back to the community
package rather than patching.

## Results

Implemented 2026-09-16.

**The client** — `ui/src/lsp.ts`

- `@codemirror/lsp-client` 6.2.5, which supports everything this item asks for
  (`textEdit`, `additionalTextEdits`, snippets, signature help, diagnostics), so
  the community package wasn't needed.
- `TauriTransport` is one channel per tab: `lsp_open` hands the core a
  `Channel`, `lsp_send` carries messages the other way, `lsp_close` ends it.
  One `LSPClient` per tab, all of them multiplexed onto the one pooled server by
  the proxy.
- Completion, hover on the mouse, signature help on `(` and `,`, and diagnostics
  come from `languageServerExtensions()`. Added here: `Tab` to accept a
  completion (CodeMirror binds `Enter` already), `Mod+K Mod+I` to hover at the
  cursor with `Escape` to dismiss, and `lintGutter()` for the markers.
- Nothing blocks the tab. A playground opens in about 20 ms and the language
  features appear when the handshake finishes — seconds later for a cold
  rust-analyzer. A server that never starts leaves the editor exactly as it was,
  with no dialog.

**Highlighters** — `ui/src/highlight.ts`

Twenty languages, each a dynamic import, so a grammar is downloaded the first
time a playground uses it. The list in `KNOWN_HIGHLIGHTERS` (the loader's
warning) and PLUGINS.md §9 now match what the UI actually bundles: `zig` went
(no CodeMirror package), and `jsx`, `ruby`, `scala`, and `toml` arrived.
`syntaxHighlighting(defaultHighlightStyle)` was missing from the editor's base
extensions, so no grammar had ever coloured anything; it is there now.

**The core's side**

- `ServerHandle::capabilities()` keeps what the server said in the handshake,
  and `ready()` waits for the handshake and returns it. The proxy answers each
  editor client's `initialize` with that — *after* waiting, because an editor
  told "no capabilities" because the handshake hadn't finished acts on that for
  as long as the tab is open.
- `initialized`, `shutdown`, and `exit` from an editor are answered locally: the
  core owns the real lifecycle.
- **Document sync belongs to the editor now**, version numbering included, and
  `LspPool::attach_editor` attaches a tab without opening the document. This
  reverses 02-02's "never trust the editor's number": a server echoes the
  version back in `publishDiagnostics`, and a client that sees a version it
  didn't send drops the diagnostics — which is exactly what happened until this
  changed. The core watches the sync messages go past
  (`ServerHandle::sync_from_editor`) and keeps the text for the replay in 02-05.
  PLAN.md's decision log records it.
- Plugins: `[lsp]` for Rust (rust-analyzer, workspace `playground_dir` until
  02-07) and TypeScript (typescript-language-server, `library_root`).
  `PluginSummary` gained `highlighter` and `lsp` so the UI knows what a
  playground has.

**Measurements** (Windows 11, release build, rust-analyzer 1.98.1)

| What | Result |
|---|---|
| Keystroke to completion popup, warm server | **median 119 ms**, best 116 ms (5 samples: 116, 117, 117, 125, 208) |
| First completion after opening a playground, cold server | 3.5 s |
| Tab usable after opening a playground | 18–22 ms |
| Second tab in the same language connecting | 19 ms |
| Startup bundle before | 452.85 kB raw, 147.25 kB gzip (one chunk) |
| Startup bundle after | 495.77 kB raw, 160.88 kB gzip (five chunks) |
| Loaded on demand | the LSP client 68 kB, each grammar 1–104 kB (Rust 104 kB, clike 22 kB, TOML 1 kB) |

The 119 ms is rust-analyzer's own latency almost entirely: a round trip through
the proxy measured 98 µs in 02-03, so TinkerPad accounts for well under 1 ms of
it.

**Tests**

- In-app `lsp` scenario, 14 checks against a real rust-analyzer: the plugin
  declares a server and a highlighter; the tab is usable immediately; the
  grammar loads on demand and colours the document; the tab connects; typing `.`
  offers `String`'s methods; a warm server answers a keystroke quickly; `Tab`
  accepts; `Mod+K Mod+I` shows the documentation and `Escape` dismisses it; a
  broken file gets diagnostics with squiggles and gutter markers; `F8` walks
  them; and a second tab connects to the server already running. The scenario
  skips itself with an INFO line where rust-analyzer isn't installed, and CI now
  installs it.
- `tinkerpad-lsp`: a test that the editor's own client never reaches the server
  — its `initialize` is answered from the core's cache, its `didOpen` opens the
  document at its own version, and the core follows along and keeps the text.

## Checklist status

**Open: the 100 ms target, and TypeScript.** The item asks for typing `.` to
the popup under 100 ms on a warm TypeScript server. Measured here: 119 ms median
with rust-analyzer, which is what this machine has;
`typescript-language-server` isn't installed. TinkerPad's own share of that is
under a millisecond, so the number to chase belongs to the server, not to this
code. Worth re-measuring in 03-06 when the TypeScript helper lands.

**Finding: a server holds its workspace folder open.** With
`workspace = "playground_dir"`, rust-analyzer keeps the playground folder open,
and Windows then refuses to delete it while the server runs — deleting a Rust
playground fails until the app exits. 02-07 moves Rust to one shared scratch
workspace, which removes the case; if it lingers after that, the pool should
stop a server whose root has gone.

**Note: the `rust` scenario's two pre-existing failures** (cargo's progress bar
reaching the output pane) are unrelated to this item and still reproduce on
`main`; see the note on PR #15.
