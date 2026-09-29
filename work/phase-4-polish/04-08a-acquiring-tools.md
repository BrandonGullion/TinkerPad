# 04-08a · Acquiring a language's tools

**Phase:** 4 · **Size:** M · **Crate/area:** `tinkerpad-plugin`, `tinkerpad-process`, `tinkerpad-app` · **Depends on:** 01-03, 02-08
**Design:** PLUGINS.md §2 (`[toolchain]`), ARCHITECTURE.md §4.1

> Split from 04-08 (see `04-08b-tool-offers.md` for the offers and the UI). This
> half is the machinery: what a plugin may declare, and what the core does when
> asked. Nothing here is user-facing on its own.

## What

A plugin says how to **get** what it needs, and the core can do it on request.
Two kinds, because two kinds cover everything worth covering:

```toml
[toolchain.acquire]
# 1. Ask the package manager the user already has.
kind = "command"
command = ["rustup", "component", "add", "rust-analyzer"]
needs = "rustup"                      # what must exist for this to work
label = "rustup component add rust-analyzer"

[lsp.acquire]
# 2. Or fetch a release, when there is no package manager to ask.
kind = "download"
version = "0.3.2000"
[[lsp.acquire.file]]
os = "windows"
arch = "x86_64"
url = "https://example/rust-analyzer-x86_64-pc-windows-msvc.zip"
sha256 = "…"
member = "rust-analyzer.exe"          # what to take out of the archive
```

Downloads land in `<app-data>/tools/<plugin-id>/<version>/`, which the resolver
(01-03) searches after settings overrides and before `PATH`. Nothing outside
TinkerPad changes, and removing the folder undoes it.

TinkerPad does not own, vendor, or wrap any of these tools. It runs the command
their own maintainers tell you to run, or fetches the release they publish, and
then gets out of the way — the same tool, in the same place, that you would have
installed by hand.

## Why

The detection half already works: the app knows precisely which tool is missing,
for which language, and the plugin already carries the text telling you how to
install it. This is what turns that text into something that can be acted on.

Doing the machinery first, with tests and no UI, keeps the risky parts —
downloading, checksums, archive extraction — reviewable on their own.

## Expectations

- [ ] Manifest: `[toolchain.acquire]` and `[lsp.acquire]`, each `kind = "command"`
      or `kind = "download"`, validated like every other section (unknown keys are
      errors; per-OS entries as elsewhere). `install_hint` stays as the text shown
      when there is no `acquire`, or when acquiring fails.
- [ ] Checksums are per platform *and* per version, so `version` is required on a
      `download` and keys the folder it lands in. A plugin that gets this wrong
      fails to load rather than downloading something unverified.
- [ ] `kind = "command"` runs through `tinkerpad-process` like everything else,
      with its output streamed the way `[deps]` commands are (01-14), and is
      refused with a clear reason when `needs` isn't there ("rustup isn't
      installed").
- [ ] `kind = "download"` verifies the SHA-256 before unpacking, refuses on a
      mismatch, extracts with no path traversal and no symlinks, sets the
      executable bit on Unix, and leaves either the finished tool or nothing —
      never half a file.
- [ ] The resolver searches `<app-data>/tools/<plugin-id>/<version>/` after
      settings overrides and before `PATH`, so an acquired tool is found by runs,
      servers and helpers alike.
- [ ] After either kind, the toolchain probe runs again and the language becomes
      usable without restarting the app.
- [ ] Settings: `[tools] acquire = "ask" | "never"`, and a way to point at an
      internal mirror. `never` means the core refuses to acquire at all, however
      it is asked.
- [ ] Tests: a fake plugin whose `acquire` command writes a stub binary, and a
      `download` against a local file server with a good and a bad checksum.
      Nothing in the test suite reaches the network.

## Out of scope

Anything the user sees: offers, buttons, the status strip list, and the bundled
plugins' own `acquire` sections are 04-08b. Updating tools that are already
installed (a later item, alongside 04-06's app updates). Elevation: anything
needing administrator rights is a hint, not an offer.

## Gotchas

`npm install -g` writes outside the app and can need elevation on some setups;
prefer a download, or an install into `<app-data>/tools/` with `--prefix`, over
touching a global prefix.

A download needs an HTTP client, an archive reader and a hash. Pick them once,
for the workspace, and keep the surface small: no shell, no `curl | sh`, no
post-install scripts.
