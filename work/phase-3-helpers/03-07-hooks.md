# 03-07 · Hooks

**Phase:** 3 · **Size:** M · **Crate/area:** `tinkerpad-library`, `tinkerpad-lsp`, `tinkerpad-plugin` · **Depends on:** 03-02, 01-04, 02-07
**Design:** PLUGINS.md §5 (Hooks)

## What

Wire the four hook messages into the places they belong: `playground_created`
after scaffold, `playground_opened`/`playground_closed` on LSP attach/detach,
`add_dependency` from the add-package command. Apply returned `files` and
`lsp_config`.

## Why

Hooks are the escape hatch that keeps language-specific logic out of the Rust
core. Go's `go.work` rewriting is the canonical need; having the mechanism proven
with the fake helper means the Go plugin (04-07) is a manifest plus a tiny script.

## Expectations

- [ ] `playground_created`: called after `[scaffold]` files are written; returned
      `files` are written (may overwrite scaffold output, since the hook is more
      specific).
- [ ] `playground_opened`/`closed`: called with the full list of currently open
      playgrounds of that language; returned `files` written; returned
      `lsp_config` merged into what `[lsp.register]` would send and delivered via
      `workspace/didChangeConfiguration`.
- [ ] `add_dependency`: if the plugin's helper lists it, it takes precedence over
      `[deps]`; returned `files` written.
- [ ] All hooks are best-effort: timeout or helper failure logs a warning and the
      operation continues as if the hook returned nothing. The user sees a toast
      only for `add_dependency` failures.
- [ ] File writes from hooks are constrained to the playground folder or the
      plugin's scratch dir; anything else is rejected and logged.
- [ ] Tests with the fake helper: each hook fires at the right moment with the
      right payload; returned files land; a path outside the sandbox is refused.

## Out of scope

New hook types. Add them when a real plugin needs one, with a protocol version
note.

## Gotchas

Starting a helper just to answer `playground_created` for a language that only
uses hooks (no `run`) is fine; idle shutdown will reap it.
