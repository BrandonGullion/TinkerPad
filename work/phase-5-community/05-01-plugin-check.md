# 05-01 · Plugin conformance CLI

**Phase:** 5 · **Size:** L · **Crate/area:** `tinkerpad-cli` · **Depends on:** 01-01, 01-06, 01-07, 03-01, 03-07
**Design:** PLUGINS.md §8 (Conformance), PLAN.md §5 Phase 5

## What

`tinkerpad plugin check <dir>`: loads the manifest, probes the toolchain,
scaffolds a playground of every kind into a temp library, runs each, asserts the
output, asserts a `dump` round-trip, exercises every declared hook and the helper
lifecycle, and prints a report. Exit code 0 only if everything passes.

## Why

This is what makes community plugins trustworthy without a review team. If a
contributor's plugin passes `check`, it works. CI runs it on the bundled plugins so
the core can't break them silently.

## Expectations

- [ ] Ships as a subcommand of the main binary (`tinkerpad plugin check`) or a
      separate `tinkerpad-cli` binary that links the same crates; pick one and
      note it in 00-01's gotcha.
- [ ] Checks, each reported pass/fail/skip with a one-line reason:
      1. Manifest loads (01-01 errors surfaced verbatim).
      2. Toolchain probe succeeds (skip the rest with a clear message if not).
      3. Every template renders; every `[scaffold]` file is written.
      4. Each kind's fresh playground runs with exit code 0 and produces some
         stdout within 60 s.
      5. A `dump` smoke test: the plugin provides `check/dump.<ext>` (a playground
         that dumps a known table); the tool asserts a `table` node with the
         expected columns and row count arrives.
      6. `[output].error_regex`: the plugin provides `check/error.<ext>` (a
         playground with a compile/runtime error); at least one annotation with a
         line number is produced.
      7. If `[helper]`: build (if declared) succeeds; `hello`/`ready` within 10 s;
         `run` works; `cancel` on a `check/hang.<ext>` playground completes within
         3 s; each declared hook responds within 5 s with a well-formed result;
         `shutdown` exits cleanly.
      8. If `[lsp]`: server spawns, `initialize` succeeds, a `didOpen` of the
         `dump` playground produces zero error diagnostics within 30 s.
- [ ] `--json` output for CI; `--only <check>` to iterate on one.
- [ ] Runs against every bundled plugin in CI on Windows (and other OSes when
      04-06 enables them); toolchains installed by the workflow.
- [ ] `check/` folder convention documented in PLUGINS.md §8 and the template
      repo (05-03).

- [ ] `tinkerpad tools list` and `tinkerpad tools install <language>`, sharing
      the code the app's own offers use (04-08a, 04-08b), so a machine or a CI
      runner can be set up without opening the window.

## Out of scope

Performance assertions (report timings, don't fail on them). Security scanning.

## Gotchas

The checker must use the exact same code paths as the app (same loader, same
runner, same parser), not reimplementations, or it will pass plugins the app
rejects.
