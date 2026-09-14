# 00-02 · Tauri 2 shell and UI scaffold

**Phase:** 0 · **Size:** S · **Crate/area:** `tinkerpad-app`, `ui/` · **Depends on:** 00-01
**Design:** PLAN.md §3.1, ARCHITECTURE.md §7

## What

Turn `tinkerpad-app` into a Tauri 2 application whose frontend is a Vite +
TypeScript project in `ui/`. No framework decision is forced here, but keep the UI
small: vanilla TS or a light library (Preact/Solid) is fine; avoid anything that
makes the bundle or startup heavier than it must be.

## Why

Everything else in Phase 0 needs a window to show up in. This item is deliberately
tiny so that the "is the shell tolerable?" question gets answered in an hour.

## Expectations

- [ ] `cargo tauri dev` opens a window with the title "TinkerPad" and a placeholder
      page. `cargo tauri build` produces an installer or exe on Windows.
- [x] Cold start to window visible is measured and written into this file. Target:
      under 500 ms on this machine. If it's over, note why before moving on.
- [x] One Tauri command (`ping`) and one event (`pong`) round-trip between UI and
      core, as the seed for 00-05.
- [x] The window remembers size and position between launches (Tauri's window-state
      plugin is fine).
- [x] `ui/` has `npm run dev`, `npm run build`, and `npm run typecheck` scripts,
      and typecheck passes.
- [x] CSP in `tauri.conf.json` is set, not disabled.
- [ ] Builds and opens on macOS and Linux (CI build from 00-08; a manual launch
      on at least one of them, VM is fine). Record cold-start time per platform
      here as they become available; Linux/WebKitGTK is the one to watch.
- [ ] macOS keeps Tauri's default app menu so Cmd+Q, Cmd+W, Cmd+, work natively.

## Out of scope

Editor, output pane, tabs, theming. Anything that looks like the product.

## Gotchas

WebView2 is present on Windows 11 by default; the installer should still declare
the dependency so it works on a clean Windows 10.

## Results

Implemented 2026-09-14.

**Launch time**, measured from `run()` starting to the UI's first `ping`
reaching the core (a slight overestimate of "window visible"). Release build,
Windows 11 x64, three consecutive launches:

| Launch | Time |
|---|---|
| First (cold) | 386 ms |
| Second | 287 ms |
| Third | 286 ms |

Under the 500 ms target.

**macOS and Linux:** not launched by hand. The CI `launch` job builds the app
on both and runs `scripts/smoke-launch.sh`, which starts the real window (under
Xvfb on Linux) and waits for the UI to report ready, printing the same timing.
First CI results, debug builds on shared runners (not comparable to the
Windows release numbers above):

| Platform | Time to UI ready |
|---|---|
| macOS arm64 runner | 4346 ms |
| Ubuntu runner under Xvfb | 34275 ms |

The Linux figure is software rendering in a headless VM with a debug build, so
it proves the app starts, not how fast it starts. A hands-on look at WebKitGTK
rendering and startup on real hardware is still needed before a release.

**Window state:** verified by moving and resizing the window, closing it
normally, relaunching, and reading back the same position and size.

**Notes**
- Rust minimum version raised to 1.88; current Tauri 2 dependencies require it.
- The Tauri CLI finds `crates/tinkerpad-app/tauri.conf.json` when run from the
  repo root, so commands run as `ui/node_modules/.bin/tauri …` from there.
- Tauri 3.0 is in alpha; staying on Tauri 2 per `PLAN.md`.

## Checklist status

Checked 2026-09-14.

**Open**
- *`cargo tauri dev` opens a window.* The Tauri CLI is installed through npm, not
  `cargo install`, so the command is `ui/node_modules/.bin/tauri dev`. The
  `tauri build` half is verified: it produced `target/release/tinkerpad.exe`,
  which opens the window. `tauri dev` itself has not been run.
- *Builds and opens on macOS and Linux, with a manual launch on at least one.*
  CI builds and launches the real app on both and runs the in-app self-test
  there. No manual launch has been done.
- *macOS keeps the default app menu.* Tauri 2 enables its default macOS menu
  unless the app sets one, and TinkerPad sets none. Not checked on a Mac.
