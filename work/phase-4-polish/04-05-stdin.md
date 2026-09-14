# 04-05 · Stdin and interactive programs

**Phase:** 4 · **Size:** M · **Crate/area:** `tinkerpad-process`, `tinkerpad-runner`, `ui/` · **Depends on:** 01-06, 03-03
**Design:** ARCHITECTURE.md §5.1, PLAN.md §6

## What

Let a running program read from stdin: an input line at the bottom of the output
pane, sent on Enter, with EOF (`Ctrl+D`) support. Optionally a PTY mode for
programs that check `isatty` or need ANSI colour.

## Why

`Console.ReadLine()` and `input()` show up in half of all quick experiments. A
playground that hangs on them is a trap.

## Expectations

- [ ] Per-run: `SpawnSpec.stdin = true` keeps a pipe; the UI input box appears
      when a run starts and hides on exit. Text sent is echoed into the output
      pane in a distinct style.
- [ ] Detect a likely stdin wait: if the process has been silent for 500 ms and
      stdin is open, highlight the input box. Heuristic only.
- [ ] Helpers: extend the protocol with `{"type":"stdin","id":..,"data":..}` and
      `{"type":"stdin_eof","id":..}`; helpers declare `"stdin"` in `hooks` to opt
      in. Python and C# helpers implement it; update PLUGINS.md §5.
- [ ] ANSI colours in output: parse SGR sequences in the output pane (a small
      parser; the OSC parser from 01-07 already skips unknown OSC codes).
- [ ] PTY mode (optional, behind a per-plugin `[run].pty = true`): via
      `portable-pty`; only if a bundled language needs it. Record here whether it
      was needed.

## Out of scope

A full terminal emulator. Cursor-addressing programs.

## Gotchas

Windows pipes are fine for line-based stdin; only PTY mode needs ConPTY, which
`portable-pty` wraps.
