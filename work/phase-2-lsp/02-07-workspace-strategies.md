# 02-07 · Workspace strategies and registration

**Phase:** 2 · **Size:** M · **Crate/area:** `tinkerpad-lsp`, `plugins/*` · **Depends on:** 02-02, 02-04
**Design:** ARCHITECTURE.md §4.4, PLUGINS.md §4, §7

## What

Implement all four `[lsp].workspace` strategies and the declarative
`[lsp.register]` mechanism, then add the `[lsp]` section to each bundled plugin
and verify completions work in all three.

## Why

Each language wants its server rooted differently. This item is where the plugin
manifest proves it can describe rust-analyzer, tsserver, and Roslyn without
language-specific Rust.

## Expectations

- [ ] Strategies: `none` (no root URI, `workspaceFolders: null`); `playground_dir`
      (one server per playground; pool key includes the dir); `library_root` (root
      is the library, one server); `scratch_workspace` (root is
      `<app-data>/scratch/<plugin-id>/`, one server).
- [ ] `[lsp.register]`: on attach/detach the core sends
      `workspace/didChangeConfiguration` with `settings` containing the configured
      `setting` path set to the list of `value` for all currently attached
      playgrounds. Also answered when the server asks via
      `workspace/configuration`.
- [ ] **TypeScript:** `typescript-language-server --stdio`, `library_root`, and a
      scaffolded `tsconfig.json` at the library root (written once, never
      overwritten) so loose `.ts` files type-check with Node types. Completions on
      `[].` work.
- [ ] **Rust:** `rust-analyzer`, `scratch_workspace`, `[lsp.register]` with
      `rust-analyzer.linkedProjects` = `{dir}/Cargo.toml`. Opening a second Rust
      playground adds it without a restart; completions on `Vec::` work in both.
      Time to first completion after opening a fresh playground recorded here.
- [ ] **C#:** Roslyn LSP with `workspace = "none"` if it works with file-based
      apps, else `csharp-ls` with a scaffolded `main.csproj`. Whichever ships, the
      plugin README explains how to install it. Add `[toolchain.acquire]` to the
      manifest schema only if an automatic download is the chosen route; document
      the decision in PLAN.md's decision log.
- [ ] A missing server binary degrades gracefully per 02-02 and the status strip
      says which binary and how to get it.
- [ ] Tests: the fake server receives the expected `rootUri` for each strategy;
      `didChangeConfiguration` payload for two attached playgrounds matches.

## Out of scope

Helper-driven registration via hooks (03-07). Multiple workspace folders.

## Gotchas

rust-analyzer needs `linkedProjects` at `initialize` time *or* via config change
plus a `rust-analyzer/reloadWorkspace` request. Try config change alone first; add
the reload request if it doesn't pick up the new project.
