// Release builds on Windows are GUI apps with no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// `--version`, `--help` and the subcommands answer and exit without opening a
/// window (work items 04-06, 05-01a), so a packaged build can be asked what it
/// is — and asked to check a plugin — from a terminal or a script.
///
/// Written by hand rather than with a parser: a handful of flags, one
/// subcommand, and nothing a dependency would make clearer.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("plugin") {
        std::process::exit(plugin_subcommand(&args[1..]));
    }
    for arg in &args {
        match arg.as_str() {
            "-V" | "--version" => return say(&format!("{} {}", NAME, env!("CARGO_PKG_VERSION"))),
            "-h" | "--help" => return say(HELP),
            _ => {}
        }
    }
    tinkerpad_app::run();
}

/// `tinkerpad plugin check <dir> [--json] [--only <check>]`.
///
/// Returns the exit code: 0 unless a check failed. A plugin whose toolchain
/// isn't on this machine skips rather than fails, so the same command is
/// useful on a laptop that has three languages and a CI runner that has one.
fn plugin_subcommand(args: &[String]) -> i32 {
    let mut rest = args.iter().map(String::as_str);
    if rest.next() != Some("check") {
        say(PLUGIN_HELP);
        return 2;
    }
    let mut dir: Option<std::path::PathBuf> = None;
    let mut json = false;
    let mut only = None;
    while let Some(arg) = rest.next() {
        match arg {
            "--json" => json = true,
            "--only" => match rest.next() {
                Some(value) => only = Some(value.to_owned()),
                None => {
                    say("--only needs the name of a check, such as `--only manifest`.");
                    return 2;
                }
            },
            "-h" | "--help" => {
                say(PLUGIN_HELP);
                return 0;
            }
            other if other.starts_with('-') => {
                say(&format!("Unknown option `{other}`.\n\n{PLUGIN_HELP}"));
                return 2;
            }
            path => dir = Some(std::path::PathBuf::from(path)),
        }
    }
    let Some(dir) = dir else {
        say(&format!("Which plugin folder?\n\n{PLUGIN_HELP}"));
        return 2;
    };

    let options = tinkerpad_app::check::Options { dir, json, only };
    if !options.json {
        say(&format!("Checking {}", options.dir.display()));
    }
    let report = tinkerpad_app::check::check(&options, &mut |line| say(line));
    report.exit_code()
}

const PLUGIN_HELP: &str = "\
Check that a plugin folder works.

USAGE:
    tinkerpad plugin check <dir> [OPTIONS]

OPTIONS:
        --json           Print the report as JSON, for CI
        --only <check>   Run only this check: manifest, toolchain, templates,
                         run, or one kind's run such as run:statements
    -h, --help           Print this and exit

The check loads the manifest, probes the toolchain, scaffolds a playground of
every kind into a temporary folder, and runs each one. It exits 0 unless
something failed; a toolchain this machine hasn't got is a skip, not a
failure.";

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
    "    tinkerpad plugin check <dir>
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
