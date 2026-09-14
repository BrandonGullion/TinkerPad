# 01-02 · Plugin discovery

**Phase:** 1 · **Size:** S · **Crate/area:** `tinkerpad-plugin`, `tinkerpad-app` · **Depends on:** 01-01
**Design:** PLUGINS.md §1, §8; ARCHITECTURE.md §2

## What

A `PluginRegistry` that finds plugin folders in the bundled location and in
`<app-data>/plugins/`, loads each, applies the override rule, and exposes them to
the rest of the app. Bundled plugins are copied into the app bundle at build time.

## Why

"A plugin is a folder" only means something if the app finds folders at runtime and
treats bundled ones no differently from installed ones.

## Expectations

- [ ] Bundled `plugins/` is included in the Tauri bundle as a resource and located
      at runtime via Tauri's resource path API. `cargo tauri dev` also finds it.
- [ ] User plugins dir is `<app-data>/plugins/`, created on first launch. The path
      is shown in the plugins panel later (05-02) and logged at startup now.
- [ ] Load order: bundled first, then user. A user plugin with the same `id`
      replaces the bundled one; a log line says so.
- [ ] A plugin that fails to load is reported (log + a startup toast naming the
      folder and the error) and skipped. One bad plugin never prevents the others
      from loading.
- [ ] `PluginRegistry` API: `all()`, `get(id)`, `by_extension(ext)`. Read-only after
      startup for now; a `reload()` exists but only the CLI/plugins panel will call it.
- [ ] Startup time impact measured: loading three bundled plugins adds under 5 ms.
- [ ] `tinkerpad-core` gains a `LanguageId` newtype; nothing else about plugins
      leaks into core.

## Out of scope

Installing plugins (05-02), hot reload while running, plugin UI.

## Gotchas

Tauri resource paths differ between dev and bundled builds. Wrap the lookup in one
function and test it in both modes before moving on.
