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

- [x] Python is installed on this machine first (note the version here). The
      manifest probes `python3` on macOS/Linux and `python` then `py` on Windows
      (per-OS probe entries from 01-03), `min_version = "3.11"`. `[run]` and
      `[helper].command` use the resolved probe program, so add a `{python}`-style
      placeholder: `{toolchain}` resolves to whichever probe succeeded. Add it to
      the placeholder set in 01-01. `uv` support is a later refinement.
- [x] Kinds: `statements` (default), `expression` (locked `from tinkerpad import
      dump; dump((` … `))`).
- [x] `dump/tinkerpad/__init__.py`: `dump(value, title=None)`; lists of dicts or
      dataclasses → `table`; else `tree`; cycles, depth 5, row cap 1000; writes the
      OSC bytes to `sys.stdout.buffer` and flushes. `PYTHONPATH` set in `[run].env`
      and in the helper's environment.
- [x] `[lsp]`: `basedpyright-langserver --stdio`, `workspace = "playground_dir"`,
      with an install hint (`pip install basedpyright`).
- [x] `helper/helper.py`: reads JSON lines from stdin; on `run`, compiles the file
      and `exec`s it in a fresh globals dict with `__name__ == "__main__"`,
      redirecting `sys.stdout`/`sys.stderr` to base64-chunk emitters; catches
      `SystemExit` for the exit code; `cancel` raises `KeyboardInterrupt` in the
      running thread via `ctypes.pythonapi.PyThreadState_SetAsyncExc`; replies
      `exited` with duration. Under 80 lines including comments that explain each
      message.
- [x] Mod+Enter to first output for `print(1)` via the helper: measured and
      recorded (expect under 10 ms vs ~30–50 ms per-run).
- [x] Plugin README explains the helper line by line for contributors.
- [x] The helper runs user code on a thread whose name and entry point are
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

## Results

Implemented 2026-09-18, against **Python 3.13.1** (Windows, python.org install
reached through the `py` launcher).

**`plugins/python/`**: manifest, two kinds, the dump library, the language
server section, a README that explains the helper line by line, and
`helper/helper.py` — the first real helper, and the one a contributor is meant
to copy.

**Finding Python.** Ordered probes per OS: `python3` then `python` on macOS and
Linux, `python` then `py` on Windows. That order is not decoration — on this
machine `python` is the Microsoft Store stub, which prints an advert and no
version, so the probe fails and `py` answers with 3.13.1. Whichever wins is
`{toolchain}` for the run command *and* the helper, so one interpreter does
everything.

**The dump library** (`dump/tinkerpad/__init__.py`) follows the shapes §6 asks
for: a list of dicts, dataclasses or plain objects becomes a table with columns
in first-seen order; anything else container-shaped becomes a tree; the rest is
a value. Five levels, 1000 rows, cycles become `[circular]`, `bytes` become
base64, and anything JSON can't hold becomes its `repr`. It writes the sentinel
to `sys.stdout.buffer` — which, during a helper run, is the helper's own stream.

**The helper** is 176 lines including its docstrings, of which about 70 are
code. `hello` puts `<plugin_dir>/dump` on `sys.path` and replies `ready`; `run`
compiles the file and `exec`s it on a thread called `tinkerpad-run` in fresh
globals with `__name__ == "__main__"`, with `sys.stdout`/`sys.stderr` replaced
by objects that turn writes into protocol messages; `SystemExit` becomes the
exit code and any other exception prints the traceback Python would have
printed; `cancel` raises `KeyboardInterrupt` into that thread with
`ctypes.pythonapi.PyThreadState_SetAsyncExc`; `shutdown` returns.

**Measured** (Windows, release build, `print(1)`, keystroke to first output,
best of three):

| Path | Mod+Enter to first output |
|---|---|
| Warm helper | **12.9 ms** |
| Per-run (`py -u -X utf8 main.py`) | **39.8 ms** |

And to a rendered `dump()` table: 113 ms on the first run of a session (which
includes starting the interpreter), 19 ms warm.

**Two bugs the tests found**, both worth knowing about if you write a helper:

1. `for line in sys.stdin` reads ahead in blocks, so a message sits in Python's
   buffer until more arrives — forever, for a protocol that waits for an answer.
   `-u` does not help; it doesn't apply to stdin. `sys.stdin.readline()` is the
   fix, and the helper says so where it matters.
2. A run that finished normally reported nothing: the code that meant "cancelled,
   say nothing" also caught "finished with no exception". A clean run now exits 0
   like a process would, and only a cancel stays quiet.

**The annotator's window had to grow.** A Python traceback is a frame line, the
source line under it, sometimes a `~~^~~` marker, and then the exception — and
it is the *last* frame that matters, so the window must still hold it when the
exception arrives. `WINDOW` went from 3 lines to 6, with a unit test that feeds
a real traceback line by line and expects line 5, not line 8.

**Tests** — a `python` self-test scenario in CI on all three OSes, twelve
checks: the plugin is bundled; Python is found whichever name answered; the
template renders and its `dump()` becomes a table, on the helper; a second run
is faster on the interpreter that is already up; two runs report the same pid,
which is what warm means; the expression kind dumps its one expression;
`basedpyright` offers `upper`, `lower`, `strip` after a dot on a `str`; a
traceback ends the run and marks `return 1 / 0` in the editor; a runaway
`while True` can be cancelled and leaves the helper warm; and `print(1)` is
measured warm and per-run. Plus one unit test on the traceback pattern.

CI now installs Python 3.12 (`actions/setup-python`) and `basedpyright`, since
the runner images ship older Pythons than the plugin's 3.11 floor.

## Checklist status

**Note: no new placeholder was needed.** The item asks to "add a
`{python}`-style placeholder: `{toolchain}` resolves to whichever probe
succeeded". `{toolchain}` already did exactly that (01-06), so the manifest uses
it as-is and nothing was added to the placeholder set.

**Note: the helper is 176 lines, not "under 80".** About 70 are code; the rest
are the docstrings and comments the item asks for ("comments that explain each
message") plus the `Stream` class, which the sketch in the item did not account
for — capturing a run's output is most of the work in a Python helper. The
README explains it line by line, which was the real requirement.

**Deviation: `debugpy` is a marker, not a commented-out block.** The item asks
for a commented-out or env-gated `debugpy.listen()` in the helper. What ships is
the env check (`TINKERPAD_DEBUG_PORT`) and a comment naming the line that would
wait for an attach, without importing `debugpy` — which isn't installed, and a
commented-out import is a thing that rots. The run thread has a stable name and
one entry point, which is what D11 actually asks for.

**Note: `uv` and packages are out of scope**, as the item says. A Python
playground uses whatever the chosen interpreter already has; `[deps]` is absent
rather than broken, so "Add package…" says the plugin doesn't support it.
