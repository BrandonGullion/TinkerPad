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
- [ ] Cold start to window visible is measured and written into this file. Target:
      under 500 ms on this machine. If it's over, note why before moving on.
- [ ] One Tauri command (`ping`) and one event (`pong`) round-trip between UI and
      core, as the seed for 00-05.
- [ ] The window remembers size and position between launches (Tauri's window-state
      plugin is fine).
- [ ] `ui/` has `npm run dev`, `npm run build`, and `npm run typecheck` scripts,
      and typecheck passes.
- [ ] CSP in `tauri.conf.json` is set, not disabled.
- [ ] Builds and opens on macOS and Linux (CI build from 00-08; a manual launch
      on at least one of them, VM is fine). Record cold-start time per platform
      here as they become available; Linux/WebKitGTK is the one to watch.
- [ ] macOS keeps Tauri's default app menu so Cmd+Q, Cmd+W, Cmd+, work natively.

## Out of scope

Editor, output pane, tabs, theming. Anything that looks like the product.

## Gotchas

WebView2 is present on Windows 11 by default; the installer should still declare
the dependency so it works on a clean Windows 10.
