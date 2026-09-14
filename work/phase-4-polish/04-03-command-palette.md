# 04-03 · Command palette and keybindings

**Phase:** 4 · **Size:** M · **Crate/area:** `ui/` · **Depends on:** 01-12, 04-02
**Design:** ARCHITECTURE.md §7

## What

A command registry on the UI side that every action goes through, a palette
(`Mod+Shift+P`) that lists them with their shortcuts, and a `keybindings.toml`
that overrides defaults.

## Why

A registry is what makes the app discoverable without menus and lets the shortcuts
be changed without touching code. Several earlier items already assumed a palette.

## Expectations

- [ ] `registerCommand({ id, title, run, when? })` with categories; all existing
      actions (run, cancel, new, open, close, rename, add package, restart server,
      toggle library, open settings, …) migrate to it.
- [ ] Palette with fuzzy filter, shows shortcut and category, remembers the last
      used order at the top.
- [ ] `<app-data>/keybindings.toml` maps `command.id = "mod+shift+r"`; `mod` is
      Cmd on macOS and Ctrl elsewhere; `ctrl`/`cmd` may be given explicitly for
      platform-specific bindings. Applied over defaults at load and on change
      (watcher). Conflicts reported in a toast.
- [ ] Palette and docs render shortcuts with the platform's glyphs (`⌘⇧R` on
      macOS, `Ctrl+Shift+R` elsewhere).
- [ ] Default keymap documented in a generated `docs/KEYBINDINGS.md` (a command
      that dumps the registry).
- [ ] Editor keymap and app keymap don't fight: app shortcuts are registered with
      CodeMirror as high-precedence keymap entries when the editor has focus.

## Out of scope

A visual keybinding editor. Vim/Emacs modes (CodeMirror has packages; make it a
setting later if wanted).

## Gotchas

`Mod+Enter` inside a completion popup should accept the completion, not run.
Order the keymaps accordingly and test it.
