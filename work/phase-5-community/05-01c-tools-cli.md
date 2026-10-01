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

- [ ] `tinkerpad tools list`, with `--json`, printing what `tools_status`
      already answers: every language, everything it needs, whether it is
      there, where it was found, and whether TinkerPad could install it.
- [ ] `tinkerpad tools install <language> [--which toolchain|lsp|workspace]`,
      running exactly what the window's Install button runs, streaming the same
      output, and exiting non-zero if it fails.
- [ ] It prints what it is about to do before doing it, as the window does.
      `--yes` for a script that can't answer; without it, a download asks.
- [ ] `[tools] acquire = "never"` refuses here too, with the same sentence.
- [ ] Shares `tools::status`, `tools::offer` and `tools::acquire` with the app
      (04-08a) rather than reimplementing any of it.
- [ ] A test that lists and installs against the fake plugin and local server
      the 04-08a tests already use. Nothing reaches the network.

## Out of scope

The checker (05-01a, 05-01b). Installing plugins themselves (05-02).

## Gotchas

`tools::Env` is built from `AppState` today. It needs a way to exist without a
window — the data it holds is a data directory, an event sink, a resolver and
two settings, none of which needs Tauri.

Progress arrives as `CoreEvent::Tool`; on a terminal that wants to be lines on
stdout, not an event bus nobody is draining.
