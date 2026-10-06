# The plugin index

`index.json` is the list of TinkerPad language plugins. The window's **Browse**
tab reads it; adding to it is a pull request.

That is the whole mechanism. No server, no accounts, no uploads — an entry is a
name and a git URL, and installing one does exactly what typing that URL into
`tinkerpad plugin install` does, trust notice and all. Nothing is hosted here;
your plugin stays in your repository and you keep owning it.

## Adding yours

1. Make sure `tinkerpad plugin check .` passes in your plugin's folder.
2. Add an entry to `plugins` in `index.json`, in id order.
3. Open a pull request. Paste the `check` output into it — the template asks.

```json
{
  "id": "elm",
  "name": "Elm",
  "description": "Elm playgrounds, with elm-format on save.",
  "git": "https://github.com/someone/tinkerpad-elm",
  "languages": ["elm"],
  "maintainer": "someone",
  "verified_with": "0.2.0"
}
```

| Field | | |
|---|---|---|
| `id` | required | The `[language].id` from your manifest. Lowercase, digits and hyphens. Unique here. |
| `name` | required | What it calls itself. |
| `description` | required | One line, 120 characters or fewer. |
| `git` | required\* | What `tinkerpad plugin install` is given. `https://` or `ssh://`. |
| `languages` | | The languages it adds, for searching. Usually one. |
| `maintainer` | required | Who to open an issue with. |
| `verified_with` | | The TinkerPad version its `check` last passed on. The weekly job keeps this honest. |
| `bundled` | | Shipped with TinkerPad. Needs no `git`. |

\* unless `bundled`.

`index.schema.json` is generated from the types the app reads the index with,
so it describes what this build actually accepts. Point your editor at it.

## What is checked

`tinkerpad registry check` runs on every pull request. It reports **every**
problem at once rather than one per push: a missing field, a description that
is a paragraph, an id listed twice, a URL that isn't fetchable, an unknown
field (a typo should fail, not be silently dropped).

A weekly job clones every entry and runs `tinkerpad plugin check` on it,
writing the result to `status.json`. An entry that stops working is marked
there rather than removed — a plugin that is broken today may be fixed
tomorrow, and deleting someone's entry because their CI was down is rude.

## What is deliberately not here

Ratings, download counts, hosted bundles, signatures. Each of those needs
somebody to maintain it, and a number nobody maintains is worse than no number.
If the index ever needs more than a JSON file and a pull request, that will be
because this one is too popular — a good problem, and a later one.
