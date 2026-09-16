// A small keyboard-first list picker: type to filter, arrows to move, Enter to
// choose, Escape to cancel. Used by the new-playground flow and quick open.

export interface PickerItem {
  id: string;
  label: string;
  /** Shown dimmed after the label, and matched by the filter too. */
  detail?: string;
}

let open: { close: (item: PickerItem | null) => void } | null = null;

/** True while a picker is showing. */
export function pickerOpen(): boolean {
  return open !== null;
}

/**
 * Show `items` and resolve with the chosen one, or `null` if cancelled.
 * Opening a picker closes any other.
 */
export function pick(title: string, items: PickerItem[], placeholder = "Type to filter"): Promise<PickerItem | null> {
  open?.close(null);
  const previousFocus = document.activeElement as HTMLElement | null;

  const backdrop = document.createElement("div");
  backdrop.className = "picker-backdrop";
  const dialog = document.createElement("div");
  dialog.className = "picker";
  dialog.setAttribute("role", "dialog");
  dialog.setAttribute("aria-label", title);
  const heading = document.createElement("div");
  heading.className = "picker-title";
  heading.textContent = title;
  const input = document.createElement("input");
  input.className = "picker-input";
  input.placeholder = placeholder;
  input.setAttribute("aria-label", title);
  const list = document.createElement("ul");
  list.className = "picker-list";
  list.setAttribute("role", "listbox");
  const empty = document.createElement("div");
  empty.className = "picker-empty";
  empty.textContent = "Nothing matches";
  dialog.append(heading, input, list, empty);
  backdrop.append(dialog);
  document.body.append(backdrop);

  let shown: PickerItem[] = [];
  let selected = 0;

  const render = () => {
    const query = input.value.trim().toLowerCase();
    shown = items.filter((item) => matches(query, `${item.label} ${item.detail ?? ""}`.toLowerCase()));
    selected = Math.min(selected, Math.max(shown.length - 1, 0));
    list.replaceChildren(
      ...shown.map((item, i) => {
        const li = document.createElement("li");
        li.className = "picker-item";
        li.setAttribute("role", "option");
        li.setAttribute("aria-selected", String(i === selected));
        li.dataset.id = item.id;
        const label = document.createElement("span");
        label.className = "picker-label";
        label.textContent = item.label;
        li.append(label);
        if (item.detail) {
          const detail = document.createElement("span");
          detail.className = "picker-detail";
          detail.textContent = item.detail;
          li.append(detail);
        }
        li.addEventListener("mousedown", (e) => {
          e.preventDefault();
          finish(item);
        });
        return li;
      }),
    );
    empty.hidden = shown.length > 0;
    list.children[selected]?.scrollIntoView({ block: "nearest" });
  };

  let resolveResult: (item: PickerItem | null) => void = () => {};
  const result = new Promise<PickerItem | null>((resolve) => (resolveResult = resolve));

  const finish = (item: PickerItem | null) => {
    if (open !== handle) return;
    open = null;
    backdrop.remove();
    if (!item) previousFocus?.focus();
    resolveResult(item);
  };
  const handle = { close: finish };
  open = handle;

  input.addEventListener("input", () => {
    selected = 0;
    render();
  });
  input.addEventListener("keydown", (e) => {
    switch (e.key) {
      case "ArrowDown":
        selected = Math.min(selected + 1, shown.length - 1);
        break;
      case "ArrowUp":
        selected = Math.max(selected - 1, 0);
        break;
      case "Enter":
        finish(shown[selected] ?? null);
        break;
      case "Escape":
        finish(null);
        break;
      default:
        return;
    }
    e.preventDefault();
    e.stopPropagation();
    if (open === handle) render();
  });
  backdrop.addEventListener("mousedown", (e) => {
    if (e.target === backdrop) finish(null);
  });

  render();
  input.focus();
  return result;
}

/** Every word of the query appears in the text, in any order. */
function matches(query: string, text: string): boolean {
  return query.split(/\s+/).every((word) => text.includes(word));
}
