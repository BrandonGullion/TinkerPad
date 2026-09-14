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
