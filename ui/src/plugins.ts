// The plugins panel (work item 05-02).
//
// What languages this TinkerPad has, where each came from, whether its
// toolchain is there, and the three things worth doing about them: install one,
// remove one, load the folders again.
//
// A plugin is a program that runs with your privileges, so installing one is
// not a thing to do by accident: the panel says so and asks before it copies
// anything. Removing moves the folder to the recycle bin, like a playground.

import { invoke } from "./ipc";
import { pick, prompt } from "./picker";
import type { LanguageToolchain, PluginsStatus, PluginSummary } from "./protocol";
import { toast } from "./toast";

/** What every install agrees to, in the words the core uses. */
const TRUST =
  "A plugin is a program. It runs with your privileges, like an editor extension: " +
  "it can read and write your files and start other programs. Install plugins you would run by hand.";

export class PluginsPanel {
  private element: HTMLElement | null = null;
  private status: PluginsStatus | null = null;
  private toolchains: LanguageToolchain[] = [];
  private busy = false;

  /** Open it, or close it if it is already open. */
  async toggle(): Promise<void> {
    if (this.element) {
      this.close();
      return;
    }
    await this.open();
  }

  async open(): Promise<void> {
    await this.refresh();
    this.render();
  }

  close(): void {
    this.element?.remove();
    this.element = null;
  }

  /** A `pluginsChanged` event arrived, or something here changed them. */
  async refresh(): Promise<void> {
    try {
      [this.status, this.toolchains] = await Promise.all([
        invoke<PluginsStatus>("list_plugins"),
        invoke<LanguageToolchain[]>("toolchain_status"),
      ]);
    } catch {
      return;
    }
    if (this.element) this.render();
  }

  private toolchainOf(id: string): string {
    const state = this.toolchains.find((entry) => entry.language === id)?.state;
    if (!state) return "";
    if (state.kind === "ready") return `${state.program} ${state.version ?? ""}`.trim();
    if (state.kind === "checking") return "checking…";
    return "not installed";
  }

  private render(): void {
    const panel = this.element ?? document.createElement("div");
    panel.className = "plugins-panel";
    panel.setAttribute("role", "dialog");
    panel.setAttribute("aria-label", "Plugins");
    panel.replaceChildren();

    const head = document.createElement("div");
    head.className = "plugins-head";
    const title = document.createElement("h2");
    title.textContent = "Plugins";
    const close = document.createElement("button");
    close.className = "plugins-close";
    close.textContent = "Close";
    close.setAttribute("aria-label", "Close plugins");
    close.addEventListener("click", () => this.close());
    head.append(title, close);
    panel.append(head);

    const list = document.createElement("div");
    list.className = "plugins-list";
    for (const plugin of this.status?.plugins ?? []) {
      list.append(this.row(plugin));
    }
    for (const id of this.status?.overridden ?? []) {
      list.append(this.overriddenRow(id));
    }
    for (const failure of this.status?.failures ?? []) {
      const row = document.createElement("div");
      row.className = "plugins-row plugins-failed";
      const name = document.createElement("div");
      name.className = "plugins-name";
      name.textContent = failure.dir;
      const why = document.createElement("div");
      why.className = "plugins-note";
      why.textContent = failure.message;
      row.append(name, why);
      list.append(row);
    }
    panel.append(list);

    const actions = document.createElement("div");
    actions.className = "plugins-actions";
    actions.append(
      this.button("Install…", () => void this.install()),
      this.button("Load again", () => void this.reload()),
    );
    const where = document.createElement("span");
    where.className = "plugins-note";
    where.textContent = this.status?.userDir ?? "";
    where.title = where.textContent;
    actions.append(where);
    panel.append(actions);

    if (!this.element) {
      this.element = panel;
      document.body.append(panel);
      // Escape closes it, as it closes every other overlay here.
      panel.addEventListener("keydown", (e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          this.close();
        }
      });
      close.focus();
    }
  }

  private row(plugin: PluginSummary): HTMLElement {
    const row = document.createElement("div");
    row.className = "plugins-row";

    const name = document.createElement("div");
    name.className = "plugins-name";
    name.textContent = plugin.name;
    const version = document.createElement("span");
    version.className = "plugins-version";
    version.textContent = plugin.version ?? "";
    name.append(version);

    // Two lines rather than one: the id and the toolchain are short and always
    // worth seeing, and a plugin folder's path is neither — it goes on its own
    // line where it can be cut without taking anything else with it.
    const facts = document.createElement("div");
    facts.className = "plugins-note";
    facts.textContent = [plugin.id, this.toolchainOf(plugin.id)]
      .filter(Boolean)
      .join(" · ");

    // Where it came from matters most for a user plugin: a URL is something
    // `plugin update` can go back to, a folder is someone's working copy.
    const where = document.createElement("div");
    where.className = "plugins-note plugins-where";
    where.textContent =
      plugin.source === "bundled" ? "bundled" : (plugin.origin ?? plugin.dir);
    where.title = plugin.dir;

    row.append(name, facts, where);
    if (plugin.source !== "bundled") {
      const remove = this.button("Remove", () => void this.remove(plugin));
      remove.classList.add("plugins-remove");
      row.append(remove);
    }
    return row;
  }

  private overriddenRow(id: string): HTMLElement {
    const row = document.createElement("div");
    row.className = "plugins-row plugins-overridden";
    const name = document.createElement("div");
    name.className = "plugins-name";
    name.textContent = id;
    const note = document.createElement("div");
    note.className = "plugins-note";
    // Why the bundled one isn't what is running, which is otherwise a mystery.
    note.textContent = "bundled · replaced by a plugin of the same id in your plugins folder";
    row.append(name, note);
    return row;
  }

  private button(label: string, run: () => void): HTMLButtonElement {
    const button = document.createElement("button");
    button.className = "plugins-action";
    button.textContent = label;
    button.addEventListener("click", run);
    return button;
  }

  private async install(): Promise<void> {
    if (this.busy) return;
    const answer = await prompt("Install a plugin", [
      { label: "Folder or git URL", placeholder: "/path/to/plugin or https://…" },
    ]);
    const source = answer?.[0]?.trim();
    if (!source) return;

    // Said before it happens, every time, not once ever.
    const agreed = await pick(TRUST, [
      { id: "no", label: "Cancel" },
      { id: "yes", label: `Install from ${source}`, detail: "Its manifest is checked first" },
    ]);
    if (agreed?.id !== "yes") return;

    this.busy = true;
    try {
      const name = await invoke<string>("install_plugin", { source });
      toast("info", `${name} is installed.`);
      await this.refresh();
    } catch (error) {
      // The core's message says what was wrong with the folder, which is the
      // useful part; the toast from `invoke` has already shown it.
      void error;
    } finally {
      this.busy = false;
    }
  }

  private async remove(plugin: PluginSummary): Promise<void> {
    if (this.busy) return;
    const answer = await pick(`Remove ${plugin.name}?`, [
      { id: "no", label: "Cancel" },
      {
        id: "yes",
        label: "Move to recycle bin",
        detail: "Tabs open in this language keep their text",
      },
    ]);
    if (answer?.id !== "yes") return;
    this.busy = true;
    try {
      await invoke("remove_plugin", { id: plugin.id });
      toast("info", `${plugin.name} was removed.`);
      await this.refresh();
    } catch {
      // Shown as a toast.
    } finally {
      this.busy = false;
    }
  }

  private async reload(): Promise<void> {
    if (this.busy) return;
    this.busy = true;
    try {
      await invoke("reload_plugins");
      await this.refresh();
      toast("info", "Plugins loaded again.");
    } catch {
      // Shown as a toast.
    } finally {
      this.busy = false;
    }
  }
}
