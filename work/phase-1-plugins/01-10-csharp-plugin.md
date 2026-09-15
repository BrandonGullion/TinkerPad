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

- [x] `plugin.toml`: probe `dotnet --version` with `min_version = "10.0"`; `[run]`
      = `dotnet run {main}` with `DOTNET_NOLOGO=1`, `DOTNET_CLI_UI_LANGUAGE=en`,
      `DOTNET_CLI_TELEMETRY_OPTOUT=1`; `[output].error_regex` matching
      `main.cs(3,9): error CS1002: ; expected`; `[deps]` = inline directive
      `#:package {name}@{version}`.
- [x] Templates: `statements.cs` starts with `#:property` lines if needed and a
      `using static TinkerPad.Dump;`-style import so `.Dump()` works on line one;
      `program.cs` with `static void Main`; `expression.cs` with locked
      `TinkerPad.Dump.Show(` … `);`.
- [x] `dump/`: a single `.cs` file compiled into the playground. Mechanism: the
      template references it via `#:project` or a `#:package` pointing at a local
      folder-based NuGet source under `{plugin_dir}/dump/`; pick whichever `dotnet
      run` supports cleanly and document it in the plugin README. It provides
      `T Dump<T>(this T value, string? title = null)`, serialises with
      `System.Text.Json` into the schema, handles `IEnumerable<T>` of records as
      `table`, cycles (`ReferenceHandler.IgnoreCycles`), depth 5, row cap 1000.
- [x] Console output encoding is UTF-8 (`Console.OutputEncoding` set in the dump
      library's static constructor or via `DOTNET_` env) so the OSC bytes survive.
- [x] First run of a new playground and second run of an unchanged one timed and
      recorded here. Expect ~1.5–3 s and ~0.5–1 s. This number is the motivation
      for 03-05.
- [ ] Compile errors show as annotations via 01-13.
- [x] README in the plugin folder covering the SDK requirement and the `#:package`
      syntax.

## Out of scope

The LSP (02-07, with the Roslyn acquisition problem), the Roslyn helper (03-05).

## Gotchas

`dotnet run main.cs` writes its build artefacts to a temp location keyed by file
path; renaming a playground folder means a full rebuild. Acceptable.

## Results

Implemented 2026-09-15.

**Pieces** (`plugins/csharp/`)
- `plugin.toml`:
  - id `csharp`, highlighter `csharp`, and a probe for `dotnet --version` with a
    minimum of 10.0;
  - kinds `statements` (default), `program`, and `expression` (locked
    `TinkerPad.Dump.Show(` … `);`);
  - `[run]` = `dotnet run --file {main}` with `DOTNET_NOLOGO`,
    `DOTNET_CLI_UI_LANGUAGE=en`, `DOTNET_CLI_TELEMETRY_OPTOUT`,
    `DOTNET_SKIP_FIRST_TIME_EXPERIENCE`, and `TINKERPAD_CSHARP_DUMP`;
  - `[deps]` = inline directive `#:package {name}@{version}`;
  - `[output].error_regex`, which captures file, line, column, severity
    (`error` or `warning`), and message.
- `templates/`: `statements.cs`, `program.cs` (`class Program` with
  `static void Main`), `expression.cs`, and `Directory.Build.props`.
- `dump/TinkerPad.Dump.cs`: one file with `Dump.Show(value, title)` and the
  `.Dump(title)` extension method, both returning their argument.
  - A sequence of objects with members (records, classes, anonymous types)
    becomes a table, with columns from the union of members and C# type names
    (`int`, `string[]`, `anonymous[]`). Nested sequences in cells become nested
    tables.
  - Other objects, collections, and dictionaries become a tree. Dictionaries
    with non-scalar keys become `{key, value}` pairs.
  - Scalars become values with their C# type. A null shows its declared type,
    such as `string`.
  - Limits: five levels of nesting and 1,000 rows (with `truncated`). Only 1,001
    items are ever enumerated, so an endless sequence works. A reference back to
    an enclosing object becomes `"[circular]"`, and a getter that throws becomes
    `[error: …]`.
  - Output is built with System.Text.Json's `JsonNode`.
  - A `[ModuleInitializer]` sets `Console.OutputEncoding` to UTF-8 without a BOM
    before any user code runs.
- `README.md`: the SDK requirement, kinds, `Dump` rules, `#:package` and other
  directives, how the library gets in, and speed.

**How the dump library gets in.** Neither `#:project` nor a local NuGet source
works without writing the plugin's install path into the user's `main.cs` or a
config file. Instead, each playground gets a `Directory.Build.props`:
- It compiles `$(TINKERPAD_CSHARP_DUMP)` in and adds a global `using TinkerPad`.
  `[run].env` points that variable at `{plugin_dir}/dump/TinkerPad.Dump.cs`.
- It sets `PublishAot=false`. File-based apps default to AOT settings, which
  printed IL2026 and IL3050 warnings on every build, and
  `JsonSerializer.Serialize` threw "Reflection-based serialization has been
  disabled".
- It sets `Nullable=annotations`, so nullable warnings don't print on every run.
- Changing only the dump library doesn't trigger a rebuild, but touching
  `main.cs` does. The app rewrites `main.cs` before every run, so an updated
  dump library is always picked up.

This is recorded in PLAN.md's decision log and PLUGINS.md §7.

**Other changes**
- Runner test helpers moved to `tinkerpad-runner/tests/common/`, shared by the
  TypeScript and C# plugin tests.
- CI installs .NET 10 in the Rust and launch jobs. The launch timeout went from
  120 s to 300 s. The plugins and toolchains scenarios now run before the slow
  language scenarios, because the startup toast they check had disappeared by
  the time the C# scenario finished.

**Tests**
- `tinkerpad-plugin/tests/bundled.rs`: C# loads cleanly on all three OSes. Every
  kind scaffolds `Directory.Build.props`, the expression kind's locked regions
  are exact, and `[output]`, `[deps]`, and the dump variable are present.
- `tinkerpad-runner/tests/csharp_plugin.rs` has six tests, each running
  `dotnet run --file` for real (they fail in CI if .NET is missing):
  - each of the three templates;
  - console output, stderr, UTF-8 text including an emoji, and dumps interleave
    in order, and a 1,005-row table keeps 1,000 rows;
  - a missing semicolon is matched by `[output].error_regex` as `main.cs`,
    line 3, column 39, `error`, `CS1002: ; expected`;
  - the output of `tests/plugins/csharp/shapes.cs` matches the captured fixture.
- `tests/dump_cases/14-csharp-shapes.txt`: the captured output of every shape
  (tables, nested table, tree, 14 scalar types, dictionaries, set, cycle, depth
  limit, a throwing getter, and chaining). The dump parser's split-point tests
  run it too.
- New in-app `csharp` scenario, with 10 checks covering: a bundled plugin, a
  ready SDK, the template, its table, an unchanged rerun, an edit with UTF-8
  text, a compile error in the output with a failed run, the expression kind's
  locked lines and dump, and the program kind. All 112 checks in 10 scenarios
  pass on Windows. CI runs it on macOS and Linux.
- A screenshot showed text, the `people` table, then text, in order.

**Run times** (Windows 11, .NET SDK 10.0.201, release build, Mod+Enter to the
dump visible)

| Run | Compiler server already running | Compiler server started by TinkerPad |
|---|---|---|
| First run of a new playground | 1,122 ms | 2,713 ms |
| Second run, unchanged | 310 ms | 1,931 ms |
| Run after an edit | 296 ms | 1,938 ms |
| New expression or program playground | 1,024 to 1,039 ms | 2,668 to 2,691 ms |

"Already running" means the compiler server had been started outside TinkerPad,
here by a terminal `dotnet run`.

**Finding: the compiler server dies with every run on Windows.**
- Each run gets a Windows job object with `KILL_ON_JOB_CLOSE` (00-04). When the
  run ends, the job closes and takes the compiler server (`VBCSCompiler`) with
  it, because the run started it.
- Watching process IDs showed seven runs starting seven servers, none left
  afterwards. So without an outside server, every run pays the cold compiler
  start, about 1.6 s.
- Terminal runs confirmed the cause. Rebuilds took 306 ms with a warm server and
  1,919 ms with the server shut down before each run.
  `UseSharedCompilation=false` doesn't help: `dotnet run --file` still starts a
  server, and it measured 1,910 ms.
- macOS and Linux weren't measured.
- This isn't changed here. Letting build servers outlive a run means changing
  how 00-04 supervises processes, for example keeping a run's job open until the
  app exits. That is a design decision for its own work item, or for 03-05,
  which removes the compile anyway.

## Checklist status

**Ticked with a note**
- *`[run]` = `dotnet run {main}`.* It's `dotnet run --file {main}`, so a stray
  project file in the folder can't change what runs.
- *`statements.cs` imports with `using static TinkerPad.Dump;`.* It's
  `using TinkerPad;`, because `.Dump()` is an extension method in that
  namespace. The props file adds the same `using` globally, so any kind works
  even without it. No `#:property` lines are needed because the props file sets
  those properties.
- *`#:project` or a local NuGet source.* Neither is used. See "How the dump
  library gets in" above.
- *Cycles with `ReferenceHandler.IgnoreCycles`.* The library walks the object
  graph itself, so it can choose tables, cap rows, and limit depth. Like the
  TypeScript plugin, it marks a cycle as `"[circular]"` rather than writing
  `null`.
- *Expected ~1.5–3 s and ~0.5–1 s.* Measured 2.7 s and 1.9 s when the compiler
  server starts cold, which is the usual case inside TinkerPad on Windows, and
  1.1 s and 0.3 s with a warm server. See the finding above.

**Open**
- *Compile errors show as annotations via 01-13.* `[output].error_regex` is in
  place and tested against real compiler output, and errors already show in the
  output pane. Editor annotations arrive with 01-13.
