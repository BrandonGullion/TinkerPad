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

- [x] `plugin.toml` generated from `Manifest::example()` (01-01) so it never
      drifts from the schema; a CI job in the main repo diffs the two.
      **Done differently, and better — see Results.** The example manifest is
      the C# one from PLUGINS.md §2, and a template that *is* the
      documentation example is a bad template. The drift check instead loads
      the template with the real loader on all three operating systems and
      asserts it mentions every section the example uses. It caught a missing
      `[deps]` the first time it ran.
- [x] README walks through PLUGINS.md §10's checklist step by step with the
      commands to run after each.
- [x] `helper/` is the Python reference helper verbatim, with a note "delete this
      folder if your language doesn't need a helper".
- [x] The workflow runs `check` on push and PR. **The badge is deferred**:
      TinkerPad has no public releases and the repository is private, so a
      badge would point at nothing. The workflow builds the checker from
      source and carries the release-download form commented out beside it,
      ready for the first release.
- [x] Creating a PowerShell-like plugin from the template takes under 30 minutes
      end to end; time it with a fresh clone and record here.

## Results

Built 2026-10-03. The template lives at `plugin-template/` in this repository
rather than a repository of its own — decided with the maintainer, who will
push it to a template repo when the project goes public. Everything in it is
written to work either way: no absolute URLs, no badge pointing at a run that
doesn't exist.

**It passes before anything is changed**, which is the whole point:

```
$ tinkerpad plugin check plugin-template
  ok   manifest           loads, no warnings
  ok   toolchain          py 3.13.1 at …
  ok   templates          2 kinds, 0 scaffold files
  ok   run:statements     exit 0 in 80 ms
  ok   run:expression     exit 0 in 61 ms
  ok   dump               a table of 2 rows, columns name, n
  ok   errors             line 3: ZeroDivisionError: division by zero
  ok   helper:start       built and ready in 96 ms
  ok   helper:run         ran warm in 12 ms, the table arrived
  ok   helper:cancel      stopped 45 ms after cancel
  ok   helper:shutdown    stopped in 8 ms
11 passed, 0 failed, 0 skipped
```

Out of the box it runs playgrounds with Python, so a contributor sees what
passing looks like before they start and every failure after that is theirs.
Four places in the manifest are marked `CHANGE ME`, numbered, and the README's
checklist is PLUGINS.md §10 with the command to run after each step.

**The timed exercise.** Converting the template into a PowerShell plugin took
**1 minute 49 seconds** of wall clock, against the item's budget of 30
minutes. That number is a floor rather than a contributor's experience: it was
done by someone who knows the schema, scripting the edits, with the bundled
PowerShell plugin open as a reference. What it does show is that the template
is a working starting point and that `plugin check` leads you through — it
caught three distinct mistakes, in order, each with a message that said what
to do:

| Attempt | What the check said | The fix |
|---|---|---|
| 1 | `plugin.toml:121:29: in [output]: invalid literal string` | A regex mangled by my own editing |
| 2 | `exit 64: -outputformat` | Python's `-u` left in `[run]`; pwsh read it as an abbreviation of `-outputformat` |
| 3 | `failed because the file does not have a '.ps1' extension` | pwsh's `-File` insists on `.ps1` |

The second is the interesting one: a wrong argument left over from the
template produced an error from PowerShell that means nothing on its own, and
the check put it next to the plugin it came from.

**The drift check** is `crates/tinkerpad-plugin/tests/template.rs`, five tests:
the template loads with no warnings on all three operating systems; it has
every file its README promises; it mentions every section the documented
example uses; its helper is the Python one byte for byte; and the four marked
places are there and numbered. CI also runs `plugin check` over
`plugin-template/` alongside the bundled plugins.

**What is in the template**

| | |
|---|---|
| `plugin.toml` | 124 lines, every section, commented, four marked edits |
| `templates/` | One file per kind, including a locked-lines kind |
| `dump/` | The Python rich-output library as a stub to rewrite |
| `helper/` | `plugins/python/helper/helper.py` verbatim, with "delete this folder if your language doesn't need one" |
| `check/` | `dump`, `error` and `hang`, the 05-01b convention |
| `.github/workflows/check.yml` | Runs `plugin check` on push and PR, and asserts it checked something rather than skipping everything |

That last point is worth keeping: `plugin check` exits 0 when a toolchain
simply isn't on the runner, so a green tick could mean "skipped everything".
The workflow reads the `--json` report and fails if the manifest and toolchain
didn't actually pass, or if nothing ran.

**To publish it later**, when the project goes public:

```sh
gh repo create <owner>/tinkerpad-plugin-template --public \
    --description "Start a TinkerPad language plugin"
cd plugin-template && git init && git add -A
git commit -m "A TinkerPad language plugin template"
git remote add origin git@github.com:<owner>/tinkerpad-plugin-template
git push -u origin main
gh repo edit --template
```

Then add the badge to its README and uncomment the release-download step in
its workflow.

## Out of scope

A cookiecutter/CLI generator (the template repo is enough).

## Gotchas

Keep the template on the *previous* protocol version's floor when the core moves
forward, so contributors' plugins keep working during the overlap window (05-04).
