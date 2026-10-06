# Versioning and compatibility

Once someone else's plugin exists, the manifest and the two protocols are
public APIs. This is what TinkerPad promises about them. It is written down now
rather than after the first breaking change is tempting, which is the only time
a policy like this is worth anything.

## What is covered

| | Where it is declared | What it governs |
|---|---|---|
| **Manifest `api`** | `api = 1` in `plugin.toml` | Every key the loader reads |
| **`tinkerpad-plugin-v1`** | The `hello` message's `protocol` | The helper protocol |
| **`tinkerpad-dump-v1`** | `"v": 1` in a dump payload | Rich output values |

Each is an integer that goes up by one when something that was valid stops
being valid. They move independently: a change to the helper protocol does not
bump the manifest.

The reference pages are generated from the types that implement them
([manifest](reference/manifest.md), [helper protocol](reference/helper-protocol.md),
[dump](reference/dump.md)), so a version that changed without the documentation
changing is a failed build rather than a surprise.

## The promise

**Two versions are supported at a time: the current one and the one before
it.** A plugin written against `api = N-1` keeps working when the core moves to
`N`. It loads with a warning naming what to change and by when; it does not
fail.

**A deprecation is announced one minor release before it bites.** The release
that introduces `N` warns about `N-1`. The release after that may drop it. So a
plugin author has a release cycle in which their plugin works *and* the tool is
telling them what to do, which is the only window in which anyone actually
updates anything.

**Every breaking change ships with a migration note** in the changelog's
*Plugin authors* section, saying what changed, what to write instead, and
whether `tinkerpad plugin migrate` can do it. Where a change is mechanical — a
key renamed, a value's spelling changed — it is expected to be mechanical for
the tool too.

**Additive changes are not breaking.** A new optional key, a new message type,
a new dump node kind: none of these bump a version. Both sides ignore what they
do not recognise, in both directions, which is a rule the protocols are built
on rather than a courtesy:

- A manifest key this build has never heard of is an error *with a suggestion*,
  because a typo is far more likely than a key from the future. An entire
  unknown top-level section is a warning, not an error.
- A helper message type this build does not know is logged and ignored. A field
  it does not know is ignored. The same is true in the other direction.
- A dump node of an unknown kind is kept and shown as its raw payload rather
  than dropped.

**Nothing is removed without a version bump**, including behaviour that was
never written down. If a plugin relies on it and it was observable, changing it
is a break.

## What is not covered

Anything inside TinkerPad itself: crate APIs, the event bus, the Tauri
commands, the names of internal modules. Those change whenever they need to.
A plugin that reaches into them was never using a supported interface.

The `[language].version` key in a manifest is the plugin author's own, and
TinkerPad never reads it. `api` is the contract.

## For a plugin author

- Pin nothing. Write `api = 1` and leave it; the core tells you when to change
  it.
- Run `tinkerpad plugin check` in CI. It uses the same loader the app does, so
  it fails on the version that will fail for your users, in the release where
  it starts warning rather than the one where it breaks.
- Read the *Plugin authors* section of each release's changelog. It is short,
  and it exists for exactly this.

## For whoever is changing the core

Before removing or renaming anything a manifest or a protocol can name:

1. Is it additive instead? Most things can be.
2. If not, bump the version, keep the old one loading with a warning, and write
   the migration note in the same pull request.
3. Add it to `tinkerpad plugin migrate` if the change is mechanical.
4. Regenerate the reference pages. CI fails if you forget.

A version bumped without the previous one still loading is not a release, it is
a fork.
