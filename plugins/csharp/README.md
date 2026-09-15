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

## Speed

Every run compiles `main.cs`, because TinkerPad saves the file before running it.
How long that takes depends mostly on whether .NET's compiler server is already
running.

| Run | Compiler server already running | Cold compiler server |
|---|---|---|
| First run of a new playground | about 1 s | about 2.7 s |
| Rerun or small edit | about 0.3 s | about 1.9 s |

These were measured on Windows 11. On Windows today, a compiler server that
TinkerPad starts ends with the run that started it, so runs inside TinkerPad
usually see the cold numbers. A warm helper (work item 03-05) will remove the
compile step.

## Files

| Path | Purpose |
|---|---|
| `plugin.toml` | The manifest. |
| `templates/` | New playground contents, including `Directory.Build.props`. |
| `dump/TinkerPad.Dump.cs` | The dump library. |
