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

- [ ] CodeMirror 6 with: line numbers, bracket matching, history, default keymap,
      a monospace font, and a basic dark theme. No language package yet.
- [ ] Output pane below the editor with a draggable splitter. Split position
      persists across launches (localStorage is fine).
- [ ] The bottom area is a tabbed panel container with "Output" as its only tab.
      Adding a tab is a one-line registration. (DEBUGGING.md D2: Variables, Call
      Stack, and Debug Console dock here later.)
- [ ] Output pane appends text efficiently: 10,000 lines appended in 1,000 chunks
      does not freeze the UI (use a virtualised list or a capped DOM with a "show
      all" affordance; measure).
- [ ] Output auto-scrolls to bottom while the user is at the bottom; stops
      auto-scrolling if the user scrolls up; a "jump to bottom" button appears.
- [ ] Stdout and stderr are visually distinct (colour or gutter mark).
- [ ] Keyboard: editor has focus on launch; `Mod+Enter` fires a `run` action;
      `Mod+L` clears output; `Escape` from the output pane returns focus to the
      editor. `Mod` is Cmd on macOS and Ctrl elsewhere, via one helper used by
      every shortcut from here on (CodeMirror's `Mod-` prefix does this natively).
- [ ] Status bar shows a placeholder for language name and a run state (idle /
      running with elapsed time).
- [ ] Works at 1280×720 and at 2560×1440 without horizontal scrolling.

## Out of scope

Tabs (01-12), Dump renderers (01-08), theming beyond one dark theme (04-04), LSP.

## Gotchas

Keep the editor mounted permanently. Later, tabs will swap `EditorState` objects
in and out of a single `EditorView`, which preserves undo history and scroll per tab.
Build with that in mind: the editor component owns one view and accepts a state.
