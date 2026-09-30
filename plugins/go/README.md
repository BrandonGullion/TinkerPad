# The Go plugin

Go has a real project model, and this plugin is what that costs: a module per
playground, one language server across all of them, and a helper that keeps
`go.work` telling the truth. It is the opposite end of the plugin format from
the PowerShell plugin next door, which needs none of that.

## A module per playground

`[scaffold]` writes a `go.mod` when a playground is created:

```go.mod
module go-self-test

go 1.21

require tinkerpad v0.0.0

replace tinkerpad => C:/.../plugins/go/dump
```

The module is named after the playground's folder, and the `replace` points at
this plugin's `dump/` module, so `tinkerpad.Dump` resolves with nothing to
install. The absolute path is written at creation and recorded in
`playground.toml` under `[plugin.go]`, so it can be repaired if TinkerPad moves.

`go run .` runs it. `go get name@version` adds a dependency, through the
command palette's **Add package**.

## No warm runner, on purpose

Go's build cache makes a second run quick without one: measured on this
machine, about 330 ms for the first run of a playground and about 130 ms for
each one after. A helper that kept a process warm would save a fraction of that
and cost the complexity of the C# one, so the plugin doesn't have one for runs.

## The helper, and `go.work`

`gopls` will happily serve several modules at once, but only if a `go.work`
names them, and it reads that file when it starts. So the helper answers the
two hooks that say which playgrounds are open (docs/PLUGINS.md §5):

- `playground_opened` and `playground_closed` arrive with every open Go
  playground, so the helper rewrites `<scratch>/go.work` from the list it was
  given rather than remembering anything between messages.
- The file also lists this plugin's `dump/` module, so every playground's
  `replace` resolves inside the workspace.
- `[lsp].env` sets `GOWORK` to that file, which is why the server finds it
  wherever it is rooted.

It is written whole and renamed into place: gopls watches the file, and half a
workspace is worse than a stale one.

The helper runs `go run helper/main.go` — a Go program to keep the plugin in one
language, and small enough to read in a sitting.

## What you need

Go 1.21 or newer on `PATH`, and `gopls` for completions and diagnostics:

```sh
go install golang.org/x/tools/gopls@latest
```

The **Languages** button in the status bar offers to run exactly that for you,
with `GOBIN` pointed at TinkerPad's own folder so the result is found without
touching your `PATH` (work item 04-08b). Go itself is a hint rather than an
offer: installing an SDK is not something an editor should be doing behind a
button.

By hand, `go install` puts it in `$(go env GOPATH)/bin`, which is not on `PATH`
on a fresh machine. Add it, or point TinkerPad at the binary in `settings.toml`:

```toml
[toolchains.go]
gopls = "C:/Users/you/go/bin/gopls.exe"
```
