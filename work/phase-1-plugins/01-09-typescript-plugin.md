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

- [ ] `plugin.toml` with `[language]` (`id = "typescript"`, `extension = "ts"`,
      `highlighter = "typescript"`), `[toolchain]` probing `node --version` with
      `min_version = "22.6"`, two kinds, `[run]` = `node
      --experimental-transform-types {main}` with `NODE_PATH` set to `{plugin_dir}/dump`.
- [ ] Templates: `statements.ts` imports `dump` and has one example line;
      `program.ts` has a `main()` and calls it.
- [ ] `dump/tinkerpad.ts` (or `.js` plus `.d.ts`) exporting `dump(value, title?)`:
      arrays of objects → `table` (columns from the union of keys, in first-seen
      order); other objects/arrays → `tree`; primitives → `value`; `Map`/`Set`
      handled; cycles replaced with `"[circular]"`; depth limit 5; row cap 1000
      with `truncated: true`; `Date`, `BigInt`, `undefined`, functions serialised
      sensibly. Writes the OSC sequence with `process.stdout.write`.
- [ ] `dump` returns its argument so `dump(x).length` style chaining works.
- [ ] Node's `console.log` still goes through as plain text, interleaved correctly.
- [ ] Check and record here whether `--experimental-transform-types` preserves
      line numbers (strip-only mode does by design; transform mode should emit
      source maps). If line numbers drift for files using `enum`, note it: Phase 6
      needs `--enable-source-maps` or strip-only for breakpoints to land.
      (DEBUGGING.md D1, D10.)
- [ ] A `README.md` in the plugin folder: what it needs, what kinds exist, how to
      use `dump`.
- [ ] Captured output from `dump` of each shape added to 01-07's fixture set.
- [ ] Hello-world and a `dump` of an array of objects render correctly in the app.
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
