# A TinkerPad language plugin

Copy this folder, change four lines, and you have a language in TinkerPad.

It works before you change anything: out of the box it runs playgrounds with
Python, so `tinkerpad plugin check .` is green on a fresh copy. That is
deliberate — you see what passing looks like before you start, and every
failure after that is something you did.

```
$ tinkerpad plugin check .
  ok   manifest           loads, no warnings
  ok   toolchain          py 3.13.1 at …
  ok   templates          2 kinds, 0 scaffold files
  ok   run:statements     exit 0 in 80 ms
  ok   run:expression     exit 0 in 61 ms
  ok   dump               a table of 2 rows, columns name, n
  ok   errors             line 3: ZeroDivisionError: division by zero
  ok   helper:start       built and ready in 96 ms
  ok   helper:run         ran warm in 12 ms, the table arrived
  ok   helper:cancel      stopped 45 ms after cancel
  ok   helper:shutdown    stopped in 8 ms
11 passed, 0 failed, 0 skipped
```

## What is in here

| | |
|---|---|
| `plugin.toml` | The manifest. Every section, commented, with four places marked **CHANGE ME**. |
| `templates/` | What a new playground starts as, one file per kind. |
| `dump/` | The library playgrounds import to get rich output. Yours to rewrite in your language. |
| `helper/` | A warm runner, so a run doesn't pay for a start-up. **Delete this folder** — and `[helper]` in the manifest — if your language starts quickly enough. |
| `check/` | Sample playgrounds `plugin check` uses: one that dumps a known table, one that fails, one that never ends. |

## The checklist

This is PLUGINS.md §10, with the command to run after each step. Run the check
after every one of them; it takes a second and tells you exactly which part
you have just broken.

### 1. Make it yours

Open `plugin.toml` and change the four marked places: `[language]`,
`[toolchain].probe`, `[run]`, and `[output].error_regex`. Rename the files in
`templates/` and `check/` to your extension.

```sh
tinkerpad plugin check .
```

Expect `manifest` and `toolchain` to pass. If the toolchain is *skipped*, your
probe didn't find the program — that is the check doing its job.

### 2. Make a playground run

`[run]` is a command and its arguments, with no shell. `{toolchain}` is
whatever the probe found, `{main}` is the file, `{dir}` is the folder.

```sh
tinkerpad plugin check . --only run
```

Expect `run:<kind>` to pass for each kind. A run must exit 0 *and* print
something: a playground that prints nothing looks broken to whoever opened it.

### 3. Make errors clickable

`[output].error_regex` turns a line of output into a position. `line` and
`message` are required; `file`, `col` and `severity` are optional.

Put a real mistake in `check/error.<ext>`, then:

```sh
tinkerpad plugin check . --only errors
```

The check fails if the sample runs cleanly — a sample that works is not
testing a pattern. Write the pattern against output you have actually seen,
not what the documentation says the compiler prints.

### 4. Make `dump` work

`dump/` is a library your playgrounds import. It writes a JSON payload inside
an escape sequence, which TinkerPad takes out of the output and renders as a
table, a tree, or a value. The protocol is PLUGINS.md §6, and the Python one
here is about forty lines.

`check/dump.<ext>` must produce exactly this table, which is what the checker
asserts:

| name | n |
|---|---|
| ada | 1 |
| grace | 2 |

```sh
tinkerpad plugin check . --only dump
```

Column names are compared without case, so name the fields the way your
language names fields.

### 5. Decide about a helper

A helper is a long-lived process that runs playgrounds and answers hooks, so a
run costs milliseconds rather than a process start. It speaks a small
line-based protocol over stdin and stdout (PLUGINS.md §5).

- **If your language starts quickly** — under a tenth of a second, say — delete
  `helper/` and the `[helper]` section. Nothing else changes.
- **If it doesn't**, `helper/helper.py` is the reference implementation,
  written to be read. Port it; the shape is the same in any language.

```sh
tinkerpad plugin check . --only helper
```

This checks the handshake, a run, a cancel within three seconds, every hook you
declared, and a clean shutdown.

### 6. Install it

```sh
tinkerpad plugin install .
```

It copies into TinkerPad's plugins folder and the language appears without a
restart. The window's **Plugins** panel does the same thing, and removes it
again.

## Your editor can check the manifest

The first line of `plugin.toml` is a `#:schema` comment pointing at
`docs/reference/plugin-toml.schema.json` in the TinkerPad repository. Editors
that understand it — Taplo, and the VS Code "Even Better TOML" extension —
then describe every key and underline the ones that don't exist, which is
quicker than running the check for a typo.

The schema is generated from the types the loader actually uses, so it says
what this build accepts rather than what the documentation remembers.

## Rules worth knowing before you start

- **Nothing language-specific goes into TinkerPad itself.** If your language
  needs something the manifest can't express, that is a gap in the schema
  worth raising, not a patch to the core.
- **A plugin runs with the user's privileges**, like an editor extension. It is
  not sandboxed. Write accordingly.
- **Paths stay inside the plugin folder**: no leading `/`, no drive letter, no
  `..`. The loader refuses anything else.
- **`api = 1` is the contract**, not `[language].version`. The core supports the
  current and previous `api` and warns on the previous one.
- **Unknown keys are errors**, with a "did you mean". A typo tells you.

## Keeping up

The manifest schema is checked against the loader in TinkerPad's own CI, so a
section that stops existing is caught there rather than by you. When the
protocol version moves forward, this template stays on the previous floor for
one release, so a plugin written from it keeps working through the overlap.
