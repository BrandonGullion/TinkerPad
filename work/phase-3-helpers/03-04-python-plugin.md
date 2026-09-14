# 03-04 · Python plugin and reference helper

**Phase:** 3 · **Size:** M · **Crate/area:** `plugins/python/` · **Depends on:** 03-03, 01-09 (pattern), 02-07
**Design:** PLUGINS.md §7 (Python), §5, §6

## What

The Python plugin: manifest, templates, dump library, LSP section, and the
simplest possible helper. The helper is written to be read: it is the reference
implementation contributors copy.

## Why

Python's helper is ~40 lines, which makes it the best first real test of the
protocol and the best teaching example for the plugin template repo (05-03).

## Expectations

- [ ] Python is installed on this machine first (note the version here). The
      manifest probes `python3` on macOS/Linux and `python` then `py` on Windows
      (per-OS probe entries from 01-03), `min_version = "3.11"`. `[run]` and
      `[helper].command` use the resolved probe program, so add a `{python}`-style
      placeholder: `{toolchain}` resolves to whichever probe succeeded. Add it to
      the placeholder set in 01-01. `uv` support is a later refinement.
- [ ] Kinds: `statements` (default), `expression` (locked `from tinkerpad import
      dump; dump((` … `))`).
- [ ] `dump/tinkerpad/__init__.py`: `dump(value, title=None)`; lists of dicts or
      dataclasses → `table`; else `tree`; cycles, depth 5, row cap 1000; writes the
      OSC bytes to `sys.stdout.buffer` and flushes. `PYTHONPATH` set in `[run].env`
      and in the helper's environment.
- [ ] `[lsp]`: `basedpyright-langserver --stdio`, `workspace = "playground_dir"`,
      with an install hint (`pip install basedpyright`).
- [ ] `helper/helper.py`: reads JSON lines from stdin; on `run`, compiles the file
      and `exec`s it in a fresh globals dict with `__name__ == "__main__"`,
      redirecting `sys.stdout`/`sys.stderr` to base64-chunk emitters; catches
      `SystemExit` for the exit code; `cancel` raises `KeyboardInterrupt` in the
      running thread via `ctypes.pythonapi.PyThreadState_SetAsyncExc`; replies
      `exited` with duration. Under 80 lines including comments that explain each
      message.
- [ ] Mod+Enter to first output for `print(1)` via the helper: measured and
      recorded (expect under 10 ms vs ~30–50 ms per-run).
- [ ] Plugin README explains the helper line by line for contributors.
- [ ] The helper runs user code on a thread whose name and entry point are
      stable, and has a commented-out (or env-gated) `debugpy.listen()` block at
      startup showing where Phase 6 plugs in. Don't wire it up; just don't
      structure the helper so it would be hard to. (DEBUGGING.md D11.)

## Out of scope

Virtualenv/uv project management, notebooks, matplotlib images (a `dump` of a
figure as `image` is a nice follow-up).

## Gotchas

`exec` in a fresh dict doesn't reset imported modules; a playground that mutates a
module's state affects the next run. Document it; add a "restart helper" action in
the status strip (02-08 already has it).
