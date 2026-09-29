// Release builds on Windows are GUI apps with no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// `--version` and `--help` answer and exit without opening a window
/// (work item 04-06), so a packaged build can be asked what it is from a
/// terminal or a script.
///
/// Written by hand rather than with a parser: two flags, no options, and
/// nothing a dependency would make clearer.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    for arg in &args {
        match arg.as_str() {
            "-V" | "--version" => return say(&format!("{} {}", NAME, env!("CARGO_PKG_VERSION"))),
            "-h" | "--help" => return say(HELP),
            _ => {}
        }
    }
    tinkerpad_app::run();
}

const NAME: &str = "TinkerPad";

const HELP: &str = concat!(
    "TinkerPad ",
    env!("CARGO_PKG_VERSION"),
    "
",
    "A code playground: write a few lines, press Ctrl+Enter, see what they do.
",
    "
",
    "USAGE:
",
    "    tinkerpad [OPTIONS]
",
    "
",
    "OPTIONS:
",
    "    -h, --help       Print this and exit
",
    "    -V, --version    Print the version and exit
",
    "
",
    "ENVIRONMENT:
",
    "    TINKERPAD_LIBRARY        Where playgrounds live, overriding settings.toml
",
    "    TINKERPAD_USER_PLUGINS   An extra plugin folder to load
",
    "    TINKERPAD_LOG            Log level or filter, e.g. debug
",
    "
",
    "Settings, plugins and logs live in TinkerPad's app-data folder; the
",
    "command palette (Ctrl+Shift+P) can open each of them.
",
);

/// Print to whatever asked.
///
/// A release build on Windows is a GUI program, which makes this less obvious
/// than it looks. Piped or redirected (`tinkerpad --version > v.txt`), the
/// process has a standard output and writing to it is right. Started from a
/// console with no redirection it has none, so this attaches to that console
/// and writes to `CONOUT$`. Double-clicked there is neither, and printing
/// nowhere is exactly right.
fn say(text: &str) {
    #[cfg(windows)]
    {
        use std::io::Write;
        unsafe extern "system" {
            fn AttachConsole(process: u32) -> i32;
            fn GetStdHandle(which: u32) -> isize;
        }
        // -11 is STD_OUTPUT_HANDLE; 0 means there is nothing to write to.
        let has_stdout = unsafe { GetStdHandle(-11i32 as u32) } != 0;
        if !has_stdout {
            // u32::MAX is ATTACH_PARENT_PROCESS.
            unsafe { AttachConsole(u32::MAX) };
            if let Ok(mut console) = std::fs::OpenOptions::new().write(true).open("CONOUT$") {
                let _ = writeln!(console, "{text}");
            }
            return;
        }
    }
    println!("{text}");
}
