# 00-03 · Editor and output pane layout

**Phase:** 0 · **Size:** M · **Crate/area:** `ui/` · **Depends on:** 00-02
**Design:** ARCHITECTURE.md §7

## What

The product's one screen: a CodeMirror 6 editor on top, a resizable output pane
below, a thin status bar at the bottom. One editor instance, plain text mode, no
language logic yet. The output pane shows a monospace stream of text and can be
cleared.

## Why

This is the surface the user lives in. Getting the feel right early (keyboard focus,
resize, scrolling behaviour under streaming output) is cheaper than retrofitting.

## Expectations

- [x] CodeMirror 6 with: line numbers, bracket matching, history, default keymap,
      a monospace font, and a basic dark theme. No language package yet.
- [ ] Output pane below the editor with a draggable splitter. Split position
      persists across launches (localStorage is fine).
- [x] The bottom area is a tabbed panel container with "Output" as its only tab.
      Adding a tab is a one-line registration. (DEBUGGING.md D2: Variables, Call
      Stack, and Debug Console dock here later.)
- [ ] Output pane appends text efficiently: 10,000 lines appended in 1,000 chunks
      does not freeze the UI (use a virtualised list or a capped DOM with a "show
      all" affordance; measure).
- [x] Output auto-scrolls to bottom while the user is at the bottom; stops
      auto-scrolling if the user scrolls up; a "jump to bottom" button appears.
- [x] Stdout and stderr are visually distinct (colour or gutter mark).
- [x] Keyboard: editor has focus on launch; `Mod+Enter` fires a `run` action;
      `Mod+L` clears output; `Escape` from the output pane returns focus to the
      editor. `Mod` is Cmd on macOS and Ctrl elsewhere, via one helper used by
      every shortcut from here on (CodeMirror's `Mod-` prefix does this natively).
- [x] Status bar shows a placeholder for language name and a run state (idle /
      running with elapsed time).
- [ ] Works at 1280×720 and at 2560×1440 without horizontal scrolling.

## Out of scope

Tabs (01-12), Dump renderers (01-08), theming beyond one dark theme (04-04), LSP.

## Gotchas

Keep the editor mounted permanently. Later, tabs will swap `EditorState` objects
in and out of a single `EditorView`, which preserves undo history and scroll per tab.
Build with that in mind: the editor component owns one view and accepts a state.

## Results

Implemented 2026-09-14.

**Verification approach.** An opt-in in-app self-test (`ui/src/selftest.ts`)
runs when the app starts with `TINKERPAD_SELFTEST=layout`. It drives
CodeMirror's real keymap with synthetic key events and inspects the DOM,
printing `selftest: PASS|FAIL|INFO` lines to stdout. The CI launch jobs on
macOS and Linux run it and fail on any `FAIL`. Nothing sends keystrokes to the
desktop.

**Windows release build, self-test results:** all checks pass.

| Check | Result |
|---|---|
| Editor accepts input | pass |
| Mod+Enter fires run (through CodeMirror keymap) | pass |
| Mod+L clears output, overriding select-line | pass |
| 10,000 lines in 1,000 chunks | 33.7 ms to render |
| stderr styled separately from stdout | pass |
| Auto-scroll while at bottom | pass |
| Scrolling up stops auto-scroll, shows jump button | pass |
| Jump to bottom resumes | pass |
| DOM capped at 20,000 lines with a notice | pass |
| Escape in output returns focus to editor | pass |
| Swapping `EditorState` in and out keeps the document | pass |
| No page-level horizontal overflow | pass |

**Not verified by the self-test**
- Dragging the splitter and its persistence across launches. The code stores
  the ratio in `localStorage` with a try/catch fallback, but nothing drives a
  real pointer drag.
- The 2560×1440 layout. The self-test checks overflow only at the window's
  current size (900×600 here).

**Notes**
- The bottom area is a tabbed `BottomPanel` with Output as its only tab
  (DEBUGGING.md D2).
- Bundle is 293 kB (95 kB gzipped) with CodeMirror. Launch to UI ready stayed
  at 313–383 ms.

## Checklist status

Checked 2026-09-14.

**Open**
- *Draggable splitter that persists.* Implemented, but no test drives a pointer
  drag or relaunches to check the saved ratio.
- *10,000 lines in 1,000 chunks with a virtualised list or a capped DOM with
  "show all".* Rendering is fast (34 ms) and the DOM is capped at 20,000 lines,
  but there is no "show all": trimmed lines are dropped, with a notice saying how
  many.
- *Works at 1280×720 and 2560×1440.* The self-test checks for page-level
  horizontal overflow only at the window's current size (900×600).

**Ticked with a note**
- *Keyboard.* The criterion names "one helper used by every shortcut". Shortcuts
  use CodeMirror's `Mod-` prefix, plus a document-level handler that accepts
  either Ctrl or Cmd for focus outside the editor.
