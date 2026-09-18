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

- [x] `textDocument/definition` and `textDocument/typeDefinition`; results may be
      `Location`, `Location[]`, or `LocationLink[]`.
- [x] In-playground target: move cursor, scroll, push a jump-back entry (`Alt+Left`
      returns).
- [x] External target: the core reads the file (read-only), and the UI opens a
      "peek" tab with a lock icon, the file's highlighter by extension, and no LSP
      attachment. Closing it is free. At most 10 peek tabs; oldest closes.
- [x] Opening a peek tab is a general `open_readonly(path, line, col)` command,
      not something only go-to-definition can trigger. (DEBUGGING.md D15: the
      debugger's call stack opens library frames the same way.)
- [x] Multiple results → a small list to choose from, Enter to open.
- [ ] Works in all three bundled languages: jump into `node_modules` typings, a
      `.nupkg`-extracted source or decompiled stub if the C# server offers one, and
      a crates.io dependency in `~/.cargo/registry`.

## Out of scope

References, rename, code actions, formatting (worth adding after Phase 2 as small
items of their own).

## Gotchas

Very large files (decompiled assemblies, bundled JS) should be capped at a few MB
for the peek tab, with a "file too large" notice instead of a frozen editor.

## Results

Implemented 2026-09-17.

**`F12`** asks the server where the thing under the cursor is defined;
**`Mod+F12`** asks where its *type* is; **`Mod`+click** does the same as `F12`
with the mouse. A target in the playground you are in moves the cursor. One
anywhere else opens read-only. **`Alt+Left`** goes back, however many jumps deep,
and works from a read-only tab too — which is the point, since that is where the
jump left you.

**Peek tabs.** A tab that is a file rather than a playground: a lock instead of
the language badge, its `folder/file.rs` as the name, the path in the status bar,
read-only in the editor, and the highlighter chosen by extension. Nothing saves
it, nothing runs in it, no language server attaches to it, and it is not written
into the session — a peek is where you looked, not what you were working on. Ten
of them are kept; the eleventh closes the oldest.

**`open_readonly(path)`** is a command of its own rather than something only
go-to-definition can reach, because the debugger's call stack opens library
frames exactly this way (`docs/DEBUGGING.md` D15). It caps at 4 MB: a decompiled
assembly or a bundled `.js` becomes a note saying how big it is, not a frozen
editor.

**Several answers** become a small list — the file and line, with the folder
underneath — and Enter opens the one you pick. A target in a playground is named
by that playground rather than by its id.

**Verified in all three languages** as far as each has a server:

| Language | `F12` lands in |
|---|---|
| Rust | `~/.rustup/toolchains/…/library/alloc/src/string.rs` for `String::new` |
| TypeScript | `node_modules/typescript/lib/lib.es5.d.ts` for `[].map` |
| C# | no language server yet (02-07); nothing to ask |

**Tests** — a `definition` scenario with 10 checks against the test plugin's
server, which answers by a rule the source asks for (a line saying "away" points
at another file, "many" gives two answers): a jump inside the file, `Alt+Left`
back, a jump into another file that opens locked and read-only with nothing to
run, two answers offered as a list, the ten-tab cap closing the oldest, and a
file too large to open saying so. Plus the two real-language checks above, in the
`lsp` and `lsp-typescript` scenarios. Four unit tests cover the command: reading
a file with its name and extension, the size cap, bytes that aren't text, and the
errors for a missing file and a folder.

## Checklist status

**Deviation: `Mod+F12` for type definitions.** The item lists
`textDocument/typeDefinition` without saying how it is reached; `F12` goes to the
definition and `Mod+F12` to the type, which is what VS Code does.

**Note: C# is not covered.** It has no language server until 03-05, so there is
nothing to ask for a definition. The item's "decompiled stub if the C# server
offers one" waits with it.
