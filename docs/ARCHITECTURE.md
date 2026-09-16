# TinkerPad — Architecture

Companion to `../PLAN.md`. This describes *how* the pieces fit. It assumes the
decisions in the plan (Tauri 2 shell, folder-per-language plugins with no Rust, Rust
core owns all processes).

## 1. Process model

```
┌─────────────────────────────────────────────────────────────────────┐
│  tinkerpad.exe (one process)                                        │
│                                                                     │
│  ┌──────────────┐   commands/events    ┌──────────────────────────┐ │
│  │  WebView2    │ ◄──────────────────► │  Rust core (tokio)       │ │
│  │  CodeMirror  │   (Tauri IPC)        │                          │ │
│  │  output pane │                      │  AppState                │ │
│  └──────────────┘                      │   ├─ Library (playgrounds)│ │
│                                        │   ├─ PluginRegistry       │ │
│                                        │   ├─ LspPool ────────┐    │ │
│                                        │   └─ HelperPool ───┐ │    │ │
│                                        └────────────────────┼─┼────┘ │
└─────────────────────────────────────────────────────────────┼─┼──────┘
                                                              │ │
              child processes (all in one job object)         ▼ ▼
   ┌────────────────┐ ┌────────────────┐ ┌───────────────┐ ┌─────────────────────┐
   │ rust-analyzer  │ │ typescript-ls  │ │ roslyn LSP    │ │ csharp plugin helper│
   │ (kept alive)   │ │ (kept alive)   │ │ (kept alive)  │ │ (warm runner+hooks) │
   └────────────────┘ └────────────────┘ └───────────────┘ └─────────────────────┘
                                                            ┌─────────────────┐
                                                            │ node main.ts    │
                                                            │ (per-run)       │
                                                            └─────────────────┘
```

- **One app process.** The Rust core runs a tokio multi-thread runtime. The webview
  is a client of the core, nothing more.
- **Every external tool is a child process** supervised by the core. All children
  are placed in a single Windows job object (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`) so
  nothing outlives the app. On Unix, a process group per child and `SIGTERM` the group.
- **Plugins are folders, not code in this process.** The core reads `plugin.toml`
  and, if the plugin ships a helper executable, supervises it like any other child.
  Nothing language-specific is linked into `tinkerpad.exe`.
- **Two pools with the same shape:** `LspPool` for language servers, `HelperPool`
  for plugin helper processes. Both are "keyed map of supervised child processes with
  a typed channel interface". Share the supervision code; keep the protocols separate
  (LSP is JSON-RPC with Content-Length framing; helpers are JSON lines). A third,
  `DebugAdapterPool`, arrives in Phase 6 on the same code (DAP is Content-Length
  framing with its own message shapes), which is why framing and JSON-RPC routing
  are separate modules in `tinkerpad-lsp`.

## 2. Crate layout

```
TinkerPad/
├── Cargo.toml                    # workspace
├── PLAN.md
├── docs/
├── crates/
│   ├── tinkerpad-core/           # domain types, events, errors, RichValue. No IO, no tokio.
│   ├── tinkerpad-plugin/         # plugin.toml schema + loader, discovery, toolchain probes,
│   │                             # helper protocol client (tinkerpad-plugin-v1)
│   ├── tinkerpad-process/        # child process supervision, job objects, streaming IO
│   ├── tinkerpad-lsp/            # JSON-RPC transport, LspPool, proxy/rewriting
│   ├── tinkerpad-runner/         # per-run execution, HelperPool, dump sentinel parsing
│   ├── tinkerpad-library/        # playground folders on disk, playground.toml, search
│   ├── tinkerpad-cli/            # `tinkerpad plugin check|install` (Phase 5)
│   └── tinkerpad-app/            # Tauri app: commands, event bridge, AppState wiring
├── ui/                           # frontend: CodeMirror 6, output pane, tabs. Vite + TS.
└── plugins/                      # bundled plugins. Same format as third-party ones.
    ├── typescript/               #   plugin.toml, templates/, dump/ (helper lib)
    ├── csharp/                   #   plugin.toml, templates/, dump/, helper/ (Roslyn host, Phase 3)
    └── rust/                     #   plugin.toml, templates/, dump/ (a tiny crate)
```

Dependency direction is strictly downward:

```
tinkerpad-app
  └─► tinkerpad-plugin ───► tinkerpad-process ───► tinkerpad-core
  └─► tinkerpad-lsp ──────► tinkerpad-process
  └─► tinkerpad-runner ───► tinkerpad-process, tinkerpad-plugin
  └─► tinkerpad-library ──► tinkerpad-core
```

There is no per-language Rust code anywhere in the workspace. The `plugins/` folder
is copied into the app bundle at build time and loaded at runtime exactly the way a
user-installed plugin in `<app-data>/plugins/` is. If a bundled plugin ever needs
something the manifest and helper protocol can't express, the fix is to extend the
protocol, never to special-case the language in Rust.

`tinkerpad-core` must compile without tokio or Tauri; it holds the types that the
UI, the CLI, and the docs generator all share.

### Key crates (Rust)

| Concern | Crate | Notes |
|---|---|---|
| Async runtime | `tokio` | `process`, `io-util`, `sync`, `time` features |
| Shell | `tauri` 2 | plus `tauri-plugin-shell` is *not* used; we own processes |
| Serialization | `serde`, `serde_json`, `toml` | |
| LSP types | `lsp-types` | Don't hand-write these |
| JSON-RPC framing | hand-rolled (~150 lines) or `async-lsp` | Content-Length framing is trivial; the proxy needs raw access anyway |
| Windows job objects | `win32job` or `windows` crate directly | |
| PTY (Phase 4) | `portable-pty` | Only if interactive stdin/colour is wanted |
| File watching | `notify` | Library changes made outside the app |
| Paths | `directories` | App data dir for scratch workspaces |
| Logging | `tracing`, `tracing-appender` | Rolling file log |
| Errors | `thiserror` in libs, `anyhow` in app | |

### Key packages (UI)

`@codemirror/state`, `@codemirror/view`, `@codemirror/lsp-client`, language packages
per plugin for highlighting (`@codemirror/lang-javascript`, `@codemirror/lang-rust`,
`@replit/codemirror-lang-csharp`), `@tauri-apps/api`.

## 3. Core domain (tinkerpad-core)

```rust
pub struct PlaygroundId(Uuid);

pub struct Playground {
    pub id: PlaygroundId,
    pub name: String,
    pub language: LanguageId,        // "csharp", "typescript", ...
    pub kind: KindId,                // "statements", "program", "expression", plugin-defined
    pub dir: PathBuf,                // ~/TinkerPad/<name>/
    pub main_file: PathBuf,          // dir/main.<ext>
    pub created: DateTime<Utc>,
    pub tags: Vec<String>,
}

pub enum RunEvent {
    Started { run_id: RunId },
    Stdout(Bytes),
    Stderr(Bytes),
    Rich(RichValue),                 // parsed from the sentinel protocol, Phase 3
    Exited { code: Option<i32>, duration: Duration },
    Failed { message: String },      // couldn't even start
}

pub enum ServerState { Stopped, Starting, Ready, Crashed { restarts: u32 }, Stopping }
```

Events flow core → UI over a single broadcast channel, tagged with playground or
server id. The UI subscribes once and routes. Commands flow UI → core as Tauri
commands. Keep both surfaces small and boring.

## 4. Language server keep-alive (tinkerpad-lsp)

### 4.1 Pool

```rust
struct ServerKey { language: LanguageId, workspace_root: PathBuf }

struct LspPool {
    servers: HashMap<ServerKey, Arc<ServerHandle>>,
    policy: IdlePolicy,              // Never | After(Duration) | MemoryAbove(bytes)
}

struct ServerHandle {
    state: watch::Sender<ServerState>,
    child: Supervised<Child>,        // from tinkerpad-process
    outbound: mpsc::Sender<RawMessage>,
    pending: Mutex<HashMap<RequestId, oneshot::Sender<Response>>>,
    open_docs: Mutex<HashMap<Url, DocState>>,   // for replay after restart
    last_activity: AtomicInstant,
}
```

Rules:

- **Lazy spawn.** The first playground opened in a language spawns its server. The
  plugin's `lsp` manifest section says how (`command`, `args`, `init_options`,
  `workspace_root` strategy).
- **Shared.** Every playground of that language attaches to the same server. Opening
  a second tab is a `textDocument/didOpen`, nothing more.
- **Survives tab close.** Closing a tab sends `didClose`. The server stays up.
- **Idle policy.** Configurable. Default: keep forever, except rust-analyzer which
  defaults to "stop after 30 min idle" because of memory. The status strip shows RSS.
- **Pre-warm.** Languages the user pins are spawned at app start on a background task,
  so the first tab is already fast.
- **Crash restart.** On unexpected exit: mark `Crashed`, wait with exponential backoff
  (1s, 2s, 4s, cap 30s, give up after 5), respawn, `initialize`, replay `didOpen` for
  every doc in `open_docs`. UI shows a toast; diagnostics come back on their own.
- **Clean shutdown.** `shutdown` request → `exit` notification → wait 2s → kill.
  Done concurrently for all servers on app exit.

### 4.2 Proxy: why the core sits in the middle

The editor could talk LSP straight to the server over a Tauri channel. It doesn't,
because the core in the middle buys three things:

1. **URI rewriting.** The editor sees `tinkerpad://playground/<id>`; the server sees
   the real `file:///` path in the library. Neither side knows about the other's
   naming, so renaming or moving the library never touches the editor.
2. **Request id ownership.** The core assigns ids and correlates responses, so a
   server restart never leaves the editor with dangling requests (they get an error
   response synthesised by the core).
3. **Observability.** Every message passes one point that can log, time, and count.

What the proxy deliberately does *not* do is rewrite positions. Playground kinds
that need wrapper code store it in the file as locked lines (see `PLUGINS.md` §3), so
the server sees exactly what the editor shows. The proxy has a table of methods that
carry URIs and passes everything else through byte-for-byte.

### 4.3 Transport

Stdio, `Content-Length` framing. One tokio task reads stdout and dispatches
(responses → `pending`, notifications/requests → editor channel). One task drains an
`mpsc` into stdin. stderr goes to the log with the server name as a span.

### 4.4 Workspace strategies (plugin-declared)

| Strategy | Meaning | Used by |
|---|---|---|
| `playground_dir` | Server root is the playground folder; one server per playground | Fallback; wasteful |
| `library_root` | One server rooted at `~/TinkerPad/` serving all playgrounds | TypeScript (`typescript-language-server` is fine with loose files) |
| `scratch_workspace` | One server rooted at an app-owned scratch dir; playgrounds registered dynamically | Rust (`linkedProjects`), Go (`go.work`) |
| `none` | Server needs no root (file-based) | C# Roslyn for file-based apps |

## 5. Execution (tinkerpad-runner)

### 5.1 Runner kinds

```rust
pub enum RunnerSpec {
    PerRun { command: CommandTemplate },   // spawn, stream, exit
    Helper,                                // the plugin's helper process runs it (Phase 3)
}
```

**Per-run** is the Phase 1 default and never goes away. The plugin manifest supplies
a command template with `{main}`, `{dir}`, `{scratch}` placeholders. The core spawns
it (cwd = playground dir, `CREATE_NO_WINDOW`, UTF-8 env vars like
`DOTNET_CLI_UI_LANGUAGE=en`, `PYTHONIOENCODING=utf-8`), streams stdout/stderr as
`RunEvent`s in 4 KiB chunks, and reports exit code and wall time. Cancel = kill the
process tree.

**Helper** runners are the plugin's helper process, living in `HelperPool` with the
same supervision rules as LSP servers (lazy, shared, restart on crash, idle policy).
Protocol is JSON lines over stdin/stdout; see `PLUGINS.md` §5. The helper is
responsible for isolating runs (new `AssemblyLoadContext`, fresh module scope, etc.)
and for reporting when it can't (e.g. a run that hangs forever → the core kills and
respawns the helper). If a plugin declares both `[run]` and `[helper]`, the helper is
preferred and `[run]` is the fallback when the helper is unavailable.

### 5.2 Warm compile without a helper

Some languages get most of the win from the filesystem, no helper required:

- **Rust:** every playground is its own tiny crate but `CARGO_TARGET_DIR` points at
  one shared `scratch/rust/target`. Dependencies compile once. Incremental compile of
  `main.rs` is sub-second for small programs.
- **C# with `dotnet run main.cs`:** the SDK caches the build under a temp folder keyed
  by file hash; second run of an unchanged file is fast, changed file is ~1–2s. Good
  enough for Phase 1; the Roslyn helper in Phase 3 makes it ~50ms.
- **Go:** module build cache does the same job.

### 5.3 Rich output: `tinkerpad-dump-v1` (Phase 1)

Programs emit lines like:

```
\x1b]1337;TinkerPad={"v":1,"kind":"table","columns":[...],"rows":[...]}\x07
```

An OSC escape sequence: invisible in a real terminal (terminals ignore unknown OSC
codes), trivially detected by the core's stdout parser, no chance of colliding with
user output. The core parses the JSON into `RichValue` and the UI renders it. Because
it rides on stdout, it works identically for per-run and helper runners, and a plugin
author only needs to write a small library in their language that serialises values
into the schema. The schema is versioned and documented in `PLUGINS.md` §6.

The parser is a small state machine over the stdout byte stream that must handle a
sentinel split across read chunks, a malformed payload (emit it as plain text plus a
warning, never drop it), and payload size limits (default 8 MiB; beyond that, truncate
with a marker so a `dump(hugeList)` doesn't freeze the UI).

## 6. Library (tinkerpad-library)

- Root is configurable; default `~/TinkerPad/`.
- Each subfolder with a `playground.toml` is a playground. Anything else is ignored.
- `playground.toml`:

  ```toml
  id = "01J8..."             # ULID; stable across renames
  language = "csharp"
  kind = "statements"
  created = 2026-09-13T20:11:00Z
  tags = ["json", "experiments"]

  [plugin.rust]              # optional; values a plugin wrote into its files
  plugin_dir = "C:/Program Files/TinkerPad/plugins/rust"
  ```

- The main file name is `main.<ext>` where the plugin declares `<ext>`. Deps manifests
  (`package.json`, `Cargo.toml`, `#:package` directives inside `main.cs`) are
  plugin-owned and live next to it.
- An in-memory index (name, tags, first line of code) is built at startup and kept
  fresh by `notify`. No database. If it ever needs one, SQLite via `rusqlite`, but a
  few thousand folders don't.

## 7. UI shell (ui/)

Keep it small: a tab strip over one CodeMirror view, a resizable output pane, a
language picker on new tab, a status strip (server states, run timer), and a command
palette for everything else. No settings pages until Phase 4; settings are a TOML
file until then.

Tabs (`ui/src/tabs.ts`, work item 01-12):

- **One editor view for every tab.** Switching tabs swaps `EditorState` objects,
  which carry each tab's document, undo history, and selection. Scroll positions
  are saved and restored alongside them.
- **Per tab:** an output pane, a run state, a dirty flag, and a pending autosave.
  A run's events reach its tab's pane even while another tab is active.
- **Saving:** 500 ms after the last edit, and always before a run. The core skips
  writes of identical text, so a file's modification time only changes with its
  content.
- **Changes on disk:** a clean tab reloads; a dirty one shows Reload and Keep mine.
- **The session:** open tabs, the active tab, scroll positions, and the last used
  language are kept in `<app-data>/session.json` and reopened at launch.
  `TINKERPAD_SESSION` overrides the file's location.
- **Shortcuts:**

  | Keys | Action |
  |---|---|
  | `Mod+N` | New playground in the last used language, with no questions. |
  | `Mod+Shift+N` | New playground: pick a language, then a kind. |
  | `Mod+O` | Quick open. |
  | `Mod+W` or middle-click | Close tab. |
  | `Ctrl+Tab`, `Ctrl+Shift+Tab` | Cycle tabs (Ctrl on every platform). |
  | `F2` or double-click | Rename. |
  | `F8`, `Shift+F8` | Go to the next or previous error or warning from the last run (01-13). |

- Names, languages, and kinds always come from the library listing, never from
  a copy kept by the tab.

CodeMirror's LSP client plugin is pointed at a transport object that forwards
messages over a Tauri `Channel` to the core proxy. One transport per language, shared
by tabs, matching the pool on the Rust side.

## 8. Threading and state

- All core state hangs off one `AppState` held by Tauri's managed state, containing
  `Arc`s to the pools and library.
- Pools use `tokio::sync` primitives, never `std::sync::Mutex` across awaits.
- Nothing in the core blocks: process IO is async, file IO goes through
  `tokio::fs` or `spawn_blocking`.
- One broadcast channel for core → UI events; Tauri's `emit` is called from a single
  forwarding task so event ordering is preserved.

## 9. Testing strategy

- `tinkerpad-process`: spawn `cmd /c echo`, spawn a sleeper and kill the tree, verify
  UTF-8 and chunk boundaries.
- `tinkerpad-lsp`: a fake LSP server (a tiny Rust binary in `tests/`) that echoes and
  can be told to crash; test restart + didOpen replay, and URI rewriting with a
  table-driven test over real captured messages.
- `tinkerpad-runner`: fake helper speaking the JSON-lines protocol; table-driven
  tests for the dump sentinel parser (split chunks, bad JSON, oversized payloads).
- `tinkerpad-plugin`: manifest fixtures that must load, and fixtures that must fail
  with a specific error (unknown key, missing template, bad placeholder).
- Plugins: the `tinkerpad plugin check` conformance run per bundled plugin, skipped
  when the toolchain is absent. Contributors run the same tool.
- UI: none beyond "it builds" until it hurts.

## 10. Platform notes

The summary table is in `PLAN.md` §3.6. Design rules that follow from it:

- **One module per platform concern.** `tinkerpad-process::platform` exposes
  `kill_tree`, `register_child`, `hide_console`; `tinkerpad-plugin::env` exposes
  `discover_path` and `find_program`; `tinkerpad-library::platform` exposes
  `move_to_trash` (macOS needs `NSFileManager` instead of scripting Finder). Each
  selects its implementation with `cfg`, and nothing outside those modules uses
  `cfg(target_os)`.
- **Children die with the app.** Windows: one job object. Linux: each child in
  its own process group plus `PR_SET_PDEATHSIG(SIGTERM)` set in `pre_exec`. macOS:
  process group only; additionally, every spawned pid is recorded in
  `<app-data>/children.pids`, and at launch any still-running pid from a previous
  session whose command line matches a known server or helper is terminated.
  Normal exit and panic paths kill all groups on every platform.
- **PATH discovery.** On macOS and Linux, at startup, run `$SHELL -ilc 'printf
  %s "$PATH"'` (with a 2 s timeout, falling back to the inherited PATH) and use
  the result for every program lookup. Cache it; offer "re-detect" in settings.
  Windows uses the inherited PATH plus `PATHEXT`.
- **Modifier key.** The UI has a `Mod` abstraction that renders as Cmd on macOS
  and Ctrl elsewhere. Every shortcut in the docs is written with `Mod`. Tauri's
  default macOS app menu is kept so Cmd+Q, Cmd+H, and Cmd+, behave natively.
- **Paths.** Always `PathBuf`; never string-concatenate separators. Case
  sensitivity: never rely on case-insensitive matches for playground folders.
  Write LF line endings in every generated file.
- **Webview.** Linux requires `libwebkit2gtk-4.1` and `libgtk-3` at runtime;
  the installer declares them. Known workaround for compositing glitches on some
  drivers: `WEBKIT_DISABLE_COMPOSITING_MODE=1`, exposed as a setting.
- **CI.** A matrix of `windows-latest`, `macos-latest` (arm64), and
  `ubuntu-latest` runs `cargo test --workspace`, `npm run typecheck`, and the
  plugin conformance checks with toolchains installed by the workflow. Cross
  builds for the best-effort targets run on tag only.

## 11. Things deliberately not built

Listed so future-you doesn't wonder if they were forgotten: multi-file
playgrounds, notebooks/cells, remote execution, telemetry, plugin sandboxing
(plugins are trusted, like editor extensions), a terminal emulator.

Debugging is not on this list. It is deferred to Phase 6 and designed for; the
constraints it places on everything above are in `DEBUGGING.md` §2.
