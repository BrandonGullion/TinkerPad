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

- [x] Bundled `plugins/` is included in the Tauri bundle as a resource and located
      at runtime via Tauri's resource path API. `cargo tauri dev` also finds it.
- [x] User plugins dir is `<app-data>/plugins/`, created on first launch. The path
      is shown in the plugins panel later (05-02) and logged at startup now.
- [x] Load order: bundled first, then user. A user plugin with the same `id`
      replaces the bundled one; a log line says so.
- [x] A plugin that fails to load is reported (log + a startup toast naming the
      folder and the error) and skipped. One bad plugin never prevents the others
      from loading.
- [x] `PluginRegistry` API: `all()`, `get(id)`, `by_extension(ext)`. Read-only after
      startup for now; a `reload()` exists but only the CLI/plugins panel will call it.
- [x] Startup time impact measured: loading three bundled plugins adds under 5 ms.
- [x] `tinkerpad-core` gains a `LanguageId` newtype; nothing else about plugins
      leaks into core.

## Out of scope

Installing plugins (05-02), hot reload while running, plugin UI.

## Gotchas

Tauri resource paths differ between dev and bundled builds. Wrap the lookup in one
function and test it in both modes before moving on.

## Results

Implemented 2026-09-14.

**Pieces**
- `tinkerpad-core`: `LanguageId`, a string newtype that serialises as a plain
  string. `Plugin::id` now uses it. No other plugin types moved into core.
- `tinkerpad-plugin` `registry.rs`: `PluginRegistry` with `load`, `reload`,
  `all`, `get`, `by_extension`, `failures`, `overrides`, and `load_time`.
  Folders are scanned in name order; entries are sorted by id.
- `tinkerpad-app` `plugins.rs`: `bundled_plugins_dir` is the one function that
  finds bundled plugins. It uses Tauri's resource directory, falling back to the
  repository's `plugins/` in debug builds. `user_plugins_dir` is
  `<app-data>/plugins`, or `TINKERPAD_USER_PLUGINS` when set.
- `plugins/README.md` holds the bundled folder's place until the first real
  plugins arrive (01-09 to 01-11). `tauri.conf.json` copies it as a resource.
- Startup failures become toasts. They are queued at startup and published on
  the UI's first `ping`, because events sent before the UI subscribes are lost.
- `list_plugins` command returns folders, plugins, failures, and load time. The
  in-app self-test uses it, and the plugins panel (05-02) will too.

**Verified in the app** with a `plugins` self-test scenario and a user plugin
folder holding one valid plugin and one with a typo:

| Mode | Bundled folder resolved to | Result |
|---|---|---|
| `tauri build --no-bundle` (release) | `target/release/plugins` (resource dir) | 1 loaded, 1 failed, toast shown, all checks pass |
| `tauri dev` | `target/debug/plugins` (resource dir) | Same |
| CI debug build, macOS and Linux | resource dir | Runs in the `launch` job, which also asserts `loaded=1 failures=1` |

Tauri copies resources next to the binary in dev builds too, so the debug
fallback to the repository folder hasn't been needed in any mode tried. It stays
as a safety net for a plain `cargo run`.

**Timing**

| Measurement | Time |
|---|---|
| Registry test, three plugins, release, median of 25 | 0.80 ms |
| Registry test, three plugins, release, worst of 25 | 1.63 ms |
| Release app startup, 1 plugin + 1 failure (from the app's own log) | 1.10 ms |
| `tauri dev` startup, same folder, debug build | 2.28 ms |

Under the 5 ms budget.

**Registry tests** (9, in `tests/registry.rs`): bundled then user sorted by id;
extension lookup ignoring case and dot; user override with the log and
`overrides()` record; broken plugins in both folders reported while others load;
two folders with one id in the same root; non-plugin folders and loose files
ignored; missing folders tolerated and the user folder created; reload picks up
additions and removals; timing.

**Decisions**
- Two folders with the same id *in the same root* are not an override. The second
  is a failure naming the first, so a copy-paste mistake doesn't silently win.
- `by_extension` returns the plugin with the lowest id when several claim one
  extension. A proper choice (such as asking the user) belongs to the new-tab
  flow in 01-12.
- The `\\?\` prefix Windows adds to resource paths is stripped for display, so
  logs, errors, and the UI show ordinary paths.
- `TINKERPAD_USER_PLUGINS` exists for tests and portable setups. It is documented
  in `plugins.rs`.
