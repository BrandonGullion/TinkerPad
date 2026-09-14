# 01-10 · C# plugin

**Phase:** 1 · **Size:** M · **Crate/area:** `plugins/csharp/` · **Depends on:** 01-09 (pattern), 01-05, 01-06, 01-07
**Design:** PLUGINS.md §7 (C#), §6

## What

The C# plugin as a pure folder using .NET 10 file-based apps: `dotnet run main.cs`.
Kinds `statements` (default), `program`, `expression` (locked lines). A `dump/`
library providing `.Dump()` as an extension method.

## Why

LINQPad's home language. It also exercises locked lines for the first time and
tests how the app copes with a 1–2 s per-run compile, which motivates the Phase 3
helper.

## Expectations

- [ ] `plugin.toml`: probe `dotnet --version` with `min_version = "10.0"`; `[run]`
      = `dotnet run {main}` with `DOTNET_NOLOGO=1`, `DOTNET_CLI_UI_LANGUAGE=en`,
      `DOTNET_CLI_TELEMETRY_OPTOUT=1`; `[output].error_regex` matching
      `main.cs(3,9): error CS1002: ; expected`; `[deps]` = inline directive
      `#:package {name}@{version}`.
- [ ] Templates: `statements.cs` starts with `#:property` lines if needed and a
      `using static TinkerPad.Dump;`-style import so `.Dump()` works on line one;
      `program.cs` with `static void Main`; `expression.cs` with locked
      `TinkerPad.Dump.Show(` … `);`.
- [ ] `dump/`: a single `.cs` file compiled into the playground. Mechanism: the
      template references it via `#:project` or a `#:package` pointing at a local
      folder-based NuGet source under `{plugin_dir}/dump/`; pick whichever `dotnet
      run` supports cleanly and document it in the plugin README. It provides
      `T Dump<T>(this T value, string? title = null)`, serialises with
      `System.Text.Json` into the schema, handles `IEnumerable<T>` of records as
      `table`, cycles (`ReferenceHandler.IgnoreCycles`), depth 5, row cap 1000.
- [ ] Console output encoding is UTF-8 (`Console.OutputEncoding` set in the dump
      library's static constructor or via `DOTNET_` env) so the OSC bytes survive.
- [ ] First run of a new playground and second run of an unchanged one timed and
      recorded here. Expect ~1.5–3 s and ~0.5–1 s. This number is the motivation
      for 03-05.
- [ ] Compile errors show as annotations via 01-13.
- [ ] README in the plugin folder covering the SDK requirement and the `#:package`
      syntax.

## Out of scope

The LSP (02-07, with the Roslyn acquisition problem), the Roslyn helper (03-05).

## Gotchas

`dotnet run main.cs` writes its build artefacts to a temp location keyed by file
path; renaming a playground folder means a full rebuild. Acceptable.
