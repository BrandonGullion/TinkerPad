# 05-02 · Plugin install

**Phase:** 5 · **Size:** M · **Crate/area:** `tinkerpad-cli`, `tinkerpad-plugin`, `ui/` · **Depends on:** 01-02, 05-01
**Design:** PLUGINS.md §8

## What

`tinkerpad plugin install <path|git-url>` and a Plugins panel in the app that
lists installed and bundled plugins, installs from a path or URL, removes, and
re-runs the registry load without restarting.

## Why

A plugin format nobody can install is a file format. This closes the loop for
contributors and users.

## Expectations

- [ ] Install from a local folder: copy to `<app-data>/plugins/<id>/` (id read
      from the manifest, which must load first). From a git URL: `git clone
      --depth 1` into a temp dir, then the same. `git` missing → clear error.
- [ ] Before copying, run the manifest-load and toolchain checks from 05-01 and
      show the result; install proceeds on manifest success even if the toolchain
      is missing (the user may install it later).
- [ ] Trust notice shown once per install: plugins run with your privileges.
- [ ] Update: `tinkerpad plugin update <id>` re-clones if the plugin folder has a
      `.tinkerpad-source` file recording its origin URL.
- [ ] Remove: moves the folder to the recycle bin; helper and LSP processes for
      that plugin are shut down first.
- [ ] `PluginRegistry::reload()` (01-02) is called after install/remove; open tabs
      for a removed language keep working as plain text with a status note.
- [ ] Plugins panel: list with id, name, version (add `version` to `[language]`
      in the manifest schema, optional), source (bundled / path / URL), toolchain
      status, and actions.

## Out of scope

Dependency between plugins. Signed plugins. Auto-update of plugins.

## Gotchas

A user plugin overriding a bundled one (same id) should show both, with the
bundled one marked "overridden", so it's obvious why behaviour changed.
