// TinkerPad's TypeScript and JavaScript helper (work item 03-06).
//
// Node starts in about forty milliseconds and strips types as it goes, so the
// per-run path is already quick. This helper is the same work done once: the
// process stays up, and a run is a dynamic `import()` of the playground's file
// with a cache-busting query, so Node's start-up and the first type-strip pass
// are paid for once per session rather than once per Mod+Enter.
//
// It speaks `tinkerpad-plugin-v1` on stdin and stdout (docs/PLUGINS.md §5).
// The Python helper next door is the one to read first; this one is here
// because a protocol with two implementations is a protocol, and one with a
// single implementation is a shape.
//
// Two things it does differently, both honest limitations of the language:
//
//   Cancel. There is no safe way to stop a module that is already running in
//   this thread. The helper answers `cancel` with silence, and the core kills
//   and respawns it after two seconds (§5). A worker thread per run would fix
//   that, at the cost of the warmth this exists for.
//
//   Retirement. The cache-busting query means every run leaves a module in
//   Node's cache for good. After RETIRE_AFTER runs the helper says `retiring`
//   and exits cleanly; the core starts a fresh one without calling it a crash.

import { pathToFileURL } from "node:url";

/** Runs before this helper replaces itself. Enough for a long session. */
const RETIRE_AFTER = 500;

// The real streams, kept before a run is allowed to borrow them: protocol
// messages must never end up inside a playground's output, and the helper's
// own notes must never end up in its results pane.
const streams = {
  stdout: process.stdout.write.bind(process.stdout),
  stderr: process.stderr.write.bind(process.stderr),
};

/**
 * What the process holds open with nothing running, so a run's own leftovers
 * stand out. Taken when a run starts rather than at startup: stdin's pipe is
 * attached below, and a previous run's leftovers are this helper's problem
 * now, not the next run's.
 */
let baseline = new Map();

let running = null;
let finished = 0;

function send(message) {
  streams.stdout(`${JSON.stringify(message)}\n`);
}

function warn(text) {
  streams.stderr(`helper: ${text}\n`);
}

/** A chunk of a run's output, as the protocol wants it (§5). */
function output(kind, id, chunk, encoding) {
  const bytes = Buffer.isBuffer(chunk)
    ? chunk
    : Buffer.from(String(chunk), typeof encoding === "string" ? encoding : "utf8");
  if (bytes.length > 0) send({ type: kind, id, data: bytes.toString("base64") });
}

/**
 * Point `console.log` and everything else at the protocol for the length of a
 * run. Patched rather than replaced so anything holding a reference to
 * `process.stdout` — the dump library, a logger — keeps working.
 */
function borrowStreams(id) {
  process.stdout.write = (chunk, encoding, callback) => {
    output("stdout", id, chunk, encoding);
    if (typeof encoding === "function") encoding();
    else if (typeof callback === "function") callback();
    return true;
  };
  process.stderr.write = (chunk, encoding, callback) => {
    output("stderr", id, chunk, encoding);
    if (typeof encoding === "function") encoding();
    else if (typeof callback === "function") callback();
    return true;
  };
}

function returnStreams() {
  process.stdout.write = streams.stdout;
  process.stderr.write = streams.stderr;
}

function hello(message) {
  send({
    type: "ready",
    name: "tinkerpad-typescript-helper",
    version: "1.0.0",
    node: process.version,
  });
}

async function run(message) {
  const { id, main, dir } = message;
  const started = Date.now();
  running = { id, started };
  baseline = tally(process.getActiveResourcesInfo());
  borrowStreams(id);

  let code = 0;
  try {
    if (dir) process.chdir(dir);
    // The query is what makes Node fetch the file again instead of handing
    // back the module it loaded last time. It is also why this helper retires.
    await import(`${pathToFileURL(main).href}?t=${Date.now()}-${finished}`);
  } catch (e) {
    report(id, e);
    code = 1;
  }
  // A module's top-level code finishing is not the program finishing: Node
  // itself keeps going until nothing is left on the event loop, and a
  // playground that starts a server or an interval expects the same here. So
  // wait for what it started, exactly as `node main.ts` would, and let the
  // user cancel when they are done — which for this helper means restarting
  // it (see the note at the top).
  if (code === 0) await drain();
  returnStreams();
  finish(id, code, started);
}

/** Wait until the run has nothing of its own left on the event loop. */
async function drain() {
  let said = false;
  let left;
  while ((left = leftovers()).length > 0) {
    if (!said) {
      said = true;
      warn(`still running: ${[...new Set(left)].join(", ")}`);
    }
    // `unref`, so this helper's own waiting isn't one of the things it is
    // waiting for — a `setInterval` and this timer look the same from
    // `getActiveResourcesInfo`.
    await new Promise((resolve) => {
      setTimeout(resolve, 25).unref();
    });
  }
}

/** An error the playground didn't catch, printed as Node prints it. */
function report(id, e) {
  const text = e instanceof Error ? (e.stack ?? `${e.name}: ${e.message}`) : String(e);
  output("stderr", id, `${text}\n`);
}

function finish(id, code, started) {
  if (running?.id !== id) return; // Already reported, or cancelled.
  running = null;
  send({ type: "exited", id, code, duration_ms: Date.now() - started });
  const left = leftovers();
  if (left.length > 0) {
    warn(`that run left something running (${[...new Set(left)].join(", ")}); Restart the helper to clear it`);
  }
  if (++finished >= RETIRE_AFTER) retire();
}

/**
 * Anything on the event loop that the helper itself didn't put there: a
 * server, an interval, a pending socket. The process is shared, so these are
 * what a run leaves behind.
 */
function leftovers() {
  const now = tally(process.getActiveResourcesInfo());
  const extra = [];
  for (const [resource, count] of now) {
    for (let i = 0; i < count - (baseline.get(resource) ?? 0); i++) {
      extra.push(resource);
    }
  }
  return extra;
}

/** How many of each kind of resource, since a run may add a second of one. */
function tally(resources) {
  const counts = new Map();
  for (const resource of resources) {
    counts.set(resource, (counts.get(resource) ?? 0) + 1);
  }
  return counts;
}

/**
 * Step aside for a fresh process. `retiring` tells the core this exit was
 * planned, so it starts another one instead of counting a crash (§5).
 */
function retire() {
  send({
    type: "retiring",
    reason: `${finished} runs; Node's module cache keeps every one of them`,
  });
  process.exit(0);
}

// A rejection or a throw that escapes the run's own stack still belongs to it.
process.on("unhandledRejection", (reason) => {
  if (!running) return warn(`unhandled rejection with nothing running: ${reason}`);
  const { id, started } = running;
  report(id, reason);
  returnStreams();
  finish(id, 1, started);
});
process.on("uncaughtException", (error) => {
  if (!running) return warn(`uncaught exception with nothing running: ${error}`);
  const { id, started } = running;
  report(id, error);
  returnStreams();
  finish(id, 1, started);
});

let buffer = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", (text) => {
  buffer += text;
  let cut;
  while ((cut = buffer.indexOf("\n")) >= 0) {
    const line = buffer.slice(0, cut).trim();
    buffer = buffer.slice(cut + 1);
    if (!line) continue;
    let message;
    try {
      message = JSON.parse(line);
    } catch {
      warn(`not a message: ${line.slice(0, 200)}`);
      continue;
    }
    switch (message.type) {
      case "hello":
        hello(message);
        break;
      case "run":
        // Not awaited: the loop has to keep reading, because `cancel` and
        // `shutdown` arrive while a run is going.
        void run(message);
        break;
      case "cancel":
        // Nothing to do, and saying so would be a lie. The core waits two
        // seconds and kills us; see the note at the top.
        warn(`cannot cancel ${message.id} from inside the process`);
        break;
      case "shutdown":
        process.exit(0);
        break;
      default:
        // A message this version doesn't know: ignore it and carry on.
        warn(`ignoring ${message.type}`);
    }
  }
});
process.stdin.on("end", () => process.exit(0));
