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

- [x] `helper/helper.csproj` targeting net10.0, referencing
      `Microsoft.CodeAnalysis.CSharp` (and `.Scripting` if the script API is
      used). `[helper].build` = `dotnet build -c Release -o bin`. First build time
      recorded here.
- [x] Protocol: hello/ready, run/cancel/shutdown per 03-01. `stdout`/`stderr`
      chunks via redirected `Console.Out`/`Console.Error` writers that flush per
      write. The `Dump()` extension is provided by the dump library and works
      without changes because it writes to `Console.Out`.
- [x] Compilation: top-level statements (`statements` kind) and full programs
      (`program` kind) both supported; `expression` kind is just statements.
      Compile errors → `failed` with `diagnostics` (file, line, col, message)
      mapped straight through (no offsets, locked lines are in the file).
- [x] Symbols from day one: the syntax tree is parsed with `path:` set to the
      on-disk `main.cs` and its encoding; emit uses `DebugInformationFormat.
      PortablePdb`; the assembly is loaded via `LoadFromStream(dll, pdb)`.
      Verify by inspecting the PDB (e.g. with `dotnet-pdb` or a stack trace from
      a thrown exception showing `main.cs:line N`). Cost is a few ms per compile.
      (DEBUGGING.md D9.)
- [x] References: the implicit .NET 10 reference set plus packages from
      `#:package` directives resolved with NuGet (`NuGet.Protocol`) into the
      user's global packages folder, cached per (name, version) across runs.
      First run with a new package downloads; subsequent runs don't touch the
      network.
- [x] Isolation: each run in a new collectible `AssemblyLoadContext`; unloaded
      after `exited`. A run that keeps threads alive is reported (`stderr`
      warning) and the context leaks until helper restart.
- [x] Cancel: `CancellationToken` passed to the script if using the scripting
      API; otherwise `Thread.Interrupt` plus a 2 s deadline then the core kills the
      helper (03-01's rule).
- [x] Timings recorded here: first run after helper start; second run unchanged;
      run after a one-line edit. Target: under 100 ms for the third.
- [x] Console encoding UTF-8; `Console.In` returns EOF (stdin belongs to the
      protocol).
- [x] Falls back to `dotnet run` cleanly when the build fails (03-03), e.g. on a
      machine with the runtime but not the SDK.

## Out of scope

Debugging, `async Main` edge cases beyond `await` at top level, `#:sdk` directives
other than the default.

## Gotchas

`Console.SetOut` is process-global; the helper runs one playground at a time so
that's fine, but never let two runs overlap. The protocol already forbids it.

## Results

Implemented 2026-09-19, against **.NET 10.0.201** on Windows 11.

**`plugins/csharp/helper/`** is a .NET console app that keeps Roslyn warm and
runs playgrounds in its own process. Seven small files: the protocol loop
(`Program.cs`), the wire (`Protocol.cs`), parse-compile-load-run-unload
(`Runner.cs`), the reference set (`References.cs`), `#:package` resolution
(`Packages.cs`), reading `main.cs` (`Sources.cs`), and Roslyn's diagnostics in
the protocol's shape (`Diagnostics.cs`).

**Measured** (release build, Mod+Enter to the output on screen, same scenario
before and after):

| | Per-run (`dotnet run --file`) | On the helper |
|---|---|---|
| First run of a session | 2249 ms | 4138 ms (builds *and* starts the helper) |
| Rerun, unchanged | 1047 ms | **64 ms** |
| Run after a one-line edit | 1174 ms | **79 ms** |
| Expression kind | 2015 ms | **74 ms** |
| Program kind | 2095 ms | **86 ms** |
| With `#:package Humanizer@2.14.1` | — | 200 ms, then **81 ms** cached |

The helper's own figure — compile, load, run, report — is about 50 ms of each
of those. Building the helper takes **7.4 s** cold (restore included) and 1.2 s
incrementally; starting it and warming Roslyn takes **1.2 s**, inside the ten
seconds the protocol allows before `ready`. It holds **189 MB** resident, which
is why its `idle_shutdown` is two hours rather than never.

**Symbols from day one** (D9). The playground is parsed with `path:` set to the
real `main.cs` and its encoding, emitted with `DebugInformationFormat.
PortablePdb`, and loaded with `LoadFromStream(dll, pdb)`. Verified by throwing:

```
Unhandled exception. System.InvalidOperationException: nope
   at Program.<<Main>$>g__Boom|0_0() in C:\...\throws.cs:line 7
   at Program.<Main>$(String[] args) in C:\...\throws.cs:line 10
```

which is what `dotnet run` prints for the same file, character for character —
the helper's own reflection frames are trimmed off the end, and the exit code
it reports is the one .NET uses for an unhandled exception (`0xE0434352` on
Windows, 134 elsewhere) rather than something of its own invention.

**Compile errors are diagnostics, not text.** Roslyn's errors come back as
`failed` with file, line, column and message, so the editor marks the spot
without a regex having to find it in the output. Positions go straight through:
the file is compiled as it is written, locked lines and all.

**Isolation** is a collectible `AssemblyLoadContext` per run, unloaded after the
run reports. A playground that leaves a thread or a timer going keeps its
context alive; the helper says so on its log and the memory stays until it is
restarted from the status strip.

**Packages.** `#:package Name@Version` is resolved with NuGet into the same
global packages folder `dotnet restore` uses, with dependencies, and kept in
memory for the session. The load context is told where those assemblies are, so
`using Humanizer;` works at runtime and not only at compile time.

**Four bugs worth recording**, all found by running the thing:

1. **`InvariantGlobalization` is contagious.** The helper had it on, as a
   start-up saving. A playground runs *in the helper's process*, so Humanizer
   threw `CultureNotFoundException` on `en-US` — something it would never do
   under `dotnet run`. The setting is now explicitly off, with a comment saying
   why.
2. **A collectible context can't be collected while a local variable holds
   it.** Every run reported "still holding memory" until the unload was split
   into a `[MethodImpl(NoInlining)]` method that returns a `WeakReference`, with
   the collection happening after it returns.
3. **The framework folder has native DLLs in it.** `MetadataReference.
   CreateFromFile` takes them happily and the compiler fails at emit with
   CS0009. They are now filtered by asking for the assembly name first.
4. **A stale `obj/` in the plugin folder broke the build.** MSBuild's default
   globs only exclude the *current* intermediate folder, and TinkerPad builds
   with that folder elsewhere, so generated assembly attributes collided
   (CS0579). The project now excludes `bin/**` and `obj/**` explicitly.

**One fix outside the plugin.** A diagnostic pointing one past the end of a
line — which is exactly where `; expected` points — produced no squiggle at
all, because the mark's range was empty and was skipped. It now underlines the
character the missing token would have followed, which is what an editor does.
That affected the per-run path too; nobody had noticed because the old C# test
only looked at the output text.

**Tests.** The C# self-test scenario, which already covered templates, kinds,
UTF-8, dumps and compile errors, now runs entirely through the helper and
passes unchanged apart from the compile-error check, which asserts the
structured diagnostic and the squiggle instead of a line of matched text. Four
checks were added: a `#:package` directive resolved and used, the second run not
fetching it again, a runaway loop cancelled, and the helper still warm
afterwards.

## Checklist status

**Deviation: `CSharpCompilation`, not the Scripting API.** The item allows
either. A plain compilation compiles the file as written, so line numbers and
diagnostics map one-to-one with the editor and no offsets are needed anywhere —
which is also what makes the PDB point at `main.cs`. The scripting API wraps the
code, which would have cost that.

**Deviation: the build goes to the app's data folder, not `-o bin`.** An
installed plugin may sit somewhere read-only (`Program Files`, an app bundle),
so `[helper.build]` writes the binaries and the intermediate folder under
`{scratch}` instead. Everything else about the item's build step is as written.

**Note: the dump library is compiled once, not per run.** The per-run path
compiles `TinkerPad.Dump.cs` into the playground; doing that here cost about 70
ms of every run. It is now compiled once into an assembly that every run
references and shares. The visible difference is that `Dump`'s own statics live
as long as the helper rather than as long as a run, which is invisible unless a
playground reaches into them.

**Note: cancel is `Thread.Interrupt`, the item's second option.** There is no
`CancellationToken` to hand a plain program. A blocking wait ends at once
(measured: 21 ms from `cancel` to `exited`); a tight loop doesn't notice and the
core kills the helper after its two-second grace, exactly as 03-01 says.

**Note: packages don't do full conflict resolution.** Dependencies are walked
and the highest version of a repeated package wins, rather than running NuGet's
resolver over the whole graph. A playground is one file with a handful of
packages; the README says so.

**Note: the fallback to `dotnet run` was exercised for real.** Before the
`obj/` fix, the helper build failed on every run and C# kept working — every
scenario passed on the per-run path, with the toast saying why. That is 03-03's
machinery doing its job, and it is why the failure took a while to notice.
