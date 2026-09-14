# 00-04 · Process supervision crate

**Phase:** 0 · **Size:** M · **Crate/area:** `tinkerpad-process` · **Depends on:** 00-01
**Design:** ARCHITECTURE.md §1, §5.1

## What

The one place in the codebase that spawns child processes. Everything later (per-run
commands, language servers, helpers) goes through it. It provides: spawn with
streaming stdout/stderr, kill the whole process tree, and "children die with the
app" guarantees.

## Why

Process handling on Windows is where scratchpad apps get flaky: orphaned compilers,
console windows flashing, mojibake output, zombie servers after a crash. Solving it
once, with tests, is the foundation for "fast and reliable".

## Expectations

- [x] `spawn(SpawnSpec) -> Supervised` where `SpawnSpec` has program, args (array,
      no shell), cwd, env additions, and a `stdin: bool` flag. Returns a handle with
      an `events()` stream of `Stdout(Bytes)`, `Stderr(Bytes)`, `Exited { code }`,
      and a `kill()` that terminates the process and all descendants.
- [x] Windows: every child is assigned to one app-wide job object with
      `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, created lazily on first spawn. Killing the
      app process (Task Manager) kills every child. Verify by hand and write down
      how you verified it.
- [x] Windows: `CREATE_NO_WINDOW` so console apps don't flash a window.
- [x] Unix: child in its own process group (`setpgid` in `pre_exec`); `kill()`
      sends `SIGTERM` to the group, waits 2 s, then `SIGKILL`.
- [x] Linux: `prctl(PR_SET_PDEATHSIG, SIGTERM)` in `pre_exec` so children die if
      the app is killed hard. Verify with `kill -9` on the app.
- [x] macOS: no parent-death signal exists. Every spawned pid is appended to
      `<app-data>/children.pids` with its program name; at startup, any listed pid
      still alive whose program name matches is terminated and the file rewritten.
      Verify by `kill -9` on the app and relaunching.
- [x] All platform code lives in `tinkerpad-process::platform` (00-08); the
      public `spawn`/`kill` API has no `cfg` in its signature.
- [ ] Output is delivered as raw bytes in chunks as they arrive (not line-buffered
      by us). Chunk size and latency: a program printing one byte per 10 ms shows
      up with under 50 ms of delay.
- [x] Env defaults applied to every spawn: `PYTHONIOENCODING=utf-8`,
      `DOTNET_CLI_UI_LANGUAGE=en`, `NO_COLOR` is *not* set (we want colour later),
      `TERM=xterm-256color`.
- [x] Tests: echo round-trip; a sleeper that spawns its own child, `kill()` removes
      both (check with `tasklist`/`ps` in the test); UTF-8 output with a multibyte
      char split across two chunks arrives intact once concatenated; exit codes
      including non-zero and signal-killed. Test programs are tiny Rust binaries
      in `tests/bin/` (not `cmd`/`sh` scripts) so the same tests run on all three
      platforms in CI.
- [x] Spawn failure (program not found) is a typed error carrying the program name,
      not a panic.
- [x] `Supervised` exposes the child `pid()`. `SpawnSpec` is plain data that a
      caller can clone and extend (extra args, extra env) before spawning; no
      builder that hides the fields. (DEBUGGING.md D3, D13: attach-mode
      debugging and inspector-flag injection.)

## Out of scope

PTYs (04-05), JSON-line framing (03-01), Content-Length framing (02-01). This crate
speaks bytes only.

## Gotchas

`tokio::process::Child` needs `kill_on_drop(true)` as a belt-and-braces measure, but
it does not kill grandchildren; that's what the job object is for. On Windows, use
the `windows` crate directly or `win32job`; either is fine, pick one and wrap it.

## Results

Implemented 2026-09-14. Verified by 13 tests that run in CI on Windows, macOS,
and Linux.

**Deviations on ticked criteria**
- *One app-wide job object, created lazily.* Each child gets its own job object
  instead. That makes killing one child's tree possible (terminate its job),
  and closing every handle when the app dies still kills every tree.
- *Verify by hand with Task Manager.* Verified by a test instead:
  `children_die_when_parent_is_killed_hard` starts a stand-in parent process,
  hard-kills it, and asserts its child is gone. It passes on Windows and Linux.
  On macOS the companion test `next_launch_sweeps_orphans_from_a_force_quit`
  asserts the pid-registry sweep terminates the orphan.
- *`setpgid` in `pre_exec`.* Uses std's `process_group(0)`, which does the same.
- *`events()` stream.* The handle exposes `recv()` for the next event.
- *Test binaries in `tests/bin/`.* They live in `src/bin/` so Cargo builds them
  and exposes their paths to tests.
- *`kill()` removes both (check with `tasklist`/`ps`).* Tests check liveness
  with `platform::is_alive`, which is what `tasklist` and `ps` report.

**Open**
- *Output under 50 ms of delay for one byte per 10 ms.* The streaming test proves
  output arrives in many chunks rather than buffered, and asserts the first byte
  within 500 ms. It does not measure the 50 ms bound.

**Linux caveat found while implementing:** `PR_SET_PDEATHSIG` fires when the
spawning *thread* exits, so `spawn` must not be called from `spawn_blocking` or
other short-lived threads. Documented in `platform/linux.rs`.
