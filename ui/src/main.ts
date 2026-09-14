import { Editor } from "./editor";
import { invoke, ready, subscribe } from "./ipc";
import { createLayout } from "./layout";
import { OutputPane } from "./output";
import { BottomPanel } from "./panels";
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
statusBar.setLanguage("JavaScript (node)");
statusBar.onCancel = () => void runs.cancel();

const editor = new Editor(
  editorHost,
  {
    run: () => void runs.run(editor.text),
    cancel: () => void runs.cancel(),
    clearOutput: () => output.clear(),
  },
  'console.log("hello from TinkerPad");\n',
);
editor.focus();

// App shortcuts that must work when focus is outside the editor too. Inside
// the editor, CodeMirror's keymap handles them.
document.addEventListener("keydown", (e) => {
  const mod = e.metaKey || e.ctrlKey;
  if (!mod) return;
  const inEditor = e.target instanceof HTMLElement && e.target.closest(".cm-editor") !== null;
  if (inEditor) return;
  if (e.key === "Enter") {
    e.preventDefault();
    void runs.run(editor.text);
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
await maybeRunSelftest({ editor, output, runs });
