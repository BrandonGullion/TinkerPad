# TinkerPad — Debugging (deferred design)

Debugging is planned for the end of the project (Phase 6). This document exists
now for one reason: to name the constraints that earlier phases must respect so
that adding a debugger later is ordinary work rather than a rewrite. Each
constraint points at the work item that enforces it.

## 1. How it will work

The Debug Adapter Protocol (DAP) is LSP's sibling: a child process speaking
`Content-Length`-framed JSON over stdio, with `request`/`response`/`event`
messages instead of JSON-RPC. One open-source adapter exists per planned language:

| Language | Adapter | License | Mode |
|---|---|---|---|
| TypeScript / JS | `js-debug` (vscode-js-debug, DAP server mode) or Node's inspector directly | MIT | attach to `node --inspect`, or launch |
| C# | `netcoredbg` | MIT | attach to the helper, or launch the built dll |
| Rust | `codelldb` or `lldb-dap` | MIT / Apache | launch the built binary |
| Python | `debugpy` | MIT | embedded in the helper (`debugpy.listen`) or launch |
| Go | `dlv dap` | MIT | launch |

The core gets a `DebugAdapterPool` built on the same supervision code as the LSP
and helper pools, a DAP client that reuses the framing layer from `tinkerpad-lsp`,
and a proxy to the UI like the LSP proxy (URI rewriting only). Plugins get a
`[debug]` manifest section: adapter command, how to launch or attach for each
runner kind, and a launch-config template. The helper protocol gets a `debug`
field on `run` so a helper can start the program stopped at entry with the
debugger attached.

The UI gets a breakpoint gutter, a current-line marker, hover-to-evaluate, and
three panels in the bottom tab bar: Variables, Call Stack, Debug Console. The
Output tab keeps working; `Dump()` still rides on stdout.

## 2. Constraints on earlier phases

These are the decisions that would be expensive to reverse. Each is enforced as
an acceptance criterion in the named work item.

| # | Constraint | Why | Enforced in |
|---|---|---|---|
| D1 | The file on disk is exactly what the editor shows. No hidden preludes, no mirroring to another path before compile. | Breakpoints map editor lines to source lines 1:1. | PLUGINS.md §3, 01-05 |
| D2 | The bottom area is a tabbed panel container with Output as its first tab. | Variables / Call Stack / Console panels dock beside it later. | 00-03 |
| D3 | `tinkerpad-process` exposes the child pid and allows extra spawn args/env per launch. | Attach-mode debugging and `--inspect` injection. | 00-04 |
| D4 | `Content-Length` framing is its own module, independent of JSON-RPC routing. | DAP uses the same framing with different message shapes. | 02-01 |
| D5 | Pool supervision (spawn, restart, idle, RSS) is generic and shared. | A third pool for debug adapters costs nothing. | 03-02 |
| D6 | Unknown top-level tables in `plugin.toml` are warnings, not errors; unknown keys inside known tables are errors. | A plugin can add `[debug]` without breaking older cores. | 01-01 |
| D7 | Helpers ignore unknown fields in protocol messages. | A `debug` flag on `run` can be added without a protocol version bump. | 03-01 |
| D8 | Rust playgrounds build with a named profile for debugging that keeps line tables (`debug = "line-tables-only"` at minimum), separate from the fast run profile, sharing the same target dir. | `debug = 0` strips what breakpoints need. | 01-11 |
| D9 | The C# helper parses source with the on-disk path, emits a portable PDB, and loads the assembly with its PDB stream. | netcoredbg needs sequence points that name `main.cs`. | 03-05 |
| D10 | Node-based helpers and runs can be started with `--inspect=0` (random port) via a manifest field, and report the port on stderr. | js-debug attaches to that port. | 03-06, 01-09 |
| D11 | The Python helper can import and `listen()` with debugpy when asked. | debugpy is the only Python debugger that speaks DAP. | 03-04 |
| D12 | No default run timeouts anywhere in the core. A process stopped at a breakpoint is silent for minutes and must not be killed by a heuristic. | Obvious once stated; easy to add by accident. | 01-06, 03-01 |
| D13 | `RunSpec` stays plain data (program, args, cwd, env) that a caller can modify before spawn. | A debug launch is a `RunSpec` with extra args and env. | 00-06, 01-06 |
| D14 | `[run]` may later be split into optional `[build]` + `[run]`. The core must not assume run is a single opaque command with no reusable artifact. | Launch-mode debuggers run the built artifact, not the build command. | 01-06 (note only) |
| D15 | Peek tabs can open arbitrary read-only files by path. | Stepping into library source. | 02-09 |

## 3. Things that look like corners but aren't

- **In-process helpers.** Attaching a debugger to the helper process is how
  LINQPad has always worked. As long as symbols are emitted (D9) and the helper is
  launchable with inspector flags (D10, D11), warm execution and debugging coexist.
- **Collectible `AssemblyLoadContext` in the C# helper.** netcoredbg handles
  dynamically loaded assemblies when the PDB is supplied at load time. If it turns
  out to be flaky, the fallback is a non-collectible context while a debug session
  is active, which is a runtime switch, not a redesign.
- **`Dump()` during debugging.** Rides on stdout; unaffected.
- **The webview.** CodeMirror gutters are extensible; breakpoints are another
  gutter marker set, like the annotation gutter from 01-13.

## 4. When Phase 6 starts

Write the work items then, not now. The expected shape: DAP client + pool (M),
UI panels and gutter (L), then one item per language adapter (S–M each), with
Node first because attach-to-inspector is the simplest path.
