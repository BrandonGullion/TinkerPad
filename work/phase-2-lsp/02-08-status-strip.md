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

- [x] One chip per server: plugin name, a state dot (grey stopped, amber starting,
      green ready, red crashed), and RSS in MB. Chips for `Stopped` servers are
      hidden unless the language has an open tab.
- [x] Popover: uptime, restarts, open documents, messages/sec, request p50/p95
      (from 02-03), buttons Restart / Stop / Open log, and the resolved binary path.
- [x] A per-server log viewer: the last 500 stderr lines and `logMessage`s in a
      panel with copy-all. Not a full log browser.
- [x] "Install hint" shown in the chip popover when the server binary is missing,
      with a copyable command.
- [ ] Also shows plugin helper processes once 03-02 exists (same chip design; leave
      a hook for it).

## Out of scope

Settings editing from here (04-02). Graphs.

## Gotchas

Sample-based numbers (RSS, latency) should update at 1 Hz at most; don't re-render
the strip on every LSP message.

## Results

Implemented 2026-09-17.

**The strip** (`ui/src/servers.ts`) sits at the right of the status bar: one chip
per language server, each a coloured dot, the language, and its resident memory.
Grey stopped, amber starting, green ready, red crashed — and a starting or
crashed dot pulses, because those are the two states that are about to change.
A stopped server keeps its chip only while a tab still wants it.

**The popover** answers "what is this thing doing?" in one click: the process id,
the version it reported, how long it has been up, its memory, how many documents
it has open, how many requests it has answered and how quickly (p50 and p95), the
restarts it has come back from, its workspace, and the program that was actually
started. Then the three things worth doing — **Restart**, **Stop**, and **Log**.

**Log** shows what the server itself said: its stderr and its
`window/logMessage`s, the last 500 lines, with Copy all. That is the difference
between "the Rust server is red" and "the Rust server says it can't find
cargo.toml".

**A server that never started** shows its reason and the plugin's install hint
with a Copy button, which is the whole of 02-07's "install this yourself"
instruction in the place where you find out you need it.

**The numbers**

Everything shown is a sample, so the strip polls `server_status` once a second
and refreshes immediately on a `server` event — never per LSP message. Memory is
sampled by the pool every ten seconds (02-06), so a new chip shows its memory a
few seconds after it turns green.

**Where they come from**

- `ServerHandle` now keeps its own traffic: messages received, requests sent, and
  a 512-entry window of round trips for the percentiles. A request is timed from
  the moment it goes out to the moment its answer is routed, so the number covers
  the whole trip including the server's own queue. All of it resets when a
  process starts, because it describes the process, not the handle.
- The handle also keeps the server's last 500 log lines. `TransportEvent` gained
  a `Stderr(String)` variant to carry them: they were already being logged, and
  are now also kept for the strip.
- `server_status` grew to what the strip shows, and two commands joined it:
  `server_log` and `stop_server` (`restart_server` was already there from 02-05).

**Tests** — an in-app `servers` scenario, 7 checks: a server appears in the bar;
its chip says ready with a dot and its memory; clicking it shows what the server
is doing; Log shows what it has said; Stop stops it and the chip stays while a
tab wants it; Restart brings it back; and with no tab open a stopped server is
not shown. It runs in the language-server launch in CI, which already has a
server to look at.

Verified by screenshot as well as by assertion: chip, popover, and every row of
numbers, on Windows.

## Checklist status

**Open: helper processes.** The item asks for plugin helpers to appear as chips
once 03-02 exists. The hook is that the strip renders a list of plain
`ServerStatus` records and nothing in it is LSP-specific: give a helper the same
shape and it gets a chip with no changes here. Noted in the file.

**Deviation: no separate log panel.** The item describes "a per-server log viewer
in a panel"; the log opens inside the popover instead, scrolled to the end, with
Copy all. It is 500 lines — a panel would be a bigger thing than the content
deserves, and the popover keeps it next to the server it belongs to. If it starts
feeling cramped, the panel is a small change from here.
