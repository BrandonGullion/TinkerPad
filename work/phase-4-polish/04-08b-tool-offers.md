# 04-08b · Offering to install what a language needs

**Phase:** 4 · **Size:** M · **Crate/area:** `ui/`, `tinkerpad-app`, `plugins/*` · **Depends on:** 04-08a, 02-08
**Design:** PLUGINS.md §2 (`[toolchain]`), ARCHITECTURE.md §4.1

> Split from 04-08 (see `04-08a-acquiring-tools.md` for the machinery). This half
> is what the user sees and what the bundled plugins declare.

## What

"rust-analyzer isn't installed. Install it?", one button, in the place that
already says so — and somewhere to see the whole picture for every language.

## Why

Right now a language works only if you already happen to have its server. That is
a bad first five minutes for an app whose pitch is "open it and start typing",
and it gets worse with every language added.

## Expectations

- [x] **Never silent, never automatic.** Missing tool → an offer, in the status
      strip (02-08) and in the toast that already reports it, showing exactly what
      will happen: the command line to be run, or the URL, size, and checksum to
      be fetched. Installing is one click and one confirmation, and nothing is
      installed without it.
- [x] While it runs, the user can see it: a command's output streams somewhere
      they can read, a download shows progress, and both can be left alone —
      neither blocks the window.
- [x] Failure is ordinary: the hint is shown, the reason is kept, and the offer
      can be made again. A machine with `[tools] acquire = "never"`, or with no
      network, sees the hint and no offer rather than a failing download.
- [x] Workspace dependencies count as tools: the TypeScript server needs a
      `typescript` in the library folder (02-07), so "install what this language
      needs" has to be able to run that `npm install` too, not just fetch a
      binary.
- [x] The bundled plugins declare what they need: rustup for rust-analyzer, npm
      for typescript-language-server plus the workspace's TypeScript, gopls for Go
      (04-07), and whatever C# settles on.
- [x] Somewhere to see it all: a list of what each language needs, whether it is
      there, where it was found, and a button per missing one. The status strip
      (02-08) is the natural home; a language with everything it needs should say
      so as plainly as one without.
- [x] Tests: the offer appears for a missing tool and not for a present one;
      accepting runs the acquisition and the language becomes usable without a
      restart; `acquire = "never"` shows the hint and no button.

## Results

Measured on Windows 11, 2026-09-29: `cargo test --workspace`, plus the in-app
`tools` and `tools-acquire` self-test scenarios.

**Where the pieces live**

| Piece | File |
|---|---|
| What each language needs | `crates/tinkerpad-app/src/tools.rs` (`Need`, `LanguageNeeds`, `status`, `needs`) |
| The command the window calls | `tools_status` in `crates/tinkerpad-app/src/lib.rs` |
| The panel and the offers | `ui/src/tools.ts` |
| Its place in the status bar | `ui/src/layout.ts`, `View → What each language needs` in the palette |
| Workspace dependencies | `[lsp].workspace_needs` / `[lsp.workspace_acquire]`, `Which::Workspace` |
| The plugins' own declarations | `plugins/{rust,go,typescript,python}/plugin.toml` |

**Three kinds of thing a language needs.** The toolchain (what `[toolchain]`
probes for), the language server (what `[lsp]` runs), and now the workspace —
something the server needs *in its workspace* rather than on `PATH`. The third
exists because typescript-language-server analyses with a TypeScript it finds in
the library folder, and an offer that can't run that `npm install` would leave
the language half-working.

**Where installs land.** `{tools_dir}` in an acquire command or its `env` is the
plugin's own folder under `<app-data>/tools/`, which the resolver searches.
`npm install --prefix {tools_dir}` and `GOBIN={tools_dir} go install` both put
their programs somewhere the app finds without elevation and without touching
`PATH`; `<prefix>/bin`, `<prefix>/Scripts` and `<prefix>/node_modules/.bin` are
searched too, because that is where package managers actually put things.

**What the bundled plugins declare.**

| Plugin | Offer |
|---|---|
| Rust | `rustup component add rust-analyzer` (needs rustup) |
| Go | `go install golang.org/x/tools/gopls@latest` with `GOBIN = {tools_dir}` (needs go) |
| TypeScript | `npm install --prefix {tools_dir} typescript-language-server typescript` (needs npm), plus `npm install` in the library folder for the workspace |
| Python | `pip install basedpyright` (needs pip) |
| C#, PowerShell | No offer. Both need an SDK install that can want administrator rights, which the item puts out of scope: a hint, not a button. |

**Accepting an offer, end to end.** The `tools-acquire` scenario, from an empty
`<app-data>/tools/`:

| Step | Result |
|---|---|
| Offer shown before anything runs | the command line, exactly as it will run |
| `npm install --prefix {tools_dir} typescript-language-server typescript` | 1,253 ms, output streamed to the row |
| The app finds it, no restart | `…/tools/typescript/acquired/node_modules/.bin/typescript-language-server.cmd` |
| The offer disappears | nothing is offered for something present |
| `npm install` in the library folder | 948 ms |
| The server starts and reaches ready | `{"kind":"ready","pid":50280}` |

Nothing was restarted at any point: the core re-probes after an acquisition and
rebuilds the resolver, so the same question answers differently a moment later.

**Self-tests.** `TINKERPAD_SELFTEST=tools`, 15 checks, all passing:

```
PASS every loaded plugin is in the list, with at least its toolchain
PASS a language with a server gets a row for it, and one without doesn't
PASS a workspace dependency is a need of its own, with a path rather than a program
PASS nothing is offered for a tool that is already installed
PASS the status bar says how many things are missing, and only when some are
PASS clicking it opens a panel with a block per language
PASS a language with everything it needs says so
PASS TypeScript: a missing tool with an offer gets a button
PASS the button confirms first, showing exactly what would happen
PASS declining puts the button back and installs nothing
PASS a language missing something shows the plugin's own instructions, once
PASS acquire = "never" blocks every offer and keeps the hints
PASS and the panel shows no Install button at all
PASS clicking again closes the panel
```

`tools` never installs anything and never reaches the network, so it belongs in
any batch. `tools-acquire` does both, so it is a scenario of its own that nothing
runs by default.

Six more Rust tests in `crates/tinkerpad-app/tests/tools.rs` cover the same
questions without a window: a present tool has no offer, a missing one carries
the offer and the hint, `acquire = "never"` blocks it while keeping the hint, a
plugin with no `acquire` gets a row and no button, and a workspace dependency
flips from missing to present when the path appears.

**Two bugs this found.**

- `[tools]` was never written to `settings.toml`. `save_settings` writes each
  section explicitly (04-02) and the section added in 04-08a had been missed, so
  switching offers off in the window worked until the next launch. Fixed, with
  the setting added to the round-trip test.
- `cargo run` and `tauri dev` stopped working after 04-08a added a second
  binary to `tinkerpad-app`: neither could tell which one to run. Fixed with
  `default-run = "tinkerpad"`.

**Deliberately not done.** No control for `[tools] acquire` in the settings
window: the file documents it in place, and the offers themselves are where the
decision is actually made. The CLI (`tinkerpad tools list` / `install`) is 05-01c
and shares `tools_status` and `acquire_tool` as they now stand.

## Out of scope

The acquisition machinery itself (04-08a). The CLI (`tinkerpad tools list` and
`tinkerpad tools install <language>`), which is a subcommand in 05-01c and shares
the same code.

## Gotchas

The status strip is small and this list is not; it needs a home that can hold a
table without crowding what the strip is for.

An offer that appears while someone is typing is an interruption. The toast
already exists and is ignorable; the strip is where the offer should live.
