# 05-04 · Published docs and versioning policy

**Phase:** 5 · **Size:** M · **Crate/area:** `docs/`, CI · **Depends on:** 05-01, 05-03
**Design:** PLAN.md §5 Phase 5, §6 (protocol churn)

## What

Publish `docs/` (plus generated manifest and protocol references) as a static site
from the repo, and write down the compatibility policy for `api`,
`tinkerpad-plugin-v1`, and `tinkerpad-dump-v1`.

## Why

Once someone else's plugin exists, the manifest and protocols are public APIs.
The policy has to be written before the first breaking change is tempting.

## Expectations

- [ ] Generated reference pages: manifest schema (from the Rust types via
      `schemars` → JSON Schema → rendered table), helper protocol messages (from
      the `HelperMessage` types), dump schema (from `RichValue`). Generation is a
      CI step; hand-written prose in PLUGINS.md links to them.
- [ ] Static site (mdBook or similar) built from `docs/` on push to `main`,
      deployed to GitHub Pages.
- [ ] `docs/VERSIONING.md`: the core supports manifest `api` N and N-1 and
      protocol vN and vN-1; deprecations are announced one minor release before
      removal; every breaking change ships with a migration note and, where
      possible, a `tinkerpad plugin migrate` transformation.
- [ ] A `CHANGELOG.md` with a "Plugin authors" section per release.
- [ ] JSON Schema for `plugin.toml` published at a stable URL and referenced in
      the template repo so editors validate the manifest as you type.

## Out of scope

Translations. A marketing site.

## Gotchas

Generate, don't hand-copy: any doc that duplicates a Rust type will drift within a
month.
