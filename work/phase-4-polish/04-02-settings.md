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

- [x] `Settings` struct in `tinkerpad-core` with `serde` defaults for every field:
      `library.root`, `editor.font_size`, `editor.font_family`, `editor.tab_size`,
      `run.clear_output`, `toolchains.<id>.<program>`, `lsp.prewarm`,
      `lsp.<id>.idle_shutdown`, `helpers.<id>.idle_shutdown`, `theme`.
- [x] File at `<app-data>/settings.toml`; written with comments for every field
      (generate from doc comments or a hand-maintained template) on first launch.
- [x] Watcher on the file: edits from outside apply within a second; invalid TOML
      keeps the last good settings and shows a toast with the line number.
- [x] Settings UI (`Mod+,`): a simple form grouped by section, with per-plugin
      subsections generated from the plugin registry (toolchain path override,
      LSP idle, helper idle). Saving writes the file; the watcher applies it, so
      there's one code path.
- [x] Changing `library.root` prompts to confirm and reloads the library.
- [x] "Open settings file" and "Reset to defaults" commands.

## Out of scope

Per-playground settings. Sync. Keybinding editing (04-03).

## Gotchas

Font changes must reach the editor without recreating it (CodeMirror theme
reconfigure via a `Compartment`).

## Results

Implemented 2026-09-21.

**The schema lives in the core.** `tinkerpad_core::settings::Settings` is the
whole of `settings.toml` with a default for every field, so a missing file, a
missing section and a missing key all mean the same thing. Nothing in it reads
a file: the app owns that. Its `TEMPLATE` is the commented file written on
first launch, hand-written because a good settings file reads like prose and a
generated one reads like a schema — with two tests keeping it honest, one
parsing every uncommented default and comparing it with `Settings::default()`,
one checking the per-plugin examples are the shape they claim.

**`crates/tinkerpad-app/src/settings.rs` owns the file**, and two rules shape
it:

- *A broken file costs you the edit and nothing else.* Invalid TOML keeps the
  settings that were working and toasts what is wrong with the line number.
  Never a failure to start, never a reason to overwrite what the user wrote.
- *Saving keeps the file yours.* Writes go through `toml_edit`, so comments,
  key order, and anything a newer build left behind all survive. A setting put
  back to its default has its line removed rather than written out, and an
  emptied `[toolchains.python]` goes with it.

**One path from a change to the app.** The file is watched (folder-level, since
most editors save by rename); the settings form writes the file and the watcher
applies it. The window never changes the app directly, so an edit made in Vim
and an edit made in the form arrive the same way — as a `settings` event
carrying the whole of the new settings.

**What reads them now.** Toolchain overrides (`ToolchainOverrides::of`), the
`[lsp]` and `[helpers]` idle and pre-warm settings (derived per use, so a
change reaches the next server or helper started), the library root, and in the
window: editor font size, font family, tab size, whether a run clears the
output pane, and the theme name. The two text parsers those settings used to
have are gone; there is one parse of the file now.

**The form** (`Mod+,`) is grouped by section, with the per-plugin parts
generated from the plugin registry: a pre-warm box and an idle box for every
plugin with a language server, an idle box for every plugin with a helper, and
a path box for every program a plugin's toolchain probes look for. Plus "Open
settings file" and "Reset to defaults".

**Moving the library** asks twice — once for the folder, once to confirm — then
writes the setting, closes every tab, points the library at the new folder and
watches it there. No restart.

**The gotcha.** Font size and family are CSS variables, so they cost a property
each and CodeMirror never notices. Tab size is editor state, and goes in
through a `Compartment` reconfigured in place; a tab whose state was made
before the change gets the same reconfiguration when it is switched to.

**Tests.** Eighteen in the core (defaults, clamping, the template, a file from
a newer build), nine in the app (a missing file, the template written once, the
line number of a broken file, saving keeping comments and writing only what
changed, an unknown key surviving a save), plus the service-settings tests
moved onto the typed schema. In the app, a new `settings` self-test scenario,
nine checks, all passing:

```
PASS Mod+, opens the settings form, with a row per plugin that has a server
PASS saving writes the setting and the editor takes it
PASS the file keeps its comments, and only what changed is written
PASS an edit made in another editor applies on its own
PASS the editor's tab size follows the setting
PASS with clear_output off, a run adds to what is already there
PASS a broken file keeps the last good settings and says which line
PASS Reset to defaults puts the file and the window back
PASS the library can be pointed at another folder and back
```

## Checklist status

**Note: `theme` is stored and applied as `data-theme` on the document, but
there is only one theme.** 04-04 adds the rest; this item owns the setting, not
the colours.

**Note: the settings that reach the UI are the file's own snake_case keys**,
not the camelCase the rest of the protocol uses. The object the window is given
is the settings file, and it goes straight back to `save_settings`; a second
spelling of every setting would be one more thing to keep in step.

**Note: nothing in `Settings` skips a field when it serializes.** The file
decides what to hold, key by key, when it is written; the window is handed
every field, because a form missing half its rows is worse than some nulls.
