# 00-08 · Cross-platform CI and platform modules

**Phase:** 0 · **Size:** S · **Crate/area:** CI, `tinkerpad-process`, `tinkerpad-plugin` · **Depends on:** 00-01, 00-04
**Design:** PLAN.md §3.6, ARCHITECTURE.md §10

## What

A GitHub Actions matrix over Windows, macOS (arm64), and Linux that builds and
tests the workspace and the UI on every push, plus the skeleton of the two
platform modules every later item hangs platform code on.

## Why

Supporting all three platforms is a stated goal. A platform bug found the week it
was written costs minutes; the same bug found in Phase 4 costs a rewrite. CI is the
only way to keep macOS and Linux honest from a Windows desk.

## Expectations

- [ ] `.github/workflows/ci.yml`: matrix `windows-latest`, `macos-latest`,
      `ubuntu-latest`; steps: Rust stable, `cargo fmt --check`, `cargo clippy
      --workspace -- -D warnings`, `cargo test --workspace`, Node LTS, `npm ci`,
      `npm run typecheck`, `npm run build`. Linux installs `libwebkit2gtk-4.1-dev`,
      `libgtk-3-dev`, `libayatana-appindicator3-dev` before building.
- [ ] `cargo tauri build` (no bundling; `--no-bundle` or a debug build) runs on
      all three so webview linkage is verified, not just the crates.
- [ ] `tinkerpad-process::platform` module with `windows.rs`, `macos.rs`,
      `linux.rs` and a shared trait or function set: `register_child`,
      `kill_tree`, `spawn_flags`. Stub bodies are fine here; 00-04 fills them.
- [ ] `tinkerpad-plugin::env::discover_path()` module with the same three-way
      split; returns the inherited PATH for now; 01-03 fills in the login-shell
      query.
- [ ] A lint: a CI step greps for `cfg(target_os` and `cfg(windows` outside the
      two platform modules and fails if any are found.
- [ ] `.gitattributes` with `* text=auto eol=lf`; `rustfmt.toml` with
      `newline_style = "Unix"`.
- [ ] Badge in the README. All three green before moving on.

## Out of scope

Release bundling, signing, cross builds for arm64 Linux / x64 macOS (04-06).
Toolchain installs for plugin checks (added when 05-01 exists).

## Gotchas

macOS runners are arm64 now; if an x64 macOS check is ever wanted it's
`macos-13`. Linux tests that spawn processes must not assume a display; keep
webview-touching tests out of `cargo test`.
