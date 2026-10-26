# 04-12 · The library as an explorer, on either side

**Phase:** 4 · **Size:** M · **Crate/area:** `ui/`, `tinkerpad-core` (settings), `tinkerpad-app` · **Depends on:** 01-04, 04-01, 04-02
**Design:** PLAN.md §3.5, ARCHITECTURE.md §6

## What

Make the library panel work as the explorer for the folder where scripts are
saved:
- It can sit on the left or the right, and the user can swap it.
- It remembers whether it was open.
- It says which folder it's showing.
- It lists everything in that folder that's a playground, including the
  broken ones it hides today.

## Why

The ask was "a common area where the scripts are saved, and an explorer on the
right or left side that shows all of the scripts currently in the folder".
**Most of that already exists**, which is the honest starting point for this
item:

- **The common area exists.** Every playground is saved as a folder under one
  library root, `~/TinkerPad/` by default, `[library].root` in settings to
  change it (01-04, PLAN.md §3.5).
- **The explorer exists, on the left only.** The library panel (04-01, `Mod+B`)
  lists every playground in that folder with search, tags, pinned and recent,
  grouped by language or flat. It updates live from a file watcher, so a
  playground copied into the folder from outside appears within a second.

What's missing is why it doesn't feel like an explorer:

- It starts **hidden on every launch** and doesn't remember that it was open.
  `layout.ts` saves only its width (`tinkerpad.sidebar`). The only ways to find
  it are `Mod+B` or the palette, so it's easy never to discover it.
- It's **left only**. The layout is a fixed `sidebar | splitter | workspace`
  grid, and the splitter's drag maths measures from the left edge.
- It **doesn't show the folder.** Nothing in the panel names the library root
  or opens it in the file manager. Each playground has "Show in file
  explorer", but the library itself doesn't.
- It **doesn't show everything in the folder.** A folder with a malformed
  `playground.toml` is logged and returned as `broken` by
  `list_playgrounds`, but the panel never shows it. That's 01-04's one open
  criterion. A folder without `playground.toml`, or a loose file like
  `script.py` dropped into the root, is ignored by design (see Out of scope).

## Expectations

- [x] `[library].side = "left" | "right"` in settings (default `"left"`, so
      nothing moves for anyone who has it today). It's shown in the Settings
      UI (04-02) and applies live, without a restart.
- [x] A command `view.librarySide` ("Move library to other side") in the
      catalog, and the same action in the panel's own header (a small button
      or menu entry), so swapping doesn't need the settings file.
- [x] On the right, everything mirrors: the panel is on the right, the
      splitter is between it and the workspace, dragging the splitter right
      makes the panel narrower, and the saved width is the same one used on
      the left.
- [x] Whether the panel is open is remembered across launches, alongside the
      width. **On the very first launch it's open**, so a new user sees where
      their playgrounds are. After that it's whatever they left it as.
- [x] The panel header shows the library folder: a shortened path
      (`~/TinkerPad`) with the full path as a tooltip, an action to open it in
      the system file manager, and one to change it (opens Settings at
      `[library].root`).
- [x] Broken playgrounds are listed with a warning badge and their problem as
      a tooltip ("playground.toml: expected `=` at line 3"). Opening one
      explains why it can't be opened and offers to show it in the file
      manager. This closes 01-04's open criterion; tick it there in the same
      PR.
- [x] **Deleting from the list.** Right-click → **Move to recycle bin** already
      exists (04-01, `remove()` in `ui/src/library.ts`): it asks first, sends
      the folder to the OS trash, and closes the playground's tab. It works
      on both sides of the window; the self-test covers that. Missing today,
      added here:
  - [x] The **Delete** key on the selected row does the same, confirmation
        included (`Mod+Backspace` on macOS, as Finder does). Through the
        command catalog, so it's rebindable and in `docs/KEYBINDINGS.md`.
        Only while the focus is in the list, never in the search box.
  - [x] The wording follows the platform: "Move to Recycle Bin" on Windows,
        "Move to Trash" on macOS and Linux, in the menu and the
        confirmation.
  - [x] After a delete, the selection moves to the next row (or the previous
        one, if it was the last), so pressing Delete again keeps going down
        the list.
  - [x] Deleting a broken playground (one with the warning badge) works the
        same way. It's often the main thing someone wants to do with one.
- [x] Changing `[library].root` re-points the panel and the watcher at the new
      folder without a restart, or, if that isn't possible yet, says a restart
      is needed. Whichever it is, write it down here.
- [x] The existing `library-panel` self-test still passes on both sides. A new
      self-test switches to the right, checks the panel and splitter are on
      the right and that dragging resizes the right way, reloads the window
      and checks the side and open state were kept, and checks that a broken
      playground is listed with its badge. It also deletes a playground
      with the Delete key (answering the confirmation through the picker,
      not a keystroke to the OS) and checks that the selection moved on.

## Out of scope

- **Loose files and folders without `playground.toml`.** Showing any `*.py` or
  `*.ts` dropped into the library root would need a new storage design. The
  playground-is-a-folder decision (PLAN.md §3.5) is deliberate and recorded:
  dependencies, scratch state and a stable id all live in the folder. If
  "drop a script in the folder and see it" is wanted, it should be its own
  item: most likely an **Import** that turns a loose file into a playground,
  rather than the library learning a second kind of entry.
- **A file tree inside a playground** (main file, `package.json`, …).
  Playgrounds are single-file today; multi-file is a separate, larger change.
- More than one library root, or subfolders as a hierarchy (04-01 chose tags
  over folders).
- Docking any other panel (output, history) on a chosen side.
- The toolbar's show/hide button for this panel is 04-11's. If 04-11 lands
  first, its button must follow the panel to whichever side it's on: it
  toggles the panel, and its pressed state matches.
- Deleting several playgrounds at once (multi-select). Delete-and-move-on
  covers clearing out a few. A real multi-select is worth its own item if
  that turns out to be slow.
- Undo after a delete. The trash is the undo. Restoring from it isn't
  something the `trash` crate can do on macOS.

## Gotchas

- The splitter's `pointerdown` handler in `layout.ts` measures width as
  `pointer x − shell left`. On the right it's `shell right − pointer x`. Get
  this wrong and the panel grows when dragged smaller.
- `gridTemplateColumns` is set in code, and the DOM order is
  `sidebar, splitter, workspace`. Either reorder the elements or use
  `order`/named grid areas, but keep Tab order sensible on both sides.
  Screen readers follow DOM order, not visual order.
- The open/closed state belongs with the width (per machine, in the webview's
  storage), not in `settings.toml`. The side is a preference, so it does go
  in settings. Don't mix the two up, or a synced settings file will start
  opening and closing panels on other machines.
- "First launch" has to mean first launch of the app, not first launch after
  this item ships. Otherwise every existing user gets a panel popping open
  once. Probably fine either way, but decide on purpose.

## Results

Implemented 2026-10-26.

**Pieces**
- `tinkerpad-core` `settings.rs`: `[library].side`, `"left"` by default, in
  the template and written only when it isn't the default.
- `tinkerpad-library`: `broken()`, and `delete_broken(dir)`, which only
  touches a folder the index holds as broken (`LibraryError::NotBroken`
  otherwise), so a path from the window can't send anything else to the trash.
- `tinkerpad-app`: `library_info` (root, root shortened with `~`, broken
  folders), `reveal_library`, `reveal_broken_playground`,
  `delete_broken_playground`, `first_launch`. `settings::ensure_exists` now
  says whether it created the file.
- `ui/src/layout.ts`: the sidebar has a side. On the right the panel and its
  splitter move to the end of the shell in the DOM, so Tab order and screen
  readers follow what's on screen. The workspace never moves, so the editor
  keeps its state. The drag is measured from the panel's own edge. Open or
  closed is kept in `localStorage` (`tinkerpad.sidebar.open`), next to the
  width.
- `ui/src/library.ts`: a folder row under the search box (path, Open folder,
  Change folder, Move to the other side). Broken playgrounds come last, under
  "Can't be opened", with a warning badge and the problem as a tooltip. The
  list is one Tab stop (`role="listbox"`, `aria-activedescendant`), and
  `deleteSelected()` moves the selection on to the next row.
- Commands: `library.delete` (Delete and Mod+Backspace, only while the list
  has the focus), `view.librarySide`, `view.libraryFolder`.
- Settings: "Panel side" under Library. Opening Settings from the panel's
  Change folder button starts on the folder row.
- Toolbar (04-11): the Explorer button moves to the far end of the toolbar
  while the panel is on the right. It still toggles the panel, and its
  pressed state still matches.

**Decisions**
- **"First launch" means the app's first launch.** `settings.toml` is written
  on the very first launch and never removed, so its absence is the signal.
  A machine that ran TinkerPad before this item but never had an open state
  remembered keeps the panel closed, as it always was. Nobody gets a panel
  popping open after an update.
- **Changing `[library].root` already re-points without a restart.** 04-02's
  `change_library_root` moves the library and its watcher and publishes
  `libraryChanged`. The panel refreshes on that, header included. No restart
  is needed.
- **Delete and Mod+Backspace are bound on every platform**, rather than
  Delete off macOS and Mod+Backspace on it. The catalog has no per-platform
  defaults, and each binding is harmless on the other platform because it
  only works while the list has the focus.
- The side switch goes through `settings.toml` like any other preference, and
  the settings watcher applies it. So the button, the Settings form and a
  hand edit all take the same path.

**Self-test** `library-explorer`, in CI's Launch steps on Linux and macOS. It
reloads the window twice: the runner keeps a note in `sessionStorage`, so the
scenario carries on after the reload and the ones before it don't run again.
Every scenario now starts with the panel closed, without changing what the
machine remembers. Local run on Windows:

```
PASS the panel names its folder, shortened, with the full path on hover
PASS on the left, the panel comes first
PASS Change folder opens Settings at the library folder, next to the panel's side
PASS the header's button moves the panel to the right, by saving it in settings
PASS on the right, the splitter sits between the workspace and the panel, in the DOM too
PASS and the Explorer button moves to the right end of the toolbar
PASS dragging the splitter right makes a right-hand panel narrower, and the width is kept
PASS a broken playground is listed, with a warning badge and its problem as a tooltip
PASS opening it says why it can't be opened, and offers to show it or bin it
PASS Delete in the search box deletes nothing
PASS Delete on the list asks, moves the playground to the Recycle Bin, and selects the next row
PASS a broken playground is deleted the same way, and the selection steps back a row
PASS after a reload the panel is still open, on the right, at the width it was dragged to
PASS with nothing remembered, the panel is open only on the app's first launch
INFO library-panel, with the panel on the right
(all 13 library-panel checks pass on the right)
```

`layout`, `library`, `library-trash`, `tabs`, `toolbar`, `library-panel` and
`keys` pass in the same build, 116 checks in all. Two things failed in that run.
Neither comes from this change:
- `tabs` "F2 renames": the scratch library had a `renamed-tab` folder left
  from an earlier run. It passes on an empty library.
- `settings` "pointed at another folder and back" fails whenever
  `TINKERPAD_LIBRARY` is set, because the variable always beats
  `[library].root`. CI doesn't run that scenario.

**Not checked:** Mod+Backspace on a real Mac keyboard. The self-test presses
Delete. The panel's new header in light, dark and a Contrast theme hasn't
been looked at either, for the same reason as 04-11's toolbar.
