//! A stand-in plugin helper for the protocol tests (03-01). Not part of the
//! app.
//!
//! It speaks real `tinkerpad-plugin-v1` JSON lines over stdio, and behaves the
//! way a helper written by someone else would: it prints a banner before it is
//! ready, it sends a message type this version of TinkerPad has never heard
//! of, and it puts extra fields in the messages it sends. All three are legal,
//! and the core is meant to carry on regardless.
//!
//! What a `run` does is chosen by the playground `kind`, so a test asks for
//! behaviour without needing a switch for each case:
//!
//! | `kind` | What it does |
//! |---|---|
//! | `echo` (default) | streams the `main` file back as stdout in small chunks |
//! | `stderr` | writes the `main` file to the run's stderr instead |
//! | `slow` | starts and never ends, until a `cancel` arrives |
//! | `fail` | replies `failed` with a diagnostic, as a compile error would |
//! | `crash` | exits without a word, mid-run |
//!
//! Environment switches, for the paths a well-behaved helper never takes:
//!
//! | Variable | Effect |
//! |---|---|
//! | `FAKEHELPER_SLOW_READY_MS` | waits this long before `ready` |
//! | `FAKEHELPER_CRASH_ON_RUN` | exits the moment any `run` arrives |
//! | `FAKEHELPER_HANG_ON_CANCEL` | ignores `cancel` completely |
//! | `FAKEHELPER_IGNORE_SHUTDOWN` | ignores `shutdown`, so it has to be killed |
//! | `FAKEHELPER_NO_BANNER` | skips the pre-`ready` noise |
//! | `FAKEHELPER_RETIRE_AFTER` | says `retiring` and exits after this many runs |
//!
//! It also stands in for a plugin's `[helper.build]` command: with
//! `FAKEHELPER_BUILD_INTO` set it appends a line to that file, says so on
//! stdout, and exits — 0, or 3 with `FAKEHELPER_BUILD_FAILS`. A test counts
//! the lines in that file to see how often the build ran (03-02).

use std::io::{BufRead, BufReader, Write};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Value, json};

/// Bytes per stdout chunk. Small enough that anything interesting in the
/// output, a dump sentinel included, is split across at least two messages.
const CHUNK: usize = 7;

fn main() {
    if let Some(into) = env("FAKEHELPER_BUILD_INTO") {
        build(&into);
        return;
    }
    let mut input = BufReader::new(std::io::stdin());
    let mut line = String::new();
    // The run the helper is pretending to be busy with.
    let mut running: Option<String> = None;
    // Runs finished, for the helper that steps aside after a few.
    let mut finished = 0u32;
    let retire_after = env("FAKEHELPER_RETIRE_AFTER").and_then(|v| v.parse::<u32>().ok());

    loop {
        line.clear();
        match input.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => {
                eprintln!("fakehelper: could not read stdin: {e}");
                break;
            }
        }
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            eprintln!("fakehelper: not a message: {}", line.trim());
            continue;
        };
        let kind = message["type"].as_str().unwrap_or_default().to_owned();
        let id = message["id"].as_str().unwrap_or_default().to_owned();

        match kind.as_str() {
            "hello" => hello(&message),
            "run" => {
                if env("FAKEHELPER_CRASH_ON_RUN").is_some() {
                    std::process::exit(3);
                }
                running = run(&id, &message).then(|| id.clone());
                if running.is_none() {
                    finished += 1;
                    // A helper that knows it should be replaced says so before
                    // it goes, so the core doesn't call this a crash (03-06).
                    if retire_after.is_some_and(|after| finished >= after) {
                        send(&json!({"type": "retiring", "reason": "that is enough runs"}));
                        std::process::exit(0);
                    }
                }
            }
            "cancel" => {
                if env("FAKEHELPER_HANG_ON_CANCEL").is_some() {
                    eprintln!("fakehelper: ignoring the cancel of {id}");
                    continue;
                }
                if running.as_deref() == Some(id.as_str()) {
                    running = None;
                    // A cancelled program has no exit code of its own.
                    send(&json!({
                        "type": "exited",
                        "id": id,
                        "code": Value::Null,
                        "duration_ms": 1,
                        "cancelled": true,
                    }));
                }
            }
            "shutdown" => {
                if env("FAKEHELPER_IGNORE_SHUTDOWN").is_some() {
                    eprintln!("fakehelper: ignoring shutdown");
                    continue;
                }
                std::process::exit(0);
            }
            "playground_created" | "playground_opened" | "playground_closed" | "add_dependency" => {
                hook(&kind, &id, &message);
            }
            other => eprintln!("fakehelper: nothing to do with {other}"),
        }
    }
}

/// Stand in for a plugin's build command: leave a mark that it ran, and say
/// something a waiting run can be shown.
fn build(into: &str) {
    println!("fakehelper: building");
    println!("fakehelper: two lines, so a test can see them arrive");
    let _ = std::io::stdout().flush();
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(into)
        .expect("build marker");
    let _ = writeln!(file, "built");
    if env("FAKEHELPER_BUILD_FAILS").is_some() {
        eprintln!("fakehelper: the build was told to fail");
        std::process::exit(3);
    }
}

/// Answer `hello`, the way a real runtime would: noise first, then `ready`.
fn hello(message: &Value) {
    if env("FAKEHELPER_NO_BANNER").is_none() {
        // Straight to stdout, unframed, exactly as a runtime banner arrives.
        println!("fakehelper 1.0 starting up");
        println!();
        let _ = std::io::stdout().flush();
    }
    if let Some(ms) = env("FAKEHELPER_SLOW_READY_MS").and_then(|v| v.parse().ok()) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
    // Proof of what arrived, written where a test can read it: the scratch
    // folder is the one place a helper is told it may write to.
    if let Some(scratch) = message["scratch_dir"].as_str() {
        let _ = std::fs::create_dir_all(scratch);
        let _ = std::fs::write(
            std::path::Path::new(scratch).join("hello.json"),
            message.to_string(),
        );
    }
    // The extra fields are the point: a helper from a later version says more
    // than this core reads, and that has to be fine.
    send(&json!({
        "type": "ready",
        "name": "fakehelper",
        "version": "1.0.0",
        "protocol_seen": message["protocol"],
        "plugin_dir_seen": message["plugin_dir"],
        "scratch_dir_seen": message["scratch_dir"],
        "capabilities": {"warm": true, "debug": false},
    }));
    // And a message type that does not exist in any version. Ignoring it is
    // the whole forward-compatibility rule in one line.
    send(&json!({"type": "telemetry", "runs": 0}));
}

/// Start a run. `true` when it is still going afterwards.
fn run(id: &str, message: &Value) -> bool {
    let kind = message["kind"].as_str().unwrap_or("echo");
    let main = message["main"].as_str().unwrap_or_default();

    match kind {
        "slow" => {
            // Started, and nothing more until someone cancels it.
            stream(id, "stdout", b"working\n");
            return true;
        }
        "crash" => std::process::exit(4),
        "fail" => {
            send(&json!({
                "type": "failed",
                "id": id,
                "message": "fakehelper was told to fail",
                "diagnostics": [{
                    "file": main,
                    "line": 3,
                    "col": 9,
                    "message": "expected one of the things it did not get",
                    "severity": "error",
                    "code": "FAKE001",
                }],
            }));
            return false;
        }
        _ => {}
    }

    let body = std::fs::read(main)
        .unwrap_or_else(|e| format!("fakehelper could not read {main}: {e}\n").into_bytes());
    let stream_name = if kind == "stderr" { "stderr" } else { "stdout" };
    stream(id, stream_name, &body);
    send(&json!({
        "type": "exited",
        "id": id,
        "code": 0,
        "duration_ms": 12,
        "rss_kb": 4096,
    }));
    false
}

/// Send some bytes as the run's output, in chunks small enough to split
/// anything interesting in it.
fn stream(id: &str, what: &str, bytes: &[u8]) {
    for chunk in bytes.chunks(CHUNK) {
        send(&json!({
            "type": what,
            "id": id,
            "data": BASE64.encode(chunk),
        }));
    }
}

/// Answer a hook with something a real one plausibly would.
fn hook(kind: &str, id: &str, message: &Value) {
    let name = message["playground"]["name"].as_str().unwrap_or("unnamed");
    let open = message["open"].as_array().map(Vec::len).unwrap_or(0);

    let mut reply = json!({"type": "result", "id": id});
    match kind {
        "playground_created" => {
            reply["files"] = json!([{
                "path": "helper.json",
                "content": format!("{{\"made_for\": \"{name}\"}}\n"),
            }]);
        }
        "playground_opened" => {
            reply["files"] = json!([{"path": "opened.txt", "content": format!("{open} open\n")}]);
            reply["lsp_config"] = json!({"fake.setting": {"root": name, "open": open}});
        }
        "add_dependency" => {
            let package = message["name"].as_str().unwrap_or_default();
            let version = message["version"].as_str().unwrap_or("*");
            reply["files"] = json!([{
                "path": "deps.txt",
                "content": format!("{package} {version}\n"),
            }]);
        }
        // playground_closed: nothing to give back, and saying so is an answer.
        _ => {}
    }
    send(&reply);
}

fn send(message: &Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{message}");
    let _ = out.flush();
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}
