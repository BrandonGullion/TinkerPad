# TinkerPad — Language Plugins

Companion to `../PLAN.md` and `ARCHITECTURE.md`. This is the contract a plugin
author works against. It is a public API once third-party plugins exist, so it is
versioned: the manifest carries `api = 1`, the helper protocol is
`tinkerpad-plugin-v1`, the rich output schema is `tinkerpad-dump-v1`. The core
supports the current and previous version of each.

## 1. What a plugin is

**A plugin is a folder. One folder per language. No Rust.**

```
plugins/csharp/
├── plugin.toml            # everything the core needs to know (required)
├── templates/             # new-playground templates, one per kind (required)
│   ├── statements.cs
│   └── program.cs
├── dump/                  # the Dump() helper library for this language (required)
│   └── TinkerPad.Dump.cs
├── helper/                # optional: warm runner + hooks, written in C#
│   ├── helper.csproj
│   └── Program.cs
└── README.md
```

Bundled plugins and third-party plugins are the same format. The core discovers
plugins from two places: the bundled folder inside the app, and
`<app-data>/plugins/`. A user-installed plugin with the same `id` as a bundled one
overrides it.

A plugin answers these questions:

| Question | Answered by |
|---|---|
| Name, extension, how to detect the toolchain? | `[language]`, `[toolchain]` |
| What playground kinds exist and what does a new one contain? | `[[kind]]` + `templates/` |
| How do I run a playground? | `[run]` and/or `[helper]` |
| How do I start its language server and where does it want its root? | `[lsp]` |
| How does a shared server learn about a new playground? | `[lsp.register]` or a helper hook |
| Where do dependencies go and how are they added? | `[deps]` |
| How do I turn a compiler error on stderr into a clickable location? | `[output]` |
| How do values get rendered richly? | `dump/` library speaking `tinkerpad-dump-v1` |
| Anything the manifest can't express? | `helper/` process speaking `tinkerpad-plugin-v1` |

Most languages need only the manifest, templates, and dump library. The helper is
for warm execution and for the rare hook the manifest can't express.

## 2. Manifest: `plugin.toml`

Annotated example for C#. Every section other than `[language]`, `[toolchain]`,
`[[kind]]`, and one of `[run]`/`[helper]` is optional.

```toml
api = 1                              # manifest schema version

[language]
id = "csharp"                        # stable, lowercase, [a-z0-9-]
name = "C#"
extension = "cs"
highlighter = "csharp"               # CodeMirror language the UI loads; see §9 for the list

[toolchain]
# Ordered probes; first success wins. Captured {version} shows in the status strip.
# Entries may be limited to some platforms with `os = [...]`; the winning entry's
# program is available as {toolchain} in commands below.
probe = [
  { command = "dotnet", args = ["--version"], min_version = "10.0" },
]
install_hint = "Install the .NET 10 SDK: https://dot.net"

[[kind]]
id = "statements"
name = "C# Statements"
template = "statements.cs"           # in templates/; {{name}} is substituted
default = true

[[kind]]
id = "program"
name = "C# Program"
template = "program.cs"

[[kind]]
id = "expression"
name = "C# Expression"
template = "expression.cs"
# Locked lines: stored in the file, shown dimmed and read-only in the editor.
# The LSP and the runner see the real file, so no position rewriting exists anywhere.
prelude  = "TinkerPad.Dump(\n"
postlude = "\n);"

[scaffold]
# Extra files written when a playground of this language is created.
# Rust uses this for Cargo.toml and .cargo/config.toml; C# needs nothing.
files = []

[run]
# Per-run command. Placeholders: {main} {dir} {id} {name} {scratch} {plugin_dir}
command = "dotnet"
args = ["run", "{main}"]
cwd = "{dir}"
env = { DOTNET_CLI_UI_LANGUAGE = "en", DOTNET_NOLOGO = "1" }

[helper]                             # Phase 3; optional
command = "{plugin_dir}/helper/bin/tinkerpad-csharp-helper"
build = { command = "dotnet", args = ["build", "-c", "Release", "-o", "bin"], cwd = "{plugin_dir}/helper" }
hooks = ["run"]                      # which protocol messages this helper implements
idle_shutdown = "never"

[lsp]
command = "Microsoft.CodeAnalysis.LanguageServer"
args = ["--stdio", "--logLevel", "Warning"]
workspace = "none"                   # none | playground_dir | library_root | scratch_workspace
idle_shutdown = "never"
[lsp.init_options]                   # initializationOptions, passed verbatim

[deps]
kind = "inline-directive"            # C# uses `#:package Name@Version` at the top of main.cs
directive = "#:package {name}@{version}"

[output]
# Named groups: file, line, col, message. Turns stderr lines into clickable annotations.
error_regex = '^(?<file>.+?)\((?<line>\d+),(?<col>\d+)\): error \w+: (?<message>.*)$'
```

Rules:

- **One manifest works on every OS.** Probe entries and the `[run]`, `[helper]`,
  and `[lsp]` sections may carry `os = ["windows", "macos", "linux"]` and be
  repeated with different lists; the loader picks the matching one. Use
  `{toolchain}` (the program of the probe that succeeded) instead of hard-coding
  `python` vs `python3`. Never use shell syntax; there is no shell on any platform.
- Unknown keys *inside a known section* are a load error. Fail loudly on
  `idle_shutdwon`. Unknown *top-level sections* are a warning, so a plugin can
  carry a section a newer core understands (a future `[debug]`) without breaking
  on an older one. `tinkerpad plugin check` treats both as errors.
- Every command resolves against `PATH` and against user overrides in settings
  (`[toolchains.csharp] dotnet = "C:/..."`).
- Placeholders are the only templating. Args are arrays; there is no shell.
- `[lsp.register]` (see §4) covers the common "tell the shared server about this
  playground" case declaratively. Anything beyond that is a helper hook.

### 2.1 Reference

What the loader (`tinkerpad-plugin`, work item 01-01) accepts, beyond the
example above.

**Per-OS sections.** TOML can't repeat `[run]`, so write per-OS variants as an
array of tables. A single `[run]` table is still fine when one command works
everywhere.

```toml
[[run]]                      # applies wherever no specific entry does
command = "{toolchain}"
args = ["{main}"]

[[run]]
os = ["windows"]             # wins over the generic entry on Windows
command = "{toolchain}"
args = ["-X", "utf8", "{main}"]
```

The same works for `[[helper]]` and `[[lsp]]`, and probe entries take `os`
inline. For each OS, at most one entry may list it and at most one entry may
omit `os`. These rules are checked for all three operating systems, whichever
one you're on, so a mistake that only affects macOS still fails on Windows.

**Placeholders by context.** `{{` and `}}` are literal braces.

| Where | Allowed |
|---|---|
| `[run]`, `[helper]` (and its `build`), `[lsp]`, `[lsp.register].value` | `{main}` `{dir}` `{id}` `{name}` `{scratch}` `{plugin_dir}` `{version}` `{toolchain}` |
| `[toolchain].probe` | `{plugin_dir}` `{scratch}` |
| `[deps]` | `{name}` `{version}`, meaning the package being added |

**Keys not shown in the example**

| Key | Meaning |
|---|---|
| `[language].main_path` | The main file inside a playground. Default `main.<extension>`. Rust uses `src/main.rs`. |
| `[scaffold].files` | `[{ path = "Cargo.toml", template = "Cargo.toml" }]`. `path` is relative to the playground, `template` to `templates/`. |
| `[helper].args`, `[helper].env` | As for `[run]`. |
| `[lsp].args`, `[lsp].env` | As for `[run]`. |
| `[lsp].workspace` | Defaults to `playground_dir`. |
| `[deps]` with `kind = "command"` | Needs `add = ["cargo", "add", "{name}@{version}"]`. |
| `[deps]` with `kind = "inline-directive"` | Needs `directive = "#:package {name}@{version}"`. |
| `[output].error_regex_multiline` | `true` to match across up to three lines (work item 01-13). |

**Other rules**
- Exactly one `[[kind]]` has `default = true`.
- Ids (`language.id`, kind ids) use lowercase letters, digits, and hyphens.
- Paths are relative and stay inside their folder: no leading `/`, no drive
  letter, no `..`.
- `idle_shutdown` is `"never"` or a duration such as `"90s"`, `"30m"`, `"2h"`.
- `min_version` is a dotted version such as `"10.0"`.
- `[output].error_regex` must name the groups `line` and `message`. `file`, `col`,
  and `severity` are optional.
- An unknown `highlighter`, an unknown helper hook, or an unused regex group is a
  warning, not an error.
- Every problem the loader finds is reported at once, each with its key path
  (such as `kind[1].template`). Unknown keys and values come with a line,
  column, section, and a "did you mean" suggestion.

## 3. Locked lines instead of hidden preludes

LINQPad hides its wrapper code. TinkerPad doesn't. A kind with a `prelude`/`postlude`
stores them in `main.<ext>` and the editor renders those lines dimmed and read-only.
The language server, the runner, and the compiler all see exactly the file on disk.

This costs a little visual noise on the `expression` kind and removes an entire
category of bugs (off-by-N diagnostics, hover at the wrong line, positions inside a
prelude). Plugin authors don't have to think about it at all.

## 4. Shared servers and playground registration

Some servers want one instance for many playgrounds and need to be told about each
one (rust-analyzer's `linkedProjects`, gopls's `go.work`). The manifest covers the
configuration-setting case:

```toml
[lsp]
workspace = "scratch_workspace"

[lsp.register]
# On open/close, the core sends workspace/didChangeConfiguration with this setting
# holding {value} for every currently open playground of this language.
setting = "rust-analyzer.linkedProjects"
value = "{dir}/Cargo.toml"
```

If registration means writing a file (gopls's `go.work`), that is a helper hook
(`playground_opened` / `playground_closed`, §5), and the helper writes the file.

## 5. Helper protocol: `tinkerpad-plugin-v1`

A helper is any executable speaking JSON lines on stdin/stdout. stderr goes to the
log. The core starts it lazily on first need, keeps it alive under the same rules as
LSP servers, and restarts it on crash. `hooks` in the manifest lists which messages
it implements; the core never sends the others.

Every message has `type` and, for requests, `id`. Replies carry the same `id`.

**Lifecycle**

```json
→ {"type":"hello","protocol":"tinkerpad-plugin-v1","plugin_dir":"...","scratch_dir":"..."}
← {"type":"ready","name":"tinkerpad-csharp-helper","version":"0.1.0"}
→ {"type":"shutdown"}
```

**Execution** (`hooks = ["run"]`)

```json
→ {"type":"run","id":"r1","main":"C:/.../main.cs","dir":"C:/...","kind":"statements"}
← {"type":"stdout","id":"r1","data":"SGVsbG8K"}              // base64, chunked; may contain dump sentinels
← {"type":"stderr","id":"r1","data":"..."}
← {"type":"exited","id":"r1","code":0,"duration_ms":41}
← {"type":"failed","id":"r1","message":"...","diagnostics":[{"file":"...","line":3,"col":9,"message":"..."}]}
→ {"type":"cancel","id":"r1"}
```

**Hooks** (each optional; declare in `hooks`)

```json
→ {"type":"playground_created","id":"h1","playground":{...}}
← {"type":"result","id":"h1","files":[{"path":"go.mod","content":"..."}]}

→ {"type":"playground_opened","id":"h2","playground":{...},"open":[{...},{...}]}
← {"type":"result","id":"h2","files":[...],"lsp_config":{"gopls":{...}}}

→ {"type":"playground_closed","id":"h3","playground":{...},"open":[...]}
← {"type":"result","id":"h3"}

→ {"type":"add_dependency","id":"h4","playground":{...},"name":"serde","version":"1"}
← {"type":"result","id":"h4","files":[...]}
```

Rules:

- One `run` at a time per helper. Run while running = cancel the old one first.
- No `ready` within 10s of spawn → kill, fall back to `[run]`, show a warning.
- No response to `cancel` within 2s → kill and respawn. Warm state is lost; fine.
- Hooks must reply within 5s or the core proceeds as if the hook returned nothing.
- **Forward compatibility:** helpers must ignore fields they don't recognise in
  any message, and the core ignores unknown fields from helpers. New optional
  fields (a future `debug` flag on `run`, for example) are added without a
  protocol version bump; only removed or re-typed fields bump it.
- `run` has no timeout. A program runs until it exits or the user cancels.
- The helper isolates runs as well as the language allows (collectible
  `AssemblyLoadContext`, fresh module scope, fresh globals). Perfect isolation is
  not required; the restart button exists.
- If `[helper.build]` is present, the core runs it on first use and whenever the
  helper source folder changes. This is how a C# helper ships as source and a
  contributor never has to publish binaries.

## 6. Rich output: `tinkerpad-dump-v1`

A program writes one OSC sequence per value to stdout:

```
ESC ] 1337 ; TinkerPad= <json> BEL
```

The JSON:

```json
{"v":1,"kind":"table","title":"users",
 "columns":[{"name":"id","type":"int"},{"name":"name","type":"string"}],
 "rows":[[1,"ada"],[2,"grace"]],
 "truncated":false}
```

Kinds:

| kind | fields | renders as |
|---|---|---|
| `value` | `value` (scalar), `type` | inline literal with type badge |
| `table` | `columns[{name,type}]`, `rows[[...]]`, `truncated` | sortable grid; cells may themselves be `{"v":1,...}` nodes for nesting |
| `tree` | `value` (arbitrary JSON), `type` | collapsible tree, first two levels open |
| `text` | `text`, `lang` (optional) | monospace block with optional highlighting |
| `html` | `html` | sanitised HTML (no scripts, no external loads); escape hatch |
| `image` | `mime`, `base64` | inline image |
| `timing` | `label`, `ms` | small timing badge |

Optional on every node: `title` (string), `id` (string, for future lazy expansion).

**The dump library, not the core, is responsible for:** cycle detection, depth limit
(default 5), item cap (default 1000 rows, sets `truncated`), choosing `table` for
sequences of records and `tree` for everything else, and type names. The core only
parses and renders. That keeps language semantics in the language.

Each plugin's `dump/` folder ships that library. How the user gets it:

| Language | Helper | Mechanism |
|---|---|---|
| C# | `x.Dump()` extension method | `#:package` directive in the template pointing at a local source; injected as a global when the helper runs the code |
| TypeScript | `import { dump } from "tinkerpad"` | `NODE_PATH` env var pointing at `dump/` |
| Rust | `tinkerpad::dump(&x)` | Path dependency in the scaffolded `Cargo.toml`; requires `serde::Serialize` |
| Python | `from tinkerpad import dump` | `PYTHONPATH` |

The template for each kind already imports the helper, so a new playground can call
`dump` on line one.

## 7. Per-language notes

### TypeScript / JavaScript (Phase 1)

- **Run:** `node --experimental-transform-types main.ts`. Node 24 strips types
  natively; the flag adds `enum` and `namespace` support.
- **Kinds:** `statements` (an ES module, default), `program` (template with a
  `main()` call).
- **LSP:** `typescript-language-server --stdio`, workspace `library_root`, with a
  scaffolded `tsconfig.json` at the library root (`"module": "nodenext"`,
  `"allowImportingTsExtensions"`, `"noEmit"`). Deno (`deno lsp`) is a one-binary
  alternative that also runs `.ts` directly; start with Node because it's installed.
- **Deps:** `[deps] kind = "command"`, `add = ["npm", "install", "{name}@{version}"]`,
  manifest `package.json` in the playground folder.
- **Helper (Phase 3):** a Node script that `import()`s the file with a cache-busting
  query. Trivial; worth it mainly to skip Node startup.

### C# (Phase 1, helper in Phase 3)

- **Run:** `dotnet run main.cs` (.NET 10 file-based apps). `#:package Foo@1.2.3` at
  the top of the file pulls NuGet packages.
- **Kinds:** `statements` (top-level statements, default), `program`, `expression`
  (locked lines).
- **LSP:** Roslyn's `Microsoft.CodeAnalysis.LanguageServer` understands loose `.cs`
  files and file-based apps. Acquisition is the risk: it ships as a NuGet package on
  a Microsoft Azure DevOps feed, not as a `dotnet tool`. Plan: `[toolchain.acquire]`
  downloads it on first use. Fallback: `csharp-ls` (`dotnet tool install -g
  csharp-ls`), which may not understand `#:package`. Last resort: scaffold a minimal
  `main.csproj` per playground; any C# server understands that and `dotnet run`
  doesn't care.
- **Helper (Phase 3):** a small .NET console app in `helper/`, built on first use via
  `[helper.build]`. Uses `Microsoft.CodeAnalysis.CSharp.Scripting` to compile and run
  in a collectible `AssemblyLoadContext`, redirects `Console.Out`, injects `Dump()`
  as a global. This is LINQPad's engine; expect ~50ms per run after the first.

### Rust (Phase 1)

- **Layout:** each playground folder is a real crate. `[scaffold]` writes
  `Cargo.toml` (with the `tinkerpad` dump crate as a path dependency) and
  `.cargo/config.toml` setting `build.target-dir` to `{scratch}/target`, shared by
  every Rust playground so dependencies compile once.
- **Run:** `cargo run --quiet`, cwd `{dir}`. `[output].error_regex` turns rustc's
  `--> src/main.rs:3:9` lines into clickable annotations.
- **Kinds:** `program` (default; template has `fn main()`), `expression` (locked
  `fn main() { tinkerpad::dump(&(` ... `)); }`).
- **LSP:** one `rust-analyzer`, workspace `scratch_workspace`, `[lsp.register]` with
  `rust-analyzer.linkedProjects` = `{dir}/Cargo.toml`. RA reloads the workspace set
  on `didChangeConfiguration` without a restart.
- **Deps:** `[deps] kind = "command"`, `add = ["cargo", "add", "{name}@{version}"]`.
- **No helper needed.** The shared target dir is the warm cache.
- **Memory:** RA is why `idle_shutdown` exists. Default 30 minutes.

### Python (once installed)

- **Run:** `python main.py`, or `uv run main.py` when `uv` is present (inline script
  metadata per PEP 723 for deps).
- **LSP:** `basedpyright-langserver --stdio`, workspace `playground_dir`.
- **Helper:** ~40 lines: read JSON lines, `exec(compile(src, path, "exec"), {})`,
  capture stdout. The cheapest demonstration of the helper protocol; write it first
  when Phase 3 starts, and use it as the reference implementation in the plugin
  template repo.

### Go (later)

- Each playground is a module (`[scaffold]` writes `go.mod`). `go run .`. `gopls`
  with `workspace = "scratch_workspace"` and a helper implementing
  `playground_opened`/`playground_closed` that rewrites `go.work`.

### PowerShell / shell (whenever)

- No LSP needed. `pwsh -NoProfile -File main.ps1`. A good test that a plugin with
  nothing but `[language]`, `[toolchain]`, one `[[kind]]`, and `[run]` works.

## 8. Distribution and installation

- **Install** = a folder under `<app-data>/plugins/<id>/`. `tinkerpad plugin install
  <path|git-url>` copies or clones it; the UI's plugins panel does the same.
- **Helper binaries are built, not shipped.** `[helper.build]` runs on first use with
  the toolchain the plugin already requires. Prebuilt binaries in `helper/bin/<os>-<arch>/`
  are allowed as an optimisation, never required.
- **Trust model:** a plugin runs with the user's privileges, like an editor
  extension. The install UI says so. No sandbox.
- **Versioning:** `api` in the manifest and the protocol names are the compatibility
  contract. The core supports the current and previous version of each and warns on
  the previous. Breaking changes get a migration note in the changelog.
- **Conformance:** `tinkerpad plugin check <dir>` validates the manifest, probes the
  toolchain, scaffolds a playground of every kind, runs each, asserts a `dump` round
  trip, and (if a helper exists) exercises every declared hook. Contributors run it
  locally; CI runs it on bundled plugins.

## 9. Highlighters

The UI bundles CodeMirror language packages and the manifest picks one by name.
Initial list: `javascript`, `typescript`, `csharp`, `rust`, `python`, `go`, `json`,
`markdown`, `shell`, `sql`, `yaml`, `cpp`, `java`, `kotlin`, `swift`, `lua`, `zig`.
A plugin whose language has no bundled highlighter gets plain text and files an issue.
Loading highlighters from the plugin folder is a possible later addition.

## 10. Adding a language: the checklist

1. Create the folder with `plugin.toml`, `templates/`, and `dump/`.
2. Fill in `[language]`, `[toolchain]`, one `[[kind]]`, and `[run]`. Run
   `tinkerpad plugin check`. Hello-world should pass.
3. Write the dump library (§6). Check again; the `dump` round-trip should pass.
4. Add `[lsp]`. If the server is shared across playgrounds, add `[lsp.register]`.
5. Add `[deps]` and `[output]` if the language has them.
6. Only if the manifest can't express something, or runs are slow: add `helper/`.
7. Add a README with the toolchain requirements and open a PR, or publish the repo.
