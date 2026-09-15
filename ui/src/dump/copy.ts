// "Copy as JSON" / "Copy as CSV" actions.

import { el } from "./json";

/** Writes to the system clipboard. An object so tests can swap it out. */
export const clipboard = {
  async write(text: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
      return;
    } catch {
      // Some webviews refuse the async API outside a direct user gesture.
    }
    const area = el("textarea");
    area.value = text;
    area.style.position = "fixed";
    area.style.opacity = "0";
    document.body.append(area);
    area.select();
    const ok = document.execCommand("copy");
    area.remove();
    if (!ok) throw new Error("copy refused");
  },
};

export function copyButton(format: string, text: () => string): HTMLButtonElement {
  const button = el("button", "dump-action", format);
  button.type = "button";
  button.title = `Copy as ${format}`;
  button.addEventListener("click", (e) => {
    // Don't also toggle whatever row the button sits in.
    e.stopPropagation();
    clipboard.write(text()).then(
      () => flash(button, "Copied"),
      () => flash(button, "Copy failed"),
    );
  });
  return button;
}

function flash(button: HTMLButtonElement, message: string): void {
  const label = button.dataset.label ?? button.textContent ?? "";
  button.dataset.label = label;
  button.textContent = message;
  button.classList.add("flashed");
  window.setTimeout(() => {
    button.textContent = label;
    button.classList.remove("flashed");
  }, 1200);
}

/** Quote a CSV field when it needs it (RFC 4180). */
export function csvField(s: string): string {
  return /[",\r\n]/.test(s) ? `"${s.replaceAll('"', '""')}"` : s;
}
