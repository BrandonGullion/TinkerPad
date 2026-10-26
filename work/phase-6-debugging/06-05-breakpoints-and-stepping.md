# 06-05 · Breakpoints, the current line, and the keys

**Phase:** 6 · **Size:** M · **Crate/area:** `ui/` · **Depends on:** 06-04
**Design:** DEBUGGING.md §1, D1, D15

## What

The editor half: a breakpoint gutter, a marker on the line that is stopped, the
keys that drive a session, and hover-to-evaluate.

## Why

This is the part people mean by "it has a debugger". Everything before it is
plumbing that nobody can use.

## Expectations

- [ ] A gutter you click to set a breakpoint, beside the annotation gutter from
      01-13 rather than instead of it: a line can have an error *and* a
      breakpoint, and that is a normal Tuesday.
- [ ] Breakpoints survive closing and reopening a tab, and are per playground.
      They live with the session, not in the file — nothing is written into
      someone's source.
- [ ] A breakpoint the adapter moved is drawn where it ended up, with a hint
      saying where it was asked for. A breakpoint the adapter refused is drawn
      as refused rather than silently kept.
- [ ] The stopped line is marked distinctly from the cursor, and the editor
      scrolls to it. Stepping moves the marker; the cursor stays where the
      person put it.
- [ ] Keys, through the catalogue so they are rebindable and documented
      (04-03): start, continue, pause, step over, step in, step out, stop.
      Follow the shortcuts people already have in their fingers (F5, F9, F10,
      F11) and let the catalogue sort out collisions. **F5 is already Run**
      (04-10, as in LINQPad). Decided there (PLAN.md decision log,
      2026-10-26): while a session is paused, F5 continues; starting a
      session needs a key of its own, picked here. `commandFor` already
      picks the first *enabled* command for a key, so a "continue" command
      bound to F5, enabled only while paused (with `run.start` disabled
      then), would work, but `applyBindings` reports any shared key as a
      conflict today. Teach it that two commands whose `when` conditions
      exclude each other don't clash, or make F5 one command that does the
      right thing in each state.
- [ ] Hovering an expression while stopped evaluates it in the selected frame
      and shows the result, without the hover fighting the LSP's (02-04).
- [ ] Stepping into a file that is not a playground opens it read-only (D15),
      which is the peek tab 02-09 already built.
- [ ] A self-test scenario driving a whole session against a bundled language:
      set a breakpoint, start, stop there, read a variable, step, continue,
      finish. Synthetic events, not keystrokes.

## Out of scope

Variables, Call Stack and Debug Console (06-06). Watch expressions and
conditional breakpoints — both worth having, both after this works.

## Gotchas

CodeMirror renumbers lines as the document changes, so a breakpoint has to be a
position that moves with an edit, not a line number remembered in a list. Use
the same mechanism the annotation gutter uses.

A click on the gutter and a click on the fold marker are a pixel apart. Test the
hit areas rather than trusting them.

While stopped, the editor is showing a file that may already differ from what is
running. 06-04 detects that; this item has to *say* it, somewhere that is not a
toast that disappears.
