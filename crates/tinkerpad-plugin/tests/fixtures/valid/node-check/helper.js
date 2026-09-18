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
