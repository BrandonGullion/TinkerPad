import { Editor } from "./editor";
import { invoke, ready, subscribe } from "./ipc";
import { createLayout } from "./layout";
import { OutputPane } from "./output";
import { BottomPanel } from "./panels";
import { maybeRunSelftest } from "./selftest";
import { toast } from "./toast";
import "./style.css";

const root = document.querySelector<HTMLDivElement>("#app")!;
const { editorHost, panelHost, statusBar } = createLayout(root);

const output = new OutputPane(() => editor.focus());
const panel = new BottomPanel();
panel.register({ id: "output", title: "Output", element: output.element });
panelHost.append(panel.element);

const editor = new Editor(
  editorHost,
  {
    run: () => requestRun(),
    clearOutput: () => output.clear(),
  },
  'console.log("hello from TinkerPad");\n',
);
editor.focus();

// Running arrives in work item 00-06; for now, acknowledge the shortcut.
function requestRun(): void {
  output.append("info", "Run requested (not wired up yet)\n");
}

// App shortcuts that must work when focus is outside the editor too.
document.addEventListener("keydown", (e) => {
  const mod = e.metaKey || e.ctrlKey;
  if (!mod) return;
  // Inside the editor, CodeMirror's keymap already handles these.
  const inEditor = e.target instanceof HTMLElement && e.target.closest(".cm-editor") !== null;
  if (inEditor) return;
  if (e.key === "Enter") {
    e.preventDefault();
    requestRun();
  } else if (e.key.toLowerCase() === "l") {
    e.preventDefault();
    output.clear();
  }
});

subscribe(({ event }) => {
  switch (event.type) {
    case "pong":
      statusBar.setLanguage(`Plain text · ready in ${event.readyMs} ms`);
      break;
    case "toast":
      toast(event.level, event.message);
      break;
  }
});

await ready();
await invoke("ping");
await maybeRunSelftest({ editor, output });
