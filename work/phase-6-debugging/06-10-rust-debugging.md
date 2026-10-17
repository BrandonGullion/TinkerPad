# 06-10 · Debugging Rust

**Phase:** 6 · **Size:** M · **Crate/area:** `plugins/rust`, possibly `tinkerpad-plugin` · **Depends on:** 06-07
**Design:** DEBUGGING.md §1, D8, D14

## What

`codelldb` or `lldb-dap`, launching the binary a playground builds.

## Why

The first launch-mode language, and the one that forces the question D14 parked:
a launch adapter runs the built artifact, not the build command, so something
has to know what the artifact is.

## Expectations

- [ ] A debug profile that keeps line tables (D8), separate from the fast run
      profile and sharing the same target directory so debugging does not mean
      a second full build.
- [ ] Breakpoints in `src/main.rs` stop on the right lines.
- [ ] Variables are legible: a `Vec`, a `HashMap`, an `Option`, a struct with a
      `Debug` impl. Both candidate adapters ship Rust formatters; which one
      does it better is a reason to pick it, and the measurement belongs here.
- [ ] **The D14 decision, made and written down.** Either `[debug].config`
      naming the artifact is enough, or `[run]` gains an optional `[build]`
      beside it. Whichever: say which, say why, and if it is the split, note
      that it is its own work item rather than smuggled into this one.
- [ ] The adapter is acquired, not bundled. `codelldb` is a VS Code extension
      archive; `lldb-dap` comes with LLVM. Whichever is chosen, say what the
      user is being asked to install and from where.
- [ ] Both kinds debug: `program` and the locked-lines `expression`.
- [ ] Recorded: time from key to first breakpoint, cold and warm, against the
      run timings 01-11 already measured. A debug build that takes a minute is
      a feature nobody uses.

## Out of scope

Debugging a playground's dependencies' source beyond stepping into it. Pretty
printers of our own — if the adapter's are poor, that is a reason to choose the
other adapter, not to write formatters.

## Gotchas

`debug = 0` strips exactly what breakpoints need, which is why D8 exists. The
profile has to be *named* and selected for a debug run, not a global change that
slows every ordinary run down.

rust-analyzer and a debug build both want cargo's lock on the target directory.
02-07 already moved the server out of the run's target dir for this reason;
check the debug profile has not undone it.

A `cargo build` that fails during a debug start must report as a build failure
with its annotations (01-13), not as "the debugger could not start".
