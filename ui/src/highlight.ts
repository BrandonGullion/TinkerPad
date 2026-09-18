// Syntax highlighting per language, loaded only when a playground needs it.
//
// A plugin names its highlighter in `[language].highlighter` (docs/PLUGINS.md
// §9). Each one is a dynamic import, so a language's grammar is a chunk of its
// own: opening a TypeScript playground never downloads the Rust parser, and
// the startup bundle carries none of them.

import { StreamLanguage } from "@codemirror/language";
import type { Extension } from "@codemirror/state";

/** The highlighters PLUGINS.md §9 promises, and how to load each one. */
const LANGUAGES: Record<string, () => Promise<Extension>> = {
  javascript: async () => (await import("@codemirror/lang-javascript")).javascript(),
  typescript: async () =>
    (await import("@codemirror/lang-javascript")).javascript({ typescript: true }),
  jsx: async () => (await import("@codemirror/lang-javascript")).javascript({ jsx: true }),
  rust: async () => (await import("@codemirror/lang-rust")).rust(),
  python: async () => (await import("@codemirror/lang-python")).python(),
  go: async () => (await import("@codemirror/lang-go")).go(),
  json: async () => (await import("@codemirror/lang-json")).json(),
  markdown: async () => (await import("@codemirror/lang-markdown")).markdown(),
  sql: async () => (await import("@codemirror/lang-sql")).sql(),
  yaml: async () => (await import("@codemirror/lang-yaml")).yaml(),
  cpp: async () => (await import("@codemirror/lang-cpp")).cpp(),
  java: async () => (await import("@codemirror/lang-java")).java(),
  // Languages without a full parser package use the legacy stream parsers,
  // which highlight well enough and cost a few kilobytes.
  csharp: async () =>
    StreamLanguage.define((await import("@codemirror/legacy-modes/mode/clike")).csharp),
  kotlin: async () =>
    StreamLanguage.define((await import("@codemirror/legacy-modes/mode/clike")).kotlin),
  scala: async () =>
    StreamLanguage.define((await import("@codemirror/legacy-modes/mode/clike")).scala),
  shell: async () =>
    StreamLanguage.define((await import("@codemirror/legacy-modes/mode/shell")).shell),
  lua: async () => StreamLanguage.define((await import("@codemirror/legacy-modes/mode/lua")).lua),
  swift: async () =>
    StreamLanguage.define((await import("@codemirror/legacy-modes/mode/swift")).swift),
  ruby: async () =>
    StreamLanguage.define((await import("@codemirror/legacy-modes/mode/ruby")).ruby),
  toml: async () =>
    StreamLanguage.define((await import("@codemirror/legacy-modes/mode/toml")).toml),
};

/** Every highlighter name a plugin may ask for. */
export function highlighters(): string[] {
  return Object.keys(LANGUAGES).sort();
}

/** File extensions, for a file with no plugin behind it (peek tabs, 02-09). */
const BY_EXTENSION: Record<string, string> = {
  ts: "typescript", mts: "typescript", cts: "typescript", tsx: "typescript",
  js: "javascript", mjs: "javascript", cjs: "javascript", jsx: "jsx",
  rs: "rust", py: "python", go: "go", json: "json", jsonc: "json",
  md: "markdown", markdown: "markdown", sql: "sql", yaml: "yaml", yml: "yaml",
  toml: "toml", c: "cpp", h: "cpp", cc: "cpp", cpp: "cpp", hpp: "cpp",
  java: "java", cs: "csharp", kt: "kotlin", kts: "kotlin", scala: "scala",
  sh: "shell", bash: "shell", zsh: "shell", lua: "lua", swift: "swift",
  rb: "ruby",
};

/** The highlighter for a file extension, without the dot. */
export function highlighterFor(extension: string): Promise<Extension | null> {
  return highlighter(BY_EXTENSION[extension.toLowerCase().replace(/^./, "")]);
}

const loaded = new Map<string, Promise<Extension | null>>();

/**
 * The editor extension for a highlighter name, or `null` for a language
 * TinkerPad has no grammar for — such a playground is plain text, which is
 * still perfectly usable.
 *
 * Each grammar is loaded once and shared by every tab in that language.
 */
export function highlighter(name: string | null | undefined): Promise<Extension | null> {
  if (!name) return Promise.resolve(null);
  const key = name.toLowerCase();
  const load = LANGUAGES[key];
  if (!load) {
    console.info(`no bundled highlighter for "${name}"; showing plain text`);
    return Promise.resolve(null);
  }
  let pending = loaded.get(key);
  if (!pending) {
    pending = load().catch((error: unknown) => {
      console.warn(`could not load the ${key} highlighter`, error);
      return null;
    });
    loaded.set(key, pending);
  }
  return pending;
}
