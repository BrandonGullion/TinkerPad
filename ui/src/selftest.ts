// Opt-in in-app self-test, used to verify UI behaviour from scripts and CI
// without sending keystrokes to the desktop. Enabled only when the app is
// started with TINKERPAD_SELFTEST set; results go to the app's stdout as
// "selftest: PASS ..." / "selftest: FAIL ..." lines, ending with
// "selftest: done".

import { undo } from "@codemirror/commands";
import { EditorSelection } from "@codemirror/state";
import { closeCompletion, currentCompletions, startCompletion } from "@codemirror/autocomplete";
import { forEachDiagnostic } from "@codemirror/lint";
import { type EditorView, runScopeHandlers } from "@codemirror/view";

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
import type { LanguageToolchain, Listing, Playground, PluginsStatus, RunEvent, Session } from "./protocol";
import type { RunController } from "./run";
import type { TabManager } from "./tabs";

export interface SelftestContext {
  editor: Editor;
  /** The active tab's output pane. */
  readonly output: OutputPane;
  runs: RunController;
  status: StatusBar;
  tabs: TabManager;
  /** The app's global shortcut handler, for pressing keys without the OS. */
  handleGlobalKey: (e: KeyboardEvent) => boolean;
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
  const { editor, runs } = ctx;
  await openTestPlayground(ctx, "Layout Test");
  const view = editor.view;
  const outText = () => ctx.output.element.querySelector(".output-body")?.textContent ?? "";

  // Typing into the editor.
  view.dispatch({ changes: { from: view.state.doc.length, insert: "const x = [1, 2, 3];\n" } });
  await check("editor accepts input", editor.text.includes("const x = [1, 2, 3];"));

  // Mod+Enter through CodeMirror's real keymap starts a run.
  const end = runs.waitForEnd();
  const handled = runScopeHandlers(view, key("Enter", { mod: true }), "editor");
  await check("Mod+Enter handled by keymap", handled);
  await end;
  await ctx.output.flushed();

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
    ctx.output.append(chunk % 7 === 0 ? "stderr" : "stdout", text);
  }
  await ctx.output.flushed();
  const ms = performance.now() - start;
  // Timing is informational: CI renders in software and would be flaky.
  await report(`INFO 10k lines in 1k chunks rendered in ${ms.toFixed(1)} ms`);
  await check(
    "stderr styled separately",
    ctx.output.element.querySelector(".out-stderr") !== null &&
      ctx.output.element.querySelector(".out-stdout") !== null,
  );

  // Auto-scroll sticks to the bottom while at the bottom.
  const el = ctx.output.element;
  const atBottom = () => el.scrollHeight - el.scrollTop - el.clientHeight <= 4;
  await check("auto-scrolls while at bottom", atBottom());

  // Scrolling up stops auto-scroll and shows the jump button.
  el.scrollTop = 0;
  el.dispatchEvent(new Event("scroll"));
  ctx.output.append("stdout", "more\n");
  await ctx.output.flushed();
  const jump = el.querySelector<HTMLButtonElement>(".output-jump")!;
  await check("scrolled up stays put", el.scrollTop === 0 && !jump.hidden);

  // Jump to bottom resumes.
  jump.click();
  await frame();
  await check("jump to bottom resumes", atBottom() && jump.hidden === true);

  // Line cap: push past 20,000 lines and expect a trimmed notice.
  let big = "";
  for (let i = 0; i < 15_000; i++) big += "x\n";
  ctx.output.append("stdout", big);
  await ctx.output.flushed();
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
  const { runs, editor } = ctx;
  const main = await openTestPlayground(ctx, "Run Test");
  const outText = () => ctx.output.element.querySelector(".output-body")?.textContent ?? "";
  const status = () => document.querySelector(".status-run")?.textContent ?? "";

  // console.log(1) end to end.
  let end = runs.waitForEnd();
  runCode(ctx, "console.log(1)\n");
  let outcome = await withTimeout(end, 15_000, "console.log(1)");
  await ctx.output.flushed();
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
  await ctx.output.flushed();
  await check(
    "stderr shown and non-zero exit reported",
    outcome.kind === "exited" &&
      outcome.code === 2 &&
      (ctx.output.element.querySelector(".out-stderr")?.textContent ?? "").includes("oops") &&
      status().startsWith("Exit 2"),
    JSON.stringify({ outcome, status: status() }),
  );

  // UTF-8 survives the process boundary.
  end = runs.waitForEnd();
  runCode(ctx, 'console.log("héllo wörld → 日本 🎉")\n');
  await withTimeout(end, 15_000, "utf8");
  await ctx.output.flushed();
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
  await ctx.output.flushed();
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
  const { runs } = ctx;
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
  await ctx.output.flushed();
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
  const body = ctx.output.element.querySelector(".output-body")!;
  const blocks = [...body.children].map((c) =>
    c.classList.contains("dump") ? `dump:${(c as HTMLElement).dataset.kind}` : (c as HTMLElement).dataset.stream,
  );
  const tableFoot = body.querySelector(".dump-kind-table .dump-table-foot")?.textContent ?? "";
  await check(
    "output pane renders dumps as blocks between the text",
    JSON.stringify(blocks) ===
      JSON.stringify(["stdout", "dump:value", "stdout", "stderr", "info", "dump:table", "dump:text", "stdout"]) &&
      tableFoot.startsWith(`${(20_000).toLocaleString()} rows`),
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
    rowsBefore === 1 + TREE_PAGE * 3 && (more?.textContent ?? "").includes(`${(49_800).toLocaleString()} more`) && treeMs < 1000,
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

/** The bundled TypeScript plugin, end to end in the app (01-09). */
async function typescriptScenario(ctx: SelftestContext, check: Check) {
  const { runs, editor, status } = ctx;
  const outText = () => ctx.output.element.querySelector(".output-body")?.textContent ?? "";

  const plugins = await invoke<PluginsStatus>("list_plugins");
  const ts = plugins.plugins.find((p) => p.id === "typescript");
  await check("the TypeScript plugin is bundled", ts?.source === "bundled", JSON.stringify(ts ?? null));

  let state: LanguageToolchain["state"] | undefined;
  for (let i = 0; i < 100; i++) {
    state = (await invoke<LanguageToolchain[]>("toolchain_status")).find((s) => s.language === "typescript")?.state;
    if (state && state.kind !== "checking") break;
    await sleep(100);
  }
  await check("Node is ready for TypeScript", state?.kind === "ready", JSON.stringify(state ?? null));
  if (state?.kind !== "ready") return;

  const created = await invoke<Playground>("create_playground", { name: "TS Self Test", language: "typescript" });
  await openPlayground(editor, status, created.id);
  await check(
    "a new playground starts from the statements template",
    created.kind === "statements" && editor.text.startsWith('import { dump } from "tinkerpad";\n'),
    JSON.stringify(editor.text.slice(0, 60)),
  );

  // Mod+Enter on the untouched template, until its table is on screen.
  const tableVisible = async (label: string) => {
    const end = runs.waitForEnd();
    const start = performance.now();
    runScopeHandlers(editor.view, key("Enter", { mod: true }), "editor");
    let ms: number | null = null;
    while (performance.now() - start < 15_000) {
      const table = ctx.output.element.querySelector<HTMLElement>(".dump-kind-table");
      if (table && table.getClientRects().length > 0) {
        ms = performance.now() - start;
        break;
      }
      await new Promise((r) => requestAnimationFrame(r));
    }
    const outcome = await withTimeout(end, 15_000, label);
    await report(`INFO ${label}: Mod+Enter to table visible ${ms === null ? "never" : `${ms.toFixed(0)} ms`}`);
    return { ms, outcome };
  };
  const first = await tableVisible("TypeScript first run");
  await ctx.output.flushed();
  const table = ctx.output.element.querySelector(".dump-kind-table");
  await check(
    "the template's dump renders a table",
    first.ms !== null &&
      first.outcome.kind === "exited" &&
      first.outcome.code === 0 &&
      table?.querySelector(".dump-title")?.textContent === "people" &&
      table.querySelectorAll("tbody tr").length === 2 &&
      !ctx.output.element.querySelector(".out-stderr"),
    JSON.stringify({ outcome: first.outcome, text: outText().slice(0, 200) }),
  );
  await tableVisible("TypeScript warm run");

  const end = runs.waitForEnd();
  runCode(ctx, 'const who: string = "world";\nconsole.log(`Hello, ${who}!`);\n');
  const hello = await withTimeout(end, 15_000, "hello world");
  await ctx.output.flushed();
  await check(
    "hello world prints",
    hello.kind === "exited" && hello.code === 0 && outText() === "Hello, world!\n",
    JSON.stringify({ hello, out: outText() }),
  );

  const program = await invoke<Playground>("create_playground", {
    name: "TS Program Test",
    language: "typescript",
    kind: "program",
  });
  await openPlayground(editor, status, program.id);
  const programEnd = runs.waitForEnd();
  runScopeHandlers(editor.view, key("Enter", { mod: true }), "editor");
  const programOutcome = await withTimeout(programEnd, 15_000, "program template");
  await ctx.output.flushed();
  await check(
    "the program template prints and dumps",
    programOutcome.kind === "exited" &&
      outText().startsWith("Hello from TS Program Test\n") &&
      ctx.output.element.querySelector(".dump-kind-tree") !== null,
    JSON.stringify({ programOutcome, out: outText().slice(0, 120) }),
  );

  // Leave a table and some text on screen for a screenshot.
  await openPlayground(editor, status, created.id);
  const last = runs.waitForEnd();
  runCode(
    ctx,
    [
      'import { dump } from "tinkerpad";',
      "",
      'console.log("Hello, world!");',
      "const people = dump(",
      "  [",
      '    { name: "Ada Lovelace", born: 1815, languages: ["Analytical Engine"] },',
      '    { name: "Grace Hopper", born: 1906, languages: ["COBOL", "FLOW-MATIC"] },',
      '    { name: "Alan Turing", born: 1912 },',
      "  ],",
      '  "people",',
      ");",
      'console.log(`${people.length} people`);',
      "",
    ].join("\n"),
  );
  await withTimeout(last, 15_000, "final run");
  await ctx.output.flushed();
}

/** A language's toolchain state once its check has finished (or 10 s passed). */
async function settledToolchain(language: string): Promise<LanguageToolchain["state"] | undefined> {
  let state: LanguageToolchain["state"] | undefined;
  for (let i = 0; i < 100; i++) {
    state = (await invoke<LanguageToolchain[]>("toolchain_status")).find((s) => s.language === language)?.state;
    if (state && state.kind !== "checking") break;
    await sleep(100);
  }
  return state;
}

/**
 * Press Mod+Enter in the editor and time how long until `selector` is visible
 * in the output pane. Resolves once the run has ended too.
 */
async function runUntilVisible(
  { editor, output, runs }: SelftestContext,
  selector: string,
  label: string,
  timeoutMs: number,
): Promise<{ ms: number | null; outcome: Awaited<ReturnType<RunController["waitForEnd"]>> }> {
  const end = runs.waitForEnd();
  const start = performance.now();
  runScopeHandlers(editor.view, key("Enter", { mod: true }), "editor");
  let ms: number | null = null;
  let done = false;
  void end.then(() => (done = true));
  while (performance.now() - start < timeoutMs) {
    const found = output.element.querySelector<HTMLElement>(selector);
    if (found && found.getClientRects().length > 0) {
      ms = performance.now() - start;
      break;
    }
    // Stop polling once the run is over and its output has been drawn.
    if (done) {
      await output.flushed();
      if (!output.element.querySelector(selector)) break;
    }
    await new Promise((r) => requestAnimationFrame(r));
  }
  const outcome = await withTimeout(end, timeoutMs, label);
  await output.flushed();
  await report(`INFO ${label}: Mod+Enter to ${selector} visible ${ms === null ? "never" : `${ms.toFixed(0)} ms`}`);
  return { ms, outcome };
}

/** The bundled C# plugin, end to end in the app (01-10). */
async function csharpScenario(ctx: SelftestContext, check: Check) {
  const { editor, status } = ctx;
  const outText = () => (ctx.output.element.querySelector(".output-body")?.textContent ?? "").replaceAll("\r\n", "\n");
  // A cold first build (restore, compiler start) can take a while on CI.
  const LONG = 180_000;

  const plugins = await invoke<PluginsStatus>("list_plugins");
  const cs = plugins.plugins.find((p) => p.id === "csharp");
  await check("the C# plugin is bundled", cs?.source === "bundled", JSON.stringify(cs ?? null));
  const state = await settledToolchain("csharp");
  await check("the .NET 10 SDK is ready for C#", state?.kind === "ready", JSON.stringify(state ?? null));
  if (state?.kind !== "ready") return;

  const created = await invoke<Playground>("create_playground", { name: "CS Self Test", language: "csharp" });
  await openPlayground(editor, status, created.id);
  await check(
    "a new playground starts from the statements template",
    created.kind === "statements" && editor.text.includes("using TinkerPad;\n") && editor.text.includes(".Dump(\"people\")"),
    JSON.stringify(editor.text.slice(0, 120)),
  );

  const table = () => ctx.output.element.querySelector(".dump-kind-table");
  const first = await runUntilVisible(ctx, ".dump-kind-table", "C# first run of a new playground", LONG);
  await check(
    "the template's .Dump() renders a table",
    first.ms !== null &&
      first.outcome.kind === "exited" &&
      first.outcome.code === 0 &&
      table()?.querySelector(".dump-title")?.textContent === "people" &&
      table()?.querySelectorAll("tbody tr").length === 2 &&
      !ctx.output.element.querySelector(".out-stderr"),
    JSON.stringify({ outcome: first.outcome, text: outText().slice(0, 300) }),
  );
  const second = await runUntilVisible(ctx, ".dump-kind-table", "C# second run, unchanged", LONG);
  await check("an unchanged rerun shows the table again", second.ms !== null && second.outcome.kind === "exited");

  editor.view.dispatch({
    changes: {
      from: 0,
      to: editor.view.state.doc.length,
      insert: 'using TinkerPad;\n\nConsole.WriteLine("héllo 日本");\n(6 * 7).Dump("answer");\n',
    },
  });
  const edited = await runUntilVisible(ctx, ".dump-kind-value", "C# run after an edit", LONG);
  await check(
    "UTF-8 text and a value dump after an edit",
    edited.outcome.kind === "exited" &&
      outText().startsWith("héllo 日本\n") &&
      ctx.output.element.querySelector(".dump-kind-value .j-num")?.textContent === "42",
    JSON.stringify(outText().slice(0, 120)),
  );

  editor.view.dispatch({
    changes: { from: 0, to: editor.view.state.doc.length, insert: 'using TinkerPad;\n\nConsole.WriteLine("x")\n' },
  });
  const broken = await runUntilVisible(ctx, ".dump", "C# compile error", LONG);
  await check(
    "a compile error is shown and the run fails",
    broken.outcome.kind === "exited" &&
      broken.outcome.code !== 0 &&
      /main\.cs\(3,23\): error CS1002/.test(outText()),
    JSON.stringify({ outcome: broken.outcome, text: outText().slice(0, 300) }),
  );

  const expression = await invoke<Playground>("create_playground", {
    name: "CS Expression Test",
    language: "csharp",
    kind: "expression",
  });
  const opened = await openPlayground(editor, status, expression.id);
  await check(
    "the expression kind opens with locked lines",
    opened.lock === "locked" && editor.text.startsWith("TinkerPad.Dump.Show(\n") && editor.text.endsWith("\n);\n"),
    JSON.stringify({ lock: opened.lock, text: editor.text }),
  );
  const expr = await runUntilVisible(ctx, ".dump-kind-table", "C# expression kind", LONG);
  await check(
    "the expression kind dumps its value",
    expr.outcome.kind === "exited" && table()?.querySelectorAll("tbody tr").length === 5,
    JSON.stringify({ outcome: expr.outcome, text: outText().slice(0, 200) }),
  );

  const program = await invoke<Playground>("create_playground", {
    name: "CS Program Test",
    language: "csharp",
    kind: "program",
  });
  await openPlayground(editor, status, program.id);
  const prog = await runUntilVisible(ctx, ".dump-kind-tree", "C# program kind", LONG);
  await check(
    "the program kind prints and dumps",
    prog.outcome.kind === "exited" && outText().startsWith("Hello from C#\n") && prog.ms !== null,
    JSON.stringify({ outcome: prog.outcome, text: outText().slice(0, 200) }),
  );

  // Leave a table on screen for a screenshot.
  await openPlayground(editor, status, created.id);
  editor.view.dispatch({
    changes: {
      from: 0,
      to: editor.view.state.doc.length,
      insert: [
        "using TinkerPad;",
        "",
        'Console.WriteLine("Hello from C#");',
        "var people = new[]",
        "{",
        '    new Person("Ada Lovelace", 1815, ["Analytical Engine"]),',
        '    new Person("Grace Hopper", 1906, ["COBOL", "FLOW-MATIC"]),',
        '    new Person("Alan Turing", 1912, []),',
        '}.Dump("people");',
        'Console.WriteLine($"{people.Length} people, born {people.Min(p => p.Born)}–{people.Max(p => p.Born)}");',
        "",
        "record Person(string Name, int Born, string[] Languages);",
        "",
      ].join("\n"),
    },
  });
  await runUntilVisible(ctx, ".dump-kind-table", "C# screenshot run", LONG);
}

/** The bundled Rust plugin, end to end in the app (01-11). */
async function rustScenario(ctx: SelftestContext, check: Check) {
  const { editor, status } = ctx;
  const outText = () => (ctx.output.element.querySelector(".output-body")?.textContent ?? "").replaceAll("\r\n", "\n");
  // A first build downloads and compiles serde; slow on CI.
  const LONG = 300_000;

  const plugins = await invoke<PluginsStatus>("list_plugins");
  const rs = plugins.plugins.find((p) => p.id === "rust");
  await check("the Rust plugin is bundled", rs?.source === "bundled", JSON.stringify(rs ?? null));
  const state = await settledToolchain("rust");
  await check("Cargo is ready for Rust", state?.kind === "ready", JSON.stringify(state ?? null));
  if (state?.kind !== "ready") return;

  const created = await invoke<Playground>("create_playground", { name: "RS Self Test", language: "rust" });
  const opened = await openPlayground(editor, status, created.id);
  await check(
    "a new playground is a crate with src/main.rs from the program template",
    created.kind === "program" &&
      /[\\/]src[\\/]main\.rs$/.test(created.mainFile) &&
      editor.text.includes("fn main()") &&
      opened.lock === "none",
    JSON.stringify({ mainFile: created.mainFile, lock: opened.lock }),
  );

  const table = () => ctx.output.element.querySelector(".dump-kind-table");
  const first = await runUntilVisible(ctx, ".dump-kind-table", "Rust first run of a new playground", LONG);
  await check(
    "the template's dump renders a table",
    first.ms !== null &&
      first.outcome.kind === "exited" &&
      first.outcome.code === 0 &&
      outText().startsWith("Hello from Rust\n") &&
      table()?.querySelector(".dump-title")?.textContent === "people" &&
      table()?.querySelectorAll("tbody tr").length === 2,
    JSON.stringify({ outcome: first.outcome, text: outText().slice(0, 300) }),
  );
  const second = await runUntilVisible(ctx, ".dump-kind-table", "Rust second run, unchanged", LONG);
  await check("an unchanged rerun shows the table again", second.ms !== null && second.outcome.kind === "exited");

  editor.view.dispatch({
    changes: {
      from: 0,
      to: editor.view.state.doc.length,
      insert: editor.text.replace('println!("Hello from Rust");', 'println!("héllo 日本");'),
    },
  });
  const edited = await runUntilVisible(ctx, ".dump-kind-table", "Rust run after a one-line edit", LONG);
  await check(
    "a one-line edit rebuilds and runs",
    edited.outcome.kind === "exited" && outText().startsWith("héllo 日本\n"),
    JSON.stringify(outText().slice(0, 80)),
  );

  const other = await invoke<Playground>("create_playground", { name: "RS Second Test", language: "rust" });
  await openPlayground(editor, status, other.id);
  const shared = await runUntilVisible(ctx, ".dump-kind-table", "Rust first run of a second playground", LONG);
  await check("a second playground builds and runs", shared.ms !== null && shared.outcome.kind === "exited");

  editor.view.dispatch({
    changes: {
      from: 0,
      to: editor.view.state.doc.length,
      insert: "fn main() {\n    let n: u32 = 1;\n    let s: String = n;\n    println!(\"{s}\");\n}\n",
    },
  });
  const broken = await runUntilVisible(ctx, ".dump", "Rust compile error", LONG);
  await check(
    "a compile error is shown and the run fails",
    broken.outcome.kind === "exited" &&
      broken.outcome.code !== 0 &&
      outText().includes("error[E0308]: mismatched types") &&
      /--> src[\\/]main\.rs:3:21/.test(outText()),
    JSON.stringify({ outcome: broken.outcome, text: outText().slice(0, 400) }),
  );

  const expression = await invoke<Playground>("create_playground", {
    name: "RS Expression Test",
    language: "rust",
    kind: "expression",
  });
  const exprOpened = await openPlayground(editor, status, expression.id);
  await check(
    "the expression kind opens with locked lines",
    exprOpened.lock === "locked" && editor.text.startsWith("fn main() {\n    tinkerpad::dump(&(\n"),
    JSON.stringify({ lock: exprOpened.lock, text: editor.text }),
  );
  const expr = await runUntilVisible(ctx, ".dump-kind-tree", "Rust expression kind", LONG);
  await check(
    "the expression kind dumps its value",
    expr.outcome.kind === "exited" && (ctx.output.element.querySelector(".dump-kind-tree")?.textContent ?? "").includes("[10]"),
    JSON.stringify({ outcome: expr.outcome, text: outText().slice(0, 200) }),
  );

  // Leave a table on screen for a screenshot.
  await openPlayground(editor, status, created.id);
  editor.view.dispatch({
    changes: {
      from: 0,
      to: editor.view.state.doc.length,
      insert: [
        "use serde::Serialize;",
        "use tinkerpad::Dump;",
        "",
        "#[derive(Serialize)]",
        "struct Person {",
        "    name: &'static str,",
        "    born: u16,",
        "    languages: Vec<&'static str>,",
        "}",
        "",
        "fn main() {",
        '    println!("Hello from Rust");',
        "    let people = vec![",
        '        Person { name: "Ada Lovelace", born: 1815, languages: vec!["Analytical Engine"] },',
        '        Person { name: "Grace Hopper", born: 1906, languages: vec!["COBOL", "FLOW-MATIC"] },',
        '        Person { name: "Alan Turing", born: 1912, languages: vec![] },',
        "    ];",
        '    let oldest = people.dump_titled("people").iter().map(|p| p.born).min().unwrap();',
        '    println!("oldest born {oldest}");',
        "}",
        "",
      ].join("\n"),
    },
  });
  await runUntilVisible(ctx, ".dump-kind-table", "Rust screenshot run", LONG);
}

/** Clickable positions from a run's output, and the marks in the editor (01-13). */
async function annotationsScenario(ctx: SelftestContext, check: Check) {
  const { editor, tabs, runs } = ctx;
  const view = editor.view;
  const pane = () => ctx.output.element;
  const squiggles = () => [...document.querySelectorAll(".cm-annotation")].map((e) => e.textContent);
  const links = () => [...pane().querySelectorAll(".out-annotation")].map((e) => e.textContent ?? "");
  const note = () => document.querySelector(".status-note")?.textContent ?? "";

  const state = await settledToolchain("typescript");
  await check("Node is ready for the TypeScript plugin", state?.kind === "ready", JSON.stringify(state ?? null));
  if (state?.kind !== "ready") return;

  const playground = await invoke<Playground>("create_playground", { name: "Annotations Test", language: "typescript" });
  await openPlayground(editor, ctx.status, playground.id);
  const source = [
    "function boom(): never {",
    '  throw new TypeError("nope");',
    "}",
    "",
    'console.log("before");',
    "boom();",
    "",
  ].join("\n");
  let end = runs.waitForEnd();
  runCode(ctx, source);
  await withTimeout(end, 30_000, "throwing program");
  await ctx.output.flushed();
  await frame();

  await check(
    "the line an error was found on becomes clickable",
    links().length === 1 && links()[0]!.includes("at boom") && links()[0]!.includes("main.ts:2:9"),
    JSON.stringify(links()),
  );
  await check(
    "the error is marked in the editor at its line and column",
    squiggles().length === 1 && squiggles()[0] === 'new TypeError("nope");',
    JSON.stringify(squiggles()),
  );
  const marked = document.querySelector<HTMLElement>(".cm-annotation");
  const decoration = marked ? getComputedStyle(marked).textDecoration : "";
  await check(
    "the mark is a squiggle in the error colour, with the message on hover",
    (marked?.title ?? "").includes("TypeError: nope") &&
      decoration.includes("wavy") &&
      decoration.includes("underline"),
    JSON.stringify({ title: marked?.title, decoration }),
  );

  // Clicking the output line goes to the position.
  view.dispatch({ selection: { anchor: 0 } });
  pane().querySelector<HTMLElement>(".out-annotation")!.click();
  await frame();
  const clicked = view.state.doc.lineAt(view.state.selection.main.head);
  await check(
    "clicking the line moves the cursor there",
    clicked.number === 2 && view.state.selection.main.head - clicked.from === 8,
    JSON.stringify({ line: clicked.number, col: view.state.selection.main.head - clicked.from + 1 }),
  );

  // F8 cycles; a second annotation is added the way a second error would be.
  tabs.annotation(playground.id, {
    file: playground.mainFile,
    line: 5,
    col: 1,
    message: "second problem",
    severity: "warning",
    text: "second problem",
  });
  await frame();
  view.dispatch({ selection: { anchor: 0 } });
  pressKey("F8");
  const first = view.state.doc.lineAt(view.state.selection.main.head).number;
  pressKey("F8");
  const second = view.state.doc.lineAt(view.state.selection.main.head).number;
  pressKey("F8", { shift: true });
  const back = view.state.doc.lineAt(view.state.selection.main.head).number;
  await check(
    "F8 and Shift+F8 walk the annotations",
    [first, second, back].join() === "2,5,2" && squiggles().length === 2,
    JSON.stringify({ first, second, back, squiggles: squiggles() }),
  );
  await check(
    "a warning is marked differently from an error",
    document.querySelectorAll(".cm-annotation-warning").length === 1,
    `${document.querySelectorAll(".cm-annotation-warning").length}`,
  );

  // An annotation in another file is clickable but opens nothing.
  tabs.revealAnnotation(playground.id, {
    file: "node:internal/modules/esm/loader",
    line: 661,
    col: 26,
    message: "elsewhere",
    severity: "error",
    text: "at async onImport (node:internal/modules/esm/loader:661:26)",
  });
  await frame();
  await check(
    "an annotation in another file says so instead of jumping",
    note().includes("isn't part of this playground") &&
      view.state.doc.lineAt(view.state.selection.main.head).number === 2,
    note(),
  );

  // The next run clears them.
  end = runs.waitForEnd();
  runCode(ctx, 'console.log("all good");\n');
  await withTimeout(end, 30_000, "clean program");
  await ctx.output.flushed();
  await check(
    "the next run clears the marks and the links",
    squiggles().length === 0 && links().length === 0,
    JSON.stringify({ squiggles: squiggles(), links: links() }),
  );

  // A position inside locked lines is marked on the first editable line.
  const locked = await openTestPlayground(ctx, "Annotations Locked", "expression");
  tabs.annotation(locked.id, {
    file: locked.mainFile,
    line: 1,
    col: 1,
    message: "inside the wrapper",
    severity: "error",
    text: "inside the wrapper",
  });
  await frame();
  const lockedMark = document.querySelector<HTMLElement>(".cm-annotation");
  await check(
    "a position inside locked lines is marked on the first editable line, with a note",
    lockedMark?.textContent === "1 + 1" && (lockedMark.title ?? "").includes("locked lines"),
    JSON.stringify({ text: lockedMark?.textContent, title: lockedMark?.title }),
  );
}

/** A key press through the app's real document listeners. */
function pressKey(k: string, mods: { mod?: boolean; shift?: boolean; ctrl?: boolean } = {}, target?: EventTarget): boolean {
  const e = new KeyboardEvent("keydown", {
    key: k,
    bubbles: true,
    cancelable: true,
    ctrlKey: !!mods.ctrl || (!isMac && !!mods.mod),
    metaKey: isMac && !!mods.mod,
    shiftKey: !!mods.shift,
  });
  (target ?? document.activeElement ?? document.body).dispatchEvent(e);
  return e.defaultPrevented;
}

/** Choose in the open picker: type `filter`, then Enter. */
async function choosePicked(filter: string): Promise<boolean> {
  if (!(await until(() => document.querySelector(".picker-input") !== null, 5000))) return false;
  const input = document.querySelector<HTMLInputElement>(".picker-input")!;
  input.value = filter;
  input.dispatchEvent(new Event("input"));
  const matched = document.querySelectorAll(".picker-item").length > 0;
  pressKey("Enter", {}, input);
  return matched;
}

/** Tabs, the new-playground flow, autosave, changes on disk, and the session (01-12). */
async function tabsScenario(ctx: SelftestContext, check: Check) {
  const { editor, tabs, runs } = ctx;
  const view = editor.view;
  const outText = () => ctx.output.element.querySelector(".output-body")?.textContent ?? "";
  const activeEl = () => document.querySelector<HTMLElement>(".tab.active");
  const tabEl = (id: string) => document.querySelector<HTMLElement>(`.tab[data-id="${id}"]`);
  const empty = document.querySelector<HTMLElement>(".empty-state")!;
  const visible = (el: Element | null | undefined) => !!el && el.getClientRects().length > 0;

  for (const t of [...tabs.tabs]) await tabs.close(t);
  await check(
    "closing the last tab shows the empty-state hint",
    tabs.tabs.length === 0 &&
      visible(empty) &&
      /N for a new playground, .*O to open one/.test(empty.textContent ?? "") &&
      document.querySelector(".tab") === null,
    empty.textContent ?? "",
  );

  // Mod+N: last used language, default kind, "Untitled N", editor focused.
  const first = await openTestPlayground(ctx, "Tabs First");
  const before = tabs.tabs.length;
  const handled = pressKey("n", { mod: true });
  await until(() => tabs.tabs.length === before + 1 && tabs.activeId !== first.id, 10_000);
  const created = tabs.activeTab!;
  const summary = tabs.summary(created.id);
  await frame();
  await check(
    "Mod+N creates an Untitled playground in the last used language, no prompts",
    handled &&
      /^Untitled \d+$/.test(summary?.name ?? "") &&
      summary?.language === "node-check" &&
      summary.kind === "script" &&
      view.contentDOM.contains(document.activeElement) &&
      document.querySelector(".picker") === null,
    JSON.stringify({ handled, name: summary?.name, language: summary?.language, kind: summary?.kind }),
  );
  await check(
    "the tab shows its name and language",
    activeEl()?.querySelector(".tab-name")?.textContent === summary?.name &&
      (activeEl()?.querySelector(".tab-lang")?.textContent ?? "") !== "",
    activeEl()?.textContent ?? "",
  );

  // Dirty dot, then autosave.
  view.dispatch({ changes: { from: view.state.doc.length, insert: "// typed\n" } });
  const dirtyShown = activeEl()?.querySelector<HTMLElement>(".tab-dirty")?.hidden === false;
  const typedAt = performance.now();
  const saved = await until(() => !created.dirty, 5000);
  const savedAfter = performance.now() - typedAt;
  const onDisk = await invoke<string>("read_source", { id: created.id });
  await report(`INFO autosave landed ${savedAfter.toFixed(0)} ms after the edit`);
  await check(
    "typing shows the dirty dot and autosave clears it about 500 ms later",
    dirtyShown &&
      saved &&
      savedAfter >= 450 &&
      activeEl()?.querySelector<HTMLElement>(".tab-dirty")?.hidden === true &&
      onDisk === editor.text,
    JSON.stringify({ dirtyShown, saved, savedAfter, same: onDisk === editor.text }),
  );

  // Each tab keeps its text, cursor, undo history, and scroll.
  const longText = Array.from({ length: 300 }, (_, i) => `// line ${i + 1}`).join("\n") + "\n";
  view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: longText } });
  const cursor = view.state.doc.line(150).from + 3;
  view.dispatch({ selection: EditorSelection.cursor(cursor) });
  await frame();
  view.scrollDOM.scrollTop = 1500;
  await frame();
  const scrolled = view.scrollDOM.scrollTop;
  tabEl(first.id)!.click();
  await frame();
  const showsFirst = tabs.activeId === first.id && !editor.text.startsWith("// line 1\n");
  tabEl(created.id)!.click();
  await frames(3);
  await check(
    "switching tabs keeps each tab's text, cursor, and scroll",
    showsFirst &&
      editor.text === longText &&
      view.state.selection.main.head === cursor &&
      Math.abs(view.scrollDOM.scrollTop - scrolled) < 5,
    JSON.stringify({ showsFirst, head: view.state.selection.main.head, cursor, scroll: view.scrollDOM.scrollTop, scrolled }),
  );
  undo(view);
  await check("undo history survives a tab switch", editor.text.endsWith("// typed\n"), JSON.stringify(editor.text.slice(-40)));

  // Ctrl+Tab cycles (Ctrl on every platform).
  const order = tabs.tabs.map((t) => t.id);
  const at = order.indexOf(created.id);
  pressKey("Tab", { ctrl: true });
  const next = tabs.activeId;
  pressKey("Tab", { ctrl: true, shift: true });
  await check(
    "Ctrl+Tab and Ctrl+Shift+Tab cycle tabs",
    next === order[(at + 1) % order.length] && tabs.activeId === created.id,
    JSON.stringify({ next, active: tabs.activeId, order }),
  );

  // Output and run state are per tab.
  tabEl(first.id)!.click();
  runCode(ctx, 'console.log("first out"); setInterval(() => {}, 1000);\n');
  await until(() => outText().includes("first out"), 15_000);
  tabEl(created.id)!.click();
  await frame();
  const firstRunning = tabEl(first.id)?.querySelector<HTMLElement>(".tab-running")?.hidden === false;
  const statusHere = document.querySelector(".status-run")?.textContent ?? "";
  const end = runs.waitForEnd();
  runCode(ctx, 'console.log("second out")\n');
  await withTimeout(end, 15_000, "second tab run");
  await ctx.output.flushed();
  const secondText = outText();
  tabEl(first.id)!.click();
  await ctx.output.flushed();
  await check(
    "each tab keeps its own output and run state",
    firstRunning &&
      !statusHere.startsWith("Running") &&
      secondText === "second out\n" &&
      outText().startsWith("first out") &&
      (document.querySelector(".status-run")?.textContent ?? "").startsWith("Running"),
    JSON.stringify({ firstRunning, statusHere, secondText, firstText: outText().slice(0, 40) }),
  );
  await runs.cancel();
  await until(() => !runs.isRunning(first.id), 10_000);
  await check("the running indicator clears when the run ends", tabEl(first.id)?.querySelector<HTMLElement>(".tab-running")?.hidden === true);

  // Rename with F2 and with a double-click.
  tabEl(created.id)!.click();
  editor.focus();
  pressKey("F2");
  const renameInput = activeEl()?.querySelector<HTMLInputElement>(".tab-rename");
  if (renameInput) {
    renameInput.value = "Renamed Tab";
    pressKey("Enter", {}, renameInput);
  }
  await until(() => activeEl()?.querySelector(".tab-name")?.textContent === "Renamed Tab", 5000);
  const listing = await invoke<Listing>("list_playgrounds");
  const renamed = listing.playgrounds.find((p) => p.id === created.id);
  await check(
    "F2 renames the playground and its folder",
    !!renameInput && renamed?.name === "Renamed Tab" && /renamed-tab$/.test(renamed.dir),
    JSON.stringify({ input: !!renameInput, name: renamed?.name, dir: renamed?.dir }),
  );
  tabEl(first.id)!.querySelector(".tab-name")!.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
  const dblInput = tabEl(first.id)?.querySelector<HTMLInputElement>(".tab-rename");
  if (dblInput) pressKey("Escape", {}, dblInput);
  await check(
    "double-clicking a tab name starts a rename; Escape cancels it",
    !!dblInput && !tabEl(first.id)?.querySelector(".tab-rename") && tabs.summary(first.id)?.name === "Tabs First",
  );

  // Changed on disk: a clean tab reloads; a dirty one asks.
  tabEl(created.id)!.click();
  await invoke("selftest_touch_source", { id: created.id, text: "// changed outside\n" });
  await check(
    "a clean tab reloads silently when its file changes on disk",
    (await until(() => editor.text === "// changed outside\n", 10_000)) &&
      !visible(document.querySelector(".conflict-bar")) &&
      !visible(empty),
    JSON.stringify(editor.text.slice(0, 40)),
  );
  const bar = () => document.querySelector<HTMLElement>(".conflict-bar")!;
  view.dispatch({ changes: { from: 0, insert: "// mine\n" } });
  await invoke("selftest_touch_source", { id: created.id, text: "// theirs\n" });
  const askedKeep = await until(() => visible(bar()), 10_000);
  bar().querySelector<HTMLButtonElement>(".conflict-keep")!.click();
  await until(() => bar().hidden && !created.dirty, 5000);
  const kept = await invoke<string>("read_source", { id: created.id });
  await check(
    "a dirty tab asks, and Keep mine saves the tab's text",
    askedKeep && !visible(bar()) && kept === "// mine\n// changed outside\n" && editor.text === kept,
    JSON.stringify({ askedKeep, kept }),
  );
  view.dispatch({ changes: { from: 0, insert: "// mine again\n" } });
  await invoke("selftest_touch_source", { id: created.id, text: "// theirs wins\n" });
  const askedReload = await until(() => visible(bar()), 10_000);
  bar().querySelector<HTMLButtonElement>(".conflict-reload")!.click();
  await check(
    "Reload replaces the tab's text with the file",
    askedReload && (await until(() => editor.text === "// theirs wins\n", 5000)) && !created.dirty,
    JSON.stringify(editor.text),
  );

  // New playground through the picker: language, then kind.
  const beforePick = tabs.tabs.length;
  pressKey("n", { mod: true, shift: true });
  const pickedLanguage = await choosePicked("node check");
  const pickedKind = await choosePicked("expression");
  await until(() => tabs.tabs.length === beforePick + 1, 10_000);
  const pickedSummary = tabs.activeId ? tabs.summary(tabs.activeId) : undefined;
  await check(
    "Mod+Shift+N picks a language, then a kind",
    pickedLanguage && pickedKind && pickedSummary?.language === "node-check" && pickedSummary.kind === "expression",
    JSON.stringify({ pickedLanguage, pickedKind, language: pickedSummary?.language, kind: pickedSummary?.kind }),
  );

  // Closing: Mod+W and middle-click.
  const beforeClose = tabs.tabs.length;
  const closing = tabs.activeId;
  pressKey("w", { mod: true });
  await until(() => tabs.tabs.length === beforeClose - 1, 5000);
  const middle = tabs.tabs.find((t) => t.id !== created.id && t.id !== first.id);
  middle?.element.dispatchEvent(new MouseEvent("auxclick", { button: 1, bubbles: true }));
  await until(() => tabs.tabs.length === beforeClose - (middle ? 2 : 1), 5000);
  await check(
    "Mod+W and middle-click close tabs",
    !tabs.find(closing ?? "") && (!middle || !tabs.find(middle.id)),
    JSON.stringify(tabs.tabs.map((t) => t.id)),
  );

  // The session follows the tabs.
  tabEl(created.id)!.click();
  await sleep(900);
  const session = await invoke<Session>("load_session");
  await check(
    "the session file lists the open tabs and the active one",
    JSON.stringify(session.tabs.map((t) => t.playground)) === JSON.stringify(tabs.tabs.map((t) => t.id)) &&
      session.active === created.id &&
      session.lastLanguage === "node-check",
    JSON.stringify(session),
  );

  // Quick open from the empty state.
  for (const t of [...tabs.tabs]) await tabs.close(t);
  pressKey("o", { mod: true });
  const opened = await choosePicked("Renamed Tab");
  await until(() => tabs.activeId === created.id, 5000);
  await check("Mod+O opens a playground by name", opened && tabs.activeId === created.id && empty.hidden === true);

  // Restoring a session reopens its tabs, skips missing ones, and says so.
  for (const t of [...tabs.tabs]) await tabs.close(t);
  const toastsBefore = document.querySelectorAll(".toast").length;
  const restored = await tabs.restoreSession({
    tabs: [
      { playground: first.id, scrollTop: 0 },
      { playground: "01ZZZZZZZZZZZZZZZZZZZZZZZZ", scrollTop: 0 },
      { playground: created.id, scrollTop: 0 },
    ],
    active: created.id,
    lastLanguage: "node-check",
  });
  await frame();
  const warned = [...document.querySelectorAll(".toast")]
    .slice(toastsBefore)
    .some((t) => (t.textContent ?? "").includes("no longer in the library"));
  await check(
    "restoring a session reopens tabs and skips a missing playground with a toast",
    restored.opened === 2 &&
      restored.skipped === 1 &&
      JSON.stringify(tabs.tabs.map((t) => t.id)) === JSON.stringify([first.id, created.id]) &&
      tabs.activeId === created.id &&
      warned,
    JSON.stringify({ restored, warned }),
  );
}

/**
 * First of two launches: leave a session with two tabs and a scrolled active
 * tab. `session-restore` checks the next launch reopens it.
 */
async function sessionPrepareScenario(ctx: SelftestContext, check: Check) {
  const { tabs, editor } = ctx;
  for (const t of [...tabs.tabs]) await tabs.close(t);
  const a = await openTestPlayground(ctx, "Session A");
  const b = await openTestPlayground(ctx, "Session B");
  editor.view.dispatch({
    changes: { from: 0, to: editor.view.state.doc.length, insert: Array.from({ length: 300 }, (_, i) => `// ${i}`).join("\n") },
  });
  await tabs.save(tabs.find(b.id)!);
  await frames(2);
  editor.view.scrollDOM.scrollTop = 1200;
  await frames(2);
  await tabs.saveSession();
  const session = await invoke<Session>("load_session");
  await check(
    "session saved for the next launch",
    session.tabs.length === 2 && session.active === b.id && (session.tabs[1]?.scrollTop ?? 0) > 1000,
    JSON.stringify(session),
  );
  await report(`INFO session tabs ${a.id} ${b.id}`);
}

/** Second launch: the tabs from `session-prepare` are back. */
async function sessionRestoreScenario(ctx: SelftestContext, check: Check) {
  const { tabs, editor } = ctx;
  const session = await invoke<Session>("load_session");
  await frames(3);
  const names = tabs.tabs.map((t) => tabs.summary(t.id)?.name);
  await check(
    "the last session's tabs reopen at launch, with the active tab and its scroll",
    JSON.stringify(names) === JSON.stringify(["Session A", "Session B"]) &&
      tabs.activeId === session.active &&
      Math.abs(editor.view.scrollDOM.scrollTop - (session.tabs[1]?.scrollTop ?? -1)) < 5,
    JSON.stringify({ names, active: tabs.activeId, scroll: editor.view.scrollDOM.scrollTop, session }),
  );
}

/** Fill the open prompt's fields and submit. */
async function fillPrompt(values: string[]): Promise<boolean> {
  if (!(await until(() => document.querySelector(".prompt .picker-input") !== null, 5000))) return false;
  const inputs = [...document.querySelectorAll<HTMLInputElement>(".prompt .picker-input")];
  values.forEach((value, i) => {
    if (inputs[i]) inputs[i]!.value = value;
  });
  pressKey("Enter", {}, inputs[0]);
  return inputs.length === values.length;
}

/** Open the palette with Mod+Shift+P and choose the command matching `filter`. */
async function runCommand(filter: string): Promise<boolean> {
  pressKey("p", { mod: true, shift: true });
  return choosePicked(filter);
}

/** The command palette, the prompt, and inline directives (01-14). */
async function packagesScenario(ctx: SelftestContext, check: Check) {
  const { editor, tabs } = ctx;
  const view = editor.view;
  const text = () => view.state.doc.toString();
  const note = () => document.querySelector(".status-note")?.textContent ?? "";
  const toastText = () =>
    [...document.querySelectorAll(".toast")].map((t) => t.textContent ?? "").join(" | ");
  const clearToasts = () => document.querySelectorAll(".toast").forEach((t) => t.remove());

  // With nothing open there is nothing to add a package to.
  for (const t of [...tabs.tabs]) await tabs.close(t);
  const opened = await runCommand("Add package");
  await until(() => note() !== "", 3000);
  await check(
    "Mod+Shift+P lists Add package, which says there is nothing open",
    opened && note().includes("Open a playground"),
    JSON.stringify({ opened, note: note() }),
  );
  await check("the palette closes after choosing", document.querySelector(".picker") === null);

  // C# adds packages by putting a directive at the top of the file.
  const csharp = await invoke<Playground>("create_playground", {
    name: "Packages Directive",
    language: "csharp",
    kind: "statements",
  });
  await openPlayground(editor, ctx.status, csharp.id);
  const before = text();

  let done = runCommand("Add package");
  await check("choosing it prompts for a package", await fillPrompt(["Newtonsoft.Json", "13.0.3"]));
  await done;
  await until(() => text() !== before, 5000);
  await check(
    "the directive goes on the first line, and the rest of the file is untouched",
    text() === `#:package Newtonsoft.Json@13.0.3\n${before}`,
    JSON.stringify(text().slice(0, 80)),
  );
  await check(
    "it is saved at once, so the next run compiles with it",
    tabs.activeTab?.savedText === text() && tabs.activeTab?.dirty === false,
    JSON.stringify({ dirty: tabs.activeTab?.dirty }),
  );

  // A second package goes after the first, not above it.
  done = runCommand("Add package");
  await fillPrompt(["Humanizer", ""]);
  await done;
  // The template mentions Humanizer in a comment, so wait for the line itself.
  await until(() => text().split("\n")[1]?.startsWith("#:package Humanizer") === true, 5000);
  await check(
    "another package goes after the directives already there, with no version if none was given",
    text() === `#:package Newtonsoft.Json@13.0.3\n#:package Humanizer\n${before}`,
    JSON.stringify(text().split("\n").slice(0, 3)),
  );

  // The same package again replaces its line instead of adding a second.
  done = runCommand("Add package");
  await fillPrompt(["Newtonsoft.Json", "13.0.1"]);
  await done;
  await until(() => text().includes("13.0.1"), 5000);
  await check(
    "adding the same package again changes its line, and does not add a second",
    text() === `#:package Newtonsoft.Json@13.0.1\n#:package Humanizer\n${before}`,
    JSON.stringify(text().split("\n").slice(0, 3)),
  );

  // Nothing is applied when the core refuses the name.
  const unchanged = text();
  clearToasts();
  done = runCommand("Add package");
  await fillPrompt(["--global", ""]);
  await done;
  await until(() => toastText().includes("dash"), 5000);
  await check(
    "a name that would change the command is refused, and nothing is inserted",
    text() === unchanged && toastText().includes("contain"),
    JSON.stringify({ toast: toastText().slice(0, 120) }),
  );

  // A kind whose first lines are locked cannot take a directive, since the
  // directive has to be the first thing in the file.
  const expression = await invoke<Playground>("create_playground", {
    name: "Packages Locked",
    language: "csharp",
    kind: "expression",
  });
  await openPlayground(editor, ctx.status, expression.id);
  const locked = text();
  clearToasts();
  done = runCommand("Add package");
  await fillPrompt(["Newtonsoft.Json", ""]);
  await done;
  await until(() => toastText().includes("locked"), 5000);
  await check(
    "a kind with locked lines refuses rather than writing a broken file",
    text() === locked && toastText().includes("locked"),
    JSON.stringify({ changed: text() !== locked, toast: toastText().slice(0, 120) }),
  );

  // A plugin with no [deps] section says so.
  clearToasts();
  const plain = await openTestPlayground(ctx, "Packages Unsupported");
  const plainText = text();
  done = runCommand("Add package");
  await fillPrompt(["anything", ""]);
  await done;
  await until(() => toastText().includes("deps"), 5000);
  await check(
    "a plugin that does not say how to add packages reports that",
    text() === plainText && toastText().includes("[deps]"),
    JSON.stringify({ toast: toastText().slice(0, 120) }),
  );

  for (const t of [...tabs.tabs]) await tabs.close(t);
  for (const id of [plain.id, expression.id, csharp.id]) {
    await invoke("delete_playground", { id });
  }
}

/**
 * The real thing: add a package with each bundled plugin and use it in the
 * next run. Needs the network, so it is not part of CI (work item 01-14).
 */
async function packagesLiveScenario(ctx: SelftestContext, check: Check) {
  const { editor, tabs, runs } = ctx;
  const view = editor.view;
  const outText = () => ctx.output.element.querySelector(".output-body")?.textContent ?? "";

  const cases = [
    {
      language: "typescript",
      kind: "statements",
      pkg: "zod",
      version: "",
      code: 'import { z } from "zod";\nconsole.log("zod says", z.string().parse("ok"));\n',
      expect: "zod says ok",
    },
    {
      language: "rust",
      kind: "program",
      pkg: "serde",
      version: "1",
      code: 'fn takes<T: serde::Serialize>(_v: T) {}\n\nfn main() {\n    takes(7);\n    println!("serde ok");\n}\n',
      expect: "serde ok",
    },
    {
      language: "csharp",
      kind: "statements",
      pkg: "Newtonsoft.Json",
      version: "13.0.3",
      code: "Console.WriteLine(Newtonsoft.Json.JsonConvert.SerializeObject(new { a = 1 }));\n",
      expect: '{"a":1}',
    },
  ];

  for (const c of cases) {
    const state = await settledToolchain(c.language);
    if (state?.kind !== "ready") {
      await check(`${c.language} toolchain is ready`, false, JSON.stringify(state ?? null));
      continue;
    }
    const playground = await invoke<Playground>("create_playground", {
      name: `Packages ${c.pkg}`,
      language: c.language,
      kind: c.kind,
    });
    await openPlayground(editor, ctx.status, playground.id);
    // The code goes in first: adding a package edits the file too.
    view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: c.code } });

    const started = performance.now();
    const end = runs.waitForEnd();
    const done = runCommand("Add package");
    await fillPrompt([c.pkg, c.version]);
    await done;
    // Either a command runs to the end (npm, cargo) or a directive appears in
    // the file; which one depends on the plugin, so wait for whichever comes.
    const how = await Promise.race([
      end.then(() => "command" as const),
      until(() => view.state.doc.toString() !== c.code, 300_000).then(() => "directive" as const),
    ]);
    await report(`INFO ${c.language}: ${c.pkg} added by ${how}`);
    await report(`INFO ${c.language}: added ${c.pkg} in ${((performance.now() - started) / 1000).toFixed(1)} s`);

    const runStarted = performance.now();
    const runEnd = runs.waitForEnd();
    void runs.run(playground.id, editor.text);
    const outcome = await withTimeout(runEnd, 300_000, `running with ${c.pkg}`);
    await ctx.output.flushed();
    await check(
      `${c.language}: the next run picks up ${c.pkg} with no extra step`,
      outcome.kind === "exited" && outcome.code === 0 && outText().includes(c.expect),
      JSON.stringify({ outcome, out: outText().slice(0, 200) }),
    );
    await report(`INFO ${c.language}: first run with ${c.pkg} took ${((performance.now() - runStarted) / 1000).toFixed(1)} s`);
    await tabs.close(tabs.activeTab!);
    await invoke("delete_playground", { id: playground.id });
  }

  // A package that does not exist: npm fails, and the user sees why.
  const failing = await invoke<Playground>("create_playground", {
    name: "Packages Missing",
    language: "typescript",
    kind: "statements",
  });
  await openPlayground(editor, ctx.status, failing.id);
  const failed = runs.waitForEnd();
  const chosen = runCommand("Add package");
  await fillPrompt(["tinkerpad-no-such-package-9a7f2", ""]);
  await chosen;
  const outcome = await withTimeout(failed, 300_000, "adding a package that does not exist");
  await ctx.output.flushed();
  await check(
    "a package that does not exist fails with the tool's own message in the pane",
    outcome.kind === "exited" && outcome.code !== 0 && /404|not found/i.test(outText()),
    JSON.stringify({ outcome, out: outText().slice(-200) }),
  );
  await tabs.close(tabs.activeTab!);
  await invoke("delete_playground", { id: failing.id });
}

/**
 * The editor's language server client against a real rust-analyzer (02-04).
 *
 * Skipped with an INFO line when rust-analyzer isn't installed, so the rest of
 * the self-test still runs on a machine without it.
 */
async function lspScenario(ctx: SelftestContext, check: Check) {
  const { editor, tabs } = ctx;
  const view = editor.view;

  const status = await invoke<PluginsStatus>("list_plugins");
  const rust = status.plugins.find((p) => p.id === "rust");
  await check(
    "the Rust plugin declares a language server and a highlighter",
    rust?.lsp === true && rust.highlighter === "rust",
    JSON.stringify({ lsp: rust?.lsp, highlighter: rust?.highlighter }),
  );
  const toolchain = await settledToolchain("rust");
  if (toolchain?.kind !== "ready") {
    await report("INFO rust toolchain missing; skipping the language server checks");
    return;
  }

  const playground = await invoke<Playground>("create_playground", {
    name: "Lsp Test",
    language: "rust",
    kind: "program",
  });
  const source = [
    "fn main() {",
    '    let greeting = String::from("hello");',
    "    println!(\"{greeting}\");",
    "}",
    "",
  ].join("\n");
  const openedAt = performance.now();
  await openPlayground(editor, ctx.status, playground.id);
  view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: source } });
  await check(
    "the tab is usable straight away, without waiting for the server",
    performance.now() - openedAt < 2000 && view.state.doc.toString() === source,
    `${(performance.now() - openedAt).toFixed(0)} ms`,
  );

  // The grammar is a separate chunk, loaded when the first Rust tab opens.
  // CodeMirror styles tokens with generated class names, so the test asks the
  // same question a reader would: is any of this text coloured?
  const highlighted = await until(
    () => document.querySelector(".cm-content .cm-line span[class]") !== null,
    20_000,
  );
  await check(
    "the Rust grammar loads on demand and highlights the document",
    highlighted,
    document.querySelector(".cm-content")?.innerHTML.slice(0, 120) ?? "",
  );

  const tab = tabs.activeTab!;
  const connected = await until(() => tab.lsp !== null, 120_000);
  if (!connected) {
    await report("INFO rust-analyzer did not connect; skipping the rest");
    await tabs.close(tab);
    await invoke("delete_playground", { id: playground.id });
    return;
  }
  await check("the tab connects to rust-analyzer", tab.lsp !== null, tab.lsp?.uri ?? "");

  // Completions. Type a dot after a String and ask for the popup, the way
  // typing one does.
  const dotAt = view.state.doc.line(3).from;
  view.dispatch({
    changes: { from: dotAt, insert: "    greeting.\n" },
    selection: { anchor: dotAt + "    greeting.".length },
  });
  // A cold server answers with nothing while it is still indexing, so ask
  // again until it has something, the way a person would keep typing.
  const requestedAt = performance.now();
  let completions: readonly { label: string }[] = [];
  let gotCompletions = false;
  for (let attempt = 0; attempt < 60 && !gotCompletions; attempt++) {
    startCompletion(view);
    gotCompletions = await until(() => {
      completions = currentCompletions(view.state);
      return completions.length > 0;
    }, 2000);
  }
  const completionMs = performance.now() - requestedAt;
  await check(
    "typing `.` offers the methods rust-analyzer knows about",
    gotCompletions && completions.some((c) => c.label.startsWith("len")),
    JSON.stringify(completions.slice(0, 6).map((c) => c.label)),
  );
  await report(
    `INFO first completion, from a cold server, took ${(completionMs / 1000).toFixed(1)} s`,
  );

  // Then the number the work item asks for: a warm server, typing a
  // character, to the popup. CodeMirror starts the request itself, which is
  // the path a person is on.
  const samples: number[] = [];
  for (let n = 0; n < 5; n++) {
    closeCompletion(view);
    await frame();
    const at = performance.now();
    view.dispatch({
      changes: { from: view.state.selection.main.head, insert: "l" },
      selection: { anchor: view.state.selection.main.head + 1 },
      userEvent: "input.type",
    });
    if (await until(() => currentCompletions(view.state).length > 0, 30_000)) {
      samples.push(performance.now() - at);
    }
    view.dispatch({
      changes: { from: view.state.selection.main.head - 1, to: view.state.selection.main.head },
    });
  }
  samples.sort((a, b) => a - b);
  const median = samples[Math.floor(samples.length / 2)] ?? Number.POSITIVE_INFINITY;
  await check(
    "a warm server answers a keystroke quickly",
    samples.length >= 3 && median < 1000,
    JSON.stringify(samples.map((ms) => Math.round(ms))),
  );
  await report(
    `INFO warm completion: median ${median.toFixed(0)} ms, best ${(samples[0] ?? 0).toFixed(0)} ms, ${samples.length} samples`,
  );
  closeCompletion(view);

  // Tab takes the highlighted completion, the way Enter does.
  const before = view.state.doc.toString();
  startCompletion(view);
  const offered = await until(() => currentCompletions(view.state).length > 0, 30_000);
  const taken = offered && runScopeHandlers(view, key("Tab"), "editor");
  await frame();
  await check(
    "Tab accepts the highlighted completion",
    taken && view.state.doc.toString() !== before,
    view.state.doc.line(3).text.trim(),
  );
  closeCompletion(view);

  // Hover from the keyboard: Mod+K Mod+I, as in VS Code.
  const stringAt = view.state.doc.toString().indexOf("String::from");
  view.dispatch({ selection: { anchor: stringAt + 2 } });
  runScopeHandlers(view, key("k", { mod: true }), "editor");
  runScopeHandlers(view, key("i", { mod: true }), "editor");
  const hovered = await until(
    () => (document.querySelector(".cm-lsp-hover")?.textContent ?? "").length > 0,
    30_000,
  );
  await check(
    "Mod+K Mod+I shows what the server knows about the symbol",
    hovered,
    (document.querySelector(".cm-lsp-hover")?.textContent ?? "").slice(0, 80),
  );
  runScopeHandlers(view, key("Escape"), "editor");
  await frame();
  await check(
    "Escape dismisses it",
    document.querySelector(".cm-lsp-hover") === null,
  );

  // Diagnostics: a syntax error is reported by the server and walkable with F8.
  view.dispatch({
    changes: { from: 0, to: view.state.doc.length, insert: "fn main() {\n    let x: i32 =\n}\n" },
  });
  const marked = await until(() => countDiagnostics(view) > 0, 60_000);
  await check(
    "a broken file gets diagnostics from the server",
    marked,
    `${countDiagnostics(view)} diagnostics`,
  );
  const squiggles = document.querySelectorAll(".cm-lintRange, .cm-lint-marker").length;
  await check("they are drawn in the editor and its gutter", squiggles > 0, `${squiggles} marks`);

  view.dispatch({ selection: { anchor: 0 } });
  ctx.handleGlobalKey(key("F8"));
  await frame();
  const line = view.state.doc.lineAt(view.state.selection.main.head).number;
  await check("F8 walks the server's diagnostics too", line > 1, `line ${line}`);

  // Closing the tab closes the document; the server stays up for the next one
  // (02-02), so a second tab connects without another cold start.
  await tabs.close(tab);
  const second = await invoke<Playground>("create_playground", {
    name: "Lsp Second",
    language: "rust",
    kind: "program",
  });
  const secondAt = performance.now();
  await openPlayground(editor, ctx.status, second.id);
  const secondTab = tabs.activeTab!;
  const reconnected = await until(() => secondTab.lsp !== null, 60_000);
  const reconnectMs = performance.now() - secondAt;
  await check("a second Rust tab connects to the server already running", reconnected, `${reconnectMs.toFixed(0)} ms`);
  await report(`INFO second tab connected in ${reconnectMs.toFixed(0)} ms`);

  await tabs.close(secondTab);
  for (const id of [second.id, playground.id]) {
    // rust-analyzer keeps the playground folder open (its workspace is that
    // folder until 02-07), and Windows won't delete a folder in use. The
    // scenario is about the editor, so a failure here is reported, not fatal.
    try {
      await invoke("delete_playground", { id });
    } catch (e) {
      await report(`INFO could not delete ${id} while its server is running: ${String(e)}`);
    }
  }
}

/** How many diagnostics the editor is showing. */
function countDiagnostics(view: EditorView): number {
  let count = 0;
  forEachDiagnostic(view.state, () => {
    count++;
  });
  return count;
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
  typescript: typescriptScenario,
  csharp: csharpScenario,
  rust: rustScenario,
  tabs: tabsScenario,
  annotations: annotationsScenario,
  "session-prepare": sessionPrepareScenario,
  "session-restore": sessionRestoreScenario,
  plugins: pluginsScenario,
  toolchains: toolchainsScenario,
  library: libraryScenario,
  "library-trash": libraryTrashScenario,
  locked: lockedScenario,
  lsp: lspScenario,
  packages: packagesScenario,
  "packages-live": packagesLiveScenario,
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
async function lockedScenario({ editor, status, tabs }: SelftestContext, check: Check) {
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
  await tabs.save(tabs.find(expr.id)!);
  const onDisk = await invoke<string>("read_source", { id: expr.id });
  await check("file on disk is byte-identical to the editor", onDisk === text(), JSON.stringify(onDisk));

  // Edited outside the app without the wrapper: the clean tab reloads, nothing
  // is locked, and the status bar says why.
  await invoke("selftest_touch_source", { id: expr.id, text: "40 + 2\n" });
  await until(() => text() === "40 + 2\n", 10_000);
  await frame();
  const mismatch = tabs.find(expr.id)?.opened.lock;
  await check(
    "a file without its wrapper opens unlocked",
    mismatch === "mismatch" && document.querySelectorAll(".cm-locked-line").length === 0,
    `${mismatch} ${JSON.stringify(text())}`,
  );
  await check("the status bar explains why", noteShown() && note() === MISMATCH_NOTE, note());
  view.dispatch({ changes: { from: 0, insert: "// free to edit\n" }, userEvent: "input.type" });
  await check("an unlocked file is fully editable", text().startsWith("// free to edit\n"));
}
