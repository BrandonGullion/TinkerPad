// Keyboard use of rendered dumps.
//
// Everything interactive inside a dump (tree rows, table headers, cell
// toggles, clipped text, images) carries `data-nav` and `tabIndex = -1`, so Tab
// doesn't walk thousands of rows. Focus the output pane, then:
//
//   Down / Up        next / previous item
//   Home / End       first / last item
//   Enter / Space    toggle or activate (sort a column, open a cell)
//   Right            open, or move down when already open
//   Left             close, or move to the parent tree row

const SELECTOR = "[data-nav]";

function visibleItems(scope: HTMLElement): HTMLElement[] {
  return [...scope.querySelectorAll<HTMLElement>(SELECTOR)].filter((e) => e.getClientRects().length > 0);
}

function focus(target: HTMLElement | undefined): void {
  if (!target) return;
  target.focus({ preventScroll: true });
  target.scrollIntoView({ block: "nearest", inline: "nearest" });
}

/** Handle a keydown inside the output pane. Returns true when it was used. */
export function handleDumpKey(e: KeyboardEvent, pane: HTMLElement): boolean {
  if (e.altKey || e.ctrlKey || e.metaKey) return false;
  const target = e.target as HTMLElement;

  if (target === pane) {
    if (e.key !== "ArrowDown" && e.key !== "Home") return false;
    const first = visibleItems(pane)[0];
    if (!first) return false;
    focus(first);
    return true;
  }
  if (!target.matches(SELECTOR)) return false;

  const move = (delta: number) => {
    const items = visibleItems(pane);
    focus(items[items.indexOf(target) + delta]);
  };
  const expanded = target.getAttribute("aria-expanded");

  switch (e.key) {
    case "Enter":
    case " ":
      target.click();
      return true;
    case "ArrowDown":
      move(1);
      return true;
    case "ArrowUp":
      move(-1);
      return true;
    case "Home":
      focus(visibleItems(pane)[0]);
      return true;
    case "End":
      focus(visibleItems(pane).at(-1));
      return true;
    case "ArrowRight":
      if (expanded === "false") target.click();
      else move(1);
      return true;
    case "ArrowLeft":
      if (expanded === "true") {
        target.click();
      } else {
        const parent = target.closest(".tree-node")?.parentElement?.closest(".tree-node");
        focus(parent?.querySelector<HTMLElement>(":scope > .tree-row") ?? undefined);
      }
      return true;
    default:
      return false;
  }
}
