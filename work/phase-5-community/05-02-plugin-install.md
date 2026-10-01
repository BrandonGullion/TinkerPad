# 05-02 · Plugin install

**Phase:** 5 · **Size:** M · **Crate/area:** `tinkerpad-cli`, `tinkerpad-plugin`, `ui/` · **Depends on:** 01-02, 05-01a
**Design:** PLUGINS.md §8

## What

`tinkerpad plugin install <path|git-url>` and a Plugins panel in the app that
lists installed and bundled plugins, installs from a path or URL, removes, and
re-runs the registry load without restarting.

## Why

A plugin format nobody can install is a file format. This closes the loop for
contributors and users.

## Expectations

- [x] Install from a local folder: copy to `<app-data>/plugins/<id>/` (id read
      from the manifest, which must load first). From a git URL: `git clone
      --depth 1` into a temp dir, then the same. `git` missing → clear error.
- [x] Before copying, run the manifest-load and toolchain checks from 05-01a and
      show the result; install proceeds on manifest success even if the toolchain
      is missing (the user may install it later).
- [x] Trust notice shown once per install: plugins run with your privileges.
- [x] Update: `tinkerpad plugin update <id>` re-clones if the plugin folder has a
      `.tinkerpad-source` file recording its origin URL.
- [x] Remove: moves the folder to the recycle bin; helper and LSP processes for
      that plugin are shut down first.
- [x] `PluginRegistry::reload()` (01-02) is called after install/remove; open tabs
      for a removed language keep working as plain text with a status note.
- [x] Plugins panel: list with id, name, version (add `version` to `[language]`
      in the manifest schema, optional), source (bundled / path / URL), toolchain
      status, and actions.

## Results

Measured on Windows 11, 2026-10-01.

**The loop, end to end, without a restart.** The `plugins-panel` self-test
writes a plugin of its own, installs it, finds it in the registry, the panel
and the list of what each language needs, then removes it and finds the count
back where it started — all in one session:

```
PASS the palette opens the plugins panel
PASS every loaded plugin is listed ({"rows":9,"plugins":8})
PASS a bundled plugin says so, and offers no Remove
PASS installing a folder loads the plugin without a restart (Panel Demo)
PASS and it appears with the version and source its manifest gives
PASS the panel notices by itself, and offers Remove for a plugin that isn't bundled
PASS a new language shows up in what each language needs
PASS removing it takes it out of the registry, again without a restart
PASS and the count is back where it started ({"before":8,"after":8})
PASS the command closes it again
```

**The command line.**

| Command | Verified |
|---|---|
| `plugin install <folder>` | Copies, shows the trust notice, then says what the language still needs |
| `plugin install <not-a-plugin>` | Refused with the loader's own words; nothing copied |
| `plugin list` | Bundled and user plugins, with version and origin |
| `plugin remove <id>` | Folder to the recycle bin, gone from the list |

```
$ tinkerpad plugin install …/myplug
A plugin is a program. It runs with your privileges, like an editor extension:
it can read and write your files and start other programs. Install plugins you
would run by hand.

Installed Demo 0.3.1 at …\plugins\demo
It still needs:
  Toolchain tp-not-here
    There is nothing to install; this is a demonstration.
`tinkerpad tools install demo` can get what it can.
```

**What `reload` has to do before it reloads.** A plugin's language server and
helper are processes started from a folder that may no longer be there, so
they are stopped first; they start again on demand from whatever is there now.
Open tabs are left alone: a tab whose language has just been removed keeps its
text and stops having a language. Nothing the user typed is worth losing over
a plugin they uninstalled.

**Decisions worth recording.**

- **The id in the manifest names the folder**, not the source folder's name.
  Two plugins with the same id are the same plugin, and installing one over
  another replaces it — with the old folder going to the recycle bin first, so
  a failed install doesn't take the previous version with it. A test checks
  that a file the new copy doesn't have is gone afterwards.
- **`.git`, `node_modules`, `target`, `bin` and `obj` are not copied.** A
  repository is not a plugin; a 14 MB `bin/obj` in an installed plugin was the
  exact mistake 04-06 found in the installer. Helper binaries are built on
  first use (PLUGINS.md §8).
- **A missing toolchain doesn't block an install.** The install says what is
  still needed and points at `tinkerpad tools install`, because installing the
  plugin is usually how someone finds out what to install next.
- **`[language].version` is optional and never interpreted.** `api` is the
  compatibility contract; this is what the plugin calls itself, for the list.

**The panel.** Reached from the palette ("Plugins…"). One row per plugin: name
and version, then id and toolchain, then where it came from — a URL for a
plugin cloned from one, a folder for a working copy, "bundled" otherwise. A
bundled plugin gets no Remove button. A bundled plugin a user plugin has
replaced is listed too, marked, so it is obvious why behaviour changed.

The first screenshot showed rows three lines tall where a long folder path
wrapped, and a button broken across two lines; the path now gets a line of its
own that truncates, with the whole of it in the tooltip.

**Tests** — 12 in `crates/tinkerpad-app/tests/install.rs` and 3 unit tests in
the module, plus the 10-check `plugins-panel` scenario. The install tests cover
the copy, a folder that isn't a plugin, a manifest that doesn't load (asserting
the loader's suggestion survives), replacing an existing plugin, the skipped
folders, a path install having no URL to update from, removing, removing
something that isn't there, and that a warning in a manifest reaches the
caller rather than being swallowed.

**Not tested here: cloning.** `git clone` needs a network, so the only part
checked is that a failure says which program is missing or what git said.

## Out of scope

Dependency between plugins. Signed plugins. Auto-update of plugins.

## Gotchas

A user plugin overriding a bundled one (same id) should show both, with the
bundled one marked "overridden", so it's obvious why behaviour changed.
