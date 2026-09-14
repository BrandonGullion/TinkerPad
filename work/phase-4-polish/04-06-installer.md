# 04-06 · Installer and updates

**Phase:** 4 · **Size:** L · **Crate/area:** `tinkerpad-app`, CI · **Depends on:** 00-02, 00-08, 01-02
**Design:** PLAN.md §3.6, §5 Phase 4

## What

Release bundles for all three platforms from one tag: NSIS on Windows, a
signed and notarized `.dmg` on macOS, AppImage and `.deb` on Linux; a GitHub
Actions release workflow; and Tauri's updater pointing at GitHub releases.

## Why

The moment someone else installs it, "cargo tauri dev" stops being a distribution
strategy. Also needed before Phase 5 makes sense.

## Expectations

- [ ] `cargo tauri build` produces an NSIS installer that installs per-user (no
      admin), registers an uninstaller, and includes the bundled `plugins/`.
- [ ] App icon (a real one, even if simple) in all required sizes.
- [ ] macOS: universal or per-arch `.app` in a `.dmg`; signed with a Developer
      ID certificate and notarized in CI (`tauri-action` supports both given the
      secrets). Without an Apple developer account, ship unsigned and document the
      right-click → Open workaround in the README; decide which and record it in
      PLAN.md's decision log.
- [ ] Linux: AppImage (self-contained, includes WebKitGTK deps) and `.deb`
      declaring `libwebkit2gtk-4.1-0` and `libgtk-3-0`. Tested on a fresh Ubuntu
      LTS VM.
- [ ] GitHub Actions: on tag `v*`, the 00-08 matrix builds bundles for
      windows x64, macos arm64 (+ x64 if signing is set up), linux x64; arm64 Linux
      as a best-effort cross build; upload artifacts, create a release with the
      updater manifest.
- [ ] Updater: checks on launch (after the window is visible), shows a
      non-blocking "Update available" toast, installs on next launch or on click.
      Signing key for updater manifests generated and stored as a repo secret.
- [ ] `--version` and `--help` on the exe work without opening a window.
- [ ] A fresh Windows VM install → launch → run hello-world in TS/C#/Rust (with
      toolchains installed) works with no dev tooling present.

## Out of scope

Code signing certificates (add if SmartScreen becomes annoying). Store listings.

## Gotchas

WebView2 bootstrapper: choose "download bootstrapper" in Tauri's bundle config so
the installer stays small but still works on a machine without WebView2.
