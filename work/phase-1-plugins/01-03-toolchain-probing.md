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

- [ ] Probes run concurrently across plugins at startup, on a background task, with
      a 3 s timeout each. The UI never waits for them; status arrives as an event.
- [ ] `min_version` compared semver-ish (major.minor.patch, tolerate extra suffixes
      like `10.0.100-rc.1`). Below minimum reports `Missing` with a "found 9.0.3,
      need 10.0" message.
- [ ] Settings file (`<app-data>/settings.toml`, created if absent) supports
      `[toolchains.<plugin-id>] <program> = "<path>"`. Overrides are used for
      probes *and* for every later command spawn of that program.
- [ ] Program lookup on Windows tries `.exe`, `.cmd`, `.bat` extensions in `PATHEXT`
      order, so `npm` and `typescript-language-server` resolve.
- [ ] macOS and Linux: `discover_path()` (00-08) runs `$SHELL -ilc 'printf %s
      "$PATH"'` once at startup with a 2 s timeout and uses the result for every
      lookup, because GUI apps don't inherit the shell's PATH. Falls back to the
      inherited PATH on failure and logs it. Verified by launching the `.app` from
      Finder on a Mac with Homebrew-installed Node and seeing it probe as Ready.
- [ ] Probe lists may have per-OS entries: `{ command = "python3", os = ["macos",
      "linux"] }`, `{ command = "py", os = ["windows"] }`. Entries without `os`
      apply everywhere. Add the field to the manifest schema (01-01).
- [ ] A "recheck toolchains" command re-runs probes without restart.
- [ ] Creating a playground in a language whose toolchain is `Missing` is allowed
      (you might install it later); running one shows the `install_hint` in the
      output pane.
- [ ] Tests with a fake probe binary (a tiny script in `tests/`) for: success, wrong
      version, not found, timeout.

## Out of scope

`[toolchain.acquire]` downloads (needed for the Roslyn LSP; do it in 02-07 or the
C# plugin item when it becomes real).

## Gotchas

`dotnet --version` prints only the SDK version; `node --version` prints `v24.14.0`
with a leading `v`. Strip leading non-digits before parsing.
