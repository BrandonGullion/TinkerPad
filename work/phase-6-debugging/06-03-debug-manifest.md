# 06-03 · `[debug]` in the manifest

**Phase:** 6 · **Size:** S · **Crate/area:** `tinkerpad-plugin` · **Depends on:** 01-01, 06-01
**Design:** DEBUGGING.md §1, D6, D14

## What

What a plugin declares to make its language debuggable: the adapter to run, how
a session starts, and the configuration the adapter expects.

```toml
[debug]
adapter = "js-debug"                 # resolved like every other program
args = ["--server={port}"]
mode = "attach"                      # or "launch"
# Passed to the adapter as the launch/attach request's arguments. The
# placeholders a run gets, plus {port} and {pid}.
config = { type = "pwa-node", cwd = "{dir}", program = "{main}" }
```

## Why

Nothing language-specific goes in the core, debugging included. D6 made an
unknown top-level table a warning rather than an error precisely so a plugin can
carry `[debug]` on a core that predates it.

## Expectations

- [ ] Validated like every other section: unknown keys are errors with a
      suggestion, per-OS variants as `[[debug]]`, paths relative and inside the
      plugin folder.
- [ ] `mode = "attach"` makes `{port}` and `{pid}` available; `launch` does not.
      Using one where it has no meaning fails to load, rather than at a
      breakpoint an hour later.
- [ ] The `config` table is passed through untouched beyond placeholder
      substitution. It is the adapter's schema, not TinkerPad's, and pretending
      otherwise would mean tracking five of them and being wrong about each.
- [ ] A plugin with no `[debug]` is not an error. The window says debugging
      isn't available for that language rather than offering a button that
      fails.
- [ ] The generated manifest reference (05-04) picks the section up with no
      extra work. If it doesn't, that generator was not generating.
- [ ] PLUGINS.md gains a section, and `plugin-template/plugin.toml` a commented
      example, so the next plugin author sees it.

## Out of scope

Running a session with it (06-04). Whether any particular adapter works (06-07
onward).

## Gotchas

D14: a launch-mode adapter runs the *built artifact*, not the build command.
Rust and C# will want a `[build]` separate from `[run]`. Decide in 06-10 whether
that split belongs in the manifest or whether `[debug].config` naming the
artifact is enough — do not pre-emptively split `[run]` here, and do not design
for a need that has not arrived.
