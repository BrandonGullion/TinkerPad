//! A stand-in package manager for the tool-acquisition tests, so they run on
//! every OS without shell scripts. Not part of the app.
//!
//! Arguments are steps, run in order:
//!   install <path>  write a stub program at that path, creating its folder
//!   print <text>    write text and a newline to stdout
//!   exit <code>     exit with the code (default exit is 0)

use std::io::Write;

const DUMP_PREFIX: &str = "\u{1b}]1337;TinkerPad=";
/// The table `tinkerpad plugin check` expects (docs/PLUGINS.md 8.1).
const GOOD_TABLE: &str = r#"{"v":1,"title":"check","kind":"table","columns":[{"name":"name"},{"name":"n"}],"rows":[["ada",1],["grace",2]]}"#;
/// The same, with a column renamed: what a plugin that changed a field and
/// forgot would send.
const COLUMNS_WRONG: &str = r#"{"v":1,"kind":"table","columns":[{"name":"name"},{"name":"born"}],"rows":[["ada",1],["grace",2]]}"#;
/// A dump that isn't a table at all.
const NOT_A_TABLE: &str = r#"{"v":1,"kind":"value","value":42}"#;

const DUMP_END: &str = "\u{1b}\\";

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
            "stderr" => eprintln!("{value}"),
            // The wire form a dump takes (docs/PLUGINS.md 6), so the
            // checker's dump assertions can be tested without a language. The
            // payloads are built here rather than passed in, because a
            // manifest reads `{` as the start of a placeholder.
            "table" => {
                let payload = match value.as_str() {
                    "columns" => COLUMNS_WRONG,
                    "value" => NOT_A_TABLE,
                    _ => GOOD_TABLE,
                };
                print!("{DUMP_PREFIX}{payload}{DUMP_END}");
                std::io::stdout().flush().unwrap();
            }
            "sleep" => {
                std::thread::sleep(std::time::Duration::from_millis(value.parse().unwrap_or(0)))
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
