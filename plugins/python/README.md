# Python plugin

Runs Python playgrounds on a **warm interpreter**, with `dump()` for rich
output. It is also the reference implementation of the helper protocol: if you
are writing a plugin for another language, `helper/helper.py` is the file to
read, and this README explains it line by line.

## What it needs

Python 3.11 or newer on your `PATH`. TinkerPad tries, in order:

| Platform | Probes |
|---|---|
| macOS, Linux | `python3 --version`, then `python --version` |
| Windows | `python --version`, then `py --version` |

On Windows `python` is usually the Microsoft Store stub, which prints an advert
instead of a version; the probe fails and `py` — the launcher a python.org
install puts on `PATH` — answers instead. Whichever wins becomes `{toolchain}`
in the run and helper commands, so everything uses one interpreter.

If Python lives somewhere unusual, say so in `settings.toml`:

```toml
[toolchains.python]
python = "/opt/python3.13/bin/python3"
```

## Kinds

| Kind | Starts with |
|---|---|
| **Python Statements** (default) | Top-level code, run from top to bottom. |
| **Python Expression** | One expression, wrapped in a `dump(...)` that is locked. |

## `dump()`

```python
from tinkerpad import dump

dump([{"name": "Ada", "born": 1815}, {"name": "Grace", "born": 1906}], "people")
```

`dump(value, title=None)` returns `value`, so you can wrap an expression
without changing what it does. What you get back:

| You dump | You see |
|---|---|
| A list of dicts, dataclasses, or objects | A table, columns in first-seen order |
| Any other dict, list, tuple, set, dataclass, or object | A tree |
| Anything else | A single value |

Five levels deep, 1000 rows or items; deeper containers become a summary like
`dict(12)`, and a reference back to something above becomes `[circular]`.
`bytes` arrive as base64, and anything else JSON can't hold arrives as its
`repr`. The library lives in `dump/tinkerpad/`; `PYTHONPATH` points at it for
per-run execution, and the helper adds it to `sys.path` itself.

## The helper, line by line

A helper is a program that stays alive between runs so the interpreter doesn't
have to start again each time. It speaks JSON lines on stdin and stdout
(`docs/PLUGINS.md` §5). `helper/helper.py` is about a hundred lines, and here
is the whole of it:

**`send(**message)`** writes one JSON object and a newline to the stdout this
process started with, and flushes. It keeps its own reference (`out`) because a
run replaces `sys.stdout`, and the protocol must not end up inside a
playground's output.

**`class Stream`** is what `sys.stdout` and `sys.stderr` become while a run is
going. Every `write` turns into a `stdout` or `stderr` message with the bytes
base64-encoded. Its `.buffer` is itself, which is how `dump()` writes the raw
sentinel bytes of a rich value through it.

**`hello(message)`** is the first thing the core sends. It carries `plugin_dir`
and `scratch_dir`; the helper puts `<plugin_dir>/dump` on `sys.path` so
playgrounds can `from tinkerpad import dump`, and replies `ready` with a name
and version. Anything printed before `ready` is treated by the core as a
runtime banner, so a stray `print` at startup is survivable — but don't.

**`run(message)`** starts the playground on a thread called `tinkerpad-run`:

- `compile(source, main, "exec")` so tracebacks name the real file;
- `exec` in a fresh globals dict with `__name__ == "__main__"` and `__file__`
  set, and the working directory moved to the playground's folder, so relative
  paths behave as they would from a terminal;
- `SystemExit` becomes the exit code, `KeyboardInterrupt` means this run was
  cancelled, and any other exception prints its traceback to stderr exactly as
  Python would — which is what the plugin's `[output]` pattern reads the file
  and line out of for the editor's squiggles;
- `exited` reports the code and how long it took.

**`cancel(...)`** raises `KeyboardInterrupt` inside the run thread with
`ctypes.pythonapi.PyThreadState_SetAsyncExc`. Python gives you no other way to
stop a thread from outside. A thread stuck in C code — `time.sleep`, a socket
read — will not notice; the core waits two seconds and then kills the process,
which is what that grace period in the protocol is for.

**`main()`** is the loop: one line, one message, and a `print` to stderr for
anything it doesn't recognise. Unknown message types are ignored rather than
fatal, which is what lets a newer TinkerPad talk to an older helper.

### What warm costs you

`exec` in a fresh dict gives each run fresh globals — but not a fresh
interpreter. Modules imported by one run stay imported, with whatever state
they were left in:

```python
import random
random.seed(1)        # the next run inherits this
```

That is the trade for a fast start. When it bites, **Restart** the helper from
the status bar chip and the next run gets a new interpreter.

### Where a debugger will go

`hello` has a commented marker (and an env check, `TINKERPAD_DEBUG_PORT`)
showing where `debugpy.listen()` belongs. Nothing is wired up — debugging is
Phase 6 — but the run thread has a stable name and a single entry point, so
attaching later is a change to one function rather than a rewrite
(`docs/DEBUGGING.md` D11).

## Language server

`basedpyright-langserver --stdio`, one per playground folder:

```sh
pip install basedpyright
```

The **Languages** button in the status bar offers to run that for you, after
showing you the command (work item 04-08b). pip installs it beside the Python
it belongs to; if that folder isn't on your `PATH`, the reason is reported and
`[toolchains.python]` in `settings.toml` can point at the program directly.

Without it the editor still runs code; you just lose completion and
diagnostics, and the status strip says what to install.

## Packages

Not yet. `pip install` into a playground's own environment, and PEP 723 inline
script metadata with `uv`, are follow-ups; for now a playground uses whatever
your interpreter already has.
