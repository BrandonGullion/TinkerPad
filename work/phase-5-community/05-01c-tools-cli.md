# 05-01c · `tinkerpad tools list` and `tinkerpad tools install`

**Phase:** 5 · **Size:** S · **Crate/area:** `tinkerpad-app` (the subcommand) · **Depends on:** 04-08a, 04-08b, 05-01a
**Design:** PLUGINS.md §2 (`acquire`), ARCHITECTURE.md §4.1

> Split from 05-01 (see `05-01a-plugin-check-cli.md` for the checker). This is
> the other subcommand that item carried, and it shares its whole
> implementation with the window's own offers.

## What

What every language needs, and the ability to get it, without opening the
window.

```
$ tinkerpad tools list
Go            toolchain        missing   go
              language server  missing   gopls          can install
TypeScript    toolchain        ready     node 24.14.0
              language server  ready     …/tools/typescript/acquired/…
              workspace        missing   node_modules/typescript   can install

$ tinkerpad tools install go
go install golang.org/x/tools/gopls@latest
…
gopls is ready.
```

## Why

A CI runner, a new machine, or a Dockerfile wants the same thing the status bar
offers, without a display. It is also how 05-01b's CI job gets the toolchains it
needs to check the bundled plugins.

## Expectations

- [x] `tinkerpad tools list`, with `--json`, printing what `tools_status`
      already answers: every language, everything it needs, whether it is
      there, where it was found, and whether TinkerPad could install it.
- [x] `tinkerpad tools install <language> [--which toolchain|lsp|workspace]`,
      running exactly what the window's Install button runs, streaming the same
      output, and exiting non-zero if it fails.
- [x] It prints what it is about to do before doing it, as the window does.
      `--yes` for a script that can't answer; without it, a download asks.
- [x] `[tools] acquire = "never"` refuses here too, with the same sentence.
- [x] Shares `tools::status`, `tools::offer` and `tools::acquire` with the app
      (04-08a) rather than reimplementing any of it.
- [x] A test that lists and installs against the fake plugin and local server
      the 04-08a tests already use. Nothing reaches the network.

## Results

Measured on Windows 11, 2026-09-30.

**It is the window's answer, from a terminal.** `tools_status` and
`tools::acquire` are called unchanged; what this item added is the two things a
terminal needs and a window doesn't — somewhere to find the app's folders
without Tauri's path resolver, and somewhere for progress to go.

```
$ tinkerpad tools list
C#
  Toolchain        ready    dotnet 10.0.201  C:\Program Files\dotnet\dotnet.exe
Go
  Toolchain        missing  go  Install Go: https://go.dev/dl
  Language server  missing  gopls  go isn't installed, and it is what installs this
TypeScript
  Toolchain        ready    node 24.14.0  C:\Program Files\nodejs\node.exe
  Language server  ready    typescript-language-server  …\tools\typescript\acquired\…
  Workspace        ready    node_modules/typescript  C:\Users\Brandon\TinkerPad\node_modules/typescript
3 things missing. `tinkerpad tools install <language>` can get what it can.
```

That TypeScript row is the one worth looking at: the server it found is the one
the **window** installed into `<app-data>/tools` in 04-08b. The CLI builds the
same resolver — settings overrides, then acquired tools, then the discovered
`PATH` — so the two agree about what is installed. That is also the gap 05-01b
had to leave open; `plugin check` can close it now.

**Installing, for real.** The acquired server was deleted and put back through
the command line:

```
$ tinkerpad tools install typescript --which lsp
Language server: npm install typescript-language-server (into TinkerPad's own folder)
Nothing here runs without being asked: add --yes to go ahead.
Nothing was installed.                                            (exit 1)

$ tinkerpad tools install typescript --which lsp --yes
Language server: npm install typescript-language-server (into TinkerPad's own folder)
  added 3 packages in 5s
Done.                                                             (exit 0)
```

**Asking, with nobody there.** A terminal gets a prompt. A pipe gets told to
pass `--yes` and nothing runs — the alternative is a script that installs
software because it was run, which is the one thing the whole `[tools]` design
is against.

**Exit codes**: 0 when there was nothing to do or everything worked, 1 when
something failed or was refused, 2 for a usage mistake.

**Finding the app's folders without Tauri.** `dirs::data_dir()` plus the bundle
identifier gives the same path `app_data_dir()` does. The identifier is a
constant here, which could drift from `tauri.conf.json` without anyone
noticing, so a test reads that file and compares — if they ever disagree, the
CLI would quietly report on a folder the window cannot see, and this says so
instead. The plugins folder is found beside the executable, then in
`Contents/Resources` for macOS, then the repository, which is the same order
the window's resolver ends up at.

**Tests** — nine in `crates/tinkerpad-app/tests/tools_cli.rs`, over the same
fake package manager and local file server the acquisition tests use (04-08a).
Nothing reaches the network.

| Test | What it pins down |
|---|---|
| list says what is missing and whether it can be installed | the common case |
| list says why when it cannot | a plugin with no `acquire` falls back to its hint |
| list as JSON is the same answer the window gets | shape and field names |
| install asks before it runs anything | exit 1, nothing run, `--yes` named |
| install with `--yes` runs it and says what it said | the tool's own output is forwarded |
| install downloads through the mirror and says where it landed | URL and checksum shown first, `<app-data>/tools` after |
| a download that fails its checksum is reported | exit 1, the reason quoted |
| `acquire = "never"` refuses here too | exit 1, nothing run |
| a language that is not here says so | exit 2 |

`Context::over` exists for those tests: `Context::open` reads the machine's own
settings, plugins and `PATH`, which is right for the command line and useless
for a test.

## Out of scope

The checker (05-01a, 05-01b). Installing plugins themselves (05-02).

## Gotchas

`tools::Env` is built from `AppState` today. It needs a way to exist without a
window — the data it holds is a data directory, an event sink, a resolver and
two settings, none of which needs Tauri.

Progress arrives as `CoreEvent::Tool`; on a terminal that wants to be lines on
stdout, not an event bus nobody is draining.
