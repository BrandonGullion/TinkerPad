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

- [x] `plugin.toml`: probe `cargo --version`; kinds `program` (default, template
      contains `fn main()`) and `expression` (locked `fn main() {
      tinkerpad::dump(&(` … `)); }`); `[scaffold].files` = `Cargo.toml` and
      `.cargo/config.toml`; `[run]` = `cargo run --quiet` in `{dir}`;
      `[output].error_regex` matching rustc's `--> src/main.rs:3:9` line (note it's
      the *second* line of an error; the regex captures the location and the
      message must come from the preceding `error[E0308]: ...` line, so this item
      may need 01-13 to support a two-line pattern or accept location-only);
      `[deps]` = command `cargo add {name}@{version}`.
- [x] Scaffolded `Cargo.toml`: package name from the playground slug, edition 2024,
      `tinkerpad = { path = "<plugin_dir>/dump" }` as an absolute path (written at
      scaffold time), `[profile.dev] opt-level = 0`, `debug = "line-tables-only"`
      (fast to build, still enough for breakpoints and stepping), plus a named
      `[profile.tp-debug]` inheriting dev with `debug = true` for full variable
      inspection. `[run]` uses the dev profile; Phase 6 uses `tp-debug`. Both share
      the target dir. Never `debug = 0`. (DEBUGGING.md D8.)
- [x] `.cargo/config.toml`: `[build] target-dir = "<scratch>/target"` absolute.
- [x] `main.rs` lives at `src/main.rs`, so the playground's main file is
      `src/main.rs`, not `main.rs`. The `[language]` section needs a
      `main_path = "src/main.rs"` field; add it to the manifest schema (01-01) as
      optional, defaulting to `main.<ext>`.
- [x] `dump/` crate: `pub fn dump<T: serde::Serialize>(v: &T) -> &T` and
      `dump_titled`; `Vec<T>` of structs → `table`, else `tree`; depth and row caps;
      writes the OSC bytes to stdout with a single `write_all` and a flush.
- [x] Timings recorded here: first build of a fresh playground; second run
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

## Results

Implemented 2026-09-15.

**Pieces** (`plugins/rust/`)
- `plugin.toml`:
  - id `rust`, `main_path = "src/main.rs"`, and a probe for `cargo --version`
    with a minimum of 1.85, which edition 2024 needs;
  - kinds `program` (default) and `expression`, with locked
    `fn main() {` / `tinkerpad::dump(&(` … `));` / `}`;
  - `[scaffold].files` = `Cargo.toml` and `.cargo/config.toml`;
  - `[run]` = `cargo run --quiet`, cwd `{dir}`, with `CARGO_TERM_COLOR=never`;
  - `[deps]` = command `cargo add {name}@{version}`;
  - a two-line `[output].error_regex` with `error_regex_multiline = true`.
- `templates/Cargo.toml`:
  - package `{{package}}`, edition 2024, `serde` with `derive`, and
    `tinkerpad = { path = "{{plugin_dir}}/dump" }`;
  - an empty `[workspace]`, so a playground inside another Rust project still
    builds on its own;
  - `[profile.dev]` with `opt-level = 0` and `debug = "line-tables-only"`, and
    `[profile.tp-debug]` inheriting from `dev` with `debug = true`.
- `templates/cargo-config.toml`: `[build] target-dir = "{{scratch}}/target"`.
- `dump/`, a small crate on `serde` and `serde_json` (`preserve_order`):
  - `dump(&v) -> &T`, `dump_titled(&v, title)`, and a `Dump` trait with
    `.dump()` and `.dump_titled()` for anything `Serialize`;
  - a non-empty sequence of structs or maps becomes a table, with columns in
    field order; everything else becomes a tree or a value;
  - limits: five levels of nesting and 1,000 rows or items; integers beyond
    2^53 become strings;
  - type badges come from `std::any::type_name`, shortened (`Vec<Person>`);
  - output goes to stdout in one `write_all`, then a flush.
- `README.md`: requirements, kinds, `dump` rules, crates, the playground files,
  the recorded paths, profiles, and speed.

**Core changes the plugin needed**
- **Scaffold variables** (`tinkerpad-plugin::scaffold`). Templates now get
  `{{folder}}`, `{{package}}`, `{{plugin_dir}}`, and `{{scratch}}`, alongside
  `{{name}}` and `{{date}}`.
  - Paths are written with `/` separators and without a `\\?\` prefix.
  - `{{package}}` is the folder name when that is lowercase ASCII starting with
    a letter, otherwise `playground-<id>`. A folder like `2024-notes` or `日本`
    isn't a valid Cargo package name. Package names must also be unique,
    because playgrounds share one target folder.
  - `playground_files` reports which path variables it used.
- **Library** (`tinkerpad-library`). `Library::create_with` hands the chosen
  folder name and id to a callback that returns the files, because
  `{{folder}}` and `{{package}}` aren't known until the folder is claimed.
  - The app creates every playground this way.
  - `playground.toml` accepts a `[plugin.<id>]` table and keeps it when a
    rename rewrites the file.
  - Path variables a playground's files used are recorded there. Rust
    playgrounds get `[plugin.rust] plugin_dir` and `scratch`; TypeScript and C#
    playgrounds get nothing.
- `[language].main_path` already existed (01-01), so no schema change was
  needed.
- The root `Cargo.toml` excludes `plugins/` from the workspace, so the dump
  crate can be tested on its own.
- Documented in PLUGINS.md §3 (the variables table) and §7, ARCHITECTURE.md §6,
  and PLAN.md's decision log.

**Tests**
- `tinkerpad-library`: `create_with` gets the final folder and id, writes the
  files, records `[plugin.<id>]`, and keeps it through a rename. A failed
  callback leaves nothing behind.
- `tinkerpad-plugin` scaffold unit tests cover the variables, forward-slash
  paths (including an apostrophe and `\\?\`), and package names.
- `bundled.rs`:
  - Rust loads cleanly on all three OSes;
  - both kinds create `src/main.rs`, a `Cargo.toml` whose dump path is the
    plugin's, and a shared target dir;
  - a non-ASCII name gets `playground-<id>`;
  - `[plugin.rust]` is recorded, and TypeScript playgrounds record nothing;
  - the expression kind's locked regions are exact.
- `dump` crate unit tests (three).
- `tinkerpad-runner/tests/rust_plugin.rs` has seven tests, each running
  `cargo run` for real (they fail in CI if Cargo is missing):
  - scaffolded absolute paths exist;
  - the program and expression templates;
  - stdout, stderr, `print!`, UTF-8, and dumps interleave in order, with the
    1,000-row cap;
  - the two-line error regex matches rustc's `E0308` as message
    `mismatched types` at `src/main.rs`, line 3, column 21;
  - the captured `15-rust-shapes.txt` fixture matches;
  - the four timings below.
- `tests/dump_cases/15-rust-shapes.txt`: tables, a nested table, tuple, map,
  set, 12 scalar types, the depth limit, and chaining. The dump parser's
  split-point tests run it too.
- New in-app `rust` scenario, with 10 checks covering: a bundled plugin, Cargo
  ready, a crate with `src/main.rs`, the template's table, an unchanged rerun, a
  one-line edit with UTF-8, a second playground, a compile error in the output
  with a failed run, and the expression kind's locked lines and dump. All 122
  checks in 11 scenarios pass on Windows. CI runs it on macOS and Linux, and the
  launch timeout is now 600 s for cold `serde` builds.
- A screenshot showed text, the `people` table, then text, in order.

**Timings** (Windows 11, Rust 1.98.1, `serde` source already downloaded)

| Run | `cargo` directly (test) | In the app, Mod+Enter to table visible |
|---|---|---|
| First build of a fresh playground, empty target folder | 3,881 ms | 4,175 ms |
| Second run, unchanged | 76 ms | 422 ms |
| Second run after a one-line edit | 405 ms | 404 ms |
| First run of a second playground, shared target folder | 384 ms | 409 ms |

A second playground reuses the compiled `serde` and `tinkerpad` from the shared
target folder, so it builds as fast as an edit. In the app, an unchanged rerun
costs as much as an edit, because the app saves `main.rs` before every run and
Cargo sees a newer file. Skipping the save when nothing changed comes with
dirty tracking (01-12).

## Checklist status

**Ticked with a note**
- *Package name from the playground slug.* It's the folder name, which is the
  slug plus any `-2` suffix, so it's unique. When that isn't a valid package
  name, it's `playground-<id>`.
- *The `[language].main_path` field needs adding (01-01).* It was already in the
  schema.
- *Record both paths in `playground.toml` under `[plugin.rust]` (Gotcha).*
  Done, generically: any plugin whose files use `{{plugin_dir}}` or
  `{{scratch}}` gets them recorded.
- *Error regex may need 01-13 for two lines.* The manifest already had
  `error_regex_multiline`. The regex captures the message from the first line
  and the location from the second, tested against real rustc output.

**Open**
- *`cargo run` compile errors appear as annotations (01-13).* The regex is ready
  and tested, and errors already show in the output pane. Editor annotations
  arrive with 01-13.
