# 06-02 · Debug adapter pool

**Phase:** 6 · **Size:** M · **Crate/area:** `tinkerpad-dap`, `tinkerpad-app` · **Depends on:** 06-01, 03-02
**Design:** DEBUGGING.md §1, D5

## What

A third pool, beside the language servers and the plugin helpers, holding debug
adapters: spawn, supervise, report state, stop when the session ends.

## Why

D5 made pool supervision generic for exactly this. If this item turns out to
need more than a few hundred lines, that constraint was not actually honoured,
and the fix is to make the shared code fit all three rather than to write a
third supervisor.

## Expectations

- [ ] Built on the same `Pool` the LSP and helper pools use. Anything it needs
      that the shared code lacks is added there, not forked.
- [ ] One adapter per session, not per language: two playgrounds being debugged
      at once are two adapters. They are not kept warm — an adapter with no
      session is a process with no purpose, and the whole point of a warm
      process is that it has state worth keeping.
- [ ] An adapter that exits while a session is live ends the session with a
      reason the window can show, and does **not** restart. A debugger that
      silently reattaches is a debugger that lies about where you are.
- [ ] The status strip (02-08) shows a live session the way it shows a server
      and a helper, with Stop.
- [ ] The adapter's own stderr is kept and readable, like a server's log: an
      adapter that refuses to start usually says why there and nowhere else.
- [ ] Tests with the fake adapter from 06-01: a session that ends cleanly, an
      adapter that dies, two sessions at once, and stopping one from the strip.

## Out of scope

What a session actually does (06-04). Per-language adapters.

## Gotchas

`tinkerpad-process` kills a whole tree (00-04). An adapter that launched the
program being debugged is that program's parent, so ending a session must take
the debuggee with it. That is the behaviour already there — worth a test rather
than an assumption, because the failure mode is an orphaned process holding a
port.
