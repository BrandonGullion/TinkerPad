# 02-09 · Go to definition

**Phase:** 2 · **Size:** S · **Crate/area:** `ui/`, `tinkerpad-lsp` · **Depends on:** 02-03, 02-04
**Design:** ARCHITECTURE.md §4.2

## What

`F12` / Mod+click asks the server for the definition. A location inside the
current playground moves the cursor. A location in another file (a dependency's
source, a generated file) opens in a read-only peek tab that is not a playground.

## Why

Reading library source without leaving the scratchpad is a large part of what makes
a playground useful for exploring APIs.

## Expectations

- [ ] `textDocument/definition` and `textDocument/typeDefinition`; results may be
      `Location`, `Location[]`, or `LocationLink[]`.
- [ ] In-playground target: move cursor, scroll, push a jump-back entry (`Alt+Left`
      returns).
- [ ] External target: the core reads the file (read-only), and the UI opens a
      "peek" tab with a lock icon, the file's highlighter by extension, and no LSP
      attachment. Closing it is free. At most 10 peek tabs; oldest closes.
- [ ] Opening a peek tab is a general `open_readonly(path, line, col)` command,
      not something only go-to-definition can trigger. (DEBUGGING.md D15: the
      debugger's call stack opens library frames the same way.)
- [ ] Multiple results → a small list to choose from, Enter to open.
- [ ] Works in all three bundled languages: jump into `node_modules` typings, a
      `.nupkg`-extracted source or decompiled stub if the C# server offers one, and
      a crates.io dependency in `~/.cargo/registry`.

## Out of scope

References, rename, code actions, formatting (worth adding after Phase 2 as small
items of their own).

## Gotchas

Very large files (decompiled assemblies, bundled JS) should be capped at a few MB
for the peek tab, with a "file too large" notice instead of a frozen editor.
