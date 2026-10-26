// The toolbar above the tabs (work item 04-11).
//
// Every button is a command from the catalog: clicking one is `runCommand`,
// its tooltip is the command's title and keys, and it is disabled exactly when
// the command is. So the toolbar, the keys and the palette can't disagree, and
// adding a button is one entry in `TOOLBAR_ITEMS` — an id, a label, an icon —
// with no layout code.

import { CATALOG, bindingsFor, commands, formatAll, runCommand } from "./commands";

/** One button. `swap` replaces it with another command while `when` holds. */
export interface ToolbarItem {
  command: string;
  label: string;
  icon: string;
  /** A toggle: `aria-pressed` follows this. */
  pressed?: () => boolean;
  swap?: { when: () => boolean; command: string; label: string; icon: string };
  /** Give the editor the focus back after a click, the way the key leaves it. */
  refocus?: boolean;
}

/** 16×16 line icons, drawn in the text colour so every theme gets them right. */
const svg = (body: string) =>
  `<svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true" focusable="false" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round">${body}</svg>`;

export const ICONS = {
  explorer: svg('<rect x="1.5" y="2.5" width="13" height="11" rx="1.5"/><path d="M6 2.5v11"/>'),
  newFile: svg('<path d="M9 1.5H4a1 1 0 0 0-1 1v11a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1V5.5z"/><path d="M9 1.5v4h4M8 8v4M6 10h4"/>'),
  newFileIn: svg('<path d="M9 1.5H4a1 1 0 0 0-1 1v11a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1V5.5z"/><path d="M9 1.5v4h4M6 9l2 2 2-2"/>'),
  run: svg('<path d="M4.5 2.5v11l9-5.5z" fill="currentColor"/>'),
  stop: svg('<rect x="3.5" y="3.5" width="9" height="9" rx="1" fill="currentColor"/>'),
};

export class Toolbar {
  private buttons: { item: ToolbarItem; button: HTMLButtonElement }[] = [];
  /** The button Tab lands on: one stop for the whole toolbar. */
  private current = 0;

  constructor(
    readonly element: HTMLElement,
    items: ToolbarItem[],
    private readonly focusEditor: () => void,
  ) {
    element.setAttribute("role", "toolbar");
    element.setAttribute("aria-label", "Playground");
    element.setAttribute("aria-orientation", "horizontal");
    for (const item of items) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "toolbar-button";
      button.dataset.command = item.command;
      button.addEventListener("click", () => void this.press(item));
      this.buttons.push({ item, button });
      element.append(button);
    }
    element.addEventListener("keydown", (e) => this.onKey(e));
    element.addEventListener("focusin", (e) => {
      const at = this.buttons.findIndex(({ button }) => button === e.target);
      if (at !== -1) this.current = at;
      this.renderTabStops();
    });
    this.render();
  }

  /** Redraw from the commands as they are now. Cheap; call it on any change. */
  render(): void {
    const available = new Map(commands().map((command) => [command.id, command.enabled]));
    for (const { item, button } of this.buttons) {
      const shown = item.swap?.when() ? item.swap : item;
      const title = CATALOG.find((c) => c.id === shown.command)?.title ?? shown.label;
      const keys = formatAll(bindingsFor(shown.command));
      button.dataset.command = shown.command;
      button.innerHTML = `${shown.icon}<span class="toolbar-label">${shown.label}</span>`;
      button.title = keys ? `${title} (${keys})` : title;
      button.setAttribute("aria-label", title);
      button.disabled = !(available.get(shown.command) ?? false);
      if (item.pressed) button.setAttribute("aria-pressed", String(item.pressed()));
    }
    this.renderTabStops();
  }

  private async press(item: ToolbarItem): Promise<void> {
    const shown = item.swap?.when() ? item.swap : item;
    await runCommand(shown.command);
    if (item.refocus) this.focusEditor();
    this.render();
  }

  // ---- keyboard: one Tab stop, arrows inside -------------------------------

  private renderTabStops(): void {
    const usable = this.usable();
    if (!usable.includes(this.current)) this.current = usable[0] ?? 0;
    this.buttons.forEach(({ button }, i) => (button.tabIndex = i === this.current ? 0 : -1));
  }

  private usable(): number[] {
    return this.buttons.flatMap(({ button }, i) => (button.disabled ? [] : [i]));
  }

  private onKey(e: KeyboardEvent): void {
    const usable = this.usable();
    if (usable.length === 0) return;
    const at = Math.max(usable.indexOf(this.current), 0);
    let next: number | undefined;
    switch (e.key) {
      case "ArrowRight":
        next = usable[(at + 1) % usable.length];
        break;
      case "ArrowLeft":
        next = usable[(at - 1 + usable.length) % usable.length];
        break;
      case "Home":
        next = usable[0];
        break;
      case "End":
        next = usable[usable.length - 1];
        break;
      default:
        return;
    }
    e.preventDefault();
    if (next === undefined) return;
    this.current = next;
    this.renderTabStops();
    this.buttons[next]?.button.focus();
  }
}
