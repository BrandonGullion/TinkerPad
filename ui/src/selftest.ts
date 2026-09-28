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
import type {
  LanguageToolchain,
  Listing,
  Peek,
  Playground,
  RunRecord,
  SearchHit,
  Settings,
  PluginsStatus,
  RunEvent,
  Session,
} from "./protocol";
import type { RunController } from "./run";
import { MAX_PEEKS, type TabManager } from "./tabs";

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
function key(k: string, mods: { mod?: boolean; shift?: boolean; alt?: boolean } = {}): KeyboardEvent {
  return new KeyboardEvent("keydown", {
    key: k,
    bubbles: true,
    cancelable: true,
    ctrlKey: !isMac && !!mods.mod,
    metaKey: isMac && !!mods.mod,
    shiftKey: !!mods.shift,
    altKey: !!mods.alt,
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

  // The test plugin has a helper, and a run that has to build one first shows
  // the build in its output (03-03). That is checked in `helper-run`; here it
  // would only get in the way of comparing a program's output, so the helper
  // is built and warm before the first run.
  await invoke("start_helper", { plugin: "node-check" }).catch(() => {});

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

  // Different playgrounds can run at the same time, on the per-run path. The
  // test plugin has a helper, and a helper runs one program at a time, so two
  // playgrounds of its language queue instead — which is what the `helper-run`
  // scenario checks (03-03).
  await invoke("force_per_run", { forced: true });
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
  await invoke("force_per_run", { forced: false });

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

/** Wait until an async `ok()` holds, up to `ms`. */
async function untilAsync(ok: () => Promise<boolean>, ms = 3000): Promise<boolean> {
  const end = performance.now() + ms;
  while (!(await ok())) {
    if (performance.now() > end) return false;
    await frame();
  }
  return true;
}

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

  // The helper (03-06): warm, and honest about what it can't do.
  await openPlayground(editor, status, created.id);
  const runStatus = () => document.querySelector(".status-run")?.textContent ?? "";
  let ending = runs.waitForEnd();
  runCode(ctx, 'console.log("warm?");\nawait new Promise((r) => setTimeout(r, 1200));\n');
  const warm = await until(() => runStatus().includes("warm"), 30_000);
  await check("TypeScript runs on its helper", warm, runStatus());
  await withTimeout(ending, 30_000, "warm run");

  const before = (await invoke<HelperStatus[]>("helper_status")).find((h) => h.plugin === "typescript");
  const wasPid = before?.state.kind === "ready" ? (before.state.pid ?? 0) : 0;

  // Cancel: this helper can't stop a running module, so the core kills it and
  // starts another. The run ends, which is what was asked for.
  ending = runs.waitForEnd();
  runCode(ctx, 'console.log("looping");\nsetInterval(() => {}, 50);\n');
  await until(() => runStatus().startsWith("Running"), 30_000);
  await sleep(300);
  await runs.cancel();
  const cancelled = await withTimeout(ending, 30_000, "cancel");
  await check("a run that won't stop is cancelled anyway", cancelled.kind === "cancelled", JSON.stringify(cancelled));

  const back = await untilAsync(async () => {
    const helper = (await invoke<HelperStatus[]>("helper_status")).find((h) => h.plugin === "typescript");
    return helper?.state.kind === "ready" && (helper.state.pid ?? 0) !== wasPid;
  }, 30_000);
  await check("and the helper comes back as a new process", back, `was ${wasPid}`);

  // Leave a table and some text on screen for a screenshot.
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
  const { editor, status, runs } = ctx;
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
  // On the helper a compile error is the compiler's own diagnostic, not a line
  // of output matched by a pattern: the run `failed`, and the position came
  // with it (03-05).
  const broken = await runUntilVisible(ctx, ".dump", "C# compile error", LONG);
  await frame();
  const marked = document.querySelector<HTMLElement>(".cm-annotation");
  await check(
    // The compiler points one past the last character — that is where the
    // semicolon should have been — so the squiggle goes under what it follows.
    "a compile error fails the run and marks where the semicolon should be",
    broken.outcome.kind === "failed" &&
      broken.outcome.message.includes("CS1002: ; expected") &&
      marked?.textContent === ")" &&
      (marked.title ?? "").includes("; expected"),
    JSON.stringify({
      outcome: broken.outcome,
      mark: marked?.textContent,
      title: marked?.title,
      text: outText().slice(0, 120),
    }),
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

  // A NuGet package, resolved by the helper into the same global folder
  // `dotnet restore` uses (03-05). The first run may download it.
  await openPlayground(editor, status, created.id);
  editor.view.dispatch({
    changes: {
      from: 0,
      to: editor.view.state.doc.length,
      insert: '#:package Humanizer@2.14.1\n\nusing Humanizer;\n\nConsole.WriteLine("SomeMethodName".Humanize());\n',
    },
  });
  const packaged = await runUntilVisible(ctx, ".out-stdout", "C# with a package", LONG);
  if (packaged.outcome.kind === "failed" && /nuget|network|download/i.test(packaged.outcome.message)) {
    await report(`INFO nuget.org is not reachable; skipping the package check (${packaged.outcome.message})`);
  } else {
    await check(
      "a #:package directive is resolved and used",
      packaged.outcome.kind === "exited" && outText().trim() === "Some method name",
      JSON.stringify({ outcome: packaged.outcome, text: outText().slice(0, 200) }),
    );
    const again = await runUntilVisible(ctx, ".out-stdout", "C# with a cached package", LONG);
    await check(
      "and the second run doesn't fetch it again",
      again.outcome.kind === "exited" && (again.ms ?? 1e9) < (packaged.ms ?? 0) + 1000,
      JSON.stringify({ first: packaged.ms, second: again.ms }),
    );
  }

  // Cancel reaches code running inside the helper, and leaves it warm.
  editor.view.dispatch({
    changes: {
      from: 0,
      to: editor.view.state.doc.length,
      insert: 'Console.WriteLine("looping");\nwhile (true) { Thread.Sleep(20); }\n',
    },
  });
  const cancelling = runs.waitForEnd();
  runScopeHandlers(editor.view, key("Enter", { mod: true }), "editor");
  await until(() => (document.querySelector(".status-run")?.textContent ?? "").startsWith("Running"), LONG);
  await sleep(300);
  await runs.cancel();
  const cancelled = await withTimeout(cancelling, 30_000, "C# cancel");
  await check("a runaway loop can be cancelled", cancelled.kind === "cancelled", JSON.stringify(cancelled));
  const helper = (await invoke<HelperStatus[]>("helper_status")).find((h) => h.plugin === "csharp");
  await check(
    "and the helper is still warm afterwards",
    helper?.state.kind === "ready",
    JSON.stringify(helper?.state ?? null),
  );

  // Leave a table on screen for a screenshot.
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

  // Output and run state are per tab. Two runs at once, so this part goes on
  // the per-run path: one helper runs one program at a time, and the second
  // tab would wait for the first rather than show its own output (03-03).
  await invoke("force_per_run", { forced: true });
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
  await invoke("force_per_run", { forced: false });

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

  // With nothing open there is nothing to add a package to, so the palette
  // doesn't offer it: a command that can't run isn't listed (work item 04-03).
  for (const t of [...tabs.tabs]) await tabs.close(t);
  const offered = await runCommand("Add package");
  await check(
    "Mod+Shift+P leaves out commands that need a playground",
    !offered,
    JSON.stringify({ offered, note: note() }),
  );
  pressKey("Escape");
  await until(() => document.querySelector(".picker") === null, 3000);
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

  // A plugin with no [deps] section and no add_dependency hook says so. The
  // test plugin has both (03-07), so this needs one that has neither: `mini`,
  // when it is installed. Without it there is nothing to check here.
  clearToasts();
  const plugins = await invoke<PluginsStatus>("list_plugins");
  let plain: Playground | null = null;
  if (plugins.plugins.some((p) => p.id === "mini")) {
    plain = await invoke<Playground>("create_playground", {
      name: "Packages Unsupported",
      language: "mini",
    });
    await openPlayground(editor, ctx.status, plain.id);
    const plainText = text();
    done = runCommand("Add package");
    await fillPrompt(["anything", ""]);
    await done;
    await until(() => toastText().includes("deps"), 5000);
    await check(
      "a plugin that says nothing about packages reports that",
      text() === plainText && toastText().includes("[deps]"),
      JSON.stringify({ toast: toastText().slice(0, 120) }),
    );
  } else {
    await report("INFO no plugin without [deps] is installed, so that check was skipped");
  }

  for (const t of [...tabs.tabs]) await tabs.close(t);
  for (const id of [plain?.id, expression.id, csharp.id].filter((id) => id !== undefined)) {
    try {
      await invoke("delete_playground", { id });
    } catch (e) {
      await report(`INFO could not delete ${id}: ${JSON.stringify(e)}`);
    }
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

  // From no tabs, so the checks below are about this scenario's playgrounds
  // and not whatever an earlier one left open.
  for (const t of [...tabs.tabs]) await tabs.close(t);

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
  // Under load — the suite has just built several crates — rust-analyzer can
  // take a while to index before it reports anything.
  const marked = await until(() => countDiagnostics(view) > 0, 180_000);
  await check(
    "a broken file gets diagnostics from the server",
    marked,
    `${countDiagnostics(view)} diagnostics`,
  );
  const squiggles = document.querySelectorAll(".cm-lintRange, .cm-lint-marker").length;
  await check("they are drawn in the editor and its gutter", squiggles > 0, `${squiggles} marks`);

  const markedLines: number[] = [];
  forEachDiagnostic(view.state, (_, from) => {
    markedLines.push(view.state.doc.lineAt(from).number);
  });
  view.dispatch({ selection: { anchor: 0 } });
  ctx.handleGlobalKey(key("F8"));
  await frame();
  const line = view.state.doc.lineAt(view.state.selection.main.head).number;
  await check(
    "F8 walks the server's diagnostics too",
    markedLines.includes(line),
    JSON.stringify({ cursor: line, diagnostics: markedLines }),
  );

  // F12 into the standard library: a definition in a file that is nobody's
  // playground, opened read-only (02-09). Needs rust-src, which rustup
  // installs by default but a slim toolchain may not have.
  view.dispatch({
    changes: {
      from: 0,
      to: view.state.doc.length,
      insert: ["fn main() {", "    let s = String::new();", "    let _ = s;", "}", ""].join("\n"),
    },
    // On `Stri` of `String::new()`, which is where a person would press F12.
    selection: { anchor: ["fn main() {", "    let s = Stri"].join("\n").length },
  });
  runScopeHandlers(view, key("F12"), "editor");
  const jumped = await until(() => tabs.tabs.some((t) => t.peek !== undefined), 60_000);
  const landed = tabs.tabs.find((t) => t.peek)?.peek?.path ?? "";
  if (jumped) {
    await check(
      "F12 opens the standard library source read-only",
      landed.endsWith(".rs") && landed.includes("library"),
      landed,
    );
    await tabs.close(tabs.activeTab!);
    ctx.handleGlobalKey(key("ArrowLeft", { alt: true }));
    await frame();
  } else {
    await report("INFO no definition for String::new; rust-src may be missing");
  }

  // Back to something worth completing after that excursion.
  const completable = ["fn main() {", "    let s = String::new();", "    s.", "}", ""].join("\n");
  view.dispatch({
    changes: { from: 0, to: view.state.doc.length, insert: completable },
    selection: { anchor: completable.indexOf("    s.") + "    s.".length },
  });

  // A restart under a live tab: the documents are opened again by the core
  // (02-05), so the editor carries on without touching anything itself.
  const rustServer = (await invoke<ServerStatus[]>("server_status")).find(
    (server) => server.language === "rust",
  );
  const restarted = await invoke<boolean>("restart_server", {
    language: "rust",
    root: rustServer?.root ?? null,
  });
  await check("a language server can be restarted", restarted);
  // Keep typing until the answers come back, the way a person would.
  let working = false;
  const givingUpAt = performance.now() + 120_000;
  while (!working && performance.now() < givingUpAt) {
    closeCompletion(view);
    view.dispatch({
      changes: { from: view.state.selection.main.head, insert: "l" },
      selection: { anchor: view.state.selection.main.head + 1 },
      userEvent: "input.type",
    });
    working = await until(() => currentCompletions(view.state).length > 0, 2000);
    view.dispatch({
      changes: { from: view.state.selection.main.head - 1, to: view.state.selection.main.head },
    });
  }
  await check(
    "completions come back after the server restarts, with no reopening by the editor",
    working,
  );
  await check(
    "and the user is not shown an error banner for the requests it lost",
    document.querySelector(".cm-lsp-message-error") === null,
    document.querySelector(".cm-lsp-message-error")?.textContent ?? "",
  );
  closeCompletion(view);

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

  // One server, two playgrounds: the second was added to its linkedProjects
  // rather than starting a second rust-analyzer (02-07).
  const rustServers = (await invoke<ServerStatus[]>("server_status")).filter(
    (server) => server.language === "rust",
  );
  await check(
    "both Rust playgrounds are served by one process",
    rustServers.length === 1 && (rustServers[0]?.state.pid ?? 0) > 0,
    JSON.stringify(rustServers.map((server) => ({ root: server.root, pid: server.state.pid }))),
  );
  await check(
    "rooted at a workspace of TinkerPad's own, not at a playground",
    (rustServers[0]?.root ?? "").includes("scratch"),
    rustServers[0]?.root ?? "",
  );

  // And the second playground gets completions of its own, which is the whole
  // point of registering it.
  view.dispatch({
    changes: {
      from: 0,
      to: view.state.doc.length,
      insert: 'fn main() {\n    let v: Vec<u32> = Vec::new();\n    v.\n}\n',
    },
    selection: { anchor: "fn main() {\n    let v: Vec<u32> = Vec::new();\n    v.".length },
  });
  const secondCompletions = performance.now();
  let alsoOffered: readonly { label: string }[] = [];
  let alsoFound = false;
  for (let attempt = 0; attempt < 60 && !alsoFound; attempt++) {
    startCompletion(view);
    alsoFound = await until(() => {
      alsoOffered = currentCompletions(view.state);
      return alsoOffered.length > 0;
    }, 2000);
  }
  await check(
    "the second playground gets completions too, from the same server",
    alsoFound && alsoOffered.some((c) => c.label.startsWith("len")),
    JSON.stringify(alsoOffered.slice(0, 5).map((c) => c.label)),
  );
  await report(
    `INFO first completion in a second playground took ${((performance.now() - secondCompletions) / 1000).toFixed(1)} s`,
  );
  closeCompletion(view);

  await tabs.close(secondTab);
  // The server is rooted elsewhere now, but it has loaded these crates and
  // watches their folders, so Windows can still refuse to delete one while it
  // is running. Reported, not fatal: this scenario is about the editor.
  for (const id of [second.id, playground.id]) {
    try {
      await invoke("delete_playground", { id });
    } catch (e) {
      await report(
        `INFO could not delete ${id} while the language server watches it: ${JSON.stringify(e)}`,
      );
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

/** What the pool has running, from the app's own view of it. */
interface ServerStatus {
  language: string;
  root: string;
  state: { kind: string; pid?: number; rssBytes?: number | null; reason?: string | null };
}

/** The same, for a plugin's warm helper (work item 03-02). */
interface HelperStatus {
  plugin: string;
  uses: number;
  state: { kind: string; pid?: number; name?: string | null; reason?: string | null };
}

/**
 * Pinned language servers start at launch, before any tab is open (02-06).
 *
 * The scenario is run with TINKERPAD_LSP_PREWARM naming the test plugin's
 * server and a language that doesn't exist, so it also covers the "logs and
 * carries on" case.
 */
async function prewarmScenario(ctx: SelftestContext, check: Check) {
  const servers = async (): Promise<ServerStatus[]> => invoke<ServerStatus[]>("server_status");

  // Nothing is open, and the tab strip is empty: anything running was started
  // by pre-warm alone.
  for (const t of [...ctx.tabs.tabs]) await ctx.tabs.close(t);

  let warmed: ServerStatus | undefined;
  const ready = await untilAsync(async () => {
    warmed = (await servers()).find((s) => s.language === "node-check");
    return warmed?.state.kind === "ready";
  }, 30_000);
  await check(
    "a pinned language server is up with no playground open",
    ready && ctx.tabs.tabs.length === 0,
    JSON.stringify(warmed ?? null),
  );
  await check(
    "and it is a real process, rooted at the library",
    (warmed?.state.pid ?? 0) > 0 && warmed?.root !== "",
    JSON.stringify({ pid: warmed?.state.pid, root: warmed?.root }),
  );

  const all = await servers();
  await check(
    "a pinned language that does not exist is skipped, not fatal",
    all.every((s) => s.language !== "not-a-language"),
    JSON.stringify(all.map((s) => s.language)),
  );

  // Memory is sampled for the status strip (02-08).
  let sampled: number | null | undefined;
  const measured = await untilAsync(async () => {
    sampled = (await servers()).find((s) => s.language === "node-check")?.state.rssBytes;
    return typeof sampled === "number" && sampled > 0;
  }, 30_000);
  await check("its memory is sampled", measured, `${Math.round((sampled ?? 0) / 1024)} KiB`);
}

/**
 * The same editor path against typescript-language-server, whose server is
 * rooted at the library rather than at one playground (02-07).
 *
 * Skipped with an INFO line where the server isn't installed.
 */
async function lspTypescriptScenario(ctx: SelftestContext, check: Check) {
  const { editor, tabs } = ctx;
  const view = editor.view;

  const status = await invoke<PluginsStatus>("list_plugins");
  const plugin = status.plugins.find((p) => p.id === "typescript");
  await check(
    "the TypeScript plugin declares a language server",
    plugin?.lsp === true,
    JSON.stringify({ lsp: plugin?.lsp, highlighter: plugin?.highlighter }),
  );

  const playground = await invoke<Playground>("create_playground", {
    name: "Lsp Typescript",
    language: "typescript",
    kind: "statements",
  });
  await openPlayground(editor, ctx.status, playground.id);
  view.dispatch({
    changes: {
      from: 0,
      to: view.state.doc.length,
      insert: "const names = [\"ada\", \"grace\"];\nnames.\n",
    },
    selection: { anchor: 'const names = ["ada", "grace"];\nnames.'.length },
  });

  const tab = tabs.activeTab!;
  const connected = await until(() => tab.lsp !== null, 60_000);
  if (!connected) {
    await report("INFO typescript-language-server did not connect; skipping");
    await tabs.close(tab);
    await invoke("delete_playground", { id: playground.id });
    return;
  }

  // A server rooted at the library, not at this playground.
  const server = (await invoke<ServerStatus[]>("server_status")).find(
    (s) => s.language === "typescript",
  );
  const library = tabs.summary(playground.id)?.dir ?? "";
  await check(
    "its server is rooted at the library, above the playground",
    (server?.root.length ?? 0) > 0 && library.startsWith(server?.root ?? "\u0000"),
    JSON.stringify({ root: server?.root, playground: library }),
  );
  await check(
    "and the tsconfig.json it needs was written there",
    (await invoke<string[]>("selftest_list_dir", { dir: server?.root ?? "" })).includes(
      "tsconfig.json",
    ),
    server?.root ?? "",
  );

  let offered: readonly { label: string }[] = [];
  let found = false;
  const started = performance.now();
  for (let attempt = 0; attempt < 30 && !found; attempt++) {
    startCompletion(view);
    found = await until(() => {
      offered = currentCompletions(view.state);
      return offered.length > 0;
    }, 2000);
  }
  // tsserver labels a completion after a dot with the dot included, so compare
  // on the name itself.
  const names = offered.map((c) => c.label.replace(/^./, ""));
  await check(
    "completions on an array come from the server",
    found && names.includes("map") && names.includes("filter"),
    JSON.stringify(names.slice(0, 6)),
  );
  await report(
    `INFO first TypeScript completion took ${((performance.now() - started) / 1000).toFixed(1)} s`,
  );

  // Warm, the number worth comparing with rust-analyzer's.
  const samples: number[] = [];
  for (let n = 0; n < 5; n++) {
    closeCompletion(view);
    await frame();
    const at = performance.now();
    view.dispatch({
      changes: { from: view.state.selection.main.head, insert: "m" },
      selection: { anchor: view.state.selection.main.head + 1 },
      userEvent: "input.type",
    });
    if (await until(() => currentCompletions(view.state).length > 0, 10_000)) {
      samples.push(performance.now() - at);
    }
    view.dispatch({
      changes: { from: view.state.selection.main.head - 1, to: view.state.selection.main.head },
    });
  }
  samples.sort((a, b) => a - b);
  const median = samples[Math.floor(samples.length / 2)] ?? Number.POSITIVE_INFINITY;
  await check(
    "a warm TypeScript server answers a keystroke quickly",
    samples.length >= 3 && median < 1000,
    JSON.stringify(samples.map((ms) => Math.round(ms))),
  );
  await report(
    `INFO warm TypeScript completion: median ${median.toFixed(0)} ms, best ${(samples[0] ?? 0).toFixed(0)} ms`,
  );
  closeCompletion(view);

  // F12 into the TypeScript that ships with the server: a definition in
  // someone else's file, opened read-only (02-09).
  view.dispatch({
    changes: {
      from: 0,
      to: view.state.doc.length,
      insert: "const names: string[] = [];\nnames.map;\n",
    },
    selection: { anchor: "const names: string[] = [];\nnames.m".length },
  });
  runScopeHandlers(view, key("F12"), "editor");
  const jumped = await until(() => tabs.tabs.some((t) => t.peek !== undefined), 60_000);
  const landed = tabs.tabs.find((t) => t.peek)?.peek?.path ?? "";
  await check(
    "F12 opens the TypeScript typings read-only",
    jumped && landed.endsWith(".d.ts"),
    landed,
  );

  await tabs.close(tab);
  try {
    await invoke("delete_playground", { id: playground.id });
  } catch (e) {
    await report(`INFO could not delete the TypeScript playground: ${JSON.stringify(e)}`);
  }
}

/** The language servers in the status bar (02-08). */
async function serversScenario(ctx: SelftestContext, check: Check) {
  const { tabs } = ctx;
  const chips = () => [...document.querySelectorAll<HTMLElement>(".server-chip")];
  const chipFor = (language: string) =>
    chips().find((chip) => (chip.dataset.key ?? "").startsWith(`server|${language}|`));
  const popover = () => document.querySelector<HTMLElement>(".server-popover");

  /** The popover stays open between actions, so only open it if it is shut. */
  const openPopover = async () => {
    if (popover() === null) chipFor("node-check")?.click();
    await frame();
  };
  for (const t of [...tabs.tabs]) await tabs.close(t);

  // A playground with a server gets a chip that says what it is doing.
  const playground = await openTestPlayground(ctx, "Servers Strip");
  const appeared = await until(() => chipFor("node-check") !== undefined, 30_000);
  await check(
    "a language server shows up in the status bar",
    appeared,
    JSON.stringify(chips().map((chip) => chip.dataset.key)),
  );

  // Memory is sampled every ten seconds, so the chip says "ready" before it
  // says how much.
  const ready = await until(
    () =>
      chipFor("node-check")?.dataset.state === "ready" &&
      /\d+ MB/.test(chipFor("node-check")?.querySelector(".server-rss")?.textContent ?? ""),
    60_000,
  );
  const chip = chipFor("node-check")!;
  await check(
    "its chip says ready, with a dot and the memory it is using",
    ready &&
      chip.querySelector(".server-dot") !== null &&
      /\d+ MB/.test(chip.querySelector(".server-rss")?.textContent ?? ""),
    chip.textContent ?? "",
  );

  // Clicking it opens the details worth having when something is wrong.
  chip.click();
  await frame();
  const facts = popover()?.textContent ?? "";
  await check(
    "clicking it shows what the server is doing",
    popover() !== null &&
      facts.includes("Process") &&
      facts.includes("Documents") &&
      facts.includes("Answers") &&
      facts.includes("Program"),
    facts.slice(0, 160),
  );

  // The server's own words, on demand.
  const logButton = [...(popover()?.querySelectorAll<HTMLElement>(".server-action") ?? [])].find(
    (button) => button.textContent === "Log",
  );
  logButton?.click();
  const logged = await until(() => document.querySelector(".server-log-body") !== null, 10_000);
  await check(
    "and what the server itself has said",
    logged,
    (document.querySelector(".server-log-body")?.textContent ?? "").slice(0, 80),
  );

  // Stop and restart, the two things worth doing from here.
  const action = (label: string) =>
    [...(popover()?.querySelectorAll<HTMLElement>(".server-action") ?? [])].find(
      (button) => button.textContent === label,
    );
  action("Stop")?.click();
  const stopped = await until(() => chipFor("node-check")?.dataset.state === "stopped", 30_000);
  await check(
    "Stop stops it, and the chip stays while a tab still wants it",
    stopped && chipFor("node-check") !== undefined,
    chipFor("node-check")?.dataset.state ?? "gone",
  );

  await openPopover();
  action("Restart")?.click();
  const back = await until(() => chipFor("node-check")?.dataset.state === "ready", 60_000);
  await check("Restart brings it back", back, chipFor("node-check")?.dataset.state ?? "gone");

  // With no tab open, a stopped server is not worth a chip.
  await openPopover();
  action("Stop")?.click();
  await until(() => chipFor("node-check")?.dataset.state === "stopped", 30_000);
  await tabs.close(tabs.activeTab!);
  const hidden = await until(() => chipFor("node-check") === undefined, 30_000);
  await check("a stopped server with no tab open is not shown", hidden);

  // Leave a chip and its popover on screen for a screenshot.
  await openTestPlayground(ctx, "Servers Shot");
  await until(() => chipFor("node-check")?.dataset.state === "ready", 30_000);
  await openPopover();
  await report("INFO shot-ready");

  await invoke("delete_playground", { id: playground.id }).catch(() => {});
}

/**
 * Go to definition, and the read-only tabs it opens (02-09).
 *
 * The test plugin's server answers definitions by a rule the source can ask
 * for: a line saying "away" points at the server's own file, "many" gives two
 * places to choose from, and anything else points at the top of the same
 * document.
 */
async function definitionScenario(ctx: SelftestContext, check: Check) {
  const { editor, tabs } = ctx;
  const view = editor.view;
  const peeks = () => tabs.tabs.filter((tab) => tab.peek);
  const source = [
    "const first = 1;",
    "const here = first;",
    "const away = first;",
    "const many = first;",
    "",
  ].join("\n");

  for (const t of [...tabs.tabs]) await tabs.close(t);
  const playground = await openTestPlayground(ctx, "Definition Test");
  view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: source } });
  const tab = tabs.activeTab!;
  const connected = await until(() => tab.lsp !== null, 60_000);
  if (!connected) {
    await report("INFO the test server did not connect; skipping");
    return;
  }

  // A definition in this file moves the cursor, and Alt+Left comes back.
  const at = (line: number, col: number) => view.state.doc.line(line).from + col;
  view.dispatch({ selection: { anchor: at(2, 8) } });
  runScopeHandlers(view, key("F12"), "editor");
  const moved = await until(
    () => view.state.doc.lineAt(view.state.selection.main.head).number === 1,
    30_000,
  );
  await check(
    "F12 goes to a definition in the same file",
    moved,
    `line ${view.state.doc.lineAt(view.state.selection.main.head).number}`,
  );

  ctx.handleGlobalKey(key("ArrowLeft", { alt: true }));
  const back = await until(
    () => view.state.doc.lineAt(view.state.selection.main.head).number === 2,
    10_000,
  );
  await check(
    "Alt+Left comes back to where the jump started",
    back,
    `line ${view.state.doc.lineAt(view.state.selection.main.head).number}`,
  );

  // A definition in someone else's file opens read-only.
  view.dispatch({ selection: { anchor: at(3, 8) } });
  runScopeHandlers(view, key("F12"), "editor");
  const opened = await until(() => peeks().length === 1, 30_000);
  const peek = peeks()[0];
  await check(
    "a definition in another file opens a read-only tab",
    opened && (peek?.peek?.path ?? "").endsWith("server.js"),
    peek?.peek?.path ?? "none",
  );
  await check(
    "which shows the file, locked, and is not a playground",
    view.state.readOnly &&
      (peek?.element.querySelector(".tab-lang")?.textContent ?? "") === "🔒" &&
      view.state.doc.toString().includes("language server for the self-tests"),
    view.state.doc.line(1).text.slice(0, 60),
  );
  await check(
    "and the run command has nothing to run there",
    tabs.activePlayground() === null,
    String(tabs.activePlayground()),
  );

  // Back to the playground, then a definition with two answers.
  ctx.handleGlobalKey(key("ArrowLeft", { alt: true }));
  await until(() => tabs.activeId === playground.id, 10_000);
  view.dispatch({ selection: { anchor: at(4, 8) } });
  runScopeHandlers(view, key("F12"), "editor");
  const asked = await until(() => document.querySelector(".picker") !== null, 30_000);
  const options = [...document.querySelectorAll(".picker-item")].map((i) => i.textContent ?? "");
  await check("two definitions are offered as a list", asked && options.length === 2, JSON.stringify(options));
  await choosePicked("server.js");
  const chose = await until(() => tabs.activeTab?.peek !== undefined, 30_000);
  await check("choosing one opens it", chose, tabs.activeTab?.peek?.title ?? "none");

  // Ten is enough of them.
  const dir = `${tabs.summary(playground.id)?.dir ?? ""}/peeks`;
  for (let n = 0; n < MAX_PEEKS + 2; n++) {
    const file = `${dir}/file-${n}.txt`;
    await invoke("selftest_write_file", { path: file, text: `file number ${n}\n` });
    await tabs.openReadOnly(file);
  }
  await check(
    "at most ten read-only tabs are kept",
    peeks().length === MAX_PEEKS,
    `${peeks().length}`,
  );
  await check(
    "and the oldest are the ones that went",
    peeks().every((p) => !(p.peek?.path ?? "").endsWith("server.js")),
    JSON.stringify(peeks().map((p) => p.peek?.title)),
  );

  // A file too big to show says so instead of freezing the editor.
  const huge = `${dir}/huge.txt`;
  await invoke("selftest_write_file", { path: huge, text: "x".repeat(5 * 1024 * 1024) });
  await tabs.openReadOnly(huge);
  await check(
    "a file too large to open says so",
    view.state.doc.toString().includes("more than TinkerPad will open"),
    view.state.doc.line(3).text.slice(0, 60),
  );

  for (const t of [...tabs.tabs]) await tabs.close(t);
  await invoke("delete_playground", { id: playground.id }).catch(() => {});
}

/**
 * Plugin hooks (work item 03-07): the four moments in a playground's life
 * that its plugin's helper is told about.
 *
 * The test plugin's helper answers every one of them with a line appended to
 * `hooks.log` in the playground folder, so what arrived and when is readable
 * from the file the core wrote on its behalf.
 */
async function hooksScenario(ctx: SelftestContext, check: Check) {
  const { tabs } = ctx;
  for (const t of [...tabs.tabs]) await tabs.close(t);

  const hooksLog = async (p: Playground): Promise<string[]> => {
    const peek = await invoke<Peek>("open_readonly", { path: `${p.dir}/hooks.log` }).catch(
      () => null,
    );
    return peek ? peek.text.split("\n").filter((line) => line !== "") : [];
  };
  // Hooks other than `playground_created` are answered in the background, so
  // wait for the line rather than assuming it is already there.
  const untilLines = async (p: Playground, n: number): Promise<string[]> => {
    await untilAsync(async () => (await hooksLog(p)).length >= n, 15_000);
    return hooksLog(p);
  };
  // `open=[...]` from a line, as a sorted list: the core keeps the open set in
  // id order, which is not an order this test should depend on.
  const open = (line: string): string[] => {
    const inside = /open=\[(.*)\]/.exec(line)?.[1] ?? "";
    return inside === "" ? [] : inside.split(",").sort();
  };

  // playground_created, before the create command has even returned.
  const first = await invoke<Playground>("create_playground", {
    name: "Hooks First",
    language: "node-check",
    kind: "script",
  });
  let lines = await hooksLog(first);
  await check(
    "playground_created fires as the playground is made, and its file lands in the folder",
    lines[0] === "created Hooks First script",
    JSON.stringify(lines),
  );

  // playground_opened, with the open list.
  await openPlayground(ctx.editor, ctx.status, first.id);
  lines = await untilLines(first, 2);
  await check(
    "playground_opened fires when a tab opens it, and says which are open",
    lines[1] === "opened Hooks First open=[Hooks First]",
    JSON.stringify(lines),
  );

  // A second playground of the same language: its hook sees both.
  const second = await invoke<Playground>("create_playground", {
    name: "Hooks Second",
    language: "node-check",
    kind: "script",
  });
  await openPlayground(ctx.editor, ctx.status, second.id);
  lines = await untilLines(second, 2);
  await check(
    "the open list has every open playground of that language, this one included",
    lines[0] === "created Hooks Second script" &&
      open(lines[1] ?? "").join(",") === "Hooks First,Hooks Second",
    JSON.stringify(lines),
  );

  // What a hook said about the server reaches it, merged into the settings
  // `[lsp.register]` would have sent (02-07).
  const configured = async (): Promise<string[]> => {
    // The pool is keyed by language and root, and this plugin's server is
    // rooted at the library, so ask which root it has rather than guess.
    const server = (await invoke<ServerStatus[]>("server_status")).find(
      (s) => s.language === "node-check",
    );
    if (!server) return [];
    const log = await invoke<string[]>("server_log", {
      language: "node-check",
      root: server.root,
    });
    return log.filter((line) => line.includes("config "));
  };
  const arrived = await untilAsync(
    async () => (await configured()).some((line) => line.includes('"opened":"Hooks Second"')),
    20_000,
  );
  let configs = await configured();
  await check(
    "a hook's lsp_config is pushed to the language server",
    arrived && configs[configs.length - 1]?.includes('{"open":2,"opened":"Hooks Second"}') === true,
    JSON.stringify(configs.slice(-2)),
  );

  // Reopening the same playground is not a second opening.
  const before = (await hooksLog(first)).length;
  await openPlayground(ctx.editor, ctx.status, first.id);
  await frame();
  await check(
    "a playground that is already open does not fire the hook again",
    (await hooksLog(first)).length === before,
    JSON.stringify(await hooksLog(first)),
  );

  // playground_closed, with the ones still open.
  await tabs.close(tabs.tabs.find((t) => t.id === second.id)!);
  lines = await untilLines(second, 3);
  await check(
    "playground_closed fires when the tab closes, and the closed one is no longer open",
    lines[2] === "closed Hooks Second open=[Hooks First]",
    JSON.stringify(lines),
  );

  // Closing it takes its settings back off the server.
  const gone = await untilAsync(
    async () => (await configured()).some((line) => line.includes('"opened":"Hooks First"')),
    20_000,
  );
  configs = await configured();
  await check(
    "closing a playground drops what its hook told the server",
    gone && configs[configs.length - 1]?.includes('{"open":1,"opened":"Hooks First"}') === true,
    JSON.stringify(configs.slice(-2)),
  );

  // add_dependency: the hook does the work, and `[deps]` is not consulted.
  const tab = tabs.tabs.find((t) => t.id === first.id)!;
  tabs.activate(tab);
  const text = () => ctx.editor.view.state.doc.toString();
  const unchanged = text();
  let done = runCommand("Add package");
  await check("the add-package prompt appears", await fillPrompt(["left-pad", "1.3.0"]));
  await done;
  lines = await untilLines(first, 3);
  await check(
    "add_dependency goes to the hook, which gets the name and version",
    lines[2] === "added left-pad@1.3.0",
    JSON.stringify(lines),
  );
  await check(
    "the hook takes precedence over [deps]: no directive is inserted",
    text() === unchanged,
    JSON.stringify(text().split("\n").slice(0, 2)),
  );

  // A hook that asks to write outside the playground folder is refused, and
  // the rest of what it sent still lands.
  done = runCommand("Add package");
  await fillPrompt(["escape", ""]);
  await done;
  lines = await untilLines(first, 4);
  const escaped = await invoke<Peek>("open_readonly", {
    path: `${first.dir}/../escaped.txt`,
  }).catch(() => null);
  await check(
    "a hook's file outside the playground folder is refused, and its other files still land",
    escaped === null && lines[3] === "added escape@latest",
    JSON.stringify({ escaped: escaped?.path ?? null, lines }),
  );

  for (const t of [...tabs.tabs]) await tabs.close(t);
  await invoke("delete_playground", { id: first.id }).catch(() => {});
  await invoke("delete_playground", { id: second.id }).catch(() => {});
}

/**
 * Run timing and history (work item 03-08): every run recorded beside its
 * playground, the last few still showable, and the status bar saying how this
 * one compares with the last.
 */
async function historyScenario(ctx: SelftestContext, check: Check) {
  const { runs, tabs } = ctx;
  const outText = () => ctx.output.element.querySelector(".output-body")?.textContent ?? "";
  const status = () => document.querySelector(".status-run")?.textContent ?? "";
  const popover = () => document.querySelector(".history-popover");
  const rows = () => [...document.querySelectorAll<HTMLElement>(".history-row")];

  for (const t of [...tabs.tabs]) await tabs.close(t);
  const main = await openTestPlayground(ctx, "History Test");
  // The helper's first use builds it, and that belongs to the first run; get
  // it out of the way so the times being compared are the program's.
  await invoke("start_helper", { plugin: "node-check" }).catch(() => {});

  const history = async (): Promise<RunRecord[]> =>
    invoke<RunRecord[]>("run_history", { playground: main.id });
  await check("a playground that has never run has no history", (await history()).length === 0);

  // A slow run, then a fast one, so the comparison has something to say.
  let end = runs.waitForEnd();
  runCode(ctx, 'console.log("one"); setTimeout(() => {}, 400)\n');
  await withTimeout(end, 30_000, "the first run");
  end = runs.waitForEnd();
  runCode(ctx, 'console.log("two")\n');
  await withTimeout(end, 30_000, "the second run");

  let records = await history();
  const [second, first] = records;
  await check(
    "each run is recorded, newest first, with what it did",
    records.length === 2 &&
      first !== undefined &&
      second !== undefined &&
      first.outcome === "exited" &&
      first.code === 0 &&
      first.durationMs >= 400 &&
      second.durationMs < first.durationMs &&
      second.startedMs >= first.startedMs,
    JSON.stringify(records),
  );
  await check(
    "with the runner that ran it and where its time went",
    second?.runner === "helper" &&
      second?.reportedMs !== undefined &&
      second.reportedMs <= second.durationMs &&
      second?.firstOutputMs !== undefined,
    JSON.stringify(second),
  );

  await check(
    "the status bar compares this run with the one before it",
    document.querySelector(".status-delta")?.textContent?.startsWith("▼") === true,
    JSON.stringify({ status: status(), delta: document.querySelector(".status-delta")?.textContent }),
  );

  // A run that ends badly is remembered by what went wrong.
  end = runs.waitForEnd();
  runCode(ctx, 'console.error("it went wrong"); process.exit(3)\n');
  await withTimeout(end, 30_000, "the failing run");
  records = await history();
  await check(
    "a run that ends badly records its code and the first thing it said",
    records[0]?.code === 3 && records[0]?.error === "it went wrong",
    JSON.stringify(records[0]),
  );

  // The list itself.
  const runButton = document.querySelector<HTMLElement>(".status-run");
  runButton?.click();
  await until(() => popover() !== null, 5000);
  await check(
    "clicking the run indicator lists the recent runs, newest first",
    rows().length === 3 && rows()[0]?.textContent?.includes("Exit 3") === true,
    JSON.stringify(rows().map((r) => r.textContent)),
  );

  // Selecting one shows its output again.
  const firstRun = rows()[2];
  firstRun?.click();
  await until(() => popover() === null, 3000);
  await ctx.output.flushed();
  await check(
    "selecting a run shows the output it had",
    outText() === "one\n",
    JSON.stringify({ out: outText() }),
  );

  // Only the last five runs keep their output; older ones are their numbers.
  for (let n = 0; n < 4; n++) {
    end = runs.waitForEnd();
    runCode(ctx, `console.log(${n})\n`);
    await withTimeout(end, 30_000, `run ${n}`);
  }
  runButton?.click();
  await until(() => popover() !== null, 5000);
  const kept = rows().filter((r) => r.tagName === "BUTTON").length;
  const gone = rows().filter((r) => r.classList.contains("history-gone")).length;
  await check(
    "the last five runs keep their output; the ones before are metadata only",
    rows().length === 7 && kept === 5 && gone === 2,
    JSON.stringify({ total: rows().length, kept, gone }),
  );
  runButton?.click();
  await until(() => popover() === null, 3000);

  // The file is the playground's own business: it is not a playground, and
  // writing it is not a change to the source.
  const listing = await invoke<Listing>("list_playgrounds");
  await check(
    "runs.jsonl is not listed as a playground, and does not disturb the tab",
    listing.playgrounds.filter((p) => p.id === main.id).length === 1 &&
      listing.broken.length === 0 &&
      tabs.activeTab?.dirty === false &&
      document.querySelector<HTMLElement>(".conflict-bar")?.hidden === true,
    JSON.stringify({
      mine: listing.playgrounds.filter((p) => p.id === main.id).length,
      broken: listing.broken.length,
      dirty: tabs.activeTab?.dirty,
      conflict: !document.querySelector<HTMLElement>(".conflict-bar")?.hidden,
    }),
  );

  // The same run without a helper: what ran it is recorded too, and the
  // process's own time sits inside the time the user waited.
  await invoke("force_per_run", { forced: true });
  end = runs.waitForEnd();
  runCode(ctx, 'console.log("cold")\n');
  await withTimeout(end, 60_000, "the per-run run");
  await invoke("force_per_run", { forced: false });
  const cold = (await history())[0];
  await check(
    "a run with no helper is recorded as per-run, with its own time and first byte",
    cold?.runner === "perRun" &&
      cold?.firstOutputMs !== undefined &&
      cold?.reportedMs !== undefined &&
      cold.reportedMs <= cold.durationMs,
    JSON.stringify(cold),
  );

  for (const t of [...tabs.tabs]) await tabs.close(t);
  await invoke("delete_playground", { id: main.id }).catch(() => {});
}

/**
 * The library panel (work item 04-01): finding a playground by what is in it,
 * and doing things to it from there.
 */
async function libraryPanelScenario(ctx: SelftestContext, check: Check) {
  const { tabs } = ctx;
  for (const t of [...tabs.tabs]) await tabs.close(t);

  const panel = () => document.querySelector(".library");
  const rows = () => [...document.querySelectorAll<HTMLElement>(".library-row")];
  const names = () => rows().map((r) => r.querySelector(".library-name")?.textContent ?? "");
  const sections = () =>
    [...document.querySelectorAll(".library-section")].map((s) => s.textContent ?? "");
  const search = () => document.querySelector<HTMLInputElement>(".library-search");
  const type = async (text: string) => {
    const box = search();
    if (!box) return;
    box.value = text;
    box.dispatchEvent(new Event("input"));
    await untilAsync(async () => true, 100);
    await frame();
    await frame();
  };

  // Three playgrounds with something to tell them apart by.
  const made: Playground[] = [];
  for (const [name, source] of [
    ["Panel Alpha", 'console.log("lighthouse");\n'],
    ["Panel Beta", 'console.log("beacon");\n'],
    ["Panel Gamma", 'console.log("beacon and lighthouse");\n'],
  ] as [string, string][]) {
    const p = await invoke<Playground>("create_playground", {
      name,
      language: "node-check",
      kind: "script",
    });
    await invoke("write_source", { id: p.id, text: source });
    made.push(p);
  }
  await invoke("set_tags", { id: made[0]!.id, tags: ["panel-test", "alpha"] });
  await invoke("set_pinned", { id: made[1]!.id, pinned: true });

  // Mod+B opens it.
  pressKey("b", { mod: true });
  await until(() => panel() !== null && !!document.querySelector(".library-row"), 5000);
  await check("Mod+B opens the library panel", panel() !== null && rows().length > 0);

  // A search over the source, not just the name.
  await type("lighthouse");
  await until(() => names().filter((n) => n.startsWith("Panel")).length === 2, 5000);
  await check(
    "searching finds playgrounds by what is in their code, with the line that matched",
    names().includes("Panel Alpha") &&
      names().includes("Panel Gamma") &&
      !names().includes("Panel Beta") &&
      (rows()[0]?.querySelector(".library-detail")?.textContent ?? "").includes("lighthouse"),
    JSON.stringify({ names: names(), detail: rows()[0]?.querySelector(".library-detail")?.textContent }),
  );

  // A name beats a body, and a tag finds it too.
  await type("panel alpha");
  await until(() => names()[0] === "Panel Alpha", 5000);
  await check("a name ranks above a mention in the code", names()[0] === "Panel Alpha", JSON.stringify(names()));
  await type("panel-test");
  await until(() => names().length === 1, 5000);
  await check("a tag finds its playground", names()[0] === "Panel Alpha", JSON.stringify(names()));

  // Pinned comes first, in its own section.
  await type("Panel ");
  await until(() => names().length === 3, 5000);
  await check(
    "pinned playgrounds are first, under their own heading",
    sections()[0] === "Pinned" && names()[0] === "Panel Beta",
    JSON.stringify({ sections: sections(), names: names() }),
  );

  // Enter opens the selected one; the arrows move the selection.
  search()?.focus();
  pressKey("ArrowDown");
  const chosen = rows().find((r) => r.classList.contains("selected"))?.dataset.id;
  pressKey("Enter");
  await until(() => tabs.activeId === chosen, 10_000);
  await check(
    "arrows move through the list and Enter opens what is selected",
    tabs.activeId === chosen && chosen !== undefined,
    JSON.stringify({ chosen, active: tabs.activeId }),
  );

  // What is open is marked, and it turns up under Recent.
  await type("Panel ");
  await until(() => document.querySelector(".library-row.open") !== null, 5000);
  await check(
    "the playground in the editor is marked, and lately-opened ones have a section",
    document.querySelector(".library-row.open")?.getAttribute("data-id") === tabs.activeId &&
      sections().includes("Recent"),
    JSON.stringify({ sections: sections(), active: tabs.activeId }),
  );

  // Right-clicking a row offers what can be done to it.
  rows()[0]?.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
  await until(() => document.querySelector(".picker") !== null, 5000);
  const offered = [...document.querySelectorAll(".picker-item")].map((i) => i.textContent ?? "");
  pressKey("Escape");
  await until(() => document.querySelector(".picker") === null, 3000);
  await check(
    "right-clicking offers open, rename, tags, pin, duplicate, reveal and delete",
    // A pinned one says Unpin, so match without minding the case.
    ["open", "rename", "tags", "pin", "duplicate", "file explorer", "recycle bin"].every((label) =>
      offered.some((item) => item.toLowerCase().includes(label)),
    ),
    JSON.stringify(offered),
  );

  // Duplicating copies the code into a new playground and opens it.
  const before = (await invoke<SearchHit[]>("search_playgrounds", { query: "Panel " })).length;
  const copy = await invoke<Playground>("duplicate_playground", { id: made[0]!.id });
  await until(() => tabs.activeId !== null, 5000);
  const copied = await invoke<string>("read_source", { id: copy.id });
  const after = (await invoke<SearchHit[]>("search_playgrounds", { query: "Panel " })).length;
  await check(
    "duplicating makes a copy of the code under a new name",
    copy.name === "Panel Alpha copy" &&
      copy.id !== made[0]!.id &&
      copied.includes("lighthouse") &&
      after === before + 1,
    JSON.stringify({ name: copy.name, before, after }),
  );

  // Tags and pinning from the panel's commands reach the file and come back.
  await invoke("set_tags", { id: copy.id, tags: ["alpha", "alpha", " kept "] });
  const tagged = (await invoke<SearchHit[]>("search_playgrounds", { query: "kept" }))[0];
  await check(
    "tags are tidied and searchable at once",
    tagged?.id === copy.id && JSON.stringify(tagged?.tags) === JSON.stringify(["alpha", "kept"]),
    JSON.stringify(tagged?.tags),
  );

  // Deleting from the panel closes its tab too.
  await tabs.open(copy.id);
  await until(() => tabs.activeId === copy.id, 5000);
  await invoke("delete_playground", { id: copy.id });
  tabs.closed(copy.id);
  await until(() => tabs.find(copy.id) === undefined, 5000);
  await check(
    "a deleted playground takes its tab with it",
    tabs.find(copy.id) === undefined &&
      (await invoke<SearchHit[]>("search_playgrounds", { query: "Panel Alpha copy" })).length === 0,
    JSON.stringify({ open: tabs.tabs.length }),
  );

  // Escape clears the search, then closes the panel.
  await type("Panel ");
  search()?.focus();
  pressKey("Escape");
  await until(() => (search()?.value ?? "x") === "", 3000);
  await check("Escape clears the search first", (search()?.value ?? "x") === "");
  pressKey("Escape");
  await until(() => document.querySelector(".sidebar")?.hasAttribute("hidden") === true, 3000);
  await check(
    "and closes the panel next",
    document.querySelector(".sidebar")?.hasAttribute("hidden") === true,
  );

  for (const t of [...tabs.tabs]) await tabs.close(t);
  for (const p of made) await invoke("delete_playground", { id: p.id }).catch(() => {});
}

/**
 * Settings (work item 04-02): one file, a form over it, and one path from a
 * change to the app whichever way the change was made.
 */
async function settingsScenario(ctx: SelftestContext, check: Check) {
  const { tabs } = ctx;
  for (const t of [...tabs.tabs]) await tabs.close(t);

  const dialog = () => document.querySelector(".settings");
  const rows = () => [...document.querySelectorAll<HTMLElement>(".settings-row")];
  const labelled = (text: string) =>
    rows().find((row) => row.querySelector(".settings-label")?.textContent === text);
  const fontSize = () =>
    getComputedStyle(document.documentElement).getPropertyValue("--editor-font-size").trim();
  const get = () => invoke<Settings>("get_settings");
  const toastText = () =>
    [...document.querySelectorAll(".toast")].map((t) => t.textContent ?? "").join(" ");
  const clearToasts = () => document.querySelectorAll(".toast").forEach((t) => t.remove());
  const file = await invoke<string>("settings_file");
  const before = await get();

  // Mod+, opens it, and the per-plugin rows come from the registry.
  pressKey(",", { mod: true });
  await until(() => dialog() !== null, 5000);
  const plugins = await invoke<PluginsStatus>("list_plugins");
  const withServers = plugins.plugins.filter((p) => p.lsp).map((p) => p.name);
  const shown = rows().map((row) => row.querySelector(".settings-label")?.textContent ?? "");
  await check(
    "Mod+, opens the settings form, with a row per plugin that has a server",
    dialog() !== null && withServers.every((name) => shown.includes(name)),
    JSON.stringify({ withServers, shown }),
  );

  // Changing a setting and saving applies it and writes the file.
  const size = labelled("Font size")?.querySelector<HTMLInputElement>("input");
  if (size) {
    size.value = "19";
    size.dispatchEvent(new Event("input"));
  }
  [...document.querySelectorAll<HTMLButtonElement>(".settings-button")]
    .find((b) => b.textContent === "Save")
    ?.click();
  await until(() => dialog() === null, 5000);
  await untilAsync(async () => (await get()).editor.font_size === 19, 5000);
  await check(
    "saving writes the setting and the editor takes it",
    (await get()).editor.font_size === 19 && fontSize() === "19px",
    JSON.stringify({ saved: (await get()).editor.font_size, css: fontSize() }),
  );

  // The file keeps its comments, and a default stays out of it.
  const text = (await invoke<Peek>("open_readonly", { path: file })).text;
  await check(
    "the file keeps its comments, and only what changed is written",
    text.includes("# TinkerPad settings.") &&
      text.includes("font_size = 19") &&
      !text.includes("\nclear_output"),
    JSON.stringify({ head: text.slice(0, 60) }),
  );

  // An edit made outside the app arrives within a second.
  await invoke("selftest_write_file", {
    path: file,
    text: "[editor]\nfont_size = 15\ntab_size = 4\n\n[run]\nclear_output = false\n",
  });
  const applied = await untilAsync(async () => (await get()).editor.font_size === 15, 5000);
  await check(
    "an edit made in another editor applies on its own",
    applied && fontSize() === "15px",
    JSON.stringify({ size: (await get()).editor.font_size, css: fontSize() }),
  );

  // A tab size of 4 reaches the editor without it being rebuilt.
  const main = await openTestPlayground(ctx, "Settings Test");
  await check(
    "the editor's tab size follows the setting",
    ctx.editor.view.state.tabSize === 4,
    JSON.stringify({ tabSize: ctx.editor.view.state.tabSize }),
  );

  // clear_output = false: the second run's output joins the first.
  const outText = () => ctx.output.element.querySelector(".output-body")?.textContent ?? "";
  await invoke("start_helper", { plugin: "node-check" }).catch(() => {});
  let end = ctx.runs.waitForEnd();
  runCode(ctx, 'console.log("first")\n');
  await withTimeout(end, 30_000, "the first run");
  end = ctx.runs.waitForEnd();
  runCode(ctx, 'console.log("second")\n');
  await withTimeout(end, 30_000, "the second run");
  await ctx.output.flushed();
  await check(
    "with clear_output off, a run adds to what is already there",
    outText().includes("first") && outText().includes("second"),
    JSON.stringify({ out: outText() }),
  );

  // Nonsense in the file keeps the last settings that worked, and says where.
  clearToasts();
  await invoke("selftest_write_file", {
    path: file,
    text: "[editor]\nfont_size = 15\ntab_size = \n",
  });
  const complained = await until(() => toastText().includes("line"), 5000);
  await check(
    "a broken file keeps the last good settings and says which line",
    complained && (await get()).editor.font_size === 15 && toastText().includes("line 3"),
    JSON.stringify({ toast: toastText().slice(0, 160) }),
  );

  // Reset puts everything back, including the comments.
  await invoke("reset_settings");
  await untilAsync(async () => (await get()).editor.font_size === 13, 5000);
  await check(
    "Reset to defaults puts the file and the window back",
    (await get()).editor.font_size === 13 &&
      (await get()).run.clear_output &&
      fontSize() === "13px",
    JSON.stringify(await get()),
  );

  // Moving the library: only when there is nothing to put back wrongly.
  if (before.library.root === null) {
    const temp = `${(await invoke<Listing>("list_playgrounds")).root}-settings-test`;
    const moved = await invoke<string>("change_library_root", { root: temp });
    const there = await invoke<Listing>("list_playgrounds");
    await invoke("change_library_root", { root: null });
    const back = await invoke<Listing>("list_playgrounds");
    await check(
      "the library can be pointed at another folder and back",
      moved.replace(/\\/g, "/") === temp.replace(/\\/g, "/") &&
        there.playgrounds.length === 0 &&
        back.playgrounds.length > 0,
      JSON.stringify({ moved, there: there.playgrounds.length, back: back.playgrounds.length }),
    );
  } else {
    await report("INFO library.root is set, so the move check was skipped");
  }

  for (const t of [...tabs.tabs]) await tabs.close(t);
  await invoke("delete_playground", { id: main.id }).catch(() => {});
}

/**
 * The command registry, the palette, and `keybindings.toml` (work item 04-03).
 */
async function keysScenario(ctx: SelftestContext, check: Check) {
  const { tabs } = ctx;
  const items = () => [...document.querySelectorAll<HTMLElement>(".picker-item")];
  const shownIn = (text: string) => items().some((item) => (item.textContent ?? "").includes(text));
  const file = await invoke<string>("keybindings_file");
  const toastText = () =>
    [...document.querySelectorAll(".toast")].map((t) => t.textContent ?? "").join(" | ");
  const clearToasts = () => document.querySelectorAll(".toast").forEach((t) => t.remove());

  for (const t of [...tabs.tabs]) await tabs.close(t);
  const main = await openTestPlayground(ctx, "Keys Test");
  await invoke("start_helper", { plugin: "node-check" }).catch(() => {});

  // Start from the defaults, whatever an earlier run left in the file.
  await invoke("selftest_write_file", { path: file, text: "" });
  await untilAsync(
    async () => Object.keys(await invoke<Record<string, string>>("get_keybindings")).length === 0,
    5000,
  );
  await frame();

  // The palette lists commands with their category and their keys.
  pressKey("p", { mod: true, shift: true });
  await until(() => document.querySelector(".picker") !== null, 5000);
  const input = document.querySelector<HTMLInputElement>(".picker-input");
  if (input) {
    input.value = "run";
    input.dispatchEvent(new Event("input"));
  }
  await frame();
  await check(
    "the palette lists commands with their category and shortcut",
    shownIn("Run") && shownIn("Ctrl+Enter"),
    JSON.stringify(items().map((i) => i.textContent)),
  );
  pressKey("Escape");
  await until(() => document.querySelector(".picker") === null, 3000);

  // What was used last comes first next time.
  await runCommand("Clear output");
  pressKey("p", { mod: true, shift: true });
  await until(() => document.querySelector(".picker") !== null, 5000);
  await check(
    "the command used last is offered first",
    (items()[0]?.textContent ?? "").includes("Clear output"),
    JSON.stringify(items()[0]?.textContent),
  );
  pressKey("Escape");
  await until(() => document.querySelector(".picker") === null, 3000);

  // A default binding runs its command.
  ctx.editor.focus();
  const code = 'console.log("bound")\n';
  ctx.editor.view.dispatch({
    changes: { from: 0, to: ctx.editor.view.state.doc.length, insert: code },
  });
  let end = ctx.runs.waitForEnd();
  pressKey("Enter", { mod: true }, ctx.editor.view.contentDOM);
  let outcome = await withTimeout(end, 30_000, "Mod+Enter");
  await check("the default binding runs its command", outcome.kind === "exited", JSON.stringify(outcome));

  // keybindings.toml moves it, and the move arrives on its own.
  await invoke("selftest_write_file", {
    path: file,
    text: '"run.start" = "mod+shift+r"' + '\n',
  });
  const moved = await untilAsync(
    async () => (await invoke<Record<string, string>>("get_keybindings"))["run.start"] === "mod+shift+r",
    5000,
  );
  await frame();

  // The editor's own keymap was rebuilt: its old key does nothing now.
  ctx.editor.focus();
  pressKey("Enter", { mod: true }, ctx.editor.view.contentDOM);
  const stillIdle = !(await until(() => ctx.runs.running, 1500));
  await check(
    "a rebound command loses its old key, in the editor too",
    moved && stillIdle,
    JSON.stringify({ moved, running: ctx.runs.running }),
  );

  // And the new one runs it.
  end = ctx.runs.waitForEnd();
  pressKey("R", { mod: true, shift: true }, document.body);
  outcome = await withTimeout(end, 30_000, "the moved binding");
  await check(
    "the binding from keybindings.toml runs the command",
    outcome.kind === "exited",
    JSON.stringify(outcome),
  );

  // And the palette shows the new keys.
  pressKey("p", { mod: true, shift: true });
  await until(() => document.querySelector(".picker") !== null, 5000);
  const runRow = items().find((item) => (item.textContent ?? "").startsWith("Run"));
  await check(
    "the palette shows the keys the user chose",
    (runRow?.textContent ?? "").includes("Ctrl+Shift+R"),
    JSON.stringify(runRow?.textContent),
  );
  pressKey("Escape");
  await until(() => document.querySelector(".picker") === null, 3000);

  // Mod+Enter with a completion showing accepts it rather than running
  // (the item's gotcha). Put the default back first, so it is Mod+Enter.
  await invoke("selftest_write_file", { path: file, text: "" });
  await untilAsync(
    async () => Object.keys(await invoke<Record<string, string>>("get_keybindings")).length === 0,
    5000,
  );
  await frame();
  const view = ctx.editor.view;
  // The test server offers `<file>Value`, so type a prefix it matches.
  view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: "main" } });
  view.dispatch({ selection: { anchor: view.state.doc.length } });
  ctx.editor.focus();
  // The tab's language server attaches in the background, so ask until it
  // answers, as the `lsp` scenario does.
  let offered = false;
  for (let attempt = 0; attempt < 20 && !offered; attempt++) {
    startCompletion(view);
    offered = await until(() => currentCompletions(view.state).length > 0, 1500);
  }
  const wanted = currentCompletions(view.state)[0]?.label ?? "";
  // CodeMirror ignores an accept within `interactionDelay` (75 ms) of the
  // popup opening, so a stray keystroke can't accept something you never saw.
  await new Promise((resolve) => setTimeout(resolve, 200));
  pressKey("Enter", { mod: true }, view.contentDOM);
  await frame();
  const accepted = await until(() => view.state.doc.toString() === wanted, 3000);
  await check(
    "Mod+Enter accepts a completion instead of running",
    offered && accepted && !ctx.runs.running,
    JSON.stringify({ offered, wanted, text: view.state.doc.toString(), running: ctx.runs.running }),
  );
  closeCompletion(view);

  // Two commands on one shortcut is worth saying out loud.
  clearToasts();
  await invoke("selftest_write_file", {
    path: file,
    text: '"run.start" = "mod+b"' + '\n',
  });
  const warned = await until(() => toastText().includes("share a shortcut"), 5000);
  await check(
    "two commands on one shortcut are reported",
    warned && toastText().includes("Ctrl+B"),
    JSON.stringify({ toast: toastText().slice(0, 160) }),
  );

  // Put the file back, and check the default returns.
  await invoke("selftest_write_file", { path: file, text: "" });
  await untilAsync(
    async () => Object.keys(await invoke<Record<string, string>>("get_keybindings")).length === 0,
    5000,
  );
  await frame();
  end = ctx.runs.waitForEnd();
  pressKey("Enter", { mod: true }, ctx.editor.view.contentDOM);
  outcome = await withTimeout(end, 30_000, "the default again");
  await check(
    "emptying the file gives the defaults back",
    outcome.kind === "exited",
    JSON.stringify(outcome),
  );

  for (const t of [...tabs.tabs]) await tabs.close(t);
  await invoke("delete_playground", { id: main.id }).catch(() => {});
}

/**
 * Themes (work item 04-04): two palettes, the system followed when asked, and
 * enough contrast in both to read.
 *
 * The contrast is measured rather than eyeballed: every pair below is computed
 * from the variables actually in force, by the WCAG formula.
 */
async function themesScenario(ctx: SelftestContext, check: Check) {
  const { tabs } = ctx;
  const root = document.documentElement;
  const value = (name: string) => getComputedStyle(root).getPropertyValue(name).trim();

  /** A CSS colour as RGB, through the browser rather than a parser of our own. */
  const rgb = (colour: string): [number, number, number] => {
    const probe = document.createElement("span");
    probe.style.color = colour;
    document.body.append(probe);
    const computed = getComputedStyle(probe).color;
    probe.remove();
    const parts = computed.match(/[0-9.]+/g)?.map(Number) ?? [0, 0, 0];
    return [parts[0] ?? 0, parts[1] ?? 0, parts[2] ?? 0];
  };
  const luminance = (colour: string): number => {
    const channel = (c: number) => {
      const v = c / 255;
      return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
    };
    const [r, g, bch] = rgb(colour);
    return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(bch);
  };
  /** WCAG contrast ratio between two variables. */
  const ratio = (a: string, bg: string): number => {
    const first = luminance(value(a));
    const second = luminance(value(bg));
    const [light, dark] = first > second ? [first, second] : [second, first];
    return (light + 0.05) / (dark + 0.05);
  };

  const settings = await invoke<Settings>("get_settings");
  const wasTheme = settings.theme;
  const setTheme = async (theme: string) => {
    await invoke("save_settings", { settings: { ...settings, theme } });
    await untilAsync(async () => root.dataset.theme === theme, 5000);
    await frame();
  };

  for (const t of [...tabs.tabs]) await tabs.close(t);
  const main = await openTestPlayground(ctx, "Themes Test");

  // Both themes, measured the same way.
  const text: [string, string][] = [
    ["--fg", "--bg"],
    ["--fg", "--panel"],
    ["--muted", "--bg"],
    ["--muted", "--panel"],
    ["--error", "--bg"],
    ["--warning", "--bg"],
    ["--success", "--bg"],
    ["--accent", "--bg"],
    ["--link", "--panel"],
    ["--code-keyword", "--bg"],
    ["--code-string", "--bg"],
    ["--code-number", "--bg"],
    ["--code-comment", "--bg"],
    ["--code-function", "--bg"],
    ["--code-type", "--bg"],
    ["--code-constant", "--bg"],
    ["--code-operator", "--bg"],
    ["--j-string", "--dump-bg"],
    ["--j-number", "--dump-bg"],
    ["--j-bool", "--dump-bg"],
    ["--j-key", "--dump-bg"],
    ["--j-null", "--dump-bg"],
  ];
  // What the eye has to act on — the accent marks focus, selection and the
  // active tab — is held to WCAG's 3:1 for non-text. Borders are separators
  // rather than affordances, so they are only required to be visible.
  const affordances: [string, string][] = [
    ["--accent", "--bg"],
    ["--accent", "--panel"],
    ["--accent", "--panel-2"],
  ];
  const separators: [string, string][] = [
    ["--border", "--bg"],
    ["--border", "--panel"],
    ["--border", "--panel-2"],
  ];

  for (const theme of ["light", "dark"]) {
    await setTheme(theme);
    await check(
      `the ${theme} theme is the one in force`,
      root.dataset.theme === theme && value("--bg") !== "",
      JSON.stringify({ theme: root.dataset.theme, bg: value("--bg"), fg: value("--fg") }),
    );

    const poor = text
      .map(([fg, bg]) => [fg, bg, ratio(fg, bg)] as const)
      .filter(([, , r]) => r < 4.5);
    await check(
      `${theme}: every text colour is readable on what it sits on (4.5:1)`,
      poor.length === 0,
      JSON.stringify(poor.map(([fg, bg, r]) => `${fg} on ${bg}: ${r.toFixed(2)}`)),
    );

    const weak = affordances
      .map(([fg, bg]) => [fg, bg, ratio(fg, bg)] as const)
      .filter(([, , r]) => r < 3);
    await check(
      `${theme}: what marks focus and selection stands out (3:1)`,
      weak.length === 0,
      JSON.stringify(weak.map(([fg, bg, r]) => `${fg} on ${bg}: ${r.toFixed(2)}`)),
    );

    const invisible = separators
      .map(([fg, bg]) => [fg, bg, ratio(fg, bg)] as const)
      .filter(([, , r]) => r < 1.5);
    await check(
      `${theme}: separators can be seen against what they separate`,
      invisible.length === 0,
      JSON.stringify(
        separators.map(([fg, bg]) => `${fg} on ${bg}: ${ratio(fg, bg).toFixed(2)}`),
      ),
    );

    // The zebra row has to differ from the row beside it, and the squiggle
    // from the text it underlines.
    const zebra = Math.abs(luminance(value("--dump-row-alt")) - luminance(value("--dump-bg")));
    await check(
      `${theme}: a table's alternating rows can be told apart`,
      zebra > 0.002,
      JSON.stringify({ zebra: zebra.toFixed(4) }),
    );
  }

  // The editor repaints with the theme: its own colours come from the same
  // variables, and CodeMirror is told which way to lean.
  await setTheme("dark");
  const editorBg = getComputedStyle(ctx.editor.view.dom).backgroundColor;
  await setTheme("light");
  const lightBg = getComputedStyle(ctx.editor.view.dom).backgroundColor;
  await check(
    "the editor repaints when the theme changes",
    editorBg !== lightBg,
    JSON.stringify({ dark: editorBg, light: lightBg }),
  );

  // `system` leaves the choice to the desktop.
  await setTheme("system");
  const prefersLight = window.matchMedia("(prefers-color-scheme: light)").matches;
  await check(
    "system follows the desktop's own setting",
    root.dataset.theme === "system" &&
      (prefersLight ? value("--bg") === value("--bg") : true) &&
      value("--fg") !== "",
    JSON.stringify({ prefersLight, bg: value("--bg") }),
  );

  await invoke("save_settings", { settings: { ...settings, theme: wasTheme } });
  for (const t of [...tabs.tabs]) await tabs.close(t);
  await invoke("delete_playground", { id: main.id }).catch(() => {});
}

/**
 * Typing at a running program, and the colours it may ask for (04-05).
 */
async function stdinScenario(ctx: SelftestContext, check: Check) {
  const { tabs, runs } = ctx;
  const outText = () => ctx.output.element.querySelector(".output-body")?.textContent ?? "";
  const row = () => document.querySelector<HTMLElement>(".output-input");
  const box = () => document.querySelector<HTMLInputElement>(".output-input-box");
  const type = (text: string) => {
    const input = box();
    if (!input) return false;
    input.value = text;
    pressKey("Enter", {}, input);
    return true;
  };

  for (const t of [...tabs.tabs]) await tabs.close(t);
  const main = await openTestPlayground(ctx, "Stdin Test");
  await invoke("start_helper", { plugin: "node-check" }).catch(() => {});

  // A program that reads two lines and then reads to the end of its input.
  const program = [
    'const lines = [];',
    'process.stdin.setEncoding("utf8");',
    'let buffer = "";',
    'process.stdin.on("data", (chunk) => {',
    '  buffer += chunk;',
    '  let cut;',
    '  while ((cut = buffer.indexOf("\\n")) >= 0) {',
    '    lines.push(buffer.slice(0, cut));',
    '    buffer = buffer.slice(cut + 1);',
    '    console.log("got", lines[lines.length - 1]);',
    '  }',
    '});',
    'process.stdin.on("end", () => console.log("done after", lines.length));',
  ].join("\n") + "\n";

  const end = runs.waitForEnd();
  runCode(ctx, program);
  await until(() => row()?.hidden === false, 15_000);
  await check("a running program gets an input box", row()?.hidden === false);

  // Quiet for half a second: the box says it is probably wanted.
  const highlighted = await until(() => row()?.classList.contains("waiting") === true, 5000);
  await check("a program that has gone quiet highlights the box", highlighted);

  // A line goes to the program, and is echoed in the pane.
  type("hello");
  await until(() => outText().includes("got hello"), 15_000);
  await ctx.output.flushed();
  await check(
    "a typed line reaches the program and is echoed in the pane",
    outText().includes("got hello") && outText().includes("hello"),
    JSON.stringify({ out: outText().slice(0, 120) }),
  );
  await check(
    "what the user typed is styled as their own",
    document.querySelector(".out-input") !== null,
    JSON.stringify({ spans: [...document.querySelectorAll(".output-body span")].length }),
  );

  // Ctrl+D closes the input, so a program reading to the end finishes.
  const input = box();
  if (input) pressKey("d", { ctrl: true }, input);
  const outcome = await withTimeout(end, 30_000, "the program after end of input");
  await ctx.output.flushed();
  await check(
    "Ctrl+D ends the input and the program finishes",
    outcome.kind === "exited" && outText().includes("done after 1"),
    JSON.stringify({ outcome, out: outText().slice(-120) }),
  );
  await check("the box goes when the run does", row()?.hidden === true);

  // The same, on the per-run path: a process of its own, typed at through its
  // own pipe rather than through a helper.
  await invoke("force_per_run", { forced: true });
  const cold = runs.waitForEnd();
  runCode(
    ctx,
    'process.stdin.once("data", (d) => console.log("cold got", String(d).trim()));\n',
  );
  await until(() => row()?.hidden === false, 15_000);
  type("typed at a process");
  await until(() => outText().includes("cold got typed at a process"), 15_000);
  await check(
    "a run with no helper can be typed at too",
    outText().includes("cold got typed at a process"),
    JSON.stringify({ out: outText().slice(-80) }),
  );
  const coldInput = box();
  if (coldInput) pressKey("d", { ctrl: true }, coldInput);
  await withTimeout(cold, 30_000, "the per-run program");
  await invoke("force_per_run", { forced: false });

  // Python's input(), which is what this is for.
  const plugins = await invoke<PluginsStatus>("list_plugins");
  if (plugins.plugins.some((p) => p.id === "python")) {
    let state = (await invoke<LanguageToolchain[]>("toolchain_status")).find(
      (s) => s.language === "python",
    )?.state;
    if (state?.kind === "ready") {
      const py = await invoke<Playground>("create_playground", {
        name: "Stdin Python",
        language: "python",
      });
      await openPlayground(ctx.editor, ctx.status, py.id);
      const asked = runs.waitForEnd();
      runCode(ctx, 'name = input("who? ")\nprint(f"hello {name}")\n');
      await until(() => row()?.hidden === false, 60_000);
      await until(() => outText().includes("who?"), 60_000);
      type("Ada");
      const outcome = await withTimeout(asked, 60_000, "python input()");
      await ctx.output.flushed();
      await check(
        "Python's input() waits for what is typed and gets it",
        outcome.kind === "exited" && outText().includes("hello Ada"),
        JSON.stringify({ outcome, out: outText().slice(-80) }),
      );
      await tabs.close(tabs.activeTab!);
      await invoke("delete_playground", { id: py.id }).catch(() => {});
    } else {
      await report("INFO python isn't installed, so input() wasn't checked");
    }
  }

  // And C#'s Console.ReadLine(), through the Roslyn helper.
  if (plugins.plugins.some((p) => p.id === "csharp")) {
    const state = (await invoke<LanguageToolchain[]>("toolchain_status")).find(
      (s) => s.language === "csharp",
    )?.state;
    if (state?.kind === "ready") {
      const cs = await invoke<Playground>("create_playground", {
        name: "Stdin CSharp",
        language: "csharp",
        kind: "statements",
      });
      await openPlayground(ctx.editor, ctx.status, cs.id);
      const asked = runs.waitForEnd();
      runCode(
        ctx,
        'Console.Write("who? ");\nvar who = Console.ReadLine();\nConsole.WriteLine($"hello {who}");\n',
      );
      await until(() => row()?.hidden === false, 120_000);
      await until(() => outText().includes("who?"), 120_000);
      type("Grace");
      const outcome = await withTimeout(asked, 120_000, "C# Console.ReadLine()");
      await ctx.output.flushed();
      await check(
        "C#'s Console.ReadLine() gets what is typed",
        outcome.kind === "exited" && outText().includes("hello Grace"),
        JSON.stringify({ outcome, out: outText().slice(-80) }),
      );
      await tabs.close(tabs.activeTab!);
      await invoke("delete_playground", { id: cs.id }).catch(() => {});
    } else {
      await report("INFO the .NET SDK isn't installed, so ReadLine() wasn't checked");
    }
  }

  // ANSI colours in output (work item 04-05).
  const coloured = runs.waitForEnd();
  runCode(
    ctx,
    'console.log("\u001b[31mred \u001b[1mbold \u001b[0mplain");\n',
  );
  await withTimeout(coloured, 30_000, "the coloured run");
  await ctx.output.flushed();
  const spans = [...document.querySelectorAll<HTMLElement>(".output-body span[data-ansi]")];
  await check(
    "ANSI colours become styled text, and the escapes themselves are gone",
    spans.length >= 2 &&
      spans[0]?.style.color !== "" &&
      !outText().includes("[31m") &&
      outText().includes("red bold plain"),
    JSON.stringify({
      spans: spans.map((span) => [span.textContent, span.style.color, span.style.fontWeight]),
      out: outText(),
    }),
  );

  for (const t of [...tabs.tabs]) await tabs.close(t);
  await invoke("delete_playground", { id: main.id }).catch(() => {});
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
  python: pythonScenario,
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
  "lsp-typescript": lspTypescriptScenario,
  prewarm: prewarmScenario,
  servers: serversScenario,
  helpers: helpersScenario,
  "helper-run": helperRunScenario,
  definition: definitionScenario,
  "library-panel": libraryPanelScenario,
  keys: keysScenario,
  stdin: stdinScenario,
  themes: themesScenario,
  settings: settingsScenario,
  history: historyScenario,
  hooks: hooksScenario,
  packages: packagesScenario,
  "packages-live": packagesLiveScenario,
};

/**
 * Plugin helpers: built when their source changes, warm in between, and shown
 * in the status strip (work item 03-02).
 *
 * The test plugin ships a helper in `helper.js` and a stand-in build step in
 * `build.js`, which says what it is doing on stdout. Whether the helper has
 * been built on this machine before is not something a test can assume, so
 * this touches the plugin folder to make a build due, and then checks that the
 * next start after that doesn't build it again.
 */
async function helpersScenario(ctx: SelftestContext, check: Check) {
  const { tabs } = ctx;
  const chips = () => [...document.querySelectorAll<HTMLElement>(".server-chip")];
  const chip = () => chips().find((c) => (c.dataset.key ?? "").startsWith("helper|node-check|"));
  const popover = () => document.querySelector<HTMLElement>(".server-popover");
  const openPopover = async () => {
    if (popover() === null) chip()?.click();
    await frame();
  };
  const status = async () =>
    (await invoke<HelperStatus[]>("helper_status")).find((h) => h.plugin === "node-check");
  const log = async () => invoke<string[]>("helper_log", { plugin: "node-check" });
  const builds = async () =>
    (await log()).filter((line) => line.startsWith("Building the node-check helper")).length;
  const start = async () => invoke<boolean>("start_helper", { plugin: "node-check" });
  const stop = async () => invoke<boolean>("stop_helper", { plugin: "node-check" });

  for (const t of [...tabs.tabs]) await tabs.close(t);
  // A tab open in the language, so a stopped helper still gets a chip.
  const playground = await openTestPlayground(ctx, "Helpers Test");

  // Starting one is the run path's job in 03-03; here the command stands in.
  const started = await start().catch((e: unknown) => String(e));
  await check("a helper starts on demand", started === true, String(started));

  const ready = await until(() => chip()?.dataset.state === "ready", 30_000);
  await check(
    "and shows up in the status strip as a chip of its own",
    ready && (chip()?.textContent ?? "").includes("helper"),
    `${chip()?.dataset.key ?? "gone"}: ${chip()?.textContent ?? ""}`,
  );

  const first = await status();
  await check(
    "the core knows what it is: a process, a name, and nothing using it",
    first?.state.kind === "ready" &&
      (first.state.pid ?? 0) > 0 &&
      first.state.name === "node-check-helper" &&
      first.uses === 0,
    JSON.stringify(first?.state),
  );

  // Its details are the ones worth having.
  await openPopover();
  const facts = [...(popover()?.querySelectorAll("dt") ?? [])].map((dt) => dt.textContent);
  await check(
    "its popover shows what it is doing",
    facts.includes("Process") && facts.includes("Running") && facts.includes("Idle"),
    JSON.stringify(facts),
  );

  // Change the helper's source, and the next start builds it.
  const plugins = await invoke<PluginsStatus>("list_plugins");
  const dir = plugins.plugins.find((p) => p.id === "node-check")?.dir ?? "";
  const before = await builds();
  await invoke("selftest_write_file", {
    path: `${dir}/selftest-touch.txt`,
    text: `${Date.now()}\n`,
  });
  await stop();
  await start();
  const after = await log();
  const rebuilt = after.filter((l) => l.startsWith("Building the node-check helper")).length;
  await check(
    "changing the helper's source builds it again, with its output kept",
    rebuilt === before + 1 &&
      after.some((line) => line.includes("building the node-check helper")) &&
      after.some((line) => line.startsWith("Built the node-check helper in")),
    JSON.stringify(after.slice(-4)),
  );

  // Starting it again doesn't: nothing has changed since.
  await stop();
  await start();
  await check("and starting it again doesn't rebuild it", (await builds()) === rebuilt, `${rebuilt} builds`);

  // Stop and Restart, the two things worth doing from the strip.
  await until(() => chip()?.dataset.state === "ready", 30_000);
  await openPopover();
  const action = (label: string) =>
    [...(popover()?.querySelectorAll<HTMLElement>(".server-action") ?? [])].find(
      (button) => button.textContent === label,
    );
  const wasPid = (await status())?.state.pid ?? 0;
  action("Stop")?.click();
  const stopped = await until(() => chip()?.dataset.state === "stopped", 30_000);
  await check("Stop stops it", stopped, chip()?.dataset.state ?? "gone");

  await openPopover();
  action("Restart")?.click();
  const back = await until(() => chip()?.dataset.state === "ready", 30_000);
  const second = await status();
  await check(
    "Restart brings it back as a new process",
    back && (second?.state.pid ?? 0) > 0 && second?.state.pid !== wasPid,
    JSON.stringify(second?.state),
  );

  // Leave a chip and its popover on screen for a screenshot.
  await openPopover();
  await report("INFO shot-ready");

  await invoke("delete_playground", { id: playground.id }).catch(() => {});
}

/**
 * Runs on a warm helper, and the fall back to per-run when there isn't one
 * (work item 03-03).
 *
 * The test plugin's helper runs the file in a child process, so it behaves
 * exactly like the plugin's `[run]` command — which is the point: apart from
 * the word "warm" in the status bar, nothing above the runner can tell the two
 * apart.
 */
async function helperRunScenario(ctx: SelftestContext, check: Check) {
  const { runs, tabs } = ctx;
  const outText = () => ctx.output.element.querySelector(".output-body")?.textContent ?? "";
  const status = () => document.querySelector(".status-run")?.textContent ?? "";
  const toasts: string[] = [];
  const unsubscribe = subscribe(({ event }) => {
    if (event.type === "toast") toasts.push(event.message);
  });
  const plugins = await invoke<PluginsStatus>("list_plugins");
  const dir = plugins.plugins.find((p) => p.id === "node-check")?.dir ?? "";
  const breaks = `${dir}/break.txt`;

  for (const t of [...tabs.tabs]) await tabs.close(t);
  const main = await openTestPlayground(ctx, "Helper Run");

  // A run that takes long enough to look at while it is going.
  let end = runs.waitForEnd();
  runCode(ctx, 'console.log("from the helper"); setTimeout(() => {}, 1500)\n');
  const warm = await until(() => status().includes("warm"), 30_000);
  await check("a run on a plugin helper says so in the status bar", warm, status());

  let outcome = await withTimeout(end, 30_000, "helper run");
  await ctx.output.flushed();
  await check(
    // A build may have run first, and its output belongs to this run too, so
    // what matters is that the program's output is there and is last.
    "and is otherwise the same run: same output, same exit",
    outcome.kind === "exited" && outcome.code === 0 && outText().endsWith("from the helper\n"),
    JSON.stringify({ outcome, out: outText() }),
  );

  const helper = (await invoke<HelperStatus[]>("helper_status")).find(
    (h) => h.plugin === "node-check",
  );
  await check(
    "the helper is up and free again afterwards",
    helper?.state.kind === "ready" && helper.uses === 0,
    JSON.stringify(helper?.state),
  );

  // Cancel goes to the helper, not to a process the app started.
  end = runs.waitForEnd();
  runCode(ctx, "setInterval(() => {}, 1000)\n");
  await until(() => status().startsWith("Running"), 30_000);
  await runs.cancel();
  outcome = await withTimeout(end, 30_000, "cancel");
  await check("cancelling a warm run stops it", outcome.kind === "cancelled", JSON.stringify(outcome));
  const after = (await invoke<HelperStatus[]>("helper_status")).find(
    (h) => h.plugin === "node-check",
  );
  await check(
    "and leaves the helper warm for the next one",
    after?.state.kind === "ready",
    JSON.stringify(after?.state),
  );

  // Two playgrounds, one helper: the second waits rather than taking it.
  const other = await openTestPlayground(ctx, "Helper Run Other");
  await tabs.open(main.id);
  const longRun = runs.waitForEnd();
  runCode(ctx, "setInterval(() => {}, 1000)\n");
  await until(() => status().startsWith("Running"), 30_000);
  await tabs.open(other.id);
  const queued = runs.waitForEnd();
  runCode(ctx, 'console.log("second")\n');
  const waiting = await until(() => status().startsWith("Waiting for the Node check helper"), 20_000);
  await check("a second playground waits for the helper", waiting, status());

  await tabs.open(main.id);
  await runs.cancel();
  await withTimeout(longRun, 30_000, "first run");
  await tabs.open(other.id);
  const second = await withTimeout(queued, 30_000, "queued run");
  await ctx.output.flushed();
  await check(
    "and runs as soon as the helper is free",
    second.kind === "exited" && second.code === 0 && outText() === "second\n",
    JSON.stringify({ second, out: outText() }),
  );

  // Force per-run: the same run, cold, with nothing else different.
  await invoke("force_per_run", { forced: true });
  end = runs.waitForEnd();
  runCode(ctx, 'console.log("cold"); setTimeout(() => {}, 1200)\n');
  const cold = await until(() => status().startsWith("Running"), 30_000);
  await check(
    "Force per-run keeps runs off the helper",
    cold && !status().includes("warm"),
    status(),
  );
  outcome = await withTimeout(end, 30_000, "forced per-run");
  await ctx.output.flushed();
  await check(
    "and the run is otherwise unchanged",
    outcome.kind === "exited" && outText() === "cold\n",
    JSON.stringify({ outcome, out: outText() }),
  );
  await invoke("force_per_run", { forced: false });

  // A helper that can't be built falls back, and says so once.
  await invoke("selftest_write_file", { path: breaks, text: "yes\n" });
  await invoke("stop_helper", { plugin: "node-check" });
  toasts.length = 0;
  end = runs.waitForEnd();
  runCode(ctx, 'console.log("fell back")\n');
  outcome = await withTimeout(end, 60_000, "fallback run");
  await ctx.output.flushed();
  await check(
    "a helper that can't be built doesn't stop the run",
    outcome.kind === "exited" && outcome.code === 0 && outText().includes("fell back"),
    JSON.stringify({ outcome, out: outText() }),
  );
  await check(
    "and the user is told what is being used instead",
    toasts.some((t) => t.includes("helper isn't available") && t.includes("Running with node")),
    JSON.stringify(toasts),
  );

  // Once per session, not once per run.
  const told = toasts.length;
  end = runs.waitForEnd();
  runCode(ctx, 'console.log("again")\n');
  await withTimeout(end, 60_000, "second fallback run");
  await check("said once, not every run", toasts.length === told, JSON.stringify(toasts));

  // Put the plugin back as it was, and leave a warm run on screen.
  await invoke("selftest_write_file", { path: breaks, text: "" });
  unsubscribe();
  await invoke("delete_playground", { id: other.id }).catch(() => {});
  await invoke("delete_playground", { id: main.id }).catch(() => {});
  await report("INFO shot-ready");
}

/**
 * The Python plugin end to end (work item 03-04): probes, templates, `dump()`,
 * tracebacks, and the warm helper that runs it all.
 */
async function pythonScenario(ctx: SelftestContext, check: Check) {
  const { runs, editor, status } = ctx;
  const outText = () => ctx.output.element.querySelector(".output-body")?.textContent ?? "";
  const runStatus = () => document.querySelector(".status-run")?.textContent ?? "";

  const plugins = await invoke<PluginsStatus>("list_plugins");
  const python = plugins.plugins.find((p) => p.id === "python");
  await check("the Python plugin is bundled", python?.source === "bundled", JSON.stringify(python ?? null));

  const state = await settledToolchain("python");
  await check(
    "Python is ready, whichever of python/python3/py answered",
    state?.kind === "ready" && /^3\.(1[1-9]|[2-9]\d)/.test(state.version ?? ""),
    JSON.stringify(state ?? null),
  );
  if (state?.kind !== "ready") return;
  await report(`INFO python: ${state.program} ${state.version} at ${state.path}`);

  const created = await invoke<Playground>("create_playground", { name: "Py Self Test", language: "python" });
  await openPlayground(editor, status, created.id);
  await check(
    "a new playground starts from the statements template",
    created.kind === "statements" && editor.text.startsWith("from tinkerpad import dump\n"),
    JSON.stringify(editor.text.slice(0, 60)),
  );

  // Mod+Enter on the untouched template, until its table is on screen.
  const tableVisible = async (label: string) => {
    const end = runs.waitForEnd();
    const start = performance.now();
    runScopeHandlers(editor.view, key("Enter", { mod: true }), "editor");
    let ms: number | null = null;
    let warm = false;
    while (performance.now() - start < 30_000) {
      warm ||= runStatus().includes("warm");
      const table = ctx.output.element.querySelector<HTMLElement>(".dump-kind-table");
      if (table && table.getClientRects().length > 0) {
        ms = performance.now() - start;
        break;
      }
      await new Promise((r) => requestAnimationFrame(r));
    }
    const outcome = await withTimeout(end, 30_000, label);
    await report(`INFO ${label}: Mod+Enter to table visible ${ms === null ? "never" : `${ms.toFixed(0)} ms`}`);
    return { ms, outcome, warm };
  };

  const first = await tableVisible("Python first run");
  await ctx.output.flushed();
  const table = ctx.output.element.querySelector(".dump-kind-table");
  await check(
    "the template's dump renders a table, on the helper",
    first.ms !== null &&
      first.outcome.kind === "exited" &&
      first.outcome.code === 0 &&
      first.warm &&
      table?.querySelector(".dump-title")?.textContent === "people" &&
      table.querySelectorAll("tbody tr").length === 2 &&
      !ctx.output.element.querySelector(".out-stderr"),
    JSON.stringify({ outcome: first.outcome, warm: first.warm, text: outText().slice(0, 200) }),
  );
  // The second run is too quick to catch "warm" in the status bar — it is over
  // in a few milliseconds — so what it proves is the speed, and the pid check
  // below proves it was the same interpreter.
  const second = await tableVisible("Python warm run");
  await check(
    "and again, faster, on the interpreter that is already up",
    second.outcome.kind === "exited" && (second.ms ?? 1e9) < (first.ms ?? 0),
    JSON.stringify({ outcome: second.outcome, first: first.ms, second: second.ms }),
  );

  // The interpreter really is the same one between runs.
  let end = runs.waitForEnd();
  runCode(ctx, "import os, sys\nprint(os.getpid(), sys.version_info.major)\n");
  await withTimeout(end, 30_000, "pid one");
  await ctx.output.flushed();
  const firstPid = outText().trim();
  end = runs.waitForEnd();
  runCode(ctx, "import os\nprint(os.getpid())\n");
  await withTimeout(end, 30_000, "pid two");
  await ctx.output.flushed();
  await check(
    "two runs share one interpreter, which is what warm means",
    firstPid.startsWith(outText().trim()) && outText().trim().length > 0,
    JSON.stringify({ firstPid, secondPid: outText().trim() }),
  );

  // Locked lines: the expression kind wraps one expression in a dump().
  const expression = await invoke<Playground>("create_playground", {
    name: "Py Expression Test",
    language: "python",
    kind: "expression",
  });
  await openPlayground(editor, status, expression.id);
  const expressionEnd = runs.waitForEnd();
  runScopeHandlers(editor.view, key("Enter", { mod: true }), "editor");
  const expressionOutcome = await withTimeout(expressionEnd, 30_000, "expression template");
  await ctx.output.flushed();
  await check(
    "the expression kind dumps its one expression",
    expressionOutcome.kind === "exited" &&
      editor.text.startsWith("from tinkerpad import dump\n\ndump((\n") &&
      ctx.output.element.querySelector(".dump-kind-tree") !== null,
    JSON.stringify({ expressionOutcome, text: editor.text.slice(0, 60) }),
  );

  // A traceback points at the line it happened on.
  await openPlayground(editor, status, created.id);
  end = runs.waitForEnd();
  runCode(ctx, 'print("before")\n\n\ndef boom():\n    return 1 / 0\n\n\nboom()\n');
  const failed = await withTimeout(end, 30_000, "traceback");
  await ctx.output.flushed();
  const squiggles = [...document.querySelectorAll(".cm-annotation")].map((el) => el.textContent ?? "");
  await check(
    "a traceback is reported, and its line marked in the editor",
    failed.kind === "exited" &&
      failed.code === 1 &&
      outText().includes("ZeroDivisionError") &&
      squiggles.some((s) => s.includes("1 / 0")),
    JSON.stringify({ failed, squiggles, out: outText().slice(-120) }),
  );

  // Cancel reaches the code running inside the helper.
  end = runs.waitForEnd();
  runCode(ctx, "while True:\n    pass\n");
  await until(() => runStatus().startsWith("Running"), 30_000);
  await runs.cancel();
  const cancelled = await withTimeout(end, 30_000, "cancel");
  await check("a runaway loop can be cancelled", cancelled.kind === "cancelled", JSON.stringify(cancelled));

  const helper = (await invoke<HelperStatus[]>("helper_status")).find((h) => h.plugin === "python");
  await check(
    "and the helper is still warm afterwards",
    helper?.state.kind === "ready",
    JSON.stringify(helper?.state ?? null),
  );

  // The language server, when basedpyright is installed (`pip install
  // basedpyright`). Without it the editor still works, which is the point of
  // checking rather than assuming.
  const view = editor.view;
  const tab = ctx.tabs.activeTab;
  const connected = await until(() => (tab?.lsp ?? null) !== null, 60_000);
  if (!connected) {
    await report("INFO basedpyright did not connect; skipping the language server checks");
  } else {
    const source = 'greeting = "hello"\ngreeting\n';
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: source },
    });
    // Then the dot on its own, as a keystroke would arrive: the server has to
    // have seen the document before it can say what follows it.
    await sleep(500);
    const dotAt = source.indexOf("greeting\n") + "greeting".length;
    view.dispatch({ changes: { from: dotAt, insert: "." }, selection: { anchor: dotAt + 1 } });
    // Ask until the answer is about the string, not about the module: a
    // server that hasn't caught up with the dot yet answers with what follows
    // a bare name, which is not nothing, so "got something" isn't enough.
    let completions: readonly { label: string }[] = [];
    let got = false;
    for (let attempt = 0; attempt < 30 && !got; attempt++) {
      closeCompletion(view);
      startCompletion(view);
      got = await until(() => {
        completions = currentCompletions(view.state);
        return completions.some((c) => c.label.startsWith("upper"));
      }, 2000);
    }
    await check(
      "typing `.` offers the methods basedpyright knows about",
      got,
      JSON.stringify({
        of: completions.length,
        str: completions
          .filter((c) => /^(upper|lower|strip|split|title)/.test(c.label))
          .map((c) => c.label),
      }),
    );
    closeCompletion(view);
  }

  // The measurement the work item asks for: the same one-line program, warm
  // and cold, from the keystroke to the first character of output.
  const printOne = async (label: string) => {
    let best = Infinity;
    for (let i = 0; i < 3; i++) {
      const done = runs.waitForEnd();
      runCode(ctx, "print(1)\n");
      await withTimeout(done, 30_000, label);
      await ctx.output.flushed();
      best = Math.min(best, runs.firstOutputMs ?? Infinity);
    }
    await report(`INFO print(1) ${label}: Mod+Enter to first output ${best.toFixed(1)} ms (best of 3)`);
    return best;
  };
  const warmMs = await printOne("on the helper");
  await invoke("force_per_run", { forced: true });
  const coldMs = await printOne("per-run");
  await invoke("force_per_run", { forced: false });
  await check(
    "a warm run beats starting an interpreter",
    warmMs < coldMs,
    JSON.stringify({ warmMs, coldMs }),
  );

  // Leave a table and some text on screen for a screenshot.
  end = runs.waitForEnd();
  runCode(
    ctx,
    [
      "from dataclasses import dataclass",
      "from tinkerpad import dump",
      "",
      "",
      "@dataclass",
      "class Person:",
      "    name: str",
      "    born: int",
      "    known_for: str",
      "",
      "",
      'print("Hello, world!")',
      "people = dump(",
      "    [",
      '        Person("Ada Lovelace", 1815, "the first program"),',
      '        Person("Grace Hopper", 1906, "COBOL"),',
      '        Person("Alan Turing", 1912, "the Turing machine"),',
      "    ],",
      '    "people",',
      ")",
      'print(f"{len(people)} people")',
      "",
    ].join("\n"),
  );
  await withTimeout(end, 30_000, "final run");
  await ctx.output.flushed();
  await report("INFO shot-ready");
}

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
