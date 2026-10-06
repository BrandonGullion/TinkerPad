# 06-08 · Debugging Python

**Phase:** 6 · **Size:** S · **Crate/area:** `plugins/python` · **Depends on:** 06-07
**Design:** DEBUGGING.md §1, D11

## What

`debugpy`, embedded in the Python helper and attached to.

## Why

D11: debugpy is the only Python debugger that speaks DAP, and it is a library
rather than a separate program — which makes this the smallest of the language
items and a good second one.

## Expectations

- [ ] The helper imports `debugpy` and calls `listen()` when a run asks to be
      debugged, reporting the port back. It is imported *when asked*, not at
      start-up: a helper that pays for debugpy on every launch is a slower
      helper for the ninety-nine runs that are not debugged.
- [ ] A per-run session for a playground run without the helper, so turning
      helpers off (03-03) does not turn debugging off.
- [ ] Breakpoints land on the right lines of `main.py` (D1).
- [ ] An exception stops where it was raised, with the exception in scope, and
      the traceback still arrives in the output pane as it does now.
- [ ] Variables are legible: a dict, a list, a dataclass, a numpy array if one
      is installed. `repr` is what a Python programmer expects to see.
- [ ] `debugpy` is acquired rather than required: `[debug].acquire` is
      `pip install debugpy`, offered the way everything else is (04-08b).
- [ ] The locked-prelude kinds (`expression`) debug too, with the prelude's
      lines being lines like any other — the file on disk is what runs (D1).

## Out of scope

Remote debugging. Attaching to a process TinkerPad did not start.

## Gotchas

`debugpy.listen()` in a process that is already serving the helper protocol on
stdin and stdout is the arrangement that broke 04-05: the helper's protocol loop
must keep its own handle on stdin. Check that the helper still reads the
protocol after debugpy has taken an interest in the process.

`debugpy` and the warm helper both want to own `sys.settrace`. If a playground
sets its own trace function the debugger stops working, which is correct and
should be said rather than looking like a bug in TinkerPad.
