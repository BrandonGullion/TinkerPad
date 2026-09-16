// The output pane: an append-only list of blocks, in order. Text blocks are
// runs of stdout, stderr, and info text; dump blocks are rendered values.
//
// Appends are batched per animation frame and consecutive text of the same
// stream is merged into one text node, so thousands of tiny chunks cost a
// handful of DOM operations. The DOM is capped at MAX_LINES; older blocks are
// dropped with a notice.

import { handleDumpKey } from "./dump/nav";
import { renderRich } from "./dump/render";
import { disposeTables } from "./dump/table";
import type { Annotation, RichValue } from "./protocol";

export type Stream = "stdout" | "stderr" | "info";

type Pending =
  | { stream: Stream; text: string }
  | { rich: RichValue }
  | { annotation: Annotation; onClick: () => void };

const MAX_LINES = 20_000;
const BOTTOM_SLACK_PX = 4;

export class OutputPane {
  readonly element: HTMLElement;
  private readonly body: HTMLElement;
  private readonly jump: HTMLButtonElement;
  private readonly trimmedNotice: HTMLElement;

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

    this.element.append(this.trimmedNotice, this.body, this.jump);

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

  append(stream: Stream, text: string): void {
    if (!text) return;
    this.pending.push({ stream, text });
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

    const emit = () => {
      if (runStream === null || !run) return;
      if (runStream === lastStream && last && fragment.childNodes.length === 0) {
        last.append(run);
      } else {
        const span = document.createElement("span");
        span.className = `out-${runStream}`;
        span.dataset.stream = runStream;
        span.textContent = run;
        fragment.append(span);
        last = span;
        lastStream = runStream;
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
      if (item.stream !== runStream) {
        emit();
        runStream = item.stream;
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
