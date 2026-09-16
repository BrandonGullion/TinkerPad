# Rust plugin

Runs Rust playgrounds with Cargo. Every playground is a real crate, with
`src/main.rs` and a `Cargo.toml`, and includes `dump` for rich output.

## What it needs

Rust 1.85 or newer (for edition 2024), installed with rustup:
https://rustup.rs. If `cargo` isn't on your `PATH`, point TinkerPad at it in
`settings.toml`:

```toml
[toolchains.rust]
cargo = "/home/me/.cargo/bin/cargo"
```

## Kinds

| Kind | Starts with |
|---|---|
| **Rust Program** (default) | `fn main()` with a struct and a dump. |
| **Rust Expression** | One expression, dumped by a locked `fn main()` around it. |

## `dump`

```rust
use serde::Serialize;
use tinkerpad::Dump;

#[derive(Serialize)]
struct Person { name: String, born: u16 }

let people = load_people().dump_titled("people");
tinkerpad::dump(&people.len());
```

Anything that implements `serde::Serialize` can be dumped. The playground's
`Cargo.toml` already depends on `serde` with `derive`. Import the trait with
`use tinkerpad::Dump;` to call `.dump()` or `.dump_titled(title)`. Or call the
functions `tinkerpad::dump(&value)` and `tinkerpad::dump_titled(&value, title)`.
Both return the reference, so calls chain.

| Value | Shown as |
|---|---|
| A non-empty `Vec`, slice, or array of structs or maps | A sortable table, with columns in field order |
| Any other collection, struct, map, tuple, or enum with data | A collapsible tree |
| Numbers, strings, `bool`, `char`, unit-like enums, `None` | A single value with its Rust type |

Details:

- **Big values.** Tables keep 1,000 rows and mark themselves truncated.
  Collections keep 1,000 items.
- **Nesting.** Five levels are shown; deeper structures become a summary such as
  `[array, 3 items]`.
- **Numbers.** Integers too big for JavaScript, such as `u64::MAX`, are shown as
  strings. `NaN` and infinities are shown as `null`, as `serde_json` writes them.
- **Serialisation errors.** A value that can't be serialised, such as a map with
  non-string keys, shows the error instead.

## Crates

Run `cargo add <crate>` in the playground folder, or edit `Cargo.toml` by hand.

## Files in a playground

| Path | Purpose |
|---|---|
| `src/main.rs` | The code. |
| `Cargo.toml` | The package: edition 2024, `serde`, and `tinkerpad` as a path dependency on this plugin's `dump/` folder. |
| `.cargo/config.toml` | Builds into a target folder shared by every Rust playground, under TinkerPad's scratch folder. |

The package name is the folder name, such as `fizz-buzz`. When the folder name
isn't a valid package name (for example `2024-notes` or `日本`), it's
`playground-` plus the playground's id. Names are unique either way, which
matters because playgrounds share one target folder.

The shared target folder is the warm cache. `serde` and the dump crate compile
once, and every later playground reuses them.

Two absolute paths are written into these files when a playground is created:
this plugin's `dump/` folder and the shared target folder. Both are also
recorded in the playground's `playground.toml` under `[plugin.rust]`. If
TinkerPad moves, those two lines are all that needs fixing.

### Profiles

- **`dev`**, used by Mod+Enter: `opt-level = 0` and `debug = "line-tables-only"`.
  Fast to build, with enough debug information for a debugger to step.
- **`tp-debug`**, for the debugger (Phase 6): `dev` with full debug information,
  so variables can be inspected. It shares the target folder.

## Speed

Measured on Windows 11, from Mod+Enter until the dump is visible:

| Run | Time |
|---|---|
| First build of a fresh playground, empty target folder | about 4 s |
| Rerun, unchanged | about 0.4 s |
| Run after a one-line edit | about 0.4 s |
| First run of a second playground | about 0.4 s |

An unchanged rerun still recompiles your crate, because TinkerPad saves
`main.rs` before every run. Run from a terminal without changes, Cargo skips the
build and it takes under 0.1 s. These times assume `serde`'s source is already
downloaded; on a machine that has never fetched it, the first build downloads it
too.

## The language server

Completions, hover, and live errors come from rust-analyzer, which ships with
rustup:

```sh
rustup component add rust-analyzer
```

One rust-analyzer serves every Rust playground. It is rooted at a folder of
TinkerPad's own (`<app-data>/scratch/rust`) rather than at any playground, and
each open playground adds its `Cargo.toml` to the server's `linkedProjects`
setting; opening another one adds it without a restart, and closing a tab takes
it out again. That is what `[lsp.register]` in the manifest does.

`checkOnSave` is off: a background `cargo check` would compete with the run
button for the shared target folder, and the errors it would report are the ones
you get by running. Syntax and type errors still appear as you type, from
rust-analyzer itself.
