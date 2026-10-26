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
- **Show/hide explorer**: toggles the library panel (`Mod+B`,
  `view.library`). A toggle button that looks pressed while the panel is
  open.

These four are a start. The toolbar should make it trivial to add a button
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

- [x] A `toolbar` row above the tab strip, spanning the editor pane, with
      `role="toolbar"` and an accessible label.
- [x] Every button calls `runCommand(id)`. No button has its own copy of what
      a command does, so the toolbar, the keys and the palette can't
      disagree.
- [x] Each button's tooltip is its title plus its current binding, from
      `bindingFor` and `format` (e.g. "Run (Ctrl+Enter)"), so a rebinding in
      `keybindings.toml` shows up without a restart.
- [x] A button is disabled when its command isn't available: Run with no
      playground open, or while the active language's toolchain isn't ready.
      It updates when that changes (tab switch, run start/end, toolchain
      state), not only on hover.
- [x] Run turns into Cancel while the active tab's playground is running, and
      back afterwards. The status bar's Cancel stays as it is.
- [x] **Show/hide explorer** is a toggle (`aria-pressed`) whose state
      follows the panel however it was opened or closed: the button, `Mod+B`,
      the palette, or Escape inside the panel. It sits at the end of the
      toolbar nearest the panel (the left end today; if 04-12 has landed, the
      end matching `[library].side`).
- [x] **New playground in…** opens the same picker as `Mod+Shift+N`. A split
      button with a language list right on the toolbar is a reasonable later
      step, not part of this item.
- [ ] Icons plus text labels, or icons with tooltips. Pick one and make it
      readable in every theme (04-04), including high contrast. Icons are
      inline SVG in the UI bundle (no icon font, no CDN).
- [x] Keyboard: the toolbar can be reached by Tab; arrow keys move between
      buttons inside it (the usual roving tabindex); Enter/Space press.
- [x] Adding a button is one entry in a list of command ids (with an icon),
      not new layout code. A short note in the code says so.
- [x] A self-test clicks each button and checks that it did the same as the
      key: a new tab in the default language, the picker opening, a run
      starting and Cancel stopping it, Run disabled with nothing open, the
      explorer opening and closing, and its button's pressed state
      following `Mod+B`.

## Out of scope

Buttons beyond these four (settings, clear output, add package). They're one list entry each once this lands, and worth adding when
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

## Results

Implemented 2026-10-26.

**The pieces.**
- `ui/src/toolbar.ts`: a `Toolbar` built from a list of `ToolbarItem`s (command
  id, label, icon, plus an optional `pressed` for a toggle and `swap` for a
  button that becomes another command while something holds).
- The list is in `main.ts`, four entries long.
- `layout.ts` puts the toolbar's host at the top of the editor pane, above the
  tab strip.

**Every button is its command.** Clicking runs `runCommand(id)`. The tooltip
is the catalog title plus every binding (`Run (Ctrl+Enter / F5)`, using 04-10's
`formatAll`). The button is disabled exactly when the registry says the
command is unavailable.

**Kept in step by three hooks, not polling.**
- `TabManager.onChange` fires from `renderStatus` (tab switch, listing
  change), `showEmpty` (last tab closed) and `runStateChanged`.
- `Sidebar.onChange` fires whenever the library panel is shown or hidden,
  whether by the button, `Mod+B`, the palette or Escape.
- `onBindingsChanged` now takes several listeners (it held one, the editor's),
  so a `keybindings.toml` edit retitles the buttons straight away.

**Run and Cancel.** Run's `swap` shows Cancel (red stop icon, `Cancel run
(Ctrl+.)`) while the active tab's playground is running or waiting for a
helper (`runs.running`). Switching tabs flips it, because a tab switch fires
`onChange`. Run and Cancel give the focus back to the editor, so typing
straight after a click works. New already does that; New in… leaves the
focus in its picker.

**Keyboard.** One Tab stop (roving `tabindex`), ArrowLeft/ArrowRight wrapping,
Home/End. Disabled buttons are skipped. Enter and Space press, because they
are real `<button>`s.

**Look.**
- Icon plus label. Five 16-px inline SVGs drawn in `currentColor`, so they
  take the button's colour in light and dark.
- Run's icon is `--success` and Cancel's `--error`. Labels stay in the text
  colour.
- The explorer toggle uses `aria-pressed` and looks pressed (raised
  background) while the panel is open.
- There's no high-contrast theme in the app (04-04 made light, dark and
  system), so "high contrast" here means the OS's forced-colours modes, such
  as Windows Contrast themes. Under `@media (forced-colors: active)` the
  buttons use system colours, and `Highlight` marks the pressed toggle.

**The split.** The toolbar is inside the editor pane, which is the split's
top row, so the split's ratio and saved position are unchanged. The editor
loses the toolbar's height (about 30 px) from its own share.

**Tests.** A new `toolbar` self-test scenario, 14 checks, all passing locally
on Windows (debug build), and added to CI's launch step on Linux and macOS:

```
PASS a toolbar sits above the tabs
PASS it has Explorer, New, New in… and Run, in that order
PASS Run is disabled with no playground open, and the rest aren't
PASS a button's tooltip names its command and every key for it
PASS New makes a playground in the default language, ready to type in
PASS and Run is enabled now there is something to run
PASS New in… asks for the language first
PASS Run starts the playground and turns into Cancel
PASS Cancel stops it, and the button is Run again
PASS the explorer button opens and closes the library, and looks pressed while it's open
PASS and its pressed state follows Mod+B and Escape too
PASS the toolbar is one Tab stop, and the arrow keys move along it
PASS a rebound key shows in the tooltip at once
PASS the editor still has the room the split gives it
```

`keys`, `layout`, `tabs` and `library-panel` still pass in the same run (74
checks in all).

## Checklist status

**Deviation: Run isn't disabled while the language's toolchain is missing.**
The item asked for that, but Run with a missing toolchain is how 04-08b's
offer to install it is reached. `Mod+Enter` and F5 work in that state, and
a button that disagreed with its own keys would break this item's first
rule. So Run follows the command's own condition (a playground is open).
If Run should be unavailable without a toolchain, that's a change to the
`run.start` command, and the button will follow it.

**Open: a look in each theme.** Nothing has checked the toolbar visually in
light, dark, or a Windows Contrast theme. A screenshot of the window was
blocked on this machine. Open the app in each and check that the icons,
labels, disabled and pressed states are readable. Then tick the icons
criterion.
