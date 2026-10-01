# 04-09 · Deleting a playground something else has open

**Phase:** 4 · **Size:** S · **Crate/area:** `tinkerpad-library`, `tinkerpad-app`, `ui/` · **Depends on:** 01-04, 02-10
**Design:** ARCHITECTURE.md §5.2

## What

Make deleting a playground work when another process has its folder open, and
say something useful when it truly can't. Found while verifying 02-07: on
Windows, moving a Rust playground to the trash fails with "Some operations were
aborted" while rust-analyzer is watching the folder, and the user is left with a
raw IO error.

## Why

"Delete" failing with an error from the filesystem, for a reason the user can't
see and didn't cause, is the kind of thing that makes an app feel unreliable.
The playground is theirs; the app's own language server should not be what stops
them removing it.

## Expectations

- [x] Before deleting, the playground is detached from its language server
      (02-10) and given a moment for the server's file watcher to let go.
- [x] A failed trash operation is retried briefly (a few attempts over about a
      second) rather than failing on the first refusal — a watcher releasing a
      handle is a race, not a permanent state.
- [x] If it still fails, the error says what is likely holding it ("rust-analyzer
      is watching this folder") and offers the one thing that always works:
      stopping that server, from the status strip (02-08). No raw
      `trash` crate error text in the UI.
- [x] The same treatment for rename, which moves the folder.
- [x] A test that deletes a playground with its server running and passes on all
      three platforms. If the retry can't be made reliable on Windows, the item
      is not done: say so and stop the server first instead.

## Results

Measured on Windows 11, 2026-09-30, with rust-analyzer running and holding the
folders — the case the item was written for.

| | Playgrounds deleted with their language server watching them |
|---|---|
| Before | **0 of 10** — every one refused, `Some operations were aborted` |
| After | **10 of 10**, in 49 s including loading each crate |

The same run renames a playground the server is watching, which used to be
refused for the same reason, and it now goes through.

**What it does, in order.**

1. **Detach** (02-10): the document is closed and the playground taken out of
   the server's registered projects before anything touches the folder. This
   is both the right thing to do and the thing most likely to free it.
2. **Retry**, in `tinkerpad-library`: six attempts over about a second
   (30, 70, 150, 300, 450 ms), because a watcher letting go is a race rather
   than a state. `rename` gets the same treatment, since it moves the folder
   too.
3. **Stop the server**, and try once more.
4. **Say something useful**, if it still won't go.

**Step 3 is there because step 2 is not enough, and that was measured.** With
rust-analyzer running, the retry alone refused all ten deletes — the handle it
holds has no `FILE_SHARE_DELETE` and waiting does not change that. The work
item anticipated this ("if the retry can't be made reliable on Windows, say so
and stop the server first instead"), so that is what happens: the server is
stopped, the user is told in a toast, and the next tab starts it again. The
retry stays because it is what covers the *other* watchers — the library's own
file watcher, an indexer, a backup agent — that do let go.

**Nothing bypasses the recycle bin.** The item's gotcha says a silent
permanent delete takes away the undo the user expects, so it isn't done. If
the folder still can't be moved after the server is stopped, the delete fails
and says so.

**What the user sees instead of a filesystem error.** Never the `trash`
crate's own words:

> My Playground couldn't be moved to the recycle bin: something still has its
> folder open, even after rust-analyzer was stopped. Close anything looking at
> that folder — a terminal, an editor, a file manager — and try again.

A unit test asserts that none of these messages contain "trash operation",
"aborted", or "Unknown", and that each names the playground and says what to
do.

**Tests**

| Where | What |
|---|---|
| `lsp` self-test scenario | Ten playgrounds loaded by rust-analyzer, deleted with their tabs open: all ten reach the recycle bin. Then one is renamed with the server watching it. |
| `crates/tinkerpad-library/src/library.rs` | `retrying` gives up in about a second, sleeps not at all when the first attempt works, waits out a refusal that passes, and reports the *last* attempt's error rather than the first. |
| `crates/tinkerpad-app/src/library.rs` | A refusal is told apart from a real failure; a program is named rather than pathed; the messages say what to do and never quote the filesystem. |

The self-test is platform-neutral: it asks whether the playgrounds are gone,
not how. Linux and macOS have never refused these deletes, so the interesting
half of it only bites on Windows — which is where it was failing.

**Two failures in the `tabs` scenario are not from this work.** "F2 renames the
playground and its folder" and "a dirty tab asks, and Keep mine saves" fail
identically on `main` with this branch stashed. Both come from this machine's
library holding about a hundred leftover self-test playgrounds, so a new
`renamed-tab` folder becomes `renamed-tab-12` and the checks that assume a
clean library don't hold. Worth a tidy-up of the library, not a code change.

## Out of scope

Deleting a playground a *run* is using — cancel the run first; that already
works. Trash behaviour on filesystems with no trash (the library already falls
back to a permanent delete with a confirmation).

## Gotchas

Windows refuses `MoveFileEx` into the recycle bin while a handle without
`FILE_SHARE_DELETE` is open, and a file watcher is exactly that. `remove_dir_all`
sometimes succeeds where trashing fails, but silently bypassing the trash takes
away the undo the user expects — only do it with their say-so.
