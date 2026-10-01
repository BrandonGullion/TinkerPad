import { renderExamples } from "./dump/examples";
import { Editor } from "./editor";
import { invoke, ready, subscribe } from "./ipc";
import { RunHistory } from "./history";
import { openSettings, settingsOpen, watchSettings } from "./settings";
import { applyTheme, followSystem, onThemeChanged, themeName } from "./theme";
import { LibraryPanel } from "./library";
import { createLayout } from "./layout";
import { BottomPanel } from "./panels";
import {
  EDITOR_COMMANDS,
  commandFor,
  copyKeybindings,
  onBindingsChanged,
  openCommandPalette,
  registerCommands,
  registerCondition,
  runCommand,
  watchKeybindings,
} from "./commands";
import { jumpBack } from "./definition";
import { pickerOpen } from "./picker";
import { PluginsPanel } from "./plugins";
import { setTabManager } from "./playground";
import { RunController } from "./run";
import { maybeRunSelftest, type SelftestContext } from "./selftest";
import { TabManager } from "./tabs";
import { toast } from "./toast";
import "./style.css";

const root = document.querySelector<HTMLDivElement>("#app")!;
const layout = createLayout(root);
const { tabStrip, conflictBar, editorHost, emptyState, panelHost, statusBar } = layout;

const outputHost = document.createElement("div");
outputHost.className = "output-host";
const panel = new BottomPanel();
panel.register({ id: "output", title: "Output", element: outputHost });
panelHost.append(panel.element);

const editor = new Editor(editorHost);

const tabs = new TabManager({
  editor,
  status: statusBar,
  tabStrip,
  conflictBar,
  editorHost,
  emptyState,
  outputHost,
});
const runs = new RunController(tabs);
tabs.runs = runs;
setTabManager(tabs);

const history = new RunHistory(tabs, runs);
const plugins = new PluginsPanel();
const library = new LibraryPanel(layout.sidebar, {
  open: (id) => tabs.open(id),
  activePlayground: () => tabs.activePlayground(),
  recent: () => tabs.recent(),
  closed: (id) => tabs.closed(id),
  moved: (id) => void tabs.languageMoved(id),
  languageName: (id) => tabs.languageName(id),
});

/** The update the launch check found, if any (work item 04-06). */
let updateWaiting: string | null = null;

subscribe(({ event }) => {
  if (event.type !== "updateAvailable") return;
  updateWaiting = event.version;
  toast("info", `TinkerPad ${event.version} is available.`, {
    label: "Install and restart",
    run: () => void installUpdate(),
  });
});

/** Download it and restart into it. The app goes away when this works. */
async function installUpdate(): Promise<void> {
  if (updateWaiting === null) return;
  await invoke("install_update").catch(() => {
    // The core has already said what went wrong.
  });
}

window.addEventListener("tinkerpad:toggle-library", () => library.toggle());
window.addEventListener("tinkerpad:open-settings", () => void openSettings(settingsHost));

/** What settings change in the window itself (work item 04-02). */
const settingsHost = {
  applyEditor: (settings: import("./protocol").Settings) => {
    editor.applySettings(
      settings.editor.font_size,
      settings.editor.font_family,
      settings.editor.tab_size,
    );
    applyTheme(themeName(settings.theme));
  },
  libraryMoved: async (_root: string) => {
    // Every open tab belongs to the folder that was there before.
    for (const tab of [...tabs.tabs]) await tabs.close(tab, { save: false });
    await tabs.refresh();
  },
};
onThemeChanged((theme) => editor.applyTheme(theme));
followSystem();
void watchSettings(settingsHost);

statusBar.onCancel = () => void runs.cancel();
statusBar.onShowHistory = (anchor) => void history.toggle(anchor);
statusBar.onOpenLogs = () => void invoke("open_log_folder").catch(() => {});

// Dev builds only: Mod+Shift+E renders one dump of every kind (01-08).
if (import.meta.env.DEV) {
  document.addEventListener(
    "keydown",
    (e) => {
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === "e") {
        e.preventDefault();
        e.stopPropagation();
        void renderExamples(tabs.activeOutput);
      }
    },
    true,
  );
}

/**
 * Every command the window has, and what each one does (work item 04-03).
 *
 * The catalog says what exists and what it is called; this says what it runs.
 * Keys, the palette, and `docs/KEYBINDINGS.md` all come from there, so adding
 * a shortcut is a line of data plus a line here.
 */
function registerEverything(): void {
  const text = () => (tabs.activeTab ? tabs.textOf(tabs.activeTab) : editor.text);
  registerCondition("playground", () => tabs.activePlayground() !== null);
  registerCommands({
    "run.start": () => void runs.run(tabs.activeId, text()),
    "run.cancel": () => void runs.cancel(),
    "run.clearOutput": () => tabs.activeOutput.clear(),
    "run.forcePerRun": () => void forcePerRun(),
    "run.history": () => void history.toggle(statusBar.runElement),

    "playground.new": () => void tabs.newPlayground(),
    "playground.newOfKind": () => void tabs.pickAndCreate(),
    "playground.open": () => void tabs.quickOpen(),
    "playground.rename": () => tabs.startRename(),
    "playground.addPackage": () => void tabs.addPackage(),

    "tab.close": () => void tabs.closeActive(),
    "tab.next": () => tabs.cycle(1),
    "tab.previous": () => tabs.cycle(-1),

    "go.nextProblem": () => tabs.cycleAnnotation(1),
    "go.previousProblem": () => tabs.cycleAnnotation(-1),
    "go.back": () => jumpBack(tabs),

    "view.library": () => library.toggle(),
    "view.commands": () => void openCommandPalette(),
    "view.tools": () => statusBar.tools.toggle(),
    "view.plugins": () => void plugins.toggle(),

    "settings.open": () => void openSettings(settingsHost),
    "settings.file": () => void invoke("open_settings_file").catch(() => {}),
    "settings.keybindings": () => void invoke("open_keybindings_file").catch(() => {}),
    "settings.copyKeybindings": () => void copyKeybindings(),
    "settings.logs": () => void invoke("open_log_folder").catch(() => {}),
    "app.update": {
      run: () => void installUpdate(),
      // Only there when there is one to install (work item 04-06).
      enabled: () => updateWaiting !== null,
    },
  });
}
registerEverything();
onBindingsChanged(() => editor.applyBindings());
void watchKeybindings();

/**
 * Keep runs off plugin helpers for the rest of the session, or let them back
 * on (work item 03-03). It's a toggle, so it says which way it went.
 */
async function forcePerRun(): Promise<void> {
  const now = await invoke<boolean>("force_per_run");
  const forced = await invoke<boolean>("force_per_run", { forced: !now });
  toast(
    "info",
    forced
      ? "Runs will start a new process each time, ignoring plugin helpers."
      : "Runs will use plugin helpers again.",
  );
}

/**
 * Keys, everywhere except inside the editor's own text: the editor gets the
 * same commands through CodeMirror, at a precedence that lets a completion
 * popup have Enter first (04-03).
 */
export function handleGlobalKey(e: KeyboardEvent): boolean {
  if (pickerOpen() || settingsOpen()) return false;
  const inTextField =
    e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement;
  const command = commandFor(e);
  if (!command) return false;
  // A plain key (F2, and anything a user binds without a modifier) belongs to
  // the field being typed in.
  if (inTextField && !(e.ctrlKey || e.metaKey || e.altKey)) return false;
  if (inEditor(e) && editorHandles(command.id)) return false;
  void runCommand(command.id);
  return true;
}

/** Whether the event happened inside the editor's text. */
function inEditor(e: KeyboardEvent): boolean {
  return e.target instanceof HTMLElement && e.target.closest(".cm-editor") !== null;
}

/** Commands CodeMirror runs itself, so they aren't run twice. */
function editorHandles(id: string): boolean {
  return EDITOR_COMMANDS.includes(id);
}

document.addEventListener(
  "keydown",
  (e) => {
    if (handleGlobalKey(e)) {
      e.preventDefault();
      e.stopPropagation();
    }
  },
  true,
);


// Unsaved edits are written before the window goes away. The writes are
// fired without waiting; each is atomic, so a cut-off one leaves the old file.
window.addEventListener("beforeunload", () => {
  for (const tab of tabs.tabs) {
    if (tab.dirty) void invoke("write_source", { id: tab.id, text: tabs.textOf(tab) }).catch(() => {});
  }
  void tabs.saveSession();
});

subscribe((env) => {
  const { event } = env;
  switch (event.type) {
    case "pong":
      console.info(`ui ready in ${event.readyMs} ms`);
      break;
    case "toast":
      toast(event.level, event.message);
      break;
    case "run":
      runs.handle(env);
      break;
    case "server":
    case "helper":
      statusBar.servers.changed();
      break;
    case "toolchain":
      statusBar.tools.changed();
      void plugins.refresh();
      break;
    case "pluginsChanged":
      // The languages changed under the window: the panel, the status strip's
      // list of what is missing, and the tabs all want to know (05-02).
      void plugins.refresh();
      statusBar.tools.changed();
      void tabs.refreshListing().catch(() => {});
      break;
    case "tool":
      statusBar.tools.progress(event.plugin, event.line);
      break;
    case "libraryChanged":
      void tabs.libraryChanged().catch(() => {});
      break;
    case "sourceChangedOnDisk":
      void tabs.sourceChanged(event.playground).catch(() => {});
      break;
  }
});

await ready();
await invoke("ping");

// The language servers in the status bar refresh themselves from here on.
statusBar.servers.start();
// And what each language needs, which only changes when a check does.
void statusBar.tools.refresh();

// A run that failed for want of a tool is exactly when an offer is wanted, and
// the only time one arrives unasked (work item 04-08b).
runs.onFailed = (playground) => {
  const language = tabs.summary(playground)?.language;
  if (language) void statusBar.tools.offerFor(language);
};

// Self-tests start from no tabs (unless they're testing session restore);
// normal launches reopen the last session.
const selftest = await invoke<string | null>("selftest_scenario");
if (!selftest || selftest.split(",").includes("session-restore")) {
  await tabs.restoreSession().catch(() => tabs.showEmpty());
} else {
  await tabs.refresh().catch(() => {});
}
if (tabs.activeTab) editor.focus();

const context: SelftestContext = {
  editor,
  get output() {
    return tabs.activeOutput;
  },
  runs,
  status: statusBar,
  tabs,
  handleGlobalKey,
};
await maybeRunSelftest(context);
