// The output pane: an append-only list of blocks, in order. Text blocks are
// runs of stdout, stderr, and info text; dump blocks are rendered values.
//
// Appends are batched per animation frame and consecutive text of the same
// stream is merged into one text node, so thousands of tiny chunks cost a
// handful of DOM operations. The DOM is capped at MAX_LINES; older blocks are
// dropped with a notice.

import { Ansi, paint, type Style, styled } from "./ansi";
import { handleDumpKey } from "./dump/nav";
import { renderRich } from "./dump/render";
import { disposeTables } from "./dump/table";
import type { Annotation, RichValue } from "./protocol";

export type Stream = "stdout" | "stderr" | "info" | "input";

/**
 * How long a program can say nothing before the input box is highlighted as
 * probably waiting for something (work item 04-05). A heuristic: some programs
 * are simply slow, and nothing breaks if the guess is wrong.
 */
const WAITING_MS = 500;

type Pending =
  | { stream: Stream; text: string; style?: Style }
  | { rich: RichValue }
  | { annotation: Annotation; onClick: () => void };

const MAX_LINES = 20_000;
const BOTTOM_SLACK_PX = 4;

export class OutputPane {
  readonly element: HTMLElement;
  private readonly body: HTMLElement;
  private readonly jump: HTMLButtonElement;
  private readonly trimmedNotice: HTMLElement;
  /** Where the user types at a running program (work item 04-05). */
  private readonly inputRow: HTMLElement;
  private readonly input: HTMLInputElement;
  /** Called with a line (newline included) when the user sends one. */
  onInput: (text: string) => void = () => {};
  /** Called when the user closes the program's input (Ctrl+D). */
  onInputEnd: () => void = () => {};
  /** Set while a run that takes input is going. */
  private waitingTimer = 0;
  /** Colours a program asked for, carried across chunks (work item 04-05). */
  private ansi = new Ansi();

  private pending: Pending[] = [];
  private frame = 0;
  private lines = 0;
  private trimmed = 0;
  private stick = true;

  constructor(private readonly onEscape: () => void) {
    this.element = document.createElement("div");
    this.element.className = "output";
    this.element.tabIndex = 0;
    this.element.setAttribute("aria-label", "Output");

    this.trimmedNotice = document.createElement("div");
    this.trimmedNotice.className = "output-trimmed";
    this.trimmedNotice.hidden = true;

    this.body = document.createElement("div");
    this.body.className = "output-body";

    this.jump = document.createElement("button");
    this.jump.className = "output-jump";
    this.jump.textContent = "↓ Jump to bottom";
    this.jump.hidden = true;
    this.jump.addEventListener("click", () => {
      this.stick = true;
      this.scrollToBottom();
      this.element.focus();
    });

    // The input row sits under the output, and is only there while a run
    // that can be typed at is going (work item 04-05).
    this.inputRow = document.createElement("div");
    this.inputRow.className = "output-input";
    this.inputRow.hidden = true;
    const prompt = document.createElement("span");
    prompt.className = "output-prompt";
    prompt.textContent = ">";
    this.input = document.createElement("input");
    this.input.className = "output-input-box";
    this.input.type = "text";
    this.input.placeholder = "Type here to send input, Enter to send";
    this.input.setAttribute("aria-label", "Input for the running program");
    this.input.addEventListener("keydown", (e) => this.onInputKey(e));
    this.input.addEventListener("input", () => this.stopWaiting());
    this.inputRow.append(prompt, this.input);

    this.element.append(this.trimmedNotice, this.body, this.jump, this.inputRow);

    this.element.addEventListener("scroll", () => {
      this.stick = this.atBottom();
      this.jump.hidden = this.stick;
    });
    this.element.addEventListener("keydown", (e) => {
      if (e.key === "Escape") {
        e.preventDefault();
        this.onEscape();
      } else if (handleDumpKey(e, this.element)) {
        e.preventDefault();
      }
    });
    // Dumps change height after they're added (images load, rows fill in,
    // nodes open). Stay pinned to the bottom if we were there.
    new ResizeObserver(() => {
      if (this.stick) this.scrollToBottom();
    }).observe(this.body);
  }

  /**
   * Show or hide the input row. Showing it starts the clock that highlights
   * it when the program has gone quiet (work item 04-05).
   */
  setInput(open: boolean): void {
    this.inputRow.hidden = !open;
    this.stopWaiting();
    if (open) this.expectWait();
    else this.input.value = "";
  }

  /** Whether the program can be typed at right now. */
  get takesInput(): boolean {
    return !this.inputRow.hidden;
  }

  /** Put the cursor in the input box, if it is there. */
  focusInput(): void {
    if (!this.inputRow.hidden) this.input.focus();
  }

  private onInputKey(e: KeyboardEvent): void {
    // Ctrl+D is end-of-input everywhere a terminal is, and it is what a
    // program blocked on "read until EOF" is waiting for.
    if (e.key === "d" && e.ctrlKey && !e.shiftKey) {
      e.preventDefault();
      this.append("info", "^D" + "\n");
      this.onInputEnd();
      return;
    }
    if (e.key !== "Enter") return;
    e.preventDefault();
    const line = this.input.value;
    this.input.value = "";
    // Echoed in its own style: what you typed is not what the program said.
    this.echo(line);
    this.onInput(line + "\n");
    this.expectWait();
  }

  /** Show a line the user sent, so the pane reads like a conversation. */
  private echo(line: string): void {
    this.pending.push({ stream: "input", text: line + "\n" });
    this.schedule();
  }

  /**
   * A program that has said nothing for a moment is probably waiting to be
   * typed at. A guess, so it only draws attention to the box (04-05).
   */
  private expectWait(): void {
    this.stopWaiting();
    this.waitingTimer = window.setTimeout(() => {
      this.inputRow.classList.add("waiting");
    }, WAITING_MS);
  }

  /** Restart the quiet clock, but only while the box is showing. */
  private expectWaitIfOpen(): void {
    if (!this.inputRow.hidden) this.expectWait();
  }

  private stopWaiting(): void {
    clearTimeout(this.waitingTimer);
    this.waitingTimer = 0;
    this.inputRow.classList.remove("waiting");
  }

  append(stream: Stream, text: string): void {
    // Anything the program says means it isn't waiting on us just now.
    if (stream !== "input") this.expectWaitIfOpen();
    if (!text) return;
    // A program's own output may carry ANSI colours; the pane's own notes
    // never do (work item 04-05).
    if (stream === "stdout" || stream === "stderr") {
      for (const run of this.ansi.feed(text)) {
        if (run.text) this.pending.push({ stream, text: run.text, style: run.style });
      }
    } else {
      this.pending.push({ stream, text });
    }
    this.schedule();
  }

  /** Add a dumped value after whatever output came before it. */
  appendRich(value: RichValue): void {
    this.pending.push({ rich: value });
    this.schedule();
  }

  /**
   * Make the line an annotation was found in clickable. Called right after the
   * text it was found in, so the line is the end of the latest output.
   */
  markAnnotation(annotation: Annotation, onClick: () => void): void {
    this.pending.push({ annotation, onClick });
    this.schedule();
  }

  clear(): void {
    cancelAnimationFrame(this.frame);
    this.frame = 0;
    this.pending = [];
    this.ansi = new Ansi();
    disposeTables(this.body);
    this.body.replaceChildren();
    this.lines = 0;
    this.trimmed = 0;
    this.trimmedNotice.hidden = true;
    this.stick = true;
    this.jump.hidden = true;
  }

  /** The pane was put back on screen: stay pinned to the bottom if it was. */
  attached(): void {
    if (this.stick) this.scrollToBottom();
  }

  /** Resolves after pending appends have reached the DOM. */
  flushed(): Promise<void> {
    return new Promise((resolve) =>
      requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
    );
  }

  private schedule(): void {
    this.frame ||= requestAnimationFrame(() => this.flush());
  }

  private flush(): void {
    this.frame = 0;
    const batch = this.pending;
    this.pending = [];
    if (batch.length === 0) return;

    // Merge consecutive text of the same stream, then extend the last span
    // if it's the same stream too.
    const fragment = document.createDocumentFragment();
    let last = this.body.lastElementChild as HTMLElement | null;
    let lastStream = last?.dataset.stream;
    let run = "";
    let runStream: Stream | null = null;
    let runStyle: Style | undefined;

    const emit = () => {
      if (runStream === null || !run) return;
      // Text that asked for colours is its own span, and never merges with
      // the one before it.
      const plain = !runStyle || !styled(runStyle);
      if (plain && runStream === lastStream && last && fragment.childNodes.length === 0) {
        last.append(run);
      } else {
        const span = document.createElement("span");
        span.className = `out-${runStream}`;
        span.dataset.stream = runStream;
        span.textContent = run;
        if (!plain && runStyle) {
          paint(span, runStyle);
          span.dataset.ansi = "";
        }
        fragment.append(span);
        last = span;
        lastStream = plain ? runStream : undefined;
      }
      run = "";
    };

    for (const item of batch) {
      if ("annotation" in item) {
        emit();
        runStream = null;
        if (last) markLine(last, item.annotation, item.onClick);
        continue;
      }
      if ("rich" in item) {
        emit();
        runStream = null;
        const node = renderRich(item.rich, { scrollRoot: this.element });
        // A block counts as one line towards the cap; inline values as none.
        node.dataset.lines = node.classList.contains("dump-inline") ? "0" : "1";
        this.lines += Number(node.dataset.lines);
        fragment.append(node);
        last = node;
        lastStream = undefined;
        continue;
      }
      if (item.stream !== runStream || !sameStyle(item.style, runStyle)) {
        emit();
        runStream = item.stream;
        runStyle = item.style;
      }
      run += item.text;
      this.lines += countNewlines(item.text);
    }
    emit();
    this.body.append(fragment);

    this.trim();
    if (this.stick) this.scrollToBottom();
  }

  private trim(): void {
    while (this.lines > MAX_LINES && this.body.firstChild) {
      const first = this.body.firstChild as HTMLElement;
      const excess = this.lines - MAX_LINES;
      if (first.dataset.lines !== undefined) {
        const n = Number(first.dataset.lines);
        disposeTables(first);
        first.remove();
        this.lines -= n;
        this.trimmed += n;
        continue;
      }
      const text = first.textContent ?? "";
      const n = countNewlines(text);
      if (n <= excess) {
        first.remove();
        this.lines -= n;
        this.trimmed += n;
      } else {
        // Cut the first `excess` lines out of this span.
        let idx = -1;
        for (let i = 0; i < excess; i++) idx = text.indexOf("\n", idx + 1);
        first.textContent = text.slice(idx + 1);
        this.lines -= excess;
        this.trimmed += excess;
      }
    }
    if (this.trimmed > 0) {
      this.trimmedNotice.hidden = false;
      this.trimmedNotice.textContent = `${this.trimmed.toLocaleString()} earlier lines not shown`;
    }
  }

  private atBottom(): boolean {
    const el = this.element;
    return el.scrollHeight - el.scrollTop - el.clientHeight <= BOTTOM_SLACK_PX;
  }

  private scrollToBottom(): void {
    this.element.scrollTop = this.element.scrollHeight;
  }
}

/**
 * Turn the last occurrence of the annotation's line inside `span` into a
 * clickable link. The line has just been appended, so it's at the end.
 */
function markLine(span: HTMLElement, annotation: Annotation, onClick: () => void): void {
  const needle = (annotation.text.split("\n").pop() ?? "").trimEnd();
  const text = span.textContent ?? "";
  const at = needle ? text.lastIndexOf(needle) : -1;
  if (at < 0) return;

  const link = document.createElement("span");
  link.className = `out-annotation out-annotation-${annotation.severity}`;
  link.textContent = needle;
  link.dataset.nav = "";
  link.tabIndex = -1;
  link.setAttribute("role", "link");
  link.title = annotation.file
    ? `${annotation.file}:${annotation.line}${annotation.col ? `:${annotation.col}` : ""} — click to go there`
    : `Line ${annotation.line} — click to go there`;
  link.addEventListener("click", (e) => {
    e.preventDefault();
    onClick();
  });
  span.replaceChildren(
    document.createTextNode(text.slice(0, at)),
    link,
    document.createTextNode(text.slice(at + needle.length)),
  );
}

function countNewlines(s: string): number {
  let n = 0;
  for (let i = s.indexOf("\n"); i !== -1; i = s.indexOf("\n", i + 1)) n++;
  return n;
}

/** Whether two runs of text want the same thing, so they can share a span. */
function sameStyle(a: Style | undefined, b: Style | undefined): boolean {
  if (a === b) return true;
  if (!a || !b) return !styled(a ?? {}) && !styled(b ?? {});
  return (
    a.fg === b.fg &&
    a.bg === b.bg &&
    !!a.bold === !!b.bold &&
    !!a.dim === !!b.dim &&
    !!a.italic === !!b.italic &&
    !!a.underline === !!b.underline &&
    !!a.inverse === !!b.inverse
  );
}
