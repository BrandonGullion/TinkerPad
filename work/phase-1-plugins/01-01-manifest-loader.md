# 01-01 · Manifest schema and loader

**Phase:** 1 · **Size:** M · **Crate/area:** `tinkerpad-plugin` · **Depends on:** 00-01, 00-07
**Design:** PLUGINS.md §2, §4

## What

The Rust types for `plugin.toml` and a loader that turns a plugin folder into a
validated `Plugin` value. This is the public contract for plugin authors, so it
must be strict, well-typed, and produce error messages a contributor can act on.

## Why

Every other Phase 1 item reads from this. It is also the first public API of the
project: what a contributor sees when their manifest is wrong is the first
impression of the whole plugin system.

## Expectations

- [ ] `Manifest` struct tree mirroring PLUGINS.md §2 exactly: `api`, `[language]`,
      `[toolchain]`, `[[kind]]`, `[scaffold]`, `[run]`, `[helper]`, `[lsp]`,
      `[lsp.register]`, `[deps]`, `[output]`. `#[serde(deny_unknown_fields)]` on
      every *section* struct, but the top-level document collects unknown tables
      into a `HashMap` and logs a warning naming each. (DEBUGGING.md D6: a plugin
      can ship a `[debug]` section that older cores ignore. `plugin check` in
      05-01 treats unknown top-level tables as errors in strict mode so typos like
      `[lps]` are still caught.)
- [ ] `Plugin::load(dir) -> Result<Plugin, ManifestError>` reads and validates:
      `api == 1`; `id` matches `[a-z0-9-]+`; at least one kind, exactly one
      `default = true`; every `template` file exists; every `[scaffold].files`
      source exists; at least one of `[run]` or `[helper]`; `error_regex` compiles
      and names at least `line` and `message`; placeholders in commands are from
      the allowed set.
- [ ] Errors say which file, which key, and what was expected. An unknown key error
      includes the nearest valid key name (a small edit-distance helper) so
      `idle_shutdwon` says "did you mean idle_shutdown".
- [ ] Placeholder substitution (`{main}`, `{dir}`, `{id}`, `{name}`, `{scratch}`,
      `{plugin_dir}`, `{version}`, `{toolchain}` = the program of the probe that
      succeeded) is one function with tests; unknown placeholders fail at load
      time, not at run time.
- [ ] Per-OS variants: any `[toolchain].probe` entry, `[run]`, `[helper]`, and
      `[lsp]` may carry `os = ["windows" | "macos" | "linux", ...]`; sections
      may be repeated with different `os` lists and the loader picks the one
      matching the current OS (exactly one must match, or none with `os` and one
      without). Keeps a single manifest per plugin across platforms.
- [ ] Fixture tests: `tests/fixtures/valid/*` must load; `tests/fixtures/invalid/*`
      each must fail with the expected error variant (unknown key, missing
      template, two defaults, bad regex, bad placeholder).
- [ ] A `Manifest::example()` that renders the annotated C# example from PLUGINS.md
      §2, used by the docs and by 05-03.

## Out of scope

Discovery from folders (01-02), running anything (01-06), helper processes (03-01).

## Gotchas

Keep the schema types free of behaviour. `Plugin` (loaded, validated, with absolute
paths) is a different type from `Manifest` (raw TOML shape). Only `Plugin` leaves
this crate.
