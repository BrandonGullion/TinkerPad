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

- [ ] `index.json` schema documented; a CI check in that repo validates entries
      and runs `tinkerpad plugin check` against each URL weekly, marking failures
      in a `status.json`.
- [ ] Submission = PR adding an entry; the PR template asks for the `check` output.
- [ ] App: Browse tab fetches the index (cached for a day), shows entries with
      status, installs through the existing path with the same trust notice.
- [ ] Bundled plugins are listed too, marked bundled, so the index is the single
      catalogue.

## Out of scope

Ratings, downloads counts, hosting plugin bundles, signatures.

## Gotchas

Fetching the index is the app's first network call. Make it opt-in on first open
of the Browse tab, and never on launch.
