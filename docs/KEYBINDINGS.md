# Keybindings

Generated from `ui/src/commands/catalog.js` by `npm run docs:keys` (run from
`ui/`). Don't edit this file; edit the catalog and run it again.

Every shortcut can be changed in `<app-data>/keybindings.toml`, one line per
command, with an empty string to take a key away:

```toml
"run.start" = "mod+shift+r"
"tab.close" = ""
```

`Mod` is Cmd on macOS and Ctrl everywhere else; write `ctrl` or `cmd` for a
binding meant for one platform only.

## Run

| Command | macOS | Windows and Linux | Id |
|---|---|---|---|
| Run | ⌘↩ | Ctrl+Enter | `run.start` |
| Cancel run | ⌘. | Ctrl+. | `run.cancel` |
| Clear output | ⌘L | Ctrl+L | `run.clearOutput` |
| Force per-run execution | — | — | `run.forcePerRun` |
| Recent runs | — | — | `run.history` |

## Playground

| Command | macOS | Windows and Linux | Id |
|---|---|---|---|
| New playground | ⌘N | Ctrl+N | `playground.new` |
| New playground… | ⇧⌘N | Ctrl+Shift+N | `playground.newOfKind` |
| Open playground… | ⌘O | Ctrl+O | `playground.open` |
| Rename playground | F2 | F2 | `playground.rename` |
| Add package… | — | — | `playground.addPackage` |

## Tabs

| Command | macOS | Windows and Linux | Id |
|---|---|---|---|
| Close tab | ⌘W | Ctrl+W | `tab.close` |
| Next tab | ⌃⇥ | Ctrl+Tab | `tab.next` |
| Previous tab | ⌃⇧⇥ | Ctrl+Shift+Tab | `tab.previous` |

## Go

| Command | macOS | Windows and Linux | Id |
|---|---|---|---|
| Next error or warning | F8 | F8 | `go.nextProblem` |
| Previous error or warning | ⇧F8 | Shift+F8 | `go.previousProblem` |
| Back | ⌥← | Alt+Left | `go.back` |

## View

| Command | macOS | Windows and Linux | Id |
|---|---|---|---|
| Library panel | ⌘B | Ctrl+B | `view.library` |
| Commands… | ⇧⌘P | Ctrl+Shift+P | `view.commands` |

## Settings

| Command | macOS | Windows and Linux | Id |
|---|---|---|---|
| Settings… | ⌘, | Ctrl+, | `settings.open` |
| Open settings file | — | — | `settings.file` |
| Open keybindings file | — | — | `settings.keybindings` |
| Copy keybindings as Markdown | — | — | `settings.copyKeybindings` |
| Open log folder | — | — | `settings.logs` |
