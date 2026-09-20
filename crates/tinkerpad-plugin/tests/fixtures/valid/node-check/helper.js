// A minimal plugin helper for the self-tests, in the language the test plugin
// already needs (Node). Not a real warm runner: it exists so the app's helper
// path — build on first use, the pool, the status chip, idle and restart —
// can be exercised on any machine and in CI, with no external install.
//
// It speaks `tinkerpad-plugin-v1` JSON lines over stdio (docs/PLUGINS.md §5):
//   hello     replies `ready`, after writing what it was told to hello.json
//             in its scratch folder so a test can see what arrived
//   run       runs the main file and streams its output back
//   cancel    ends that run
//   hooks     the four of them, each one appending a line to `hooks.log` in
//             the playground folder so a test can see exactly what arrived
//   shutdown  stops
//
// `run` starts `node <main>` rather than evaluating the file in this process.
// That is not what a real warm runner would do — the whole point of one is to
// skip the process start — but it makes this helper behave *exactly* like the
// plugin's `[run]` command, which is what the self-tests need to check: the
// two paths must be indistinguishable from above (work item 03-03).

"use strict";

const fs = require("fs");
const path = require("path");

let scratch = null;

function send(message) {
  process.stdout.write(`${JSON.stringify(message)}\n`);
}

function chunk(id, stream, text) {
  send({ type: stream, id, data: Buffer.from(text, "utf8").toString("base64") });
}

function hello(message) {
  scratch = message.scratch_dir || null;
  if (scratch) {
    try {
      fs.mkdirSync(scratch, { recursive: true });
      fs.writeFileSync(path.join(scratch, "hello.json"), JSON.stringify(message));
    } catch {
      // The scratch folder is a convenience for the tests, not a requirement.
    }
  }
  send({
    type: "ready",
    name: "node-check-helper",
    version: "1.0.0",
    // A field no version of the core defines: it must be ignored (D7).
    node: process.version,
  });
}

// One run at a time, as the protocol says.
let running = null;
let child = null;

function run(message) {
  const started = Date.now();
  running = message.id;
  const { spawn } = require("child_process");
  child = spawn(process.execPath, [message.main], {
    cwd: message.dir,
    stdio: ["ignore", "pipe", "pipe"],
  });
  child.stdout.on("data", (bytes) => {
    send({ type: "stdout", id: message.id, data: bytes.toString("base64") });
  });
  child.stderr.on("data", (bytes) => {
    send({ type: "stderr", id: message.id, data: bytes.toString("base64") });
  });
  child.on("error", (e) => {
    send({
      type: "failed",
      id: message.id,
      message: String((e && e.message) || e),
      diagnostics: [],
    });
    running = null;
    child = null;
  });
  child.on("close", (code) => {
    if (running !== message.id) return;
    send({
      type: "exited",
      id: message.id,
      code,
      duration_ms: Date.now() - started,
    });
    running = null;
    child = null;
  });
}

// ---- hooks (work item 03-07) ----------------------------------------------
//
// Each hook answers with a `hooks.log` to write into the playground folder,
// one line per call. Appending means reading what is there: a helper is given
// the folder and may look in it. The writing itself goes through the returned
// `files` rather than fs, which is the path the core has to get right.

function appended(playground, line) {
  const file = path.join(playground.dir, "hooks.log");
  let existing = "";
  try {
    existing = fs.readFileSync(file, "utf8");
  } catch {
    // The first hook of this playground's life.
  }
  return { path: "hooks.log", content: `${existing}${line}\n` };
}

function hook(message) {
  const playground = message.playground || {};
  const open = message.open || [];
  const names = open.map((p) => p.name).join(",");
  switch (message.type) {
    case "playground_created":
      return { files: [appended(playground, `created ${playground.name} ${playground.kind}`)] };
    case "playground_opened":
      return {
        files: [appended(playground, `opened ${playground.name} open=[${names}]`)],
        // Something `[lsp.register]` has no shape for: how many are open.
        lsp_config: { tinkerpad: { open: open.length, opened: playground.name } },
      };
    case "playground_closed":
      return { files: [appended(playground, `closed ${playground.name} open=[${names}]`)] };
    case "add_dependency": {
      const version = message.version || "latest";
      const files = [appended(playground, `added ${message.name}@${version}`)];
      // A name the self-test uses to check the sandbox: a path that climbs
      // out of the playground folder has to be refused by the core (§5).
      if (message.name === "escape") {
        files.push({ path: "../escaped.txt", content: "this must not be written\n" });
      }
      return { files };
    }
    default:
      return {};
  }
}

let buffer = "";
process.stdin.on("data", (bytes) => {
  buffer += bytes.toString("utf8");
  let cut;
  while ((cut = buffer.indexOf("\n")) >= 0) {
    const line = buffer.slice(0, cut).replace(/\r$/, "");
    buffer = buffer.slice(cut + 1);
    if (!line.trim()) continue;
    let message;
    try {
      message = JSON.parse(line);
    } catch {
      process.stderr.write(`helper: not a message: ${line.slice(0, 200)}\n`);
      continue;
    }
    switch (message.type) {
      case "hello":
        hello(message);
        break;
      case "run":
        run(message);
        break;
      case "cancel":
        if (running === message.id) {
          running = null;
          if (child) child.kill();
          child = null;
          // A cancelled program has no exit code of its own (§5).
          send({ type: "exited", id: message.id, code: null, duration_ms: 0 });
        }
        break;
      case "playground_created":
      case "playground_opened":
      case "playground_closed":
      case "add_dependency":
        send({ type: "result", id: message.id, ...hook(message) });
        break;
      case "shutdown":
        process.exit(0);
        break;
      default:
        // A message type this helper doesn't know: ignore it and carry on.
        process.stderr.write(`helper: ignoring ${message.type}\n`);
    }
  }
});

process.stdin.on("end", () => process.exit(0));
