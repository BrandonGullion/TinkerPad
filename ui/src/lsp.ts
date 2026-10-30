// The editor's side of a language server (work item 02-04).
//
// `@codemirror/lsp-client` speaks LSP over a `Transport`; here that transport
// is a Tauri channel to the core, which does the real talking (02-03). The
// core answers this client's `initialize` from the handshake it already did
// and routes replies back to the tab that asked, so every tab can run a client
// of its own while one server serves them all. Document sync is this client's,
// numbering included: the server echoes those version numbers back with its
// diagnostics, and the client checks them.
//
// Nothing here blocks the editor: a tab opens and is usable immediately, and
// the language features appear when the server is ready. A server that never
// starts costs the tab nothing but the absence of completions.

import { acceptCompletion } from "@codemirror/autocomplete";
import {
  LSPClient,
  LSPPlugin,
  findReferencesKeymap,
  formatKeymap,
  hoverTooltips,
  jumpToDefinitionKeymap,
  renameKeymap,
  serverCompletion,
  serverDiagnostics,
  signatureHelp,
  type Transport,
} from "@codemirror/lsp-client";
import { lintGutter } from "@codemirror/lint";
import { Prec, StateEffect, StateField, type Extension } from "@codemirror/state";
import { type Command, keymap, showTooltip, type Tooltip } from "@codemirror/view";
import type * as lsp from "vscode-languageserver-protocol";
import { Channel } from "@tauri-apps/api/core";
import DOMPurify from "dompurify";

import { highlighter } from "./highlight";
import { invoke } from "./ipc";

/** How long a request to the server may take before the client gives up. */
const TIMEOUT_MS = 10_000;

/** One tab's connection to its language server. */
export interface LspSession {
  /** The URI the core gave this document. */
  readonly uri: string;
  /** The editor extensions that make the connection do something. */
  readonly extensions: Extension;
  /** Disconnect: cancels in-flight requests and closes the document. */
  close(): Promise<void>;
}

/**
 * A `Transport` over the Tauri channel for one tab.
 *
 * Messages are JSON on both sides: the client hands us a string, the core
 * takes and gives objects, and the channel keeps them in order.
 */
class TauriTransport implements Transport {
  private readonly handlers = new Set<(value: string) => void>();
  private closed = false;

  constructor(private readonly playground: string) {}

  /** The channel the core sends this tab's messages on. */
  channel(): Channel<unknown> {
    const channel = new Channel<unknown>();
    channel.onmessage = (message) => {
      if (this.closed) return;
      const text = JSON.stringify(message);
      for (const handler of this.handlers) handler(text);
    };
    return channel;
  }

  send(message: string): void {
    if (this.closed) throw new Error("the language server connection is closed");
    // Fire and forget: the reply comes back on the channel, not from the
    // command. A failure here means the tab is gone, which the client finds
    // out from its own request timeouts.
    void invoke("lsp_send", {
      playground: this.playground,
      message: JSON.parse(message) as unknown,
    }).catch(() => {});
  }

  subscribe(handler: (value: string) => void): void {
    this.handlers.add(handler);
  }

  unsubscribe(handler: (value: string) => void): void {
    this.handlers.delete(handler);
  }

  stop(): void {
    this.closed = true;
    this.handlers.clear();
  }
}

/**
 * Connect a tab to its language server, if its plugin has one.
 *
 * Returns `null` when the language has no server, the server binary is
 * missing, or anything else goes wrong: the editor works the same, only
 * without completions, and the status bar says why (02-08).
 */
export async function connect(
  playground: string,
  languageId: string,
  extras: Extension = [],
): Promise<LspSession | null> {
  const transport = new TauriTransport(playground);
  let uri: string;
  try {
    uri = await invoke<string>("lsp_open", {
      playground,
      channel: transport.channel(),
    });
  } catch {
    // The toast from `invoke` is enough; a playground without a language
    // server is a normal thing, not an error the user must act on.
    transport.stop();
    return null;
  }

  const client = new LSPClient({
    timeout: TIMEOUT_MS,
    // Server documentation is Markdown, which may carry HTML.
    sanitizeHTML: (html) => DOMPurify.sanitize(html),
    highlightLanguage: () => null,
    // What `languageServerExtensions()` gives, except that the server's
    // completions replace the language's own word list rather than joining
    // it: C#'s highlighter offers every keyword and type name, which put
    // `abstract` and `Action` at the top after `Console.` (04-13).
    extensions: [
      serverCompletion({ override: true }),
      hoverTooltips(),
      keymap.of([...formatKeymap, ...renameKeymap, ...jumpToDefinitionKeymap, ...findReferencesKeymap]),
      signatureHelp(),
      serverDiagnostics(),
    ],
  }).connect(transport);

  return {
    uri,
    // The client's own extensions bring completion, hover on the mouse,
    // signature help, and diagnostics; the rest is this app's keyboard.
    extensions: [
      client.plugin(uri, languageId),
      lintGutter(),
      hoverKeymap,
      acceptWithTab,
      extras,
    ],
    async close() {
      transport.stop();
      client.disconnect();
      await invoke("lsp_close", { playground }).catch(() => {});
    },
  };
}

/**
 * Hover on demand: `Mod+K Mod+I`, the same keys VS Code uses.
 *
 * `hoverTooltips()` already answers the mouse. This is the keyboard's way in,
 * for people who don't want to take their hands off the keys — and the only
 * way to read documentation while typing an argument.
 */
const hoverTooltip = StateEffect.define<Tooltip | null>();

const cursorHover = StateField.define<Tooltip | null>({
  create: () => null,
  update(value, tr) {
    for (const effect of tr.effects) if (effect.is(hoverTooltip)) return effect.value;
    // Any edit or move makes the tooltip stale, so it goes.
    return tr.docChanged || tr.selection ? null : value;
  },
  provide: (field) => showTooltip.from(field),
});

/** Ask the server what is under the cursor and show it. */
const showHoverAtCursor: Command = (view) => {
  const plugin = LSPPlugin.get(view);
  if (!plugin) return false;
  const pos = view.state.selection.main.head;
  plugin.client.sync();
  void plugin.client
    .request<lsp.HoverParams, lsp.Hover | null>("textDocument/hover", {
      textDocument: { uri: plugin.uri },
      position: plugin.toPosition(pos),
    })
    .then((result) => {
      // `contents` may be a string, a `{language, value}` pair, an array of
      // either, or `MarkupContent`. Servers use all of them.
      const text = (part: lsp.MarkedString | lsp.MarkupContent): string =>
        typeof part === "string" ? part : "value" in part ? part.value : "";
      const contents = result?.contents;
      const markup = !contents
        ? ""
        : Array.isArray(contents)
          ? contents.map(text).join("\n\n")
          : text(contents);
      if (!markup.trim()) {
        view.dispatch({ effects: hoverTooltip.of(null) });
        return;
      }
      // Servers write hover text in Markdown, whether or not they say so.
      const html = plugin.docToHTML({ kind: "markdown", value: markup });
      view.dispatch({
        effects: hoverTooltip.of({
          pos,
          above: true,
          create: () => {
            const dom = document.createElement("div");
            dom.className = "cm-lsp-hover cm-tooltip-hover";
            dom.innerHTML = html;
            return { dom };
          },
        }),
      });
    })
    .catch(() => {
      view.dispatch({ effects: hoverTooltip.of(null) });
    });
  return true;
};

/** Hover from the keyboard, and Escape to dismiss it. */
const hoverKeymap: Extension = [
  cursorHover,
  Prec.high(
    keymap.of([
      { key: "Mod-k Mod-i", run: showHoverAtCursor },
      {
        key: "Escape",
        run: (view) => {
          if (!view.state.field(cursorHover, false)) return false;
          view.dispatch({ effects: hoverTooltip.of(null) });
          return true;
        },
      },
    ]),
  ),
];

/**
 * Tab accepts the highlighted completion. CodeMirror binds Enter by default;
 * Tab is what most editors also accept with, and it only takes precedence
 * while the popup is open — otherwise it still indents.
 */
const acceptWithTab: Extension = Prec.highest(
  keymap.of([{ key: "Tab", run: acceptCompletion }]),
);

/**
 * Everything a tab's editor needs for its language: the highlighter, and the
 * language server if there is one. The highlighter is ready in milliseconds;
 * the server may take seconds, so they are awaited separately by the caller.
 */
export async function languageExtensions(
  playground: string,
  highlighterName: string | null,
  hasServer: boolean,
  extras: Extension = [],
): Promise<{ extensions: Extension; session: LspSession | null }> {
  const [syntax, session] = await Promise.all([
    highlighter(highlighterName),
    hasServer ? connect(playground, highlighterName ?? "plaintext", extras) : Promise.resolve(null),
  ]);
  return {
    extensions: [syntax ?? [], session?.extensions ?? []],
    session,
  };
}
