# 01-13 · Clickable error annotations

**Phase:** 1 · **Size:** S · **Crate/area:** `tinkerpad-runner`, `ui/` · **Depends on:** 01-06, 01-01
**Design:** PLUGINS.md §2 (`[output]`)

## What

Apply the plugin's `[output].error_regex` to stderr (and stdout) lines as they
stream. Matches become `RunEvent::Annotation { file, line, col, message, severity }`.
The UI makes the line clickable in the output pane and shows a squiggle in the
editor at that position until the next run.

## Why

Before the LSP exists, this is the only way a compile error points at code. After
the LSP exists, it still matters for runtime errors and for languages whose server
doesn't report everything.

## Expectations

- [ ] Regex applied per line; named groups `file` (optional), `line`, `col`
      (optional), `message`, `severity` (optional; default error). Applied to
      both streams.
- [ ] Multi-line support: an optional `[output].error_regex_multiline = true` means
      the regex runs over a sliding window of the last 3 lines joined with `\n`.
      Rust needs this (message on one line, location on the next).
- [ ] Only annotations whose `file` is empty or resolves to the playground's main
      file get an editor squiggle; others are still clickable in the output but
      open nothing.
- [ ] Clicking an annotation moves the editor cursor and scrolls it into view.
      `F8` / `Shift+F8` cycle through annotations.
- [ ] Annotations clear at the start of the next run.
- [ ] Lines inside a locked prelude are still reported but the squiggle is placed
      on the first editable line with a note.
- [ ] Tests: the regexes shipped in the TS, C#, and Rust plugins each match one
      captured real error and don't match a normal output line.

## Out of scope

LSP diagnostics (02-04). Stack trace parsing for runtime exceptions beyond what the
regex catches.

## Gotchas

Node's stack traces reference `file:///C:/...` URLs. Either normalise URLs to paths
in the runner or let the TS plugin's regex capture the path after `file:///`.
