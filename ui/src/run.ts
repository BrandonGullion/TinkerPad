// Starting and cancelling runs, and routing their events to the tab they
// belong to.
//
// Each playground has its own run, output, and state; runs in different tabs
// overlap. The core scopes events by run id, so this keeps a run id → playground
// map, and buffers events for ids it hasn't learned yet (they can arrive before
// the `run` command returns).

import { invoke } from "./ipc";
import type { RunState } from "./layout";
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

/** What runs need from the tabs. */
export interface RunHost {
  /** The playground whose tab is active, if any. */
  activePlayground(): string | null;
  /** The output pane of a playground's tab, if it's open. */
  outputFor(playground: string): OutputPane | null;
  /** A run is about to save `code` as the playground's source. */
  beforeRun(playground: string, code: string): void;
  /** A playground's run state changed. */
  runStateChanged(playground: string, state: RunState): void;
  /** Tell the user something briefly (no playground open, for example). */
  flashNote(text: string): void;
}

interface PlaygroundRun {
  /** Id of the run whose events are shown. `null` while starting or idle. */
  current: number | null;
  /** Identifies the latest start, so a superseded start can tell. */
  token: number;
  state: RunState;
}

export class RunController {
  private readonly runs = new Map<string, PlaygroundRun>();
  private readonly runPlayground = new Map<number, string>();
  private readonly early = new Map<number, RunEvent[]>();
  private startsInFlight = 0;
  private latestToken = 0;
  private waiters: ((o: RunOutcome) => void)[] = [];
  private clickedAt = 0;
  private firstOutputAt: number | null = null;

  constructor(private readonly host: RunHost) {}

  /** Whether the active tab's playground is running or starting. */
  get running(): boolean {
    const active = this.host.activePlayground();
    return active !== null && this.isRunning(active);
  }

  isRunning(playground: string): boolean {
    return this.stateOf(playground).kind === "running";
  }

  stateOf(playground: string): RunState {
    return this.runs.get(playground)?.state ?? { kind: "idle" };
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
      this.host.flashNote(NO_PLAYGROUND_NOTE);
      return;
    }
    const record = this.record(playground);
    const token = ++this.latestToken;
    record.token = token;
    record.current = null;
    this.clickedAt = performance.now();
    this.firstOutputAt = null;
    if (CLEAR_OUTPUT_ON_RUN) this.host.outputFor(playground)?.clear();
    this.host.beforeRun(playground, code);
    this.setState(playground, record, { kind: "running", since: this.clickedAt });

    this.startsInFlight++;
    let id: number;
    try {
      id = await invoke<number>("run", { playground, text: code });
    } catch (e) {
      this.startsInFlight--;
      if (this.startsInFlight === 0) this.early.clear();
      const message = (e as { message?: string }).message ?? String(e);
      if (record.token === token) this.finish(playground, record, { kind: "failed", message });
      return;
    }
    this.startsInFlight--;
    this.runPlayground.set(id, playground);
    const early = this.early.get(id) ?? [];
    this.early.delete(id);
    if (this.startsInFlight === 0) this.early.clear();
    // A newer start for the same playground replaces this one; its events
    // are dropped.
    if (record.token !== token) return;
    record.current = id;
    for (const event of early) this.apply(playground, record, event);
  }

  /** Cancel a playground's run (the active tab's by default). */
  async cancel(playground: string | null = this.host.activePlayground()): Promise<void> {
    if (playground && this.isRunning(playground)) await invoke("cancel", { playground });
  }

  /** Resolves when the latest run started with `run` ends. */
  waitForEnd(): Promise<RunOutcome> {
    return new Promise((resolve) => this.waiters.push(resolve));
  }

  /** A playground's tab closed: forget its run. */
  forget(playground: string): void {
    this.runs.delete(playground);
  }

  handle(env: Envelope): void {
    if (env.event.type !== "run" || !env.scope.startsWith("run:")) return;
    const id = Number(env.scope.slice(4));
    const { type: _type, ...event } = env.event;
    const runEvent = event as RunEvent;

    const playground = this.runPlayground.get(id);
    if (playground === undefined) {
      if (this.startsInFlight > 0) {
        const list = this.early.get(id) ?? [];
        list.push(runEvent);
        this.early.set(id, list);
      }
      return;
    }
    const record = this.runs.get(playground);
    if (!record || record.current !== id) return;
    this.apply(playground, record, runEvent);
  }

  private record(playground: string): PlaygroundRun {
    let record = this.runs.get(playground);
    if (!record) {
      record = { current: null, token: 0, state: { kind: "idle" } };
      this.runs.set(playground, record);
    }
    return record;
  }

  private setState(playground: string, record: PlaygroundRun, state: RunState): void {
    record.state = state;
    this.host.runStateChanged(playground, state);
  }

  private apply(playground: string, record: PlaygroundRun, event: RunEvent): void {
    const output = this.host.outputFor(playground);
    const latest = record.token === this.latestToken;
    switch (event.kind) {
      case "started":
        break;
      case "stdout":
      case "stderr":
        if (latest) this.firstOutputAt ??= performance.now();
        output?.append(event.kind, event.text);
        break;
      case "rich":
        if (latest) this.firstOutputAt ??= performance.now();
        output?.appendRich(event.value);
        break;
      case "warning":
        output?.append("info", `⚠ ${event.message}\n`);
        break;
      case "exited":
        this.finish(playground, record, { kind: "exited", code: event.code, durationMs: event.durationMs });
        break;
      case "cancelled":
        this.finish(playground, record, { kind: "cancelled", durationMs: event.durationMs });
        break;
      case "failed":
        this.finish(playground, record, { kind: "failed", message: event.message });
        break;
    }
  }

  private finish(playground: string, record: PlaygroundRun, outcome: RunOutcome): void {
    record.current = null;
    const output = this.host.outputFor(playground);
    switch (outcome.kind) {
      case "exited":
        this.setState(playground, record, { kind: "done", code: outcome.code, ms: outcome.durationMs });
        break;
      case "cancelled":
        output?.append("info", "\n[cancelled]\n");
        this.setState(playground, record, { kind: "cancelled", ms: outcome.durationMs });
        break;
      case "failed":
        output?.append("stderr", `${outcome.message}\n`);
        this.setState(playground, record, { kind: "failed" });
        break;
    }
    if (record.token !== this.latestToken) return;
    const waiters = this.waiters;
    this.waiters = [];
    for (const w of waiters) w(outcome);
  }
}
