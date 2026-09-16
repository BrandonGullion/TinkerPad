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

- [ ] Deleting a playground closes its document on any server that has it open
      (`textDocument/didClose`) and removes its entry from the registered setting
      (`[lsp.register]`), before the folder is moved to the trash — not as a side
      effect of the tab closing, which may already have happened.
- [ ] Renaming a playground (the folder moves, 01-04) does the same and reopens
      it under the new URI, so a rename doesn't leave a server pointing at a path
      that no longer exists.
- [ ] After a delete, the reload request (if the plugin declares one) runs, and a
      server that can't reload usefully is restarted rather than left stale.
      Decide by measurement, not by guessing: rust-analyzer is the case to test.
- [ ] Repro test: open and delete ten Rust playgrounds in one session, then open
      an eleventh and check diagnostics arrive. This is the check that would have
      caught it, and it belongs in the `lsp` self-test scenario.
- [ ] With this in place, the language-server scenarios no longer need a launch
      of their own in CI (02-07 split them out as a workaround); put them back in
      the main list and confirm.

## Out of scope

Deleting a playground whose folder the server has *locked* — that is 04-09.

## Gotchas

The tab may or may not be open when a playground is deleted, so the detach has to
work from the library's delete path, not from the tab's close path. Watch out for
deleting a playground that another tab still shows (the library allows it).
