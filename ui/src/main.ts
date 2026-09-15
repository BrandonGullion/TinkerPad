import { renderExamples } from "./dump/examples";
import { Editor } from "./editor";
import { invoke, ready, subscribe } from "./ipc";
import { createLayout } from "./layout";
import { OutputPane } from "./output";
import { BottomPanel } from "./panels";
import { currentPlayground, openMostRecent } from "./playground";
import { RunController } from "./run";
import { maybeRunSelftest } from "./selftest";
import { toast } from "./toast";
import "./style.css";

const root = document.querySelector<HTMLDivElement>("#app")!;
const { editorHost, panelHost, statusBar } = createLayout(root);

const output = new OutputPane(() => editor.focus());
const panel = new BottomPanel();
panel.register({ id: "output", title: "Output", element: output.element });
panelHost.append(panel.element);

const runs = new RunController(output, statusBar);
statusBar.onCancel = () => void runs.cancel();
statusBar.onOpenLogs = () => void invoke("open_log_folder").catch(() => {});

/** Shown until a playground is open. Creating and opening playgrounds from
 * the UI arrives with tabs (01-12); bundled language plugins with 01-09. */
const NO_PLAYGROUND_TEXT = `// No playground is open.
//
// Runs now use each language's plugin. Bundled plugins arrive with work item
// 01-09, and creating and opening playgrounds from here with 01-12. Until then
// the most recently used playground opens at startup, if its plugin is loaded.
`;

const runCurrent = () => void runs.run(currentPlayground(), editor.text);

const editor = new Editor(
  editorHost,
  {
    run: runCurrent,
    cancel: () => void runs.cancel(),
    clearOutput: () => output.clear(),
  },
  NO_PLAYGROUND_TEXT,
);
statusBar.setLanguage("No playground");
editor.focus();

// Dev builds only: Mod+Shift+E renders one dump of every kind (01-08).
if (import.meta.env.DEV) {
  document.addEventListener(
    "keydown",
    (e) => {
      if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === "e") {
        e.preventDefault();
        e.stopPropagation();
        void renderExamples(output);
      }
    },
    true,
  );
}

// App shortcuts that must work when focus is outside the editor too. Inside
// the editor, CodeMirror's keymap handles them.
document.addEventListener("keydown", (e) => {
  const mod = e.metaKey || e.ctrlKey;
  if (!mod) return;
  const inEditor = e.target instanceof HTMLElement && e.target.closest(".cm-editor") !== null;
  if (inEditor) return;
  if (e.key === "Enter") {
    e.preventDefault();
    runCurrent();
  } else if (e.key === ".") {
    e.preventDefault();
    void runs.cancel();
  } else if (e.key.toLowerCase() === "l") {
    e.preventDefault();
    output.clear();
  }
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
  }
});

await ready();
await invoke("ping");

// Self-tests set up their own playgrounds; otherwise reopen the latest one.
const selftest = await invoke<string | null>("selftest_scenario");
if (!selftest) {
  await openMostRecent(editor, statusBar).catch(() => false);
}
await maybeRunSelftest({ editor, output, runs, status: statusBar });
