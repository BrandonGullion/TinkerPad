// Opt-in in-app self-test, used to verify UI behaviour from scripts and CI
// without sending keystrokes to the desktop. Enabled only when the app is
// started with TINKERPAD_SELFTEST set; results go to the app's stdout as
// "selftest: PASS ..." / "selftest: FAIL ..." lines, ending with
// "selftest: done".

import { undo } from "@codemirror/commands";
import { EditorSelection } from "@codemirror/state";
import { runScopeHandlers } from "@codemirror/view";

import { clipboard } from "./dump/copy";
import { renderExamples } from "./dump/examples";
import { renderRich } from "./dump/render";
import { CHUNK_ROWS } from "./dump/table";
import { TREE_PAGE } from "./dump/tree";
import type { Editor } from "./editor";
import { invoke, subscribe } from "./ipc";
import type { StatusBar } from "./layout";
import { LOCKED_HINT, MISMATCH_NOTE, openPlayground } from "./playground";
import type { OutputPane } from "./output";
import type { LanguageToolchain, Listing, Playground, PluginsStatus, RunEvent } from "./protocol";
import type { RunController } from "./run";

export interface SelftestContext {
  editor: Editor;
  output: OutputPane;
  runs: RunController;
  status: StatusBar;
}

const isMac = navigator.platform.toLowerCase().includes("mac");

/** A keydown as the user would press it, with Mod mapped per platform. */
function key(k: string, mods: { mod?: boolean; shift?: boolean } = {}): KeyboardEvent {
  return new KeyboardEvent("keydown", {
    key: k,
    bubbles: true,
    cancelable: true,
    ctrlKey: !isMac && !!mods.mod,
    metaKey: isMac && !!mods.mod,
    shiftKey: !!mods.shift,
  });
}

const frame = () => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));

async function report(line: string): Promise<void> {
  await invoke("selftest_report", { line });
}

export async function maybeRunSelftest(ctx: SelftestContext): Promise<void> {
  const scenario = await invoke<string | null>("selftest_scenario");
  if (!scenario) return;

  let failures = 0;
  const check: Check = async (name, ok, detail = "") => {
    if (!ok) failures++;
    await report(`${ok ? "PASS" : "FAIL"} ${name}${detail ? ` (${detail})` : ""}`);
  };

  for (const name of scenario.split(",").map((s) => s.trim())) {
    try {
      const run = SCENARIOS[name];
      if (!run) {
        await check(`unknown scenario ${name}`, false);
        continue;
      }
      await report(`scenario ${name}`);
      await run(ctx, check);
    } catch (e) {
      await check(`${name} threw`, false, String(e));
    }
  }
  await report(`done failures=${failures}`);
}

type Check = (name: string, ok: boolean, detail?: string) => Promise<void>;

async function layoutScenario(ctx: SelftestContext, check: Check) {
  const { editor, output, runs } = ctx;
  await openTestPlayground(ctx, "Layout Test");
  const view = editor.view;
  const outText = () => output.element.querySelector(".output-body")?.textContent ?? "";

  // Typing into the editor.
  view.dispatch({ changes: { from: view.state.doc.length, insert: "const x = [1, 2, 3];\n" } });
  await check("editor accepts input", editor.text.includes("const x = [1, 2, 3];"));

  // Mod+Enter through CodeMirror's real keymap starts a run.
  const end = runs.waitForEnd();
  const handled = runScopeHandlers(view, key("Enter", { mod: true }), "editor");
  await check("Mod+Enter handled by keymap", handled);
  await end;
  await output.flushed();

  // Mod+L clears output (overriding CodeMirror's select-line).
  runScopeHandlers(view, key("l", { mod: true }), "editor");
  await frame();
  await check("Mod+L clears output", outText() === "");

  // Stress: 10,000 lines in 1,000 chunks.
  const start = performance.now();
  let n = 0;
  for (let chunk = 0; chunk < 1000; chunk++) {
    let text = "";
    for (let i = 0; i < 10; i++) text += `line ${++n} of 10000\n`;
    output.append(chunk % 7 === 0 ? "stderr" : "stdout", text);
  }
  await output.flushed();
  const ms = performance.now() - start;
  // Timing is informational: CI renders in software and would be flaky.
  await report(`INFO 10k lines in 1k chunks rendered in ${ms.toFixed(1)} ms`);
  await check(
    "stderr styled separately",
    output.element.querySelector(".out-stderr") !== null &&
      output.element.querySelector(".out-stdout") !== null,
  );

  // Auto-scroll sticks to the bottom while at the bottom.
  const el = output.element;
  const atBottom = () => el.scrollHeight - el.scrollTop - el.clientHeight <= 4;
  await check("auto-scrolls while at bottom", atBottom());

  // Scrolling up stops auto-scroll and shows the jump button.
  el.scrollTop = 0;
  el.dispatchEvent(new Event("scroll"));
  output.append("stdout", "more\n");
  await output.flushed();
  const jump = el.querySelector<HTMLButtonElement>(".output-jump")!;
  await check("scrolled up stays put", el.scrollTop === 0 && !jump.hidden);

  // Jump to bottom resumes.
  jump.click();
  await frame();
  await check("jump to bottom resumes", atBottom() && jump.hidden === true);

  // Line cap: push past 20,000 lines and expect a trimmed notice.
  let big = "";
  for (let i = 0; i < 15_000; i++) big += "x\n";
  output.append("stdout", big);
  await output.flushed();
  const notice = el.querySelector<HTMLElement>(".output-trimmed")!;
  await check("caps DOM lines", !notice.hidden, notice.textContent ?? "");

  // Escape in the output pane returns focus to the editor.
  el.focus();
  el.dispatchEvent(key("Escape"));
  await frame();
  // Check DOM focus rather than view.hasFocus, which also needs OS window
  // focus and would be flaky on headless CI.
  await check("Escape returns focus to editor", view.contentDOM.contains(document.activeElement));

  // Tabs will swap states in one view: undo history must survive a swap.
  const a = view.state;
  view.dispatch({ changes: { from: 0, insert: "// edited\n" } });
  const edited = view.state;
  editor.setState(editor.createState("other tab"));
  editor.setState(edited);
  await check("state swap keeps document", editor.text.startsWith("// edited"));
  void a;

  // The page itself never scrolls horizontally; long lines scroll inside the editor.
  const doc = document.documentElement;
  await check("no page-level horizontal overflow", doc.scrollWidth <= doc.clientWidth);
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** Put code in the editor and press Mod+Enter through the real keymap. */
function runCode({ editor }: SelftestContext, code: string): void {
  const view = editor.view;
  view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: code } });
  runScopeHandlers(view, key("Enter", { mod: true }), "editor");
}

async function withTimeout<T>(p: Promise<T>, ms: number, what: string): Promise<T> {
  let timer = 0;
  const timeout = new Promise<never>((_, reject) => {
    timer = window.setTimeout(() => reject(new Error(`${what} timed out after ${ms} ms`)), ms);
  });
  try {
    return await Promise.race([p, timeout]);
  } finally {
    clearTimeout(timer);
  }
}

async function runScenario(ctx: SelftestContext, check: Check) {
  const { output, runs, editor } = ctx;
  const main = await openTestPlayground(ctx, "Run Test");
  const outText = () => output.element.querySelector(".output-body")?.textContent ?? "";
  const status = () => document.querySelector(".status-run")?.textContent ?? "";

  // console.log(1) end to end.
  let end = runs.waitForEnd();
  runCode(ctx, "console.log(1)\n");
  let outcome = await withTimeout(end, 15_000, "console.log(1)");
  await output.flushed();
  await check(
    "console.log(1) prints 1 and exits 0",
    outcome.kind === "exited" && outcome.code === 0 && outText() === "1\n",
    JSON.stringify({ outcome, out: outText() }),
  );
  await check("status bar shows exit 0", status().startsWith("Exit 0"), status());
  await report(`INFO Mod+Enter to first output: ${runs.firstOutputMs?.toFixed(1)} ms`);

  // Warm repeat, for a second timing sample.
  end = runs.waitForEnd();
  runCode(ctx, "console.log(2)\n");
  await withTimeout(end, 15_000, "console.log(2)");
  await report(`INFO second run, Mod+Enter to first output: ${runs.firstOutputMs?.toFixed(1)} ms`);

  // stderr and a non-zero exit code.
  end = runs.waitForEnd();
  runCode(ctx, 'console.error("oops"); process.exit(2)\n');
  outcome = await withTimeout(end, 15_000, "exit 2");
  await output.flushed();
  await check(
    "stderr shown and non-zero exit reported",
    outcome.kind === "exited" &&
      outcome.code === 2 &&
      (output.element.querySelector(".out-stderr")?.textContent ?? "").includes("oops") &&
      status().startsWith("Exit 2"),
    JSON.stringify({ outcome, status: status() }),
  );

  // UTF-8 survives the process boundary.
  end = runs.waitForEnd();
  runCode(ctx, 'console.log("héllo wörld → 日本 🎉")\n');
  await withTimeout(end, 15_000, "utf8");
  await output.flushed();
  await check("UTF-8 output intact", outText() === "héllo wörld → 日本 🎉\n", outText());

  // An infinite loop can be cancelled with Mod+.
  end = runs.waitForEnd();
  runCode(ctx, "while (true) {}\n");
  await sleep(500);
  const cancelButton = document.querySelector<HTMLButtonElement>(".status-cancel")!;
  await check("cancel button visible while running", cancelButton.hidden === false);
  runScopeHandlers(editor.view, key(".", { mod: true }), "editor");
  outcome = await withTimeout(end, 10_000, "cancel");
  await check(
    "Mod+. cancels an infinite loop",
    outcome.kind === "cancelled" && status().startsWith("Cancelled"),
    JSON.stringify({ outcome, status: status() }),
  );

  // Run while running: the first run is cancelled and its output never
  // appears after the second starts.
  runCode(ctx, 'setInterval(() => console.log("A"), 10)\n');
  await sleep(400);
  end = runs.waitForEnd();
  runCode(ctx, 'console.log("B")\n');
  outcome = await withTimeout(end, 15_000, "run while running");
  await sleep(200); // give any stray "A" a chance to show up
  await output.flushed();
  await check(
    "run while running replaces the old run cleanly",
    outcome.kind === "exited" && outText() === "B\n",
    JSON.stringify({ outcome, out: outText().slice(0, 80) }),
  );

  // The run executed the saved file: what's on disk is what ran.
  await check(
    "run saves the playground first",
    (await invoke<string>("read_source", { id: main.id })) === 'console.log("B")\n',
  );

  // Different playgrounds can run at the same time.
  const ended = new Map<number, string>();
  const unsubscribe = subscribe(({ scope, event }) => {
    if (event.type === "run" && ["exited", "cancelled", "failed"].includes(event.kind)) {
      ended.set(Number(scope.slice(4)), event.kind === "failed" ? `failed: ${event.message}` : event.kind);
    }
  });
  const other = await invoke<Playground>("create_playground", {
    name: "Run Test Other",
    language: "node-check",
  });
  const longRun = await invoke<number>("run", {
    playground: main.id,
    text: "setInterval(() => {}, 1000)\n",
  });
  const shortRun = await invoke<number>("run", {
    playground: other.id,
    text: 'console.log("other")\n',
  });
  for (let i = 0; i < 100 && !ended.has(shortRun); i++) await sleep(50);
  await check(
    "runs in different playgrounds overlap",
    ended.get(shortRun) === "exited" && !ended.has(longRun),
    JSON.stringify([...ended]),
  );
  await invoke("cancel", { playground: main.id });
  for (let i = 0; i < 100 && !ended.has(longRun); i++) await sleep(50);
  await check("cancel stops only that playground's run", ended.get(longRun) === "cancelled");

  // A plugin whose toolchain is missing fails with the reason instead of running.
  const plugins = await invoke<PluginsStatus>("list_plugins");
  if (plugins.plugins.some((p) => p.id === "mini")) {
    const broken = await invoke<Playground>("create_playground", {
      name: "Run Test Missing",
      language: "mini",
    });
    const failedRun = await invoke<number>("run", { playground: broken.id, text: "x\n" });
    for (let i = 0; i < 100 && !ended.has(failedRun); i++) await sleep(50);
    const result = ended.get(failedRun) ?? "";
    await check(
      "missing toolchain fails with the reason",
      result.startsWith("failed:") && result.includes("isn't ready") && result.includes("not found"),
      result,
    );
  }
  unsubscribe();
}

/** A program that writes dump sentinels the way a dump library would, plus the
 * awkward cases: bad JSON, a payload far bigger than one pipe read, and the ST
 * terminator. */
const DUMP_PROGRAM = String.raw`const dump = (v) => process.stdout.write("\x1b]1337;TinkerPad=" + JSON.stringify(v) + "\x07");
process.stdout.write("before ");
dump({ v: 1, kind: "value", value: 42, type: "int", title: "answer" });
process.stdout.write(" after\n");
process.stdout.write("\x1b]1337;TinkerPad={broken\x07");
const rows = Array.from({ length: 20000 }, (_, i) => [i, "row " + i]);
dump({ v: 1, kind: "table", title: "rows", columns: [{ name: "id" }, { name: "name" }], rows, truncated: false });
process.stdout.write("\x1b]1337;TinkerPad=" + JSON.stringify({ kind: "text", text: "st" }) + "\x1b\\");
console.log("end");
`;

async function dumpScenario(ctx: SelftestContext, check: Check) {
  const { output, runs } = ctx;
  await openTestPlayground(ctx, "Dump Test");
  const seen: { scope: string; event: RunEvent }[] = [];
  const unsubscribe = subscribe(({ scope, event }) => {
    if (event.type !== "run") return;
    const { type: _type, ...rest } = event;
    seen.push({ scope, event: rest as RunEvent });
  });
  const end = runs.waitForEnd();
  runCode(ctx, DUMP_PROGRAM);
  const outcome = await withTimeout(end, 15_000, "dump program");
  await output.flushed();
  unsubscribe();

  const scope = seen.filter((s) => s.event.kind === "exited").at(-1)?.scope;
  // Adjacent text is merged: how it's chunked depends on the pipe.
  const events: RunEvent[] = [];
  for (const { event } of seen.filter((s) => s.scope === scope && s.event.kind !== "started")) {
    const last = events.at(-1);
    if (last && (last.kind === "stdout" || last.kind === "stderr") && last.kind === event.kind) {
      last.text += (event as { text: string }).text;
    } else {
      events.push(structuredClone(event));
    }
  }
  const describe = (e: RunEvent) => {
    switch (e.kind) {
      case "stdout":
      case "stderr":
        return `${e.kind} ${JSON.stringify(e.text)}`;
      case "rich":
        return `rich ${e.value.kind}${e.value.title ? ` ${e.value.title}` : ""}`;
      default:
        return e.kind;
    }
  };
  const actual = events.map(describe);
  const expected = [
    'stdout "before "',
    "rich value answer",
    'stdout " after\\n"',
    'stderr "{broken"',
    "warning",
    "rich table rows",
    "rich text",
    'stdout "end\\n"',
    "exited",
  ];
  await check(
    "dumped values arrive in order with the text around them",
    outcome.kind === "exited" && JSON.stringify(actual) === JSON.stringify(expected),
    JSON.stringify(actual),
  );

  const rich = events.filter((e) => e.kind === "rich").map((e) => e.value);
  const value = rich.find((v) => v.kind === "value");
  await check(
    "a value dump keeps its fields",
    value?.kind === "value" && value.value === 42 && value.type === "int" && value.v === 1,
    JSON.stringify(value),
  );
  const table = rich.find((v) => v.kind === "table");
  await check(
    "a dump bigger than one pipe read parses whole",
    table?.kind === "table" && table.rows.length === 20000 && table.rows[19999]?.[1] === "row 19999",
    table?.kind === "table" ? `${table.rows.length} rows` : "no table",
  );
  const warning = events.find((e) => e.kind === "warning");
  await check(
    "malformed dump shows its text and a warning",
    warning?.kind === "warning" && warning.message.includes("couldn't be read"),
    JSON.stringify(warning),
  );
  const text = rich.find((v) => v.kind === "text");
  await check("ST-terminated dump parses", text?.kind === "text" && text.text === "st", JSON.stringify(text));

  // The pane shows them as blocks, in order with the text.
  const body = output.element.querySelector(".output-body")!;
  const blocks = [...body.children].map((c) =>
    c.classList.contains("dump") ? `dump:${(c as HTMLElement).dataset.kind}` : (c as HTMLElement).dataset.stream,
  );
  const tableFoot = body.querySelector(".dump-kind-table .dump-table-foot")?.textContent ?? "";
  await check(
    "output pane renders dumps as blocks between the text",
    JSON.stringify(blocks) ===
      JSON.stringify(["stdout", "dump:value", "stdout", "stderr", "info", "dump:table", "dump:text", "stdout"]) &&
      tableFoot.startsWith("20,000 rows"),
    JSON.stringify({ blocks, tableFoot }),
  );
}

const frames = async (n: number) => {
  for (let i = 0; i < n; i++) await frame();
};

/** Wait until `ok()` holds, checking every frame, up to `ms`. */
async function until(ok: () => boolean, ms = 3000): Promise<boolean> {
  const end = performance.now() + ms;
  while (!ok()) {
    if (performance.now() > end) return false;
    await frame();
  }
  return true;
}

async function dumpRenderScenario(ctx: SelftestContext, check: Check) {
  const { output } = ctx;
  const pane = output.element;
  output.clear();
  const titled = (kind: string, title: string) =>
    [...pane.querySelectorAll<HTMLElement>(`.dump-kind-${kind}`)].find(
      (d) => d.querySelector(":scope > .dump-head > .dump-title")?.textContent === title,
    );

  // Every kind, from the dev "render examples" command.
  const examples = await renderExamples(output);
  await output.flushed();
  const missing = examples.map((e) => e.kind).filter((k) => !pane.querySelector(`.dump-kind-${k}`));
  await check(
    "render examples shows one of every kind",
    examples.length === 9 && missing.length === 0,
    `${examples.length} examples, missing ${JSON.stringify(missing)}`,
  );
  const captions = [...pane.querySelectorAll(".dump-title")].map((t) => t.textContent);
  await check(
    "titles render as captions",
    ["answer", "users", "config"].every((t) => captions.includes(t)),
    JSON.stringify(captions),
  );

  const value = titled("value", "answer");
  await check(
    "value shows the literal and a type badge",
    value?.querySelector(".j-num")?.textContent === "42" && value.querySelector(".dump-type")?.textContent === "int",
    value?.textContent ?? "missing",
  );
  const timing = pane.querySelector<HTMLElement>(".dump-kind-timing");
  await check(
    "timing is a small inline badge",
    !!timing?.classList.contains("dump-inline") && timing.querySelector(".timing-ms")?.textContent === "12.5 ms",
    timing?.textContent ?? "missing",
  );
  const text = pane.querySelector<HTMLElement>(".dump-text");
  await check(
    "text with a bundled highlighter is highlighted",
    text?.dataset.lang === "json" && text.querySelector(".tok-propertyName") !== null,
    text?.innerHTML.slice(0, 120) ?? "missing",
  );
  const plainText = renderRich({ v: 1, kind: "text", text: "fn main() {}", lang: "rust" }, { scrollRoot: pane });
  await check(
    "text without a bundled highlighter is plain",
    plainText.querySelector(".dump-text")?.children.length === 0 &&
      plainText.querySelector(".dump-text")?.textContent === "fn main() {}",
  );
  const htmlHost = pane.querySelector<HTMLElement>(".dump-html");
  await check(
    "html renders in its own isolated root",
    htmlHost?.shadowRoot?.querySelector("strong")?.textContent === "world" &&
      getComputedStyle(htmlHost).overflowY === "auto",
  );

  const img = pane.querySelector<HTMLImageElement>(".dump-image")!;
  await until(() => img.complete);
  await check(
    "image shows inline with a max height",
    img.naturalWidth === 160 && getComputedStyle(img).maxHeight === "240px",
    `${img.naturalWidth}px, max ${getComputedStyle(img).maxHeight}`,
  );
  img.click();
  await check("clicking an image expands it", getComputedStyle(img).maxHeight === "none");

  const unknown = pane.querySelector(".dump-kind-unknown");
  await check(
    "an unknown kind shows its raw JSON",
    !!unknown?.querySelector(".dump-tree") && (unknown.textContent ?? "").includes('"chart"'),
  );
  await check(
    "a truncated dump says it was too large",
    (pane.querySelector(".dump-kind-truncated")?.textContent ?? "").includes("too large"),
  );

  // Tables: headers, sorting, nested cells, clipping, copy.
  const users = titled("table", "users")!;
  const column = (table: Element, c: number) =>
    [...table.querySelectorAll("tbody tr:not(.dump-placeholder)")].map((tr) => tr.children[c]?.textContent ?? "");
  const header = (table: Element, c: number) => table.querySelectorAll<HTMLElement>("th")[c]!;
  await check(
    "table header shows names and type hints",
    header(users, 0).querySelector(".col-name")?.textContent === "id" &&
      header(users, 0).querySelector(".dump-type")?.textContent === "int",
  );
  await check(
    "table shows its row count",
    users.querySelector(".dump-table-foot")?.textContent === "3 rows",
    users.querySelector(".dump-table-foot")?.textContent ?? "",
  );
  header(users, 0).click();
  const ascending = column(users, 0);
  header(users, 0).click();
  const descending = column(users, 0);
  header(users, 0).click();
  const original = column(users, 0);
  await check(
    "clicking a header sorts up, down, then back",
    JSON.stringify([ascending, descending, original]) === JSON.stringify([["1", "2", "3"], ["3", "2", "1"], ["3", "1", "2"]]),
    JSON.stringify([ascending, descending, original]),
  );
  header(users, 2).click();
  const bornUp = column(users, 2);
  header(users, 2).click();
  const bornDown = column(users, 2);
  header(users, 2).click();
  await check(
    "missing values sort last both ways",
    JSON.stringify([bornUp, bornDown]) === JSON.stringify([["1815", "1906", "null"], ["1906", "1815", "null"]]),
    JSON.stringify([bornUp, bornDown]),
  );

  const ties = renderRich(
    {
      v: 1,
      kind: "table",
      columns: [{ name: "name" }, { name: "score" }],
      rows: [["a", 1], ["b", 1], ["c", 0], ["d", 1]],
      truncated: true,
    },
    { scrollRoot: pane },
  );
  pane.querySelector(".output-body")!.append(ties);
  header(ties, 1).click();
  const tiesUp = column(ties, 0).join("");
  header(ties, 1).click();
  const tiesDown = column(ties, 0).join("");
  await check("sorting is stable", tiesUp === "cabd" && tiesDown === "abdc", `${tiesUp} ${tiesDown}`);
  await check(
    "a truncated table says so",
    (ties.querySelector(".dump-table-foot")?.textContent ?? "").includes("truncated"),
  );

  const nested = users.querySelector<HTMLElement>(".dump-toggle")!;
  const closedBefore = nested.getAttribute("aria-expanded") === "false" && !users.querySelector(".dump-cell-content");
  nested.click();
  const cellContent = nested.parentElement?.querySelector(".dump-cell-content");
  await check(
    "a nested dump in a cell starts collapsed and expands",
    closedBefore && !!cellContent?.querySelector(".dump") && nested.getAttribute("aria-expanded") === "true",
    nested.textContent ?? "",
  );
  const clip = users.querySelector<HTMLElement>(".dump-clip")!;
  const clipped = clip.scrollWidth > clip.clientWidth && clip.title.startsWith("Rear admiral");
  clip.click();
  await check(
    "long strings are clipped with the full text on hover and expand",
    clipped && !!clip.closest("td")?.classList.contains("expanded") && clip.scrollWidth <= clip.clientWidth + 1,
    `${clip.scrollWidth} vs ${clip.clientWidth}`,
  );

  const copied: string[] = [];
  const realWrite = clipboard.write;
  clipboard.write = async (t: string) => {
    copied.push(t);
  };
  try {
    const buttons = [...users.querySelectorAll<HTMLButtonElement>(":scope > .dump-head .dump-action")];
    await check(
      "every node has copy as JSON, tables also CSV",
      JSON.stringify(buttons.map((b) => b.textContent)) === '["JSON","CSV"]' &&
        pane.querySelectorAll(".dump").length === pane.querySelectorAll(".dump .dump-action").length -
          pane.querySelectorAll(".dump-kind-table").length,
    );
    buttons[1]!.click();
    buttons[0]!.click();
    await until(() => copied.length === 2);
    const csv = copied[0] ?? "";
    await check(
      "copy as CSV writes the table in view order",
      csv.startsWith("id,name,born,tags,bio\n3,grace,1906,") &&
        csv.includes('""debugging""') &&
        csv.includes('\n2,alan,,"[""logic"",""codes""]",\n'),
      JSON.stringify(csv.slice(0, 160)),
    );
    const json = JSON.parse(copied[1] ?? "null") as { title?: string; rows?: unknown[] };
    await check("copy as JSON writes the dump", json.title === "users" && json.rows?.length === 3);
  } finally {
    clipboard.write = realWrite;
  }

  // Trees: two levels open, lengths and key counts, lazy children.
  const config = titled("tree", "config")!;
  const rowOf = (tree: Element, key: string) =>
    [...tree.querySelectorAll<HTMLElement>(".tree-row")].find((r) => r.querySelector(".tree-key")?.textContent === key);
  const version = rowOf(config, "version");
  const nestedRow = rowOf(config, "nested");
  const deep = rowOf(config, "deep");
  await check(
    "tree opens the first two levels",
    version?.getAttribute("aria-expanded") === "true" &&
      nestedRow?.getAttribute("aria-expanded") === "true" &&
      deep?.getAttribute("aria-expanded") === "false",
  );
  await check(
    "arrays show their length and closed objects their key count",
    version?.querySelector(".tree-summary")?.textContent === "[3]" &&
      deep?.querySelector(".tree-summary")?.textContent === "{1 key}",
    `${version?.querySelector(".tree-summary")?.textContent} ${deep?.querySelector(".tree-summary")?.textContent}`,
  );
  const keys = [...config.querySelectorAll(".dump-tree > .tree-node > .tree-children > .tree-node > .tree-row > .tree-key")]
    .slice(0, 3)
    .map((k) => k.textContent);
  await check("tree keeps the program's key order", keys.join(",") === "name,version,stable", keys.join(","));
  await check(
    "closed nodes have no children yet",
    deep?.parentElement?.querySelector(".tree-children")?.childElementCount === 0,
  );
  await check(
    "primitives are coloured by type",
    !!rowOf(config, "stable")?.querySelector(".j-bool") &&
      !!rowOf(config, "license")?.querySelector(".j-null") &&
      !!rowOf(config, "name")?.querySelector(".j-str"),
  );

  // Keyboard: Enter toggles, arrows move.
  const press = (target: HTMLElement, key: string) =>
    target.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
  deep!.focus();
  press(deep!, "Enter");
  const openedByEnter = deep!.getAttribute("aria-expanded") === "true";
  press(deep!, "ArrowDown");
  const movedTo = document.activeElement as HTMLElement;
  press(movedTo, "ArrowLeft");
  const backToParent = document.activeElement === deep;
  press(deep!, "ArrowLeft");
  await check(
    "keyboard: Enter toggles, arrows move, Left goes to the parent",
    openedByEnter &&
      movedTo.querySelector(".tree-key")?.textContent === "deeper" &&
      backToParent &&
      deep!.getAttribute("aria-expanded") === "false",
    `${openedByEnter} ${movedTo.textContent} ${backToParent}`,
  );
  pane.focus();
  press(pane, "ArrowDown");
  await check(
    "keyboard: Down from the pane focuses the first node",
    (document.activeElement as HTMLElement | null)?.matches("[data-nav]") === true,
  );
  press(header(users, 0), "Enter");
  await check("keyboard: Enter on a header sorts", header(users, 0).getAttribute("aria-sort") === "ascending");

  // Sanitised HTML.
  const hostile = renderRich(
    {
      v: 1,
      kind: "html",
      html: [
        '<img src="x" onerror="window.__pwned=1">',
        "<script>window.__pwned=2</script>",
        '<a href="https://example.com/">link</a>',
        '<img id="remote" src="https://example.com/x.png">',
        '<img id="inline" src="data:image/png;base64,iVBORw0KGgo=">',
        '<p id="bg" style="color: red; background: url(https://example.com/a.png)">bg</p>',
        '<div id="cover" style="position: fixed; inset: 0; background: red">cover</div>',
        '<form><input></form><iframe src="https://example.com"></iframe><style>body{display:none}</style>',
      ].join(""),
    },
    { scrollRoot: pane },
  );
  pane.querySelector(".output-body")!.append(hostile);
  await frames(2);
  const root = hostile.querySelector<HTMLElement>(".dump-html")!.shadowRoot!;
  const host = hostile.querySelector<HTMLElement>(".dump-html")!.getBoundingClientRect();
  const cover = root.querySelector("#cover")?.getBoundingClientRect();
  await check(
    "html: scripts, handlers, remote loads, links, and forms are removed",
    !root.querySelector("script, iframe, form, input, style:not(:first-child)") &&
      root.querySelectorAll("[onerror]").length === 0 &&
      !root.querySelector("a")?.hasAttribute("href") &&
      !root.querySelector("#remote")?.hasAttribute("src") &&
      root.querySelector("#inline")?.getAttribute("src")?.startsWith("data:image/png") === true &&
      !root.querySelector("#bg")?.hasAttribute("style") &&
      (window as { __pwned?: number }).__pwned === undefined,
    root.innerHTML.slice(0, 300),
  );
  await check(
    "html: position fixed stays inside the box",
    !!cover && cover.width <= host.width + 1 && cover.top >= host.top - 1,
    JSON.stringify({ cover, host }),
  );

  // Performance.
  output.clear();
  const makeRows = (n: number) =>
    Array.from({ length: n }, (_, i) => [i, `user ${i}`, i % 7 === 0 ? null : 1900 + (i % 100), i % 2 === 0]);
  const columns = [{ name: "id", type: "int" }, { name: "name" }, { name: "year" }, { name: "active" }];
  const bodyEl = pane.querySelector(".output-body")!;
  let start = performance.now();
  const thousand = renderRich({ v: 1, kind: "table", columns, rows: makeRows(1000), truncated: false }, { scrollRoot: pane });
  bodyEl.append(thousand);
  thousand.getBoundingClientRect();
  const thousandMs = performance.now() - start;
  await report(`INFO 1,000-row table rendered in ${thousandMs.toFixed(1)} ms`);
  await check(
    "a 1,000-row table renders every row at once",
    thousand.querySelectorAll("tbody tr").length === 1000 && thousandMs < 1000,
    `${thousandMs.toFixed(1)} ms`,
  );

  output.clear();
  start = performance.now();
  const big = renderRich(
    { v: 1, title: "big", kind: "table", columns, rows: makeRows(10_000), truncated: false },
    { scrollRoot: pane },
  );
  pane.querySelector(".output-body")!.append(big);
  big.getBoundingClientRect();
  const bigMs = performance.now() - start;
  await report(`INFO 10,000-row table rendered in ${bigMs.toFixed(1)} ms`);
  const realRows = () => big.querySelectorAll("tbody tr:not(.dump-placeholder)").length;
  await frames(3);
  const initialRows = realRows();
  await check(
    "a 10,000-row table is virtualised",
    big.querySelectorAll("tbody.dump-chunk").length === 10_000 / CHUNK_ROWS && initialRows < 2000 && initialRows >= CHUNK_ROWS,
    `${initialRows} rows in the DOM`,
  );
  pane.scrollTop = pane.scrollHeight;
  const lastShown = await until(() => column(big, 0).includes("9999"));
  const firstReleased = await until(() => !column(big, 0).includes("0"));
  await check(
    "scrolling fills rows in and releases far ones",
    lastShown && firstReleased && realRows() < 2000,
    `${realRows()} rows in the DOM`,
  );
  header(big, 0).click();
  header(big, 0).click();
  pane.scrollTop = 0;
  await check("sorting a virtualised table works", await until(() => column(big, 0)[0] === "9999"), column(big, 0)[0]);

  output.clear();
  const wide = Array.from({ length: 50_000 }, (_, i) => ({ id: i, name: `node ${i}` }));
  start = performance.now();
  const tree = renderRich({ v: 1, kind: "tree", value: wide }, { scrollRoot: pane });
  pane.querySelector(".output-body")!.append(tree);
  tree.getBoundingClientRect();
  const treeMs = performance.now() - start;
  await report(`INFO 50,000-node tree rendered in ${treeMs.toFixed(1)} ms`);
  const rowsBefore = tree.querySelectorAll(".tree-row").length;
  const more = tree.querySelector<HTMLElement>(".tree-more");
  await check(
    "a 50,000-node tree renders one page of children",
    // Root, a page of objects (open: second level), and their two keys each.
    rowsBefore === 1 + TREE_PAGE * 3 && (more?.textContent ?? "").includes("49,800 more") && treeMs < 1000,
    `${rowsBefore} rows, ${more?.textContent}, ${treeMs.toFixed(1)} ms`,
  );
  more?.click();
  await check(
    "show more adds the next page",
    tree.querySelectorAll(".tree-row").length === 1 + 2 * TREE_PAGE * 3,
    `${tree.querySelectorAll(".tree-row").length}`,
  );
  output.clear();

  // Leave the examples on screen for a screenshot.
  await renderExamples(output);
  await output.flushed();
}

/** Create a playground of the node-check test plugin and open it. */
async function openTestPlayground(
  { editor, status }: SelftestContext,
  name: string,
  kind = "script",
): Promise<Playground> {
  const p = await invoke<Playground>("create_playground", { name, language: "node-check", kind });
  await openPlayground(editor, status, p.id);
  return p;
}

const SCENARIOS: Record<string, (ctx: SelftestContext, check: Check) => Promise<void>> = {
  layout: layoutScenario,
  run: runScenario,
  dump: dumpScenario,
  "dump-render": dumpRenderScenario,
  plugins: pluginsScenario,
  toolchains: toolchainsScenario,
  library: libraryScenario,
  "library-trash": libraryTrashScenario,
  locked: lockedScenario,
};

async function pluginsScenario(_ctx: SelftestContext, check: Check) {
  const status = await invoke<PluginsStatus>("list_plugins");

  await check(
    "bundled plugins folder found",
    status.bundledDir !== null && /plugins[\\/]?$/.test(status.bundledDir),
    status.bundledDir ?? "null",
  );
  await check("user plugins folder set", status.userDir.length > 0, status.userDir);
  await report(`INFO plugins loaded=${status.plugins.length} failures=${status.failures.length}`);
  await report(`INFO plugin load time ${status.loadMs.toFixed(2)} ms`);
  for (const p of status.plugins) {
    await report(`INFO plugin ${p.id} (${p.source}) ${p.dir}`);
  }

  // Every failure produces a startup toast naming its folder. Toasts are
  // delivered after the first ping, so allow a moment for them to render.
  for (const f of status.failures) {
    const folder = f.dir.split(/[\\/]/).filter(Boolean).pop() ?? f.dir;
    let shown = false;
    for (let i = 0; i < 20 && !shown; i++) {
      shown = [...document.querySelectorAll(".toast")].some((t) =>
        (t.textContent ?? "").includes(`"${folder}"`),
      );
      if (!shown) await sleep(100);
    }
    await check(`toast shown for failed plugin "${folder}"`, shown);
  }
}

async function toolchainsScenario(_ctx: SelftestContext, check: Check) {
  let events = 0;
  const unsubscribe = subscribe(({ event }) => {
    if (event.type === "toolchain" && event.state.kind !== "checking") events++;
  });

  // Probes run in the background after startup; wait for all to finish.
  const settled = async (): Promise<LanguageToolchain[]> => {
    for (let i = 0; i < 100; i++) {
      const states = await invoke<LanguageToolchain[]>("toolchain_status");
      if (states.every((s) => s.state.kind !== "checking")) return states;
      await sleep(100);
    }
    return invoke<LanguageToolchain[]>("toolchain_status");
  };

  const states = await settled();
  await check(
    "every toolchain check finishes",
    states.every((s) => s.state.kind !== "checking"),
    JSON.stringify(states.map((s) => [s.language, s.state.kind])),
  );
  for (const { language, state } of states) {
    const detail =
      state.kind === "ready"
        ? `${state.program} ${state.version ?? "?"} at ${state.path}`
        : state.kind === "missing"
          ? state.reason
          : "";
    await report(`INFO toolchain ${language} ${state.kind} ${detail}`);
  }

  // The test user plugin "mini" probes a program that doesn't exist.
  const mini = states.find((s) => s.language === "mini");
  if (mini) {
    await check(
      "missing toolchain has a reason",
      mini.state.kind === "missing" && mini.state.reason.includes("not found"),
      JSON.stringify(mini.state),
    );
  }

  // The test user plugin "node-check" probes the real `node`, found through
  // the discovered PATH.
  const node = states.find((s) => s.language === "node-check");
  if (node) {
    await check(
      "real toolchain probes as ready with a version",
      node.state.kind === "ready" && node.state.version !== null,
      JSON.stringify(node.state),
    );
  }

  // Recheck runs every probe again and reports each result as an event.
  events = 0;
  await invoke("recheck_toolchains");
  const after = await settled();
  for (let i = 0; i < 50 && events < after.length; i++) await sleep(100);
  await check(
    "recheck reports every toolchain again",
    events >= after.length && after.length === states.length,
    `${events} result events for ${after.length} plugins`,
  );
  unsubscribe();
}

/** The folder name at the end of a path, on any OS. */
function folderName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

/**
 * Create, list, write, read, and rename playgrounds through the app. Needs a
 * loaded plugin with id "node-check" (the CI test plugin) and should run with
 * TINKERPAD_LIBRARY pointing at a throwaway folder.
 */
async function libraryScenario(_ctx: SelftestContext, check: Check) {
  const plugins = await invoke<PluginsStatus>("list_plugins");
  if (!plugins.plugins.some((p) => p.id === "node-check")) {
    await check("library scenario needs the node-check test plugin", false);
    return;
  }

  let libraryEvents = 0;
  let sourceEvents = 0;
  const unsubscribe = subscribe(({ event }) => {
    if (event.type === "libraryChanged") libraryEvents++;
    if (event.type === "sourceChangedOnDisk") sourceEvents++;
  });

  const before = await invoke<Listing>("list_playgrounds");
  await report(`INFO library root ${before.root} (${before.playgrounds.length} playgrounds)`);

  const a = await invoke<Playground>("create_playground", {
    name: "Self Test",
    language: "node-check",
  });
  const b = await invoke<Playground>("create_playground", {
    name: "self test",
    language: "node-check",
    kind: "script",
  });
  await check(
    "create makes slugged, numbered folders",
    folderName(a.dir).startsWith("self-test") &&
      folderName(b.dir) !== folderName(a.dir) &&
      a.name === "Self Test" &&
      a.kind === "script",
    `${folderName(a.dir)}, ${folderName(b.dir)}`,
  );

  let failed = "";
  await invoke("create_playground", { name: "Nope", language: "no-such-language" }).catch(
    (e: { code?: string }) => (failed = e.code ?? "?"),
  );
  await check("unknown language is rejected", failed === "invalidInput", failed);

  await invoke("write_source", { id: a.id, text: "console.log('from the self-test');\n" });
  const text = await invoke<string>("read_source", { id: a.id });
  await check("write then read round-trips", text === "console.log('from the self-test');\n");

  const listing = await invoke<Listing>("list_playgrounds");
  const listed = listing.playgrounds.find((p) => p.id === a.id);
  await check(
    "list shows the playground with a preview, newest first",
    listed?.preview === "console.log('from the self-test');" &&
      listing.playgrounds[0]?.id === a.id,
    JSON.stringify(listed?.preview),
  );

  const renamed = await invoke<Playground>("rename_playground", { id: a.id, name: "Renamed Test" });
  await check(
    "rename moves the folder and keeps the id",
    renamed.id === a.id && folderName(renamed.dir).startsWith("renamed-test"),
    folderName(renamed.dir),
  );
  await check(
    "source survives rename",
    (await invoke<string>("read_source", { id: a.id })) === text,
  );

  // Our own writes and renames must not look like outside changes.
  await sleep(1500);
  await check("no source-changed event for our own writes", sourceEvents === 0, `${sourceEvents}`);
  await report(`INFO libraryChanged events during scenario: ${libraryEvents}`);
  unsubscribe();
}

/** Moves a playground to the real recycle bin; CI only. */
async function libraryTrashScenario(_ctx: SelftestContext, check: Check) {
  const p = await invoke<Playground>("create_playground", {
    name: "Trash Me",
    language: "node-check",
  });
  await invoke("delete_playground", { id: p.id });
  const listing = await invoke<Listing>("list_playgrounds");
  await check("delete removes the playground", !listing.playgrounds.some((x) => x.id === p.id));
}

/**
 * Templates, scaffolding, and locked lines, through the real editor. Needs the
 * node-check test plugin (its "expression" kind has a prelude and postlude)
 * and a throwaway TINKERPAD_LIBRARY.
 */
async function lockedScenario({ editor, status }: SelftestContext, check: Check) {
  const view = editor.view;
  const text = () => view.state.doc.toString();
  const note = () => document.querySelector(".status-note")?.textContent ?? "";
  const noteShown = () => !(document.querySelector<HTMLElement>(".status-note")?.hidden ?? true);
  const prelude = "console.log(\n";
  const postlude = "\n);\n";

  // A kind without locked lines: the template gets {{name}} and {{date}}.
  const script = await invoke<Playground>("create_playground", {
    name: "Locked Script",
    language: "node-check",
    kind: "script",
  });
  const openedScript = await openPlayground(editor, status, script.id);
  await check(
    "template renders name and date",
    /^\/\/ Locked Script, created \d{4}-\d{2}-\d{2}$/m.test(openedScript.text) &&
      openedScript.lock === "none",
    JSON.stringify(openedScript.text.split("\n")[0]),
  );
  await frame();
  await check("no locked lines for a plain kind", document.querySelectorAll(".cm-locked-line").length === 0);

  // A kind with a prelude and postlude.
  const expr = await invoke<Playground>("create_playground", {
    name: "Locked Expression",
    language: "node-check",
    kind: "expression",
  });
  const opened = await openPlayground(editor, status, expr.id);
  await frame();
  await check(
    "file is prelude + template + postlude",
    text() === "console.log(\n1 + 1\n);\n" && opened.lock === "locked",
    JSON.stringify(text()),
  );
  await check(
    "locked lines are dimmed and marked",
    document.querySelectorAll(".cm-locked-line").length === 3 &&
      document.querySelectorAll(".cm-lock-marker").length === 3,
    `${document.querySelectorAll(".cm-locked-line").length} lines, ${document.querySelectorAll(".cm-lock-marker").length} markers`,
  );

  // The cursor can sit in a locked line.
  view.dispatch({ selection: EditorSelection.cursor(3) });
  await check("cursor can enter a locked line", view.state.selection.main.head === 3);

  // Typing there is refused with a hint.
  view.dispatch({ changes: { from: 3, insert: "X" }, userEvent: "input.type" });
  await sleep(20);
  await check("typing in a locked line is refused", text() === "console.log(\n1 + 1\n);\n");
  await check("refusal shows a hint", noteShown() && note() === LOCKED_HINT, note());

  // Typing in the middle works.
  view.dispatch({ changes: { from: prelude.length + 5, insert: " + 40" }, userEvent: "input.type" });
  await check("typing in the editable middle works", text() === "console.log(\n1 + 1 + 40\n);\n", JSON.stringify(text()));

  // Mod+A selects only the middle, so typing replaces just that.
  runScopeHandlers(view, key("a", { mod: true }), "editor");
  const sel = view.state.selection.main;
  await check(
    "Mod+A selects only the editable middle",
    sel.from === prelude.length && sel.to === text().length - postlude.length,
    `${sel.from}-${sel.to}`,
  );
  view.dispatch(view.state.replaceSelection("40 + 2"), { userEvent: "input.type" });
  await check("typing after Mod+A replaces only the middle", text() === "console.log(\n40 + 2\n);\n", JSON.stringify(text()));

  // A real select-all (the whole document) and delete keeps the locked lines.
  view.dispatch({
    selection: EditorSelection.single(0, text().length),
  });
  view.dispatch({ changes: { from: 0, to: text().length }, userEvent: "delete.selection" });
  await check("select-all + delete leaves the locked lines", text() === prelude + postlude, JSON.stringify(text()));

  // Undo brings the middle back.
  undo(view);
  await check("undo restores the middle", text() === "console.log(\n40 + 2\n);\n", JSON.stringify(text()));

  // Saving writes exactly the editor's text; nothing is added or stripped.
  await invoke("write_source", { id: expr.id, text: text() });
  const onDisk = await invoke<string>("read_source", { id: expr.id });
  await check("file on disk is byte-identical to the editor", onDisk === text(), JSON.stringify(onDisk));

  // Edited outside the wrapper: nothing is locked, and the status bar says why.
  await invoke("write_source", { id: expr.id, text: "40 + 2\n" });
  const mismatch = await openPlayground(editor, status, expr.id);
  await frame();
  await check(
    "a file without its wrapper opens unlocked",
    mismatch.lock === "mismatch" && document.querySelectorAll(".cm-locked-line").length === 0,
    mismatch.lock,
  );
  await check("the status bar explains why", noteShown() && note() === MISMATCH_NOTE, note());
  view.dispatch({ changes: { from: 0, insert: "// free to edit\n" }, userEvent: "input.type" });
  await check("an unlocked file is fully editable", text().startsWith("// free to edit\n"));
}
