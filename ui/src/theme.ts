// Themes (work item 04-04).
//
// The app's colours are CSS variables, two palettes' worth, chosen by the
// `data-theme` attribute on the document. CodeMirror can't read variables for
// everything it paints — a highlight style is a set of tags to colours, built
// once — so this module holds one per theme and swaps them when the theme
// changes, along with CodeMirror's own `dark` flag.
//
// `system` follows `prefers-color-scheme` as it changes, without a restart and
// without the user having chosen light or dark.

import { HighlightStyle } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";

export type Theme = "system" | "light" | "dark";

/** Whether a theme name is one we have. Anything else falls back to system. */
export function themeName(value: string | undefined): Theme {
  return value === "light" || value === "dark" || value === "system" ? value : "system";
}

/** The theme in force: what `system` resolves to right now. */
export function resolved(theme: Theme): "light" | "dark" {
  if (theme !== "system") return theme;
  return window.matchMedia("(prefers-color-scheme: light)").matches ? "light" : "dark";
}

/**
 * The colours a highlight style needs. They mirror the `--code-*` variables in
 * `style.css`; CodeMirror builds its rules once, so it gets the values rather
 * than the variables.
 */
interface Palette {
  keyword: string;
  string: string;
  number: string;
  comment: string;
  function: string;
  type: string;
  variable: string;
  constant: string;
  operator: string;
  invalid: string;
}

const DARK: Palette = {
  keyword: "#c792ea",
  string: "#a5d6a7",
  number: "#f5b97a",
  comment: "#8a8d96",
  function: "#82aaff",
  type: "#7fd1c8",
  variable: "#d4d4d8",
  constant: "#f78c6c",
  operator: "#b0b4bd",
  invalid: "#f87171",
};

const LIGHT: Palette = {
  keyword: "#7c3aed",
  string: "#0f7a3d",
  number: "#9a4a00",
  comment: "#6b7078",
  function: "#1553c4",
  type: "#0f6f6a",
  variable: "#1f2124",
  constant: "#a33800",
  operator: "#4a4f57",
  invalid: "#b3261e",
};

/**
 * One highlight style per theme, covering what the bundled highlighters emit:
 * the Lezer standard tags, which every `@codemirror/lang-*` grammar and the
 * legacy modes map onto.
 */
function styleFor(palette: Palette): HighlightStyle {
  return HighlightStyle.define([
    { tag: [t.keyword, t.modifier, t.controlKeyword, t.moduleKeyword], color: palette.keyword },
    { tag: [t.string, t.special(t.string), t.regexp], color: palette.string },
    { tag: [t.number, t.bool, t.null], color: palette.number },
    { tag: [t.comment, t.lineComment, t.blockComment, t.docComment], color: palette.comment, fontStyle: "italic" },
    { tag: [t.function(t.variableName), t.function(t.propertyName), t.macroName], color: palette.function },
    { tag: [t.typeName, t.className, t.namespace, t.annotation], color: palette.type },
    { tag: [t.variableName, t.propertyName, t.attributeName], color: palette.variable },
    { tag: [t.constant(t.variableName), t.standard(t.variableName), t.atom], color: palette.constant },
    { tag: [t.operator, t.operatorKeyword, t.punctuation, t.separator, t.bracket], color: palette.operator },
    { tag: [t.invalid], color: palette.invalid },
    { tag: [t.heading], color: palette.function, fontWeight: "bold" },
    { tag: [t.link, t.url], color: palette.function, textDecoration: "underline" },
    { tag: [t.emphasis], fontStyle: "italic" },
    { tag: [t.strong], fontWeight: "bold" },
    { tag: [t.strikethrough], textDecoration: "line-through" },
  ]);
}

const STYLES = { dark: styleFor(DARK), light: styleFor(LIGHT) };

export function highlightStyle(theme: "light" | "dark"): HighlightStyle {
  return STYLES[theme];
}

/** Called when the theme in force changes, for whatever reason. */
type Listener = (theme: "light" | "dark") => void;

const listeners = new Set<Listener>();
let chosen: Theme = "system";

export function onThemeChanged(listener: Listener): void {
  listeners.add(listener);
}

/** What the window is showing now. */
export function currentTheme(): "light" | "dark" {
  return resolved(chosen);
}

/**
 * Apply a theme: stamp it on the document, and tell whoever paints their own
 * colours (CodeMirror) what it came out as.
 */
export function applyTheme(theme: Theme): void {
  chosen = theme;
  document.documentElement.dataset.theme = theme;
  const now = resolved(theme);
  for (const listener of listeners) listener(now);
}

/**
 * Follow the system while the theme is `system`. The listener stays for the
 * life of the window: the setting can change back at any time.
 */
export function followSystem(): void {
  const query = window.matchMedia("(prefers-color-scheme: light)");
  query.addEventListener("change", () => {
    if (chosen === "system") applyTheme("system");
  });
}
