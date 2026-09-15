// Starting and cancelling runs, and routing their events to the output pane
// and status bar.

import { invoke } from "./ipc";
import type { StatusBar } from "./layout";
import type { OutputPane } from "./output";
import type { Envelope, RunEvent } from "./protocol";

export type RunOutcome =
  | { kind: "exited"; code: number | null; durationMs: number }
  | { kind: "cancelled"; durationMs: number }
  | { kind: "failed"; message: string };

/** Setting stub: clear the output pane when a run starts. Becomes a real
 * setting in work item 04-02. */
const CLEAR_OUTPUT_ON_RUN = true;

export const NO_PLAYGROUND_NOTE = "Open a playground to run it.";

export class RunController {
  /** Id of the run whose events we display. `null` while a start is pending. */
  private current: number | null = null;
  /** Events for runs whose id we haven't learned yet (they can arrive before
   * the `run` command returns). */
  private early = new Map<number, RunEvent[]>();
  private pending = false;
  private clickedAt = 0;
  private firstOutputAt: number | null = null;
  private waiters: ((o: RunOutcome) => void)[] = [];
  /** The playground of the latest run. */
  private playground: string | null = null;

  constructor(
    private readonly output: OutputPane,
    private readonly status: StatusBar,
  ) {}

  get running(): boolean {
    return this.pending || this.current !== null;
  }

  /** Milliseconds from Run to the first output of the latest run, if any. */
  get firstOutputMs(): number | null {
    return this.firstOutputAt === null ? null : this.firstOutputAt - this.clickedAt;
  }

  /**
   * Save `code` to `playground` and run it. With no playground open there is
   * nothing to run, so the status bar says so instead.
   */
  async run(playground: string | null, code: string): Promise<void> {
    if (!playground) {
      this.status.flashNote(NO_PLAYGROUND_NOTE);
      return;
    }
    this.clickedAt = performance.now();
    this.firstOutputAt = null;
    this.current = null;
    this.pending = true;
    this.playground = playground;
    if (CLEAR_OUTPUT_ON_RUN) this.output.clear();
    this.status.setRun({ kind: "running", since: this.clickedAt });

    let id: number;
    try {
      id = await invoke<number>("run", { playground, text: code });
    } catch (e) {
      this.pending = false;
      const message = (e as { message?: string }).message ?? String(e);
      this.finish({ kind: "failed", message });
      return;
    }
    this.pending = false;
    this.current = id;

    // Replay anything that arrived early for this run; drop the rest (they
    // belong to runs that were superseded).
    const early = this.early.get(id) ?? [];
    this.early.clear();
    for (const event of early) this.apply(event);
  }

  async cancel(): Promise<void> {
    if (this.running && this.playground) await invoke("cancel", { playground: this.playground });
  }

  /** Resolves when the current (or next) run ends. */
  waitForEnd(): Promise<RunOutcome> {
    return new Promise((resolve) => this.waiters.push(resolve));
  }

  handle(env: Envelope): void {
    if (env.event.type !== "run" || !env.scope.startsWith("run:")) return;
    const id = Number(env.scope.slice(4));
    const { type: _type, ...event } = env.event;
    const runEvent = event as RunEvent;

    if (this.pending) {
      const list = this.early.get(id) ?? [];
      list.push(runEvent);
      this.early.set(id, list);
      return;
    }
    if (id !== this.current) return;
    this.apply(runEvent);
  }

  private apply(event: RunEvent): void {
    switch (event.kind) {
      case "started":
        break;
      case "stdout":
      case "stderr":
        this.firstOutputAt ??= performance.now();
        this.output.append(event.kind, event.text);
        break;
      case "rich":
        this.firstOutputAt ??= performance.now();
        this.output.appendRich(event.value);
        break;
      case "warning":
        this.output.append("info", `⚠ ${event.message}\n`);
        break;
      case "exited":
        this.finish({ kind: "exited", code: event.code, durationMs: event.durationMs });
        break;
      case "cancelled":
        this.finish({ kind: "cancelled", durationMs: event.durationMs });
        break;
      case "failed":
        this.finish({ kind: "failed", message: event.message });
        break;
    }
  }

  private finish(outcome: RunOutcome): void {
    this.current = null;
    switch (outcome.kind) {
      case "exited":
        this.status.setRun({ kind: "done", code: outcome.code, ms: outcome.durationMs });
        break;
      case "cancelled":
        this.output.append("info", "\n[cancelled]\n");
        this.status.setRun({ kind: "cancelled", ms: outcome.durationMs });
        break;
      case "failed":
        this.output.append("stderr", `${outcome.message}\n`);
        this.status.setRun({ kind: "failed" });
        break;
    }
    const waiters = this.waiters;
    this.waiters = [];
    for (const w of waiters) w(outcome);
  }
}
