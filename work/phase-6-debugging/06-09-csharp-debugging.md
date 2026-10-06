# 06-09 · Debugging C#

**Phase:** 6 · **Size:** M · **Crate/area:** `plugins/csharp` · **Depends on:** 06-07
**Design:** DEBUGGING.md §1, §3, D9

## What

`netcoredbg`, attached to the C# helper while it runs a playground.

## Why

D9 made the helper emit a portable PDB and load the assembly with its PDB
stream, so that the sequence points name `main.cs` rather than a temporary file.
This is the item that finds out whether that was enough.

## Expectations

- [ ] A session attaches to the helper process and stops on a breakpoint in
      `main.cs`, at the line the editor shows (D1, D9).
- [ ] Both a helper session and a per-run one, as 06-07 established.
- [ ] Variables are legible: a `List<T>`, a `Dictionary<K,V>`, an anonymous
      type, a record. These are what a C# playground is full of.
- [ ] `netcoredbg` is acquired rather than required, by download (04-08a) since
      there is no package manager answer that works everywhere.
- [ ] All three kinds debug: `statements`, `program` and the locked-lines
      `expression`.
- [ ] Recorded here: whether a collectible `AssemblyLoadContext` holds up under
      a debugger, which DEBUGGING.md §3 flags as the thing most likely to be
      flaky. If it isn't, the fallback is a non-collectible context while a
      session is live — a runtime switch, not a redesign. Measure before
      choosing.

## Out of scope

Debugging the plugin's own helper source. Edit-and-continue — a different
feature wearing similar clothes.

## Gotchas

A PDB supplied at load time is what makes a dynamically loaded assembly
debuggable. If the helper ever stops passing the PDB stream, breakpoints bind to
nothing and the symptom is "the debugger ignores me" rather than an error.
Assert it in the plugin's own `check/`, not just here.

`netcoredbg` releases are per-platform archives with inconsistent layouts
inside. The `acquire` download needs a `member` per OS and a checksum per
version — the machinery 04-08a built, used as intended.

Attaching to a process that is mid-`Dump()` should not corrupt the output
stream. Worth one test, because the failure would look like a dump parser bug.
