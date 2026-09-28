// `html` dumps: sanitised with DOMPurify, shown in an isolated, clipped box.
//
// DOMPurify is an npm dependency, so Vite bundles it into the app: nothing is
// fetched at runtime. On top of its defaults this removes anything that could
// load from outside (the CSP blocks those too) or change the app itself: forms,
// frames, media, <style>, links, and styles with url(). The result goes into a
// shadow root so the app's CSS and the dump's inline styles can't meet, and the
// host clips painting so `position: fixed` can't cover the app.

import DOMPurify, { type Config } from "dompurify";

import { el } from "./json";

const URL_ATTRS = new Set(["src", "href", "xlink:href", "action", "formaction", "poster", "background", "ping"]);

const CONFIG: Config = {
  FORBID_TAGS: [
    "style",
    "link",
    "base",
    "meta",
    "iframe",
    "frame",
    "frameset",
    "object",
    "embed",
    "form",
    "input",
    "button",
    "textarea",
    "select",
    "video",
    "audio",
    "source",
    "track",
    "picture",
  ],
  FORBID_ATTR: ["srcset"],
  ALLOW_DATA_ATTR: false,
};

let hooked = false;

function purifier(): typeof DOMPurify {
  if (!hooked) {
    DOMPurify.addHook("uponSanitizeAttribute", (_node, data) => {
      const name = data.attrName.toLowerCase();
      if (URL_ATTRS.has(name)) {
        // Inline images may stay; everything else would load or navigate.
        const inlineImage = name === "src" && /^data:image\/(png|jpe?g|gif|webp|bmp|avif);/i.test(data.attrValue.trim());
        if (!inlineImage) data.keepAttr = false;
      } else if (name === "style" && /url\s*\(|@import|expression\s*\(/i.test(data.attrValue)) {
        data.keepAttr = false;
      }
    });
    hooked = true;
  }
  return DOMPurify;
}

/** Sanitise `html` into a fragment ready to insert. */
export function sanitize(html: string): DocumentFragment {
  return purifier().sanitize(html, { ...CONFIG, RETURN_DOM_FRAGMENT: true });
}

const BASE_CSS = `
:host { display: block; }
.root { font: 13px/1.45 var(--font-ui); color: var(--fg); }
.root > :first-child { margin-top: 0; }
.root > :last-child { margin-bottom: 0; }
a { color: var(--link); }
img { max-width: 100%; }
table { border-collapse: collapse; }
td, th { border: 1px solid var(--border); padding: 2px 6px; }
`;

export function renderHtml(html: string): HTMLElement {
  const host = el("div", "dump-html");
  const shadow = host.attachShadow({ mode: "open" });
  const style = el("style");
  style.textContent = BASE_CSS;
  const root = el("div", "root");
  root.append(sanitize(html));
  shadow.append(style, root);
  return host;
}
