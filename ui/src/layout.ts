// Editor-over-panel layout with a draggable splitter, and the status bar.

const SPLIT_KEY = "tinkerpad.split";
const MIN_PX = 80;

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
  editorHost: HTMLElement;
  panelHost: HTMLElement;
  statusBar: StatusBar;
}

export function createLayout(root: HTMLElement): Layout {
  const workspace = document.createElement("main");
  workspace.className = "workspace";

  const editorHost = document.createElement("div");
  editorHost.className = "editor-host";

  const splitter = document.createElement("div");
  splitter.className = "splitter";
  splitter.setAttribute("role", "separator");
  splitter.setAttribute("aria-orientation", "horizontal");

  const panelHost = document.createElement("div");
  panelHost.className = "panel-host";

  workspace.append(editorHost, splitter, panelHost);
  const statusBar = new StatusBar();
  root.replaceChildren(workspace, statusBar.element);

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

  return { editorHost, panelHost, statusBar };
}

export type RunState =
  | { kind: "idle" }
  | { kind: "running"; since: number }
  | { kind: "done"; code: number | null; ms: number }
  | { kind: "cancelled"; ms: number }
  | { kind: "failed" };

export class StatusBar {
  readonly element: HTMLElement;
  private readonly language: HTMLElement;
  private readonly run: HTMLElement;
  private timer = 0;

  constructor() {
    this.element = document.createElement("footer");
    this.element.className = "statusbar";
    this.language = document.createElement("span");
    this.language.className = "status-language";
    this.language.textContent = "Plain text";
    const spacer = document.createElement("span");
    spacer.className = "status-spacer";
    this.run = document.createElement("span");
    this.run.className = "status-run";
    this.element.append(this.language, spacer, this.run);
    this.setRun({ kind: "idle" });
  }

  setLanguage(name: string): void {
    this.language.textContent = name;
  }

  /** Update the run indicator. While running, elapsed time ticks at ~10 Hz. */
  setRun(state: RunState): void {
    clearInterval(this.timer);
    this.timer = 0;
    this.run.className = `status-run run-${state.kind}`;
    switch (state.kind) {
      case "idle":
        this.run.textContent = "Ready";
        break;
      case "running": {
        const tick = () => {
          this.run.textContent = `Running ${formatMs(performance.now() - state.since)}`;
        };
        tick();
        this.timer = window.setInterval(tick, 100);
        break;
      }
      case "done":
        this.run.textContent =
          state.code === 0
            ? `Exit 0 · ${formatMs(state.ms)}`
            : `Exit ${state.code ?? "?"} · ${formatMs(state.ms)}`;
        this.run.classList.toggle("run-error", state.code !== 0);
        break;
      case "cancelled":
        this.run.textContent = `Cancelled · ${formatMs(state.ms)}`;
        break;
      case "failed":
        this.run.textContent = "Failed to start";
        break;
    }
  }
}

export function formatMs(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`;
  const m = Math.floor(ms / 60_000);
  const s = Math.floor((ms % 60_000) / 1000);
  return `${m}m ${s.toString().padStart(2, "0")}s`;
}
