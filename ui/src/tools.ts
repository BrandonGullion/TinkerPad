// What each language needs, and offering to get it (work item 04-08b).
//
// The core knows precisely which tool is missing for which language (02-08,
// 04-08a). This is where that becomes something you can act on, under three
// rules the item is explicit about.
//
// **Never silent, never automatic.** Nothing is installed without a click and a
// confirmation that shows exactly what will happen: the command line that will
// run, or the URL and checksum that will be fetched. `[tools] acquire = "never"`
// in settings, or a plugin with no `acquire` at all, leaves the hint and no
// button.
//
// **The strip, not your elbow.** An offer that appears while you are typing is
// an interruption, so the list lives behind a status-bar button you open when
// you want it. The one toast that offers anything is the one answering something
// you just tried and which failed for want of a tool.
//
// **Failure is ordinary.** A command that exits non-zero, a download that can't
// be reached, a checksum that doesn't match: the reason is shown next to the
// thing that needs it, the hint comes back, and the button is still there.

import { invoke } from "./ipc";
import type { LanguageNeeds, Need, Which } from "./protocol";
import { toast } from "./toast";

/** How many lines of a running command's output the row keeps. */
const KEEP_LINES = 200;

/** A row's identity, for the busy set and the log. */
function keyOf(language: string, which: Which): string {
  return `${language}|${which}`;
}

export class ToolsPanel {
  /** The status-bar button this hangs from. */
  readonly element: HTMLButtonElement;
  private readonly count: HTMLElement;
  private popover: HTMLElement | null = null;
  private needs: LanguageNeeds[] = [];
  /** Rows with an acquisition in flight. */
  private readonly busy = new Set<string>();
  /** What each acquisition has said so far. */
  private readonly lines = new Map<string, string[]>();
  /** Rows showing their confirmation rather than their button. */
  private readonly confirming = new Set<string>();
  /** Languages already offered in a toast this session, so it is asked once. */
  private readonly offered = new Set<string>();

  constructor() {
    this.element = document.createElement("button");
    this.element.className = "status-link tools-button";
    this.element.title = "What each language needs";
    const label = document.createElement("span");
    label.textContent = "Languages";
    this.count = document.createElement("span");
    this.count.className = "tools-count";
    this.count.hidden = true;
    this.element.append(label, this.count);
    this.element.addEventListener("click", () => this.toggle());
  }

  /** Ask the core what every language needs, and redraw. */
  async refresh(): Promise<void> {
    try {
      this.needs = await invoke<LanguageNeeds[]>("tools_status");
    } catch {
      // Only fails as the app goes away.
      return;
    }
    this.render();
  }

  /** A `toolchain` event arrived: what is missing may have changed. */
  changed(): void {
    void this.refresh();
  }

  /** A line of progress from an acquisition (`tool` event). */
  progress(plugin: string, line: string): void {
    // The event names the plugin, not which of its tools; whichever row is
    // working is the one talking.
    for (const key of this.busy) {
      if (key.startsWith(`${plugin}|`)) {
        const kept = this.lines.get(key) ?? [];
        kept.push(line);
        if (kept.length > KEEP_LINES) kept.splice(0, kept.length - KEEP_LINES);
        this.lines.set(key, kept);
        this.renderPopover();
        return;
      }
    }
  }

  /**
   * Offer what a language is missing, once, in a toast — for when something the
   * user just did failed for want of it. Silent when there is nothing to offer,
   * which includes a machine with offers switched off.
   */
  async offerFor(language: string): Promise<void> {
    if (this.offered.has(language)) return;
    await this.refresh();
    const entry = this.needs.find((n) => n.language === language);
    const missing = entry?.needs.find((need) => !need.present && need.offer && !need.offer.blocked);
    if (!entry || !missing?.offer) return;
    this.offered.add(language);
    toast("warning", `${entry.name}: ${missing.what} isn't installed.`, {
      label: "Install…",
      run: () => {
        this.confirming.add(keyOf(entry.language, missing.which));
        this.open();
      },
    });
  }

  toggle(): void {
    if (this.popover) this.close();
    else this.open();
  }

  open(): void {
    this.element.setAttribute("aria-expanded", "true");
    void this.refresh().then(() => this.renderPopover());
    this.renderPopover();
  }

  close(): void {
    this.popover?.remove();
    this.popover = null;
    this.element.setAttribute("aria-expanded", "false");
  }

  /** The button's own label: quiet when everything is where it should be. */
  private render(): void {
    const missing = this.needs.reduce(
      (total, entry) => total + entry.needs.filter((need) => !need.present).length,
      0,
    );
    this.count.hidden = missing === 0;
    this.count.textContent = missing === 0 ? "" : `${missing}`;
    this.element.classList.toggle("tools-missing", missing > 0);
    this.element.title =
      missing === 0
        ? "Every language has what it needs"
        : `${missing} thing${missing === 1 ? "" : "s"} missing across your languages`;
    if (this.popover) this.renderPopover();
  }

  private renderPopover(): void {
    if (!this.popover) {
      this.popover = document.createElement("div");
      this.popover.className = "tools-popover";
      this.popover.setAttribute("role", "dialog");
      this.popover.setAttribute("aria-label", "What each language needs");
      this.element.parentElement?.append(this.popover);
    }
    const popover = this.popover;
    popover.replaceChildren();

    const title = document.createElement("div");
    title.className = "tools-title";
    title.textContent = "What each language needs";
    popover.append(title);

    if (this.needs.length === 0) {
      const empty = document.createElement("div");
      empty.className = "tools-empty";
      empty.textContent = "No language plugins are loaded.";
      popover.append(empty);
    }

    for (const entry of this.needs) {
      popover.append(this.languageBlock(entry));
    }

    const box = this.element.getBoundingClientRect();
    popover.style.left = `${Math.max(8, Math.min(box.left - 200, window.innerWidth - 460))}px`;
  }

  private languageBlock(entry: LanguageNeeds): HTMLElement {
    const block = document.createElement("section");
    block.className = "tools-language";

    const head = document.createElement("h3");
    head.className = "tools-language-name";
    head.textContent = entry.name;
    const ready = entry.needs.every((need) => need.present);
    const state = document.createElement("span");
    state.className = ready ? "tools-ok" : "tools-warn";
    // A language with everything it needs says so as plainly as one without.
    state.textContent = ready ? "ready" : "needs something";
    head.append(state);
    block.append(head);

    // The plugin's own instructions belong to the language, not to each of its
    // rows: showing them once is the difference between a reminder and a wall.
    const hint = entry.needs.find((need) => !need.present && need.hint)?.hint;
    if (hint) block.append(this.hint(hint));

    for (const need of entry.needs) {
      block.append(this.needRow(entry, need));
    }
    return block;
  }

  private needRow(entry: LanguageNeeds, need: Need): HTMLElement {
    const key = keyOf(entry.language, need.which);
    const row = document.createElement("div");
    row.className = "tools-need";
    row.dataset.present = String(need.present);

    const what = document.createElement("div");
    what.className = "tools-need-what";
    const label = document.createElement("span");
    label.className = "tools-need-label";
    label.textContent = need.label;
    const name = document.createElement("code");
    name.textContent = need.what;
    what.append(label, name);
    row.append(what);

    const where = document.createElement("div");
    where.className = "tools-need-where";
    if (need.present) {
      where.textContent = need.foundAt
        ? `${need.version ? `${need.version} · ` : ""}${need.foundAt}`
        : "found";
      where.title = need.foundAt ?? "";
    } else {
      where.textContent = need.reason ?? "not found";
      where.title = need.reason ?? "";
    }
    row.append(where);

    if (need.present) return row;

    if (this.busy.has(key)) {
      row.append(this.working(key));
      return row;
    }
    if (!need.offer) {
      const none = document.createElement("div");
      none.className = "tools-need-note";
      none.textContent = `${entry.name} doesn't say how to install this, so it has to be done by hand.`;
      row.append(none);
      return row;
    }
    if (need.offer.blocked) {
      const blocked = document.createElement("div");
      blocked.className = "tools-need-note";
      blocked.textContent = need.offer.blocked;
      row.append(blocked);
      return row;
    }
    if (this.confirming.has(key)) {
      row.append(this.confirmation(entry, need, key));
    } else {
      const install = document.createElement("button");
      install.className = "tools-install";
      install.textContent = "Install…";
      install.addEventListener("click", () => {
        this.confirming.add(key);
        this.renderPopover();
      });
      row.append(install);
    }
    const kept = this.lines.get(key);
    if (kept?.length) row.append(this.log(kept));
    return row;
  }

  /** Exactly what will happen, before it happens. */
  private confirmation(entry: LanguageNeeds, need: Need, key: string): HTMLElement {
    const offer = need.offer;
    const box = document.createElement("div");
    box.className = "tools-confirm";

    const lead = document.createElement("div");
    lead.className = "tools-confirm-lead";
    lead.textContent = offer?.kind === "download" ? "This will download:" : "This will run:";
    box.append(lead);

    const detail = document.createElement("pre");
    detail.className = "tools-confirm-detail";
    const parts = [offer?.label ?? ""];
    if (offer?.url) parts.push(offer.url);
    if (offer?.sha256) parts.push(`sha256 ${offer.sha256}`);
    detail.textContent = parts.filter(Boolean).join("\n");
    box.append(detail);

    const actions = document.createElement("div");
    actions.className = "tools-confirm-actions";
    const go = document.createElement("button");
    go.className = "tools-install";
    go.textContent = offer?.kind === "download" ? "Download and install" : "Run it";
    go.addEventListener("click", () => void this.acquire(entry, need, key));
    const cancel = document.createElement("button");
    cancel.className = "tools-cancel";
    cancel.textContent = "Not now";
    cancel.addEventListener("click", () => {
      this.confirming.delete(key);
      this.renderPopover();
    });
    actions.append(go, cancel);
    box.append(actions);
    return box;
  }

  private async acquire(entry: LanguageNeeds, need: Need, key: string): Promise<void> {
    this.confirming.delete(key);
    this.busy.add(key);
    this.lines.set(key, []);
    this.renderPopover();
    try {
      await invoke("acquire_tool", { language: entry.language, which: need.which });
    } catch (error) {
      // The core has already said so in a toast; keep the reason on the row.
      const kept = this.lines.get(key) ?? [];
      kept.push(String((error as { message?: string } | null)?.message ?? error));
      this.lines.set(key, kept);
    } finally {
      this.busy.delete(key);
    }
    // Whether it worked is the core's answer, not ours: ask again.
    await this.refresh();
    this.renderPopover();
  }

  /** While it runs: something to read, and nothing blocking the window. */
  private working(key: string): HTMLElement {
    const box = document.createElement("div");
    box.className = "tools-working";
    const note = document.createElement("div");
    note.className = "tools-need-note";
    note.textContent = "Installing… you can carry on working; this keeps going.";
    box.append(note);
    const kept = this.lines.get(key);
    if (kept?.length) box.append(this.log(kept));
    return box;
  }

  private log(lines: string[]): HTMLElement {
    const log = document.createElement("pre");
    log.className = "tools-log";
    log.textContent = lines.join("\n");
    // Newest line visible without a scroll of its own.
    requestAnimationFrame(() => {
      log.scrollTop = log.scrollHeight;
    });
    return log;
  }

  /** The plugin's own instructions, ready to copy. */
  private hint(hint: string): HTMLElement {
    const block = document.createElement("div");
    block.className = "tools-hint";
    const text = document.createElement("code");
    text.textContent = hint;
    const copy = document.createElement("button");
    copy.className = "status-link";
    copy.textContent = "Copy";
    copy.addEventListener("click", () => void navigator.clipboard.writeText(hint));
    block.append(text, copy);
    return block;
  }
}
