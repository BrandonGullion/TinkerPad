# 05-04 · Published docs and versioning policy

**Phase:** 5 · **Size:** M · **Crate/area:** `docs/`, CI · **Depends on:** 05-01b, 05-03
**Design:** PLAN.md §5 Phase 5, §6 (protocol churn)

## What

Publish `docs/` (plus generated manifest and protocol references) as a static site
from the repo, and write down the compatibility policy for `api`,
`tinkerpad-plugin-v1`, and `tinkerpad-dump-v1`.

## Why

Once someone else's plugin exists, the manifest and protocols are public APIs.
The policy has to be written before the first breaking change is tempting.

## Expectations

- [x] Generated reference pages: manifest schema (from the Rust types via
      `schemars` → JSON Schema → rendered table), helper protocol messages (from
      the `HelperMessage` types), dump schema (from `RichValue`). Generation is a
      CI step; hand-written prose in PLUGINS.md links to them.
- [x] Static site (mdBook or similar) built from `docs/` on push to `main`,
      ~~deployed to GitHub Pages~~. **Built, not deployed.** The repository is
      private, so Pages would publish to nobody; CI builds the site on every
      push, which is the half that can fail on a typo. Deployment is four lines
      of workflow whenever the project goes public.
- [x] `docs/VERSIONING.md`: the core supports manifest `api` N and N-1 and
      protocol vN and vN-1; deprecations are announced one minor release before
      removal; every breaking change ships with a migration note and, where
      possible, a `tinkerpad plugin migrate` transformation.
- [x] A `CHANGELOG.md` with a "Plugin authors" section per release.
- [x] JSON Schema for `plugin.toml` ~~published at a stable URL~~ at a stable
      *path* and referenced in the template so editors validate the manifest as
      you type. **The URL waits on publication**; the template carries a
      `#:schema` line pointing at the file, and a test fails if that path stops
      existing.

## Results

Built 2026-10-05.

**Generated, not transcribed.** `cargo run -p tinkerpad-plugin --bin
tinkerpad-docgen` writes four files into `docs/reference/`, each from the types
that actually do the work:

| Page | Generated from |
|---|---|
| `plugin-toml.schema.json` (744 lines) | `schemars` over the types the loader deserialises |
| `manifest.md` (162 lines) | the same schema, rendered as tables |
| `helper-protocol.md` (139 lines) | one example of every `ToHelper` and `FromHelper` message, serialised by the real types |
| `dump.md` (61 lines) | `RichValue`, serialised the same way |

`--check` compares what is committed against what the types say now and exits
non-zero if they differ, naming the files and the command to run. CI runs it,
so a type changed without regenerating is a failed build rather than a page
that quietly lies.

The descriptions in the manifest reference come from the doc comments on the
fields, which means writing a good comment on a type is now also writing the
documentation — the only arrangement that survives contact with a deadline.

**Two types gained `Serialize`** so the protocol page could be written by them
rather than beside them: `FromHelper`, and the `Diagnostic`/`HelperFile` it
carries. Nothing in the app serialises a `FromHelper`; the derive exists for
the reference, and the comment in the file says so.

**The site.** `mdbook build docs` renders `docs/` into `docs/book/`, which is
gitignored. Twelve pages: the plugin contract, versioning, the three generated
references, architecture, the debugging constraints, the keybindings, and the
changelog. CI builds it on Linux.

**`docs/VERSIONING.md`** is the policy the item asked for, written before the
first breaking change is tempting rather than after:

- `api`, `tinkerpad-plugin-v1` and `tinkerpad-dump-v1` version independently.
- Two are supported at a time: N and N-1. A plugin on N-1 loads with a warning
  naming what to change, and does not fail.
- A deprecation is announced one minor release before it may be removed, so an
  author has a cycle in which their plugin works *and* the tool is telling them
  what to do — the only window in which anyone updates anything.
- Additive is not breaking, and the three protocols are built to ignore what
  they do not recognise in both directions.
- There is a section for whoever is changing the core, ending: a version bumped
  without the previous one still loading is not a release, it is a fork.

**`docs/CHANGELOG.md`** has the *Plugin authors* section per release, with an
Unreleased entry listing what a plugin author would want to know about the
manifest keys added during Phase 4 and 5, and a worked example of an entry
describing a breaking change for whoever writes the first real one.

**The schema in an editor.** `plugin-template/plugin.toml` starts with
`#:schema ../docs/reference/plugin-toml.schema.json`, which Taplo and VS Code's
Even Better TOML read: keys are described and unknown ones underlined as you
type. A test asserts that path still resolves, so the line cannot quietly stop
working.

**Two criteria recorded as partly deferred**, both for the same reason and both
noted above: the repository is private, so there is no URL to publish the
schema at and nobody for Pages to serve. Both are a handful of lines when that
changes, and both have the half that can actually fail — the build, the
generation, the drift check — running in CI today.

## Out of scope

Translations. A marketing site.

## Gotchas

Generate, don't hand-copy: any doc that duplicates a Rust type will drift within a
month.
