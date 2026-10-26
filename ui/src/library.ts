// The library panel: everything you have, findable (work items 04-01, 04-12).
//
// Mod+B shows it. A search box filters by name, tags, and source — the core
// does the matching and the ranking — and the results arrive in three
// sections: Pinned, Recent (what this window has opened lately), and the rest.
// Enter opens the selected one, Mod+Enter opens it and stays here so several
// can be opened in a row, and the context menu does the things a playground
// needs doing to it.
//
// It is also the explorer for the library folder (04-12): the header says
// which folder that is, and the list ends with the folders in it that look
// like playgrounds but can't be opened, so nothing in the folder is invisible.
//
// The panel holds no state of its own beyond the query and what is selected:
// every refresh asks the core again, so a change on disk, in another tab, or
// from the watcher lands here the same way.

import { invoke, subscribe } from "./ipc";
import type { Sidebar } from "./layout";
import { pick, prompt } from "./picker";
import type { BrokenPlayground, LibraryInfo, PlaygroundSummary, SearchHit } from "./protocol";
import { toast } from "./toast";
import { ICONS } from "./toolbar";

/** How many playgrounds the panel asks for. */
const LIMIT = 300;

/** How many recently opened ones it shows. */
const RECENT = 10;

/** What the OS calls the place a deleted folder goes. */
export const TRASH = navigator.platform.toLowerCase().startsWith("win") ? "Recycle Bin" : "Trash";

/** A broken playground's row id: it has no playground id, only a folder. */
const BROKEN = "broken:";

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
  /** Open Settings at the library folder. */
  changeFolder(): void;
  /** Put the panel on the other side of the window. */
  switchSide(): void;
}

type Group = { title: string; hits: SearchHit[] };

export class LibraryPanel {
  private readonly search: HTMLInputElement;
  private readonly folder: HTMLElement;
  private readonly folderPath: HTMLElement;
  private readonly sideButton: HTMLButtonElement;
  private readonly tagRow: HTMLElement;
  private readonly list: HTMLElement;
  private readonly count: HTMLElement;
  private hits: SearchHit[] = [];
  private broken: BrokenPlayground[] = [];
  /** The tag filter, if one is on. */
  private tag: string | null = null;
  private grouped = false;
  /** A playground id, or `broken:` and a folder. */
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

    // The folder this is a view of, and what can be done with it (04-12).
    this.folder = document.createElement("div");
    this.folder.className = "library-folder";
    const folderIcon = document.createElement("span");
    folderIcon.className = "library-folder-icon";
    folderIcon.innerHTML = ICONS.folder;
    this.folderPath = document.createElement("span");
    this.folderPath.className = "library-folder-path";
    const reveal = this.iconButton("library-folder-open", ICONS.openOut, "Open the library folder", () =>
      void invoke("reveal_library").catch(fail),
    );
    const change = this.iconButton("library-folder-change", ICONS.edit, "Change the library folder…", () =>
      this.host.changeFolder(),
    );
    this.sideButton = this.iconButton("library-side", ICONS.swapSides, "", () => this.host.switchSide());
    this.folder.append(folderIcon, this.folderPath, reveal, change, this.sideButton);
    this.renderSide();
    this.sidebar.onChange(() => this.renderSide());

    this.tagRow = document.createElement("div");
    this.tagRow.className = "library-tags";
    this.list = document.createElement("div");
    this.list.className = "library-list";
    // One Tab stop for the whole list; the arrows move inside it, and Delete
    // acts on what is selected (04-12).
    this.list.tabIndex = 0;
    this.list.setAttribute("role", "listbox");
    this.list.setAttribute("aria-label", "Playgrounds");
    this.count = document.createElement("div");
    this.count.className = "library-count";

    panel.append(header, this.folder, this.tagRow, this.list, this.count);
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

  /** Whether the keyboard is in the list (not the search box): Delete's cue. */
  get listFocused(): boolean {
    return this.sidebar.visible() && this.list.contains(document.activeElement);
  }

  /** Show or hide the panel. Showing it refreshes and takes focus. */
  toggle(visible = !this.sidebar.visible()): void {
    this.sidebar.setVisible(visible);
    if (visible) {
      void this.refresh().then(() => this.search.focus());
    }
  }

  /** Show it without taking the focus: on the first launch (04-12). */
  show(): void {
    this.sidebar.setVisible(true);
    void this.refresh();
  }

  /** Ask the core again and redraw. */
  async refresh(): Promise<void> {
    const token = ++this.token;
    const query = this.search.value;
    const [hits, info] = await Promise.all([
      invoke<SearchHit[]>("search_playgrounds", { query, limit: LIMIT }).catch(() => [] as SearchHit[]),
      invoke<LibraryInfo>("library_info").catch(() => null),
    ]);
    if (token !== this.token) return;
    this.hits = hits;
    this.broken = info?.broken ?? [];
    if (info) {
      this.folderPath.textContent = info.display;
      this.folderPath.title = info.root;
    }
    this.render();
  }

  /** Move what is selected to the trash, asking first (04-12). */
  async deleteSelected(): Promise<void> {
    const id = this.selected;
    if (!id) return;
    if (id.startsWith(BROKEN)) {
      const broken = this.broken.find((b) => BROKEN + b.dir === id);
      if (broken) await this.removeBroken(broken);
    } else {
      const hit = this.hits.find((h) => h.id === id);
      if (hit) await this.remove(hit);
    }
  }

  // ---- drawing -------------------------------------------------------------

  private renderSide(): void {
    const other = this.sidebar.side() === "left" ? "right" : "left";
    const label = `Move the library to the ${other}`;
    this.sideButton.title = label;
    this.sideButton.setAttribute("aria-label", label);
  }

  private iconButton(className: string, icon: string, label: string, onClick: () => void): HTMLButtonElement {
    const button = document.createElement("button");
    button.type = "button";
    button.className = `library-folder-button ${className}`;
    button.innerHTML = icon;
    button.title = label;
    button.setAttribute("aria-label", label);
    button.addEventListener("click", onClick);
    return button;
  }

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

    // The ones that can't be opened, last: a tag can't match them, and a
    // search matches their folder's name.
    const query = this.search.value.trim().toLowerCase();
    const broken = this.tag
      ? []
      : this.broken.filter((b) => !query || folderName(b.dir).toLowerCase().includes(query));

    this.renderTags();
    this.list.replaceChildren(
      ...groups.flatMap((group) => this.renderGroup(group)),
      ...(broken.length ? [this.heading("Can't be opened"), ...broken.map((b) => this.renderBroken(b))] : []),
    );
    const total = all.length;
    const unusable = broken.length ? ` · ${broken.length} can't be opened` : "";
    this.count.textContent =
      total === 0 && !broken.length
        ? this.hits.length === 0 && !this.search.value
          ? "No playgrounds yet."
          : "Nothing matches."
        : `${total} playground${total === 1 ? "" : "s"}${unusable}`;

    const ids = this.rows().map((row) => row.dataset.id);
    if (this.selected && !ids.includes(this.selected)) this.selected = null;
    this.selected ??= ids[0] ?? null;
    this.markSelected();
  }

  private heading(title: string): HTMLElement {
    const heading = document.createElement("div");
    heading.className = "library-section";
    heading.textContent = title;
    return heading;
  }

  private renderGroup(group: Group): HTMLElement[] {
    return [this.heading(group.title), ...group.hits.map((hit) => this.renderRow(hit))];
  }

  private renderRow(hit: SearchHit): HTMLElement {
    const row = document.createElement("div");
    row.className = "library-row";
    // The one in the editor, so the panel says where you are.
    if (hit.id === this.host.activePlayground()) row.classList.add("open");
    row.dataset.id = hit.id;
    row.id = `library-row-${hit.id}`;
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
      this.select(hit.id);
      void this.open(hit, false);
    });
    row.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      this.select(hit.id);
      void this.menu(hit);
    });
    return row;
  }

  /** A folder with a playground.toml that can't be read (04-12). */
  private renderBroken(broken: BrokenPlayground): HTMLElement {
    const row = document.createElement("div");
    row.className = "library-row library-broken";
    row.dataset.id = BROKEN + broken.dir;
    row.id = `library-broken-${this.broken.indexOf(broken)}`;
    row.tabIndex = -1;
    row.setAttribute("role", "option");
    row.title = broken.message;

    const top = document.createElement("div");
    top.className = "library-row-top";
    const badge = document.createElement("span");
    badge.className = "library-badge";
    badge.innerHTML = ICONS.warning;
    badge.setAttribute("role", "img");
    badge.setAttribute("aria-label", "Can't be opened");
    const name = document.createElement("span");
    name.className = "library-name";
    name.textContent = folderName(broken.dir);
    top.append(badge, name);
    const detail = document.createElement("div");
    detail.className = "library-detail";
    detail.textContent = broken.message;
    row.append(top, detail);

    row.addEventListener("click", () => {
      this.select(row.dataset.id!);
      void this.explain(broken);
    });
    row.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      this.select(row.dataset.id!);
      void this.explain(broken);
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

  private select(id: string): void {
    this.selected = id;
    this.markSelected();
  }

  private markSelected(): void {
    let active: HTMLElement | undefined;
    for (const row of this.rows()) {
      const on = row.dataset.id === this.selected;
      row.classList.toggle("selected", on);
      row.setAttribute("aria-selected", String(on));
      if (on) active = row;
    }
    if (active) this.list.setAttribute("aria-activedescendant", active.id);
    else this.list.removeAttribute("aria-activedescendant");
  }

  private rows(): HTMLElement[] {
    return [...this.list.querySelectorAll<HTMLElement>(".library-row")];
  }

  /** The row after `id`, or the one before it if it is the last. */
  private neighbour(id: string): string | null {
    const ids = this.rows().map((row) => row.dataset.id ?? "");
    const at = ids.indexOf(id);
    if (at === -1) return null;
    return ids[at + 1] ?? ids[at - 1] ?? null;
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
      { id: "delete", label: `Move to ${TRASH}` },
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

  /** Why a broken playground can't be opened, and what can be done (04-12). */
  private async explain(broken: BrokenPlayground): Promise<void> {
    const chosen = await pick(`${folderName(broken.dir)} can't be opened: ${broken.message}`, [
      { id: "reveal", label: "Show in file explorer", detail: "Fix its playground.toml there" },
      { id: "delete", label: `Move to ${TRASH}` },
      { id: "cancel", label: "Cancel" },
    ]);
    if (chosen?.id === "reveal") {
      await this.act("reveal_broken_playground", { dir: broken.dir });
    } else if (chosen?.id === "delete") {
      await this.removeBroken(broken);
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
    await this.trash(hit.id, hit.name, async () => {
      await invoke("delete_playground", { id: hit.id });
      this.host.closed(hit.id);
    });
  }

  private async removeBroken(broken: BrokenPlayground): Promise<void> {
    await this.trash(BROKEN + broken.dir, folderName(broken.dir), () =>
      invoke("delete_broken_playground", { dir: broken.dir }),
    );
  }

  /**
   * Ask, delete, and move the selection on to the next row, so pressing
   * Delete again keeps going down the list (04-12). The focus goes back to
   * the list if that is where it was.
   */
  private async trash(id: string, name: string, remove: () => Promise<unknown>): Promise<void> {
    const inList = this.listFocused;
    const answer = await pick(`Move ${name} to the ${TRASH}?`, [
      { id: "no", label: "Cancel" },
      { id: "yes", label: `Move to ${TRASH}`, detail: "You can restore it from there" },
    ]);
    if (answer?.id !== "yes") return;
    const next = this.neighbour(id);
    const removed = await remove().then(
      () => true,
      (e: { message?: string }) => {
        toast("error", e.message ?? "That didn't work.");
        return false;
      },
    );
    if (removed && this.selected === id) this.selected = next;
    await this.refresh();
    if (inList) this.list.focus();
  }

  /** Run a command that changes the library, then redraw. */
  private async act(command: string, args: Record<string, unknown>): Promise<void> {
    await invoke(command, args).catch(fail);
    await this.refresh();
  }

  // ---- keyboard ------------------------------------------------------------

  private onKey(e: KeyboardEvent): void {
    const rows = this.rows();
    const at = rows.findIndex((row) => row.dataset.id === this.selected);
    const selectedHit = () => this.hits.find((h) => h.id === this.selected);
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
        const broken = this.broken.find((b) => BROKEN + b.dir === this.selected);
        if (broken) {
          e.preventDefault();
          void this.explain(broken);
          return;
        }
        const hit = selectedHit();
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
        const hit = selectedHit();
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

/** The last part of a path, whichever separator it uses. */
function folderName(dir: string): string {
  return dir.split(/[\\/]/).filter(Boolean).pop() ?? dir;
}

function fail(e: { message?: string }): void {
  toast("error", e.message ?? "That didn't work.");
}
