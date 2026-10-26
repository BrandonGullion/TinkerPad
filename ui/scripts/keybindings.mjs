// Write docs/KEYBINDINGS.md from the command catalog (work item 04-03).
//
// Run with `npm run docs:keys` from `ui/`. It imports the same catalog the app
// does — plain JavaScript for exactly this reason — so the document can't
// drift from the keymap. CI runs it and fails if the file would change.

import { writeFileSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { CATALOG, CATEGORIES, defaultBindings } from "../src/commands/catalog.js";

const here = dirname(fileURLToPath(import.meta.url));
const out = join(here, "..", "..", "docs", "KEYBINDINGS.md");

/** A binding in a platform's own spelling: `⌘⇧P`, or `Ctrl+Shift+P`. */
function format(binding, mac) {
  const parts = binding.split("+").map((part) => part.trim().toLowerCase());
  const key = parts.pop() ?? "";
  const has = (name) => parts.includes(name);
  const mod = has("mod");
  const pieces = [];
  if (mac) {
    if (has("ctrl")) pieces.push("⌃");
    if (has("alt")) pieces.push("⌥");
    if (has("shift")) pieces.push("⇧");
    if (mod || has("cmd")) pieces.push("⌘");
    return `${pieces.join("")}${keyName(key, mac)}`;
  }
  if (mod || has("ctrl")) pieces.push("Ctrl");
  if (has("cmd")) pieces.push("Win");
  if (has("alt")) pieces.push("Alt");
  if (has("shift")) pieces.push("Shift");
  pieces.push(keyName(key, mac));
  return pieces.join("+");
}

function keyName(key, mac) {
  const arrows = { arrowleft: "←", arrowright: "→", arrowup: "↑", arrowdown: "↓" };
  const words = { arrowleft: "Left", arrowright: "Right", arrowup: "Up", arrowdown: "Down" };
  if (key in arrows) return mac ? arrows[key] : words[key];
  if (key === "enter") return mac ? "↩" : "Enter";
  if (key === "tab") return mac ? "⇥" : "Tab";
  if (key === "escape") return "Esc";
  if (key.length === 1) return key.toUpperCase();
  return key.charAt(0).toUpperCase() + key.slice(1);
}

const bindings = defaultBindings();
const lines = [
  "# Keybindings",
  "",
  "Generated from `ui/src/commands/catalog.js` by `npm run docs:keys` (run from",
  "`ui/`). Don't edit this file; edit the catalog and run it again.",
  "",
  "Every shortcut can be changed in `<app-data>/keybindings.toml`, one line per",
  "command: one binding, a list of them, or an empty string to take its keys",
  "away:",
  "",
  "```toml",
  '"run.start" = "mod+shift+r"',
  '"playground.new" = ["mod+n", "mod+t"]',
  '"tab.close" = ""',
  "```",
  "",
  "`Mod` is Cmd on macOS and Ctrl everywhere else; write `ctrl` or `cmd` for a",
  "binding meant for one platform only.",
  "",
];

for (const category of CATEGORIES) {
  const rows = CATALOG.filter((command) => command.category === category);
  if (rows.length === 0) continue;
  lines.push(`## ${category}`, "", "| Command | macOS | Windows and Linux | Id |", "|---|---|---|---|");
  for (const command of rows) {
    const keys = bindings[command.id] ?? [];
    const mac = keys.length ? keys.map((key) => format(key, true)).join(" / ") : "—";
    const other = keys.length ? keys.map((key) => format(key, false)).join(" / ") : "—";
    lines.push(`| ${command.title} | ${mac} | ${other} | \`${command.id}\` |`);
  }
  lines.push("");
}

const text = `${lines.join("\n")}`;
const check = process.argv.includes("--check");
if (check) {
  const current = readFileSync(out, "utf8");
  if (current !== text) {
    console.error("docs/KEYBINDINGS.md is out of date; run `npm run docs:keys` in ui/");
    process.exit(1);
  }
  console.log("docs/KEYBINDINGS.md is up to date");
} else {
  writeFileSync(out, text);
  console.log(`wrote ${out}`);
}
