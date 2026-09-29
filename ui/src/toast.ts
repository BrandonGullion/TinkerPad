// Non-blocking notices in the corner of the window.

import type { Level } from "./protocol";

const DISMISS_MS: Record<Level, number> = {
  info: 4000,
  warning: 6000,
  error: 8000,
};

let container: HTMLElement | null = null;

function root(): HTMLElement {
  if (!container) {
    container = document.createElement("div");
    container.className = "toasts";
    container.setAttribute("role", "status");
    container.setAttribute("aria-live", "polite");
    document.body.append(container);
  }
  return container;
}

export function toast(level: Level, message: string, action?: Action): void {
  const el = document.createElement("div");
  el.className = `toast toast-${level}`;
  const text = document.createElement("span");
  text.className = "toast-text";
  text.textContent = message;
  el.append(text);
  el.title = "Click to dismiss";
  el.addEventListener("click", () => el.remove());

  if (action) {
    // A toast with something to do stays until it is answered: an offer that
    // disappears while you are reading it is not an offer (work item 04-06).
    const button = document.createElement("button");
    button.className = "toast-action";
    button.textContent = action.label;
    button.addEventListener("click", (e) => {
      e.stopPropagation();
      el.remove();
      action.run();
    });
    el.append(button);
    root().append(el);
    return;
  }

  root().append(el);
  setTimeout(() => el.remove(), DISMISS_MS[level]);
}

/** Something a toast offers to do, shown as a button beside its text. */
export interface Action {
  label: string;
  run: () => void;
}
