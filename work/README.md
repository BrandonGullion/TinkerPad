# Work items

One file per item, grouped by phase. Each item is meant to be finishable in one
sitting to one week of evenings and has acceptance criteria you can tick off.
Design references point into `../PLAN.md`, `../docs/ARCHITECTURE.md`, and
`../docs/PLUGINS.md`; the work item says *what done looks like*, the design doc
says *how it fits*.

**Sizes:** S = an evening · M = a weekend · L = a week of evenings.

**Order:** items within a phase are roughly ordered; "Depends on" is the real
constraint. Don't start a phase until the previous phase's "done when" in
`PLAN.md` §5 holds.

Tick the box here when an item is finished. An item can be ticked with open
criteria when the remainder can only be verified later (on real hardware, after
time passes, or by a later item); the item file says why each one is open.

## Phase 0 — Skeleton

- [x] [00-01 Cargo workspace and crate skeletons](phase-0-skeleton/00-01-workspace.md) · S
- [x] [00-02 Tauri 2 shell and UI scaffold](phase-0-skeleton/00-02-tauri-shell.md) · S · 3 open criteria
- [x] [00-03 Editor and output pane layout](phase-0-skeleton/00-03-editor-and-output-pane.md) · M · 3 open criteria
- [x] [00-04 Process supervision crate](phase-0-skeleton/00-04-process-supervision.md) · M · 1 open criterion
- [x] [00-05 Core to UI event channel and command surface](phase-0-skeleton/00-05-event-channel.md) · S · 1 open criterion
- [x] [00-06 Hard-coded run, end to end, with cancel](phase-0-skeleton/00-06-hardcoded-run.md) · S · 1 open criterion
- [x] [00-07 Logging and error conventions](phase-0-skeleton/00-07-logging.md) · S · 4 open criteria
- [x] [00-08 Cross-platform CI and platform modules](phase-0-skeleton/00-08-cross-platform-ci.md) · S · 1 open criterion

## Phase 1 — Plugins, first languages, Dump()

- [x] [01-01 Manifest schema and loader](phase-1-plugins/01-01-manifest-loader.md) · M
- [x] [01-02 Plugin discovery](phase-1-plugins/01-02-plugin-discovery.md) · S
- [x] [01-03 Toolchain probing](phase-1-plugins/01-03-toolchain-probing.md) · S · 3 open criteria
- [x] [01-04 Playground library on disk](phase-1-plugins/01-04-playground-library.md) · M · 1 open criterion
- [x] [01-05 Kinds, templates, scaffold, locked lines](phase-1-plugins/01-05-kinds-and-scaffold.md) · M
- [x] [01-06 Per-run runner from manifest](phase-1-plugins/01-06-per-run-runner.md) · S · 1 open criterion
- [x] [01-07 Dump sentinel parser](phase-1-plugins/01-07-dump-parser.md) · M · 1 open criterion
- [x] [01-08 Dump renderers in the UI](phase-1-plugins/01-08-dump-renderers.md) · L
- [x] [01-09 TypeScript plugin](phase-1-plugins/01-09-typescript-plugin.md) · M
- [x] [01-10 C# plugin](phase-1-plugins/01-10-csharp-plugin.md) · M · 1 open criterion
- [x] [01-11 Rust plugin](phase-1-plugins/01-11-rust-plugin.md) · M · 1 open criterion
- [x] [01-12 Tabs, new-tab flow, autosave](phase-1-plugins/01-12-tabs-and-autosave.md) · M
- [x] [01-13 Clickable error annotations](phase-1-plugins/01-13-error-annotations.md) · S
- [x] [01-14 Dependencies: add package](phase-1-plugins/01-14-dependencies.md) · S

## Phase 2 — Language servers, kept alive

- [x] [02-01 JSON-RPC transport](phase-2-lsp/02-01-jsonrpc-transport.md) · S
- [x] [02-02 Server pool: spawn, share, keep alive, shutdown](phase-2-lsp/02-02-server-pool.md) · M · 1 open criterion
- [x] [02-03 Proxy: URI rewriting and id ownership](phase-2-lsp/02-03-proxy.md) · M · 1 open criterion
- [x] [02-04 Editor LSP client](phase-2-lsp/02-04-editor-lsp-client.md) · M · 1 open criterion
- [x] [02-05 Crash restart and replay](phase-2-lsp/02-05-crash-restart.md) · S
- [x] [02-06 Idle policy and pre-warm](phase-2-lsp/02-06-idle-and-prewarm.md) · S
- [x] [02-07 Workspace strategies and registration](phase-2-lsp/02-07-workspace-strategies.md) · M · 2 open criteria
- [x] [02-08 Server status strip](phase-2-lsp/02-08-status-strip.md) · S · 1 open criterion
- [x] [02-09 Go to definition](phase-2-lsp/02-09-go-to-definition.md) · S · 1 open criterion
- [x] [02-10 A deleted playground leaves its language server](phase-2-lsp/02-10-deleted-playground-detach.md) · S · 1 deferred criterion

## Phase 3 — Plugin helpers and warm runners

- [x] [03-01 Helper protocol and fake helper](phase-3-helpers/03-01-helper-protocol.md) · M
- [x] [03-02 Helper pool and build-on-first-use](phase-3-helpers/03-02-helper-pool.md) · M
- [x] [03-03 Helper-run path with fallback](phase-3-helpers/03-03-helper-run.md) · S
- [x] [03-04 Python plugin and reference helper](phase-3-helpers/03-04-python-plugin.md) · M
- [x] [03-05 C# Roslyn helper](phase-3-helpers/03-05-csharp-helper.md) · L
- [x] [03-06 TypeScript Node helper](phase-3-helpers/03-06-typescript-helper.md) · S
- [x] [03-07 Hooks](phase-3-helpers/03-07-hooks.md) · M
- [x] [03-08 Run timing and history](phase-3-helpers/03-08-run-history.md) · S

## Phase 4 — Polish

- [x] [04-01 Library search, tags, recents](phase-4-polish/04-01-library-search.md) · M
- [x] [04-02 Settings](phase-4-polish/04-02-settings.md) · M
- [x] [04-03 Command palette and keybindings](phase-4-polish/04-03-command-palette.md) · M
- [x] [04-04 Themes](phase-4-polish/04-04-themes.md) · S
- [x] [04-05 Stdin and interactive programs](phase-4-polish/04-05-stdin.md) · M
- [ ] [04-06 Installer and updates](phase-4-polish/04-06-installer.md) · M
- [x] [04-07 Go and PowerShell plugins](phase-4-polish/04-07-go-and-powershell.md) · M
- [x] [04-08a Acquiring a language's tools](phase-4-polish/04-08a-acquiring-tools.md) · M
- [x] [04-08b Offering to install what a language needs](phase-4-polish/04-08b-tool-offers.md) · M
- [x] [04-09 Deleting a playground something else has open](phase-4-polish/04-09-deleting-a-playground-in-use.md) · S

## Phase 5 — Community

- [x] [05-01a `tinkerpad plugin check`: the tool and the first four checks](phase-5-community/05-01a-plugin-check-cli.md) · M
- [x] [05-01b `plugin check`: dump, errors, helpers and servers](phase-5-community/05-01b-plugin-check-deep.md) · M
- [x] [05-01c `tinkerpad tools list` and `tinkerpad tools install`](phase-5-community/05-01c-tools-cli.md) · S
- [x] [05-02 Plugin install](phase-5-community/05-02-plugin-install.md) · M
- [x] [05-03 Plugin template repo](phase-5-community/05-03-plugin-template.md) · S · 1 deferred criterion
- [x] [05-04 Published docs and versioning policy](phase-5-community/05-04-docs-and-versioning.md) · M · 2 partly deferred
- [x] [05-05 Registry index](phase-5-community/05-05-registry.md) · S

## Phase 6 — Debugging

The design is [docs/DEBUGGING.md](../docs/DEBUGGING.md), and D1 to D15 there are
the constraints every earlier phase carried to keep this door open. Each is named
by the item that now spends it.

The order is the one §4 of that document prescribed: the client, then the pool,
then a session, then what you can see — and Node first among the languages,
because attach-to-inspector is the simplest path and proves both modes.

- [ ] [06-01 DAP transport and client](phase-6-debugging/06-01-dap-transport.md) · M
- [ ] [06-02 Debug adapter pool](phase-6-debugging/06-02-adapter-pool.md) · M
- [ ] [06-03 `[debug]` in the manifest](phase-6-debugging/06-03-debug-manifest.md) · S
- [ ] [06-04 A debug session, in the core](phase-6-debugging/06-04-debug-session.md) · M
- [ ] [06-05 Breakpoints, the current line, and the keys](phase-6-debugging/06-05-breakpoints-and-stepping.md) · M
- [ ] [06-06 Variables, Call Stack and Debug Console](phase-6-debugging/06-06-debug-panels.md) · M
- [ ] [06-07 Debugging TypeScript and JavaScript](phase-6-debugging/06-07-typescript-debugging.md) · M
- [ ] [06-08 Debugging Python](phase-6-debugging/06-08-python-debugging.md) · S
- [ ] [06-09 Debugging C#](phase-6-debugging/06-09-csharp-debugging.md) · M
- [ ] [06-10 Debugging Rust](phase-6-debugging/06-10-rust-debugging.md) · M
- [ ] [06-11 Debugging Go](phase-6-debugging/06-11-go-debugging.md) · S
- [ ] [06-12 `plugin check` covers `[debug]`](phase-6-debugging/06-12-check-covers-debug.md) · S

06-07 is the first item with a user-visible debugger. Everything before it is
plumbing that cannot be used, and everything after it is the same plumbing
pointed at another language — so 06-07 is where this phase is either working or
not.

## Item template

```markdown
# NN-NN · Title

**Phase:** N · **Size:** S/M/L · **Crate/area:** ... · **Depends on:** NN-NN, NN-NN
**Design:** ARCHITECTURE.md §x, PLUGINS.md §y

## What
## Why
## Expectations
- [ ] ...
## Out of scope
## Gotchas
```
