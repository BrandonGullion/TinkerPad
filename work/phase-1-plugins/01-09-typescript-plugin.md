# 01-09 · TypeScript plugin

**Phase:** 1 · **Size:** M · **Crate/area:** `plugins/typescript/` · **Depends on:** 01-05, 01-06, 01-07
**Design:** PLUGINS.md §7 (TypeScript), §6

## What

The first real plugin, as a pure folder: `plugin.toml`, templates for `statements`
and `program`, and a `dump/` library that emits `tinkerpad-dump-v1`. No helper. No
LSP section yet (02-07 adds it).

## Why

Node is installed, runs `.ts` directly, and starts in ~40 ms. It proves the whole
Phase 1 loop with the least friction, and its dump library is the easiest to write.

## Expectations

- [x] `plugin.toml` with `[language]` (`id = "typescript"`, `extension = "ts"`,
      `highlighter = "typescript"`), `[toolchain]` probing `node --version` with
      `min_version = "22.6"`, two kinds, `[run]` = `node
      --experimental-transform-types {main}` with `NODE_PATH` set to `{plugin_dir}/dump`.
- [x] Templates: `statements.ts` imports `dump` and has one example line;
      `program.ts` has a `main()` and calls it.
- [x] `dump/tinkerpad.ts` (or `.js` plus `.d.ts`) exporting `dump(value, title?)`:
      arrays of objects → `table` (columns from the union of keys, in first-seen
      order); other objects/arrays → `tree`; primitives → `value`; `Map`/`Set`
      handled; cycles replaced with `"[circular]"`; depth limit 5; row cap 1000
      with `truncated: true`; `Date`, `BigInt`, `undefined`, functions serialised
      sensibly. Writes the OSC sequence with `process.stdout.write`.
- [x] `dump` returns its argument so `dump(x).length` style chaining works.
- [x] Node's `console.log` still goes through as plain text, interleaved correctly.
- [x] Check and record here whether `--experimental-transform-types` preserves
      line numbers (strip-only mode does by design; transform mode should emit
      source maps). If line numbers drift for files using `enum`, note it: Phase 6
      needs `--enable-source-maps` or strip-only for breakpoints to land.
      (DEBUGGING.md D1, D10.)
- [x] A `README.md` in the plugin folder: what it needs, what kinds exist, how to
      use `dump`.
- [x] Captured output from `dump` of each shape added to 01-07's fixture set.
- [x] Hello-world and a `dump` of an array of objects render correctly in the app.
      Mod+Enter to table visible measured and recorded here.

## Out of scope

The LSP section (02-07), the Node helper (03-06), npm deps (01-14 wires the
manifest; this item may declare `[deps]` but needn't test it).

## Gotchas

`NODE_PATH` is ignored for ES module resolution. Either ship `dump/` as a package
with a `package.json` and use `--import` to preload it, or use a `#!` import map.
Simplest that works: template imports `from "tinkerpad"` and `[run]` passes
`--import {plugin_dir}/dump/register.mjs` which registers a resolve hook. Try the
plain `NODE_PATH` route first with a CommonJS shim and pick the one that works.

## Results

Implemented 2026-09-15.

**Pieces** (`plugins/typescript/`)
- `plugin.toml`:
  - id `typescript`, extension `ts`, highlighter `typescript`;
  - a probe for `node --version` with a minimum of 22.7;
  - kinds `statements` (default) and `program`;
  - a scaffolded `package.json` with `"type": "module"`;
  - `[run]` = `node --experimental-transform-types
    --disable-warning=ExperimentalWarning --require {plugin_dir}/dump/register.cjs
    {main}`.
- `templates/`: `statements.ts` imports `dump` and dumps a two-row table.
  `program.ts` has `main()`, prints a greeting, dumps it, and calls `main()`.
- `dump/tinkerpad.js` with `tinkerpad.d.ts`: `dump(value, title?)` returns its
  argument, and `toDumpNode` builds the node without writing it.
  - An array of objects becomes a table, with columns from the union of keys in
    first-seen order and per-column type hints. Cells that hold structures become
    nested nodes: a table for an array of objects, a tree otherwise.
  - Other objects, arrays, `Map`, `Set`, and typed arrays become a tree.
    `Map`s with object keys become `{key, value}` pairs.
  - Everything else becomes a value with its type.
  - Limits: five levels of nesting, 1,000 table rows (with `truncated: true`),
    and 1,000 items per collection (with an "… N more" entry). A reference back
    to an enclosing object becomes `"[circular]"`.
  - Values JSON can't hold:
    - `undefined` becomes `null` (type `undefined`), and `NaN` and `Infinity`
      become strings.
    - A `bigint` becomes `10n`, a `Date` its ISO string, and a `RegExp` its
      source.
    - A function becomes `[Function: name]` and a class `[class Name]`.
    - An `Error` becomes its name, message, and stack.
- `dump/register.cjs` (plus `hooks.mjs` for Node 22.7 to 22.14): resolves
  `"tinkerpad"` for `import` and `require`.
- `README.md`: requirements, kinds, the run command, `dump` rules, and files.

**Other changes**
- The app's `build.rs` now watches `plugins/`. Before, editing a bundled plugin
  didn't re-copy it into `target/` until something else rebuilt the app.
- PLUGINS.md §7 describes the real run command. PLAN.md's decision log records
  the resolve hook and the Node 22.7 minimum.

**Tests**
- `tinkerpad-plugin/tests/bundled.rs`:
  - every bundled plugin loads on all three OSes with no warnings, and has
    `[run]`, an install hint, and a README;
  - both TypeScript kinds scaffold a rendered `main.ts` and a module
    `package.json`.
- `tinkerpad-runner/tests/typescript_plugin.rs` runs the plugin for real through
  `run_spec`, the process runner, and `OutputDecoder`. It skips when Node is
  missing, except in CI, where it fails. Five tests:
  - the statements template produces exactly one "people" table;
  - the program template prints and then dumps;
  - `console.log`, `process.stdout.write`, `console.error`, and dumps
    interleave in order, and a 1,005-row table keeps 1,000 rows and is marked
    truncated;
  - line numbers match with `enum` and parameter properties;
  - the output of `tests/plugins/typescript/shapes.ts` matches the captured
    fixture.
- `tests/dump_cases/13-typescript-shapes.txt`: the captured output of every
  shape (tables, nested table, tree, 11 scalar kinds, maps, set, cycle, depth
  limit, and chaining). The dump parser's split-point tests run it too.
- New in-app `typescript` scenario, with six checks:
  - the plugin is bundled;
  - Node is ready;
  - a new playground uses the statements template;
  - Mod+Enter on the template renders the table with nothing on stderr;
  - hello world prints;
  - the program template prints and dumps.

  All 102 checks in nine scenarios pass on Windows. CI runs it on macOS and
  Linux.
- A screenshot showed text, the "people" table with nested cells, and the text
  after it, in order.

**Line numbers** (DEBUGGING.md D1, D10). With `--experimental-transform-types`
on Node 24.14, a throw on line 8 of a file using `enum` and parameter properties
reports `main.ts:8`. That holds for ES modules and CommonJS. Nothing drifts, so
Phase 6 doesn't need `--enable-source-maps` or strip-only mode for this plugin.

**Mod+Enter to table visible** (Windows 11, release build, template playground)

| Run | Time |
|---|---|
| First run after launch | 78 to 96 ms |
| Warm run | 66 to 78 ms |

## Checklist status

**Ticked with a note**
- *`min_version = "22.6"`.* It's 22.7, because `--experimental-transform-types`
  first shipped in Node 22.7. On 22.6 the run would fail with an unknown flag.
- *`NODE_PATH` set to `{plugin_dir}/dump`.* Not set. `NODE_PATH` is ignored for
  ES modules, and templates are ES modules. The Gotcha's fallback is used
  instead: a `--require` preload registers a resolve hook, because `--import`
  rejects Windows absolute paths. `--disable-warning=ExperimentalWarning` was
  added so every run doesn't print a warning to stderr.
- *Node 22.7 to 22.14.* The `module.register` fallback in `hooks.mjs` was checked
  by hiding `registerHooks` on Node 24. No real Node 22.x was tried.
