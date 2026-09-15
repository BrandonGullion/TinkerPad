# TypeScript plugin

Runs TypeScript (and plain JavaScript) playgrounds on Node.js, with `dump()` for
rich output.

## What it needs

Node.js 22.7 or newer on your `PATH`; the current LTS is best. Node runs `.ts`
files directly, so there is no build step and no `npm install`. If Node lives
somewhere unusual, point TinkerPad at it in `settings.toml`:

```toml
[toolchains.typescript]
node = "/opt/node/bin/node"
```

## Kinds

| Kind | Starts with |
|---|---|
| **TypeScript Statements** (default) | Top-level code, run from top to bottom. |
| **TypeScript Program** | A `main()` function and a call to it. |

Each playground is an ES module: its folder gets a `package.json` with
`"type": "module"`.

The run command is:

```sh
node --experimental-transform-types --disable-warning=ExperimentalWarning \
     --require <plugin>/dump/register.cjs main.ts
```

`--experimental-transform-types` also compiles `enum` and `namespace`.
Line numbers in stack traces still match `main.ts`.

## `dump`

```ts
import { dump } from "tinkerpad";

dump([{ name: "Ada", born: 1815 }, { name: "Grace", born: 1906 }], "people");
const total = dump(orders.map((o) => o.total)).length; // dump returns its argument
```

`dump(value, title?)` shows `value` in the output pane and returns it:

| Value | Shown as |
|---|---|
| A non-empty array whose items are all objects | A sortable table, with columns from every key in first-seen order |
| Any other object, array, `Map`, or `Set` | A collapsible tree |
| Anything else | A single value with its type |

Details:

- **Big values.** Tables keep 1,000 rows and mark themselves truncated. Arrays,
  maps, sets, and objects keep 1,000 items, then say how many more there were.
- **Nesting.** Five levels are shown; anything deeper becomes a summary such as
  `[Object]`. A reference back to an enclosing object shows as `"[circular]"`.
- **Values JSON can't hold.**
  - `undefined` becomes `null`, and `NaN` and `Infinity` become strings.
  - A `bigint` shows as `10n`, and a `Date` as its ISO string.
  - Functions show as `[Function: name]` and classes as `[class Name]`.
  - Errors show their name, message, and stack.
- **Ordering.** `console.log` output and dumps appear in the order they were
  written.

`dump` writes an escape sequence to stdout (`tinkerpad-dump-v1`, see
`docs/PLUGINS.md` §6). In a normal terminal it's invisible.

## Files

| Path | Purpose |
|---|---|
| `plugin.toml` | The manifest. |
| `templates/` | New playground contents. |
| `dump/tinkerpad.js`, `dump/tinkerpad.d.ts` | The dump library and its types. |
| `dump/register.cjs`, `dump/hooks.mjs` | Make `import ... from "tinkerpad"` find the library. `hooks.mjs` is only used on Node 22.7 to 22.14. |
