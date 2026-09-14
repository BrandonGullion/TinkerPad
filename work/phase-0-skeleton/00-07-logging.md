# 00-07 · Logging and error conventions

**Phase:** 0 · **Size:** S · **Crate/area:** all crates · **Depends on:** 00-01
**Design:** ARCHITECTURE.md §2 (key crates)

## What

`tracing` everywhere, written to a rolling file in the app data dir and to stderr in
dev. `thiserror` error types in library crates, `anyhow` only in `tinkerpad-app`.
A "copy logs" affordance in the UI.

## Why

When a language server misbehaves in six months, the log is the only evidence. It's
a one-evening job now and a painful retrofit later.

## Expectations

- [ ] `tracing-subscriber` with an `EnvFilter` (`TINKERPAD_LOG=debug` overrides),
      a daily rolling file via `tracing-appender` in `<app-data>/logs/`, and a
      pretty stderr layer in debug builds only.
- [ ] Log dir is created on first launch; old logs pruned to the last 7 days.
- [ ] Every spawned process gets a `tracing` span with its name and pid; its stderr
      lines are logged at `debug` inside that span (done in `tinkerpad-process`).
- [ ] Each library crate has one `Error` enum via `thiserror` with variants that
      carry context (which file, which program). No `Box<dyn Error>` in public APIs.
- [ ] A `Help → Open log folder` menu item or command.
- [ ] Panics are logged with a backtrace before the app dies (`std::panic::set_hook`).

## Out of scope

Telemetry of any kind. A log viewer in the UI (02-08 adds a small one for servers).

## Gotchas

Never log the contents of the user's code at `info` or above; at `trace` is fine
and off by default.

## Results

Implemented 2026-09-14.

- `tinkerpad-app` `logging.rs`: `EnvFilter` from `TINKERPAD_LOG` (default
  `info`), a daily rolling file `tinkerpad.YYYY-MM-DD.log` in Tauri's app log
  folder keeping 7 files, and a compact stderr layer in debug builds only.
- Log folder on Windows:
  `%LOCALAPPDATA%\io.github.brandongullion.tinkerpad\logs`. macOS uses
  `~/Library/Logs/<identifier>`, Linux `~/.local/share/<identifier>/logs`.
- `tinkerpad-process`: every child's tasks run inside a
  `process{program, pid}` span; its stderr is logged line by line at `debug`,
  in pieces of at most 4 KiB, and only when debug is enabled.
- Panic hook logs location, message, and a forced backtrace, then calls the
  previous hook.
- "Logs" button in the status bar calls the `open_log_folder` command.
- Errors: `SpawnError` (process) and `RunError` (runner) are `thiserror`
  enums carrying the program name. The other library crates have no fallible
  code yet.
- No user code is logged. Spawn arguments (the scratch file path) are logged at
  `debug`.

**Verified** by running the release app with `TINKERPAD_LOG=debug` and the
`run` self-test, then reading the file:

```
INFO  tinkerpad_app::logging: TinkerPad starting version="0.1.0" platform="windows" log_dir=…
INFO  tinkerpad_app::commands: ui ready ready_ms=327
DEBUG process{program=node pid=26848}: tinkerpad_process::spawn: spawned args=[…scratch.js]
INFO  tinkerpad_app::runs: run started run=1 pid=26848
DEBUG process{program=node pid=40988}: tinkerpad_process::spawn: stderr=oops
INFO  tinkerpad_app::runs: run finished run=3
```

**Not verified**
- The panic hook firing. Nothing in the app panics on demand.
- The "Logs" button opening a file manager. Testing it would open a window on
  the desktop; it compiles against `tauri-plugin-opener` and uses its
  documented Rust API.
- Pruning to 7 days, which needs a week of files. It relies on
  `tracing-appender`'s `max_log_files`.
