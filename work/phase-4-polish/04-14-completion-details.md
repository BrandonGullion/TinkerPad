# 04-14 · A method's parameters in the completion list

**Phase:** 4 · **Size:** M · **Crate/area:** `ui/src/lsp.ts`, `plugins/csharp/helper/`, `crates/tinkerpad-lsp` · **Depends on:** 02-04, 04-13
**Design:** ARCHITECTURE.md (LSP keep-alive), PLUGINS.md §7 (C#)

## What

When a method is highlighted in the completion list, a panel beside the list
shows its signature (what parameters it takes, and how many overloads there
are) and the first part of its documentation, as VS Code does. After `(`, the
parameters stay visible while the arguments are typed (signature help).

## Why

Reported on 2026-11-03: "when your cursor is hovering over a method in
intellisense, it will need to show what params are needed for that method."
Today nothing shows, for two reasons:

- **The editor never asks.** TypeScript's and Python's servers send a bare
  list and give each item's signature and documentation only when the editor
  asks about it (`completionItem/resolve`). `@codemirror/lsp-client` 6.2.5
  never asks, so those details never arrive.
- **C#'s server never sends it.** `plugins/csharp/helper/Playgrounds.cs`
  fills only Roslyn's inline description, which is usually empty, and the
  server has no `textDocument/signatureHelp`.

## Expectations

- [x] Highlighting a completion asks the server about that one item, if the
      server resolves items, and shows the answer in the panel beside the
      list: the signature first, then the documentation. Items are asked
      about one at a time, as they are highlighted, never the whole list.
- [x] A server that sends detail and documentation up front (rust-analyzer)
      shows them in the same panel, with no extra request.
- [x] The core tells servers it can resolve `detail` and `documentation`
      lazily.
- [x] C#: the server resolves items with Roslyn's description, e.g.
      `void Console.WriteLine(string? value) (+ 17 overloads)` and the
      summary from the XML docs.
- [x] C#: the server answers `textDocument/signatureHelp` after `(` and `,`,
      with every overload, the active one, and the active parameter.
- [x] Self-tests: in C#, highlighting `WriteLine` after `Console.` shows its
      parameters; after `Console.WriteLine(` signature help lists overloads.
      In TypeScript, a highlighted method shows its signature.

## Results (2026-11-03, Windows)

- C#, `Console.` then `WriteLine` highlighted: `void Console.WriteLine() (+ 19 overloads)`,
  then "Writes the current line terminator to the standard output stream."
- C#, `Console.WriteLine("hi", |)`: `15/20 void Console.WriteLine(string format, object? arg0)`
  with `object? arg0` marked, and the parameter's own docs.
- C#, `new List<int>(`: `List<int>()`, `List<int>(int capacity)`, `List<int>(IEnumerable<int> collection)`.
- TypeScript, `map` highlighted on a `string[]`:
  `(method) Array<string>.map<U>(callbackfn: (value: string, index: number, array: string[]) => U, thisArg?: any): U[]`
  and its description.
- Warm completion is unchanged: C# median 125 ms, TypeScript median 140 ms, both
  as the editor sees them, typing debounce included.
- Python (Pyright) goes through the same resolve path, but wasn't run here.
