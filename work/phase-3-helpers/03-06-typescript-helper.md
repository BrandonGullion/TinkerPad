# 03-06 · TypeScript Node helper

**Phase:** 3 · **Size:** S · **Crate/area:** `plugins/typescript/helper/` · **Depends on:** 03-03, 01-09
**Design:** PLUGINS.md §7 (TypeScript), §5

## What

A Node script that stays resident and runs playgrounds by dynamic `import()` with
a cache-busting query, so Node's ~40 ms startup and the type-strip pass happen
once. Small, and mostly here to prove a second helper against the protocol.

## Why

Node is already fast per-run. The helper wins ~40 ms per Mod+Enter and, more
importantly, gives the protocol a second real implementation before contributors
write theirs.

## Expectations

- [ ] `helper/helper.mjs`, no build step (`[helper]` without `build`).
- [ ] On `run`: `await import(pathToFileURL(main) + "?t=" + Date.now())`, with
      `process.stdout.write` and `process.stderr.write` monkey-patched for the
      duration to emit base64 chunks; unhandled rejections and thrown errors →
      `stderr` with the stack plus `exited` code 1.
- [ ] Cancel: no safe way to abort a running module in-process; on `cancel` the
      helper replies nothing and lets the core kill and respawn it (03-01's 2 s
      rule). Document this trade-off in the plugin README.
- [ ] Module cache growth: the query-string trick leaks one module entry per run.
      Restart the helper automatically after 500 runs (helper-side counter,
      reply `exited` then exit 0; the pool restarts it on next use without
      counting it as a crash — add a `{"type":"retiring"}` message to the
      protocol if the pool needs to distinguish this, and update PLUGINS.md §5).
- [ ] `--experimental-transform-types` passed in `[helper].command` args so
      `enum` works like the per-run path.
- [ ] The helper's Node args come entirely from the manifest (`[helper].command`
      + args), never hard-coded in the script, so Phase 6 can add
      `--inspect=0` via a `[debug]` section. Confirm by hand that starting the
      helper with `--inspect=0` prints the inspector URL on stderr and the helper
      still speaks the protocol on stdout. (DEBUGGING.md D10.)

## Out of scope

Worker-thread isolation (would allow cancel; consider if the respawn cost annoys).

## Gotchas

A playground that starts a server (`http.createServer`) keeps the helper busy
after `exited`. Track open handles with `process.getActiveResourcesInfo()` and
warn on `stderr` if any remain.
