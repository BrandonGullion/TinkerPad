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

- [x] Per-run: `SpawnSpec.stdin = true` keeps a pipe; the UI input box appears
      when a run starts and hides on exit. Text sent is echoed into the output
      pane in a distinct style.
- [x] Detect a likely stdin wait: if the process has been silent for 500 ms and
      stdin is open, highlight the input box. Heuristic only.
- [x] Helpers: extend the protocol with `{"type":"stdin","id":..,"data":..}` and
      `{"type":"stdin_eof","id":..}`; helpers declare `"stdin"` in `hooks` to opt
      in. Python and C# helpers implement it; update PLUGINS.md §5.
- [x] ANSI colours in output: parse SGR sequences in the output pane (a small
      parser; the OSC parser from 01-07 already skips unknown OSC codes).
- [x] PTY mode (optional, behind a per-plugin `[run].pty = true`): via
      `portable-pty`; only if a bundled language needs it. Record here whether it
      was needed.

## Out of scope

A full terminal emulator. Cursor-addressing programs.

## Gotchas

Windows pipes are fine for line-based stdin; only PTY mode needs ConPTY, which
`portable-pty` wraps.

## Results

Implemented 2026-09-28.

**A pipe, not `/dev/null`.** Every run now keeps its standard input open. A
program that reads waits for the user, which is what writing `input()` meant;
before, it read end-of-file and fell over. `RunEvent::Started` gained `stdin`,
so the window knows whether this run can be typed at.

**One shape for both paths**, as `Cancel` already is: `Input` is a channel per
run, and `Stdin::Data`/`Stdin::Eof` are what goes down it. The per-run side
writes to the child's pipe from a task of its own — a program that never reads
costs nothing — and the helper side sends protocol messages.

**The protocol** gained `{"type":"stdin","id":…,"data":…}` (base64, newline
included) and `{"type":"stdin_eof","id":…}`, opted into by declaring `stdin` in
`hooks`. A helper that doesn't gets neither the messages nor an input box for
its runs. `docs/PLUGINS.md` §5 documents both, with the two traps that cost a
hung helper: keep a handle on your *own* standard input before a run starts,
and close a cancelled run's input.

The second trap was not hypothetical — the Python helper hit it. Its protocol
loop read `sys.stdin`, and a run points `sys.stdin` at the user's typing, so
the helper started reading the user's input as protocol messages and answered
nothing again. Reproduced outside the app by driving the helper over a pipe,
fixed by capturing the real stream at import, the way it already captured
`sys.stdout`. The C# helper was safe by construction: it opens the standard
input stream directly, so `Console.SetIn` during a run can't reach it.

**Both bundled helpers implement it.** Python gets an `Input` object standing
in for `sys.stdin`, blocking on a condition variable until the core sends
something, with `.buffer` reading the same stream as bytes. C# gets a
`ConsoleInput : TextReader` behind `Console.SetIn`, with `ReadLine`, `Read`,
`Peek` and `ReadToEnd`. The node-check test helper passes input to the program
it spawns, which is what the self-test drives.

**In the window**, an input row appears under the output while a run that takes
input is going, and goes when the run does. Enter sends the line and echoes it
in the accent colour — what you typed is not what the program said. `Ctrl+D`
closes the input and prints `^D`. If the program says nothing for 500 ms the
row is highlighted: a guess, and a wrong guess costs nothing.

**ANSI colours** (`ui/src/ansi.ts`): a small state machine over SGR sequences,
carrying colour across chunks and across a sequence split between two reads.
The sixteen named colours are CSS variables, one set per theme (04-04); 256
and 24-bit colours become `rgb()`. Everything else an escape sequence might ask
for — cursor moves, clears, OSC titles — is dropped, because the pane is a
transcript, not a terminal.

**Tests.** A protocol test against the fake helper (typed lines arrive, and
end-of-input ends the program), and a new `stdin` self-test scenario: ten
checks, all passing, across the helper path, the per-run path, Python's
`input()` and C#'s `Console.ReadLine()`:

```
PASS a running program gets an input box
PASS a program that has gone quiet highlights the box
PASS a typed line reaches the program and is echoed in the pane
PASS what the user typed is styled as their own
PASS Ctrl+D ends the input and the program finishes
PASS the box goes when the run does
PASS a run with no helper can be typed at too
PASS Python's input() waits for what is typed and gets it
PASS C#'s Console.ReadLine() gets what is typed
PASS ANSI colours become styled text, and the escapes themselves are gone
```

## Checklist status

**PTY mode was not needed, and was not built.** The question the item asked to
record: every bundled language reads a pipe without complaint —
`Console.ReadLine()`, `input()`, and Node's `process.stdin` all work as they
are, on Windows included. A PTY would only be needed by a program that checks
`isatty` and changes behaviour, or one that draws with cursor movement, and
neither is what a playground is for. `[run].pty` is left unbuilt rather than
built unused; ANSI colours, the part of a terminal that playground output
actually uses, are handled in the pane instead.
