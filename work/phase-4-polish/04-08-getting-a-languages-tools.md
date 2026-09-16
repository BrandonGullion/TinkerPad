# 04-08 · Getting a language's tools

**Phase:** 4 · **Size:** M · **Crate/area:** `tinkerpad-plugin`, `tinkerpad-app`, `ui/`, `plugins/*` · **Depends on:** 01-03, 02-08
**Design:** PLUGINS.md §2 (`[toolchain]`), ARCHITECTURE.md §4.1

## What

TinkerPad already *checks* what a language needs: the toolchain probes (01-03)
run at launch and report each language as ready or missing with a hint. This item
is the other half — a plugin says how to **get** what is missing, and TinkerPad
offers to run it: "rust-analyzer isn't installed. Install it?", one button, in
the place that already says so.

TinkerPad does not own, vendor, or wrap any of these tools. It runs the command
their own maintainers tell you to run, or fetches the release they publish, and
then gets out of the way — the same tool, in the same place, that you would have
installed by hand.

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

Right now a language works only if you already happen to have its server. The
plugin knows the exact command; the user has to find a README and run it in a
terminal they may not have open. That is a bad first five minutes for an app
whose pitch is "open it and start typing", and it gets worse with every language
added.

The detection half already works, which is what makes this cheap: the app knows
precisely which tool is missing, for which language, and the plugin already
carries the text telling you how to install it. Turning that text into a button
is a small step with a large effect.

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
- [ ] Somewhere to see it all: a list of what each language needs, whether it is
      there, where it was found, and a button per missing one. The status strip
      (02-08) is the natural home; a language with everything it needs should say
      so as plainly as one without.
- [ ] The same from a terminal, for setting up a machine or a CI runner without
      opening the window: `tinkerpad tools list` and `tinkerpad tools install
      <language>`, sharing the code the button uses. (The CLI itself is 05-01;
      this is one more subcommand in it.)
- [ ] Tests: a fake plugin whose `acquire` command writes a stub binary, and a
      `download` against a local file server with a good and a bad checksum.
      Nothing in the test suite reaches the network.

## Out of scope

Updating tools that are already installed (a later item, alongside 04-06's app
updates). Installing *plugins* — that is 05-02. Elevation: anything needing
administrator rights is a hint, not an offer.

**Shipping the servers inside TinkerPad** is a different idea and not this one:
it would triple the download for tools most people won't use, pin versions that
go stale, and take on licence obligations that aren't ours. Installing the same
thing the user would have installed themselves keeps the tool theirs — upgradable
with their package manager, shared with their editor, and removable without us.

## Gotchas

`npm install -g` writes outside the app and can need elevation on some setups;
prefer a download, or an install into `<app-data>/tools/` with `--prefix`, over
touching a global prefix. Checksums have to be per platform *and* per version, so
the manifest needs a version field to key them; a plugin that gets this wrong
should fail to load rather than download something unverified.
