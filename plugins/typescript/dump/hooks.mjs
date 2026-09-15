// Resolve hook for Node versions without `module.registerHooks` (22.7 to
// 22.14). Loaded by register.cjs through `module.register`.

let library;

export function initialize(data) {
  library = data.library;
}

export async function resolve(specifier, context, nextResolve) {
  if (specifier === "tinkerpad") return { url: library, format: "module", shortCircuit: true };
  return nextResolve(specifier, context);
}
