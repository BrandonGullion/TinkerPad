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

- [ ] A documented `check/` folder convention: `check/dump.<ext>`,
      `check/error.<ext>`, `check/hang.<ext>`. A plugin without one skips the
      checks that need it, with a message saying which file to add.
- [ ] **Dump** — `check/dump.<ext>` dumps a known table; the checker asserts a
      `table` node arrives with the expected columns and row count, through the
      same parser the app uses (01-07).
- [ ] **Errors** — `check/error.<ext>` has a compile or runtime error; at least
      one annotation with a line number is produced by `[output].error_regex`.
- [ ] **Helper**, if `[helper]` is declared: `build` succeeds if present;
      `hello`/`ready` within 10 s; a `run` works; `cancel` on
      `check/hang.<ext>` completes within 3 s; every declared hook answers
      within 5 s with a well-formed `result`; `shutdown` exits cleanly.
- [ ] **Language server**, if `[lsp]` is declared: it spawns, `initialize`
      succeeds, and a `didOpen` of the dump playground produces zero error
      diagnostics within 30 s.
- [ ] Every bundled plugin passes, and gets whatever `check/` files it needs.
- [ ] CI runs `plugin check` over every bundled plugin, on the platforms whose
      toolchains the workflow installs.
- [ ] The convention documented in PLUGINS.md §8, so 05-03's template repo can
      ship it.

## Out of scope

The tool itself and checks 1–4 (05-01a). `tinkerpad tools` (05-01c).

## Gotchas

A hook check has to use the real helper pool, not a hand-rolled client, or it
proves nothing about what the app will do.

`cancel` is the check most likely to hang the checker itself: give every wait a
deadline, and fail the check rather than the process.
