# 04-13 · IntelliSense for C#

**Phase:** 4 · **Size:** L · **Crate/area:** `plugins/csharp/`, possibly `plugins/csharp/helper/` · **Depends on:** 02-04, 02-07, 03-05, 04-08a
**Design:** PLUGINS.md §7 (C#), PLAN.md decision log 2026-09-16, ARCHITECTURE.md (LSP keep-alive)

## What

C# playgrounds get what TypeScript, Rust and Python already have: completions
as you type, hover, diagnostics while editing (not only on run), and go to
definition. A new C# playground has them without the user installing anything
by hand beyond the .NET SDK.

## Why

Reported on 2026-10-25: "IntelliSense does not seem to be working when I open a
new C# playground." That's accurate, and it isn't a bug in something that
exists. **C# has never had a language server in TinkerPad**:

- `plugins/csharp/plugin.toml` has no `[lsp]` section, so no server starts
  and no chip appears in the status strip for a C# tab. The editor still
  highlights syntax (`highlighter = "csharp"`), which can make it look like
  more is going on than there is.
- On 2026-09-16 the decision log said "C# gets no language server until the
  Roslyn helper in 03-05", because the helper "can answer LSP requests from
  the same compilation".
- 03-05 was then scoped and built as a **runner** only. Its expectations never
  mention completions, hover or diagnostics, and `plugins/csharp/helper/` has
  no code for them. The deferral was never picked up. This item is where it
  is.
- PLUGINS.md §7 still describes a different plan: Roslyn's
  `Microsoft.CodeAnalysis.LanguageServer`, downloaded on first use, falling
  back to `csharp-ls`, falling back to a scaffolded `main.csproj`. None of it
  is implemented.

C# is the language LINQPad is best known for, so this is the biggest gap in
the bundled languages.

## Expectations

**First, choose the approach with evidence, and write it down here.** Spend
no more than a couple of evenings trying each of these against a real
`main.cs` playground (top-level statements, a `#:package` directive, and the
`Directory.Build.props` the scaffold writes):

1. **Roslyn's language server** (`Microsoft.CodeAnalysis.LanguageServer`, the
   one the VS Code C# extension uses). Does it handle a loose file or a .NET 10
   file-based app with no project file, and does it see `#:package`
   references? How is it acquired: which feed, which package per platform, how
   big, can `[lsp.acquire]` (04-08a) fetch it? It doesn't speak plain LSP
   for opening a workspace (it uses custom `solution/open`/`project/open`
   notifications), so what would the plugin need to send?
2. **`csharp-ls`** (`dotnet tool install -g csharp-ls`). What does it do with
   no project file? With a scaffolded `main.csproj`?
3. **A scaffolded `main.csproj`** next to `main.cs`, for any server. Does
   `dotnet run --file main.cs` still work with a project file in the same
   folder, or does it refuse? Does the warm helper (03-05) still run it?
4. **The helper answers LSP itself**, as the 2026-09-16 decision intended,
   using Roslyn's completion and quick-info services over the compilation it
   already builds. This means writing a small LSP server, which is the most
   work but has no acquisition problem.

- [ ] The findings for each of the four, with what was measured (start-up
      time, memory once warm, time to first completion, download size), and
      the choice. Add a row to PLAN.md's decision log replacing the
      2026-09-16 one, and update PLUGINS.md §7 to match what was built.
- [ ] If the choice makes this item bigger than L, split it here (one PR for
      the split, per AGENTS.md) before implementing.

**Then, whichever approach wins:**

- [ ] `plugins/csharp/plugin.toml` gains `[lsp]` (and `[lsp.acquire]` or
      `[lsp.register]` as needed). Nothing language-specific goes into Rust.
- [ ] Opening a new C# playground, with only the .NET 10 SDK installed, shows
      a C# chip in the status strip and reaches ready. If something has to be
      downloaded, it's offered the way 04-08b offers tools, not done silently,
      and it works offline afterwards.
- [ ] Typing `Console.` offers `WriteLine` and friends; typing after a
      `#:package` reference offers that package's types; `.Dump(` is offered
      on any value (the dump library compiled in by the scaffold is visible to
      the server).
- [ ] Hover (`Mod+K Mod+I`) shows the type and documentation of the symbol
      under the cursor.
- [ ] A syntax or type error is squiggled while typing, before any run, at the
      same line and column the run reports (01-13).
- [ ] Go to definition on a framework type opens decompiled or metadata
      source read-only (the peek tab from 02-09), or, if the server can't,
      the item says so.
- [ ] All three kinds work, including **Expression**. The locked prelude and
      postlude are in the file, so positions need no mapping, but check that
      completions inside the expression make sense.
- [ ] Warm-completion latency measured and recorded here, against the same
      target the other languages were held to (02-04: the `servers`/`lsp`
      self-tests' "a warm server answers a keystroke quickly", median under
      1 s).
- [ ] A self-test scenario like the Rust and TypeScript ones (`lsp`,
      `lsp-typescript`): connects, completion after `.`, hover, diagnostics,
      restart. It runs in CI on all three platforms, which already have .NET
      for the C# run tests.
- [ ] `plugins/csharp/README.md` says what's needed and what happens on first
      use.

## Out of scope

C# debugging (06-09). Refactorings, code actions, rename, formatting. They're
worth having later if the chosen server provides them, but not required here.
Making the other languages' IntelliSense better: if one of them has a real
problem, it gets its own item.

## Gotchas

- The playground must stay a **file-based app**. Whatever the server needs,
  `dotnet run --file main.cs` and the helper must keep working, and the user
  must not have to see or maintain a project file. If a `.csproj` turns out
  to be the price, write it to the plugin's scratch space or the LSP
  workspace (`[lsp].workspace_files`), not the playground folder, if at all
  possible.
- Roslyn takes a few hundred MB once warm, and the helper already holds its
  own Roslyn. Running both doubles that. Measure it, and set an idle policy
  (02-06) that fits.
- `Directory.Build.props` points at the dump library through
  `TINKERPAD_CSHARP_DUMP`. The language server has to get the same variable
  in its environment (`[lsp].env`), or `Dump` will be an unknown symbol and
  every playground will have a red squiggle on its first line.
- The first `#:package` restore can take seconds and needs the network. The
  server shouldn't sit at "starting" without saying why; the status strip
  should show it's restoring.
