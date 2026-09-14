# 01-11 · Rust plugin

**Phase:** 1 · **Size:** M · **Crate/area:** `plugins/rust/` · **Depends on:** 01-09 (pattern), 01-05, 01-06, 01-07
**Design:** PLUGINS.md §7 (Rust), §6

## What

The Rust plugin as a pure folder. Each playground is a real crate; `[scaffold]`
writes `Cargo.toml` and `.cargo/config.toml` pointing at a shared target dir so
dependencies compile once across playgrounds. `dump/` is a tiny crate.

## Why

Dogfooding, and the first plugin that needs `[scaffold]` and `{scratch}`. It proves
the "warm cache from the filesystem" idea in ARCHITECTURE.md §5.2.

## Expectations

- [ ] `plugin.toml`: probe `cargo --version`; kinds `program` (default, template
      contains `fn main()`) and `expression` (locked `fn main() {
      tinkerpad::dump(&(` … `)); }`); `[scaffold].files` = `Cargo.toml` and
      `.cargo/config.toml`; `[run]` = `cargo run --quiet` in `{dir}`;
      `[output].error_regex` matching rustc's `--> src/main.rs:3:9` line (note it's
      the *second* line of an error; the regex captures the location and the
      message must come from the preceding `error[E0308]: ...` line, so this item
      may need 01-13 to support a two-line pattern or accept location-only);
      `[deps]` = command `cargo add {name}@{version}`.
- [ ] Scaffolded `Cargo.toml`: package name from the playground slug, edition 2024,
      `tinkerpad = { path = "<plugin_dir>/dump" }` as an absolute path (written at
      scaffold time), `[profile.dev] opt-level = 0`, `debug = "line-tables-only"`
      (fast to build, still enough for breakpoints and stepping), plus a named
      `[profile.tp-debug]` inheriting dev with `debug = true` for full variable
      inspection. `[run]` uses the dev profile; Phase 6 uses `tp-debug`. Both share
      the target dir. Never `debug = 0`. (DEBUGGING.md D8.)
- [ ] `.cargo/config.toml`: `[build] target-dir = "<scratch>/target"` absolute.
- [ ] `main.rs` lives at `src/main.rs`, so the playground's main file is
      `src/main.rs`, not `main.rs`. The `[language]` section needs a
      `main_path = "src/main.rs"` field; add it to the manifest schema (01-01) as
      optional, defaulting to `main.<ext>`.
- [ ] `dump/` crate: `pub fn dump<T: serde::Serialize>(v: &T) -> &T` and
      `dump_titled`; `Vec<T>` of structs → `table`, else `tree`; depth and row caps;
      writes the OSC bytes to stdout with a single `write_all` and a flush.
- [ ] Timings recorded here: first build of a fresh playground; second run
      unchanged; second run after a one-line edit; first run of a second playground
      (should reuse the compiled `tinkerpad` crate and std).
- [ ] `cargo run` compile errors appear as annotations (01-13).

## Out of scope

rust-analyzer (02-07). Cargo features UI.

## Gotchas

Absolute paths in scaffolded files break if the plugin folder or scratch dir moves.
Record both in `playground.toml` under a plugin-owned `[plugin.rust]` table so a
future "repair" command can rewrite them. Add that free-form table to the
`playground.toml` schema in 01-04 if not already there.
