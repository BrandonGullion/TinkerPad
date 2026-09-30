# 02-10 · A deleted playground leaves its language server

**Phase:** 2 · **Size:** S · **Crate/area:** `tinkerpad-lsp`, `tinkerpad-app` · **Depends on:** 02-05, 02-07
**Design:** ARCHITECTURE.md §4.1, §4.4

## What

Deleting or renaming a playground tells its language server, rather than leaving
the server holding a document and a project that are no longer there. Found while
verifying 02-07: after a session that opens and deletes a dozen Rust playgrounds,
the shared rust-analyzer still answers completions but stops publishing
diagnostics until the app is restarted.

## Why

A scratchpad is where playgrounds are created and thrown away constantly. If an
afternoon of that quietly costs you your errors-as-you-type, the keep-alive
design has failed in exactly the case it was built for.

## Expectations

- [x] Deleting a playground closes its document on any server that has it open
      (`textDocument/didClose`) and removes its entry from the registered setting
      (`[lsp.register]`), before the folder is moved to the trash — not as a side
      effect of the tab closing, which may already have happened.
- [x] Renaming a playground (the folder moves, 01-04) does the same and reopens
      it under the new URI, so a rename doesn't leave a server pointing at a path
      that no longer exists.
- [x] After a delete, the reload request (if the plugin declares one) runs, and a
      server that can't reload usefully is restarted rather than left stale.
      Decide by measurement, not by guessing: rust-analyzer is the case to test.
- [x] Repro test: open and delete ten Rust playgrounds in one session, then open
      an eleventh and check diagnostics arrive. This is the check that would have
      caught it, and it belongs in the `lsp` self-test scenario.
- [ ] **Deferred.** With this in place, the language-server scenarios no longer
      need a launch of their own in CI (02-07 split them out as a workaround);
      put them back in the main list and confirm. The change is a two-line edit
      to `ci.yml`; the "confirm" half needs a CI run, and Actions has been
      billing-blocked for this repository throughout. Merging the launches
      without a green run would be a guess, and a wrong guess breaks the
      pipeline for every later PR. See the Results below for what was
      confirmed locally instead.

## Results

Measured on Windows 11, 2026-09-30, with the in-app `lsp` and `lsp-typescript`
self-test scenarios against a real rust-analyzer and typescript-language-server.

**What was added**

| Piece | File |
|---|---|
| The detach | `crates/tinkerpad-app/src/lsp.rs` (`detach`) |
| Called before the folder moves | `delete_playground`, `rename_playground` in `lib.rs` |
| The tab comes back after a rename | `TabManager::languageMoved` in `ui/src/tabs.ts` |
| Registered projects, so the leak is visible | `ServerStatus.projects` |

It runs from the library's delete and rename paths, not from the tab's close
path, because the tab may have been closed an hour ago or may never have
existed — a playground can be deleted from the library panel while its tab is
still showing it.

**The regression test, and proof that it is one.** The repro in the `lsp`
scenario opens ten Rust playgrounds, waits until rust-analyzer answers a
completion in each (attached is not loaded: the handshake comes back long
before the crate is read), then deletes all ten *with their tabs still open*
and asks the server how many projects it still has.

| | `projects` after deleting all ten |
|---|---|
| With the detach | **0** — PASS |
| With the detach disabled | **10** — FAIL |

The first two versions of this test passed either way, which is why it went
through three revisions: closing each tab before deleting it exercises the
attachment's own drop, which has always cleaned up after itself, and a
playground rust-analyzer never actually loaded is one whose deletion costs it
nothing. Both had to be removed before the test could see the bug.

**The reported symptom did not reproduce.** 02-07 reported that after a dozen
create-and-delete rounds the shared rust-analyzer kept answering completions
but stopped publishing diagnostics. On this machine the eleventh playground got
its diagnostics either way. What did reproduce, and is fixed, is the leak
underneath it: ten deleted playgrounds stayed in `linkedProjects` for the rest
of the session. Whether the diagnostics failure was a consequence of that on an
older rust-analyzer, or something else since fixed, is not something this
measurement can say.

**The reload, decided by measurement.** `unregister` pushes the new
configuration and then sends the plugin's reload request — for Rust that is
`rust-analyzer/reloadWorkspace`. After ten deletes the projects count is 0 and
an eleventh playground still diagnoses, so the reload is enough and no restart
is needed.

Restarting a server that declares *no* reload request was implemented and then
removed, because the measurement said it was not worth it: with
typescript-language-server, a restart after a rename did not bring completions
about the new path back any sooner. What is stale there is tsserver's own
project cache, not anything TinkerPad holds, and a restart on every delete is a
real cost for no gain.

**Rename.** `lsp-typescript` renames a playground with its tab open. The folder
moves, the tab comes back with a live session, and it is the same server
process with no restarts — nothing is left pointing at the old path. The tab is
rebuilt rather than reconnected in place: swapping the client inside a live
editor leaves the server with no document at all, because however the old
client's `didClose` is ordered against the new one's `didOpen`, the close is
the one that sticks.

A measured limitation, reported by the scenario rather than asserted on:
**after a rename, typescript-language-server does not answer about the file at
its new path for at least two minutes.** The editor keeps working throughout —
this costs completions, not the playground. It is tsserver noticing a moved
folder in the workspace it watches; neither reopening the tab nor restarting
the server shortened it. rust-analyzer, which reloads on request, does not have
the problem. Worth its own item if it proves annoying in use.

**What could not be measured here.** Every delete in the repro is refused by
Windows while rust-analyzer watches the folder:

```
INFO ten playgrounds loaded and deleted in 51.6 s; 0 moved to the trash,
     10 refused by the OS (could not move ...\lsp-churn-0 to the trash:
     "Some operations were aborted")
```

That is work item 04-09, and it is why the test asks the server what it still
holds rather than asking the filesystem what is gone: the detach happens before
the trash operation either way, so the check holds on a platform where the
delete itself cannot. On Linux and macOS the same deletes succeed. 04-09 will
make this platform agree.

## Out of scope

Deleting a playground whose folder the server has *locked* — that is 04-09.

## Gotchas

The tab may or may not be open when a playground is deleted, so the detach has to
work from the library's delete path, not from the tab's close path. Watch out for
deleting a playground that another tab still shows (the library allows it).
