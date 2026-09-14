# 02-08 · Server status strip

**Phase:** 2 · **Size:** S · **Crate/area:** `ui/`, `tinkerpad-app` · **Depends on:** 02-02, 02-06
**Design:** PLAN.md §5 Phase 2

## What

A section of the status bar showing every language server the pool knows about:
state, memory, uptime, and request latency. Click for a popover with Restart, Stop,
and Open log.

## Why

When something is slow or broken, the user needs to see which server it is and
kick it, without opening a log file. It also makes the keep-alive behaviour visible,
which builds trust in it.

## Expectations

- [ ] One chip per server: plugin name, a state dot (grey stopped, amber starting,
      green ready, red crashed), and RSS in MB. Chips for `Stopped` servers are
      hidden unless the language has an open tab.
- [ ] Popover: uptime, restarts, open documents, messages/sec, request p50/p95
      (from 02-03), buttons Restart / Stop / Open log, and the resolved binary path.
- [ ] A per-server log viewer: the last 500 stderr lines and `logMessage`s in a
      panel with copy-all. Not a full log browser.
- [ ] "Install hint" shown in the chip popover when the server binary is missing,
      with a copyable command.
- [ ] Also shows plugin helper processes once 03-02 exists (same chip design; leave
      a hook for it).

## Out of scope

Settings editing from here (04-02). Graphs.

## Gotchas

Sample-based numbers (RSS, latency) should update at 1 Hz at most; don't re-render
the strip on every LSP message.
