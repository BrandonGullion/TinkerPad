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

- [x] `cargo tauri build` produces an NSIS installer that installs per-user (no
      admin), registers an uninstaller, and includes the bundled `plugins/`.
- [x] App icon (a real one, even if simple) in all required sizes.
- [ ] macOS: universal or per-arch `.app` in a `.dmg`; signed with a Developer
      ID certificate and notarized in CI (`tauri-action` supports both given the
      secrets). Without an Apple developer account, ship unsigned and document the
      right-click → Open workaround in the README; decide which and record it in
      PLAN.md's decision log.
- [ ] Linux: AppImage (self-contained, includes WebKitGTK deps) and `.deb`
      declaring `libwebkit2gtk-4.1-0` and `libgtk-3-0`. Tested on a fresh Ubuntu
      LTS VM.
- [x] GitHub Actions: on tag `v*`, the 00-08 matrix builds bundles for
      windows x64, macos arm64 (+ x64 if signing is set up), linux x64; arm64 Linux
      as a best-effort cross build; upload artifacts, create a release with the
      updater manifest.
- [x] Updater: checks on launch (after the window is visible), shows a
      non-blocking "Update available" toast, installs on next launch or on click.
      Signing key for updater manifests generated and stored as a repo secret.
- [x] `--version` and `--help` on the exe work without opening a window.
- [ ] A fresh Windows VM install → launch → run hello-world in TS/C#/Rust (with
      toolchains installed) works with no dev tooling present.

## Out of scope

Code signing certificates (add if SmartScreen becomes annoying). Store listings.

## Gotchas

WebView2 bootstrapper: choose "download bootstrapper" in Tauri's bundle config so
the installer stays small but still works on a machine without WebView2.

## Results

Implemented 2026-09-29.

**Bundles.** `cargo tauri build` produces an NSIS installer and an MSI on
Windows, a `.dmg` on macOS, and an AppImage and `.deb` on Linux, all from the
one config. NSIS is `installMode: currentUser`: no administrator prompt, an
uninstaller in Add or remove programs, and WebView2 through the download
bootstrapper so the installer stays small and still works on a machine without
it. The `.deb` declares `libwebkit2gtk-4.1-0` and `libgtk-3-0`.

**Verified by installing it.** The signed NSIS installer ran silently with no
elevation, landed in `%LOCALAPPDATA%\TinkerPad`, registered an uninstaller
under `HKCU` with publisher and version, and carried all four plugins with
their manifests. `tinkerpad --version` answered from the installed build, and
the uninstaller removed the folder and the registry entry.

**A packaging bug found on the way.** The first installer was 8.4 MiB, of which
14 MiB unpacked was `plugins/csharp/helper/bin` and `obj` — what a hand-run
`dotnet build` leaves in the plugin folder. The helper is built into the app's
data folder at runtime and those files should never ship (a stale `obj` caused
a real compile failure back in 03-05). They are now ignored for every plugin,
not just C#, and the installer is 5.05 MiB.

**An icon of its own**: a dark rounded tile with a bracket pair and a caret,
drawn as a PNG by a script with no image dependencies (pure `zlib` and
`struct`, supersampled four times), then expanded by `tauri icon` into every
size Windows, macOS and Linux ask for.

**`--version` and `--help`** answer and exit without opening a window. On
Windows that needed care: a release build is a GUI program with no console, so
it writes to standard output when it has one (piped or redirected) and attaches
to the calling console and writes to `CONOUT$` when it doesn't. Double-clicked
there is neither, and it prints nowhere, which is right.

**The updater** checks once, five seconds after the window appears — never
before it — and says so in a toast with an **Install and restart** button.
Toasts with something to do now stay until they are answered; the others still
fade. The palette gained **Install update**, offered only when there is one. A
failed check is a debug line: nobody asked. `createUpdaterArtifacts` is on, and
a local build with the key signs both bundles.

**The release workflow** (`.github/workflows/release.yml`) builds on a `v*` tag
across Windows x64, macOS arm64 and x64, and Linux x64, with Linux arm64 on a
native runner as best effort (`continue-on-error`). It signs the update
manifests from repository secrets and opens a **draft** release with
`includeUpdaterJson: true` — draft, because nothing should publish itself
before a person has looked at what was built.

**Measured**: Windows installer 5.05 MiB (NSIS), 7.20 MiB (MSI), installed size
about 21 MiB of which the app is 20 MiB.

## Checklist status

**Deferred: the two fresh-VM criteria.** A clean Windows VM install-and-run and
a fresh Ubuntu LTS test of the `.deb` and AppImage can't be done from here.
What was done instead: a real per-user install and uninstall on this machine
(above), and the Linux packages built by the workflow. The checklist for the
VMs is below — it wants doing before the first release is published, not before
this merges.

**Deferred: macOS and Windows code signing.** Decided and recorded in PLAN.md's
log: ship unsigned, document the right-click → Open workaround (it is in the
README), and add signing when there is a certificate. The workflow already
reads the signing secrets, so adding them is all it will take.

**Note: the updater's key was generated for this work item.** The public key is
in `tauri.conf.json`; the private key and its (empty) password go in the
repository's secrets as `TAURI_SIGNING_PRIVATE_KEY` and
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. The private key is not in this repository
and never should be.

**Replaced on 2026-10-17, before the first release.** The original private key
(`747D267B11B15EEA`) wasn't to hand, so a new pair (`77EDAA68ED9320D7`) was
generated and its public key put in `tauri.conf.json`. Nothing had shipped, so
no installed copy trusted the old one.

## Before the first release

1. Add the two `TAURI_SIGNING_*` secrets to the repository.
2. Tag a version (`git tag v0.1.0 && git push --tags`) and watch the workflow;
   it leaves a draft release.
3. On a clean Windows VM: install from the NSIS installer, launch, create a
   TypeScript playground and run it (with Node installed), then uninstall.
4. On a fresh Ubuntu LTS VM: `apt install ./TinkerPad_*.deb`, launch, run a
   playground; then the AppImage on the same machine with nothing else
   installed.
5. On macOS: confirm the right-click → Open path matches what the README says.
6. Publish the draft, then check that an older build offers the update and
   installs it.
