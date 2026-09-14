# 00-01 · Cargo workspace and crate skeletons

**Phase:** 0 · **Size:** S · **Crate/area:** whole workspace · **Depends on:** nothing
**Design:** ARCHITECTURE.md §2

## What

Create the Cargo workspace and an empty crate for each of `tinkerpad-core`,
`tinkerpad-plugin`, `tinkerpad-process`, `tinkerpad-lsp`, `tinkerpad-runner`,
`tinkerpad-library`, `tinkerpad-app`. Each has a `lib.rs` with a doc comment saying
what the crate is for and nothing else. Add the `ui/` and `plugins/` folders with a
placeholder file each.

## Why

The dependency direction in ARCHITECTURE.md §2 is the architecture. Setting it up
empty, first, makes it impossible to accidentally have `tinkerpad-core` depend on
tokio or the app depend on a plugin.

## Expectations

- [ ] `cargo build` succeeds from the repo root with no warnings.
- [ ] Workspace `Cargo.toml` uses `[workspace.dependencies]` for shared crates
      (tokio, serde, serde_json, toml, thiserror, anyhow, tracing) so versions are
      set in one place.
- [ ] `tinkerpad-core` has no dependency on tokio or tauri. Add a comment in its
      `Cargo.toml` saying why.
- [ ] Each crate's `Cargo.toml` lists only the internal crates it is allowed to
      depend on per the diagram. Write the diagram into the workspace `Cargo.toml`
      as a comment.
- [ ] `rust-toolchain.toml` pins stable. `.gitignore` covers `target/`, `ui/node_modules/`,
      `ui/dist/`. `.gitattributes` forces LF line endings for all text files.
- [ ] `cargo clippy --workspace -- -D warnings` and `cargo fmt --check` pass. Keep
      them passing from here on.
- [ ] `git init` and a first commit.

## Out of scope

Any real code. Any UI.

## Gotchas

Name the app crate's binary `tinkerpad` so the CLI in Phase 5 can be a subcommand
of the same binary or a sibling; decide then.
