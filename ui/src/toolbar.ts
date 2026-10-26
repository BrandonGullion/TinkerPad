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
  /** At the far end of the bar while this holds: the Explorer button follows
   * the library panel to the right (work item 04-12). */
  trailing?: () => boolean;
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
  // The library panel's header (work item 04-12).
  folder: svg('<path d="M1.5 4a1 1 0 0 1 1-1h3.5l1.5 1.5h6a1 1 0 0 1 1 1V12a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1z"/>'),
  openOut: svg('<path d="M9.5 2.5h4v4M13.5 2.5 8 8M12 9.5V13a.5.5 0 0 1-.5.5h-8A.5.5 0 0 1 3 13V5a.5.5 0 0 1 .5-.5H7"/>'),
  edit: svg('<path d="M11 2.5l2.5 2.5L6 12.5H3.5V10z"/>'),
  swapSides: svg('<path d="M2.5 5.5h10M10 3l2.5 2.5L10 8M13.5 10.5h-10M6 8l-2.5 2.5L6 13"/>'),
  warning: svg('<path d="M8 2 14.5 13.5h-13z"/><path d="M8 6.5v3M8 11.5v.01"/>'),
};

export class Toolbar {
  private buttons: { item: ToolbarItem; button: HTMLButtonElement }[] = [];
  /** The buttons in the order they were given, which `arrange` starts from. */
  private readonly given: { item: ToolbarItem; button: HTMLButtonElement }[] = [];
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
      this.given.push({ item, button });
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
    this.arrange();
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

  /**
   * Put trailing buttons at the end, in the DOM as well as on screen, so the
   * arrow keys and a screen reader go the way the eye does. The button Tab
   * lands on stays the same one.
   */
  private arrange(): void {
    const current = this.buttons[this.current]?.button;
    const trailing = (entry: { item: ToolbarItem }) => entry.item.trailing?.() ?? false;
    const ordered = [
      ...this.given.filter((entry) => !trailing(entry)),
      ...this.given.filter(trailing),
    ];
    if (ordered.every((entry, i) => entry === this.buttons[i])) return;
    this.buttons = ordered;
    for (const { item, button } of ordered) {
      button.classList.toggle("toolbar-trailing", trailing({ item }));
      this.element.append(button);
    }
    this.current = Math.max(
      ordered.findIndex(({ button }) => button === current),
      0,
    );
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
