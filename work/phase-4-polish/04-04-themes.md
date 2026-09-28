# 04-04 · Themes

**Phase:** 4 · **Size:** S · **Crate/area:** `ui/` · **Depends on:** 00-03, 04-02
**Design:** PLAN.md §5 Phase 4

## What

Light and dark themes, following the OS by default, switchable in settings. One set
of CSS variables for the app chrome and a matching CodeMirror theme for each.

## Why

A dark-only app is fine for you and not for everyone. Doing it as variables now
keeps it one afternoon instead of a refactor.

## Expectations

- [x] All colours in the UI come from CSS custom properties defined once per theme.
      No hard-coded colours remain (grep for `#` in CSS as the check).
- [x] CodeMirror: one highlight style per theme covering the token types the
      bundled highlighters emit; Dump renderers use the same variables.
- [x] `theme = "system" | "light" | "dark"` in settings; system follows
      `prefers-color-scheme` live.
- [x] Contrast checked for text, diagnostics squiggles, and table zebra rows in
      both themes.

## Out of scope

User-defined themes. Importing VS Code themes (possible later, the token set is
similar).

## Gotchas

Tauri's window background should be set to the theme colour before the webview
paints to avoid a white flash at launch in dark mode.

## Results

Implemented 2026-09-28.

**Two palettes, one set of names.** Every colour in the app comes from CSS
variables defined twice: dark on `:root` (so a window paints correctly before
any setting has been read) and light under `[data-theme="light"]`, plus the
same light block under `[data-theme="system"]` inside a
`prefers-color-scheme: light` media query. Nothing outside those blocks names a
colour — `grep` for `#` in `style.css` finds only the palettes, and the four
hard-coded colours that were left in TypeScript (the editor's selection, the
isolated HTML dump's text, link and border) are variables now. The dump
stylesheet got the same treatment, including the table's alternating rows.

**The editor.** A highlight style is built once and can't read variables, so
`ui/src/theme.ts` holds one per theme, covering the Lezer standard tags every
bundled grammar and legacy mode emits, and swaps it — with CodeMirror's own
`dark` flag and the editor's colour rules — in a `Compartment`. Changing theme
repaints the open editor without recreating the view.

**`theme = "system" | "light" | "dark"`**, `system` by default: the app should
look like the rest of the machine until told otherwise. `system` follows
`prefers-color-scheme` live, through a media-query listener that stays for the
life of the window. The settings form grew an Appearance section with the
choice.

**The launch flash.** `index.html` carries an inline style that paints the
ground per `prefers-color-scheme` before any stylesheet loads, and the window's
own background colour is set from the theme in settings during `setup`, before
the webview paints. Dark no longer flashes white; light no longer flashes dark.

**Contrast is measured, not eyeballed.** The `themes` self-test computes WCAG
ratios from the variables actually in force, in both themes:

| What | Bar | Result |
|---|---|---|
| Text, muted text, diagnostics, links, code tokens, dump values (22 pairs) | 4.5:1 | all pass |
| The accent, which marks focus and selection, on three surfaces | 3:1 | all pass |
| Separators (borders) against what they separate | visible (≥1.5:1) | 1.60–1.95 |
| A table's alternating rows | tellable apart | 0.007 (dark), 0.09 (light) luminance |

Twelve checks, all passing, plus `dump-render` and `annotations` unchanged.

## Checklist status

**Note: borders are held to a lower bar than the accent.** WCAG's 3:1 for
non-text is about things you act on; a separator that meets it is a box, not a
hairline. The accent — focus rings, the active tab, selection — meets 3:1 on
every surface it appears on, and borders are required to be visible. Both are
measured, and the numbers are in the table above.

**Note: the default theme changed from `dark` to `system`.** 04-02 stored the
setting but had only one theme to point it at.
