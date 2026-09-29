# 04-07 · Go and PowerShell plugins

**Phase:** 4 · **Size:** M · **Crate/area:** `plugins/go/`, `plugins/powershell/` · **Depends on:** 03-07, 02-07
**Design:** PLUGINS.md §7 (Go, PowerShell)

## What

Two more bundled plugins that stress opposite ends of the plugin format:
PowerShell needs almost nothing; Go needs `[scaffold]`, a shared-workspace LSP, and
a hook that rewrites `go.work`.

## Why

Five bundled plugins is enough evidence that the format is general before inviting
contributors. These two are the cheapest way to cover the remaining corners.

## Expectations

**PowerShell**
- [x] `[language]`, `[toolchain]` (`pwsh --version`, fall back to
      `powershell` on Windows), one `statements` kind, `[run] = pwsh -NoProfile
      -NonInteractive -File {main}`. No LSP, no deps, no dump library beyond a
      10-line `Dump` function in the template's prelude (locked) that emits the
      OSC line via `ConvertTo-Json`.
- [x] Proves that a manifest with only the required sections loads and runs.
- [x] On macOS and Linux `pwsh` is optional; when absent the plugin shows its
      install hint and the app is otherwise unaffected. Verified in CI where
      `pwsh` is present on all runners, and by hand with it removed.

**Go**
- [x] Go installed first (note version). `[scaffold]` writes `go.mod` with the
      playground slug as module name; `[run] = go run .`; `[output].error_regex`
      for `./main.go:5:2: message`; `[deps]` = command `go get {name}@{version}`.
- [x] `dump/` as a small Go module (`tinkerpad.Dump(v any)`), referenced by a
      `replace` directive in the scaffolded `go.mod` pointing at the plugin folder.
- [x] `[lsp]` = `gopls`, `workspace = "scratch_workspace"`, and a helper (in Go
      or a short script) implementing `playground_opened`/`playground_closed`
      that rewrites `<scratch>/go.work` to `use` every open playground dir.
      Completions on `strings.` work in two open Go playgrounds.
- [x] Go's build cache means second runs are fast without a helper `run`; record
      timings.

## Out of scope

Go helper-based warm runs (not needed).

## Gotchas

gopls needs the `go.work` to exist at startup; the helper's `playground_opened`
must write it before the server is spawned. Order the calls: hook first, then
attach.

## Results

Implemented 2026-09-29, against **Go 1.27.0**, **gopls v0.23.0** and
**PowerShell 7.6.6**.

Two plugins at opposite ends of the format, which is what the item was for.

### PowerShell: the smallest a plugin gets

`[language]`, `[toolchain]`, one `[[kind]]`, `[run]`, and an `[output]`
pattern. No `[lsp]`, no `[deps]`, no `[helper]`, no `dump/` folder — and it
loads and runs, which is the thing this half was meant to prove.

Rich output comes from the kind's locked prelude: about twenty lines of
PowerShell that write the §6 sentinel. It takes the pipeline, so
`$things | Dump` is one table rather than one dump per object — the first
version got that wrong, and the fix is `begin`/`process`/`end` with the rows
gathered before anything is written.

The prelude sets two other things, both of which turned out to matter:

- `$ErrorView = 'NormalView'`. PowerShell 7's default error display puts the
  message last, inside line art; the classic view puts the message first and
  the position second, which is what `[output].error_regex` can read. Without
  it there is no clickable error.
- `$ErrorActionPreference = 'Stop'`. A PowerShell script whose command fails
  keeps going and exits 0 — so a broken playground looked like a working one.
  Now the run stops and exits 1, which is what a scratchpad should do.

On Windows the toolchain falls back to the built-in `powershell` 5.1, so the
plugin works on a stock machine; elsewhere `pwsh` is optional and its absence
costs the language and nothing else.

### Go: everything the format has

A module per playground (`[scaffold]` writes `go.mod`, named after the folder,
with `replace tinkerpad => <plugin>/dump`), `go run .`, `go get {name}@{version}`
for dependencies, gopls over a shared workspace, and a helper that answers
`playground_opened`/`playground_closed` by rewriting `<scratch>/go.work` from
the list of open playgrounds it is handed — plus the `dump/` module, so every
`replace` resolves. It writes the file whole and renames it into place, because
gopls watches it.

The dump library is a small Go module with one exported function: a slice of
structs or string-keyed maps becomes a table, scalars become values with their
type, everything else a tree. It imports only the standard library.

**Measured** (this machine, warm module cache):

| | Go |
|---|---|
| First run of a playground | 298–345 ms |
| Every run after | 126–174 ms |

That is why the plugin has no warm runner: Go's build cache already does the
job, and a helper would add a process to keep alive for a saving of about a
tenth of a second.

### A bug this found in the core

`[lsp]` is documented (§3, the placeholder table) as accepting `{scratch}`, and
it didn't: the substitution list had `dir`, `main`, `plugin_dir`, `root` and
`name`. The Go plugin needs it — `GOWORK = "{scratch}/go.work"` is how gopls
finds the workspace the helper writes — and the failure was silent: the config
errored, no server started, and nothing was said. Both are fixed. The
placeholder is now supported, and a language server that can't even be
configured says so in a toast once, rather than leaving a language quietly
without completions.

### Tests

Two self-test scenarios, sixteen checks, all passing.

```
PASS the Go plugin is bundled, with a server and a helper
PASS a new playground is scaffolded as a module with the dump library replaced in
PASS it builds and runs, and a slice of structs arrives as a table
PASS runs are quick enough that no warm runner is needed
PASS a compile error fails the run and is clickable in the output
PASS Add package runs `go get`, and the module file says so
PASS the helper's go.work names every open playground and the dump module
PASS gopls answers completions on `strings.` in a second open playground
PASS closing a playground takes it out of go.work

PASS the PowerShell plugin is bundled, with no server and no helper
PASS a new playground starts from the template, under the locked Dump
PASS it runs, and the pipeline's objects arrive as one table
PASS a scalar dumps as a value with its type
PASS an error stops the script and fails the run, with a position to point at
PASS and the error line is clickable in the output
```

## Checklist status

**Note: `gopls` is not on `PATH` after `go install`.** It lands in
`$(go env GOPATH)/bin`, which a fresh machine doesn't search. The plugin's
README says so and gives the `settings.toml` override; the language server's
absence is reported like any other missing program.

**Note: the PowerShell plugin was verified with `pwsh` present.** The
"missing toolchain" path is covered by the scenario's other branch (it checks
the install hint when `pwsh` isn't there) but was exercised here by the `mini`
fixture plugin rather than by uninstalling PowerShell.
