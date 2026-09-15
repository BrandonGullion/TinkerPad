// The dev-only "render examples" command: one dump of every kind in the output
// pane, for checking the renderers by eye. Press Mod+Shift+E in a dev build
// (`tauri dev`).

import { invoke } from "../ipc";
import type { OutputPane } from "../output";
import type { RichValue } from "../protocol";

/** Values the core never sends in `dump::examples()` but can send at runtime. */
const EXTRA: RichValue[] = [
  { v: 1, kind: "unknown", raw: { v: 2, kind: "chart", series: [3, 1, 4, 1, 5] } },
  { v: 1, kind: "truncated", bytes: 9_000_000 },
];

export async function renderExamples(output: OutputPane): Promise<RichValue[]> {
  const examples = [...(await invoke<RichValue[]>("dump_examples")), ...EXTRA];
  output.append("info", "Dump examples: one of every kind\n");
  for (const value of examples) {
    output.append("stdout", `\n${value.kind}: `);
    output.appendRich(value);
  }
  output.append("stdout", "\n");
  return examples;
}
