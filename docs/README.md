# TinkerPad

A code playground: write a few lines, press Ctrl+Enter, see what they do.
One plugin per language, language servers and runtimes kept warm, rich output
instead of `print`.

This is the documentation. The source is in the repository above.

## If you are adding a language

Start with [the plugin contract](PLUGINS.md) — it is the public API, and §10 is
a checklist. Copy `plugin-template/` from the repository and change four lines;
it passes `tinkerpad plugin check` before you touch it.

The three reference pages are generated from the types that implement them, so
they cannot drift:

- [Manifest](reference/manifest.md) — every key `plugin.toml` accepts, plus a
  [JSON Schema](reference/plugin-toml.schema.json) your editor can validate
  against.
- [Helper protocol](reference/helper-protocol.md) — the messages a warm helper
  speaks.
- [Dump values](reference/dump.md) — what rich output looks like on the wire.

[Versioning](VERSIONING.md) says what TinkerPad promises about all three, and
[the changelog](CHANGELOG.md) has a *Plugin authors* section per release.

## If you are working on TinkerPad itself

[Architecture](ARCHITECTURE.md) is the process model, the crates, and how runs
and language servers are supervised. [Debugging constraints](DEBUGGING.md) is
the list of things Phase 6 must not be blocked by, which every earlier phase
has to respect. [Keyboard shortcuts](KEYBINDINGS.md) is generated from the
command catalogue.
