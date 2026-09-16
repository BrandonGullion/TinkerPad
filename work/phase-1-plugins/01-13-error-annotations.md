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

- [x] Regex applied per line; named groups `file` (optional), `line`, `col`
      (optional), `message`, `severity` (optional; default error). Applied to
      both streams.
- [x] Multi-line support: an optional `[output].error_regex_multiline = true` means
      the regex runs over a sliding window of the last 3 lines joined with `\n`.
      Rust needs this (message on one line, location on the next).
- [x] Only annotations whose `file` is empty or resolves to the playground's main
      file get an editor squiggle; others are still clickable in the output but
      open nothing.
- [x] Clicking an annotation moves the editor cursor and scrolls it into view.
      `F8` / `Shift+F8` cycle through annotations.
- [x] Annotations clear at the start of the next run.
- [x] Lines inside a locked prelude are still reported but the squiggle is placed
      on the first editable line with a note.
- [x] Tests: the regexes shipped in the TS, C#, and Rust plugins each match one
      captured real error and don't match a normal output line.

## Out of scope

LSP diagnostics (02-04). Stack trace parsing for runtime exceptions beyond what the
regex catches.

## Gotchas

Node's stack traces reference `file:///C:/...` URLs. Either normalise URLs to paths
in the runner or let the TS plugin's regex capture the path after `file:///`.

## Results

Implemented 2026-09-15.

**Core**
- `tinkerpad-core`: `RunEvent::Annotation(Annotation)` with `file`, `line`,
  `col`, `message`, `severity`, and `text` (the matched line or lines).

**Runner**
- `annotate.rs`: `Annotator` takes the plugin's `[output]` spec and is fed one
  completed line at a time.
  - A single-line pattern matches each line.
  - A multi-line pattern matches the last three lines joined with `\n`, and only
    a match reaching the newest line counts, so one error is reported once as
    the window slides.
  - `line` and `message` are required; `file`, `col`, and `severity` are
    optional. `severity` is a warning only when it says so.
- `OutputDecoder` keeps a partial line per stream and offers each completed line
  to the annotator, so lines split across chunks (or interleaved with the other
  stream) still match. The text event containing the line is flushed first, then
  the annotation, so the UI always marks the line it just drew. A last line
  without a newline is still offered at the end.
- `runs.rs` builds the decoder from the plugin's `[output]` and logs how many
  annotations a run produced.

**UI**
- `annotations.ts`: an editor extension holding each tab's annotations as a
  state field, drawn as a wavy underline in the error or warning colour from
  the reported column to the end of the line, with the message on hover. Tabs
  keep their own, because the state belongs to the tab.
- The output pane turns the matched line into a link (`markAnnotation`), styled
  with a dotted underline and reachable by the output pane's arrow-key
  navigation.
- `tabs.ts`:
  - an annotation is marked in the editor only when its file is missing or
    resolves to the tab's main file, allowing for relative paths, `file://`
    URLs, and Windows case;
  - a position inside a kind's locked prelude is marked on the first editable
    line, with a note saying where it really was;
  - clicking a link activates the tab and moves the cursor; a link for another
    file says so in the status bar instead;
  - `F8` and `Shift+F8` walk the annotations;
  - starting a run clears them.
- The TypeScript plugin gained an `[output]` section. Node prints the error and
  then the stack, so the pattern spans two lines and accepts both plain paths
  and `file://` URLs.

**Tests**
- `annotate.rs` unit tests: the C# pattern against real `dotnet` output
  (including a warning), the Rust pattern reporting once across a sliding
  window, and a pattern with no optional groups.
- `output.rs` unit tests: an annotation follows the text it was found in, lines
  split across chunks and streams still match, and a last line without a
  newline still annotates.
- Each bundled plugin now has two tests in its runner test file: a real failing
  run produces the expected annotation, and a normal run produces none.
  - TypeScript: `Error: on line 8` at `main.ts` line 8, column 7, and only one
    annotation, because later stack frames have no message line.
  - C#: `CS1002: ; expected` at line 3, column 39.
  - Rust: `mismatched types` at line 3, column 21, with both lines kept in
    `text`.
- New in-app `annotations` scenario, 10 checks: the clickable line, the squiggle
  at the right line and column (computed style `underline wavy rgb(248, 113,
  113)`), the message on hover, clicking moving the cursor to line 2 column 9,
  `F8` and `Shift+F8` walking 2 → 5 → 2, a warning marked differently, another
  file saying so instead of jumping, the next run clearing everything, and a
  locked-line position marked on the first editable line with a note.
- All 151 checks in 13 scenarios pass on Windows. CI runs the scenario on macOS
  and Linux.
- A screenshot showed the underlined stack frame in the output pane.

**Docs**
- PLUGINS.md §2.1 now describes `[output]` in detail: the groups, the three-line
  window, which annotations get marked, locked lines, and clearing.
- ARCHITECTURE.md §7 lists `F8` and `Shift+F8`.

## Checklist status

**Ticked with a note**
- *Applied to both streams.* Yes, with one partial-line buffer per stream, so a
  line split across reads still matches.
- *Squiggle placed on the first editable line with a note.* The note is the
  hover text on the mark, which reads "Reported on line N, inside this kind's
  locked lines."
- *Node's `file:///` URLs.* Handled in two places: the TypeScript pattern skips
  the `file:///` prefix, and the UI's file matching also decodes URLs, so a
  plugin that captures the URL as-is still works.
