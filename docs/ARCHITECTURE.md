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
struct ServerKey { language: String, root: Option<PathBuf> }

struct LspPool {
    servers: Mutex<HashMap<ServerKey, Arc<ServerHandle>>>,
    events: Option<mpsc::UnboundedSender<CoreEvent>>,   // state changes for the UI
}

struct ServerHandle {
    state: watch::Sender<ServerState>,   // Stopped | Starting | Ready | Crashed | Stopping
    sender: Option<Sender>,              // into the transport's writer
    kill: Option<KillHandle>,
    pending: Mutex<HashMap<i64, oneshot::Sender<RawMessage>>>,
    open_docs: Mutex<HashMap<String, DocState>>,   // uri -> version + text, for replay
    from_server: broadcast::Sender<RawMessage>,    // notifications, for the proxy
}
```

The pool is told what to start by a `ServerConfig` — language, root, a
`SpawnSpec`, `initializationOptions` — which the app builds from the plugin's
`[lsp]` section. `tinkerpad-lsp` never reads `plugin.toml` itself, so the
dependency direction in the root `Cargo.toml` holds.

`LspPool::attach(config, uri, text)` starts the server if needed, waits for the
handshake, sends `didOpen`, and returns an `Attachment`. Dropping it sends
`didClose`; that is the only thing closing a tab does to a server. The core owns
each document's version counter, because LSP requires versions to increase per
document and the editor's own numbering can't be trusted across a reload.

Rules:

- **Lazy spawn.** The first playground opened in a language spawns its server. The
  plugin's `lsp` manifest section says how (`command`, `args`, `init_options`,
  `workspace_root` strategy).
- **Shared.** Every playground of that language attaches to the same server. Opening
  a second tab is a `textDocument/didOpen`, nothing more.
- **Survives tab close.** Closing a tab sends `didClose`. The server stays up.
- **Idle policy.** A server's clock starts when its last document closes and stops
  when one opens. How long it may run down comes from the plugin's `idle_shutdown`,
  overridable per language in settings (`[lsp.rust] idle_shutdown = "30m"`), and
  `"never"` means never. The exception is a server that has never been given a
  document at all — a pre-warmed language nobody opened — which is held to an hour
  whatever the setting says. Reaching the limit is a clean `shutdown`/`exit` and a
  `Stopped` state with nothing to explain and no toast: nobody asked, so nobody is
  told. The next tab starts it again, transparently, on the same handle.
- **Memory.** Resident memory is sampled every 10 s per server and carried in the
  `Ready` state for the status strip (02-08).
- **Pre-warm.** `[lsp] prewarm = ["typescript"]` in settings starts those servers
  once the UI reports it is up — never before, so the window is never held up by one;
  measured at 399 ms to the window without pre-warm and 400 ms with. A language whose
  server lives in each playground's folder can't be pre-warmed (there is no playground
  yet) and is logged and skipped, as is a language id with no plugin.
- **Crash restart.** An exit nobody asked for marks the server `Crashed { restarts }`
  and starts it again after 1s, 2s, 4s, 8s, 16s, then 30s. Six crashes in a row (in
  ten minutes; a longer gap starts the count over) and the pool stops trying, leaving
  a `Stopped` state that says so — until someone opens a tab in that language or
  presses Restart, either of which is a person asking for it again. The handle
  survives all of this, so tabs, attachments and the proxy never learn that the
  process changed. Requests in flight get a `-32000` error response, which the
  editor treats as transient and retries on the next keystroke. After the handshake,
  every open document is sent again as `didOpen` with the text and version the editor
  last had — never the history of changes, since the text is the whole state a server
  needs. A toast says what happened; diagnostics come back on their own.
- **Clean shutdown.** `shutdown` request → `exit` notification → wait 2s → kill.
  Done concurrently for all servers on app exit, under a 3s budget for the lot:
  quitting is never held up by a wedged server.

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
the server sees exactly what the editor shows.

**How rewriting works.** A message is walked for the keys that hold URIs
(`uri`, `targetUri`, `newUri`, `oldUri`, `documentUri`, `externalUri`, and the
keys of a `WorkspaceEdit.changes` object); everything else passes through
byte-for-byte. Walking by key rather than by a path per method means a server
extension nobody here has heard of still gets its URIs translated.
`uri::REWRITTEN_METHODS` lists the methods known to carry one, and every entry
is covered by a round-trip test. A file URI that isn't an open playground —
a dependency a go-to-definition landed in — is left as a `file:///` URI, which
is how 02-09 opens it read-only.

**Ids.** The editor numbers its requests per tab, so two tabs both send id 1.
The core allocates an id of its own per request and keeps
`core_id → (tab, editor_id, method, started)`; the answer goes back to the tab
that asked, under the id it used. A tab that closes cancels what it was waiting
for (`$/cancelRequest`) and drops its entries, so a late answer goes nowhere.
A request to a server that isn't running is answered with an error rather than
left hanging.

**Questions the server asks us** — `workspace/configuration`,
`client/registerCapability`, `window/workDoneProgress/create` — are answered by
the core, not passed to the editor: they are about the session, which the core
owns. `window/showMessage` becomes a toast, `window/logMessage` a log line, and
`textDocument/publishDiagnostics` goes to the one tab that shows that document.

### 4.3 Transport

Stdio, `Content-Length` framing. One tokio task reads stdout and dispatches
(responses → `pending`, notifications/requests → editor channel). One task drains an
`mpsc` into stdin. stderr goes to the log with the server name as a span.

Three modules in `tinkerpad-lsp`, smallest first:

- `framing`: the `Content-Length` codec, and nothing else. It turns bytes into
  message bodies, tolerating headers in any order or case, a missing
  `Content-Type`, `\n` line endings, bodies split across reads, and several
  messages in one read. A header block it can't read is reported and skipped by
  scanning for the next `Content-Length`, so a server that writes a panic
  message into its stdout doesn't end the session. DAP frames the same way and
  reuses this module unchanged (`DEBUGGING.md` D4).
- `jsonrpc`: `RawMessage`, a `serde_json::Value` with the classification the
  router needs — request, response, notification — plus `id` and `method`.
  Messages stay as JSON here; only the proxy deserialises, and only the methods
  it inspects.
- `transport`: `Transport::spawn` over `tinkerpad-process`. One task reads
  stdout and publishes `Message`, `Broken`, and a final `Exited` event; one
  drains an `mpsc` into stdin; stderr goes to the log line by line under an
  `lsp{server, pid}` span. Both queues are bounded, so a stalled server slows
  its sender instead of growing a buffer.

Tests run against `tinkerpad-fakelsp`, a stand-in server that speaks real
framing and can be told to log, answer big messages, or cut its stdout in half.

### 4.4 Workspace strategies (plugin-declared)

| Strategy | Meaning | Used by |
|---|---|---|
| `playground_dir` | Server root is the playground folder; one server per playground | Fallback; wasteful |
| `library_root` | One server rooted at `~/TinkerPad/` serving all playgrounds | TypeScript (`typescript-language-server` is fine with loose files) |
| `scratch_workspace` | One server rooted at `<app-data>/scratch/<plugin-id>/`; playgrounds registered dynamically | Rust (`linkedProjects`), Go (`go.work`) |
| `none` | Server needs no root (file-based) | C# Roslyn for file-based apps |

**Registration.** A server rooted at a workspace of its own has no way to find the
playgrounds it is meant to serve, so the plugin names the setting they go in:

```toml
[lsp.register]
setting = "rust-analyzer.linkedProjects"
value = "{dir}/Cargo.toml"
reload = "rust-analyzer/reloadWorkspace"
```

Every attached playground contributes its `value` to that setting, and the whole
list is sent whenever it changes. Three things make it land:

1. **At startup** the list goes in `initializationOptions`, without its leading
   namespace — `initializationOptions` is a server's own settings, so
   `rust-analyzer.linkedProjects` is sent as `linkedProjects`. A server that reads
   its projects once, at startup, needs them there; rust-analyzer does. The pool
   therefore records a tab's registration *before* starting the server for it.
2. **Afterwards** each change is a `workspace/didChangeConfiguration`, and a server
   that asks instead (`workspace/configuration`) is answered from the same place.
3. **Then the reload**, if the plugin names one, sent after the server has read the
   new configuration — the core waits for the server's configuration request before
   sending it, with a two-second cap, because reloading first would reload the old
   list.

A restart replays all of this before the documents (02-05), so a crashed
rust-analyzer comes back knowing which crates it is serving.

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
  | `F8`, `Shift+F8` | Go to the next or previous error or warning — from the last run (01-13) or from the language server (02-04). |
  | `Mod+K Mod+I` | Hover documentation at the cursor (02-04). |
  | `Tab`, `Enter` | Accept the highlighted completion. |

- Names, languages, and kinds always come from the library listing, never from
  a copy kept by the tab.

The status bar (`ui/src/servers.ts`, work item 02-08) ends with one chip per
language server: a dot for its state, the language, and its resident memory.
Clicking one opens what is worth knowing when something is slow — uptime,
restarts, open documents, request p50/p95, the program that was started — and the
three things worth doing: Restart, Stop, and reading what the server itself said
(its stderr and `window/logMessage`s, the last 500 lines). A server that has
stopped keeps its chip only while a tab still wants it, and one that could not
start shows the plugin's install hint with a copy button. The numbers are
samples: the strip polls once a second and on each `server` event, never per LSP
message.

Language support (`ui/src/lsp.ts`, `ui/src/highlight.ts`, work item 02-04):

- **A client per tab, a server per language.** `@codemirror/lsp-client` talks over
  a `Transport` that forwards messages through a Tauri `Channel` to the core's
  proxy (02-03), which multiplexes every tab onto the one pooled server (02-02).
  Each tab's client does its own handshake; the core answers it from the
  handshake it already did, so a second tab in a language costs nothing.
- **Document sync belongs to the editor**, version numbering included: a server
  echoes those versions back with its diagnostics, and a client that sees a
  version it didn't send drops them. The core watches the sync messages go past
  and keeps the latest text for the replay in 02-05.
- **Nothing blocks the tab.** Opening a playground never waits for a server:
  the editor is usable in milliseconds and the language features appear when
  the handshake finishes, which for a cold rust-analyzer is seconds later.
- **Highlighters load per language.** `[language].highlighter` names a
  CodeMirror language package (`PLUGINS.md` §9); each is a dynamic import, so
  a grammar is downloaded the first time a playground uses it and never
  otherwise. The LSP client itself is a chunk of its own too.
- **Diagnostics** are drawn by `@codemirror/lint` (squiggle, gutter marker,
  hover text) and walked by the same `F8` that walks a run's annotations.

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
