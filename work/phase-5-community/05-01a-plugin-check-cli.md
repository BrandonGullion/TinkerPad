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

- [ ] Shipped as `tinkerpad plugin check <dir>`, a subcommand of the main
      binary rather than a second one. Decide it here and note it in 00-01's
      gotcha. (It must not open a window: a check is a terminal thing.)
- [ ] Each check reported pass/fail/skip with a one-line reason:
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
- [ ] `--json` for CI and `--only <check>` to iterate on one.
- [ ] The same code paths as the app: the same loader, the same scaffolder, the
      same runner. Not a reimplementation, or it will pass plugins the app
      rejects.
- [ ] A test that runs the checker over a fixture plugin that passes and one
      that fails each check, asserting the exit code and the report.

## Out of scope

Everything needing a `check/` folder (dump, errors, helper, server): 05-01b.
`tinkerpad tools`: 05-01c. Performance assertions — report timings, don't fail
on them.

## Gotchas

The binary is a GUI subsystem program on Windows (04-06), so its output needs
the same console handling `--version` has.

A check scaffolds into a temporary library, never the user's.
