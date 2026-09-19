# C# plugin

Runs C# playgrounds as .NET 10 file-based apps: a single `main.cs`, no project
file, run with `dotnet run --file main.cs`. Includes `.Dump()` for rich output.

## What it needs

The .NET 10 SDK (`dotnet --version` should print 10.0 or later):
https://dot.net. If `dotnet` isn't on your `PATH`, point TinkerPad at it in
`settings.toml`:

```toml
[toolchains.csharp]
dotnet = "C:/Program Files/dotnet/dotnet.exe"
```

## Kinds

| Kind | Starts with |
|---|---|
| **C# Statements** (default) | Top-level statements. |
| **C# Program** | A `Program` class with `static void Main`. |
| **C# Expression** | One expression, shown with `Dump`. The lines around it are locked. |

## `Dump`

```csharp
using TinkerPad;

var people = db.People.Where(p => p.Born < 1900).ToList().Dump("people");
Dump.Show(DateTime.Now);
```

`value.Dump(title)` shows `value` in the output pane and returns it, so it
chains. `Dump.Show(value, title)` is the same thing as a static call.

| Value | Shown as |
|---|---|
| A sequence whose items all have public properties or fields (records, classes, anonymous types) | A sortable table, with columns from every member in first-seen order |
| Any other object, collection, or dictionary | A collapsible tree |
| Numbers, strings, `bool`, `char`, enums, dates and times, `Guid`, `Type` | A single value with its C# type |

Details:

- **Big values.** Tables keep 1,000 rows and mark themselves truncated. Only
  1,001 items are ever enumerated, so an endless `IEnumerable` is fine.
  Collections and objects keep 1,000 items.
- **Nesting.** Five levels are shown; deeper objects become a summary such as
  `[Person]`. A reference back to an enclosing object shows as `"[circular]"`.
- **Errors.** A property that throws shows `[error: message]` instead of
  stopping the dump.
- **Numbers.** Integers too big for JavaScript, such as `ulong.MaxValue`, are
  shown as strings, and so are `NaN` and `Infinity`.
- **Text.** Console output is switched to UTF-8 before your code runs, so
  non-ASCII text arrives intact.

## NuGet packages

Add `#:package` directives at the very top of `main.cs`, above any `using`.
Comments may come first.

```csharp
#:package Humanizer@2.14.1
using Humanizer;
using TinkerPad;

"PascalCaseText".Humanize().Dump();
```

Other file-based-app directives work the same way, such as
`#:property LangVersion=preview` or `#:sdk Microsoft.NET.Sdk.Web`.

## How `Dump` gets into your playground

Every C# playground gets a `Directory.Build.props` next to `main.cs`. When
TinkerPad runs the playground, it sets `TINKERPAD_CSHARP_DUMP` to this plugin's
`dump/TinkerPad.Dump.cs`, and the props file compiles that file in with a global
`using TinkerPad`. No install path is written into your playground: move the app
and it still works. Outside TinkerPad the variable isn't set, so a playground that
calls `Dump` won't build there.

The props file also:
- turns off the file-based-app native-AOT defaults, which warn about
  reflection and block it;
- sets nullable to annotations only, so `string?` works without warnings on
  every run.

## The helper

C# runs on a **warm helper** (work item 03-05): a small .NET console app that
keeps Roslyn in memory and runs each playground in this process instead of
starting a compiler and a runtime every time.

TinkerPad builds it the first time you run C#, with
`dotnet build -c Release`, into your app data folder rather than the plugin
folder — an installed plugin may live somewhere read-only. The build takes a
few seconds and happens once; you see its output in the results pane of the run
that triggered it, and it runs again only when something in this folder
changes. If the build fails — a machine with the .NET runtime but no SDK, for
example — the run falls back to `dotnet run --file` and says so once.

What a run does, in `helper/`:

| File | What it is |
|---|---|
| `Program.cs` | The protocol loop: `hello`, `run`, `cancel`, `shutdown`. |
| `Protocol.cs` | JSON lines in and out. Nothing here knows what a run is. |
| `Runner.cs` | Parse, compile, load into a collectible context, run, unload. |
| `References.cs` | The .NET reference set, read once and kept. |
| `Packages.cs` | `#:package` directives, resolved with NuGet. |
| `Sources.cs` | Reading `main.cs`, blanking `#:` lines without moving any. |
| `Diagnostics.cs` | Roslyn's errors, in the shape the protocol wants. |

Three things are worth knowing:

**Your code runs in the helper's process.** Each run gets a collectible
`AssemblyLoadContext`, so types, statics and loaded assemblies go when the run
does. What doesn't go is anything your code started and left running — a
`Task.Run` that never finishes, a timer — which keeps its context alive; the
helper says so on its log and the memory stays until you **Restart** it from
the status bar chip.

**Errors point at your file.** The compilation emits portable PDBs and is
parsed with the real path of `main.cs`, so an unhandled exception's stack trace
reads `main.cs:line 7`, exactly as it does under `dotnet run`. A compile error
comes back as the compiler's own diagnostic — file, line, column — rather than
text a pattern had to match, and lands as a squiggle in the editor.

**Packages are resolved, not restored.** `#:package Humanizer@2.14.1` is
downloaded into the same global packages folder `dotnet restore` uses
(`~/.nuget/packages`), with its dependencies, and kept in memory for the rest of
the session. A package you already have is not fetched again. What this does
*not* do is resolve version conflicts the way a full restore would: if two
packages want different versions of the same thing, the higher one wins.

## Speed

Every run compiles `main.cs`, because TinkerPad saves the file before running
it. On the helper that compile is tens of milliseconds, because the compiler,
the .NET reference set and the dump library are already in memory.

| Run | With the helper | Per-run (`dotnet run --file`) |
|---|---|---|
| First run of a new playground | about 1.5 s (starting the helper) | about 2.2 s |
| Rerun, unchanged | **76 ms** | about 1.0 s |
| Run after an edit | **104 ms** | about 1.2 s |

Measured on Windows 11 with .NET 10.0.201, from Mod+Enter to the output being
on screen. The helper's own figure — compile, load, run — is about 50 ms of
that.

To compare for yourself, **Force per-run execution** in the command palette
turns the helper off for the session.

## Files

| Path | Purpose |
|---|---|
| `plugin.toml` | The manifest. |
| `templates/` | New playground contents, including `Directory.Build.props`. |
| `dump/TinkerPad.Dump.cs` | The dump library. |
| `helper/` | The warm runner, built on first use. |
