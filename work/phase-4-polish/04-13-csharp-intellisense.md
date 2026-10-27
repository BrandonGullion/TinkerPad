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

- [x] The findings for each of the four, with what was measured (start-up
      time, memory once warm, time to first completion, download size), and
      the choice. Add a row to PLAN.md's decision log replacing the
      2026-09-16 one, and update PLUGINS.md §7 to match what was built.
- [x] If the choice makes this item bigger than L, split it here (one PR for
      the split, per AGENTS.md) before implementing.

**Then, whichever approach wins:**

- [x] `plugins/csharp/plugin.toml` gains `[lsp]` (and `[lsp.acquire]` or
      `[lsp.register]` as needed). Nothing language-specific goes into Rust.
- [x] Opening a new C# playground, with only the .NET 10 SDK installed, shows
      a C# chip in the status strip and reaches ready. If something has to be
      downloaded, it's offered the way 04-08b offers tools, not done silently,
      and it works offline afterwards.
- [x] Typing `Console.` offers `WriteLine` and friends; typing after a
      `#:package` reference offers that package's types; `.Dump(` is offered
      on any value (the dump library compiled in by the scaffold is visible to
      the server).
- [x] Hover (`Mod+K Mod+I`) shows the type and documentation of the symbol
      under the cursor.
- [x] A syntax or type error is squiggled while typing, before any run, at the
      same line and column the run reports (01-13).
- [x] Go to definition on a framework type opens decompiled or metadata
      source read-only (the peek tab from 02-09), or, if the server can't,
      the item says so.
- [x] All three kinds work, including **Expression**. The locked prelude and
      postlude are in the file, so positions need no mapping, but check that
      completions inside the expression make sense.
- [x] Warm-completion latency measured and recorded here, against the same
      target the other languages were held to (02-04: the `servers`/`lsp`
      self-tests' "a warm server answers a keystroke quickly", median under
      1 s).
- [x] A self-test scenario like the Rust and TypeScript ones (`lsp`,
      `lsp-typescript`): connects, completion after `.`, hover, diagnostics,
      restart. It runs in CI on all three platforms, which already have .NET
      for the C# run tests.
- [x] `plugins/csharp/README.md` says what's needed and what happens on first
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

## Findings

Measured 2026-10-26 on Windows 11 with the .NET 10.0.201 SDK. A small Node
LSP client drove each server against a real playground: `#:package
Humanizer@2.14.1`, the scaffold's `Directory.Build.props` with
`TINKERPAD_CSHARP_DUMP` set, top-level statements, and `int wrong = "text";`
on line 9. `dotnet run --file` reports that line as `main.cs(9,13): error
CS0029`.

**1. Roslyn's language server.** It now ships as `roslyn-language-server`, a
.NET 10 RID-specific `dotnet tool` on the `vs-impl` Azure DevOps feed (not on
nuget.org). `dotnet tool install --add-source <feed> --tool-path …` takes 8 s:
a 21 KB pointer package plus a 67 MB `win-x64` package, 278 MB on disk.
- It understands file-based apps without being told. It classifies `main.cs`,
  loads it as a project in about 0.8 s, and sees `#:package` (Humanizer's
  extension methods are offered).
- Hover: 0.4 s, with documentation. Go to definition on `Console`: 0.3 s, to
  decompiled source under `%TEMP%\MetadataAsSource`.
- Diagnostics are **pull only** (`textDocument/diagnostic`). Pulled, they are
  right: `9:13 CS0029`. TinkerPad's editor client only handles pushed
  diagnostics, so this would need client work too.
- 384 MB once warm (the launcher plus `Microsoft.CodeAnalysis.LanguageServer`).
- **Blocking: completion ignores global usings.** `Console.` and `Math.` come
  back empty, and `numbers.` has no LINQ and no `Dump`. Adding `using System;`
  to the file fixes `Console.`, and `System.` works, so the semantic model
  used for completion is missing the implicit usings. Hover in the same file
  resolves `Console` fine. The same happened on 5.6, 5.9 and 5.13, with the
  usings in a real compiled-in file, with an ordinary `main.csproj` sent
  through `project/open`, and after a build had written
  `main.GlobalUsings.g.cs`. The cause wasn't found inside the time box. A
  playground with no `#:` line is also served as a "miscellaneous file", not
  a file-based app, and the C# Statements template has none.

**2. `csharp-ls`** (0.28.0, nuget.org, 99 MB installed). With no project file
it exits: "no or .csproj/.fsproj or sln files found". With a scaffolded
`main.csproj` it is good: first completion 2.6 s after start, warm median
6 ms, hover with documentation, diagnostics at 11:13 (the test file differed),
236 MB. It doesn't read `#:package`, and `Dump` wasn't visible: the
`Directory.Build.props` condition on `TINKERPAD_CSHARP_DUMP` didn't see the
variable.

**3. A scaffolded `main.csproj`.** `dotnet run --file main.cs` still works with
one beside it, and the helper never looks at it. But it has to mirror every
`#:package` as a `PackageReference`, kept in step as the user edits. And
csharp-ls looks for it in the workspace root, which here is the playground
folder: exactly the visible project file the Gotchas say to avoid.

**4. The helper answers LSP.** A prototype (`Microsoft.CodeAnalysis.CSharp.Features`
4.14.0, the Roslyn version the helper already uses) put the playground, the
implicit usings and the Dump library into an `AdhocWorkspace` and asked
Roslyn's own services:
- First `Console.` completion 582 ms in (about 1 s from process start).
- **Warm completion after an edit: median 10 ms.**
- `WriteLine`, LINQ, `Dump` and Humanizer's `Humanize` all offered.
- Quick info 36 ms. Diagnostics at the run's own position (`CS0029`).
- 258 MB working set. Nothing to download: the packages come from nuget.org
  when the helper is built, like Roslyn already does.
- The cost is writing a small LSP server. And Roslyn's metadata-as-source is
  internal, so a framework definition needs its own answer.

**Choice: 4, the helper answers LSP**, as the 2026-09-16 decision intended. It
is the only option that gets the headline cases right (`Console.`, `.Dump(`,
`#:package`), works offline, needs no visible project file, and compiles a
playground exactly the way a run on the helper does, so a squiggle and a run
can't disagree. It runs as a second process from the same build of the
helper (`--lsp`), so a language-server crash can't take a run down with it.
The two Roslyns cost about 260 MB each; the server's idle policy is set to
match the helper's.

Size: still L. A split isn't needed.

## Results

Implemented 2026-10-26, as option 4 from the findings above.

**Pieces**
- `plugins/csharp/helper/`. The helper started with `--lsp` is a language
  server (`LanguageServer.cs`): stdio JSON-RPC, one work queue, full document
  sync, completion, hover, push diagnostics 250 ms after the last edit, and
  definition.
  - `Playgrounds.cs` keeps each open playground as a project in one
    `AdhocWorkspace`, compiled with `Runner`'s own implicit usings, parse
    options and compile options, plus the dump library as source.
  - `#:package` lines are blanked the way a run blanks them, and their
    packages are fetched off the queue through the same `References` and
    `Packages` code a run uses.
  - `MetadataSource.cs` writes a read-only page of signatures for F12 into an
    assembly.
  - `References` can attach XML docs: from beside a package's DLL, or from the
    SDK's reference pack for the framework.
  - `Microsoft.CodeAnalysis.CSharp.Features` and `.Workspaces` 4.14.0, the
    Roslyn the helper already uses, are new package references.
- `[lsp].from_helper` (manifest, validation, schema, PLUGINS.md). Before the
  server starts, the app waits for the launch's toolchain check, then runs the
  helper's build (`HelperHandle::ensure_built`, with a build lock shared with
  the run path). It does the same for a pre-warmed server.
- `plugins/csharp/plugin.toml`: `[lsp]` runs `dotnet {scratch}/helper/…dll --lsp`,
  with `workspace = "scratch_workspace"` (one server for every C# playground)
  and the helper's 2 h idle shutdown.
- `ui/src/lsp.ts`, for every language: when a tab has a server, the server's
  completions replace the language's own word list instead of joining it.
  C#'s highlighter offered every keyword and type name, so `abstract` and
  `Action` topped the list after `Console.`. The Rust, TypeScript and keys
  scenarios still pass.

**Self-test** `lsp-csharp`, 15 checks, in CI's Language servers steps on
Linux and macOS, last in the list. `keys` presses Mod+N and expects the
`node-check` test server, and Mod+N uses the last language used. Local run
on Windows:

```
PASS a C# tab connects to its language server
PASS Console. offers WriteLine and friends, from the implicit usings, and only those
INFO first C# completion 2.0 s after the tab opened
PASS the status strip has a C# server, and it is ready
PASS a warm C# server answers a keystroke quickly
INFO warm C# completion: median 125 ms, best 117 ms
PASS .Dump( is offered on a value, and LINQ with it
PASS a #:package reference's methods are offered
PASS hover shows the type and its documentation
PASS a type error is squiggled before any run, at the line and column a run reports
PASS F12 on Console opens its signatures read-only
PASS in an Expression playground, completions inside the expression make sense
PASS in a Program playground, completions inside Main know its locals
PASS the C# server can be restarted
PASS completions come back after the restart
```

**Measurements** (Windows 11, .NET 10.0.201):
- **Warm completion:** median 125 ms in the editor (best 117 ms), against the
  02-04 target of under 1 s. Most of that is the editor's own typing
  debounce: at the server it is about 4 ms (driven directly, median of 9).
  rust-analyzer and TypeScript measure 125 ms and 116 ms the same way.
- **First completion:** 1.3 to 2.0 s after the tab opens, with the helper
  already built. Building it the first time is a `dotnet build` of `helper/`.
- **Memory:** about 240 to 260 MB once warm, a second Roslyn beside the
  helper's. Both stop after two idle hours.
- `tinkerpad plugin check plugins/csharp` passes all 13 checks, including
  "server: started, and found nothing wrong with check/dump".

**Checklist notes**
- **Nothing to offer or download.** The only fetches are the helper's own
  build (Roslyn from nuget.org, as before this item) and `#:package` restores,
  which a run does too. Once a package is in the global packages folder, the
  server works offline.
- **Restoring isn't on the chip.** "Restoring Humanizer 2.14.1…" goes to the
  server's log (`window/logMessage`), and squiggles wait for it, but the chip
  itself says "ready" meanwhile. Showing it there would need a server state
  the status strip doesn't have yet.
- **Definition into an assembly** opens generated signatures with their
  summaries, not decompiled source. Roslyn's metadata-as-source isn't public
  API.
- **CI on Windows** doesn't run the launch self-tests (no Windows launch step
  exists). On Windows CI, the plugin conformance step starts this server and
  checks its diagnostics. The full scenario was run locally on Windows.
