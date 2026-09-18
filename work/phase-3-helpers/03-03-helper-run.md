# 03-03 · Helper-run path with fallback

**Phase:** 3 · **Size:** S · **Crate/area:** `tinkerpad-runner` · **Depends on:** 03-02, 01-06
**Design:** ARCHITECTURE.md §5.1

## What

When a plugin declares `[helper]` with `"run"` in its `hooks`, Mod+Enter goes to
the helper. If the helper is unavailable (build failed, gave up restarting, missing
binary), fall back to `[run]` and tell the user once.

## Why

This is the switch that turns the warm runner on. It must be invisible when it
works and honest when it doesn't.

## Expectations

- [x] `RunnerSpec` resolution: `Helper` if `[helper]` exists and lists `run`;
      otherwise `PerRun`. If `Helper` is chosen but the pool reports `Failed` or
      `GaveUp`, use `PerRun` and emit a one-time toast per session: "C# helper
      unavailable (reason); using dotnet run".
- [x] `RunEvent` stream is identical from either path; the UI cannot tell them
      apart except for a small "warm" badge in the status bar for helper runs.
- [x] Cancel goes to the right place: helper `cancel` or process kill.
- [x] One run per playground still holds; two playgrounds of the same language
      queue on the single helper (protocol allows one run at a time). The queued
      run shows "waiting for helper" in its tab.
- [x] A `Force per-run` command palette entry for debugging, sticky per session.
- [x] Test with the fake helper: run routes to helper; with `crash_on_run` and
      give-up reached, next run routes to per-run with the toast event emitted once.

## Out of scope

Real helpers (03-04..06).

## Gotchas

Save-before-run still applies: the helper reads the file from disk, same as the
per-run path. Don't pass source text over the protocol.

## Results

Implemented 2026-09-18.

**The switch.** A run goes to the plugin's helper when it has one whose `hooks`
include `run`, and to `[run]` otherwise. `tinkerpad_runner::helper_run::start`
takes a pool, a config and a `RunRequest` and gives back the same `RunHandle`
the per-run path gives back — same `RunEvent`s, through the same
`OutputDecoder` and the same dump parser — so the output pane, annotations,
cancel and the per-playground run slot are written once and work for both.

**Nothing waits before the run exists.** Getting a helper can mean building it
or waiting for another playground's turn, so the run is registered and its id
returned first, and the waiting happens inside it. That is what makes the tab
able to say what it is waiting for: `RunEvent::Waiting { message }` for another
playground's turn, and the build's own output as this run's `stdout` — the
"Building the C# helper…" header included. Before this, the `run` command sat
unanswered while a helper was built and the UI could only show an optimistic
"Running".

**Queueing, not cancelling.** A helper runs one program at a time, so a second
playground of the same language waits on the helper's run slot
(`HelperHandle::take_run_slot`) rather than cancelling the first. The protocol's
"a second `run` cancels the first" now only happens where it means something: a
person pressing Run twice in the same playground.

**Cancel goes to the right place.** `RunHandle` carries a `Cancel` that is
either a process kill or a helper's `cancel` message, and a helper run may be
cancelled before it has started — in which case the wait ends and the run is
reported cancelled without anything having run.

**Falling back.** Any `Unavailable` (build failed, wouldn't start, given up on
after crashing) drops the run to `[run]` and shows one toast per plugin per
session: *"The Node check helper isn't available (the node-check helper could
not be built: the build command exited 3). Running with node instead."* The run
itself still produces its output and exit code; the user loses warmth, not work.

**Telling the two apart.** Exactly one thing does: `warm` on `started`, which
the status bar renders as `Running 108 ms · warm`. **Force per-run execution**
in the command palette turns helpers off for the session, and says which way it
went.

**Measured** (Windows, the test plugin's helper, which spawns `node` so the two
paths are genuinely comparable): Mod+Enter to first output **50.4 ms** on the
first warm run and **42.3 ms** on the second, against 42 ms for the same program
on the per-run path in 01-06. The number to watch here is not the saving — this
helper saves nothing on purpose — but that routing a run through a helper costs
nothing measurable: a queue, a message, and a base64 hop.

**Tests** — 7 integration tests against real processes with the fake helper: a
helper run producing the same stream as a process (`warm: true`, the same
output, the same exit), a dumped value arriving as a dumped value through the
same parser, cancel asking the helper and leaving it warm, two playgrounds
queueing and the second running when the first ends, a compile error ending the
run with somewhere to point, a build shown live to the run waiting for it, and a
helper that keeps dying being reported so the caller can fall back.

**End to end in the app**, as a `helper-run` self-test scenario in CI, twelve
checks: a warm run says so and is otherwise identical; the helper is free again
afterwards; cancelling a warm run stops it and leaves the helper warm; a second
playground says "Waiting for the Node check helper" and runs when the first
ends; Force per-run keeps runs off the helper without changing anything else; a
helper that can't be built doesn't stop the run; the user is told what is being
used instead, once and not every run.

**The test plugin's helper now runs the file in a child process** rather than
evaluating it in-process. That is not what a real warm runner does, and the
comment says so — it is there to make the helper behave *exactly* like the
plugin's `[run]` command, so the existing `run`, `tabs`, `locked` and
`annotations` scenarios now exercise the helper path without being rewritten.
They pass unchanged, which is the strongest statement available that the two
paths are indistinguishable.

## Checklist status

**Note: two existing self-test checks were adjusted, both for real reasons.**
"Runs in different playgrounds overlap" and the tabs scenario's per-tab run
state both run two programs of the same language at once, which a single helper
cannot do; both now ask for per-run execution for that part, with a comment
saying why. The queueing they used to imply is checked in `helper-run` instead.
The `run` scenario also warms the helper before its first timing, so the build's
output doesn't land in the middle of a comparison of program output.

**Note: `RunnerSpec` stayed a decision rather than a type.** ARCHITECTURE.md
§5.1 sketches `enum RunnerSpec { PerRun, Helper }`; what ships decides per run
inside `Runs::start`, because both arms need the same per-run spec anyway — the
helper arm keeps it as the fallback. A type holding "a helper config *and* the
command to use if it doesn't work" would be the same two values with a name.

**Note: the "warm" badge is in the status bar, not a badge.** The item asks for
"a small warm badge in the status bar"; it is the word `· warm` after the
elapsed time, which is where the run's other facts already are.
