// The run history of the active playground (work item 03-08).
//
// Clicking the run indicator in the status bar opens a list of recent runs,
// newest first, read from the playground's `runs.jsonl`. Runs this window
// still has the output of can be shown again; older ones are their numbers and
// nothing more, which is honest — the outputs were never written down.

import { invoke } from "./ipc";
import { formatMs } from "./layout";
import type { RetainedRun, RunController } from "./run";
import type { RunRecord } from "./protocol";

/** How many runs the list shows. */
const SHOWN = 20;

/** What the history needs from the rest of the window. */
export interface HistoryHost {
  /** The playground whose tab is active, if any. */
  activePlayground(): string | null;
  /** Its name, for the title. */
  activeName(): string | null;
}

export class RunHistory {
  private popover: HTMLElement | null = null;
  private anchor: HTMLElement | null = null;

  constructor(
    private readonly host: HistoryHost,
    private readonly runs: RunController,
  ) {
    document.addEventListener("mousedown", (e) => {
      if (!this.popover) return;
      const target = e.target as Node;
      if (this.popover.contains(target) || this.anchor?.contains(target)) return;
      this.close();
    });
    document.addEventListener("keydown", (e) => {
      if (e.key === "Escape" && this.popover) {
        e.preventDefault();
        this.close();
        this.anchor?.focus();
      }
    });
  }

  get open(): boolean {
    return this.popover !== null;
  }

  close(): void {
    this.popover?.remove();
    this.popover = null;
    this.anchor = null;
  }

  /** Open the list under `anchor`, or close it if it is already that one's. */
  async toggle(anchor: HTMLElement): Promise<void> {
    if (this.popover && this.anchor === anchor) {
      this.close();
      return;
    }
    this.close();
    const playground = this.host.activePlayground();
    if (!playground) return;

    const records = await invoke<RunRecord[]>("run_history", {
      playground,
      limit: SHOWN,
    }).catch(() => [] as RunRecord[]);
    // The tab may have changed while that was in flight.
    if (this.host.activePlayground() !== playground) return;

    const retained = new Map(this.runs.retained(playground).map((r) => [r.id, r]));
    this.popover = this.render(playground, records, retained);
    this.anchor = anchor;
    document.body.append(this.popover);
    this.place(anchor);
    this.popover.querySelector<HTMLElement>(".history-row")?.focus();
  }

  private render(
    playground: string,
    records: RunRecord[],
    retained: Map<number, RetainedRun>,
  ): HTMLElement {
    const popover = document.createElement("div");
    popover.className = "history-popover";
    popover.setAttribute("role", "dialog");
    popover.setAttribute("aria-label", "Recent runs");

    const title = document.createElement("div");
    title.className = "history-title";
    title.textContent = this.host.activeName() ?? "Recent runs";
    popover.append(title);

    if (records.length === 0) {
      const empty = document.createElement("div");
      empty.className = "history-empty";
      empty.textContent = "No runs yet.";
      popover.append(empty);
      return popover;
    }

    const list = document.createElement("div");
    list.className = "history-list";
    for (const record of records) {
      list.append(this.row(playground, record, retained.get(record.run)));
    }
    popover.append(list);
    return popover;
  }

  private row(
    playground: string,
    record: RunRecord,
    kept: RetainedRun | undefined,
  ): HTMLElement {
    const row = document.createElement(kept ? "button" : "div");
    row.className = "history-row";
    if (kept) {
      row.title = "Show this run's output again";
      row.addEventListener("click", () => {
        this.runs.replay(playground, kept);
        this.close();
      });
    } else {
      row.title = "This run's output is no longer in memory";
      row.classList.add("history-gone");
    }

    const when = document.createElement("span");
    when.className = "history-when";
    when.textContent = clock(record.startedMs);

    const outcome = document.createElement("span");
    // A run that exited non-zero ended badly, whatever the outcome says.
    const went = record.outcome === "exited" && record.code !== 0 ? "failed" : record.outcome;
    outcome.className = `history-outcome history-${went}`;
    outcome.textContent = label(record);

    const took = document.createElement("span");
    took.className = "history-took";
    took.textContent = formatMs(record.durationMs);
    took.title = detail(record);

    const how = document.createElement("span");
    how.className = "history-runner";
    how.textContent = record.runner === "helper" ? "warm" : "";

    row.append(when, outcome, took, how);
    if (record.error) {
      const error = document.createElement("span");
      error.className = "history-error";
      error.textContent = record.error;
      row.append(error);
    }
    return row;
  }

  private place(anchor: HTMLElement): void {
    const popover = this.popover;
    if (!popover) return;
    const box = anchor.getBoundingClientRect();
    const width = popover.offsetWidth;
    popover.style.left = `${Math.max(8, Math.min(box.right - width, window.innerWidth - width - 8))}px`;
    popover.style.bottom = `${Math.max(8, window.innerHeight - box.top + 6)}px`;
  }
}

/** `14:02:11`, in the user's own clock. */
function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

function label(record: RunRecord): string {
  switch (record.outcome) {
    case "exited":
      return `Exit ${record.code ?? "?"}`;
    case "cancelled":
      return "Cancelled";
    case "failed":
      return "Failed";
  }
}

/** The breakdown, for the hover: where the time went. */
function detail(record: RunRecord): string {
  const parts = [`${formatMs(record.durationMs)} in all`];
  if (record.firstOutputMs !== undefined) {
    parts.push(`first output after ${formatMs(record.firstOutputMs)}`);
  }
  if (record.reportedMs !== undefined) {
    const overhead = Math.max(0, record.durationMs - record.reportedMs);
    const what = record.runner === "helper" ? "the helper" : "the process";
    parts.push(`${formatMs(record.reportedMs)} in ${what}, ${formatMs(overhead)} around it`);
  }
  return parts.join(" · ");
}
