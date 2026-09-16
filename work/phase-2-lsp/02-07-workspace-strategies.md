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

- [x] Strategies: `none` (no root URI, `workspaceFolders: null`); `playground_dir`
      (one server per playground; pool key includes the dir); `library_root` (root
      is the library, one server); `scratch_workspace` (root is
      `<app-data>/scratch/<plugin-id>/`, one server).
- [x] `[lsp.register]`: on attach/detach the core sends
      `workspace/didChangeConfiguration` with `settings` containing the configured
      `setting` path set to the list of `value` for all currently attached
      playgrounds. Also answered when the server asks via
      `workspace/configuration`.
- [x] **TypeScript:** `typescript-language-server --stdio`, `library_root`, and a
      scaffolded `tsconfig.json` at the library root (written once, never
      overwritten) so loose `.ts` files type-check with Node types. Completions on
      `[].` work.
- [x] **Rust:** `rust-analyzer`, `scratch_workspace`, `[lsp.register]` with
      `rust-analyzer.linkedProjects` = `{dir}/Cargo.toml`. Opening a second Rust
      playground adds it without a restart; completions on `Vec::` work in both.
      Time to first completion after opening a fresh playground recorded here.
- [ ] **C#:** Roslyn LSP with `workspace = "none"` if it works with file-based
      apps, else `csharp-ls` with a scaffolded `main.csproj`. Whichever ships, the
      plugin README explains how to install it. Add `[toolchain.acquire]` to the
      manifest schema only if an automatic download is the chosen route; document
      the decision in PLAN.md's decision log.
- [x] A missing server binary degrades gracefully per 02-02 and the status strip
      says which binary and how to get it.
- [x] Tests: the fake server receives the expected `rootUri` for each strategy;
      `didChangeConfiguration` payload for two attached playgrounds matches.

## Out of scope

Helper-driven registration via hooks (03-07). Multiple workspace folders.

## Gotchas

rust-analyzer needs `linkedProjects` at `initialize` time *or* via config change
plus a `rust-analyzer/reloadWorkspace` request. Try config change alone first; add
the reload request if it doesn't pick up the new project.

## Results

Implemented 2026-09-16.

**The four strategies**, in `crates/tinkerpad-app/src/lsp.rs`:

| `[lsp].workspace` | Root | One server per |
|---|---|---|
| `none` | none; no `rootUri`, no `workspaceFolders` | language |
| `playground_dir` | the playground's folder | playground |
| `library_root` | the library | language |
| `scratch_workspace` | `<app-data>/scratch/<plugin-id>/lsp` | language |

The pool keys on `(language, root)`, so `playground_dir` gives one server per
playground and the rest share. A root that doesn't exist yet is created, and a
server whose workspace is one playground's folder can't be pre-warmed — there
is nothing to root it at until a tab is open.

**`[lsp.register]`** — how a shared server learns what it is serving

```toml
[lsp.register]
setting = "rust-analyzer.linkedProjects"
value = "{dir}/Cargo.toml"
reload = "rust-analyzer/reloadWorkspace"   # optional
```

Each attached playground contributes its `value`; the whole list is sent
whenever it changes. Getting a server to act on it took three things:

1. **At startup** the list goes into `initializationOptions` *without its
   leading namespace* — that field is a server's own settings, so
   `rust-analyzer.linkedProjects` is sent as `linkedProjects`. The pool
   therefore records a tab's registration before starting the server for it
   (`ServerHandle::prepare` / `begin`).
2. **Afterwards** each change is a `workspace/didChangeConfiguration`, and a
   server that asks instead (`workspace/configuration`) is answered from the
   same place.
3. **Then the reload**, if the plugin names one — sent *after* the server has
   read the new configuration. The core waits for the server's configuration
   request first, with a two-second cap, because reloading before it has read
   the change reloads the old list.

A restart replays all of this before the documents (02-05).

**Two things had to be right for rust-analyzer to serve a shared workspace**,
and both cost an afternoon to find:

- **The workspace root is `<app-data>/scratch/<plugin-id>/lsp`, beside the
  plugin's scratch folder rather than inside it.** Rust playgrounds share a
  cargo target directory in that scratch folder; a server rooted above it
  indexes every build artefact in it.
- **rust-analyzer gets its own `cargo.targetDir`.** Otherwise it and the run
  button take turns holding cargo's lock on the shared target folder, and a
  server that loses reports "failed to fetch workspace" and then serves
  nothing at all.

With those two, a configuration change plus the reload request is enough: a
second playground opened while the first is still open gets completions in
2.4 s, from the same process.

**`[lsp].workspace_files`** is new in the manifest: files the server's workspace
needs, written there the first time it starts and never overwritten. Same shape
as `[scaffold].files`, but relative to the workspace rather than a playground.

**The plugins**

- **Rust:** `scratch_workspace`, `linkedProjects`, and the reload request. One
  rust-analyzer serves every Rust playground; opening a second adds it without
  a restart, closing a tab takes it out. Nothing holds a playground's folder as
  its workspace any more.
- **TypeScript:** `library_root` with `tsconfig.json` and `package.json` written
  into the library the first time the server starts. The server needs a
  TypeScript of its own to analyse with and looks for one in the workspace, so
  the README says to run `npm install` there once; without it the server reports
  "Could not find a valid TypeScript installation" and the editor carries on
  without completions.
- **C#:** no language server yet — see the deviation below.

**Measurements** (Windows 11, release build)

| What | Result |
|---|---|
| Rust: first completion in a fresh playground, cold server | 3.4 s |
| Rust: first completion in a *second* playground, server already up | 2.4 s |
| Rust: warm completion, keystroke to popup | median 117 ms |
| TypeScript: first completion in a fresh playground | 0.4 s |
| TypeScript: warm completion, keystroke to popup | median 109 ms |
| Second Rust tab attaching to the running server | 19 ms |

**Tests** — 6 against real processes, 1 unit, and two in-app scenarios.

- The fake server reports the root and workspace folders it was given, so each
  strategy is checked against what the server actually sees, including `none`
  meaning none.
- Two playgrounds on a shared server are both registered, in a stable order,
  with the same process throughout; closing one tab takes its project out.
- A server that asks for its configuration is answered the same way it would be
  told.
- The registrations survive a restart.
- A plugin that registers nothing sends nothing.
- In the app: the `lsp` scenario now checks that both Rust playgrounds are
  served by one process rooted at TinkerPad's own workspace, and that the second
  playground gets completions of its own. A new `lsp-typescript` scenario checks
  the library-rooted server, the `tsconfig.json` written for it, and completions
  on an array. CI installs `typescript-language-server` and a TypeScript for it.

**Docs** — ARCHITECTURE.md §4.4 describes the strategies and the three-step
registration; PLUGINS.md §2.1 documents `workspace_files` and
`[lsp.register].reload`; both plugin READMEs explain what to install and what
TinkerPad writes where.

## Checklist status

**Deviation: C# has no language server.** Both options want a project file in
the playground folder, which is exactly what `dotnet run --file main.cs` must
not have: `csharp-ls` needs a `.csproj`, and Roslyn's
`Microsoft.CodeAnalysis.LanguageServer` is a NuGet-delivered binary with no
supported standalone install. 03-05 already builds a Roslyn host to *run* C#,
and it can answer LSP requests from the same compilation, so that is where the
C# server belongs. PLAN.md's decision log records it. No `[toolchain.acquire]`
was added, since nothing is being downloaded.

**Open: diagnostics after heavy churn in one session.** Open, edit, and delete
a dozen Rust playgrounds — what the `rust` self-test scenario does — and the
shared rust-analyzer afterwards serves completions but stops publishing
diagnostics until the app restarts. Completions, hover, and the rest are
unaffected, and a fresh session is fine, so the language-server scenarios now
run in a launch of their own (as the session tests already do). Worth its own
item: the likely cure is dropping a crate from `linkedProjects` when its
playground is deleted rather than only when its tab closes.

**Finding: deleting a playground can fail while a server watches it.** A
language server keeps the folders it has loaded open, and Windows refuses to
move a folder to the trash while something holds it — so deleting a Rust
playground fails until the server stops, even now that it isn't rooted there.
Worth its own work item in Phase 4 (close the documents, unregister, and retry
before giving up); the self-tests report it rather than failing on it.
