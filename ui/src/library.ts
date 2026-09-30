// The library panel: everything you have, findable (work item 04-01).
//
// Mod+B shows it. A search box filters by name, tags, and source — the core
// does the matching and the ranking — and the results arrive in three
// sections: Pinned, Recent (what this window has opened lately), and the rest.
// Enter opens the selected one, Mod+Enter opens it and stays here so several
// can be opened in a row, and the context menu does the things a playground
// needs doing to it.
//
// The panel holds no state of its own beyond the query and what is selected:
// every refresh asks the core again, so a change on disk, in another tab, or
// from the watcher lands here the same way.

import { invoke, subscribe } from "./ipc";
import type { Sidebar } from "./layout";
import { pick, prompt } from "./picker";
import type { PlaygroundSummary, SearchHit } from "./protocol";
import { toast } from "./toast";

/** How many playgrounds the panel asks for. */
const LIMIT = 300;

/** How many recently opened ones it shows. */
const RECENT = 10;

/** What the panel needs from the rest of the window. */
export interface LibraryHost {
  /** Open a playground in a tab. */
  open(id: string): Promise<unknown>;
  /** The playground in the active tab, if any. */
  activePlayground(): string | null;
  /** Playgrounds opened lately, most recent first. */
  recent(): string[];
  /** A playground is gone: close its tab if it has one. */
  closed(id: string): void;
  /** A playground's folder moved: reconnect its tab's language server. */
  moved(id: string): void;
  /** The language plugin's display name, for the grouping headers. */
  languageName(id: string): string;
}

type Group = { title: string; hits: SearchHit[] };

export class LibraryPanel {
  private readonly search: HTMLInputElement;
  private readonly tagRow: HTMLElement;
  private readonly list: HTMLElement;
  private readonly count: HTMLElement;
  private hits: SearchHit[] = [];
  /** The tag filter, if one is on. */
  private tag: string | null = null;
  private grouped = false;
  private selected: string | null = null;
  /** Bumped per refresh, so a slow answer can't overwrite a newer one. */
  private token = 0;

  constructor(
    private readonly sidebar: Sidebar,
    private readonly host: LibraryHost,
  ) {
    const panel = document.createElement("div");
    panel.className = "library";

    const header = document.createElement("div");
    header.className = "library-header";
    this.search = document.createElement("input");
    this.search.className = "library-search";
    this.search.type = "search";
    this.search.placeholder = "Search names, tags, code";
    this.search.setAttribute("aria-label", "Search the library");
    const group = document.createElement("button");
    group.className = "library-group-toggle";
    group.title = "Group by language";
    group.textContent = "⧉";
    group.addEventListener("click", () => {
      this.grouped = !this.grouped;
      group.classList.toggle("on", this.grouped);
      this.render();
    });
    header.append(this.search, group);

    this.tagRow = document.createElement("div");
    this.tagRow.className = "library-tags";
    this.list = document.createElement("div");
    this.list.className = "library-list";
    this.count = document.createElement("div");
    this.count.className = "library-count";

    panel.append(header, this.tagRow, this.list, this.count);
    this.sidebar.host.replaceChildren(panel);

    this.search.addEventListener("input", () => void this.refresh());
    this.search.addEventListener("keydown", (e) => this.onKey(e));
    this.list.addEventListener("keydown", (e) => this.onKey(e));

    // Anything that changes the library changes the panel.
    subscribe(({ event }) => {
      if (event.type === "libraryChanged" || event.type === "sourceChangedOnDisk") {
        if (this.sidebar.visible()) void this.refresh();
      }
    });
  }

  get visible(): boolean {
    return this.sidebar.visible();
  }

  /** Show or hide the panel. Showing it refreshes and takes focus. */
  toggle(visible = !this.sidebar.visible()): void {
    this.sidebar.setVisible(visible);
    if (visible) {
      void this.refresh().then(() => this.search.focus());
    }
  }

  /** Ask the core again and redraw. */
  async refresh(): Promise<void> {
    const token = ++this.token;
    const query = this.search.value;
    const hits = await invoke<SearchHit[]>("search_playgrounds", {
      query,
      limit: LIMIT,
    }).catch(() => [] as SearchHit[]);
    if (token !== this.token) return;
    this.hits = hits;
    this.render();
  }

  // ---- drawing -------------------------------------------------------------

  private render(): void {
    const all = this.tag
      ? this.hits.filter((hit) => hit.tags.some((t) => t.toLowerCase() === this.tag))
      : this.hits;

    const recent = this.host.recent().slice(0, RECENT);
    const shown = new Set<string>();
    const take = (hits: SearchHit[]): SearchHit[] => {
      const kept = hits.filter((hit) => !shown.has(hit.id));
      for (const hit of kept) shown.add(hit.id);
      return kept;
    };

    const groups: Group[] = [];
    const pinned = take(all.filter((hit) => hit.pinned));
    if (pinned.length) groups.push({ title: "Pinned", hits: pinned });
    const lately = take(
      recent
        .map((id) => all.find((hit) => hit.id === id))
        .filter((hit): hit is SearchHit => hit !== undefined),
    );
    if (lately.length) groups.push({ title: "Recent", hits: lately });

    const rest = take(all);
    if (this.grouped) {
      const byLanguage = new Map<string, SearchHit[]>();
      for (const hit of rest) {
        const list = byLanguage.get(hit.language) ?? [];
        list.push(hit);
        byLanguage.set(hit.language, list);
      }
      for (const [language, hits] of [...byLanguage].sort((a, b) => a[0].localeCompare(b[0]))) {
        groups.push({ title: this.host.languageName(language), hits });
      }
    } else if (rest.length) {
      const title = this.search.value.trim() ? "Results" : groups.length ? "All" : "Playgrounds";
      groups.push({ title, hits: rest });
    }

    this.renderTags();
    this.list.replaceChildren(...groups.flatMap((group) => this.renderGroup(group)));
    const total = all.length;
    this.count.textContent =
      total === 0
        ? this.hits.length === 0 && !this.search.value
          ? "No playgrounds yet."
          : "Nothing matches."
        : `${total} playground${total === 1 ? "" : "s"}`;

    if (this.selected && !all.some((hit) => hit.id === this.selected)) this.selected = null;
    this.selected ??= groups[0]?.hits[0]?.id ?? null;
    this.markSelected();
  }

  private renderGroup(group: Group): HTMLElement[] {
    const heading = document.createElement("div");
    heading.className = "library-section";
    heading.textContent = group.title;
    return [heading, ...group.hits.map((hit) => this.renderRow(hit))];
  }

  private renderRow(hit: SearchHit): HTMLElement {
    const row = document.createElement("div");
    row.className = "library-row";
    // The one in the editor, so the panel says where you are.
    if (hit.id === this.host.activePlayground()) row.classList.add("open");
    row.dataset.id = hit.id;
    row.tabIndex = -1;
    row.setAttribute("role", "option");

    const top = document.createElement("div");
    top.className = "library-row-top";
    const name = document.createElement("span");
    name.className = "library-name";
    name.textContent = hit.name;
    const language = document.createElement("span");
    language.className = "library-language";
    language.textContent = this.host.languageName(hit.language);
    top.append(name, language);
    if (hit.pinned) {
      const pin = document.createElement("span");
      pin.className = "library-pin";
      pin.textContent = "★";
      pin.title = "Pinned";
      top.append(pin);
    }

    const detail = document.createElement("div");
    detail.className = "library-detail";
    // What it matched on is worth more than the first line, when they differ.
    detail.textContent = hit.snippet ?? hit.preview;
    if (hit.field === "source") detail.classList.add("library-in-code");

    row.append(top, detail);
    if (hit.tags.length) {
      const tags = document.createElement("div");
      tags.className = "library-row-tags";
      for (const tag of hit.tags) {
        const chip = document.createElement("button");
        chip.className = "library-tag";
        chip.textContent = tag;
        chip.title = `Show only ${tag}`;
        chip.addEventListener("click", (e) => {
          e.stopPropagation();
          this.filterByTag(tag);
        });
        tags.append(chip);
      }
      row.append(tags);
    }

    row.addEventListener("click", () => {
      this.selected = hit.id;
      this.markSelected();
      void this.open(hit, false);
    });
    row.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      this.selected = hit.id;
      this.markSelected();
      void this.menu(hit);
    });
    return row;
  }

  private renderTags(): void {
    const counts = new Map<string, number>();
    for (const hit of this.hits) {
      for (const tag of hit.tags) {
        const key = tag.toLowerCase();
        counts.set(key, (counts.get(key) ?? 0) + 1);
      }
    }
    if (counts.size === 0) {
      this.tagRow.replaceChildren();
      this.tagRow.hidden = true;
      return;
    }
    this.tagRow.hidden = false;
    const chips = [...counts]
      .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
      .map(([tag, n]) => {
        const chip = document.createElement("button");
        chip.className = "library-tag";
        chip.classList.toggle("on", this.tag === tag);
        chip.textContent = `${tag} ${n}`;
        chip.addEventListener("click", () => this.filterByTag(tag));
        return chip;
      });
    this.tagRow.replaceChildren(...chips);
  }

  private filterByTag(tag: string): void {
    const key = tag.toLowerCase();
    this.tag = this.tag === key ? null : key;
    this.render();
  }

  private markSelected(): void {
    for (const row of this.rows()) {
      row.classList.toggle("selected", row.dataset.id === this.selected);
    }
  }

  private rows(): HTMLElement[] {
    return [...this.list.querySelectorAll<HTMLElement>(".library-row")];
  }

  // ---- doing things --------------------------------------------------------

  private async open(hit: SearchHit, stay: boolean): Promise<void> {
    await this.host.open(hit.id).catch((e: { message?: string }) => {
      toast("error", e.message ?? `Couldn't open ${hit.name}.`);
    });
    if (stay) this.search.focus();
  }

  /** What can be done to a playground, as a picker. */
  private async menu(hit: SearchHit): Promise<void> {
    const chosen = await pick(hit.name, [
      { id: "open", label: "Open" },
      { id: "rename", label: "Rename…" },
      { id: "tags", label: "Tags…", detail: hit.tags.join(", ") },
      { id: "pin", label: hit.pinned ? "Unpin" : "Pin" },
      { id: "duplicate", label: "Duplicate" },
      { id: "reveal", label: "Show in file explorer" },
      { id: "delete", label: "Move to recycle bin" },
    ]);
    if (!chosen) return;
    switch (chosen.id) {
      case "open":
        await this.open(hit, false);
        break;
      case "rename":
        await this.rename(hit);
        break;
      case "tags":
        await this.editTags(hit);
        break;
      case "pin":
        await this.act("set_pinned", { id: hit.id, pinned: !hit.pinned });
        break;
      case "duplicate":
        await this.duplicate(hit);
        break;
      case "reveal":
        await this.act("reveal_playground", { id: hit.id });
        break;
      case "delete":
        await this.remove(hit);
        break;
    }
  }

  private async rename(hit: SearchHit): Promise<void> {
    const answer = await prompt(`Rename ${hit.name}`, [
      { label: "Name", value: hit.name },
    ]);
    const name = answer?.[0]?.trim();
    if (!name || name === hit.name) return;
    await this.act("rename_playground", { id: hit.id, name });
    // The folder moved, so a tab showing it is talking about the old path
    // (work item 02-10).
    this.host.moved(hit.id);
  }

  private async editTags(hit: SearchHit): Promise<void> {
    const answer = await prompt(`Tags for ${hit.name}`, [
      { label: "Tags", value: hit.tags.join(", "), placeholder: "json, katas", optional: true },
    ]);
    if (answer === null) return;
    const tags = (answer[0] ?? "")
      .split(",")
      .map((tag) => tag.trim())
      .filter((tag) => tag !== "");
    await this.act("set_tags", { id: hit.id, tags });
  }

  private async duplicate(hit: SearchHit): Promise<void> {
    const copy = await invoke<PlaygroundSummary>("duplicate_playground", { id: hit.id }).catch(
      (e: { message?: string }) => {
        toast("error", e.message ?? `Couldn't duplicate ${hit.name}.`);
        return null;
      },
    );
    if (!copy) return;
    await this.refresh();
    await this.host.open(copy.id).catch(() => {});
  }

  private async remove(hit: SearchHit): Promise<void> {
    const answer = await pick(`Move ${hit.name} to the recycle bin?`, [
      { id: "no", label: "Cancel" },
      { id: "yes", label: "Move to recycle bin", detail: "You can restore it from there" },
    ]);
    if (answer?.id !== "yes") return;
    await this.act("delete_playground", { id: hit.id });
    this.host.closed(hit.id);
  }

  /** Run a command that changes the library, then redraw. */
  private async act(command: string, args: Record<string, unknown>): Promise<void> {
    await invoke(command, args).catch((e: { message?: string }) => {
      toast("error", e.message ?? "That didn't work.");
    });
    await this.refresh();
  }

  // ---- keyboard ------------------------------------------------------------

  private onKey(e: KeyboardEvent): void {
    const rows = this.rows();
    const at = rows.findIndex((row) => row.dataset.id === this.selected);
    switch (e.key) {
      case "ArrowDown":
      case "ArrowUp": {
        if (rows.length === 0) return;
        e.preventDefault();
        const next = e.key === "ArrowDown" ? at + 1 : at - 1;
        const row = rows[Math.min(Math.max(next, 0), rows.length - 1)];
        this.selected = row?.dataset.id ?? this.selected;
        this.markSelected();
        row?.scrollIntoView({ block: "nearest" });
        break;
      }
      case "Enter": {
        const hit = this.hits.find((h) => h.id === this.selected);
        if (!hit) return;
        e.preventDefault();
        // Mod+Enter opens it and leaves the focus here, for opening several.
        void this.open(hit, e.metaKey || e.ctrlKey);
        break;
      }
      case "Escape":
        e.preventDefault();
        // The first Escape clears a search; the second closes the panel.
        if (this.search.value || this.tag) {
          this.search.value = "";
          this.tag = null;
          void this.refresh();
        } else {
          this.toggle(false);
        }
        break;
      case "F2": {
        const hit = this.hits.find((h) => h.id === this.selected);
        if (!hit) return;
        e.preventDefault();
        void this.rename(hit);
        break;
      }
      default:
        return;
    }
  }
}
