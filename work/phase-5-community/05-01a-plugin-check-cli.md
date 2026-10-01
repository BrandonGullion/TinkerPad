# 05-01a · `tinkerpad plugin check`: the tool and the first four checks

**Phase:** 5 · **Size:** M · **Crate/area:** `tinkerpad-app` (the binary), `tinkerpad-plugin`, `tinkerpad-runner` · **Depends on:** 01-01, 01-06, 03-01
**Design:** PLUGINS.md §8 (Conformance), PLAN.md §5 Phase 5

> Split from 05-01 (see `05-01b-plugin-check-deep.md` for the dump, error,
> helper and server checks, and `05-01c-tools-cli.md` for `tinkerpad tools`).
> This part is the tool itself and the checks that need nothing from the plugin
> beyond what it already ships.

## What

A command that loads a plugin folder, probes its toolchain, scaffolds a
playground of every kind into a temporary library, runs each one, and prints a
report. Exit code 0 only if everything passed.

```
$ tinkerpad plugin check plugins/go
go (Go)
  ✓ manifest            loads, 0 warnings
  ✓ toolchain           go 1.24.2 at /usr/local/go/bin/go
  ✓ templates           2 kinds, 1 scaffold file
  ✓ run: statements     exit 0 in 312 ms, 14 bytes of stdout
  ✓ run: program        exit 0 in 298 ms, 9 bytes of stdout
4 passed, 0 failed, 0 skipped
```

## Why

This is what makes community plugins trustworthy without a review team. It also
has to exist before the checks that need a `check/` folder convention, because
those need somewhere to be reported.

## Expectations

- [x] Shipped as `tinkerpad plugin check <dir>`, a subcommand of the main
      binary rather than a second one. Decide it here and note it in 00-01's
      gotcha. (It must not open a window: a check is a terminal thing.)
- [x] Each check reported pass/fail/skip with a one-line reason:
      1. **Manifest loads** — 01-01's errors and warnings surfaced verbatim,
         including the "did you mean" suggestions.
      2. **Toolchain probe succeeds** — on failure, every later check is
         skipped with the plugin's own `install_hint`, and the exit code says
         skipped rather than failed.
      3. **Templates and scaffold** — every `[[kind]].template` renders and
         every `[scaffold].files` entry is written, into a temporary library
         that is removed afterwards.
      4. **Each kind runs** — a fresh playground of every kind exits 0 and
         produces some stdout within 60 s.
- [x] `--json` for CI and `--only <check>` to iterate on one.
- [x] The same code paths as the app: the same loader, the same scaffolder, the
      same runner. Not a reimplementation, or it will pass plugins the app
      rejects.
- [x] A test that runs the checker over a fixture plugin that passes and one
      that fails each check, asserting the exit code and the report.

## Results

Measured on Windows 11, 2026-09-30.

**The decision 00-01 asked for: a subcommand of the same binary.** `tinkerpad
plugin check` runs in the binary users already have — nothing extra to install,
nothing extra to sign (04-06), and no chance of a CLI built from different code
than the app it is checking, which is the whole point. Noted in 00-01's gotcha.
The cost is that the binary is a GUI-subsystem program on Windows, so its
output goes through the same console handling `--version` uses; that already
existed.

**Every bundled plugin, checked:**

| Plugin | Result |
|---|---|
| C# | 6 passed — 3 kinds, runs in 3.2–3.9 s each |
| Go | 1 passed, 3 skipped — `go` isn't on this machine |
| PowerShell | 4 passed — one kind, 430 ms |
| Python | 5 passed — 72 and 71 ms |
| Rust | 5 passed — `program` 5.2 s cold, `expression` 480 ms |
| TypeScript | 5 passed — 220 and 72 ms |

Go skipping rather than failing is the point of that distinction: this machine
not having Go says nothing about the Go plugin.

**A bug the checker found in itself, immediately.** The first run of
`plugin check plugins/python` failed both kinds with
`ModuleNotFoundError: No module named 'tinkerpad'`. The cause was the checker's
own: it passed the folder through as typed, so `{plugin_dir}` stayed relative,
and `PYTHONPATH = "{plugin_dir}/dump"` resolved against the playground folder
rather than the repository. The app never has this problem because it loads
plugins from absolute paths. Fixed by making the folder absolute before
loading — which is exactly the class of bug the "same code paths as the app"
rule exists to prevent, caught by the first thing it was pointed at.

**A second one, found by the tests.** The temporary workspace was named
`tinkerpad-check-<plugin>-<pid>`, so two checks of the same plugin in one
process — a test suite, or a CI matrix running in parallel — would share a
folder and delete each other's work. Now unique per check.

**`--only` skips the work, not just the reporting.** `--only manifest` on a
plugin whose run sleeps two seconds returns in under 1.5 s, asserted by a test;
a flag for iterating on one check is worthless if it still waits for the
others.

**Tests** — `crates/tinkerpad-app/tests/check.rs`, nine of them, using
`tinkerpad-faketool` as the fixture plugin's entire toolchain so they are
deterministic and need no language installed:

| Test | What it pins down |
|---|---|
| a plugin that works passes every check | the happy path, and the summary line |
| a manifest that doesn't load fails and says why verbatim | the loader's own words and its "did you mean" |
| a toolchain that isn't here skips rather than fails | exit 0, and the plugin's `install_hint` in the reason |
| a run that fails is reported with what it said | the exit code and the last line of stderr |
| a run that prints nothing is a failure even at exit 0 | an empty output pane is not a working plugin |
| a template that isn't there fails before anything runs | — |
| `--only` runs one check and doesn't do the others' work | the timing above |
| `--only` can name one kind's run | `run:statements` |
| nothing is left behind | the temporary library is gone afterwards |

`tinkerpad-faketool`, added in 04-08a as a stand-in package manager, gained
`stderr` and `sleep` steps for these, matching `tinkerpad-fakeprobe`'s
vocabulary.

**Documented** in PLUGINS.md §8.1: what each check means, that a missing
toolchain is a skip, the flags, and that nothing touches the user's library.

## Out of scope

Everything needing a `check/` folder (dump, errors, helper, server): 05-01b.
`tinkerpad tools`: 05-01c. Performance assertions — report timings, don't fail
on them.

## Gotchas

The binary is a GUI subsystem program on Windows (04-06), so its output needs
the same console handling `--version` has.

A check scaffolds into a temporary library, never the user's.
