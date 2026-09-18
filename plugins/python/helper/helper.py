"""TinkerPad's Python helper: the reference implementation of
``tinkerpad-plugin-v1`` (docs/PLUGINS.md §5).

One JSON object per line on stdin, one per line on stdout. The core says
``hello`` and expects ``ready``; then ``run`` starts a playground and
``stdout``/``stderr``/``exited`` report it, ``cancel`` stops it, and
``shutdown`` ends this process.

What makes it a *warm* runner is that the interpreter is already up: a run is
``exec`` in a fresh globals dict, not a new process. Fresh globals are not a
fresh interpreter, though — imported modules keep whatever state the last run
left them (README.md, "What warm costs you").

Read top to bottom; each message is one function.
"""

import base64
import ctypes
import json
import os
import sys
import threading
import time
import traceback

PROTOCOL = "tinkerpad-plugin-v1"
#: The thread every playground runs on. Named and started in one place so a
#: debugger can find it, and so `cancel` knows where to raise
#: (docs/DEBUGGING.md D11).
RUN_THREAD = "tinkerpad-run"

out = sys.stdout  # Kept before `run` replaces sys.stdout for the playground.


def send(**message):
    """One message to the core."""
    out.write(json.dumps(message) + "\n")
    out.flush()


class Stream:
    """A replacement for ``sys.stdout``/``sys.stderr`` during a run.

    Everything the playground prints becomes a protocol message. ``.buffer``
    is this same object, so the dump library can write raw sentinel bytes
    through it (§6).
    """

    def __init__(self, kind, run_id):
        self.kind, self.run_id, self.buffer = kind, run_id, self
        self.encoding = "utf-8"

    def write(self, data):
        if isinstance(data, str):
            data = data.encode("utf-8", "replace")
        if data:
            send(type=self.kind, id=self.run_id, data=base64.b64encode(data).decode("ascii"))
        return len(data)

    def flush(self):
        pass

    def isatty(self):
        return False


def hello(message):
    """Say what we are, and put the plugin's ``dump`` package on the path."""
    sys.path.insert(0, os.path.join(message.get("plugin_dir", ""), "dump"))
    # Where Phase 6 plugs a debugger in: nothing is wired up, but the run
    # thread is stable and this is the line that would wait for an attach
    # (docs/DEBUGGING.md D11).
    if os.environ.get("TINKERPAD_DEBUG_PORT"):
        send(type="stderr", id="", data=base64.b64encode(b"debugging is Phase 6\n").decode())
    send(type="ready", name="tinkerpad-python-helper", version="1.0.0")


def run(message):
    """Run a playground on its own thread and report how it went."""
    run_id, main = message["id"], message["main"]
    started = time.monotonic()

    def body():
        # A run that finishes without saying otherwise exited 0, as a process
        # would. `cancelled` is the one case that reports nothing: `cancel`
        # sends the ending itself, because it knows when it asked.
        code, cancelled = 0, False
        stdout, stderr = Stream("stdout", run_id), Stream("stderr", run_id)
        sys.stdout, sys.stderr = stdout, stderr
        try:
            with open(main, "rb") as f:
                source = f.read()
            # `__name__ == "__main__"` so `if __name__ == "__main__":` runs, and
            # `__file__` so relative paths in the playground behave.
            globals_ = {"__name__": "__main__", "__file__": main, "__builtins__": __builtins__}
            os.chdir(message.get("dir") or os.path.dirname(main))
            exec(compile(source, main, "exec"), globals_)
        except SystemExit as exit_:
            code = exit_.code if isinstance(exit_.code, int) else (0 if exit_.code is None else 1)
        except KeyboardInterrupt:
            # What `cancel` raises.
            cancelled = True
        except BaseException:
            # The traceback is what the plugin's `[output]` pattern reads the
            # position out of, so it goes to stderr as Python writes it.
            traceback.print_exc()
            code = 1
        finally:
            sys.stdout, sys.stderr = sys.__stdout__, sys.__stderr__
        if not cancelled:
            send(
                type="exited",
                id=run_id,
                code=code,
                duration_ms=int((time.monotonic() - started) * 1000),
            )

    thread = threading.Thread(target=body, name=RUN_THREAD, daemon=True)
    thread.start()
    return thread


def cancel(thread, run_id, started):
    """Stop the running playground by raising ``KeyboardInterrupt`` in it.

    Python has no way to stop a thread from outside, so this is the way in:
    the interpreter raises the exception in that thread the next time it
    checks. A thread blocked in C code (``time.sleep``, a socket read) won't
    notice, and the core kills this process after two seconds — which is
    exactly what the protocol says that grace is for.
    """
    if thread and thread.is_alive():
        ctypes.pythonapi.PyThreadState_SetAsyncExc(
            ctypes.c_ulong(thread.ident), ctypes.py_object(KeyboardInterrupt)
        )
        thread.join(timeout=1.5)
    send(type="exited", id=run_id, code=None, duration_ms=int((time.monotonic() - started) * 1000))


def main():
    thread, current, started = None, None, 0.0
    while True:
        # `readline`, not `for line in sys.stdin`: iterating a file object reads
        # ahead in blocks, so a message sits in Python's buffer until more
        # arrives — which, for a protocol that waits for an answer, is forever.
        # `-u` doesn't help; it doesn't apply to stdin.
        line = sys.stdin.readline()
        if not line:
            return  # The core closed our stdin: it is done with us.
        line = line.strip()
        if not line:
            continue
        try:
            message = json.loads(line)
        except ValueError:
            print(f"helper: not a message: {line[:200]}", file=sys.stderr)
            continue

        kind = message.get("type")
        if kind == "hello":
            hello(message)
        elif kind == "run":
            current, started = message["id"], time.monotonic()
            thread = run(message)
        elif kind == "cancel" and message.get("id") == current:
            cancel(thread, current, started)
            thread, current = None, None
        elif kind == "shutdown":
            return
        else:
            # A message this version doesn't know: ignore it and carry on.
            print(f"helper: ignoring {kind}", file=sys.stderr)


if __name__ == "__main__":
    main()
