# 05-03 · Plugin template repo

**Phase:** 5 · **Size:** S · **Crate/area:** separate repo `tinkerpad-plugin-template` · **Depends on:** 05-01b, 03-04
**Design:** PLAN.md §5 Phase 5

## What

A GitHub template repository a contributor clicks "Use this template" on: a
commented `plugin.toml` with every section, placeholder templates, a `dump/`
stub, the Python reference helper as `helper/` with comments, `check/`
playgrounds, and a CI workflow that runs `tinkerpad plugin check`.

## Why

The difference between "you could write a plugin" and "people write plugins" is a
template that passes `check` on the first push.

## Expectations

- [ ] `plugin.toml` generated from `Manifest::example()` (01-01) so it never
      drifts from the schema; a CI job in the main repo diffs the two.
- [ ] README walks through PLUGINS.md §10's checklist step by step with the
      commands to run after each.
- [ ] `helper/` is the Python reference helper verbatim, with a note "delete this
      folder if your language doesn't need a helper".
- [ ] The workflow installs the latest TinkerPad release's CLI and runs `check`
      on push and PR; the badge is in the README.
- [ ] Creating a PowerShell-like plugin from the template takes under 30 minutes
      end to end; time it with a fresh clone and record here.

## Out of scope

A cookiecutter/CLI generator (the template repo is enough).

## Gotchas

Keep the template on the *previous* protocol version's floor when the core moves
forward, so contributors' plugins keep working during the overlap window (05-04).
