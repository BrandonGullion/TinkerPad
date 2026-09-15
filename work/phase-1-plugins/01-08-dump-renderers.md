# 01-08 · Dump renderers in the UI

**Phase:** 1 · **Size:** L · **Crate/area:** `ui/` · **Depends on:** 01-07, 00-03
**Design:** PLUGINS.md §6

## What

The output pane learns to render `RichValue` nodes inline with plain text: tables,
trees, scalar values with type badges, text blocks, sanitised HTML, images, and
timing badges. Nested values inside table cells render recursively.

## Why

This is what makes the app LINQPad rather than a terminal with an editor. It
deserves the most UI effort of any Phase 1 item.

## Expectations

- [x] Output pane becomes a list of blocks: text runs and rich nodes, in order.
      Text runs keep the streaming behaviour from 00-03.
- [x] `table`: header with column names and type hints; click a header to sort
      (client-side, stable); row count and a "truncated" note; cells that are
      themselves `RichValue` nodes render inline collapsed with an expand toggle;
      long strings clipped with ellipsis and full text on hover/expand.
- [x] `tree`: collapsible; first two levels open by default; arrays show length;
      objects show key count when collapsed; primitives coloured by type.
- [x] `value`: the literal plus a small type badge.
- [x] `text`: monospace block; if `lang` is given and a highlighter is bundled,
      highlight it.
- [x] `html`: sanitised with DOMPurify (bundled from cdnjs is fine in dev; vendored
      into the bundle for release), rendered in a constrained box.
- [x] `image`: inline, max height with click to expand.
- [x] `timing`: a small inline badge.
- [x] `title` renders as a caption above any node.
- [x] Every node has a "copy as JSON" action; tables also have "copy as CSV".
- [x] Performance: a 1,000-row table renders in under 100 ms; 10,000 rows uses
      virtualised rows. A tree with 50,000 nodes doesn't freeze on render (lazy
      children).
- [x] Keyboard: nodes are focusable; Enter toggles expand; arrows move.
- [x] A dev-only "render examples" command shows one of every kind from
      `dump::examples()` for visual checking.

## Out of scope

Lazy expansion round-trips to the running program (future, via `id`). Charts.
Editing values.

## Gotchas

Avoid a heavy grid library. A hand-rolled table with `content-visibility: auto` or
a small virtualiser is enough and keeps the bundle small, which keeps launch fast.

## Results

Implemented 2026-09-15.

**Pieces** (all in `ui/src/dump/` unless noted)
- `output.ts` (in `ui/src/`): the output pane is now an ordered list of text
  spans and dump blocks. Text keeps its batched, merged streaming. A block
  counts as one line towards the 20,000-line cap. A resize observer keeps the
  pane pinned to the bottom while dumps grow.
- `render.ts`: one renderer per kind, plus a caption for `title` and
  **Copy JSON** (and **Copy CSV** for tables) on every node, nested ones too.
  Untitled `value` and `timing` dumps sit inline in the text. `unknown` shows its
  raw JSON as a tree, and `truncated` shows its size.
- `table.ts`: a hand-rolled grid.
  - Headers show names and type hints. Clicking a header sorts ascending, then
    descending, then back to the original order. The sort is stable, and missing
    values stay last.
  - The footer shows the row count and a truncated note.
  - Nested dump values and plain JSON in cells start collapsed and render the
    first time they open. Strings over 48 characters are clipped, with the full
    text on hover and on click.
  - Tables over 2,000 rows are virtualised in chunks of 200 rows. A chunk fills
    in as it nears the view and turns back into a placeholder when far away,
    unless something inside it is open or focused.
- `tree.ts`: the first two levels start open. Arrays show their length, closed
  objects their key count, and primitives are coloured by type. Children render
  on first open, 200 at a time, with a "… N more" row.
- `html.ts`: DOMPurify 3.4.15 from npm, bundled by Vite, so nothing loads at
  runtime.
  - Besides scripts and event handlers, it removes `<style>`, forms, frames,
    media, link targets, `srcset`, and styles containing `url()`. It also drops
    every URL except inline `data:image` sources.
  - The result renders in a shadow root inside a clipped box, so page CSS can't
    reach it and `position: fixed` can't cover the app.
- `highlight.ts`: Lezer highlighting for `text` dumps, with a registry that
  02-04's editor languages can join. Only `@codemirror/lang-json` is bundled for
  now; other `lang` values show plain.
- `nav.ts`: interactive parts carry `data-nav`, so Tab doesn't walk every row.
  From the focused pane:
  - Down and Up move between parts; Home and End jump to the first and last.
  - Enter and Space toggle an item or sort a column.
  - Right opens an item, and Left closes it or moves to the parent tree row.
- `examples.ts` and the app's `dump_examples` command: in dev builds,
  **Mod+Shift+E** renders `dump::examples()` plus an unknown kind and a truncated
  dump. The examples now use a visible SVG image, JSON text, and a table with
  nested, plain-JSON, null, and long-string cells.
- `serde_json` now keeps object key order across the workspace. Without it,
  dumped objects came out alphabetised. This is logged in PLAN.md's decision log,
  with a Rust test.
- PLUGINS.md §6 now covers inline versus block dumps, highlighting, and what HTML
  sanitising removes.

**Measurements** (Windows 11, release build, in-app self-test)

| What | Result |
|---|---|
| 1,000-row table, render and layout | 34 ms (target under 100 ms) |
| 10,000-row table, first render | 7 ms, 200 rows in the DOM |
| 50,000-element tree, render and layout | 16 ms, one page of 200 children |
| JS bundle | 315 kB → 398 kB (gzip 102 → 132 kB) |

**Tests**
- New in-app `dump-render` scenario, with 38 checks covering every criterion:
  - each kind and its caption;
  - sort order, stability, and nulls;
  - nested cells and clipping;
  - CSV and JSON copy text;
  - tree defaults, key order, and lazy children;
  - keyboard toggling and movement;
  - hostile HTML: scripts, handlers, remote images, links, forms, `url()` styles,
    and a fixed overlay;
  - the three performance cases, including scrolling and sorting a virtualised
    table.
- The `dump` scenario now checks that a real run's output pane holds text and
  dump blocks in order.
- All 96 checks in eight scenarios pass on Windows. CI runs both dump scenarios
  on macOS and Linux.
- Screenshots of the examples confirmed the layout by eye: tables, trees,
  highlighted JSON, HTML, image, badges, and virtual rows mid-table.

## Checklist status

**Ticked with a note**
- *Dev-only "render examples" command.* The self-test calls the same function
  and command. The Mod+Shift+E key binding exists only in dev builds and wasn't
  pressed, because tests don't send keystrokes.
- *Copy as JSON / CSV.* The self-test checks the exact text sent to the
  clipboard by swapping the clipboard writer. Writing to the real system
  clipboard wasn't checked.
- *Performance timings* come from the Windows release build. CI's software
  rendering only checks a loose 1-second bound, to avoid flaky failures.
