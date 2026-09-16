# 04-08 · Getting a language's tools

**Phase:** 4 · **Size:** M · **Crate/area:** `tinkerpad-plugin`, `tinkerpad-app`, `ui/`, `plugins/*` · **Depends on:** 01-03, 02-08
**Design:** PLUGINS.md §2 (`[toolchain]`), ARCHITECTURE.md §4.1

## What

A plugin can say how to *get* the tools it needs, not only how to find them, and
TinkerPad offers to do it: "rust-analyzer isn't installed. Install it?" with one
button, in the place that already says the tool is missing.

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

## Why

Right now a language works if you already happen to have its server. The plugin
knows the exact command; the user has to read a README and run it in a terminal
they may not have open. That is a bad first five minutes for a tool whose pitch
is "open it and start typing", and it gets worse with every language added.

Bundling the servers instead was considered and rejected: it triples the download
for tools most people won't use, pins versions that go stale, and takes on
licence obligations that aren't ours to take.

## Expectations

- [ ] Manifest: `[toolchain.acquire]` and `[lsp.acquire]`, each `kind = "command"`
      or `kind = "download"`, validated like every other section (unknown keys are
      errors; per-OS entries as elsewhere). `install_hint` stays as the text shown
      when there is no `acquire`, or when acquiring fails.
- [ ] **Never silent, never automatic.** Missing tool → an offer, in the status
      strip (02-08) and in the toast that already reports it, showing exactly what
      will happen: the command line to be run, or the URL, size, and checksum to
      be fetched. Installing is one click and one confirmation, and nothing is
      installed without it.
- [ ] `kind = "command"` runs through `tinkerpad-process` like everything else,
      with its output streamed into a pane the way `[deps]` commands are (01-14),
      and is skipped with a clear reason when `needs` isn't there ("rustup isn't
      installed").
- [ ] `kind = "download"` verifies the SHA-256 before unpacking, refuses on a
      mismatch, extracts with no path traversal and no symlinks, sets the
      executable bit on Unix, and is resumable-or-restartable rather than leaving
      half a file. No shell, no `curl | sh`, no post-install scripts.
- [ ] After either kind, the toolchain probe runs again and the language becomes
      usable without restarting the app.
- [ ] Workspace dependencies count as tools: the TypeScript server needs a
      `typescript` in the library folder (02-07), so "install what this language
      needs" has to be able to run that `npm install` too, not just fetch a binary.
- [ ] Settings: `[tools] acquire = "ask" | "never"`, and a way to point at an
      internal mirror. An offline or locked-down machine should see the hint and
      no offer, not a failing download.
- [ ] The three bundled plugins declare what they need: rustup for rust-analyzer,
      npm for typescript-language-server plus the workspace's TypeScript, and
      whatever 03-05 settles on for C#.
- [ ] Tests: a fake plugin whose `acquire` command writes a stub binary, and a
      `download` against a local file server with a good and a bad checksum.
      Nothing in the test suite reaches the network.

## Out of scope

Updating tools that are already installed (a later item, alongside 04-06's app
updates). Installing *plugins* — that is 05-02. Elevation: anything needing
administrator rights is a hint, not an offer.

## Gotchas

`npm install -g` writes outside the app and can need elevation on some setups;
prefer a download, or an install into `<app-data>/tools/` with `--prefix`, over
touching a global prefix. Checksums have to be per platform *and* per version, so
the manifest needs a version field to key them; a plugin that gets this wrong
should fail to load rather than download something unverified.
