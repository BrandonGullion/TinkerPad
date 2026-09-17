// The language servers, in the status bar (work item 02-08).
//
// One chip per server: a coloured dot, the language, and how much memory it is
// using. Clicking one opens a popover with the details worth knowing when
// something is slow or broken — uptime, restarts, open documents, how quickly
// it answers — and the three things worth doing about it: Restart, Stop, and
// reading what the server itself said.
//
// The numbers are samples, so the strip refreshes once a second and on every
// `server` event, never per LSP message.
//
// Plugin helper processes (03-02) are the same idea and should become chips
// here too: give them the same shape as a `ServerStatus` and this renders them
// with no changes.

import { invoke } from "./ipc";
import type { ServerState, ServerStatus } from "./protocol";

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

/**
 * The servers section of the status bar. It owns its own refresh, so the rest
 * of the UI doesn't have to think about it.
 */
export class ServerStrip {
  readonly element: HTMLElement;
  private servers: ServerStatus[] = [];
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

  /** The languages currently open in tabs, whose servers are worth showing. */
  setLanguages(languages: Iterable<string>): void {
    this.interesting = new Set(languages);
    this.render();
  }

  /** A `server` event arrived: refresh now rather than at the next tick. */
  changed(): void {
    void this.refresh();
  }

  private async refresh(): Promise<void> {
    try {
      this.servers = await invoke<ServerStatus[]>("server_status");
    } catch {
      // The command only fails if the app is going away.
      return;
    }
    this.render();
  }

  /** Servers worth a chip: running ones, and stopped ones a tab still wants. */
  private shown(): ServerStatus[] {
    return this.servers.filter(
      (server) => server.state.kind !== "stopped" || this.interesting.has(server.language),
    );
  }

  private render(): void {
    const shown = this.shown();
    this.element.replaceChildren(...shown.map((server) => this.chip(server)));
    this.element.hidden = shown.length === 0;
    // A popover for a server that is no longer shown goes with it.
    if (this.open && !shown.some((s) => key(s) === this.open)) this.closePopover();
    else if (this.open) this.renderPopover();
  }

  private chip(server: ServerStatus): HTMLElement {
    const chip = document.createElement("button");
    chip.className = "server-chip";
    chip.dataset.key = key(server);
    chip.dataset.state = server.state.kind;
    const dot = DOTS[server.state.kind];
    chip.setAttribute(
      "aria-label",
      `${server.name} language server: ${dot.label}. Click for details.`,
    );

    const light = document.createElement("span");
    light.className = "server-dot";
    light.style.background = dot.colour;
    const name = document.createElement("span");
    name.className = "server-name";
    name.textContent = server.name;
    chip.append(light, name);

    const rss = server.state.kind === "ready" ? megabytes(server.state.rssBytes) : "";
    if (rss) {
      const memory = document.createElement("span");
      memory.className = "server-rss";
      memory.textContent = rss;
      chip.append(memory);
    }

    chip.title = `${server.name}: ${dot.label}${rss ? `, ${rss}` : ""}`;
    chip.addEventListener("click", () => this.togglePopover(server));
    return chip;
  }

  private togglePopover(server: ServerStatus): void {
    if (this.open === key(server)) {
      this.closePopover();
      return;
    }
    this.open = key(server);
    this.renderPopover();
  }

  private closePopover(): void {
    this.popover?.remove();
    this.popover = null;
    this.open = null;
  }

  private renderPopover(): void {
    const server = this.servers.find((s) => key(s) === this.open);
    const chip = this.element.querySelector<HTMLElement>(`.server-chip[data-key="${this.open}"]`);
    if (!server || !chip) {
      this.closePopover();
      return;
    }

    const popover = document.createElement("div");
    popover.className = "server-popover";
    popover.setAttribute("role", "dialog");
    popover.setAttribute("aria-label", `${server.name} language server`);

    const title = document.createElement("div");
    title.className = "server-popover-title";
    title.textContent = `${server.name} · ${DOTS[server.state.kind].label}`;
    popover.append(title);

    const reason =
      server.state.kind === "stopped"
        ? server.state.reason
        : server.state.kind === "crashed"
          ? server.state.reason
          : null;
    if (reason) {
      const why = document.createElement("div");
      why.className = "server-popover-reason";
      why.textContent = reason;
      popover.append(why);
    }

    const hint = server.state.kind === "stopped" ? server.state.hint : null;
    if (hint || (server.state.kind === "stopped" && server.installHint)) {
      popover.append(this.hintBlock(hint ?? server.installHint ?? ""));
    }

    popover.append(this.facts(server));

    const actions = document.createElement("div");
    actions.className = "server-popover-actions";
    actions.append(
      this.button("Restart", () => this.act(server, "restart_server")),
      this.button("Stop", () => this.act(server, "stop_server"), server.state.kind === "stopped"),
      this.button("Log", () => void this.showLog(server, popover)),
    );
    popover.append(actions);

    this.popover?.remove();
    this.popover = popover;
    this.element.append(popover);
    const chipBox = chip.getBoundingClientRect();
    popover.style.left = `${Math.max(8, Math.min(chipBox.left, window.innerWidth - 360))}px`;
  }

  /** The numbers worth having when something feels slow. */
  private facts(server: ServerStatus): HTMLElement {
    const list = document.createElement("dl");
    list.className = "server-facts";
    const rows: [string, string][] = [];
    if (server.state.kind === "ready") {
      rows.push(["Process", `${server.state.pid}`]);
      if (server.state.version) rows.push(["Version", server.state.version]);
      rows.push(["Up", duration(server.uptimeMs)]);
      rows.push(["Memory", megabytes(server.state.rssBytes) || "—"]);
    }
    rows.push(["Documents", `${server.documents}`]);
    rows.push([
      "Answers",
      server.requests > 0
        ? `${server.requests} · ${server.p50Ms.toFixed(0)} ms p50 · ${server.p95Ms.toFixed(0)} ms p95`
        : "none yet",
    ]);
    if (server.restarts > 0) rows.push(["Restarts", `${server.restarts}`]);
    if (server.root) rows.push(["Workspace", server.root]);
    rows.push(["Program", server.program]);

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

  /** A missing server: what to run, ready to copy. */
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

  private async act(server: ServerStatus, command: string): Promise<void> {
    await invoke(command, { language: server.language, root: server.root || null }).catch(() => {});
    await this.refresh();
  }

  /** What the server itself said: its stderr and log messages. */
  private async showLog(server: ServerStatus, popover: HTMLElement): Promise<void> {
    const lines = await invoke<string[]>("server_log", {
      language: server.language,
      root: server.root || null,
    }).catch(() => [] as string[]);

    popover.querySelector(".server-log")?.remove();
    const log = document.createElement("div");
    log.className = "server-log";
    const body = document.createElement("pre");
    body.className = "server-log-body";
    body.textContent = lines.length > 0 ? lines.join("\n") : "This server hasn't said anything.";
    const copy = document.createElement("button");
    copy.className = "status-link";
    copy.textContent = "Copy all";
    copy.addEventListener("click", () => void navigator.clipboard.writeText(lines.join("\n")));
    log.append(copy, body);
    popover.append(log);
    body.scrollTop = body.scrollHeight;
  }
}

/**
 * A server is one per language and workspace root. The key goes into a
 * `data-key` attribute and is looked up with a CSS selector, so it is escaped
 * down to characters a selector can carry.
 */
function key(server: ServerStatus): string {
  return `${encodeURIComponent(server.language)}|${encodeURIComponent(server.root)}`;
}
