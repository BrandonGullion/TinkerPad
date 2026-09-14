// The bottom panel: a tab strip over interchangeable panels.
//
// Output is the only tab today. The debugger's Variables, Call Stack, and
// Debug Console panels will register here later (docs/DEBUGGING.md D2).

export interface Panel {
  id: string;
  title: string;
  element: HTMLElement;
}

export class BottomPanel {
  readonly element: HTMLElement;
  private readonly strip: HTMLElement;
  private readonly content: HTMLElement;
  private readonly panels = new Map<string, { panel: Panel; tab: HTMLButtonElement }>();
  private active: string | null = null;

  constructor() {
    this.element = document.createElement("section");
    this.element.className = "bottom-panel";
    this.strip = document.createElement("div");
    this.strip.className = "panel-tabs";
    this.strip.setAttribute("role", "tablist");
    this.content = document.createElement("div");
    this.content.className = "panel-content";
    this.element.append(this.strip, this.content);
  }

  /** Add a panel. The first one registered becomes active. */
  register(panel: Panel): void {
    const tab = document.createElement("button");
    tab.className = "panel-tab";
    tab.textContent = panel.title;
    tab.setAttribute("role", "tab");
    tab.addEventListener("click", () => this.show(panel.id));
    panel.element.hidden = true;
    panel.element.setAttribute("role", "tabpanel");
    this.strip.append(tab);
    this.content.append(panel.element);
    this.panels.set(panel.id, { panel, tab });
    if (this.active === null) this.show(panel.id);
  }

  show(id: string): void {
    for (const [pid, { panel, tab }] of this.panels) {
      const on = pid === id;
      panel.element.hidden = !on;
      tab.classList.toggle("active", on);
      tab.setAttribute("aria-selected", String(on));
    }
    this.active = id;
  }
}
