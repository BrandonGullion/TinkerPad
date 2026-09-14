# AGENTS.md

Instructions for anyone, human or AI agent, making changes to this repository.

## What this project is

TinkerPad is a LINQPad-style desktop code playground. Rust core, Tauri 2 shell,
CodeMirror 6 editor, one plugin folder per language, language servers and
runtimes kept warm. Read these before changing anything non-trivial:

- `PLAN.md`: vision, decisions, roadmap, decision log.
- `docs/ARCHITECTURE.md`: process model, crates, LSP keep-alive, execution.
- `docs/PLUGINS.md`: the plugin contract. This is a public API.
- `docs/DEBUGGING.md`: constraints D1 to D15 that every phase must respect.
- `work/README.md`: the backlog, one file per work item.

## Workflow rules

These apply to all work after Phase 0.

1. **Never commit directly to `main`.** Every change goes through a pull request.
2. **Branch per work item.** Branch names use the format
   `dev/{meaningful-name}`, lowercase, words separated by hyphens. Include the
   work item id when there is one, for example `dev/01-01-manifest-loader`.
3. **One work item per pull request.** Each PR implements exactly one work item
   from `work/`. Don't bundle a second item into it, and don't start the next
   item on the same branch. If an item is too large, split the *work item file*
   first (in its own PR), then implement each part in its own PR.
4. **Link the work item.** The PR title starts with the item id, for example
   `01-01 Manifest schema and loader`. The PR description links the work item
   file and lists each acceptance criterion as done, not done, or deferred with
   a reason.
5. **Tick the box in the same PR.** When the item is complete, change its
   checkbox in `work/README.md` from `[ ]` to `[x]` as part of the PR. Record
   any measurements the item asks for inside the work item file itself.
6. **Changes that aren't a work item** (a typo fix, a CI repair, a doc
   correction) still use a `dev/` branch and a PR. Say in the description that
   no work item applies.
7. **CI must be green on Windows, macOS, and Linux** before a PR is merged.
8. **Don't run ahead.** Do the item you were asked to do. Don't start the next
   one unless asked.

## Engineering rules

- Keep `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  and `cargo test --workspace` passing. In `ui/`, keep `npm run typecheck` and
  `npm run build` passing.
- Respect the crate dependency direction in the root `Cargo.toml`.
  `tinkerpad-core` never depends on tokio or tauri.
- Platform-specific code lives only in `tinkerpad-process/src/platform/` and
  `tinkerpad-plugin/src/env.rs`. `scripts/lint-platform-cfg.sh` enforces this.
- Nothing language-specific goes into Rust. It belongs in a plugin folder.
- Spawn child processes only through `tinkerpad-process`.
- Tests that spawn processes use the test binaries in `tinkerpad-process`, not
  shell commands, so they run on every OS.
- Write shortcuts as `Mod+…` in docs and use CodeMirror's `Mod-` prefix in code.
- Don't violate a `docs/DEBUGGING.md` constraint without updating that document
  and `PLAN.md`'s decision log in the same PR.
- When a design decision changes, add a row to the decision log in `PLAN.md`.

## Commands

```sh
# Rust
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
bash scripts/lint-platform-cfg.sh

# UI (from ui/)
npm ci
npm run typecheck
npm run build

# App (from repo root)
npm --prefix ui run tauri dev
```
