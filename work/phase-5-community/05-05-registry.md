# 05-05 · Registry index

**Phase:** 5 · **Size:** S · **Crate/area:** separate repo `tinkerpad-plugins`, `ui/` · **Depends on:** 05-02, 05-04
**Design:** PLAN.md §5 Phase 5

## What

A JSON index in a GitHub repo listing community plugins (id, name, description,
git URL, languages, maintainer, last verified core version), and a "Browse" tab in
the Plugins panel that reads it and installs with one click via 05-02.

## Why

Discovery. Nothing more elaborate than Zed's early extension index; a PR to a JSON
file is a fine submission process for a hobby community.

## Expectations

- [x] `index.json` schema documented; a CI check in that repo validates entries
      and runs `tinkerpad plugin check` against each URL weekly, marking failures
      in a `status.json`.
- [x] Submission = PR adding an entry; the PR template asks for the `check` output.
- [x] App: Browse tab fetches the index (cached for a day), shows entries with
      status, installs through the existing path with the same trust notice.
- [x] Bundled plugins are listed too, marked bundled, so the index is the single
      catalogue.

## Results

Built 2026-10-05. The index and everything around it live in `registry/` in
this repository, as the plugin template does, and go to a repository of their
own when the project is public — the same decision, for the same reason.

**The format.** `registry/index.json`: a version, and a list of entries with
an id, a name, a one-line description, a git URL, the languages it adds, a
maintainer, and the TinkerPad version its `check` last passed on. Deliberately
small. Any field that would need maintaining — a download count, a rating, a
mirror of the code — is a field that goes stale and misleads, so there is none.

`registry/index.schema.json` is generated from the types the app reads the
index with, by `tinkerpad registry schema`, with `--check` in CI. Same rule as
05-04: a schema written beside a type is a schema that drifts.

**The checks.** `tinkerpad registry check` reports *every* problem at once
rather than one per push — a missing field, a description that is a paragraph,
an id listed twice, an unknown field (a typo in a hand-edited file should fail
rather than be dropped), a URL `git clone` can't take, an entry that is neither
`bundled` nor has a URL. Six unit tests cover those.

**Fetching is opt-in, and the scenario proves it.** The item's gotcha is that
the index is the app's first network call. So: nothing is fetched on launch,
and opening the Browse tab shows an offer rather than a list. The
`browse` self-test checks exactly that:

```
PASS the plugins panel opens
PASS it has an Installed tab and a Browse tab
PASS opening Browse offers to fetch rather than fetching ({"rows":1})
PASS and says what it would read, before reading it
PASS pressing the button reads it (6 rows)
PASS the bundled plugins are in the index, so it is the whole catalogue
PASS and each is marked as already here rather than offered
PASS which is what the rows show ({"already":6,"entries":6})
PASS leaving the tab and coming back doesn't read it again
PASS the command closes the panel
```

The scenario reads the index through `TINKERPAD_REGISTRY`, which takes a path
as happily as a URL — so the test needs no network, and a contributor can try
their own entry before submitting it.

Once fetched, a copy is kept for a day. Only a *valid* index is cached: a 404
page should not become tomorrow's catalogue. An index nobody can reach is an
empty list and a sentence, not an error.

**Installing from it** goes through 05-02 unchanged — the same path, and the
same trust notice, as typing the URL by hand. Nothing about an entry being in
a list makes its code more trustworthy, and the window does not pretend
otherwise.

**The bundled plugins are listed**, marked `bundled`, so the index is the whole
catalogue rather than only the parts that aren't here. They need no URL and the
Browse tab shows them as already present rather than offering to install them.

**For the registry's own repository**: a README that is the submission process
in one page, a pull request template that asks for the `check` output (which
*is* the review), a workflow validating the index on every PR, and a weekly one
that clones each entry, runs `plugin check`, and writes `status.json`. An entry
that stops working is marked there rather than removed — a plugin broken today
may be fixed tomorrow, and deleting someone's entry because their repository
was briefly unreachable is rude.

**Not published yet**, for the reason 05-03 and 05-04 were not: the repository
is private. `DEFAULT_URL` points at where the index will live and says so in a
comment; until then the Browse tab reads whatever `TINKERPAD_REGISTRY` names,
which in a checkout is `registry/index.json`.

## Out of scope

Ratings, downloads counts, hosting plugin bundles, signatures.

## Gotchas

Fetching the index is the app's first network call. Make it opt-in on first open
of the Browse tab, and never on launch.
