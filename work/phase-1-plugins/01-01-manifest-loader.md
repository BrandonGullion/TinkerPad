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

- [x] `Manifest` struct tree mirroring PLUGINS.md §2 exactly: `api`, `[language]`,
      `[toolchain]`, `[[kind]]`, `[scaffold]`, `[run]`, `[helper]`, `[lsp]`,
      `[lsp.register]`, `[deps]`, `[output]`. `#[serde(deny_unknown_fields)]` on
      every *section* struct, but the top-level document collects unknown tables
      into a `HashMap` and logs a warning naming each. (DEBUGGING.md D6: a plugin
      can ship a `[debug]` section that older cores ignore. `plugin check` in
      05-01a treats unknown top-level tables as errors in strict mode so typos like
      `[lps]` are still caught.)
- [x] `Plugin::load(dir) -> Result<Plugin, ManifestError>` reads and validates:
      `api == 1`; `id` matches `[a-z0-9-]+`; at least one kind, exactly one
      `default = true`; every `template` file exists; every `[scaffold].files`
      source exists; at least one of `[run]` or `[helper]`; `error_regex` compiles
      and names at least `line` and `message`; placeholders in commands are from
      the allowed set.
- [x] Errors say which file, which key, and what was expected. An unknown key error
      includes the nearest valid key name (a small edit-distance helper) so
      `idle_shutdwon` says "did you mean idle_shutdown".
- [x] Placeholder substitution (`{main}`, `{dir}`, `{id}`, `{name}`, `{scratch}`,
      `{plugin_dir}`, `{version}`, `{toolchain}` = the program of the probe that
      succeeded) is one function with tests; unknown placeholders fail at load
      time, not at run time.
- [x] Per-OS variants: any `[toolchain].probe` entry, `[run]`, `[helper]`, and
      `[lsp]` may carry `os = ["windows" | "macos" | "linux", ...]`; sections
      may be repeated with different `os` lists and the loader picks the one
      matching the current OS (exactly one must match, or none with `os` and one
      without). Keeps a single manifest per plugin across platforms.
- [x] Fixture tests: `tests/fixtures/valid/*` must load; `tests/fixtures/invalid/*`
      each must fail with the expected error variant (unknown key, missing
      template, two defaults, bad regex, bad placeholder).
- [x] A `Manifest::example()` that renders the annotated C# example from PLUGINS.md
      §2, used by the docs and by 05-03.

## Out of scope

Discovery from folders (01-02), running anything (01-06), helper processes (03-01).

## Gotchas

Keep the schema types free of behaviour. `Plugin` (loaded, validated, with absolute
paths) is a different type from `Manifest` (raw TOML shape). Only `Plugin` leaves
this crate.

## Results

Implemented 2026-09-14 in `crates/tinkerpad-plugin`. 36 tests: 12 unit tests
(placeholders, suggestions, durations, versions, locations) and 24 fixture tests
over 4 valid and 15 invalid plugin folders.

**What an author sees.** Real output for some of the invalid fixtures:

```
invalid/unknown-key/plugin.toml:22:1: in [lsp]: unknown key `idle_shutdwon`; did you mean `idle_shutdown`? (expected one of `command`, `args`, `env`, `workspace`, `idle_shutdown`, `init_options`, `register`, `os`)

invalid/wrong-os-name/plugin.toml:9:33: in [toolchain]: unknown value `mac`; did you mean `macos`? (expected one of `windows`, `macos`, `linux`)

invalid/bad-placeholder/plugin.toml: 1 problem
  - run.args[0]: unknown placeholder `{mian}`; did you mean `{main}`? allowed here: {main}, {dir}, {id}, {name}, {scratch}, {plugin_dir}, {version}, {toolchain}

invalid/unsafe-paths/plugin.toml: 3 problems
  - language.main_path: `../escape.x` must be a relative path that stays inside its folder
  - kind[0].template: `C:/Windows/win.ini` must be a relative path that stays inside its folder
  - scaffold.files[0].template: templates/missing.toml does not exist
```

**Deviations on ticked criteria**
- *Top-level unknown tables collected into a `HashMap`.* The file is parsed twice
  instead: a loose pass into a `toml::Table` finds unknown top-level sections and
  checks `api`, then a strict pass fills the schema. Collecting extras with
  `#[serde(flatten)]` would have buffered the whole document and lost line
  numbers from every error inside it.
- *`Manifest::example()`.* The gotcha says only `Plugin` leaves the crate, so the
  raw `Manifest` type stays private and the example is
  `tinkerpad_plugin::example_manifest()`. A test keeps it identical to the block
  in `docs/PLUGINS.md` §2, and another loads it as a real plugin.
- *"Exactly one must match, or none with `os` and one without".* Implemented as:
  an entry listing the OS wins over an entry without `os`. It's an error if two
  entries list the same OS, or if more than one entry omits `os`. All three
  operating systems are checked on every load.

**Beyond the criteria, needed to make the rest work**
- Repeated sections are TOML arrays of tables (`[[run]]`); a single `[run]` still
  works. A hand-written deserializer accepts either while keeping error locations.
- Placeholders are validated per context: `[deps]` uses `{name}` and `{version}`
  for the package, and probes only get `{plugin_dir}` and `{scratch}`.
  `{{` and `}}` are literal braces.
- Keys the spec needed but didn't list: `[language].main_path` (01-11),
  `[helper].args` and `env` (03-06), `[lsp].args` and `env`,
  `[output].error_regex_multiline` (01-13), and the `[scaffold].files` and
  `[deps]` shapes. All are documented in `docs/PLUGINS.md` §2.1.
- Every validation problem is collected and reported together, not just the
  first.
- Paths are rejected if absolute, drive-prefixed, or containing `..`, on every OS.
- Warnings rather than errors for an unknown highlighter, an unknown helper hook,
  an unused regex group, and an OS with no runner other than the current one.
