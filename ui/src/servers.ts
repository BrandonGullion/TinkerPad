// The language servers and plugin helpers, in the status bar (work items
// 02-08, 03-02).
//
// One chip per process: a coloured dot, the language, and how much memory it
// is using. Clicking one opens a popover with the details worth knowing when
// something is slow or broken — uptime, restarts, what it is doing, how
// quickly it answers — and the three things worth doing about it: Restart,
// Stop, and reading what it said for itself.
//
// Servers and helpers are supervised by the same code in the core, so they get
// the same chip here. Only two things differ: the numbers worth showing, and
// which commands the buttons call. Both are folded into `Child` below, and
// everything after that is one path.
//
// The numbers are samples, so the strip refreshes once a second and on every
// `server` or `helper` event, never per message.

import { invoke } from "./ipc";
import type { HelperStatus, ServerState, ServerStatus } from "./protocol";

/** How often the samples are refreshed while the window is visible. */
const REFRESH_MS = 1000;

/** State → what the dot means. */
const DOTS: Record<ServerState["kind"], { colour: string; label: string }> = {
  stopped: { colour: "var(--muted)", label: "stopped" },
  starting: { colour: "var(--warning)", label: "starting" },
  ready: { colour: "var(--ok)", label: "ready" },
  crashed: { colour: "var(--error)", label: "crashed" },
  stopping: { colour: "var(--muted)", label: "stopping" },
};

function megabytes(bytes: number | null | undefined): string {
  return bytes ? `${Math.round(bytes / (1024 * 1024))} MB` : "";
}

function duration(ms: number): string {
  const seconds = Math.round(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

/** One long-lived process, whichever kind it is. */
interface Child {
  /** Unique, and safe to put in a CSS selector. */
  key: string;
  /** What it is, for the label a screen reader reads out. */
  what: string;
  /** The language or plugin id, for deciding whether to show a stopped one. */
  language: string;
  /** The display name: "Rust", not "rust". */
  name: string;
  state: ServerState;
  uptimeMs: number;
  restarts: number;
  program: string;
  installHint: string | null;
  /** Rows for the popover, on top of the ones every child has. */
  facts: [string, string][];
  /** The commands its buttons call, and the arguments they take. */
  commands: { restart: string; stop: string; log: string };
  args: Record<string, unknown>;
}

/**
 * The key goes into a `data-key` attribute and is looked up with a CSS
 * selector, so it is escaped down to characters a selector can carry.
 */
function keyOf(...parts: string[]): string {
  return parts.map(encodeURIComponent).join("|");
}

/** A language server as a chip. One per language and workspace root. */
function fromServer(server: ServerStatus): Child {
  return {
    key: keyOf("server", server.language, server.root),
    what: "language server",
    language: server.language,
    name: server.name,
    state: server.state,
    uptimeMs: server.uptimeMs,
    restarts: server.restarts,
    program: server.program,
    installHint: server.installHint,
    facts: [
      ["Documents", `${server.documents}`],
      [
        "Answers",
        server.requests > 0
          ? `${server.requests} · ${server.p50Ms.toFixed(0)} ms p50 · ${server.p95Ms.toFixed(0)} ms p95`
          : "none yet",
      ],
      ...(server.root ? ([["Workspace", server.root]] as [string, string][]) : []),
    ],
    commands: { restart: "restart_server", stop: "stop_server", log: "server_log" },
    args: { language: server.language, root: server.root || null },
  };
}

/** A plugin helper as a chip. One per plugin. */
function fromHelper(helper: HelperStatus): Child {
  return {
    key: keyOf("helper", helper.plugin, ""),
    what: "helper",
    language: helper.plugin,
    name: `${helper.name} helper`,
    state: helper.state,
    uptimeMs: helper.uptimeMs,
    restarts: helper.restarts,
    program: helper.program,
    installHint: helper.installHint,
    facts: [
      ["Running", helper.uses > 0 ? `${helper.uses}` : "nothing"],
      ["Idle", helper.idleMs === null ? "in use" : duration(helper.idleMs)],
    ],
    commands: { restart: "restart_helper", stop: "stop_helper", log: "helper_log" },
    args: { plugin: helper.plugin },
  };
}

/**
 * The servers section of the status bar. It owns its own refresh, so the rest
 * of the UI doesn't have to think about it.
 */
export class ServerStrip {
  readonly element: HTMLElement;
  private children: Child[] = [];
  private open: string | null = null;
  private popover: HTMLElement | null = null;
  private timer = 0;
  /** Languages with a tab open, so a stopped server can still be shown. */
  private interesting = new Set<string>();

  constructor() {
    this.element = document.createElement("span");
    this.element.className = "status-servers";
    document.addEventListener("mousedown", (e) => {
      if (!this.popover || this.popover.contains(e.target as Node)) return;
      if ((e.target as HTMLElement).closest(".server-chip")) return;
      this.closePopover();
    });
  }

  /** Start refreshing. Called once the window is up. */
  start(): void {
    void this.refresh();
    this.timer = window.setInterval(() => void this.refresh(), REFRESH_MS);
  }

  stop(): void {
    clearInterval(this.timer);
    this.timer = 0;
  }

  /** The languages currently open in tabs, whose processes are worth showing. */
  setLanguages(languages: Iterable<string>): void {
    this.interesting = new Set(languages);
    this.render();
  }

  /** A `server` or `helper` event arrived: refresh now, not at the next tick. */
  changed(): void {
    void this.refresh();
  }

  private async refresh(): Promise<void> {
    try {
      const [servers, helpers] = await Promise.all([
        invoke<ServerStatus[]>("server_status"),
        invoke<HelperStatus[]>("helper_status"),
      ]);
      this.children = [...servers.map(fromServer), ...helpers.map(fromHelper)];
    } catch {
      // The commands only fail if the app is going away.
      return;
    }
    this.render();
  }

  /** Worth a chip: running ones, and stopped ones a tab still wants. */
  private shown(): Child[] {
    return this.children.filter(
      (child) => child.state.kind !== "stopped" || this.interesting.has(child.language),
    );
  }

  private render(): void {
    const shown = this.shown();
    this.element.replaceChildren(...shown.map((child) => this.chip(child)));
    this.element.hidden = shown.length === 0;
    // A popover for something no longer shown goes with it.
    if (this.open && !shown.some((c) => c.key === this.open)) this.closePopover();
    else if (this.open) this.renderPopover();
  }

  private chip(child: Child): HTMLElement {
    const chip = document.createElement("button");
    chip.className = "server-chip";
    chip.dataset.key = child.key;
    chip.dataset.state = child.state.kind;
    const dot = DOTS[child.state.kind];
    chip.setAttribute("aria-label", `${child.name}: ${dot.label}. Click for details.`);

    const light = document.createElement("span");
    light.className = "server-dot";
    light.style.background = dot.colour;
    const name = document.createElement("span");
    name.className = "server-name";
    name.textContent = child.name;
    chip.append(light, name);

    const rss = child.state.kind === "ready" ? megabytes(child.state.rssBytes) : "";
    if (rss) {
      const memory = document.createElement("span");
      memory.className = "server-rss";
      memory.textContent = rss;
      chip.append(memory);
    }

    chip.title = `${child.name}: ${dot.label}${rss ? `, ${rss}` : ""}`;
    chip.addEventListener("click", () => this.togglePopover(child));
    return chip;
  }

  private togglePopover(child: Child): void {
    if (this.open === child.key) {
      this.closePopover();
      return;
    }
    this.open = child.key;
    this.renderPopover();
  }

  private closePopover(): void {
    this.popover?.remove();
    this.popover = null;
    this.open = null;
  }

  private renderPopover(): void {
    const child = this.children.find((c) => c.key === this.open);
    const chip = this.element.querySelector<HTMLElement>(`.server-chip[data-key="${this.open}"]`);
    if (!child || !chip) {
      this.closePopover();
      return;
    }

    const popover = document.createElement("div");
    popover.className = "server-popover";
    popover.dataset.key = child.key;
    popover.setAttribute("role", "dialog");
    popover.setAttribute("aria-label", `${child.name} ${child.what}`);

    const title = document.createElement("div");
    title.className = "server-popover-title";
    title.textContent = `${child.name} · ${DOTS[child.state.kind].label}`;
    popover.append(title);

    const reason =
      child.state.kind === "stopped"
        ? child.state.reason
        : child.state.kind === "crashed"
          ? child.state.reason
          : null;
    if (reason) {
      const why = document.createElement("div");
      why.className = "server-popover-reason";
      why.textContent = reason;
      popover.append(why);
    }

    const hint = child.state.kind === "stopped" ? child.state.hint : null;
    if (hint || (child.state.kind === "stopped" && child.installHint)) {
      popover.append(this.hintBlock(hint ?? child.installHint ?? ""));
    }

    popover.append(this.facts(child));

    const actions = document.createElement("div");
    actions.className = "server-popover-actions";
    actions.append(
      this.button("Restart", () => this.act(child, child.commands.restart)),
      this.button("Stop", () => this.act(child, child.commands.stop), child.state.kind === "stopped"),
      this.button("Log", () => void this.showLog(child, popover)),
    );
    popover.append(actions);

    this.popover?.remove();
    this.popover = popover;
    this.element.append(popover);
    const chipBox = chip.getBoundingClientRect();
    popover.style.left = `${Math.max(8, Math.min(chipBox.left, window.innerWidth - 360))}px`;
  }

  /** The numbers worth having when something feels slow. */
  private facts(child: Child): HTMLElement {
    const list = document.createElement("dl");
    list.className = "server-facts";
    const rows: [string, string][] = [];
    if (child.state.kind === "ready") {
      rows.push(["Process", `${child.state.pid}`]);
      if (child.state.version) rows.push(["Version", child.state.version]);
      rows.push(["Up", duration(child.uptimeMs)]);
      rows.push(["Memory", megabytes(child.state.rssBytes) || "—"]);
    }
    rows.push(...child.facts);
    if (child.restarts > 0) rows.push(["Restarts", `${child.restarts}`]);
    rows.push(["Program", child.program]);

    for (const [label, value] of rows) {
      const dt = document.createElement("dt");
      dt.textContent = label;
      const dd = document.createElement("dd");
      dd.textContent = value;
      dd.title = value;
      list.append(dt, dd);
    }
    return list;
  }

  /** Something missing: what to run, ready to copy. */
  private hintBlock(hint: string): HTMLElement {
    const block = document.createElement("div");
    block.className = "server-popover-hint";
    const text = document.createElement("code");
    text.textContent = hint;
    const copy = document.createElement("button");
    copy.className = "status-link";
    copy.textContent = "Copy";
    copy.addEventListener("click", () => void navigator.clipboard.writeText(hint));
    block.append(text, copy);
    return block;
  }

  private button(label: string, run: () => void, disabled = false): HTMLButtonElement {
    const button = document.createElement("button");
    button.className = "server-action";
    button.textContent = label;
    button.disabled = disabled;
    button.addEventListener("click", run);
    return button;
  }

  private async act(child: Child, command: string): Promise<void> {
    await invoke(command, child.args).catch(() => {});
    await this.refresh();
  }

  /** What it said for itself: stderr, log messages, and a helper's build. */
  private async showLog(child: Child, popover: HTMLElement): Promise<void> {
    const lines = await invoke<string[]>(child.commands.log, child.args).catch(() => [] as string[]);

    popover.querySelector(".server-log")?.remove();
    const log = document.createElement("div");
    log.className = "server-log";
    const body = document.createElement("pre");
    body.className = "server-log-body";
    body.textContent = lines.length > 0 ? lines.join("\n") : "This one hasn't said anything.";
    const copy = document.createElement("button");
    copy.className = "status-link";
    copy.textContent = "Copy all";
    copy.addEventListener("click", () => void navigator.clipboard.writeText(lines.join("\n")));
    log.append(copy, body);
    popover.append(log);
    body.scrollTop = body.scrollHeight;
  }
}
