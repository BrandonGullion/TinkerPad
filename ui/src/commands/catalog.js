// Every command TinkerPad has, as data (work item 04-03).
//
// Plain JavaScript on purpose: the app imports it for the palette and the
// keymap, and `npm run docs:keys` imports the same file in Node to write
// `docs/KEYBINDINGS.md`. One list, so the documentation cannot drift from the
// app, and no build step to make that true.
//
// A binding is written the way `keybindings.toml` writes it: lowercase parts
// joined by `+`, with `mod` meaning Cmd on macOS and Ctrl everywhere else.
// Use `ctrl` or `cmd` explicitly only when a binding really is
// platform-specific (Ctrl+Tab is, because that is what every tabbed app uses).

/**
 * @typedef {object} CommandInfo
 * @property {string} id        Stable id: what `keybindings.toml` names.
 * @property {string} title     What the palette shows.
 * @property {string} category  The group it belongs to.
 * @property {string} [key]     Default binding, e.g. `mod+shift+p`.
 * @property {string} [when]    A condition id; absent means always.
 * @property {boolean} [hidden] Kept out of the palette (it has its own key).
 */

/** @type {CommandInfo[]} */
export const CATALOG = [
  // ---- running -------------------------------------------------------------
  { id: "run.start", title: "Run", category: "Run", key: "mod+enter", when: "playground" },
  { id: "run.cancel", title: "Cancel run", category: "Run", key: "mod+.", when: "playground" },
  { id: "run.clearOutput", title: "Clear output", category: "Run", key: "mod+l" },
  {
    id: "run.forcePerRun",
    title: "Force per-run execution",
    category: "Run",
  },
  {
    id: "run.history",
    title: "Recent runs",
    category: "Run",
    when: "playground",
  },

  // ---- playgrounds ---------------------------------------------------------
  { id: "playground.new", title: "New playground", category: "Playground", key: "mod+n" },
  {
    id: "playground.newOfKind",
    title: "New playground…",
    category: "Playground",
    key: "mod+shift+n",
  },
  { id: "playground.open", title: "Open playground…", category: "Playground", key: "mod+o" },
  {
    id: "playground.rename",
    title: "Rename playground",
    category: "Playground",
    key: "f2",
    when: "playground",
  },
  {
    id: "playground.addPackage",
    title: "Add package…",
    category: "Playground",
    when: "playground",
  },

  // ---- tabs ----------------------------------------------------------------
  { id: "tab.close", title: "Close tab", category: "Tabs", key: "mod+w" },
  { id: "tab.next", title: "Next tab", category: "Tabs", key: "ctrl+tab" },
  { id: "tab.previous", title: "Previous tab", category: "Tabs", key: "ctrl+shift+tab" },

  // ---- moving around -------------------------------------------------------
  {
    id: "go.nextProblem",
    title: "Next error or warning",
    category: "Go",
    key: "f8",
    when: "playground",
  },
  {
    id: "go.previousProblem",
    title: "Previous error or warning",
    category: "Go",
    key: "shift+f8",
    when: "playground",
  },
  { id: "go.back", title: "Back", category: "Go", key: "alt+arrowleft" },

  // ---- the window ----------------------------------------------------------
  { id: "view.library", title: "Library panel", category: "View", key: "mod+b" },
  { id: "view.commands", title: "Commands…", category: "View", key: "mod+shift+p" },
  { id: "view.tools", title: "What each language needs", category: "View" },

  // ---- settings ------------------------------------------------------------
  { id: "settings.open", title: "Settings…", category: "Settings", key: "mod+," },
  { id: "settings.file", title: "Open settings file", category: "Settings" },
  { id: "settings.keybindings", title: "Open keybindings file", category: "Settings" },
  { id: "settings.copyKeybindings", title: "Copy keybindings as Markdown", category: "Settings" },
  { id: "settings.logs", title: "Open log folder", category: "Settings" },
  { id: "app.update", title: "Install update", category: "Settings" },
];

/** The default binding of every command that has one, by id. */
export function defaultBindings() {
  /** @type {Record<string, string>} */
  const bindings = {};
  for (const command of CATALOG) {
    if (command.key) bindings[command.id] = command.key;
  }
  return bindings;
}

/** The categories, in the order the palette and the docs show them. */
export const CATEGORIES = ["Run", "Playground", "Tabs", "Go", "View", "Settings"];
