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

- [ ] All colours in the UI come from CSS custom properties defined once per theme.
      No hard-coded colours remain (grep for `#` in CSS as the check).
- [ ] CodeMirror: one highlight style per theme covering the token types the
      bundled highlighters emit; Dump renderers use the same variables.
- [ ] `theme = "system" | "light" | "dark"` in settings; system follows
      `prefers-color-scheme` live.
- [ ] Contrast checked for text, diagnostics squiggles, and table zebra rows in
      both themes.

## Out of scope

User-defined themes. Importing VS Code themes (possible later, the token set is
similar).

## Gotchas

Tauri's window background should be set to the theme colour before the webview
paints to avoid a white flash at launch in dark mode.
