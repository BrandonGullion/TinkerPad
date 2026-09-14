# 04-01 · Library search, tags, recents

**Phase:** 4 · **Size:** M · **Crate/area:** `tinkerpad-library`, `ui/` · **Depends on:** 01-04, 01-12
**Design:** ARCHITECTURE.md §6

## What

A proper library panel: full-text search over playground names, tags, and source;
tag editing; recents and pinned sections; sort options. Replaces the minimal
quick-open from 01-12 as the main way to find things (quick-open stays).

## Why

After a few months a library has hundreds of playgrounds. LINQPad's "My Queries"
tree is one of its most used features.

## Expectations

- [ ] Index gains source text (already read for the preview line); search is a
      case-insensitive substring or simple fuzzy match over name, tags, and
      source, ranked name > tags > source. Under 20 ms for 2,000 playgrounds
      (measure).
- [ ] Side panel toggled with `Mod+B`: search box on top, sections Pinned,
      Recent (last 10 opened), All (grouped by language or flat, toggle). Type to
      filter, Enter opens, `Mod+Enter` opens in a new tab beside.
- [ ] Tags: editable from the tab context menu or the panel; stored in
      `playground.toml`; a tag cloud/filter row in the panel.
- [ ] Pinned: a boolean in `playground.toml`; pinned items sort first.
- [ ] Context menu on an item: open, rename, duplicate (copies the folder with a
      new id), reveal in file explorer, move to recycle bin.
- [ ] The panel updates live from the watcher (01-04) when files change on disk.

## Out of scope

Folders/hierarchy in the library (tags cover it). Cross-library search.

## Gotchas

Don't index `runs.jsonl`, `node_modules`, `target`, or `bin/obj` folders inside a
playground. Only the main file's text is searched.
