# TinkerPad — Plan

A LINQPad-style desktop playground, written in Rust, where each language is a plugin.
Open a tab, pick a language, type code, hit Run, see output. Language servers and
compilers stay warm between runs so the loop feels instant.

This document is the plan of attack: vision, guiding principles, the big decisions,
and a phased roadmap. The design itself lives in:

- `docs/ARCHITECTURE.md` — process model, crate layout, LSP keep-alive, execution.
- `docs/PLUGINS.md` — the plugin contract, manifest format, and notes per language.

Treat all three as living documents. When a decision changes, update the decision
log at the bottom of this file rather than rewriting history.

---

## 1. Vision

**What it is:** a scratchpad for code. Not an IDE, not a notebook. One file per
playground, one Run button, a fast output pane, and good editor assistance
(completion, diagnostics, hover) from a real language server.

**What LINQPad gets right that we want to keep:**

- Zero ceremony. No "create project" step. New tab → code → run.
- Three flavours of playground: *expression*, *statements*, *program*.
- `Dump()` — rich, structured output instead of `println` soup.
- Fast iteration because the compiler process stays alive.
- A library of saved playgrounds you can search and reopen.

**What we add:** many languages behind one consistent UI, each one a plugin that
anyone can write without touching the Rust core.

**Three things are load-bearing and non-negotiable:**

1. **Fast.** Launch under half a second, completions from a warm server, runs that
   don't pay a process-startup tax where the language allows it.
2. **`Dump()` everywhere.** Rich structured output is a core protocol from Phase 1,
   not a later feature. Every language plugin ships a helper that speaks it.
3. **Community plugins.** One folder per language, loadable at runtime, written in
   the language being added (not Rust). The core must never need a rebuild to gain
   a language.

**Deferred, but designed for:**

- Debugging via the Debug Adapter Protocol, at the very end (Phase 6). Earlier
  phases carry constraints so this stays possible; see `docs/DEBUGGING.md`.

**Non-goals (for now, to stay simple):**

- No multi-file projects. One playground = one source file (plus a deps manifest).
- No cloud sync, accounts, or collaboration.
- No hosted plugin registry until there are third-party plugins to list. Plugins
  install from a folder or a git URL first.

## 2. Guiding principles

1. **Warm everything.** Language servers, compilers, and runtimes are spawned once
   and reused. Cold start is a bug to be engineered away, per language.
2. **Plugins are data plus, at most, one helper process.** A language plugin is a
   folder: a TOML manifest, templates, a `Dump()` helper library, and optionally one
   executable (written in that language) that speaks a small JSON-lines protocol for
   warm execution and any logic the manifest can't express. No Rust, no ABI, no
   rebuild. A contributor who knows Kotlin can add Kotlin.
3. **The core never knows about a specific language.** If `tinkerpad-core` mentions
   Cargo or NuGet, something is wrong. Bundled plugins use exactly the same folder
   format as third-party ones; there is no privileged path.
4. **Plain files on disk.** Playgrounds are ordinary source files in ordinary
   folders. Git-friendly, greppable, survivable if the app dies.
5. **Simple over clever.** Prefer a subprocess and JSON lines over an in-process
   ABI. Prefer one well-understood thread model over a fancy one.
6. **Cross-platform from the first commit.** Windows, macOS, and Linux, on x64 and
   arm64, are all first-class. CI builds and tests on all three from Phase 0.
   Platform-specific code lives behind one function per concern (process kill,
   PATH discovery, modifier key), never scattered `cfg` blocks. See §3.6.

## 3. The big decisions

### 3.1 UI framework — Tauri 2 + CodeMirror 6 (recommended), pure-Rust as the alternative

The single most expensive piece of UI in this app is the code editor: syntax
highlighting, completion popups, diagnostic squiggles, hover cards, signature help,
multi-cursor, bracket matching. That problem is solved on the web and is not solved
in any pure-Rust widget toolkit today.

| Option | Editor story | LSP client | Perf/size | Verdict |
|---|---|---|---|---|
| **Tauri 2 + CodeMirror 6** | Excellent, mature | `@codemirror/lsp-client` (official) | Small binary, WebView2 is built into Win11 | **Recommended** |
| Tauri 2 + Monaco | Excellent (VS Code's editor) | `monaco-languageclient` | Heavier bundle than CM6 | Fine fallback if CM6 lacks something |
| egui | Basic text widget (`egui_code_editor`) | Build it yourself | Tiny, fast | Pure Rust, but you own the editor problem |
| iced | Basic `text_editor` widget | Build it yourself | Small | Same trade-off as egui |
| GPUI (Zed's toolkit) | Zed's editor is not a reusable crate | Build it yourself | Excellent | Powerful but steep; API still moving |
| Floem (Lapce's toolkit) | Has an editor view | Partial | Good | Smaller community; risky as a foundation |

The honest framing on "written in Rust for performance": every part of this app
that has a performance story is Rust regardless of the UI choice: process
supervision, LSP multiplexing, output streaming, file watching, the plugin host.
The webview only renders text, which it does extremely well. VS Code is Electron;
LINQPad is WinForms. Neither is slow because of the shell.

If the pure-Rust itch wins, the architecture below still holds: the UI is behind a
narrow command/event boundary, so swapping the shell later is contained work. The
cost is building editor assistance UI by hand. Recommendation: **start with Tauri +
CM6, revisit only if the webview actually annoys you.**

### 3.2 Plugin model — a folder per language, manifest-driven, optional helper process

Because community contributions are a goal, the plugin boundary is decided now and
it must be crossable without Rust.

| Option | Pros | Cons |
|---|---|---|
| In-tree Rust crates implementing a trait | Simplest for the author of the core | Every new language is a PR to the core and a rebuild; contributors must know Rust |
| `cdylib` + `abi_stable` | Native speed, dynamic loading | Fragile; Rust ABI pain; still Rust-only |
| WASM plugins (wasmtime / extism) | Sandboxed, stable ABI, Zed's approach | Plugins can't spawn processes, so the host proxies everything; contributor still needs a WASM toolchain |
| **Manifest folder + optional helper executable over JSON lines** | Zero Rust for contributors; helper is written in the language being added; same mechanism as the warm runner | Not sandboxed (a plugin is trusted code, like an editor extension); one extra process per language that needs one |

Decision: **a plugin is a folder.** `plugin.toml` declares everything the core needs
to know: toolchain probes, playground kinds, templates, the run command, the LSP
command and workspace strategy, dependency handling, and how to register a playground
with a shared server. Roughly 80% of languages need nothing else. Languages that need
warm execution or custom logic ship one helper executable, in their own language,
that speaks the `tinkerpad-plugin-v1` JSON-lines protocol. That single protocol
covers both the warm runner ("run this") and the hooks ("a playground was created,
what files should exist?"). Bundled plugins are the same folder format, shipped
inside the app.

What this buys: the C# plugin's helper is a C# program using Roslyn; the Python
plugin's helper is a Python script; a contributor never opens the Rust workspace.
What it costs: no sandbox. Plugins are trusted, like VS Code extensions. That is
the right trade for a community of hobbyists adding languages they love.

Full contract in `docs/PLUGINS.md`.

### 3.3 Keeping language servers alive

The Rust core owns an **LSP pool**: one server process per (language, workspace),
started lazily on first use, shared by every open tab of that language, kept alive
when tabs close, restarted on crash, and optionally pre-warmed at app start for
pinned languages. The editor never talks to a server directly; the core proxies
every message, which lets it own request ids and rewrite URIs. Full design in
`docs/ARCHITECTURE.md` §4.

### 3.4 Keeping compilers/runtimes alive

Same idea, applied to execution. A plugin provides a **runner**, which is either
*per-run* (spawn `node main.ts`, done) or *persistent* (the plugin's helper process
accepts code over stdin and executes it in-process, like LINQPad's Roslyn host).
Per-run is what Phase 1 ships; persistent runners come in Phase 3 for the languages
that need them, and they reuse the plugin helper protocol rather than a second one.
Design in `docs/ARCHITECTURE.md` §5.

### 3.4a `Dump()` is a core protocol

Rich output is how LINQPad stops being a terminal, so it is designed and versioned
now. Programs write a `RichValue` JSON document inside an OSC escape sequence on
stdout; the core parses it and the UI renders tables, trees, and values. The schema
(`tinkerpad-dump-v1`) is part of the plugin contract, and every plugin ships a helper
library for its language. Per-run and persistent runners both carry it because it
rides on stdout. Schema in `docs/PLUGINS.md` §6.

### 3.5 Playground storage

One folder per playground under a user-chosen library root (default
`~/TinkerPad/`):

```
~/TinkerPad/
  fizzbuzz/
    playground.toml      # language, kind, created, tags
    main.cs              # the code
  json-explore/
    playground.toml
    main.ts
    package.json         # optional, plugin-owned deps manifest
```

A folder (rather than a bare file) means dependency manifests and plugin scratch
state have a home, and multi-file playgrounds are possible later without a
migration. Language plugins own the toolchain-level scratch space (e.g. a shared
Cargo target dir) under the app data dir, never inside the library.

### 3.6 Platforms

All three desktop platforms are supported equally in intent. In practice you can
only verify what you can run, so the tiers are about verification, not design:

| Tier | Platforms | Meaning |
|---|---|---|
| Verified | Windows 11 x64 | Developed and used daily here |
| CI-verified | macOS arm64, Linux x64 | Built and tested in CI on every push; webview feel checked on a real machine before each release |
| Best effort | Windows arm64, Linux arm64, macOS x64 | Built in CI; no manual checks |

What differs per platform and where the plan handles it:

| Concern | Windows | macOS | Linux | Handled in |
|---|---|---|---|---|
| Webview | WebView2 (built in) | WKWebView (built in) | WebKitGTK 4.1 (system package) | 00-02, 04-06 |
| Children die with the app | Job object | Process group; no parent-death signal, so a hard kill can orphan; mitigated by a pid file sweep at next launch | Process group + `PR_SET_PDEATHSIG` | 00-04 |
| Console window flash | `CREATE_NO_WINDOW` | n/a | n/a | 00-04 |
| Finding toolchains | `PATH` + `PATHEXT` | GUI apps don't get the shell `PATH`; query the login shell | Same, milder | 01-03 |
| Toolchain names | `python`, `py`, `pwsh`/`powershell` | `python3`, `pwsh` optional | `python3`, `pwsh` optional | 01-03, 03-04, 04-07 |
| Modifier key | Ctrl | Cmd | Ctrl | 00-03, 04-03 |
| App data dir | `%APPDATA%` | `~/Library/Application Support` | `$XDG_DATA_HOME` | `directories` crate |
| Packaging | NSIS | `.dmg`, signed + notarized | AppImage + `.deb` | 04-06 |
| File watching | `ReadDirectoryChangesW` | FSEvents (coalesces) | inotify (watch limits) | 01-04 |
| Line endings | Write LF everywhere; `.gitattributes` | | | 00-01 |

## 4. First languages

Chosen by what is installed here today and by how cheaply each proves out a
different part of the design.

| Order | Language | Why first | Runner (Phase 1) | LSP |
|---|---|---|---|---|
| 1 | **TypeScript / JavaScript** | Node 24 runs `.ts` directly (type stripping). Instant runs, zero build. Proves the whole loop fast. | `node main.ts` | `typescript-language-server` (or Deno's built-in LSP) |
| 2 | **C#** | The LINQPad heritage. .NET 10 file-based apps: `dotnet run main.cs` with `#:package` directives for NuGet. | `dotnet run main.cs` | Roslyn LSP (`Microsoft.CodeAnalysis.LanguageServer`); `csharp-ls` as fallback |
| 3 | **Rust** | Dogfooding. Forces the "shared scratch workspace" and warm `target/` design. | `cargo run` in a per-playground crate with shared target dir | `rust-analyzer` with `linkedProjects` |
| 4 | Python | Trivial persistent runner (an `exec` loop). Not installed yet. | `python main.py` | `basedpyright` or `pyright` |
| 5 | Go | Cheap once Rust's scratch-workspace pattern exists. | `go run .` | `gopls` |

Then whatever you feel like: Zig, Kotlin (`kotlinc -script`), F#, Lua, shell.

## 5. Roadmap

Each phase ends with something you can actually use. Don't start the next phase
until the current one feels good in daily use.

### Phase 0 — Skeleton (get a pixel on screen and a process running)

- Cargo workspace with the crate layout from `docs/ARCHITECTURE.md` §2.
- Tauri 2 shell with a CodeMirror 6 editor and an output pane. No language logic.
- A single hard-coded "run this command, stream stdout/stderr to the pane" path,
  with cancel. This is the `tinkerpad-runner` crate's first real code.
- Child processes die with the app (Windows job object; process group on Unix).
- `tracing` logging to a file from day one.

Done when: you can type `console.log(1)` into the editor, press Ctrl+Enter, and see
`1` appear, and killing a `while(true){}` works.

### Phase 1 — Plugin folders, first three languages, per-run execution, `Dump()`

- `tinkerpad-plugin`: `plugin.toml` schema and loader, plugin discovery from the
  bundled folder and the user plugins folder, toolchain probing.
- Toolchain detection with friendly "install X" messages when a runtime is missing.
- Playground library: create, open, rename, delete, list. Folder-per-playground.
- TypeScript, C#, Rust plugins as pure manifest folders with per-run runners.
- `tinkerpad-dump-v1`: OSC sentinel parser in the core, `RichValue` schema, table
  and tree renderers in the UI, and a helper library for each of the three languages.
- Tabs. Save on run and on a debounce. Unsaved-changes indicator.

Done when: `dump(x)` renders a table in three languages and you stop opening LINQPad
or a terminal for quick experiments.

### Phase 2 — Language servers, kept alive

- `tinkerpad-lsp`: JSON-RPC transport over stdio, the server pool, the proxy that
  sits between CM6 and the server (URI rewriting, request id ownership).
- Lifecycle: lazy spawn, keep-alive across tab close, crash restart with backoff,
  configurable idle timeout, pre-warm pinned languages at launch.
- Diagnostics, completion, hover, signature help, go-to-definition (opens read-only).
- Rust: one `rust-analyzer` with `linkedProjects` updated as playgrounds open.
- A status strip: which servers are up, memory, restart button, log viewer.

Done when: editing feels like VS Code for those languages and the second tab you
open in a language gets completions immediately.

### Phase 3 — Plugin helper processes and warm runners

- `tinkerpad-plugin-v1` helper protocol (JSON lines over stdin/stdout, see
  `docs/PLUGINS.md` §5): lifecycle, `run`/`cancel`, and hook messages.
- Helper supervision in the core, sharing the pool code with LSP servers.
- C#: a small Roslyn scripting host shipped with the plugin. This is the LINQPad
  trick and where C# goes from ~1.5s per run to ~50ms.
- Python: an `exec`-loop helper. TypeScript: keep a Node worker warm.
- Rust: the helper is unnecessary; the shared target dir does the job.
- Run timing, exit code, and a run history per playground.

Done when: C# runs feel instant and the Python helper is under 60 lines.

### Phase 4 — Polish and daily-driver features

- Dependencies per playground: NuGet, npm, crates.io via the plugin's deps manifest,
  with a simple "add package" UI that just edits the manifest.
- Search across the library, tags, recently used, pinned.
- Settings UI: library path, toolchain paths, LSP idle policy, keybindings, theme.
- Stdin support for interactive programs (PTY via `portable-pty` if needed).
- Auto-update, installer, app icon, the boring stuff.

### Phase 5 — Community

Plugins have been loadable from a folder since Phase 1; this phase is about making
it pleasant for someone who isn't you.

- `tinkerpad plugin check <dir>`: a CLI conformance tool that validates the manifest,
  probes the toolchain, scaffolds a playground, runs hello-world, and asserts a
  `dump()` round-trip. Contributors run it locally; CI runs it on bundled plugins.
- `tinkerpad plugin install <git-url|path>` and a plugins panel in the UI.
- A `tinkerpad-plugin-template` repo: manifest, templates, helper skeleton, the
  check tool wired into CI.
- Protocol and schema docs published from the same source as `docs/PLUGINS.md`,
  with explicit versioning and a deprecation rule (the core supports the current
  and previous protocol version).
- A registry index later: a JSON file in a GitHub repo listing plugin git URLs, the
  way Zed's extension index started. No hosting.

### Phase 6 — Debugging

Deferred to the end by choice. The design sketch and the constraints earlier
phases must respect are in `docs/DEBUGGING.md`. Work items are written when the
phase starts. Expected shape: a DAP client and adapter pool reusing the LSP
framing and the shared supervision code, breakpoint gutter and three bottom
panels in the UI, then one adapter integration per language, Node first.

## 6. Risks and open questions

- **Roslyn LSP for file-based apps.** The Roslyn server supports `.cs` files
  without a project as of .NET 10, but it is distributed awkwardly (Azure DevOps
  NuGet feed, no `dotnet tool`). Fallback: generate a minimal `.csproj` per
  playground so any C# server works.
- **rust-analyzer memory.** A single RA instance for all Rust playgrounds is right,
  but it will sit at hundreds of MB. Idle shutdown policy matters here.
- **Wrapper code.** Kinds like `expression` need a prelude. Rather than hide it and
  rewrite every LSP position, the prelude is stored in the file and shown as locked,
  dimmed lines in the editor. Slightly noisier than LINQPad, zero position bugs.
- **Windows process hygiene.** Killing a child tree, avoiding console window
  flashes (`CREATE_NO_WINDOW`), and UTF-8 output all need deliberate handling.
- **macOS GUI PATH.** Apps launched from the Dock don't see Homebrew, nvm, or
  dotnet. Without login-shell PATH discovery every toolchain shows as missing on
  a Mac. This is the most common cross-platform bug in this class of app.
- **macOS orphans.** No parent-death signal exists, so a force-quit can leave a
  language server running. Mitigation is a pid registry swept at next launch.
- **macOS signing.** An unsigned, un-notarized app is blocked by Gatekeeper with a
  scary dialog. Public macOS releases need an Apple developer account.
- **WebKitGTK.** Linux's webview lags the other two and has compositing problems
  on some drivers. Launch time and editor rendering must be measured on Linux, not
  assumed from Windows numbers.
- **Editor choice regret.** Mitigated by the narrow UI boundary; see §3.1.
- **Plugins are trusted code.** A plugin's helper runs with the user's privileges.
  Fine for bundled and hand-installed plugins; if a registry ever exists, it needs
  at least a review step. Sandboxing is a non-goal.
- **Protocol churn.** Once third-party plugins exist, `plugin.toml` and the helper
  protocol are public APIs. Version both from the first release and never break
  the previous version without a migration note.
- **Should the helper protocol also carry stdin?** Decide when Phase 4 starts.

## 7. Decision log

| Date | Decision | Why | Revisit when |
|---|---|---|---|
| 2026-09-13 | Tauri 2 + CodeMirror 6 for the shell | Editor assistance UI is the expensive part; solved on the web | The webview becomes an actual annoyance |
| 2026-09-13 | ~~In-tree trait plugins driven by TOML manifests~~ | Superseded the same day | — |
| 2026-09-13 | Plugin = folder with `plugin.toml` + optional helper executable over JSON lines; no Rust required | Community contributions are a stated goal, so the boundary must be crossable without Rust | A real need for sandboxing appears |
| 2026-09-13 | `Dump()` (`tinkerpad-dump-v1`) is a Phase 1 core protocol | It's the feature that makes this LINQPad rather than a terminal | — |
| 2026-09-13 | Folder-per-playground under a library root | Room for deps manifests; git-friendly; no future migration | Never, probably |
| 2026-09-13 | TS → C# → Rust as the first plugins | Installed here; each proves a different design corner | — |
| 2026-09-13 | Rust core proxies all LSP traffic | Enables keep-alive, sharing, URI rewriting, id ownership | — |
| 2026-09-13 | Preludes are locked lines in the file, never hidden | Removes all LSP position rewriting; the server sees what the editor shows | Users hate the dimmed lines |
| 2026-09-14 | Windows, macOS, Linux all first-class; CI on all three from Phase 0 | Supporting as many platforms as possible is a stated goal; platform bugs are cheap early and expensive late | — |
| 2026-09-14 | Debugging deferred to Phase 6, with 15 named constraints on earlier phases (`docs/DEBUGGING.md`) | Wanted eventually; the constraints are cheap now and a rewrite later | — |
| 2026-09-14 | `tinkerpad-library/src/platform.rs` joins the modules allowed to use platform `cfg` | Moving playgrounds to the macOS Trash without a Finder automation prompt needs a macOS-only API; the concern belongs to the library, not to process handling | — |
| 2026-09-15 | `serde_json` is built with `preserve_order` across the workspace | Dumped objects must show fields in the order the program wrote them; the default sorted map reorders them on the way through the core | JSON handling ever shows up in a profile |
| 2026-09-15 | The TypeScript plugin resolves `"tinkerpad"` with a `--require` preload that registers a module resolve hook, and needs Node 22.7+ | `NODE_PATH` is ignored for ES modules and `--import` rejects Windows paths; `--experimental-transform-types` first shipped in 22.7 | Node makes type transforms stable, or a helper runner (03-06) takes over |
| 2026-09-15 | The C# dump library is compiled into each playground by a scaffolded `Directory.Build.props` reading `TINKERPAD_CSHARP_DUMP`, which `[run].env` sets | `#:project` and a local NuGet source both need an install path written into the user's file; a props file plus an environment variable needs none and keeps `main.cs` a plain file-based app | The LSP (02-07) can't see the variable, or .NET adds an include directive |
| 2026-09-15 | Templates get `{{folder}}`, `{{package}}`, `{{plugin_dir}}`, and `{{scratch}}`; absolute paths used are recorded in `playground.toml` under `[plugin.<id>]` | Rust playgrounds are real crates that need absolute paths at creation (for rust-analyzer too); recording them makes a later repair possible when the app moves | A plugin needs paths that change per run rather than per playground |
| 2026-09-16 | The editor's LSP client owns document sync, version numbering included; the core watches the messages go past and keeps the text | A server echoes the version back in `publishDiagnostics`, and a client drops diagnostics whose version it didn't send — so a core-owned counter silently threw diagnostics away. This reverses 02-02's "never trust the editor's number" | Two editors ever share one document, which tabs don't || 2026-09-16 | C# gets no language server until the Roslyn helper in 03-05; TypeScript and Rust ship one now | Both available C# servers want a project or solution file in the playground folder, which is exactly what `dotnet run --file main.cs` must not have: csharp-ls needs a `.csproj`, and Roslyn's `Microsoft.CodeAnalysis.LanguageServer` is a NuGet-delivered binary with no supported standalone install. 03-05 already builds a Roslyn host for running code, and it can answer LSP requests from the same compilation | The Roslyn language server ships as a normal tool, or file-based apps gain project-less analysis |
| 2026-09-16 | C# gets no language server until the Roslyn helper in 03-05; TypeScript and Rust ship one now | Both available C# servers want a project or solution file in the playground folder, which is exactly what `dotnet run --file main.cs` must not have: csharp-ls needs a `.csproj`, and Roslyn's `Microsoft.CodeAnalysis.LanguageServer` is a NuGet-delivered binary with no supported standalone install. 03-05 already builds a Roslyn host for running code, and it can answer LSP requests from the same compilation | The Roslyn language server ships as a normal tool, or file-based apps gain project-less analysis |
| 2026-09-29 | Release bundles are not code-signed: unsigned on macOS and Windows, with the right-click → Open workaround documented | There is no Apple Developer account and no Windows certificate to sign with; shipping unsigned now beats not shipping, and the release workflow reads signing secrets that simply aren't set yet | A certificate exists, or SmartScreen/Gatekeeper friction costs more than the certificate |
| 2026-09-29 | Updates come from GitHub releases through Tauri's updater, checked once five seconds after the window appears, installed only when the user clicks | A release manifest on the repository's own releases needs no server and no account; checking after the window means an update never competes with a launch, and asking first means the app never restarts itself while someone is typing | Releases move off GitHub, or a security fix needs to be forced |
