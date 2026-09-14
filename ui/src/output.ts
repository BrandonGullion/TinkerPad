// The output pane: a fast, append-only stream of stdout and stderr text.
//
// Appends are batched per animation frame and merged into one text node per
// stream run, so thousands of tiny chunks cost a handful of DOM operations.
// The DOM is capped at MAX_LINES; older lines are dropped with a notice.

export type Stream = "stdout" | "stderr" | "info";

const MAX_LINES = 20_000;
const BOTTOM_SLACK_PX = 4;

export class OutputPane {
  readonly element: HTMLElement;
  private readonly body: HTMLElement;
  private readonly jump: HTMLButtonElement;
  private readonly trimmedNotice: HTMLElement;

  private pending: { stream: Stream; text: string }[] = [];
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

    this.body = document.createElement("pre");
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
      }
    });
  }

  append(stream: Stream, text: string): void {
    if (!text) return;
    this.pending.push({ stream, text });
    this.frame ||= requestAnimationFrame(() => this.flush());
  }

  clear(): void {
    cancelAnimationFrame(this.frame);
    this.frame = 0;
    this.pending = [];
    this.body.replaceChildren();
    this.lines = 0;
    this.trimmed = 0;
    this.trimmedNotice.hidden = true;
    this.stick = true;
    this.jump.hidden = true;
  }

  /** Resolves after pending appends have reached the DOM. */
  flushed(): Promise<void> {
    return new Promise((resolve) =>
      requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
    );
  }

  private flush(): void {
    this.frame = 0;
    const batch = this.pending;
    this.pending = [];
    if (batch.length === 0) return;

    // Merge consecutive chunks of the same stream, then extend the last span
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

    for (const { stream, text } of batch) {
      if (stream !== runStream) {
        emit();
        runStream = stream;
      }
      run += text;
      this.lines += countNewlines(text);
    }
    emit();
    this.body.append(fragment);

    this.trim();
    if (this.stick) this.scrollToBottom();
  }

  private trim(): void {
    while (this.lines > MAX_LINES && this.body.firstChild) {
      const first = this.body.firstChild as HTMLElement;
      const text = first.textContent ?? "";
      const n = countNewlines(text);
      const excess = this.lines - MAX_LINES;
      if (n <= excess) {
        first.remove();
        this.lines -= n;
        this.trimmed += n;
        if (n === 0) break;
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

function countNewlines(s: string): number {
  let n = 0;
  for (let i = s.indexOf("\n"); i !== -1; i = s.indexOf("\n", i + 1)) n++;
  return n;
}
