# 06-12 · `plugin check` covers `[debug]`

**Phase:** 6 · **Size:** S · **Crate/area:** `tinkerpad-app` · **Depends on:** 06-07, 05-01b
**Design:** DEBUGGING.md §1, PLUGINS.md §8

## What

A `debug` check in `tinkerpad plugin check`: the adapter starts, a session
reaches a breakpoint in the plugin's own sample, and it stops where it was
asked to.

## Why

Every other thing a plugin declares is checked by the conformance tool, which is
what makes a community plugin trustworthy without anyone reading it. A `[debug]`
section nobody verifies is a section that works on the author's machine.

## Expectations

- [ ] `check/debug.<ext>` joins the `check/` convention (05-01b): a playground
      with a line worth stopping on, and a marker saying which line. A plugin
      without one skips, with the report naming the file to add.
- [ ] The check starts a session, sets a breakpoint on that line, runs, and
      asserts it stopped there — with a deadline, so an adapter that never
      stops fails the check rather than hanging the checker.
- [ ] One variable is read at the stop and asserted to be legible: the sample
      declares what it should be, and the check compares. An adapter that
      attaches but shows nothing is an adapter that is not working.
- [ ] Continue to exit, and assert the session ends with nothing left running.
- [ ] A plugin with no `[debug]` skips the check and is not penalised.
- [ ] The bundled plugins each get a `check/debug.<ext>`, and CI runs it with
      the rest of the conformance job.
- [ ] PLUGINS.md §8.2 gains the file, as the other three are documented.

## Out of scope

Checking stepping, the call stack, or evaluate. If the adapter attaches, stops
where it was told, and can read a variable, the declaration is sound; the rest
is the adapter's business.

## Gotchas

This check needs a real adapter installed, so it will *skip* on most machines
and most CI runners — exactly as the language-server check does (05-01b). Skip
is not failure, and the report must make that obvious or people will read a
green run as more than it is.

The marker naming the breakpoint line has to survive the kind's locked prelude
being prepended (05-01b writes a sample between the locked lines). Name the line
by a comment in the source rather than by number.
