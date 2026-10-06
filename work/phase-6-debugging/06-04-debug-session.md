# 06-04 · A debug session, in the core

**Phase:** 6 · **Size:** M · **Crate/area:** `tinkerpad-app`, `tinkerpad-dap` · **Depends on:** 06-02, 06-03
**Design:** DEBUGGING.md §1, D1, D3, D7, D12, D13

## What

Starting, driving and ending a debug session for one playground: breakpoints,
run control, stack, scopes, variables, evaluate. Everything except what it looks
like.

## Why

The window needs one thing to talk to, with the adapter on the far side of it —
the same shape the LSP proxy has, and for the same reason. The editor should not
know which debugger it is driving.

## Expectations

- [ ] `debug_start(playground)` resolves the plugin's `[debug]`, spawns the
      adapter through 06-02, does the handshake, sends the breakpoints the
      editor already has, and `configurationDone`.
- [ ] **Launch** builds a `RunSpec` the way a run does (D13) and hands it to the
      adapter rather than spawning it: same program, same args, same env, plus
      what `[debug]` adds. A playground that runs must debug the same code.
- [ ] **Attach** starts the program with the extra arguments and environment
      `[debug]` names (D3, D10, D11), reads the port the program reports, and
      attaches. Where an adapter wants a process id rather than a port, that is
      the pid `tinkerpad-process` already exposes (D3) — neither needs anything
      new from the spawn layer, and if it does, that is a finding worth
      recording.
- [ ] A helper that can start a run stopped at entry does so through a `debug`
      field on `run` (D7), with no protocol version bump.
- [ ] Breakpoints are editor lines sent as source lines, with no mapping at all
      (D1). A breakpoint the adapter *moves* is reported back, so the window can
      show where it actually went rather than where it was asked for.
- [ ] `continue`, `pause`, `step over`, `step in`, `step out`, stop. `stopped`
      carries the reason, the thread and the top frame.
- [ ] Stack, scopes and variables are fetched lazily — a variable tree expands a
      level at a time rather than being walked on every stop. A stop must be
      instant even with a large object in scope.
- [ ] `evaluate`, with the selected frame as context, for hover and the console.
- [ ] Output events reach the output pane as a run's do, and `Dump()` still
      works, because it rides on stdout and nothing here touches it.
- [ ] No timeouts anywhere in the session (D12). A program stopped at a
      breakpoint is silent for as long as the person is thinking.
- [ ] Ending a session leaves nothing running: the adapter and the program it
      started are both gone.
- [ ] Tests against the fake adapter: a session that stops and continues, one
      where the program exits first, one where the adapter refuses to launch,
      and one ended from the window while stopped.

## Out of scope

The gutter and the panels (06-05, 06-06). Any real adapter.

## Gotchas

A breakpoint set before the session starts and one set while stopped are the
same request to the adapter but different states in the window. Pick one source
of truth — the editor's — and resend the whole set for a file whenever it
changes, which is what DAP expects anyway.

`stopped` can name a thread the window has never heard of. Ask for threads
rather than assuming the one in the event is the only one.

A playground edited while stopped no longer matches what is running. Say so
rather than silently stepping through lines that have moved.
