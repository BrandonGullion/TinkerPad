# 03-08 · Run timing and history

**Phase:** 3 · **Size:** S · **Crate/area:** `tinkerpad-runner`, `tinkerpad-library`, `ui/` · **Depends on:** 01-06, 01-12
**Design:** PLAN.md §5 Phase 3

## What

Record each run's start time, duration, exit code, runner kind (per-run / helper),
and the first error line, per playground. Show a small history dropdown in the
status bar and keep the last N outputs viewable.

## Why

"How long did that take last time?" and "what did the output look like before I
changed this?" are constant questions in a scratchpad. Cheap to answer if recorded.

## Expectations

- [x] `runs.jsonl` inside each playground folder, appended per run: timestamp,
      duration_ms, exit code, runner, first annotation message if any. Capped at
      the last 200 entries (rewrite when exceeded).
- [x] The last 5 runs' full output (text + rich nodes, as the event list) kept in
      memory per tab; older ones dropped. Not persisted.
- [x] Status bar: last run duration with a subtle up/down indicator vs the previous
      run; click for a list of recent runs; selecting one shows its retained output
      if still in memory, or just its metadata.
- [x] Timing breakdown when available: for per-run, time-to-first-byte vs total;
      for helpers, `duration_ms` from the helper vs wall time in the core (the gap
      is protocol overhead; log it).
- [x] `runs.jsonl` is ignored by the library index and never shown as a file.

## Out of scope

Diffing outputs. Cross-playground stats. Any charting.

## Gotchas

Write `runs.jsonl` off the run's hot path (after `Exited`, on a blocking task).

## Results

Implemented 2026-09-20.

**`crates/tinkerpad-library/src/history.rs`** owns the file. One JSON object
per line in `runs.jsonl` beside the playground, appended when a run ends,
capped at 200 — under the cap an append, at it a rewrite without the oldest
entries. Lines rather than a document because a run only ever appends: a crash
halfway through a write costs the last line, and a line that doesn't parse (a
field from a later build, a half-written line) is skipped rather than failing
the read.

A record:

```json
{"run":2,"startedMs":1789941204170,"durationMs":28,"outcome":"exited","code":0,
 "runner":"helper","firstOutputMs":24,"reportedMs":28}
```

`run` is the id the run had in the session that wrote it — meaningless after a
restart, and exactly what the window needs to match a line to output it still
holds. `error` is the first annotation the plugin's `[output]` pattern found,
falling back to the first line the run said on stderr when it ended badly and
there was nothing to point at.

**Where the time goes.** `durationMs` is measured in the core, from the run
being handed to the forwarder to its last event: what the user waited for, with
the queue and the helper build left out (those are reported as waiting, and are
not the program's time). `reportedMs` is what the runner itself said — the
process's own wall time, or a helper's `duration_ms` — and the gap between them
is the overhead of getting there and back, logged at debug on every run and
shown on hover in the list. Measured here on the test plugin: warm run 28 ms
with 28 ms reported, per-run 32 ms with 32 ms reported. The protocol overhead
is under a millisecond in both directions, which is the answer the breakdown
exists to give.

**The window.** `RunController` keeps the last 5 runs' events per tab, up to
5000 events each (a run that prints more than that is remembered as having
happened, and says so when shown). Replaying a run re-applies its events to the
pane, so a dump is still a dump and an annotation is still clickable. Nothing is
persisted: a restart leaves the numbers in `runs.jsonl` and the outputs gone,
and the list says which is which.

**The status bar** compares each run with the one before it: `Exit 0 · 28 ms
▼ 94%`, shown only when the difference is at least 15% and 3 ms, so the arrow
doesn't flicker on a quiet machine. The indicator is now a button; clicking it
opens the run history of the active playground — time, outcome, duration,
whether a helper ran it, and the first thing that went wrong.

**Tests.** Five unit tests on the file (order, the cap, a garbage line, a field
this build doesn't know, no history at all). In the app, a new `history`
self-test scenario, ten checks, all passing:

```
PASS a playground that has never run has no history
PASS each run is recorded, newest first, with what it did
PASS with the runner that ran it and where its time went
PASS the status bar compares this run with the one before it
PASS a run that ends badly records its code and the first thing it said
PASS clicking the run indicator lists the recent runs, newest first
PASS selecting a run shows the output it had
PASS the last five runs keep their output; the ones before are metadata only
PASS runs.jsonl is not listed as a playground, and does not disturb the tab
PASS a run with no helper is recorded as per-run, with its own time and first byte
```

## Checklist status

**Note: the file is written after the last event, on a blocking thread**, as
the gotcha asks — `tokio::task::spawn_blocking` once the forwarder's loop has
ended, so nothing the user is waiting for touches the disk.

**Note: `[deps]` commands are not runs.** `cargo add` streams into the pane like
a run, but it isn't one of the playground's runs and isn't recorded.

**Deviation: the watcher ignores `runs.jsonl` explicitly.** The criterion asked
for the index to ignore it, which it already would (the index reads folders,
not files). Skipping it in the watcher too means recording a run can't cause a
library refresh or a conflict bar on the tab that ran it.
