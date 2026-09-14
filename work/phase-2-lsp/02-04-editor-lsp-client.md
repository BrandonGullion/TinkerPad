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

- [ ] A `TauriLspTransport` implementing the client package's transport interface:
      one per tab, created on tab open, closed on tab close.
- [ ] Document sync: incremental `didChange` on every edit, debounced no more than
      ~50 ms so diagnostics feel live.
- [ ] Completion: triggered on typing and on trigger characters the server
      declares; shows kind icons, detail, and documentation in a side panel;
      accepts with Tab/Enter; supports `textEdit`, `additionalTextEdits` (auto
      imports), and snippet placeholders (`$1`, `${2:name}`) via CodeMirror's
      snippet support.
- [ ] Hover: on mouse hover and on `Mod+K Mod+I`; renders Markdown with code
      blocks highlighted.
- [ ] Diagnostics: squiggles by severity, gutter markers, a hover message, and
      `F8`/`Shift+F8` cycling shared with 01-13's annotations.
- [ ] Signature help on `(` and `,`, dismissed on `)` or Escape.
- [ ] Locked lines (01-05): edits inside them never reach the server (they're
      rejected before the change is applied anyway); diagnostics on them still show.
- [ ] Highlighter per plugin: `[language].highlighter` selects a bundled
      CodeMirror language package (PLUGINS.md §9). Load lazily per language so the
      initial bundle stays small; measure bundle size before and after.
- [ ] When the server is `Stopped` or `Crashed`, the editor keeps working; the
      status bar shows the state (02-08). No error dialogs.
- [ ] Time from typing `.` to the completion popup, warm server, TypeScript:
      measured and recorded here. Target under 100 ms.

## Out of scope

Go to definition (02-09), rename, code actions, formatting (all easy add-ons later
once this works).

## Gotchas

Check what `@codemirror/lsp-client` supports before starting; if it lacks
`additionalTextEdits` or snippets, that's the point to fall back to the community
package rather than patching.
