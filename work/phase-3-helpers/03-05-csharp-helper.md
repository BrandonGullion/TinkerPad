# 03-05 · C# Roslyn helper

**Phase:** 3 · **Size:** L · **Crate/area:** `plugins/csharp/helper/` · **Depends on:** 03-03, 03-02, 01-10
**Design:** PLUGINS.md §7 (C#), §5; PLAN.md §3.4

## What

A .NET console app, built on first use via `[helper].build`, that keeps Roslyn
warm and executes playgrounds in-process: parse the file's `#:package` directives,
resolve packages, compile with `Microsoft.CodeAnalysis.CSharp.Scripting` or a
plain `CSharpCompilation`, load into a collectible `AssemblyLoadContext`, run,
unload. This is LINQPad's engine.

## Why

C# per-run is the slowest of the bundled languages (~1–2 s). This item is the
single biggest "extremely efficient" win in the plan.

## Expectations

- [ ] `helper/helper.csproj` targeting net10.0, referencing
      `Microsoft.CodeAnalysis.CSharp` (and `.Scripting` if the script API is
      used). `[helper].build` = `dotnet build -c Release -o bin`. First build time
      recorded here.
- [ ] Protocol: hello/ready, run/cancel/shutdown per 03-01. `stdout`/`stderr`
      chunks via redirected `Console.Out`/`Console.Error` writers that flush per
      write. The `Dump()` extension is provided by the dump library and works
      without changes because it writes to `Console.Out`.
- [ ] Compilation: top-level statements (`statements` kind) and full programs
      (`program` kind) both supported; `expression` kind is just statements.
      Compile errors → `failed` with `diagnostics` (file, line, col, message)
      mapped straight through (no offsets, locked lines are in the file).
- [ ] Symbols from day one: the syntax tree is parsed with `path:` set to the
      on-disk `main.cs` and its encoding; emit uses `DebugInformationFormat.
      PortablePdb`; the assembly is loaded via `LoadFromStream(dll, pdb)`.
      Verify by inspecting the PDB (e.g. with `dotnet-pdb` or a stack trace from
      a thrown exception showing `main.cs:line N`). Cost is a few ms per compile.
      (DEBUGGING.md D9.)
- [ ] References: the implicit .NET 10 reference set plus packages from
      `#:package` directives resolved with NuGet (`NuGet.Protocol`) into the
      user's global packages folder, cached per (name, version) across runs.
      First run with a new package downloads; subsequent runs don't touch the
      network.
- [ ] Isolation: each run in a new collectible `AssemblyLoadContext`; unloaded
      after `exited`. A run that keeps threads alive is reported (`stderr`
      warning) and the context leaks until helper restart.
- [ ] Cancel: `CancellationToken` passed to the script if using the scripting
      API; otherwise `Thread.Interrupt` plus a 2 s deadline then the core kills the
      helper (03-01's rule).
- [ ] Timings recorded here: first run after helper start; second run unchanged;
      run after a one-line edit. Target: under 100 ms for the third.
- [ ] Console encoding UTF-8; `Console.In` returns EOF (stdin belongs to the
      protocol).
- [ ] Falls back to `dotnet run` cleanly when the build fails (03-03), e.g. on a
      machine with the runtime but not the SDK.

## Out of scope

Debugging, `async Main` edge cases beyond `await` at top level, `#:sdk` directives
other than the default.

## Gotchas

`Console.SetOut` is process-global; the helper runs one playground at a time so
that's fine, but never let two runs overlap. The protocol already forbids it.
