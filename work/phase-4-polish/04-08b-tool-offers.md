# 04-08b · Offering to install what a language needs

**Phase:** 4 · **Size:** M · **Crate/area:** `ui/`, `tinkerpad-app`, `plugins/*` · **Depends on:** 04-08a, 02-08
**Design:** PLUGINS.md §2 (`[toolchain]`), ARCHITECTURE.md §4.1

> Split from 04-08 (see `04-08a-acquiring-tools.md` for the machinery). This half
> is what the user sees and what the bundled plugins declare.

## What

"rust-analyzer isn't installed. Install it?", one button, in the place that
already says so — and somewhere to see the whole picture for every language.

## Why

Right now a language works only if you already happen to have its server. That is
a bad first five minutes for an app whose pitch is "open it and start typing",
and it gets worse with every language added.

## Expectations

- [ ] **Never silent, never automatic.** Missing tool → an offer, in the status
      strip (02-08) and in the toast that already reports it, showing exactly what
      will happen: the command line to be run, or the URL, size, and checksum to
      be fetched. Installing is one click and one confirmation, and nothing is
      installed without it.
- [ ] While it runs, the user can see it: a command's output streams somewhere
      they can read, a download shows progress, and both can be left alone —
      neither blocks the window.
- [ ] Failure is ordinary: the hint is shown, the reason is kept, and the offer
      can be made again. A machine with `[tools] acquire = "never"`, or with no
      network, sees the hint and no offer rather than a failing download.
- [ ] Workspace dependencies count as tools: the TypeScript server needs a
      `typescript` in the library folder (02-07), so "install what this language
      needs" has to be able to run that `npm install` too, not just fetch a
      binary.
- [ ] The bundled plugins declare what they need: rustup for rust-analyzer, npm
      for typescript-language-server plus the workspace's TypeScript, gopls for Go
      (04-07), and whatever C# settles on.
- [ ] Somewhere to see it all: a list of what each language needs, whether it is
      there, where it was found, and a button per missing one. The status strip
      (02-08) is the natural home; a language with everything it needs should say
      so as plainly as one without.
- [ ] Tests: the offer appears for a missing tool and not for a present one;
      accepting runs the acquisition and the language becomes usable without a
      restart; `acquire = "never"` shows the hint and no button.

## Out of scope

The acquisition machinery itself (04-08a). The CLI (`tinkerpad tools list` and
`tinkerpad tools install <language>`), which is a subcommand in 05-01 and shares
the same code.

## Gotchas

The status strip is small and this list is not; it needs a home that can hold a
table without crowding what the strip is for.

An offer that appears while someone is typing is an interruption. The toast
already exists and is ignorable; the strip is where the offer should live.
