import { renderExamples } from "./dump/examples";
import { Editor } from "./editor";
import { invoke, ready, subscribe } from "./ipc";
import { createLayout } from "./layout";
import { BottomPanel } from "./panels";
import { pickerOpen } from "./picker";
import { setTabManager } from "./playground";
import { RunController } from "./run";
import { maybeRunSelftest, type SelftestContext } from "./selftest";
import { TabManager } from "./tabs";
import { toast } from "./toast";
import "./style.css";

const root = document.querySelector<HTMLDivElement>("#app")!;
const { tabStrip, conflictBar, editorHost, emptyState, panelHost, statusBar } = createLayout(root);

const outputHost = document.createElement("div");
outputHost.className = "output-host";
const panel = new BottomPanel();
panel.register({ id: "output", title: "Output", element: outputHost });
panelHost.append(panel.element);

const editor = new Editor(editorHost, {
  run: () => void runs.run(tabs.activeId, editor.text),
  cancel: () => void runs.cancel(),
  clearOutput: () => tabs.activeOutput.clear(),
});

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

statusBar.onCancel = () => void runs.cancel();
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

const isMac = navigator.platform.toLowerCase().includes("mac");

/**
 * Tab and playground shortcuts. Handled before CodeMirror sees the key, so
 * they work wherever focus is. `Mod` is Cmd on macOS and Ctrl elsewhere;
 * Ctrl+Tab is Ctrl everywhere, as in browsers.
 */
export function handleGlobalKey(e: KeyboardEvent): boolean {
  if (pickerOpen()) return false;
  const mod = isMac ? e.metaKey : e.ctrlKey;
  const key = e.key.toLowerCase();
  const inTextField = e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement;

  if (e.ctrlKey && e.key === "Tab") {
    tabs.cycle(e.shiftKey ? -1 : 1);
    return true;
  }
  if (e.key === "F2" && !mod && !inTextField) {
    tabs.startRename();
    return true;
  }
  // F8 walks the last run's errors and warnings (01-13).
  if (e.key === "F8" && !mod) {
    tabs.cycleAnnotation(e.shiftKey ? -1 : 1);
    return true;
  }
  if (!mod || e.altKey) return false;
  switch (key) {
    case "n":
      void (e.shiftKey ? tabs.pickAndCreate() : tabs.newPlayground());
      return true;
    case "o":
      if (e.shiftKey) return false;
      void tabs.quickOpen();
      return true;
    case "w":
      if (e.shiftKey) return false;
      void tabs.closeActive();
      return true;
    default:
      return false;
  }
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

// App shortcuts that must work when focus is outside the editor too. Inside
// the editor, CodeMirror's keymap handles them.
document.addEventListener("keydown", (e) => {
  const mod = e.metaKey || e.ctrlKey;
  if (!mod || pickerOpen()) return;
  const inEditor = e.target instanceof HTMLElement && e.target.closest(".cm-editor") !== null;
  if (inEditor) return;
  if (e.key === "Enter") {
    e.preventDefault();
    void runs.run(tabs.activeId, editor.text);
  } else if (e.key === ".") {
    e.preventDefault();
    void runs.cancel();
  } else if (e.key.toLowerCase() === "l") {
    e.preventDefault();
    tabs.activeOutput.clear();
  }
});

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
