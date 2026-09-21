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

- [x] Index gains source text (already read for the preview line); search is a
      case-insensitive substring or simple fuzzy match over name, tags, and
      source, ranked name > tags > source. Under 20 ms for 2,000 playgrounds
      (measure).
- [x] Side panel toggled with `Mod+B`: search box on top, sections Pinned,
      Recent (last 10 opened), All (grouped by language or flat, toggle). Type to
      filter, Enter opens, `Mod+Enter` opens in a new tab beside.
- [x] Tags: editable from the tab context menu or the panel; stored in
      `playground.toml`; a tag cloud/filter row in the panel.
- [x] Pinned: a boolean in `playground.toml`; pinned items sort first.
- [x] Context menu on an item: open, rename, duplicate (copies the folder with a
      new id), reveal in file explorer, move to recycle bin.
- [x] The panel updates live from the watcher (01-04) when files change on disk.

## Out of scope

Folders/hierarchy in the library (tags cover it). Cross-library search.

## Gotchas

Don't index `runs.jsonl`, `node_modules`, `target`, or `bin/obj` folders inside a
playground. Only the main file's text is searched.

## Results

Implemented 2026-09-20.

**The index gained text.** `crates/tinkerpad-library/src/search.rs` holds, per
playground, its name and tags lowercased and the first 64 KiB of its main file
(kept twice: as written, and ASCII-lowercased — the same byte length, so an
offset found in one slices the other and a snippet comes back in its real
case). Only the main file is indexed, so `runs.jsonl`, `node_modules`, `target`
and `bin`/`obj` stay out of it. The index also caches each playground's
modified time and preview line, which took the `stat`-per-playground out of
both listing and searching.

**Matching** is case-insensitive substring, every whitespace-separated term
having to match somewhere, so a longer query narrows. A name also matches as a
subsequence (`fzbz` finds "Fizz Buzz") — the one bit of fuzziness that earns
its keep, and deliberately not applied to source text, where a subsequence
match would match everything. Ranking is name (more for a word start, more
again for the whole name), then tags, then source; pinned first, then score,
then most recently modified.

**Measured**, 2,000 playgrounds of a few lines each (`search_of_a_big_library_is_quick`):

| Query | Hits | Release | Debug |
|---|---|---|---|
| `console` (in every one) | 50 | **2.0 ms** | 5 ms |
| `playground 1999` (two terms) | 1 | **1.1 ms** | 4 ms |
| `generated` (a tag) | 50 | **1.8 ms** | 5 ms |
| `nothing` | 0 | **0.9 ms** | 4 ms |

The budget was 20 ms. Before the modified-time cache the same search took 40 ms
in debug, almost all of it `stat`.

**`playground.toml` gained `pinned`**, absent when false so an unpinned
playground's file looks as it always did. `tags` were already there; setting
them tidies: blanks dropped, case-insensitive duplicates collapsed, order kept.

**`duplicate`** copies a playground's files into a new folder with a new id,
leaving behind `runs.jsonl` and the folders a toolchain fills, and bringing
along its tags and `[plugin.*]` tables. `Notes` becomes `Notes copy`, and a
copy of that `Notes copy 2`.

**The panel** (`ui/src/library.ts`) is a left sidebar, `Mod+B`, with its own
draggable width. A search box over the whole library; a tag row that filters;
sections Pinned, Recent (what this window has opened, from the session file),
and the rest — flat or grouped by language. Arrows move, Enter opens,
`Mod+Enter` opens and keeps the focus in the panel, F2 renames, Escape clears
the search and then closes the panel. Right-clicking a row offers open,
rename, tags, pin/unpin, duplicate, show in file explorer, and move to recycle
bin. The row of the playground in the editor is marked. Every change to the
library — from the panel, another tab, or the watcher — refreshes it.

**Tests.** Six unit tests on ranking and six integration tests on the library
(search by name/tag/source, a saved edit searchable without a rescan, tag
tidying, pinning surviving a rescan, duplicating, and the measurement above).
In the app, a new `library-panel` self-test scenario, 13 checks, all passing:

```
PASS Mod+B opens the library panel
PASS searching finds playgrounds by what is in their code, with the line that matched
PASS a name ranks above a mention in the code
PASS a tag finds its playground
PASS pinned playgrounds are first, under their own heading
PASS arrows move through the list and Enter opens what is selected
PASS the playground in the editor is marked, and lately-opened ones have a section
PASS right-clicking offers open, rename, tags, pin, duplicate, reveal and delete
PASS duplicating makes a copy of the code under a new name
PASS tags are tidied and searchable at once
PASS a deleted playground takes its tab with it
PASS Escape clears the search first
PASS and closes the panel next
```

The older `library` and `library-trash` scenarios pass unchanged.

## Checklist status

**Deviation: `Mod+Enter` opens and stays in the panel rather than opening "in a
new tab beside".** Every open is already its own tab here, so "beside" had no
meaning; what the shortcut is for — opening several in a row without going back
to the panel each time — is what it does.

**Note: the context menu is the app's picker, not a native menu.** Right-click
(or F2 for the common case) opens the same keyboard-first list the rest of the
app uses, which keeps it themed, testable, and identical on the three
platforms.

**Note: quick open (`Mod+O`) is untouched**, as the item asks.
