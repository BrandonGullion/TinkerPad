# 01-03 · Toolchain probing

**Phase:** 1 · **Size:** S · **Crate/area:** `tinkerpad-plugin` · **Depends on:** 01-01, 00-04
**Design:** PLUGINS.md §2 (`[toolchain]`)

## What

For each loaded plugin, run its `[toolchain].probe` list in order until one
succeeds, record the version, and expose a `ToolchainStatus` of `Ready { version,
path }` or `Missing { hint }`. User overrides from settings win over `PATH`.

## Why

The difference between "confusing error in the output pane" and "install the .NET
10 SDK, here's the link" is this item.

## Expectations

- [x] Probes run concurrently across plugins at startup, on a background task, with
      a 3 s timeout each. The UI never waits for them; status arrives as an event.
- [x] `min_version` compared semver-ish (major.minor.patch, tolerate extra suffixes
      like `10.0.100-rc.1`). Below minimum reports `Missing` with a "found 9.0.3,
      need 10.0" message.
- [ ] Settings file (`<app-data>/settings.toml`, created if absent) supports
      `[toolchains.<plugin-id>] <program> = "<path>"`. Overrides are used for
      probes *and* for every later command spawn of that program.
- [x] Program lookup on Windows tries `.exe`, `.cmd`, `.bat` extensions in `PATHEXT`
      order, so `npm` and `typescript-language-server` resolve.
- [ ] macOS and Linux: `discover_path()` (00-08) runs `$SHELL -ilc 'printf %s
      "$PATH"'` once at startup with a 2 s timeout and uses the result for every
      lookup, because GUI apps don't inherit the shell's PATH. Falls back to the
      inherited PATH on failure and logs it. Verified by launching the `.app` from
      Finder on a Mac with Homebrew-installed Node and seeing it probe as Ready.
- [x] Probe lists may have per-OS entries: `{ command = "python3", os = ["macos",
      "linux"] }`, `{ command = "py", os = ["windows"] }`. Entries without `os`
      apply everywhere. Add the field to the manifest schema (01-01).
- [x] A "recheck toolchains" command re-runs probes without restart.
- [ ] Creating a playground in a language whose toolchain is `Missing` is allowed
      (you might install it later); running one shows the `install_hint` in the
      output pane.
- [x] Tests with a fake probe binary (a tiny script in `tests/`) for: success, wrong
      version, not found, timeout.

## Out of scope

`[toolchain.acquire]` downloads (needed for the Roslyn LSP; do it in 02-07 or the
C# plugin item when it becomes real).

## Gotchas

`dotnet --version` prints only the SDK version; `node --version` prints `v24.14.0`
with a leading `v`. Strip leading non-digits before parsing.

## Results

Implemented 2026-09-14.

**Pieces**
- `tinkerpad-plugin` `env.rs`: `discover_path()` asks the login shell on macOS
  and Linux (`$SHELL -ilc`, 2 s timeout, markers around the output so rc-file
  banners can't pollute it, `fish` handled) and falls back to the inherited
  `PATH`. `find_program()` searches a given `PATH`, honours `PATHEXT` on
  Windows, and requires the executable bit on Unix.
- `tinkerpad-plugin` `toolchain.rs`: `ToolchainOverrides` reads
  `[toolchains.<plugin>]` from settings text. `Resolver` finds a plugin's program:
  override first, then `PATH`. `probe()` runs a plugin's probes in order with a
  timeout, parses a version from stdout or stderr, and checks `min_version`.
- `tinkerpad-core`: `ToolchainState` (`checking`, `ready`, `missing`) and a
  `toolchain` event.
- `tinkerpad-app`: `settings.rs` creates `<app-data>/settings.toml` with a
  commented template. `toolchains.rs` probes every plugin concurrently in the
  background at startup, stores results, and publishes an event for each.
  Commands `toolchain_status` and `recheck_toolchains`. A generation counter
  stops a slow older check from overwriting a newer one.
- The hard-coded Node run (00-06) now finds `node` on the discovered `PATH` and
  passes that `PATH` to the child, so a Finder-launched Mac app can find Homebrew
  Node.

**Tests**
- 8 unit tests in `toolchain.rs` and `env.rs` run on every OS: version parsing
  from real tool output (node, dotnet, cargo, python, go, a .NET prerelease,
  PowerShell, java), minimum-version comparison, settings parsing, `PATH` search
  order, explicit paths, and a missing shell. Windows adds a `PATHEXT` test.
  Unix adds tests for the executable bit, the login-shell query, and discovery.
- 11 integration tests in `tests/toolchain.rs` use `tinkerpad-fakeprobe`, a tiny
  Rust binary: success, too old ("found 9.0.3, need 10.0"), prerelease suffix,
  version on stderr, not found, non-zero exit, timeout (returns in well under a
  second with a 300 ms limit), fallback to a later probe, all reasons combined, an
  override beating `PATH`, and an override pointing nowhere.

**In the app** (`toolchains` self-test scenario, Windows release build):

```
selftest: INFO toolchain mini missing `mini` was not found
selftest: INFO toolchain node-check ready node 24.14.0 at C:\Program Files\nodejs\node.exe
selftest: PASS missing toolchain has a reason
selftest: PASS real toolchain probes as ready with a version
selftest: PASS recheck reports every toolchain again (2 result events for 2 plugins)
```

The first check finished 11 ms after startup, the recheck in 6 ms. CI runs the
same scenario on macOS and Linux with the `node-check` test plugin
(`tests/fixtures/valid/node-check`) and fails unless it probes as ready.

## Checklist status

**Ticked with a note**
- *Tests with a fake probe script in `tests/`.* A Rust binary
  (`src/bin/fakeprobe.rs`) instead of a script, per AGENTS.md, so the same tests
  run on every OS.
- *Probe lists may have per-OS entries.* Already delivered by 01-01; exercised
  here by the `per-os` fixture.
- *Status arrives as an event.* Also queryable with `toolchain_status`, because
  probes can finish before the UI subscribes (the first check took 11 ms; the UI
  is ready at about 330 ms).

**Open**
- *Overrides used for every later command spawn.* `Resolver::resolve` applies
  overrides and is ready for it, but no plugin command is spawned yet. Work item
  01-06 runs plugin commands and must go through the resolver. The hard-coded
  Node run uses the discovered `PATH` but has no plugin id to look overrides up
  under.
- *Verified by launching the `.app` from Finder on a Mac.* Not done: it needs a
  Mac. The login-shell query is tested on macOS and Linux CI (`/bin/sh`), but CI
  launches the app from a shell that already has the right `PATH`, so it can't
  show the Finder case.
- *Creating a playground with a missing toolchain is allowed; running shows the
  install hint.* Playgrounds (01-04) and plugin runs (01-06) don't exist yet.
  `ToolchainState::Missing` carries the `hint` those items need.
