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

- [x] Manifest: `[toolchain.acquire]` and `[lsp.acquire]`, each `kind = "command"`
      or `kind = "download"`, validated like every other section (unknown keys are
      errors; per-OS entries as elsewhere). `install_hint` stays as the text shown
      when there is no `acquire`, or when acquiring fails.
- [x] Checksums are per platform *and* per version, so `version` is required on a
      `download` and keys the folder it lands in. A plugin that gets this wrong
      fails to load rather than downloading something unverified.
- [x] `kind = "command"` runs through `tinkerpad-process` like everything else,
      with its output streamed the way `[deps]` commands are (01-14), and is
      refused with a clear reason when `needs` isn't there ("rustup isn't
      installed").
- [x] `kind = "download"` verifies the SHA-256 before unpacking, refuses on a
      mismatch, extracts with no path traversal and no symlinks, sets the
      executable bit on Unix, and leaves either the finished tool or nothing —
      never half a file.
- [x] The resolver searches `<app-data>/tools/<plugin-id>/<version>/` after
      settings overrides and before `PATH`, so an acquired tool is found by runs,
      servers and helpers alike.
- [x] After either kind, the toolchain probe runs again and the language becomes
      usable without restarting the app.
- [x] Settings: `[tools] acquire = "ask" | "never"`, and a way to point at an
      internal mirror. `never` means the core refuses to acquire at all, however
      it is asked.
- [x] Tests: a fake plugin whose `acquire` command writes a stub binary, and a
      `download` against a local file server with a good and a bad checksum.
      Nothing in the test suite reaches the network.

## Results

Measured on Windows 11, `cargo test --workspace`, 2026-09-29.

**Where the pieces live**

| Piece | File |
|---|---|
| Manifest shape | `crates/tinkerpad-plugin/src/manifest.rs` (`Acquire`, `AcquireFile`) |
| Validation | `crates/tinkerpad-plugin/src/plugin.rs` (`Builder::acquire`, `inside_archive`) |
| Architecture names | `crates/tinkerpad-plugin/src/os.rs` (`Arch`, with `amd64`/`x64`/`arm64` aliases) |
| Resolution | `crates/tinkerpad-plugin/src/toolchain.rs` (`Resolver::with_acquired`) |
| The engine | `crates/tinkerpad-app/src/tools.rs` |
| Settings | `crates/tinkerpad-core/src/settings.rs` (`[tools]`) |
| Progress events | `CoreEvent::Tool { plugin, line }` |
| Commands | `tool_offer`, `acquire_tool` in `crates/tinkerpad-app/src/lib.rs` |

**Resolution order.** `Resolver::resolve` tries, in order: the `[toolchains.<id>]`
override, then each acquired folder for *that plugin* (newest version first),
then `PATH`. A plugin sees only its own folder, so Go's `gopls` is never what
the Python plugin finds. Covered by
`an_acquired_tool_is_found_after_the_overrides_and_before_path` and
`the_newest_acquired_version_is_the_one_used`.

**Dependencies.** `reqwest`, `sha2`, `zip`, `flate2` and `tar` all already
arrive with `tauri-plugin-updater`, so these are direct edges to crates the
build already had rather than five new dependencies. No shell, no `curl | sh`,
no post-install scripts.

**Tests.** 21 new, all offline.

| Test file | Count | Covers |
|---|---|---|
| `crates/tinkerpad-app/tests/tools.rs` | 7 | the command that installs a stub, a failing command's message, `acquire = "never"` refusing before anything runs, a good and a bad checksum against a local server, a zip member, a tarball whose symlink is skipped |
| `crates/tinkerpad-plugin/tests/acquire.rs` | 12 | what the manifest accepts and, mostly, what it refuses |
| `crates/tinkerpad-plugin/tests/toolchain.rs` | 2 | resolution order and version choice |

Nothing reaches the network. The download tests fetch from a `TcpListener` on
`127.0.0.1` started by the test, reached through the `[tools] mirror` setting —
which exercises the mirror as well, since the manifest URL must be `https://`.
The command tests run `tinkerpad-faketool`, a stand-in package manager in
`crates/tinkerpad-app/src/bin/faketool.rs`, alongside the plugin crate's
existing `tinkerpad-fakeprobe`.

**A bug this found.** An `acquire` limited with `os = [...]` was being resolved
against the machine doing the loading rather than the OS the plugin was being
loaded *for*, so a Windows-only `acquire` would have been kept when validating
for Linux (`plugin.rs` used `Os::current()` where every other per-OS section
uses `self.os`). `a_command_for_another_os_is_simply_absent` covers it now.

**Deferred to 04-08b**, as the split says: everything the user sees, and the
bundled plugins' own `acquire` sections. The two commands exist and are
registered; nothing in the window calls them yet.

**Not fixed here (pre-existing on `main`, both from PRs merged while CI was
billing-blocked):** `scripts/lint-platform-cfg.sh` fails on
`crates/tinkerpad-app/src/main.rs:72` (04-06's Windows console attach), and
`bundled_plugins_load_cleanly_on_every_os` fails because the PowerShell plugin
names a `powershell` highlighter that isn't in `KNOWN_HIGHLIGHTERS` (04-07).
Neither is this item; each wants its own small PR.

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
