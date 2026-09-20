# 03-07 · Hooks

**Phase:** 3 · **Size:** M · **Crate/area:** `tinkerpad-library`, `tinkerpad-lsp`, `tinkerpad-plugin` · **Depends on:** 03-02, 01-04, 02-07
**Design:** PLUGINS.md §5 (Hooks)

## What

Wire the four hook messages into the places they belong: `playground_created`
after scaffold, `playground_opened`/`playground_closed` on LSP attach/detach,
`add_dependency` from the add-package command. Apply returned `files` and
`lsp_config`.

## Why

Hooks are the escape hatch that keeps language-specific logic out of the Rust
core. Go's `go.work` rewriting is the canonical need; having the mechanism proven
with the fake helper means the Go plugin (04-07) is a manifest plus a tiny script.

## Expectations

- [x] `playground_created`: called after `[scaffold]` files are written; returned
      `files` are written (may overwrite scaffold output, since the hook is more
      specific).
- [x] `playground_opened`/`closed`: called with the full list of currently open
      playgrounds of that language; returned `files` written; returned
      `lsp_config` merged into what `[lsp.register]` would send and delivered via
      `workspace/didChangeConfiguration`.
- [x] `add_dependency`: if the plugin's helper lists it, it takes precedence over
      `[deps]`; returned `files` written.
- [x] All hooks are best-effort: timeout or helper failure logs a warning and the
      operation continues as if the hook returned nothing. The user sees a toast
      only for `add_dependency` failures.
- [x] File writes from hooks are constrained to the playground folder or the
      plugin's scratch dir; anything else is rejected and logged.
- [x] Tests with the fake helper: each hook fires at the right moment with the
      right payload; returned files land; a path outside the sandbox is refused.

## Out of scope

New hook types. Add them when a real plugin needs one, with a protocol version
note.

## Gotchas

Starting a helper just to answer `playground_created` for a language that only
uses hooks (no `run`) is fine; idle shutdown will reap it.

## Results

Implemented 2026-09-20.

**`crates/tinkerpad-app/src/hooks.rs`** is the whole wiring: the four hooks, the
sandbox around what they write, and the settings they push. Everything in it is
best-effort — a hook that is absent, slow, broken, or answering nonsense is
logged and the operation carries on as if it had answered with nothing.

**When each one fires.**

| Hook | Where from |
|---|---|
| `playground_created` | `create_playground`, after `[scaffold]` has written the files and before the command returns, so the UI opens a playground the hook has already finished with |
| `playground_opened` | `open_playground`, in a background task; the app keeps the set of open playgrounds, so re-reading an open one is not a second opening |
| `playground_closed` | a new `close_playground` command, called from the UI's tab close |
| `add_dependency` | `packages::add`, before `[deps]` is looked at |

**The open list** is every open playground *of that language*, the subject
included — the list a `go.work` would need, not every tab in the app.

**Precedence.** A plugin that declares `add_dependency` never reaches `[deps]`:
the hook is asked, its files are written, and the UI is told what changed
(`AddPackage::Handled`) rather than being handed a directive to insert. The one
place a hook is allowed to interrupt the user is here — a helper that doesn't
answer gets a toast, because the user asked for something specific.

**The sandbox.** A hook's `path` is relative to the playground folder and stays
in it, through the library's own `safe_relative`: `..`, an absolute path, a
drive, a UNC share and an empty path are all refused and logged, and the rest of
the files in the same answer still land. A helper that wants to write elsewhere
has its own scratch folder and needs no permission from the core for it.

**`lsp_config`** is merged into what `[lsp.register]` sends for that server
rather than replacing it (`tinkerpad-lsp`'s new `hooked` map plus a deep
`merge`), keyed by document URI, so two playgrounds sharing a server both
contribute and the later one wins only where they actually overlap. Closing a
playground forgets its entry and pushes the rest. Because the tab attaches its
server while the hook is being asked, `configure` waits up to 10 s in its
background task for a server that is on its way rather than losing the settings
to that race.

**Tests.** Unit tests for the merge (adds to the manifest, wins on overlap,
replaces anything that isn't two objects) and for the sandbox (six paths that
must be refused). The protocol side was already covered by 03-01's fake-helper
tests. End to end, a new `hooks` self-test scenario against the node-check test
plugin, whose helper now answers all four hooks by appending a line to
`hooks.log` in the playground folder — so what arrived, and when, is readable
from the file the core wrote on its behalf. Eleven checks, all passing:

```
PASS playground_created fires as the playground is made, and its file lands in the folder
PASS playground_opened fires when a tab opens it, and says which are open
PASS the open list has every open playground of that language, this one included
PASS a hook's lsp_config is pushed to the language server
PASS a playground that is already open does not fire the hook again
PASS playground_closed fires when the tab closes, and the closed one is no longer open
PASS closing a playground drops what its hook told the server
PASS add_dependency goes to the hook, which gets the name and version
PASS the hook takes precedence over [deps]: no directive is inserted
PASS a hook's file outside the playground folder is refused, and its other files still land
```

The server checks read the test server's own log: it now prints the settings of
every `workspace/didChangeConfiguration` it receives, and the scenario sees
`{"tinkerpad":{"open":2,"opened":"Hooks Second"}}` while both are open and
`{"open":1,"opened":"Hooks First"}` after one closes.

## Checklist status

**Deviation: files land in the playground folder only, not the scratch dir.**
The item allowed either. A helper's scratch folder is the helper's own process's
to write, and routing it through the core would mean a second path shape with no
caller. `docs/PLUGINS.md` §5 says so.

**Note: `playground_opened`/`closed` fire on tab open and close, not on LSP
attach and detach** as the item's What section put it. A plugin with no language
server still wants to be told (that is most of them), and the settings a hook
sends survive the server not being up yet, which is what tying it to attach was
for.
