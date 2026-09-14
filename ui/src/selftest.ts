// Opt-in in-app self-test, used to verify UI behaviour from scripts and CI
// without sending keystrokes to the desktop. Enabled only when the app is
// started with TINKERPAD_SELFTEST set; results go to the app's stdout as
// "selftest: PASS ..." / "selftest: FAIL ..." lines, ending with
// "selftest: done".

import { runScopeHandlers } from "@codemirror/view";

import type { Editor } from "./editor";
import { invoke } from "./ipc";
import type { OutputPane } from "./output";

export interface SelftestContext {
  editor: Editor;
  output: OutputPane;
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
  const check = async (name: string, ok: boolean, detail = "") => {
    if (!ok) failures++;
    await report(`${ok ? "PASS" : "FAIL"} ${name}${detail ? ` (${detail})` : ""}`);
  };

  try {
    if (scenario === "layout") await layoutScenario(ctx, check);
    else await report(`FAIL unknown scenario ${scenario}`);
  } catch (e) {
    failures++;
    await report(`FAIL exception ${String(e)}`);
  }
  await report(`done failures=${failures}`);
}

type Check = (name: string, ok: boolean, detail?: string) => Promise<void>;

async function layoutScenario({ editor, output }: SelftestContext, check: Check) {
  const view = editor.view;
  const outText = () => output.element.querySelector(".output-body")?.textContent ?? "";

  // Typing into the editor.
  view.dispatch({ changes: { from: view.state.doc.length, insert: "const x = [1, 2, 3];\n" } });
  await check("editor accepts input", editor.text.includes("const x = [1, 2, 3];"));

  // Mod+Enter through CodeMirror's real keymap.
  output.clear();
  const handled = runScopeHandlers(view, key("Enter", { mod: true }), "editor");
  await frame();
  await check("Mod+Enter runs", handled && outText().includes("Run requested"));

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
