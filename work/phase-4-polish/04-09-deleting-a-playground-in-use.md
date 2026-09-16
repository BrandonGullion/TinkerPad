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

- [ ] Before deleting, the playground is detached from its language server
      (02-10) and given a moment for the server's file watcher to let go.
- [ ] A failed trash operation is retried briefly (a few attempts over about a
      second) rather than failing on the first refusal — a watcher releasing a
      handle is a race, not a permanent state.
- [ ] If it still fails, the error says what is likely holding it ("rust-analyzer
      is watching this folder") and offers the one thing that always works:
      stopping that server, from the status strip (02-08). No raw
      `trash` crate error text in the UI.
- [ ] The same treatment for rename, which moves the folder.
- [ ] A test that deletes a playground with its server running and passes on all
      three platforms. If the retry can't be made reliable on Windows, the item
      is not done: say so and stop the server first instead.

## Out of scope

Deleting a playground a *run* is using — cancel the run first; that already
works. Trash behaviour on filesystems with no trash (the library already falls
back to a permanent delete with a confirmation).

## Gotchas

Windows refuses `MoveFileEx` into the recycle bin while a handle without
`FILE_SHARE_DELETE` is open, and a file watcher is exactly that. `remove_dir_all`
sometimes succeeds where trashing fails, but silently bypassing the trash takes
away the undo the user expects — only do it with their say-so.
