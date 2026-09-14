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

export function toast(level: Level, message: string): void {
  const el = document.createElement("div");
  el.className = `toast toast-${level}`;
  el.textContent = message;
  el.title = "Click to dismiss";
  el.addEventListener("click", () => el.remove());
  root().append(el);
  setTimeout(() => el.remove(), DISMISS_MS[level]);
}
