// Preloaded with `node --require` (see plugin.toml). Resolves the bare
// specifier "tinkerpad" to this folder's dump library, for `import` and
// `require` alike. Everything else resolves as usual.

"use strict";

const path = require("node:path");
const { pathToFileURL } = require("node:url");
const nodeModule = require("node:module");

const LIBRARY = pathToFileURL(path.join(__dirname, "tinkerpad.js")).href;

if (typeof nodeModule.registerHooks === "function") {
  // Node 22.15 and later: synchronous, in-thread hooks.
  nodeModule.registerHooks({
    resolve(specifier, context, nextResolve) {
      if (specifier === "tinkerpad") return { url: LIBRARY, format: "module", shortCircuit: true };
      return nextResolve(specifier, context);
    },
  });
} else {
  // Node 22.7 to 22.14: the older off-thread hooks. `import` only.
  nodeModule.register(pathToFileURL(path.join(__dirname, "hooks.mjs")), {
    data: { library: LIBRARY },
  });
}
