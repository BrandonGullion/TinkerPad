# 06-07 · Debugging TypeScript and JavaScript

**Phase:** 6 · **Size:** M · **Crate/area:** `plugins/typescript` · **Depends on:** 06-05, 06-06
**Design:** DEBUGGING.md §1, D10

## What

The first real language: attach to a Node process started with `--inspect`, via
`js-debug` in its DAP server mode.

## Why

D10 says Node first because attach-to-inspector is the simplest path, and
because the TypeScript plugin has both a per-run path and a warm helper — so it
proves the two modes the rest will need.

## Expectations

- [ ] A per-run debug session: the run is started with `--inspect=0`, reports
      its port on stderr, and the adapter attaches before any user code runs.
- [ ] A **helper** debug session: the warm helper (03-06) starts the run stopped
      at entry when asked through the `debug` field on `run` (D7). Warm
      execution and debugging coexist — that is the whole LINQPad model, and
      this is where it is first shown to work.
- [ ] Breakpoints in a `.ts` playground stop on the right lines. Node strips
      types and preserves line numbers (`--experimental-transform-types`), so
      there is no source map to honour — if that stops being true, say so here
      rather than quietly adding a mapping step, which D1 exists to prevent.
- [ ] Variables show useful values: a `Map`, a `Set`, a class instance and a
      promise are each legible rather than `Object`.
- [ ] Stepping into `node_modules` opens the file read-only, and the frames are
      dimmed rather than hidden.
- [ ] The adapter is acquired, not bundled: `[debug]` carries an `acquire`
      the way `[lsp]` does (04-08a), so "install the debugger" is the button the
      window already has.
- [ ] Timings recorded: time from pressing the key to being stopped at the first
      breakpoint, warm and cold.

## Out of scope

Debugging a `.js` playground differently from a `.ts` one — they are the same
plugin and the same path. Browser targets.

## Gotchas

`--inspect=0` picks a port and prints it on **stderr**, in a line that also
contains a URL. Read the port from it rather than guessing, and do not let that
line reach the output pane as program output.

js-debug's DAP server mode and its VS Code extension mode are different entry
points. The server one is the supported-by-us path; pin the version in the
`acquire`, because its arguments have changed before.

A helper that has run other playgrounds has their modules cached. A breakpoint
in a module already loaded will work; a fresh `require` of an edited file may
not re-read it. 03-06's `retiring` message exists for exactly this sort of
accumulation — use it rather than inventing something.
