# Changelog

What changed, and what it means for people who have written a plugin.

Every release has a **Plugin authors** section. It is the one place that says
whether anything you wrote needs changing, and it says "nothing" when nothing
does, so an empty one is a useful answer rather than a missing one. The rules
behind it are in [VERSIONING.md](VERSIONING.md).

## 0.1.0 — 2026-10-17

The first release.

### Plugin authors

Nothing to change. Manifest `api = 1`, `tinkerpad-plugin-v1` and
`tinkerpad-dump-v1` are the first versions of each, so there is nothing to have
migrated from.

Worth knowing, though, since they are new:

- **`[toolchain.acquire]` and `[lsp.acquire]`** let a plugin say how to install
  what it needs, and TinkerPad offers to do it. Entirely optional;
  `install_hint` is still what someone sees without one.
- **`[lsp].workspace_needs` and `[lsp.workspace_acquire]`** cover a server that
  needs something in its *workspace* rather than on `PATH` — the TypeScript it
  analyses with, for instance.
- **A `check/` folder** (`check/dump.<ext>`, `check/error.<ext>`,
  `check/hang.<ext>`) is what `tinkerpad plugin check` uses to exercise `Dump`,
  `[output].error_regex`, and a helper's cancel. A plugin without one still
  passes; it just checks less.
- **`[language].version`** is optional and yours. TinkerPad never reads it.

### For everyone

- A playground, a library of them, and a run that streams its output.
- Language servers kept warm across tabs, with go-to-definition and
  diagnostics.
- Plugin helpers: a warm process per language, so a run costs milliseconds.
- Rich output: tables, trees, values, images, timings.
- Settings, themes, a command palette, run history, stdin.
- Installing a language's tools, from the window or the command line.
- `tinkerpad plugin check`, `tinkerpad plugin install`, `tinkerpad tools`.

---

The shape of an entry, for whoever writes the first real one:

```markdown
## 0.2.0 — 2026-11-01

### Plugin authors

- **Breaking, `api = 2`:** `[run].timeout` is gone. Nothing replaces it; a run
  ends when it exits or the user cancels it. `tinkerpad plugin migrate` removes
  the key.
- `api = 1` still loads, with a warning, until 0.4.0.

### For everyone

- …
```
