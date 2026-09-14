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
- [ ] `[language]`, `[toolchain]` (`pwsh --version`, fall back to
      `powershell` on Windows), one `statements` kind, `[run] = pwsh -NoProfile
      -NonInteractive -File {main}`. No LSP, no deps, no dump library beyond a
      10-line `Dump` function in the template's prelude (locked) that emits the
      OSC line via `ConvertTo-Json`.
- [ ] Proves that a manifest with only the required sections loads and runs.
- [ ] On macOS and Linux `pwsh` is optional; when absent the plugin shows its
      install hint and the app is otherwise unaffected. Verified in CI where
      `pwsh` is present on all runners, and by hand with it removed.

**Go**
- [ ] Go installed first (note version). `[scaffold]` writes `go.mod` with the
      playground slug as module name; `[run] = go run .`; `[output].error_regex`
      for `./main.go:5:2: message`; `[deps]` = command `go get {name}@{version}`.
- [ ] `dump/` as a small Go module (`tinkerpad.Dump(v any)`), referenced by a
      `replace` directive in the scaffolded `go.mod` pointing at the plugin folder.
- [ ] `[lsp]` = `gopls`, `workspace = "scratch_workspace"`, and a helper (in Go
      or a short script) implementing `playground_opened`/`playground_closed`
      that rewrites `<scratch>/go.work` to `use` every open playground dir.
      Completions on `strings.` work in two open Go playgrounds.
- [ ] Go's build cache means second runs are fast without a helper `run`; record
      timings.

## Out of scope

Go helper-based warm runs (not needed).

## Gotchas

gopls needs the `go.work` to exist at startup; the helper's `playground_opened`
must write it before the server is spawned. Order the calls: hook first, then
attach.
