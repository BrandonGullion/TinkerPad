# 05-01b · `plugin check`: dump, errors, helpers and servers

**Phase:** 5 · **Size:** M · **Crate/area:** `tinkerpad-app` (the subcommand), `tinkerpad-runner`, `tinkerpad-lsp` · **Depends on:** 05-01a, 01-07, 03-07
**Design:** PLUGINS.md §8 (Conformance), §5 (helper protocol), §6 (dump)

> Split from 05-01 (see `05-01a-plugin-check-cli.md` for the tool and the first
> four checks). This part is the half that needs the plugin to provide sample
> playgrounds, and the half that exercises its long-lived processes.

## What

The checks that prove a plugin's *rich* behaviour works, not just that it runs:
that `Dump` produces the nodes the UI expects, that `[output].error_regex`
catches a real error, that the helper honours the protocol, and that the
language server starts and says the sample is fine.

A `check/` folder in the plugin supplies the playgrounds these need.

## Why

Running is the easy half. A plugin whose `Dump` emits malformed nodes, whose
`error_regex` never matches, or whose helper ignores `cancel` is one that looks
fine until someone uses it. These are the checks that would have caught every
plugin bug found by hand so far (04-07 alone found four).

## Expectations

- [x] A documented `check/` folder convention: `check/dump.<ext>`,
      `check/error.<ext>`, `check/hang.<ext>`. A plugin without one skips the
      checks that need it, with a message saying which file to add.
- [x] **Dump** — `check/dump.<ext>` dumps a known table; the checker asserts a
      `table` node arrives with the expected columns and row count, through the
      same parser the app uses (01-07).
- [x] **Errors** — `check/error.<ext>` has a compile or runtime error; at least
      one annotation with a line number is produced by `[output].error_regex`.
- [x] **Helper**, if `[helper]` is declared: `build` succeeds if present;
      `hello`/`ready` within 10 s; a `run` works; `cancel` on
      `check/hang.<ext>` completes within 3 s; every declared hook answers
      within 5 s with a well-formed `result`; `shutdown` exits cleanly.
- [x] **Language server**, if `[lsp]` is declared: it spawns, `initialize`
      succeeds, and a `didOpen` of the dump playground produces zero error
      diagnostics within 30 s.
- [x] Every bundled plugin passes, and gets whatever `check/` files it needs.
- [x] CI runs `plugin check` over every bundled plugin, on the platforms whose
      toolchains the workflow installs.
- [x] The convention documented in PLUGINS.md §8, so 05-03's template repo can
      ship it.

## Results

Measured on Windows 11, 2026-09-30.

**A bug in the app, found by the first thing this was pointed at.** PowerShell
errors were never annotated — not by the checker, and not by the window
either. PowerShell 7 colours its error output even when that output is a pipe,
so the escape sequence sits between the newline and the `At ` the plugin's
`error_regex` is anchored on, and the pattern silently never matches.

Fixed in `Annotator::line`, which now strips SGR sequences before matching.
That is the right place: a pattern should match what a tool said, not what
colour it said it in, and the fix covers every language rather than the one
that was noticed. `line 30:1: this is the mistake` now, where before there was
nothing at all.

**Every bundled plugin, after being given a `check/` folder:**

| Plugin | Result |
|---|---|
| C# | 12 passed — dump, errors, and a helper that builds in 4.5 s and cancels in 8 ms |
| Go | 1 passed, 3 skipped — no Go on this machine |
| PowerShell | 6 passed — including the errors check that was broken |
| Python | 11 passed, 1 skipped — server not installed |
| Rust | 8 passed — including rust-analyzer finding nothing wrong with the sample |
| TypeScript | 11 passed, 1 skipped — server not installed |

Nothing failed. **Go could not be checked beyond its manifest**: this machine
has no Go toolchain, so its `check/` files are written but unverified. The CI
job will be the first thing to run them.

**Numbers worth keeping**, from the helper checks:

| Plugin | ready | warm run | cancel | shutdown |
|---|---|---|---|---|
| Python | 71 ms | 13 ms | 1 ms | 9 ms |
| TypeScript | 65 ms | 26 ms | **2,015 ms** | 0 ms |
| C# | 4,545 ms | 57 ms | 8 ms | 16 ms |

TypeScript's cancel is two seconds — inside the three the item allows, but an
order of magnitude slower than the others, and worth knowing before someone
notices it by pressing the button. C#'s four and a half seconds to ready is a
compile, and only on the first run.

**The `check/` convention.** `check/dump.<ext>`, `check/error.<ext>`,
`check/hang.<ext>`, documented in PLUGINS.md §8.2. Two decisions worth
recording:

- **The table is fixed** — columns `name` and `n`, rows `ada`/1 and
  `grace`/2 — so the checker can assert rather than guess. Column names are
  compared without case, so C#, Go and PowerShell can name fields the way
  those languages do.
- **A sample is written between the kind's locked lines, not over them.**
  PowerShell's `Dump` is defined in its kind's prelude; a sample written over
  the top of that would be testing a function that isn't there. So a sample is
  what a person would type, which is also what makes it readable.

**Not installed is a skip, everywhere.** A missing language server skips, as a
missing toolchain does: this machine lacking basedpyright says nothing about
the Python plugin. One gap is recorded in the code: the checker searches `PATH`
and the settings overrides, but not the folder the window acquires tools into
(04-08a), because it has no app data directory of its own. A server installed
only through the Languages button therefore skips here. `tinkerpad tools`
(05-01c) brings that directory to the command line and can close it.

**Tests** — eight more in `crates/tinkerpad-app/tests/check.rs`, seventeen in
total, all using `tinkerpad-faketool`: a dump that matches, one with a renamed
column, a run that dumps nothing, a missing `check/` folder (asserting the
report names the file to add), an error sample that produces a position, one
the pattern misses, one that succeeds when it should fail, and a plugin with no
`[output]` section. `tinkerpad-faketool` gained a `table` step that writes a
dump payload on the wire, so these need no language installed.

**CI** runs `plugin check` over every plugin folder on all three platforms. A
toolchain a runner hasn't got is a skip, so the job stays useful as languages
are added rather than becoming a list of exclusions.

## Out of scope

The tool itself and checks 1–4 (05-01a). `tinkerpad tools` (05-01c).

## Gotchas

A hook check has to use the real helper pool, not a hand-rolled client, or it
proves nothing about what the app will do.

`cancel` is the check most likely to hang the checker itself: give every wait a
deadline, and fail the check rather than the process.
