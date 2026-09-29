# The PowerShell plugin

The smallest plugin TinkerPad bundles, and deliberately so: `plugin.toml` has
only the sections a plugin can't do without — `[language]`, `[toolchain]`, one
`[[kind]]`, and `[run]` — plus an `[output]` pattern so errors are clickable.
No language server, no dependency command, no helper, and no `dump/` folder.

If you are writing a plugin for a language nobody has built tooling for, start
by copying this one.

## What runs

`pwsh -NoProfile -NonInteractive -File main.ps1`. `-NoProfile` so a profile
can't change what a playground does, and `-NonInteractive` so a prompt fails
loudly instead of hanging; what you type in the input box still reaches the
script's own reads (work item 04-05).

On Windows the toolchain probe falls back to `powershell` (5.1), which is on
every Windows machine, so the plugin works without installing anything. On
macOS and Linux `pwsh` is a download; when it is missing the plugin says so in
the status strip and nothing else in TinkerPad notices.

## The locked lines

Every script starts with three things TinkerPad needs, which the editor keeps
read-only:

```powershell
$ErrorView = 'NormalView'
$ErrorActionPreference = 'Stop'

function Dump { ... }
```

- **`$ErrorView = 'NormalView'`** turns off PowerShell 7's boxed error display,
  whose message comes last and whose position is drawn with line art. The
  classic view puts the message first and the position second, which is what
  `[output].error_regex` reads to make an error clickable.
- **`$ErrorActionPreference = 'Stop'`** makes an error end the run. A
  playground that prints an error and then exits 0 looks like it worked; this
  is a scratchpad, and a mistake should stop it.
- **`Dump`** is the whole of rich output: about twenty lines that turn a value
  into the JSON sentinel TinkerPad reads out of stdout (docs/PLUGINS.md §6). It
  takes the pipeline, so `$things | Dump` is one table rather than one dump per
  object:

```powershell
Get-Process | Select-Object Name, CPU -First 5 | Dump -Title 'processes'
42 | Dump -Title 'answer'
Dump -Value @{ a = 1 } -Title 'a map'
```

A collection of objects becomes a table, a scalar becomes a value with its type,
and anything else becomes a collapsible tree.
