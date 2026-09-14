import { invoke, ready, subscribe } from "./ipc";
import { toast } from "./toast";
import "./style.css";

const app = document.querySelector<HTMLDivElement>("#app")!;
app.textContent = "TinkerPad";

subscribe(({ event }) => {
  switch (event.type) {
    case "pong":
      app.textContent = `TinkerPad (ready in ${event.readyMs} ms)`;
      break;
    case "toast":
      toast(event.level, event.message);
      break;
  }
});

await ready();
await invoke("ping");
