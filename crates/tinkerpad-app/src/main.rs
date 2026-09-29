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
/// Straightforward everywhere but Windows, where a release build is a GUI
/// program with no console of its own; `print_to_parent_console` deals with
/// that and says whether it did, so this stays free of platform conditionals
/// (ARCHITECTURE.md §10).
fn say(text: &str) {
    if tinkerpad_process::platform::print_to_parent_console(text) {
        return;
    }
    println!("{text}");
}
