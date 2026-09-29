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

## The language server

Completions, hover, signature help, and live errors come from
[typescript-language-server]. TinkerPad does not ship it, because it and the
TypeScript it analyses with are both npm packages that belong to you rather than
to the app.

The quickest way to get both is to let TinkerPad do it: the **Languages** button
in the status bar lists what each language is missing, and offers to install it
after showing you exactly what it would run (work item 04-08b). It installs into
TinkerPad's own folder rather than globally, so nothing needs elevation and
deleting `<app-data>/tools/typescript/` undoes it.

By hand is the same two steps:

```sh
npm install -g typescript-language-server
```

The server is started once for your whole library folder (`~/TinkerPad`) rather
than once per playground, which is what it prefers for loose files. The first
time it starts, TinkerPad writes two files there and never touches them again:

| File | Why |
|---|---|
| `tsconfig.json` | Without one, your `.ts` files are analysed as loose JavaScript with no types. |
| `package.json` | Declares the TypeScript the server analyses with. |

Run `npm install` in that folder once, and the server has its TypeScript — or
let the Languages button do it, which runs the same command in the same place:

```sh
cd ~/TinkerPad && npm install
```

Until you do, the server starts and then stops with "Could not find a valid
TypeScript installation", which the status strip shows. The editor keeps working
throughout — you lose completions, not the playground.

[typescript-language-server]: https://github.com/typescript-language-server/typescript-language-server

## The helper

TypeScript runs on a **warm helper** (work item 03-06): `helper/helper.mjs`, a
Node script that stays up between runs. There is no build step — it is started
with the same Node and the same flags a per-run execution would use, so a
playground behaves identically either way.

A run is a dynamic `import()` of your file with a cache-busting query, while
`process.stdout.write` and `process.stderr.write` are borrowed to turn what you
print into protocol messages. Node's start-up and its first type-strip pass are
paid for once a session instead of once a keystroke:

| | With the helper | Per-run (`node main.ts`) |
|---|---|---|
| Mod+Enter to first output | **about 5 ms** | about 45 ms |

Two limits are worth knowing, because both are visible:

**Cancel restarts the helper.** There is no safe way to stop a module that is
already running on this thread — no `Thread.Interrupt`, no `KeyboardInterrupt`.
So the helper says nothing when asked to cancel, and TinkerPad kills it after
two seconds and starts a fresh one. Your run stops, as it should; what you lose
is the warmth, for the next run only. (Running each playground in a worker
thread would make cancel work, at the cost of the sharing this exists for. It is
a fair trade to revisit if the wait annoys.)

**A playground that leaves something running holds the helper.** A server, an
interval, an unresolved promise: the process is shared, so they outlive the run
that started them. The helper notices and says so on its log — visible under
**Logs** in the status bar — and **Restart** on its chip clears it.

The helper also steps aside by itself every 500 runs, because the cache-busting
query means Node keeps every module it has ever loaded. You will not notice:
it says `retiring`, TinkerPad starts another, and no run is interrupted.

To compare against the per-run path, **Force per-run execution** in the command
palette turns the helper off for the session.

## Files

| Path | Purpose |
|---|---|
| `plugin.toml` | The manifest. |
| `templates/` | New playground contents. |
| `dump/tinkerpad.js`, `dump/tinkerpad.d.ts` | The dump library and its types. |
| `dump/register.cjs`, `dump/hooks.mjs` | Make `import ... from "tinkerpad"` find the library. `hooks.mjs` is only used on Node 22.7 to 22.14. |
| `helper/helper.mjs` | The warm runner. |
