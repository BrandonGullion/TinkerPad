// Starting and cancelling runs, and routing their events to the tab they
// belong to.
//
// Each playground has its own run, output, and state; runs in different tabs
// overlap. The core scopes events by run id, so this keeps a run id → playground
// map, and buffers events for ids it hasn't learned yet (they can arrive before
// the `run` command returns).

import { invoke } from "./ipc";
import { clearOutputOnRun } from "./settings";
import type { RunState } from "./layout";
import type { OutputPane } from "./output";
import type { Annotation, Envelope, RunEvent } from "./protocol";

/**
 * A finished run this window still has the output of (work item 03-08). The
 * events, replayed, are exactly what the pane showed; nothing is persisted, so
 * a restart leaves the run in `runs.jsonl` with its output gone.
 */
export interface RetainedRun {
  /** The core's run id, which is what `runs.jsonl` records it under. */
  id: number;
  outcome: RunOutcome;
  events: RunEvent[];
  /** Whether the output was longer than this window keeps. */
  clipped: boolean;
}

/** How many runs of a playground keep their output. */
const KEEP_OUTPUTS = 5;

/**
 * How many events one of them keeps. A run that prints a hundred thousand
 * lines is not one whose output is worth holding five copies of; past this it
 * is remembered as having happened, and the pane says so when it is shown.
 */
const KEEP_EVENTS = 5000;

export type RunOutcome =
  | { kind: "exited"; code: number | null; durationMs: number }
  | { kind: "cancelled"; durationMs: number }
  | { kind: "failed"; message: string };



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
  /** The output pointed at a position; mark it in the tab if it belongs there. */
  annotation(playground: string, annotation: Annotation): void;
  /** Go to an annotation the user clicked in the output. */
  revealAnnotation(playground: string, annotation: Annotation): void;
}

interface PlaygroundRun {
  /** Id of the run whose events are shown. `null` while starting or idle. */
  current: number | null;
  /** Identifies the latest start, so a superseded start can tell. */
  token: number;
  /**
   * Cancel was asked for before the start returned a run id. Sent once it
   * does: a cancel that reached the core first would find no run to stop.
   */
  cancelPending: boolean;
  state: RunState;
  /** The events of the run going on now, for when it becomes a past one. */
  events: RunEvent[];
  /** Whether that list stopped growing at `KEEP_EVENTS`. */
  clipped: boolean;
  /** Finished runs whose output is still here, newest first. */
  past: RetainedRun[];
  /** How long the run before the current one took, for the comparison in the
   * status bar. */
  previousMs: number | null;
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

  /**
   * Called when a run failed before it ran — a missing toolchain, a command
   * that couldn't start. Not for a program that ran and exited non-zero: that
   * is the program's business, not the app's (work item 04-08b).
   */
  onFailed: (playground: string, message: string) => void = () => {};

  constructor(private readonly host: RunHost) {}

  /** Whether the active tab's playground is running or starting. */
  get running(): boolean {
    const active = this.host.activePlayground();
    return active !== null && this.isRunning(active);
  }

  /** Running, or waiting for a helper to be free — either way, cancellable. */
  isRunning(playground: string): boolean {
    const kind = this.stateOf(playground).kind;
    return kind === "running" || kind === "waiting";
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
    record.cancelPending = false;
    record.events = [];
    record.clipped = false;
    this.clickedAt = performance.now();
    this.firstOutputAt = null;
    if (clearOutputOnRun()) this.host.outputFor(playground)?.clear();
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
    this.cancelIfPending(playground, record);
  }

  /**
   * Stream a command started by `begin` into the playground's output pane as
   * if it were a run: adding a package, for example. `begin` invokes the
   * command and returns its run id, or `null` when there was nothing to run.
   * Events that arrive before it returns are buffered, as for a run.
   */
  async adopt(playground: string, begin: () => Promise<number | null>): Promise<void> {
    const record = this.record(playground);
    const token = ++this.latestToken;
    record.token = token;
    record.current = null;
    record.cancelPending = false;
    this.setState(playground, record, { kind: "running", since: performance.now() });

    this.startsInFlight++;
    let id: number | null;
    try {
      id = await begin();
    } catch (e) {
      this.startsInFlight--;
      if (this.startsInFlight === 0) this.early.clear();
      const message = (e as { message?: string }).message ?? String(e);
      if (record.token === token) this.finish(playground, record, { kind: "failed", message });
      throw e;
    }
    this.startsInFlight--;
    const early = id === null ? [] : (this.early.get(id) ?? []);
    if (id !== null) {
      this.runPlayground.set(id, playground);
      this.early.delete(id);
    }
    if (this.startsInFlight === 0) this.early.clear();
    if (record.token !== token) return;
    if (id === null) {
      // Nothing ran: the plugin adds packages by editing the file.
      this.setState(playground, record, { kind: "idle" });
      return;
    }
    record.current = id;
    for (const event of early) this.apply(playground, record, event);
    this.cancelIfPending(playground, record);
  }

  /** Cancel a playground's run (the active tab's by default). */
  async cancel(playground: string | null = this.host.activePlayground()): Promise<void> {
    if (!playground || !this.isRunning(playground)) return;
    const record = this.runs.get(playground);
    // Still starting: the core may not know about this run yet.
    if (record && record.current === null) {
      record.cancelPending = true;
      return;
    }
    await invoke("cancel", { playground });
  }

  /** Send a cancel that was asked for while the run was starting. */
  private cancelIfPending(playground: string, record: PlaygroundRun): void {
    if (!record.cancelPending) return;
    record.cancelPending = false;
    void invoke("cancel", { playground });
  }

  /** Resolves when the latest run started with `run` ends. */
  waitForEnd(): Promise<RunOutcome> {
    return new Promise((resolve) => this.waiters.push(resolve));
  }

  /**
   * The finished runs of a playground whose output this window still has,
   * newest first (work item 03-08).
   */
  retained(playground: string): RetainedRun[] {
    return this.runs.get(playground)?.past ?? [];
  }

  /**
   * Show a retained run's output in the playground's pane again. The events
   * are replayed into it exactly as they arrived, so a dump is a dump and an
   * annotation is still clickable.
   */
  replay(playground: string, run: RetainedRun): void {
    const output = this.host.outputFor(playground);
    if (!output) return;
    output.clear();
    for (const event of run.events) {
      switch (event.kind) {
        case "stdout":
        case "stderr":
          output.append(event.kind, event.text);
          break;
        case "rich":
          output.appendRich(event.value);
          break;
        case "warning":
          output.append("info", `⚠ ${event.message}\n`);
          break;
        case "annotation": {
          const { kind: _kind, ...annotation } = event;
          output.markAnnotation(annotation, () =>
            this.host.revealAnnotation(playground, annotation),
          );
          break;
        }
        case "failed":
          output.append("stderr", `${event.message}\n`);
          break;
        case "cancelled":
          output.append("info", "\n[cancelled]\n");
          break;
      }
    }
    if (run.clipped) {
      output.append("info", "\n[the rest of this run's output wasn't kept]\n");
    }
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
      record = {
        current: null,
        token: 0,
        cancelPending: false,
        state: { kind: "idle" },
        events: [],
        clipped: false,
        past: [],
        previousMs: null,
      };
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
    if (record.events.length < KEEP_EVENTS) record.events.push(event);
    else record.clipped = true;
    switch (event.kind) {
      case "started": {
        // The run was already showing as running (optimistically, from the
        // click); this is where it learns a helper is doing it.
        const since = record.state.kind === "running" || record.state.kind === "waiting"
          ? record.state.since
          : performance.now();
        this.setState(playground, record, { kind: "running", since, warm: event.warm });
        // A run that can be typed at gets the input row (work item 04-05).
        if (event.stdin && output) this.openInput(playground, output);
        break;
      }
      case "waiting": {
        const since = record.state.kind === "running" ? record.state.since : performance.now();
        this.setState(playground, record, { kind: "waiting", message: event.message, since });
        break;
      }
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
      case "annotation": {
        const { kind: _kind, ...annotation } = event;
        output?.markAnnotation(annotation, () => this.host.revealAnnotation(playground, annotation));
        this.host.annotation(playground, annotation);
        break;
      }
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

  /** Wire the pane's input row to this playground's run (work item 04-05). */
  private openInput(playground: string, output: OutputPane): void {
    output.onInput = (text) => {
      void invoke("send_input", { playground, text }).catch(() => {});
    };
    output.onInputEnd = () => {
      void invoke("close_input", { playground }).catch(() => {});
    };
    output.setInput(true);
  }

  private finish(playground: string, record: PlaygroundRun, outcome: RunOutcome): void {
    const id = record.current;
    record.current = null;
    const output = this.host.outputFor(playground);
    // Nothing to type at any more.
    output?.setInput(false);
    const previousMs = record.previousMs;
    switch (outcome.kind) {
      case "exited":
        this.setState(playground, record, {
          kind: "done",
          code: outcome.code,
          ms: outcome.durationMs,
          previousMs,
        });
        break;
      case "cancelled":
        output?.append("info", "\n[cancelled]\n");
        this.setState(playground, record, { kind: "cancelled", ms: outcome.durationMs });
        break;
      case "failed":
        output?.append("stderr", `${outcome.message}\n`);
        this.setState(playground, record, { kind: "failed" });
        this.onFailed(playground, outcome.message);
        break;
    }
    if (id !== null) {
      record.past.unshift({
        id,
        outcome,
        events: record.events,
        clipped: record.clipped,
      });
      record.past.length = Math.min(record.past.length, KEEP_OUTPUTS);
      record.events = [];
      record.clipped = false;
    }
    if (outcome.kind !== "failed") record.previousMs = outcome.durationMs;

    if (record.token !== this.latestToken) return;
    const waiters = this.waiters;
    this.waiters = [];
    for (const w of waiters) w(outcome);
  }
}
