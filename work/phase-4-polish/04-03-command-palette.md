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

- [x] `registerCommand({ id, title, run, when? })` with categories; all existing
      actions (run, cancel, new, open, close, rename, add package, restart server,
      toggle library, open settings, …) migrate to it.
- [x] Palette with fuzzy filter, shows shortcut and category, remembers the last
      used order at the top.
- [x] `<app-data>/keybindings.toml` maps `command.id = "mod+shift+r"`; `mod` is
      Cmd on macOS and Ctrl elsewhere; `ctrl`/`cmd` may be given explicitly for
      platform-specific bindings. Applied over defaults at load and on change
      (watcher). Conflicts reported in a toast.
- [x] Palette and docs render shortcuts with the platform's glyphs (`⌘⇧R` on
      macOS, `Ctrl+Shift+R` elsewhere).
- [x] Default keymap documented in a generated `docs/KEYBINDINGS.md` (a command
      that dumps the registry).
- [x] Editor keymap and app keymap don't fight: app shortcuts are registered with
      CodeMirror as high-precedence keymap entries when the editor has focus.

## Out of scope

A visual keybinding editor. Vim/Emacs modes (CodeMirror has packages; make it a
setting later if wanted).

## Gotchas

`Mod+Enter` inside a completion popup should accept the completion, not run.
Order the keymaps accordingly and test it.

## Results

Implemented 2026-09-28.

**One registry, in two files.** `ui/src/commands/catalog.js` is every command
as data — id, title, category, default keys, and the condition it needs — and
`registry.ts` binds handlers to those ids, resolves bindings, matches key
events, and formats keys for the platform. Nothing else in the UI decides what
a key does: the hand-written `switch` in `main.ts` and the hand-written
shortcut strings in the old palette are both gone.

The catalog is plain JavaScript with JSDoc types on purpose: the app imports it
and so does `npm run docs:keys`, running in Node, which writes
`docs/KEYBINDINGS.md`. One list, so the document cannot drift from the keymap,
and no build step to make that true. CI runs `npm run docs:keys:check`, so a
keymap change that doesn't update the document fails the build.

**Twenty-five commands** in six categories (Run, Playground, Tabs, Go, View,
Settings), including everything that had a key before and the several that only
had a palette entry. A command with a `when` it doesn't meet is not offered and
its key does nothing — the palette lists what you can actually do now.

**The palette** shows each command's category and its keys in the platform's
own glyphs (`⌘⇧P` on macOS, `Ctrl+Shift+P` elsewhere), and offers what you used
last first, remembered in `localStorage`.

**`<app-data>/keybindings.toml`** is one line per command, read and watched the
way settings are (04-02): `"run.start" = "mod+shift+r"`, and `""` to take a key
away. `mod` is Cmd on macOS and Ctrl elsewhere; `ctrl` and `cmd` mean
themselves. A broken file keeps the bindings that were working and says which
line. Two commands sharing a shortcut is a toast naming both, not a refusal:
the later one wins, because refusing both would leave the user with neither.

**The editor gets the same commands**, as a high-precedence CodeMirror keymap
built from the registry and rebuilt in a `Compartment` when bindings change —
so a rebound key works inside the text as well as outside it, with no view
recreated and no second list of shortcuts.

**The gotcha.** `Mod+Enter` with a completion popup open accepts the completion
instead of running; the handler asks `completionStatus` first and hands the key
to `acceptCompletion`. The self-test proves it by checking the document becomes
the completion's text. (Writing that test also showed CodeMirror ignores an
accept within 75 ms of the popup opening — an anti-misclick guard, not a bug.)

**Tests.** Five in the app for the file (a missing file, tidied bindings, the
template being all comments, the line number of a broken file, a `[section]`
refused with an explanation), and a new `keys` self-test scenario: nine checks,
all passing:

```
PASS the palette lists commands with their category and shortcut
PASS the command used last is offered first
PASS the default binding runs its command
PASS a rebound command loses its old key, in the editor too
PASS the binding from keybindings.toml runs the command
PASS the palette shows the keys the user chose
PASS Mod+Enter accepts a completion instead of running
PASS two commands on one shortcut are reported
PASS emptying the file gives the defaults back
```

`run`, `tabs`, `library-panel`, `settings` and `packages` pass with the keymap
rebuilt underneath them.

## Checklist status

**Deviation: a command that can't run now isn't listed.** "Add package…" with
no playground open used to be offered and then say there was nothing to add to;
it is now left out of the palette, which is what `when` is for. The `packages`
scenario was updated to expect that.

**Note: a stale check in the `packages` scenario was repaired here.** It wanted
a plugin with no `[deps]`, and the test plugin gained one in 03-07, so it had
been failing on `main`. It now uses a plugin with neither `[deps]` nor an
`add_dependency` hook when one is installed, and says so when none is.

**Note: `docs/KEYBINDINGS.md` is generated by `npm run docs:keys`**, not by an
in-app command. The app can't write into the repository; what it does have is
"Copy keybindings as Markdown", which puts the same table on the clipboard.
