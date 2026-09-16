// A minimal language server for the self-tests, in the language the test
// plugin already needs (Node). Not a real server for a real language: it
// exists so the app's LSP path — pool, proxy, editor client, pre-warm, idle —
// can be exercised on any machine and in CI, with no external install.
//
// It speaks Content-Length framing over stdio and answers:
//   initialize            capabilities: completion (trigger "."), hover
//   textDocument/didOpen  remembers the text, publishes a diagnostic for
//                         every line containing the word "broken"
//   textDocument/didChange  same, from the new text
//   textDocument/completion  three items, one of them named after the file
//   textDocument/hover    the word under the cursor, as Markdown
//   shutdown / exit       stops

"use strict";

const docs = new Map();

function send(message) {
  const body = JSON.stringify(message);
  process.stdout.write(`Content-Length: ${Buffer.byteLength(body)}\r\n\r\n${body}`);
}

function reply(id, result) {
  if (id !== undefined && id !== null) send({ jsonrpc: "2.0", id, result });
}

/** One diagnostic per line holding the word "broken". */
function publish(uri) {
  const text = docs.get(uri) ?? "";
  const diagnostics = [];
  text.split("\n").forEach((line, n) => {
    const at = line.indexOf("broken");
    if (at >= 0) {
      diagnostics.push({
        range: {
          start: { line: n, character: at },
          end: { line: n, character: at + "broken".length },
        },
        severity: 1,
        source: "node-check",
        message: "this line is broken",
      });
    }
  });
  send({
    jsonrpc: "2.0",
    method: "textDocument/publishDiagnostics",
    params: { uri, diagnostics },
  });
}

function handle(message) {
  const { id, method, params } = message;
  switch (method) {
    case "initialize":
      reply(id, {
        capabilities: {
          textDocumentSync: 1,
          completionProvider: { triggerCharacters: ["."] },
          hoverProvider: true,
        },
        serverInfo: { name: "node-check-server", version: "1", pid: process.pid },
      });
      break;
    case "textDocument/didOpen":
      docs.set(params.textDocument.uri, params.textDocument.text ?? "");
      publish(params.textDocument.uri);
      break;
    case "textDocument/didChange": {
      const uri = params.textDocument.uri;
      for (const change of params.contentChanges ?? []) {
        if (change.range === undefined) docs.set(uri, change.text);
      }
      publish(uri);
      break;
    }
    case "textDocument/didClose":
      docs.delete(params.textDocument.uri);
      break;
    case "textDocument/completion": {
      const name = (params.textDocument.uri.split("/").pop() ?? "file").replace(/\W/g, "");
      reply(id, [
        { label: `${name}Value`, kind: 6, detail: "from the test server" },
        { label: "alpha", kind: 3, detail: "a function", documentation: "The **first** one." },
        { label: "beta", kind: 3, detail: "a function" },
      ]);
      break;
    }
    case "textDocument/hover": {
      const text = docs.get(params.textDocument.uri) ?? "";
      const line = text.split("\n")[params.position.line] ?? "";
      const word = /[A-Za-z_][A-Za-z0-9_]*/.exec(line.slice(params.position.character))?.[0] ?? "";
      reply(id, {
        contents: { kind: "markdown", value: `\`${word}\`\n\nSeen by the test server.` },
      });
      break;
    }
    case "shutdown":
      reply(id, null);
      break;
    case "exit":
      process.exit(0);
      break;
    default:
      if (id !== undefined && id !== null) {
        send({
          jsonrpc: "2.0",
          id,
          error: { code: -32601, message: `no method ${method}` },
        });
      }
  }
}

// Read frames from stdin: headers, a blank line, then that many bytes.
let buffer = Buffer.alloc(0);
process.stdin.on("data", (chunk) => {
  buffer = Buffer.concat([buffer, chunk]);
  for (;;) {
    const end = buffer.indexOf("\r\n\r\n");
    if (end < 0) return;
    const header = buffer.subarray(0, end).toString("ascii");
    const match = /content-length:\s*(\d+)/i.exec(header);
    if (!match) {
      buffer = buffer.subarray(end + 4);
      continue;
    }
    const length = Number(match[1]);
    if (buffer.length < end + 4 + length) return;
    const body = buffer.subarray(end + 4, end + 4 + length).toString("utf8");
    buffer = buffer.subarray(end + 4 + length);
    try {
      handle(JSON.parse(body));
    } catch (error) {
      process.stderr.write(`node-check server: ${String(error)}\n`);
    }
  }
});
process.stdin.on("end", () => process.exit(0));
