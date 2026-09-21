// Editor-over-panel layout with a draggable splitter, and the status bar.

import { ServerStrip } from "./servers";

const SPLIT_KEY = "tinkerpad.split";
const SIDE_KEY = "tinkerpad.sidebar";
const MIN_PX = 80;
const SIDE_MIN_PX = 180;
const SIDE_MAX_PX = 560;

function load(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function save(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Storage unavailable; the split just won't persist.
  }
}

export interface Layout {
  /** Where the library panel mounts, to the left of everything (04-01). */
  sidebar: Sidebar;
  /** The playground tabs. */
  tabStrip: HTMLElement;
  /** A bar shown above the editor when a tab needs a decision. */
  conflictBar: HTMLElement;
  /** Where CodeMirror mounts. */
  editorHost: HTMLElement;
  /** Shown over the editor when no tab is open. */
  emptyState: HTMLElement;
  panelHost: HTMLElement;
  statusBar: StatusBar;
}

/** The left-hand panel: a host to mount in, and whether it is showing. */
export interface Sidebar {
  host: HTMLElement;
  visible(): boolean;
  setVisible(visible: boolean): void;
}

export function createLayout(root: HTMLElement): Layout {
  const shell = document.createElement("div");
  shell.className = "shell";
  const sidebarHost = document.createElement("div");
  sidebarHost.className = "sidebar";
  sidebarHost.hidden = true;
  const sideSplitter = document.createElement("div");
  sideSplitter.className = "side-splitter";
  sideSplitter.setAttribute("role", "separator");
  sideSplitter.setAttribute("aria-orientation", "vertical");
  sideSplitter.hidden = true;

  const workspace = document.createElement("main");
  workspace.className = "workspace";

  const editorPane = document.createElement("div");
  editorPane.className = "editor-pane";
  const tabStrip = document.createElement("div");
  tabStrip.className = "tab-strip";
  tabStrip.setAttribute("role", "tablist");
  const conflictBar = document.createElement("div");
  conflictBar.className = "conflict-bar";
  conflictBar.setAttribute("role", "alert");
  conflictBar.hidden = true;
  const editorHost = document.createElement("div");
  editorHost.className = "editor-host";
  const emptyState = document.createElement("div");
  emptyState.className = "empty-state";
  emptyState.hidden = true;
  editorHost.append(emptyState);
  editorPane.append(tabStrip, conflictBar, editorHost);

  const splitter = document.createElement("div");
  splitter.className = "splitter";
  splitter.setAttribute("role", "separator");
  splitter.setAttribute("aria-orientation", "horizontal");

  const panelHost = document.createElement("div");
  panelHost.className = "panel-host";

  workspace.append(editorPane, splitter, panelHost);
  shell.append(sidebarHost, sideSplitter, workspace);
  const statusBar = new StatusBar();
  root.replaceChildren(shell, statusBar.element);

  // The sidebar's width, in pixels, because its contents don't stretch.
  let sideWidth = Math.min(
    Math.max(Number(load(SIDE_KEY)) || 280, SIDE_MIN_PX),
    SIDE_MAX_PX,
  );
  const applySide = () => {
    shell.style.gridTemplateColumns = sidebarHost.hidden
      ? "1fr"
      : `${sideWidth}px ${sideSplitter.offsetWidth || 5}px 1fr`;
  };
  const sidebar: Sidebar = {
    host: sidebarHost,
    visible: () => !sidebarHost.hidden,
    setVisible(visible) {
      sidebarHost.hidden = !visible;
      sideSplitter.hidden = !visible;
      applySide();
    },
  };
  applySide();

  sideSplitter.addEventListener("pointerdown", (down) => {
    down.preventDefault();
    sideSplitter.setPointerCapture(down.pointerId);
    sideSplitter.classList.add("dragging");
    const left = shell.getBoundingClientRect().left;
    const move = (e: PointerEvent) => {
      sideWidth = Math.min(Math.max(e.clientX - left, SIDE_MIN_PX), SIDE_MAX_PX);
      applySide();
    };
    const up = () => {
      sideSplitter.removeEventListener("pointermove", move);
      sideSplitter.classList.remove("dragging");
      save(SIDE_KEY, String(Math.round(sideWidth)));
    };
    sideSplitter.addEventListener("pointermove", move);
    sideSplitter.addEventListener("pointerup", up, { once: true });
  });

  // The split is stored as the editor's share of the workspace height.
  let ratio = Number(load(SPLIT_KEY)) || 0.62;
  const apply = () => {
    const total = workspace.clientHeight - splitter.offsetHeight;
    const editorPx = Math.min(Math.max(total * ratio, MIN_PX), total - MIN_PX);
    workspace.style.gridTemplateRows = `${editorPx}px ${splitter.offsetHeight}px 1fr`;
  };
  apply();
  new ResizeObserver(apply).observe(workspace);

  splitter.addEventListener("pointerdown", (down) => {
    down.preventDefault();
    splitter.setPointerCapture(down.pointerId);
    splitter.classList.add("dragging");
    const top = workspace.getBoundingClientRect().top;
    const move = (e: PointerEvent) => {
      const total = workspace.clientHeight - splitter.offsetHeight;
      ratio = Math.min(Math.max((e.clientY - top) / total, 0), 1);
      apply();
    };
    const up = () => {
      splitter.removeEventListener("pointermove", move);
      splitter.classList.remove("dragging");
      save(SPLIT_KEY, ratio.toFixed(4));
    };
    splitter.addEventListener("pointermove", move);
    splitter.addEventListener("pointerup", up, { once: true });
  });

  return { sidebar, tabStrip, conflictBar, editorHost, emptyState, panelHost, statusBar };
}

export type RunState =
  | { kind: "idle" }
  /** Waiting for a plugin helper: being built, or busy with another
   * playground's run (work item 03-03). */
  | { kind: "waiting"; message: string; since: number }
  /** `warm` when a plugin helper is running it. */
  | { kind: "running"; since: number; warm?: boolean }
  /** `previousMs` is what the run before this one took, for the comparison
   * beside the time (work item 03-08). */
  | { kind: "done"; code: number | null; ms: number; previousMs?: number | null }
  | { kind: "cancelled"; ms: number }
  | { kind: "failed" };

export class StatusBar {
  readonly element: HTMLElement;
  /** The language servers, with their own refresh (work item 02-08). */
  readonly servers = new ServerStrip();
  private readonly language: HTMLElement;
  private readonly note: HTMLElement;
  private readonly run: HTMLButtonElement;
  private readonly runLabel: HTMLElement;
  private readonly delta: HTMLElement;
  private readonly cancelButton: HTMLButtonElement;
  private timer = 0;
  private noteTimer = 0;
  private stickyNote: string | null = null;

  /** Called when the user clicks Cancel while a run is in progress. */
  onCancel: () => void = () => {};
  /** Called when the user clicks Logs. */
  onOpenLogs: () => void = () => {};
  /** Called when the user clicks the run indicator: the run history (03-08). */
  onShowHistory: (anchor: HTMLElement) => void = () => {};

  constructor() {
    this.element = document.createElement("footer");
    this.element.className = "statusbar";
    this.language = document.createElement("span");
    this.language.className = "status-language";
    this.language.textContent = "Plain text";
    this.note = document.createElement("span");
    this.note.className = "status-note";
    this.note.hidden = true;
    const spacer = document.createElement("span");
    spacer.className = "status-spacer";
    // A button, because it opens the history of this playground's runs; it
    // still reads as the run indicator it has always been.
    this.run = document.createElement("button");
    this.run.className = "status-run";
    this.run.title = "Recent runs of this playground";
    this.run.addEventListener("click", () => this.onShowHistory(this.run));
    this.runLabel = document.createElement("span");
    this.runLabel.className = "status-run-label";
    this.delta = document.createElement("span");
    this.delta.className = "status-delta";
    this.delta.hidden = true;
    this.run.append(this.runLabel, this.delta);
    this.cancelButton = document.createElement("button");
    this.cancelButton.className = "status-cancel";
    this.cancelButton.textContent = "Cancel";
    this.cancelButton.title = "Cancel run (Mod+.)";
    this.cancelButton.hidden = true;
    this.cancelButton.addEventListener("click", () => this.onCancel());
    const logs = document.createElement("button");
    logs.className = "status-link";
    logs.textContent = "Logs";
    logs.title = "Open the log folder";
    logs.addEventListener("click", () => this.onOpenLogs());
    this.element.append(
      this.language,
      this.note,
      spacer,
      this.servers.element,
      this.run,
      this.cancelButton,
      logs,
    );
    this.setRun({ kind: "idle" });
  }

  setLanguage(name: string): void {
    this.language.textContent = name;
  }

  /**
   * A note beside the language. A sticky note stays until replaced (pass
   * `null` to clear); a flash shows for a moment, then the sticky note returns.
   */
  setNote(text: string | null): void {
    this.stickyNote = text;
    clearTimeout(this.noteTimer);
    this.showNote(text);
  }

  flashNote(text: string, ms = 2500): void {
    clearTimeout(this.noteTimer);
    this.showNote(text);
    this.noteTimer = window.setTimeout(() => this.showNote(this.stickyNote), ms);
  }

  private showNote(text: string | null): void {
    this.note.textContent = text ?? "";
    this.note.hidden = !text;
  }

  /** Update the run indicator. While running, elapsed time ticks at ~10 Hz. */
  setRun(state: RunState): void {
    clearInterval(this.timer);
    this.timer = 0;
    this.run.className = `status-run run-${state.kind}`;
    this.delta.hidden = true;
    this.delta.textContent = "";
    this.delta.title = "";
    this.cancelButton.hidden = state.kind !== "running" && state.kind !== "waiting";
    switch (state.kind) {
      case "idle":
        this.runLabel.textContent = "Ready";
        break;
      case "waiting": {
        // Still the user's run, so the clock keeps going; it just isn't the
        // program's time yet.
        const tick = () => {
          this.runLabel.textContent = `${state.message} ${formatMs(performance.now() - state.since)}`;
        };
        tick();
        this.timer = window.setInterval(tick, 100);
        break;
      }
      case "running": {
        // "warm" is the only sign in the whole UI that a helper ran this
        // rather than a process started for it (work item 03-03).
        const warm = state.warm ? " · warm" : "";
        const tick = () => {
          this.runLabel.textContent = `Running ${formatMs(performance.now() - state.since)}${warm}`;
        };
        tick();
        this.timer = window.setInterval(tick, 100);
        break;
      }
      case "done":
        this.runLabel.textContent =
          state.code === 0
            ? `Exit 0 · ${formatMs(state.ms)}`
            : `Exit ${state.code ?? "?"} · ${formatMs(state.ms)}`;
        this.run.classList.toggle("run-error", state.code !== 0);
        this.showDelta(state.ms, state.previousMs ?? null);
        break;
      case "cancelled":
        this.runLabel.textContent = `Cancelled · ${formatMs(state.ms)}`;
        break;
      case "failed":
        this.runLabel.textContent = "Failed to start";
        break;
    }
  }

  /**
   * How this run compares with the one before it. Shown only when the
   * difference is big enough to mean something: machines are noisy, and an
   * arrow that flickers on every run is worse than none (work item 03-08).
   */
  private showDelta(ms: number, previousMs: number | null): void {
    if (previousMs === null || previousMs <= 0) return;
    const change = (ms - previousMs) / previousMs;
    if (Math.abs(change) < 0.15 || Math.abs(ms - previousMs) < 3) return;
    const slower = change > 0;
    this.delta.textContent = `${slower ? "▲" : "▼"} ${Math.abs(Math.round(change * 100))}%`;
    this.delta.title = `${slower ? "Slower" : "Faster"} than the previous run (${formatMs(previousMs)})`;
    this.delta.classList.toggle("delta-slower", slower);
    this.delta.classList.toggle("delta-faster", !slower);
    this.delta.hidden = false;
  }
}

export function formatMs(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`;
  const m = Math.floor(ms / 60_000);
  const s = Math.floor((ms % 60_000) / 1000);
  return `${m}m ${s.toString().padStart(2, "0")}s`;
}
