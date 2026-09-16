//! A stand-in language server for transport, pool, and restart tests
//! (02-01, 02-02, 02-05). Not part of the app.
//!
//! It speaks real `Content-Length` framing over stdio and answers:
//!
//! | Message | What it does |
//! |---|---|
//! | `initialize` | replies with a small `capabilities` object and the pid |
//! | `initialized` | ignored, as a server should |
//! | `test/echo` | replies with the params it was given |
//! | `test/log` | replies `null` and sends `params.count` `window/logMessage`s |
//! | `test/big` | replies with a string of `params.bytes` characters |
//! | `test/stderr` | writes `params.text` to stderr, replies `null` |
//! | `test/halt` | writes half a frame and closes stdout, leaving the reader mid-message |
//! | `shutdown` | replies `null` |
//! | `exit` | exits, 0 after `shutdown` and 1 otherwise, as LSP says |
//!
//! `TINKERPAD_FAKELSP_GREETING` is written to stderr at startup, so a test can
//! watch for a server announcing itself before it is spoken to.

use std::io::{BufRead, BufReader, Read, Write};

use serde_json::{Value, json};

fn main() {
    if let Ok(greeting) = std::env::var("TINKERPAD_FAKELSP_GREETING") {
        eprintln!("{greeting}");
    }
    let mut input = BufReader::new(std::io::stdin());
    let mut shutdown = false;

    while let Some(body) = read_message(&mut input) {
        let message: Value = match serde_json::from_slice(&body) {
            Ok(value) => value,
            Err(e) => {
                eprintln!("fakelsp: not JSON: {e}");
                continue;
            }
        };
        let method = message["method"].as_str().unwrap_or_default().to_owned();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let id = message.get("id").cloned();

        match method.as_str() {
            "initialize" => reply(
                id,
                json!({
                    "capabilities": { "textDocumentSync": 1, "hoverProvider": true },
                    "serverInfo": { "name": "fakelsp", "version": "1", "pid": std::process::id() },
                }),
            ),
            "test/echo" => reply(id, params),
            "test/log" => {
                let count = params["count"].as_u64().unwrap_or(1);
                for n in 0..count {
                    send(json!({
                        "jsonrpc": "2.0",
                        "method": "window/logMessage",
                        "params": { "type": 3, "message": format!("log {n}") },
                    }));
                }
                reply(id, Value::Null);
            }
            "test/big" => {
                let bytes = params["bytes"].as_u64().unwrap_or(0) as usize;
                reply(id, json!({ "text": "x".repeat(bytes) }));
            }
            "test/stderr" => {
                eprintln!("{}", params["text"].as_str().unwrap_or("stderr"));
                reply(id, Value::Null);
            }
            "test/halt" => {
                // Headers promising a body that never comes, then goodbye.
                let mut out = std::io::stdout();
                let _ = out.write_all(b"Content-Length: 4096\r\n\r\n{\"jsonrpc\"");
                let _ = out.flush();
                std::process::exit(0);
            }
            "shutdown" => {
                shutdown = true;
                reply(id, Value::Null);
            }
            "exit" => std::process::exit(if shutdown { 0 } else { 1 }),
            "initialized" | "$/cancelRequest" => {}
            other => {
                if id.is_some() {
                    send(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32601, "message": format!("no method {other}") },
                    }));
                }
            }
        }
    }
}

/// Read one `Content-Length` frame. `None` at end of input.
fn read_message(input: &mut BufReader<std::io::Stdin>) -> Option<Vec<u8>> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let mut body = vec![0; length?];
    input.read_exact(&mut body).ok()?;
    Some(body)
}

fn reply(id: Option<Value>, result: Value) {
    let Some(id) = id else { return };
    send(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
}

fn send(message: Value) {
    let body = serde_json::to_vec(&message).expect("a Value always serialises");
    let mut out = std::io::stdout();
    let _ = write!(out, "Content-Length: {}\r\n\r\n", body.len());
    let _ = out.write_all(&body);
    let _ = out.flush();
}
