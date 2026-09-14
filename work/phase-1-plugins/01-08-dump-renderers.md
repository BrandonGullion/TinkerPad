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

- [ ] Output pane becomes a list of blocks: text runs and rich nodes, in order.
      Text runs keep the streaming behaviour from 00-03.
- [ ] `table`: header with column names and type hints; click a header to sort
      (client-side, stable); row count and a "truncated" note; cells that are
      themselves `RichValue` nodes render inline collapsed with an expand toggle;
      long strings clipped with ellipsis and full text on hover/expand.
- [ ] `tree`: collapsible; first two levels open by default; arrays show length;
      objects show key count when collapsed; primitives coloured by type.
- [ ] `value`: the literal plus a small type badge.
- [ ] `text`: monospace block; if `lang` is given and a highlighter is bundled,
      highlight it.
- [ ] `html`: sanitised with DOMPurify (bundled from cdnjs is fine in dev; vendored
      into the bundle for release), rendered in a constrained box.
- [ ] `image`: inline, max height with click to expand.
- [ ] `timing`: a small inline badge.
- [ ] `title` renders as a caption above any node.
- [ ] Every node has a "copy as JSON" action; tables also have "copy as CSV".
- [ ] Performance: a 1,000-row table renders in under 100 ms; 10,000 rows uses
      virtualised rows. A tree with 50,000 nodes doesn't freeze on render (lazy
      children).
- [ ] Keyboard: nodes are focusable; Enter toggles expand; arrows move.
- [ ] A dev-only "render examples" command shows one of every kind from
      `dump::examples()` for visual checking.

## Out of scope

Lazy expansion round-trips to the running program (future, via `id`). Charts.
Editing values.

## Gotchas

Avoid a heavy grid library. A hand-rolled table with `content-visibility: auto` or
a small virtualiser is enough and keeps the bundle small, which keeps launch fast.
