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
| `[run]`, `[lsp]`, `[lsp.register].value` | `{main}` `{dir}` `{id}` `{name}` `{scratch}` `{plugin_dir}` `{version}` `{toolchain}` |
| `[helper]` and its `build` | `{scratch}` `{plugin_dir}` `{version}` `{toolchain}`. A helper serves every playground of its language, so the ones that name one are not available; the playground arrives in the `run` message instead. |
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
| `[lsp].workspace_files` | `[{ path = "tsconfig.json", template = "tsconfig.json" }]`: files the server's *workspace* needs, written there the first time it starts and never overwritten. Same shape as `[scaffold].files`, but relative to the workspace root rather than a playground. |
| `[lsp.register].reload` | A request to send after the registered setting changes, for a server that reads it once: `"rust-analyzer/reloadWorkspace"`. |
| `[deps]` with `kind = "command"` | Needs `add = ["cargo", "add", "{name}@{version}"]`. |
| `[deps]` with `kind = "inline-directive"` | Needs `directive = "#:package {name}@{version}"`. |
| `[output].error_regex_multiline` | `true` to match across up to three lines. |

**`[deps]` in detail.** "Add package" asks for a name and an optional version,
then does what this section says. `{name}` and `{version}` are the only
placeholders; when no version is given, `{version}` is empty and a trailing `@`
is dropped, so `serde@{version}` becomes `serde`.

- `kind = "command"`: `add` runs in the playground folder with the same `PATH` a
  run gets, and its output streams into the output pane like a run's.
- `kind = "inline-directive"`: `directive` goes at the top of the main file,
  after any lines already starting with the same text up to `{name}`. A line for
  the same package is replaced rather than repeated, and the file is saved at
  once. Nothing is written if the core refuses the name.
- A kind with a `prelude` can't take a directive: the file has to start with the
  prelude for the lock to be recognised, so the top of the file is spoken for.
  Directive languages should give people a kind without locked lines (C# has
  `statements` and `program`).

**`[output]` in detail.** Every completed line of stdout and stderr is matched
against `error_regex`. A match becomes a position the app can use.

- Groups: `line` and `message` are required; `file`, `col`, and `severity` are
  optional. Any other group is ignored, with a warning. `severity` counts as a
  warning when it reads `warning` or `warn`, and as an error otherwise.
- With `error_regex_multiline = true` the pattern is matched against the last
  three lines joined with `\n`, and only a match reaching the newest line counts,
  so one error is reported once. Write such a pattern with `(?m)` and `^`…`$`
  anchors. rustc needs this: the message is on one line and the location on the
  next.
- The matched line becomes clickable in the output pane. It is marked in the
  editor only when `file` is missing or points at the playground's own main
  file; `file` may be absolute, relative to the playground, or a `file://` URL.
- A position inside a kind's locked prelude is marked on the first editable line
  instead, and says where it really was.
- Marks are cleared when the next run starts.

**Other rules**
- Exactly one `[[kind]]` has `default = true`.
- Ids (`language.id`, kind ids) use lowercase letters, digits, and hyphens.
- Paths are relative and stay inside their folder: no leading `/`, no drive
  letter, no `..`.
- `idle_shutdown` is `"never"` or a duration such as `"90s"`, `"30m"`, `"2h"`. The
  user may override it per plugin in `settings.toml`, under `[lsp.<id>]` for a
  language server and `[helpers.<id>]` for a helper.
- `[helper].build` runs before the helper's first use, and again whenever anything
  in the plugin folder changes (its `bin`, `obj`, `target`, `node_modules` and `.git`
  folders excepted). Nothing else runs it: a plugin that ships a built helper simply
  leaves `build` out.
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

**How a new playground's files are made**

- The main file is `prelude + template + postlude`, in that order.
- Templates can use these variables. Any other `{{…}}` is left as written.

  | Variable | Becomes |
  |---|---|
  | `{{name}}` | The display name. |
  | `{{date}}` | Today's date in UTC, such as `2026-09-14`. |
  | `{{folder}}` | The playground's folder name, such as `fizz-buzz-2`. |
  | `{{package}}` | A package-manager-safe name. It is the folder name when that is lowercase ASCII starting with a letter, otherwise `playground-<id>`. It is unique in the library. |
  | `{{plugin_dir}}` | The plugin folder, absolute. |
  | `{{scratch}}` | The plugin's scratch folder, absolute. |

- Paths are written with `/` separators, which work on Windows too and need no
  escaping in TOML or JSON strings.
- Absolute paths go stale if the app or its data folder moves. So when a playground's
  files use `{{plugin_dir}}` or `{{scratch}}`, the values written are also recorded
  in its `playground.toml` under `[plugin.<language id>]`, where a repair can find
  them.
- The prelude and postlude are written exactly as in the manifest, with no
  substitution. A playground can be renamed, and its locked lines must still
  match.
- `[scaffold].files` templates get the same variables.
  A scaffold entry at the main file's path is ignored, and an existing file is
  never overwritten.
- Templates are read from the plugin folder every time a playground is created, so
  edits to a template take effect immediately.

**How locked lines are found.** Nothing about them is stored. When a playground is
opened, the file is locked if it still starts with the prelude and ends with the
postlude, byte for byte. If it doesn't, for example after an edit in another editor
or a change of line endings, nothing is locked and the status bar says so. Write
preludes and postludes with `\n` line endings; a prelude usually ends with one and a
postlude usually starts with one, so the lines between them stay fully editable.

**In the editor.** Locked lines are dimmed with a lock in the gutter. The cursor can
go there, but edits are refused with a hint. Select-all and delete clears only the
editable middle, and Mod+A selects only the middle.

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

A helper is any executable speaking JSON lines on stdin/stdout. The core starts
it lazily on first need, keeps it alive under the same rules as LSP servers, and
restarts it on crash. `hooks` in the manifest lists which messages it implements;
the core never sends the others.

**The transport.** One JSON object per line, terminated by `\n`. A trailing `\r`
is stripped, so a runtime whose `println` ends lines the Windows way needs no
special care. A line that isn't a message is logged (its first 200 bytes) and
skipped, never fatal: helpers print banners, warnings, and progress, and many
runtimes announce themselves before they have said a word of protocol. Lines
longer than 16 MiB are dropped rather than buffered; send large output in chunks.
The helper's **stderr goes to the log** and is never shown as program output — a
*run's* stderr arrives as a `stderr` message instead, and only that reaches the
results pane.

Every message has `type` and, for requests, `id`. Replies carry the same `id`.

**Lifecycle**

```json
→ {"type":"hello","protocol":"tinkerpad-plugin-v1","plugin_dir":"...","scratch_dir":"..."}
← {"type":"ready","name":"tinkerpad-csharp-helper","version":"0.1.0"}
→ {"type":"shutdown"}
```

`plugin_dir` is the plugin folder, for the helper's own resources.
`scratch_dir` is a folder private to this plugin that the helper may write to;
it is the only place it should. Both are absolute. `name` and `version` are
optional and are shown in the log and the status strip.

**Execution** (`hooks = ["run"]`)

```json
→ {"type":"run","id":"r1","main":"C:/.../main.cs","dir":"C:/...","kind":"statements"}
← {"type":"stdout","id":"r1","data":"SGVsbG8K"}              // base64, chunked
← {"type":"stderr","id":"r1","data":"..."}
← {"type":"exited","id":"r1","code":0,"duration_ms":41}
← {"type":"failed","id":"r1","message":"...","diagnostics":[{"file":"...","line":3,"col":9,"message":"...","severity":"error"}]}
→ {"type":"cancel","id":"r1"}
```

`data` is base64 of the raw bytes the program wrote, chunked however suits the
helper: the core reassembles them in order and hands the program's output on
untouched, so a `tinkerpad-dump-v1` sentinel (§6) or a UTF-8 character may be
split across two messages without harm. Don't decode, re-encode, or line-buffer
program output; a partial line is still output.

`code` may be `null` when the platform has no code to give. `duration_ms` is the
helper's own timing and is used when present; otherwise the core times the round
trip itself. A run ends with exactly one of `exited` or `failed` — `failed` is
for a program that never ran (a compile error, a missing toolchain), and its
`diagnostics` land in the editor's gutter the same way a regex-matched error from
a per-run tool does. In a diagnostic, only `message` is required; `line` is
one-based, `severity` is `error` unless it says `warning` or `info`.

`cancel` asks the helper to stop the program; report `exited` (with `code: null`)
once it has. The helper stays alive. A helper that *can't* stop a running program
— a Node helper importing a module on its own thread, say — should say nothing,
and the core kills and restarts it after the two seconds below. That is a real
answer; pretending to cancel is not.

**Stepping aside**

```json
← {"type":"retiring","reason":"500 runs; Node's module cache keeps every one"}
```

A helper that has accumulated something it can only shed by restarting says
this and then exits 0. The core starts a fresh one immediately — no backoff, no
crash counted, nothing said to the user, who didn't ask for it. Send it when
nothing is running: a run in flight when the process goes is a failed run.

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

A `playground` is the object the UI uses, camelCase and absolute:

```json
{"id":"7f3c","name":"Scratch","language":"csharp","kind":"statements",
 "dir":"C:/Users/me/TinkerPad/scratch","mainFile":"C:/Users/me/TinkerPad/scratch/main.cs",
 "createdMs":1700000000000,"tags":["sql"]}
```

`open` is every playground open in the app, this one included. Every hook
answers with `result`, even when there is nothing to say. `files` paths are
relative to the playground folder; `version` on `add_dependency` is absent when
the user didn't pin one, and the helper picks. `lsp_config` is merged into what
`[lsp.register]` sends for that playground (§4).

**Timeouts**

| Wait | How long | What happens then |
|---|---|---|
| `hello` to `ready` | 10 s | kill, fall back to `[run]`, warn |
| `retiring` to exit | 2 s | kill; either way a fresh helper is started |
| `cancel` to `exited` | 2 s | kill and respawn; warm state is lost, which is fine |
| a hook to its `result` | 5 s | carry on as if it returned nothing |
| `shutdown` to exit | 2 s | kill |
| a `run` | never | a program runs until it exits or the user cancels it |

Rules:

- One `run` at a time per helper. A second `run` cancels the first, and the
  first run is reported as cancelled rather than abandoned. The core only sends
  one when it means it: two playgrounds of the same language *queue* on the
  helper, and the waiting one is told so (work item 03-03). A helper therefore
  sees a second `run` mid-run only when the same playground was run again,
  which is a person pressing Run twice.
- **Forward compatibility:** helpers must ignore fields they don't recognise in
  any message, and the core ignores unknown fields from helpers. A message type
  the other side has never heard of is logged and ignored, not an error. New
  optional fields (a future `debug` flag on `run`, for example) are added without
  a protocol version bump; only removed or re-typed fields bump it.
- The helper isolates runs as well as the language allows (collectible
  `AssemblyLoadContext`, fresh module scope, fresh globals). Perfect isolation is
  not required; the restart button exists.
- If `[helper.build]` is present, the core runs it on first use and whenever the
  helper source folder changes. This is how a C# helper ships as source and a
  contributor never has to publish binaries.

The Rust side of all this is `tinkerpad_plugin::helper`, and
`src/bin/fakehelper.rs` in that crate is a small working implementation to read
or copy.

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

Details a library author may want to know about how the app shows these:

- Object keys are shown in the order the library wrote them.
- A `value` or `timing` without a `title` sits inline in the surrounding text.
  Everything else, and anything with a `title`, is a block.
- A table cell that is a plain array or object shows as a collapsed tree.
- `text` is highlighted only when the app bundles a grammar for `lang`. Today that is
  `json`; the editor's languages join it with work item 02-04.
- `html` loses scripts, event handlers, `<style>`, forms, frames, media, link
  targets, and any URL except inline `data:image/…` sources. It renders in an
  isolated box, so page styles don't apply to it.

**The dump library, not the core, is responsible for:** cycle detection, depth limit
(default 5), item cap (default 1000 rows, sets `truncated`), choosing `table` for
sequences of records and `tree` for everything else, and type names. The core only
parses and renders. That keeps language semantics in the language.

**What the core does with the stream** (`tinkerpad-runner`'s dump parser):

- `ST` (`ESC \`) ends a payload as well as `BEL`.
- `v` may be left out and means 1. A `kind` the app doesn't know is kept and shown
  as raw JSON, so a library can add kinds before the app renders them.
- A payload that isn't valid JSON, or a known kind with missing fields, is shown as
  error-styled text with a warning. It is never dropped.
- A payload over 8 MiB is skipped and replaced by a "too large" marker. Keep dumps
  well under that; the item cap above is the real protection.
- A sentinel still open when the program exits is shown as plain text.
- Only stdout is scanned. Sentinels written to stderr appear as text.

The fixtures in `crates/tinkerpad-runner/tests/dump_cases/` pin down this behaviour
byte for byte.

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

### TypeScript / JavaScript

- **Run:** on the helper below, or `node --experimental-transform-types
  --disable-warning=ExperimentalWarning --require {plugin_dir}/dump/register.cjs
  main.ts` when there isn't one. Needs Node 22.7 or later.
- **Helper (JavaScript):** `helper/helper.mjs`, no build step, started with the
  same flags a run gets. A run is `await import(file + "?t=…")`, with
  `process.stdout.write` borrowed for the duration, so Node's start-up and the
  first type-strip pass are paid once a session rather than once a run: about
  5 ms against about 45 ms (work item 03-06). Two honest limits: it cannot
  cancel a running module, so it says nothing and lets the core kill it; and the
  cache-busting query keeps every module it has ever loaded, so it `retiring`s
  after 500 runs.
  - The transform flag adds `enum`, `namespace`, and parameter properties to
    Node's type stripping. Line numbers still match the file, so stack traces and
    future breakpoints land correctly.
  - `register.cjs` adds a resolve hook so `import { dump } from "tinkerpad"`
    finds `dump/`. `NODE_PATH` doesn't apply to ES modules, and `--import`
    rejects Windows paths.
  - Each playground gets a `package.json` with `"type": "module"`.
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

### C#

- **Run:** on the helper below, or `dotnet run --file main.cs` (.NET 10 file-based
  apps) when there isn't one. `#:package Foo@1.2.3` at the top of the file pulls
  NuGet packages either way.
- **Helper (C#):** `helper/`, a .NET console app built on first use
  (`[helper.build]`) into the app's data folder. It keeps Roslyn warm and runs each
  playground in-process: the file is parsed with its real path, compiled with
  portable PDBs so a stack trace says `main.cs:line N`, loaded into a collectible
  `AssemblyLoadContext`, run on a thread called `tinkerpad-run`, and unloaded.
  Compile errors come back as `failed` with the compiler's own diagnostics rather
  than as text for a pattern to match. `#:package` directives are resolved with
  NuGet into the global packages folder. A rerun costs about 70 ms against about a
  second for `dotnet run` (work item 03-05).
- **Dump:** each playground gets a `Directory.Build.props` that compiles
  `$(TINKERPAD_CSHARP_DUMP)` into the build and adds a global `using TinkerPad`.
  `[run].env` points that variable at `{plugin_dir}/dump/TinkerPad.Dump.cs`, so no
  install path is written into the playground.
  - The same file sets `PublishAot=false`. File-based apps default to AOT
    settings, which print trimming warnings on every build and disable
    reflection-based serialisation.
  - It also sets `Nullable=annotations`, so nullable warnings don't clutter every
    run.
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

- **Layout:** each playground folder is a real crate with `src/main.rs`.
  `[scaffold]` writes two files.
  - `Cargo.toml` has package name `{{package}}`, edition 2024, and `serde`. It
    adds the `tinkerpad` dump crate as a path dependency on
    `{{plugin_dir}}/dump`, and an empty `[workspace]`.
  - `.cargo/config.toml` sets `build.target-dir` to `{{scratch}}/target`, shared by
    every Rust playground so dependencies compile once. Package names are unique,
    so binaries in the shared folder don't collide.
- **Profiles:** `dev` uses `opt-level = 0` and `debug = "line-tables-only"`.
  `tp-debug` inherits from `dev` with `debug = true`, for Phase 6.
- **Run:** `cargo run --quiet`, cwd `{dir}`, with `CARGO_TERM_COLOR=never`.
  rustc prints the message on one line and ` --> src/main.rs:3:9` on the next.
  So `[output].error_regex` spans both lines, with `error_regex_multiline = true`.
- **Kinds:** `program` (default; template has `fn main()`), `expression` (locked
  `fn main() { tinkerpad::dump(&(` ... `)); }`).
- **LSP:** one `rust-analyzer`, workspace `scratch_workspace`, `[lsp.register]` with
  `rust-analyzer.linkedProjects` = `{dir}/Cargo.toml`. RA reloads the workspace set
  on `didChangeConfiguration` without a restart.
- **Deps:** `[deps] kind = "command"`, `add = ["cargo", "add", "{name}@{version}"]`.
- **No helper needed.** The shared target dir is the warm cache.
- **Memory:** RA is why `idle_shutdown` exists. Default 30 minutes.

### Python

- **Toolchain:** `python3` then `python` on macOS and Linux; `python` then `py` on
  Windows, where the first is usually the Microsoft Store stub and fails the probe.
  Whichever answers becomes `{toolchain}` for both the run command and the helper,
  so one interpreter does everything.
- **Run:** `{toolchain} -u -X utf8 main.py`, with `PYTHONPATH` pointing at the
  plugin's `dump/`. (`uv run` and PEP 723 inline metadata are a later refinement.)
- **LSP:** `basedpyright-langserver --stdio`, workspace `playground_dir`. `pip`
  puts it in Python's `Scripts`/`bin` folder, which isn't always on `PATH` — the
  install hint says so, and `[toolchains.python]` in settings.toml can point at it.
- **Helper (Python):** `helper/helper.py`, the reference implementation of §5 and the file
  to read before writing one of your own. A run is `exec(compile(src, path,
  "exec"), globals)` on a thread called `tinkerpad-run`, with `sys.stdout` and
  `sys.stderr` replaced by objects that turn writes into protocol messages, and
  `cancel` is `KeyboardInterrupt` raised into that thread. Warm means the
  interpreter is shared: globals are fresh each run, imported modules are not.

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
Bundled: `cpp`, `csharp`, `go`, `java`, `javascript`, `json`, `jsx`, `kotlin`,
`lua`, `markdown`, `python`, `ruby`, `rust`, `scala`, `shell`, `sql`, `swift`,
`toml`, `typescript`, `yaml`. Each is a separate chunk, loaded the first time a
playground in that language is opened, so none of them is in the startup bundle
(`ui/src/highlight.ts`).
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
