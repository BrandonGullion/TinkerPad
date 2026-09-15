// Syntax highlighting for `text` dumps with a `lang`.
//
// Uses CodeMirror's Lezer grammars outside the editor. Only JSON is bundled so
// far; the editor's per-language highlighters (work item 02-04) register here
// too, so dumps and the editor highlight alike.

import { jsonLanguage } from "@codemirror/lang-json";
import type { Language } from "@codemirror/language";
import { classHighlighter, highlightTree } from "@lezer/highlight";

const languages = new Map<string, Language>([["json", jsonLanguage]]);

/** Past this size a text dump is shown plain; highlighting would stall. */
const MAX_HIGHLIGHT_CHARS = 200_000;

export function registerHighlighter(names: string[], language: Language): void {
  for (const name of names) languages.set(name.toLowerCase(), language);
}

export function hasHighlighter(lang: string | undefined): boolean {
  return lang !== undefined && languages.has(lang.toLowerCase());
}

/** Fill `target` with `code`, highlighted when a grammar for `lang` exists. */
export function highlightInto(target: HTMLElement, code: string, lang: string | undefined): boolean {
  const language = lang ? languages.get(lang.toLowerCase()) : undefined;
  if (!language || code.length > MAX_HIGHLIGHT_CHARS) {
    target.textContent = code;
    return false;
  }
  const tree = language.parser.parse(code);
  const fragment = document.createDocumentFragment();
  let pos = 0;
  highlightTree(tree, classHighlighter, (from, to, classes) => {
    if (from > pos) fragment.append(code.slice(pos, from));
    const span = document.createElement("span");
    span.className = classes;
    span.textContent = code.slice(from, to);
    fragment.append(span);
    pos = to;
  });
  if (pos < code.length) fragment.append(code.slice(pos));
  target.replaceChildren(fragment);
  return true;
}
