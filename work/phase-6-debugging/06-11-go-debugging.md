# 06-11 · Debugging Go

**Phase:** 6 · **Size:** S · **Crate/area:** `plugins/go` · **Depends on:** 06-10
**Design:** DEBUGGING.md §1

## What

`dlv dap`, launching a Go playground.

## Why

Delve speaks DAP natively and is the only Go debugger anyone uses, so this is
the shortest of the language items — and it follows Rust because it is the
second launch-mode one and can take whatever 06-10 decided about artifacts.

## Expectations

- [ ] A session launches the playground and stops on a breakpoint in `main.go`.
- [ ] Variables are legible: a slice, a map, a struct, an interface holding
      something. A goroutine's stack is reachable from the Call Stack panel.
- [ ] Go's module-per-playground arrangement (04-07) is respected: the session
      runs in the playground folder with the same `go.work` a run gets, or the
      debugger will not find the `tinkerpad` module.
- [ ] `dlv` is acquired as `gopls` is — `go install github.com/go-delve/delve/cmd/dlv@latest`
      with `GOBIN` pointed at TinkerPad's own folder (04-08b), so it lands
      somewhere the resolver looks without touching the user's `PATH`.
- [ ] Both kinds debug: `program` and the locked-lines `expression`.

## Out of scope

Attaching to a running Go process. Core dumps.

## Gotchas

Delve builds with its own flags; a playground that runs with `go run` and
debugs with `dlv` are two builds of the same source. They should agree about
the module, and 04-07's helper rewrites `go.work` as tabs open — a session
started while that file is being rewritten will fail confusingly.

`dlv dap` and `dlv --headless` are different servers with different protocols.
It is the first.
