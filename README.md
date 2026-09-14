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

## Running it

```sh
npm --prefix ui ci                  # once
ui/node_modules/.bin/tauri dev      # from the repo root
```
