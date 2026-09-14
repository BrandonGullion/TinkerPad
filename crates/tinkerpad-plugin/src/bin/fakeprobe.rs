//! A stand-in toolchain for probe tests, so they run on every OS without
//! shell scripts. Not part of the app.
//!
//! Arguments are steps, run in order:
//!   print <text>    write text and a newline to stdout
//!   stderr <text>   write text and a newline to stderr
//!   sleep <ms>      wait
//!   exit <code>     exit with the code (default exit is 0)

use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let value = args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "print" => {
                println!("{value}");
                std::io::stdout().flush().unwrap();
            }
            "stderr" => eprintln!("{value}"),
            "sleep" => {
                std::thread::sleep(std::time::Duration::from_millis(value.parse().unwrap_or(0)))
            }
            "exit" => std::process::exit(value.parse().unwrap_or(1)),
            other => {
                eprintln!("fakeprobe: unknown step {other:?}");
                std::process::exit(2);
            }
        }
        i += 2;
    }
}
