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

- [ ] `[library].side = "left" | "right"` in settings (default `"left"`, so
      nothing moves for anyone who has it today). It's shown in the Settings
      UI (04-02) and applies live, without a restart.
- [ ] A command `view.librarySide` ("Move library to other side") in the
      catalog, and the same action in the panel's own header (a small button
      or menu entry), so swapping doesn't need the settings file.
- [ ] On the right, everything mirrors: the panel is on the right, the
      splitter is between it and the workspace, dragging the splitter right
      makes the panel narrower, and the saved width is the same one used on
      the left.
- [ ] Whether the panel is open is remembered across launches, alongside the
      width. **On the very first launch it's open**, so a new user sees where
      their playgrounds are. After that it's whatever they left it as.
- [ ] The panel header shows the library folder: a shortened path
      (`~/TinkerPad`) with the full path as a tooltip, an action to open it in
      the system file manager, and one to change it (opens Settings at
      `[library].root`).
- [ ] Broken playgrounds are listed with a warning badge and their problem as
      a tooltip ("playground.toml: expected `=` at line 3"). Opening one
      explains why it can't be opened and offers to show it in the file
      manager. This closes 01-04's open criterion; tick it there in the same
      PR.
- [ ] **Deleting from the list.** Right-click → **Move to recycle bin** already
      exists (04-01, `remove()` in `ui/src/library.ts`): it asks first, sends
      the folder to the OS trash, and closes the playground's tab. It works
      on both sides of the window; the self-test covers that. Missing today,
      added here:
  - [ ] The **Delete** key on the selected row does the same, confirmation
        included (`Mod+Backspace` on macOS, as Finder does). Through the
        command catalog, so it's rebindable and in `docs/KEYBINDINGS.md`.
        Only while the focus is in the list, never in the search box.
  - [ ] The wording follows the platform: "Move to Recycle Bin" on Windows,
        "Move to Trash" on macOS and Linux, in the menu and the
        confirmation.
  - [ ] After a delete, the selection moves to the next row (or the previous
        one, if it was the last), so pressing Delete again keeps going down
        the list.
  - [ ] Deleting a broken playground (one with the warning badge) works the
        same way. It's often the main thing someone wants to do with one.
- [ ] Changing `[library].root` re-points the panel and the watcher at the new
      folder without a restart, or, if that isn't possible yet, says a restart
      is needed. Whichever it is, write it down here.
- [ ] The existing `library-panel` self-test still passes on both sides. A new
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
