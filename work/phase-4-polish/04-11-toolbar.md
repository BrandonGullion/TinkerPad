# 04-11 · Toolbar above the tabs

**Phase:** 4 · **Size:** S · **Crate/area:** `ui/` · **Depends on:** 04-03, 04-10
**Design:** ARCHITECTURE.md (UI), 04-03 (commands)

## What

A thin toolbar across the top of the editor pane, above the tab strip, with
buttons for the things done most often:

- **New playground**: a new playground in the default language (what `Mod+N`
  does).
- **New playground in…**: pick the language (and kind, if it has more than
  one) first (what `Mod+Shift+N` does).
- **Run**: run the active playground (`Mod+Enter`, F5 after 04-10). While a run
  is going, the same button becomes **Cancel** (`Mod+.`).

These three are a start. The toolbar should make it trivial to add a button
later, for anything that's already a command.

## Why

Every one of these exists today but only as a key or a palette entry
(`Mod+Shift+P`). Someone who hasn't learnt the keys has no visible way to run
code: the only run control on screen is the Cancel link in the status bar,
which appears once something is *already* running. A playground tool with no
Run button looks broken to a new user.

## Current state (2026-10-25)

- `ui/src/layout.ts` builds the editor pane as `tab-strip`, `conflict-bar`,
  `editor-host`. There's no toolbar and no Run button anywhere.
- The commands exist and have handlers in `ui/src/main.ts`: `playground.new`
  → `tabs.newPlayground()`, `playground.newOfKind` → `tabs.pickAndCreate()`
  (language picker, then kind picker when there's more than one),
  `run.start`, `run.cancel`.
- The registry (`ui/src/commands/registry.ts`) already knows whether a command
  is available right now (`enabled`) and what it's bound to (`bindingFor`,
  `format`).

## Expectations

- [ ] A `toolbar` row above the tab strip, spanning the editor pane, with
      `role="toolbar"` and an accessible label.
- [ ] Every button calls `runCommand(id)`. No button has its own copy of what
      a command does, so the toolbar, the keys and the palette can't
      disagree.
- [ ] Each button's tooltip is its title plus its current binding, from
      `bindingFor` and `format` (e.g. "Run (Ctrl+Enter)"), so a rebinding in
      `keybindings.toml` shows up without a restart.
- [ ] A button is disabled when its command isn't available: Run with no
      playground open, or while the active language's toolchain isn't ready.
      It updates when that changes (tab switch, run start/end, toolchain
      state), not only on hover.
- [ ] Run turns into Cancel while the active tab's playground is running, and
      back afterwards. The status bar's Cancel stays as it is.
- [ ] **New playground in…** opens the same picker as `Mod+Shift+N`. A split
      button with a language list right on the toolbar is a reasonable later
      step, not part of this item.
- [ ] Icons plus text labels, or icons with tooltips. Pick one and make it
      readable in every theme (04-04), including high contrast. Icons are
      inline SVG in the UI bundle (no icon font, no CDN).
- [ ] Keyboard: the toolbar can be reached by Tab; arrow keys move between
      buttons inside it (the usual roving tabindex); Enter/Space press.
- [ ] Adding a button is one entry in a list of command ids (with an icon),
      not new layout code. A short note in the code says so.
- [ ] A self-test clicks each button and checks that it did the same as the
      key: a new tab in the default language, the picker opening, a run
      starting and Cancel stopping it, Run disabled with nothing open.

## Out of scope

Buttons beyond these three (library toggle, settings, clear output, add
package). They're one list entry each once this lands, and worth adding when
someone wants them. Customising which buttons appear. A menu bar. Moving the
tab strip.

## Gotchas

- The editor pane height is shared by the editor and the output split.
  Taking 30-odd pixels for the toolbar changes the split's maths; check the
  draggable splitter and its saved position still behave.
- A click on a toolbar button takes the focus out of the editor. After **Run**
  or **New playground**, put the focus back in the editor, the way the keys
  leave it, or typing straight after a click goes nowhere.
- "Running" is per playground (`ui/src/run.ts` keeps state per playground).
  The button reflects the **active** tab's playground, so switching tabs
  during a run should flip it between Run and Cancel.
