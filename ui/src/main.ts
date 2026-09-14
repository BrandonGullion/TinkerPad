import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const app = document.querySelector<HTMLDivElement>("#app")!;
app.textContent = "TinkerPad";

// Seed round-trip for work item 00-05: invoke `ping`, expect a `pong` event.
await listen<string>("pong", (event) => {
  app.textContent = `TinkerPad (${event.payload})`;
});
await invoke("ping");
