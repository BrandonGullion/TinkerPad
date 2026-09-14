# 04-02 · Settings

**Phase:** 4 · **Size:** M · **Crate/area:** `tinkerpad-core`, `tinkerpad-app`, `ui/` · **Depends on:** 01-03, 02-06, 03-02
**Design:** PLAN.md §5 Phase 4

## What

One `settings.toml` with a typed schema, hot-reloaded on change, and a settings UI
that edits it. Everything that earlier items stubbed as "in settings" becomes real
here.

## Why

Until now settings were a hand-edited file. That's fine for you and wrong for
anyone else.

## Expectations

- [ ] `Settings` struct in `tinkerpad-core` with `serde` defaults for every field:
      `library.root`, `editor.font_size`, `editor.font_family`, `editor.tab_size`,
      `run.clear_output`, `toolchains.<id>.<program>`, `lsp.prewarm`,
      `lsp.<id>.idle_shutdown`, `helpers.<id>.idle_shutdown`, `theme`.
- [ ] File at `<app-data>/settings.toml`; written with comments for every field
      (generate from doc comments or a hand-maintained template) on first launch.
- [ ] Watcher on the file: edits from outside apply within a second; invalid TOML
      keeps the last good settings and shows a toast with the line number.
- [ ] Settings UI (`Mod+,`): a simple form grouped by section, with per-plugin
      subsections generated from the plugin registry (toolchain path override,
      LSP idle, helper idle). Saving writes the file; the watcher applies it, so
      there's one code path.
- [ ] Changing `library.root` prompts to confirm and reloads the library.
- [ ] "Open settings file" and "Reset to defaults" commands.

## Out of scope

Per-playground settings. Sync. Keybinding editing (04-03).

## Gotchas

Font changes must reach the editor without recreating it (CodeMirror theme
reconfigure via a `Compartment`).
