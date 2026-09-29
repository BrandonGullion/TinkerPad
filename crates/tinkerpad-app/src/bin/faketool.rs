//! A stand-in package manager for the tool-acquisition tests, so they run on
//! every OS without shell scripts. Not part of the app.
//!
//! Arguments are steps, run in order:
//!   install <path>  write a stub program at that path, creating its folder
//!   print <text>    write text and a newline to stdout
//!   exit <code>     exit with the code (default exit is 0)

use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let value = args.get(i + 1).cloned().unwrap_or_default();
        match args[i].as_str() {
            "install" => {
                let path = std::path::PathBuf::from(&value);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).unwrap();
                }
                std::fs::write(&path, b"stub\n").unwrap();
                println!("installed {value}");
                std::io::stdout().flush().unwrap();
            }
            "print" => {
                println!("{value}");
                std::io::stdout().flush().unwrap();
            }
            "exit" => std::process::exit(value.parse().unwrap_or(1)),
            other => {
                eprintln!("faketool: unknown step {other:?}");
                std::process::exit(2);
            }
        }
        i += 2;
    }
}
