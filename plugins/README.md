# Bundled plugins

Each folder here is a language plugin shipped with TinkerPad. The format is
exactly the same as a plugin a user installs; see `docs/PLUGINS.md`.

This folder is copied into the app bundle as a resource
(`crates/tinkerpad-app/tauri.conf.json`, `bundle.resources`). At runtime the app
loads these first, then the user's plugin folder, where a plugin with the same
id replaces the bundled one.

Bundled today: `typescript` (01-09), `csharp` (01-10), `rust` (01-11), and
`python` (03-04) — the last of which ships a warm helper, and is the one to read
if you are writing a plugin of your own.

Editing a file here re-copies the folder into `target/` on the next build (the
app's `build.rs` watches it). To try changes without rebuilding, copy a plugin
into your user plugins folder instead: it overrides the bundled one.
