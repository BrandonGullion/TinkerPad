//! A tiny program the integration tests spawn so the same tests run on every
//! platform without shell scripts. Not part of the app.
//!
//! Usage:
//!   testchild echo <words…>        print words joined by spaces, exit 0
//!   testchild stderr <words…>      same, to stderr
//!   testchild exit <code>          exit with the code
//!   testchild utf8                 print "héllo wörld" one byte at a time with flushes
//!   testchild slow <n>             print n bytes, one every 10 ms, flushed
//!   testchild sleep <ms>           sleep then exit 0
//!   testchild spawn-sleeper        spawn self as `sleep 60000`, print "child <pid>", then sleep
//!   testchild env <NAME>           print the env var (or "<unset>")
//!   testchild cat                  copy stdin to stdout until EOF

use std::io::{Read, Write};
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let rest = &args[args.len().min(1)..];
    let mut out = std::io::stdout().lock();

    match cmd {
        "echo" => {
            writeln!(out, "{}", rest.join(" ")).unwrap();
        }
        "stderr" => {
            eprintln!("{}", rest.join(" "));
        }
        "exit" => {
            let code = rest.first().and_then(|s| s.parse().ok()).unwrap_or(0);
            std::process::exit(code);
        }
        "utf8" => {
            for byte in "héllo wörld".as_bytes() {
                out.write_all(&[*byte]).unwrap();
                out.flush().unwrap();
                sleep(Duration::from_millis(5));
            }
        }
        "slow" => {
            let n: usize = rest.first().and_then(|s| s.parse().ok()).unwrap_or(10);
            for _ in 0..n {
                out.write_all(b".").unwrap();
                out.flush().unwrap();
                sleep(Duration::from_millis(10));
            }
        }
        "sleep" => {
            let ms: u64 = rest.first().and_then(|s| s.parse().ok()).unwrap_or(1000);
            sleep(Duration::from_millis(ms));
        }
        "spawn-sleeper" => {
            let me = std::env::current_exe().unwrap();
            let mut child = Command::new(me).args(["sleep", "60000"]).spawn().unwrap();
            writeln!(out, "child {}", child.id()).unwrap();
            out.flush().unwrap();
            sleep(Duration::from_millis(60_000));
            let _ = child.kill();
            let _ = child.wait();
        }
        "env" => {
            let name = rest.first().map(String::as_str).unwrap_or("");
            let value = std::env::var(name).unwrap_or_else(|_| "<unset>".to_owned());
            writeln!(out, "{value}").unwrap();
        }
        "cat" => {
            let mut buf = Vec::new();
            std::io::stdin().read_to_end(&mut buf).unwrap();
            out.write_all(&buf).unwrap();
        }
        other => {
            eprintln!("unknown command: {other:?}");
            std::process::exit(2);
        }
    }
}
