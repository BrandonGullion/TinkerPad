# 01-14 · Dependencies: add package

**Phase:** 1 · **Size:** S · **Crate/area:** `tinkerpad-plugin`, `tinkerpad-app`, `ui/` · **Depends on:** 01-01, 01-06, 01-12
**Design:** PLUGINS.md §2 (`[deps]`)

## What

A minimal "add package" command that does what the plugin's `[deps]` says: either
runs a command (`cargo add`, `npm install`) in the playground folder, or inserts an
inline directive (`#:package Foo@1.0`) at the top of the main file.

## Why

A scratchpad without packages is a toy. The full UI can wait (04-01), but the
mechanism should exist so the manifest field is proven with three real plugins.

## Expectations

- [x] `[deps] kind = "command"` with `add = [...]` (placeholders `{name}`,
      `{version}`; `{version}` may be empty → strip a trailing `@`).
- [x] `[deps] kind = "inline-directive"` with `directive = "..."` inserted after
      any existing directives at the top of the file (after locked lines if the
      prelude is first), one per line, no duplicates by name.
- [x] Command palette / `Mod+Shift+P` → "Add package…" prompts for `name` and
      optional `version`. Command output streams into the output pane like a run.
- [x] Failure (network, unknown package) shows the tool's stderr; nothing is
      half-applied for the inline-directive kind.
- [x] After adding, the next run picks the package up with no extra step in all
      three bundled plugins. Verify with `serde`, `zod`, and `Newtonsoft.Json`.

## Out of scope

Package search/autocomplete, version pickers, removal UI, lockfile handling.

## Gotchas

`npm install` needs a `package.json`; the TS plugin's `[scaffold]` should write a
minimal one (`{"type":"module","private":true}`) so this works on a fresh playground.

## Results

Implemented 2026-09-16.

**Runner**
- `spec.rs` gained two pure functions beside `run_spec`, so a `[deps]` action is
  plain data before anything runs:
  - `deps_command(plugin, playground, ctx, package) -> Option<RunSpec>`: the
    command to run, in the playground folder, with the same `PATH` a run gets.
    `None` when the plugin uses a directive.
  - `deps_directive(plugin, package) -> Option<(line, prefix)>`: the line to
    insert and the text before `{name}` in it, which identifies an existing line
    for the same package. `None` when the plugin uses a command.
  - `{name}` and `{version}` are filled in both; with no version a trailing `@`
    is dropped (`serde@` → `serde`), so "latest" needs no special case in a
    manifest.
  - A plugin with no `[deps]` returns `SpecError::NoDeps`, which reaches the user
    as "Node check doesn't say how to add packages (no [deps] section)".

**App**
- `packages.rs`: the `add_package` command. It rejects names and versions that
  would change the command line (whitespace, quotes, a leading dash), then
  returns either `{"kind":"command","run":<id>,"command":"npm install zod"}` or
  `{"kind":"directive","line":"#:package Foo@1.0","prefix":"#:package "}`.
  Nothing is written for the directive kind; the UI applies it, so the editor,
  the file, and undo stay in step.
- `runs.rs`: `start()` split into a `spawn()` shared with the new
  `start_spec()`, which runs an arbitrary spec as the playground's current run.
  A package command therefore streams, cancels, and reports exactly like a run.

**UI**
- `picker.ts`: `prompt(title, fields)` beside `pick`, for short typed answers.
  Enter moves to the next required field and submits on the last; Escape
  cancels.
- `commands.ts`: the command palette (`Mod+Shift+P`), listing "Add package…"
  and the ten actions that already had shortcuts, each showing its key.
- `tabs.ts`: `addPackage()` asks for a name and an optional version, clears the
  output pane, and hands the command to `RunController.adopt()`, which routes
  its events to the tab's pane and status the way a run's are. A directive is
  inserted after any directives already at the top, replacing a line for the
  same package, and saved immediately so the next run compiles with it.

**Plugins**
- TypeScript: `[deps] kind = "command"`, `add = ["npm", "install",
  "{name}@{version}"]`, installing into the playground's own `node_modules`
  next to the scaffolded `package.json`.
- Rust and C# already carried their `[deps]` sections from 01-11 and 01-10
  (`cargo add {name}@{version}`, `#:package {name}@{version}`).

**Measurements** (Windows 11, release build, warm npm and cargo caches)

| Plugin | Package | Adding | First run using it |
|---|---|---|---|
| TypeScript | `zod` (no version) | 1.7 s (`npm install zod`) | 0.4 s |
| Rust | `serde@1` | 0.2 s (`cargo add serde@1`) | 0.3 s |
| C# | `Newtonsoft.Json@13.0.3` | directive, instant | 2.7 s (first restore + build) |

Each run then used the package: `z.string().parse("ok")`, a generic bounded by
`serde::Serialize`, and `JsonConvert.SerializeObject`, all exiting 0 with the
expected output.

Adding a package that does not exist (`npm install tinkerpad-no-such-package-9a7f2`)
exits 1, with npm's own "404 Not Found" text in the output pane and the
exit code in the status bar, exactly as a failing run shows.

**Tests**
- `crates/tinkerpad-runner/tests/spec.rs`: four tests over `[deps]` — the
  command form (placeholders, empty version, working directory), the directive
  form (line and prefix), a plugin with no `[deps]`, and one that walks all
  three bundled manifests and checks what each would do.
- `packages.rs` unit test for the name and version check.
- In-app `packages` scenario (10 checks): the palette lists "Add package…";
  with nothing open it says so; the directive lands on the first line; the file
  is saved at once; a second package goes after the first; the same package
  again replaces its line; a refused name changes nothing; a kind with locked
  lines refuses; a plugin without `[deps]` reports it. It runs on Windows
  locally and on macOS and Linux in CI.
- In-app `packages-live` scenario: the three real installs above and the
  unknown-package failure. It needs the network, so it isn't in CI; the
  measurements come from it.

**Docs**
- PLUGINS.md §2.1 now describes `[deps]` in detail: the placeholders, the
  trailing `@`, where a directive is inserted, and why a kind with a prelude
  can't take one.

## Checklist status

**Deviation: directives and locked kinds.** The expectation said to insert a
directive "after locked lines if the prelude is first". That would put
`#:package …` inside C#'s `expression` wrapper, which doesn't compile, and a
directive above the prelude would break lock detection — the file has to *start*
with the prelude for the locked ranges to be found at all. So a kind with a
prelude refuses, with a toast pointing at a kind without locked lines, and
nothing is written. Every language that uses directives ships such a kind (C#
has `statements` and `program`).
