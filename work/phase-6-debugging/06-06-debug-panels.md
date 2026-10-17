# 06-06 · Variables, Call Stack and Debug Console

**Phase:** 6 · **Size:** M · **Crate/area:** `ui/` · **Depends on:** 06-04
**Design:** DEBUGGING.md §1, D2

## What

Three tabs beside Output in the bottom panel: what is in scope, how you got
here, and somewhere to ask a question.

## Why

D2 made the bottom area a tabbed container with Output as its first tab, so
these dock rather than displace. Output keeps working while you are stopped,
and `Dump()` keeps arriving in it.

## Expectations

- [ ] The three tabs appear when a session starts and go when it ends. They are
      not empty tabs the rest of the time, and Output stays selected unless the
      person has chosen otherwise.
- [ ] **Variables**: the scopes of the selected frame, expanded lazily — a level
      is fetched when it is opened, never before. A tree with ten thousand
      children shows the first page and says there are more, the way the dump
      tree already does (01-08).
- [ ] A variable's value is editable where the adapter says it is, and the
      field says so when it isn't, rather than accepting an edit and dropping
      it.
- [ ] **Call Stack**: frames, with the selected one driving Variables and
      hover-evaluate. Clicking a frame opens its file — a playground in its tab,
      anything else as a peek (D15). Frames the adapter marks as library or
      subtle are dimmed rather than hidden: hiding them is how people lose half
      an hour.
- [ ] **Debug Console**: evaluate in the selected frame, with history, and the
      program's own output interleaved in order. Rich values from `Dump()`
      render here as they do in Output rather than as JSON.
- [ ] Everything reuses the dump renderer (01-07, 01-08). A debugger that
      renders a list differently from the output pane is two things to learn.
- [ ] A self-test scenario: stop, read a nested variable, select a lower frame,
      watch Variables change, evaluate something in the console.

## Out of scope

Watch expressions. Memory and disassembly views. Setting a variable through the
console rather than the panel.

## Gotchas

`variablesReference` is only valid until the next `continue`. Fetching a stale
one is an error from the adapter that looks like a bug in the panel; drop every
reference on `continued` and refetch.

The console's input and the output pane's stdin box (04-05) are different
things that look identical. While a session is stopped the program is not
reading stdin, so showing both invites typing into the wrong one — show the
console's.
