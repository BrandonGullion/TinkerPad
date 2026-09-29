# TinkerPad

[![CI](https://github.com/BrandonGullion/TinkerPad/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/BrandonGullion/TinkerPad/actions/workflows/ci.yml)

A LINQPad-style code playground for many languages. Rust core, plugin per language,
language servers and compilers kept warm.

Phase 0 (the skeleton) is done: an editor, an output pane, and runs of JavaScript
through Node with streaming output and cancel, on Windows, macOS, and Linux.
Language plugins start in Phase 1.

- [AGENTS.md](AGENTS.md) — workflow and engineering rules for contributors
- [PLAN.md](PLAN.md) — vision, decisions, roadmap, decision log
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — process model, crates, LSP keep-alive, execution
- [docs/PLUGINS.md](docs/PLUGINS.md) — plugin folder format, manifest, helper protocol, Dump() schema, per-language notes
- [docs/DEBUGGING.md](docs/DEBUGGING.md) — deferred debugger design and the constraints earlier phases must respect
- [work/README.md](work/README.md) — the plan broken into work items with acceptance criteria, by phase

## Installing it

Downloads are on the [releases page](https://github.com/BrandonGullion/TinkerPad/releases):
an installer for Windows, a `.dmg` for macOS, and an AppImage or `.deb` for Linux.
The app updates itself from there — it checks once, a few seconds after the window
appears, and says so in a toast you can ignore or click.

- **Windows.** The installer needs no administrator: it installs into your own
  profile and registers an uninstaller in Add or remove programs. WebView2 is
  downloaded by the installer on the rare machine without it.
- **macOS.** The app is not signed with an Apple Developer ID, so the first launch
  is refused by Gatekeeper. Right-click (or Control-click) TinkerPad in
  Applications, choose **Open**, and confirm; macOS remembers the answer.
- **Linux.** The AppImage is self-contained: make it executable and run it. The
  `.deb` needs `libwebkit2gtk-4.1-0` and `libgtk-3-0`, which `apt` installs with it.

Languages come from plugins, and each plugin needs its own toolchain on your
machine (Node, .NET, Python, Rust). TinkerPad says which are missing, and where to
get them, in the status strip.

## Running it

```sh
npm --prefix ui ci                  # once
ui/node_modules/.bin/tauri dev      # from the repo root
```
