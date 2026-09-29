# Bundled plugins

Each folder here is a language plugin shipped with TinkerPad. The format is
exactly the same as a plugin a user installs; see `docs/PLUGINS.md`.

This folder is copied into the app bundle as a resource
(`crates/tinkerpad-app/tauri.conf.json`, `bundle.resources`). At runtime the app
loads these first, then the user's plugin folder, where a plugin with the same
id replaces the bundled one.

Bundled today: `typescript` (01-09), `csharp` (01-10), `rust` (01-11),
`python` (03-04), `go` and `powershell` (04-07).

Which to read first depends on what you are writing:

- **`powershell`** is the smallest a plugin gets — four required sections, no
  server, no helper, and rich output from twenty lines in a locked prelude.
- **`python`** is the middle: a warm helper that runs code, written to be read.
- **`go`** is the largest: a scaffolded module per playground, one language
  server across all of them, and a helper whose only job is to answer the
  playground hooks by rewriting `go.work`.

Editing a file here re-copies the folder into `target/` on the next build (the
app's `build.rs` watches it). To try changes without rebuilding, copy a plugin
into your user plugins folder instead: it overrides the bundled one.
