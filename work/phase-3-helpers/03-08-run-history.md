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

- [ ] `runs.jsonl` inside each playground folder, appended per run: timestamp,
      duration_ms, exit code, runner, first annotation message if any. Capped at
      the last 200 entries (rewrite when exceeded).
- [ ] The last 5 runs' full output (text + rich nodes, as the event list) kept in
      memory per tab; older ones dropped. Not persisted.
- [ ] Status bar: last run duration with a subtle up/down indicator vs the previous
      run; click for a list of recent runs; selecting one shows its retained output
      if still in memory, or just its metadata.
- [ ] Timing breakdown when available: for per-run, time-to-first-byte vs total;
      for helpers, `duration_ms` from the helper vs wall time in the core (the gap
      is protocol overhead; log it).
- [ ] `runs.jsonl` is ignored by the library index and never shown as a file.

## Out of scope

Diffing outputs. Cross-playground stats. Any charting.

## Gotchas

Write `runs.jsonl` off the run's hot path (after `Exited`, on a blocking task).
