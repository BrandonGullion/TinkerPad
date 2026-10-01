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
    match args.first().map(String::as_str) {
        Some("plugin") => std::process::exit(plugin_subcommand(&args[1..])),
        Some("tools") => std::process::exit(tools_subcommand(&args[1..])),
        _ => {}
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
    let verb = rest.next();
    if !matches!(
        verb,
        Some("check" | "install" | "update" | "remove" | "list")
    ) {
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
    // `list` is the only one that needs no argument.
    if verb == Some("list") {
        return tinkerpad_app::cli::plugin_list(json, &mut |line| say(line));
    }
    let Some(dir) = dir else {
        say(&format!(
            "Which plugin{}?\n\n{PLUGIN_HELP}",
            if verb == Some("check") { " folder" } else { "" }
        ));
        return 2;
    };
    let named = dir.to_string_lossy().into_owned();

    match verb {
        Some("install") => tinkerpad_app::cli::plugin_install(&named, &mut |line| say(line)),
        Some("update") => tinkerpad_app::cli::plugin_update(&named, &mut |line| say(line)),
        Some("remove") => tinkerpad_app::cli::plugin_remove(&named, &mut |line| say(line)),
        _ => {
            let options = tinkerpad_app::check::Options { dir, json, only };
            if !options.json {
                say(&format!("Checking {}", options.dir.display()));
            }
            tinkerpad_app::check::check(&options, &mut |line| say(line)).exit_code()
        }
    }
}

/// `tinkerpad tools list|install` (work item 05-01c).
///
/// The same offers the window makes, for a machine with no window: a CI
/// runner, a container, or a laptop being set up.
fn tools_subcommand(args: &[String]) -> i32 {
    let mut rest = args.iter().map(String::as_str);
    let verb = rest.next();
    let mut json = false;
    let mut yes = false;
    let mut which = None;
    let mut language = None;
    while let Some(arg) = rest.next() {
        match arg {
            "--json" => json = true,
            "--yes" | "-y" => yes = true,
            "--which" => {
                which = match rest.next() {
                    Some("toolchain") => Some(tinkerpad_app::tools::Which::Toolchain),
                    Some("lsp") => Some(tinkerpad_app::tools::Which::Lsp),
                    Some("workspace") => Some(tinkerpad_app::tools::Which::Workspace),
                    other => {
                        say(&format!(
                            "--which takes toolchain, lsp or workspace, not {}.",
                            other.unwrap_or("nothing")
                        ));
                        return 2;
                    }
                }
            }
            "-h" | "--help" => {
                say(TOOLS_HELP);
                return 0;
            }
            other if other.starts_with('-') => {
                say(&format!("Unknown option `{other}`.\n\n{TOOLS_HELP}"));
                return 2;
            }
            value => language = Some(value.to_owned()),
        }
    }

    match verb {
        Some("list") => tinkerpad_app::cli::list(json, &mut |line| say(line)),
        Some("install") => match language {
            Some(language) => {
                tinkerpad_app::cli::install(&language, which, yes, &mut |line| say(line))
            }
            None => {
                say(&format!("Install what?\n\n{TOOLS_HELP}"));
                2
            }
        },
        _ => {
            say(TOOLS_HELP);
            2
        }
    }
}

const TOOLS_HELP: &str = "\
What each language needs, and how to get it.

USAGE:
    tinkerpad tools list [--json]
    tinkerpad tools install <language> [--which toolchain|lsp|workspace] [--yes]

OPTIONS:
        --json           Print the list as JSON, for CI
        --which <what>   Install only the toolchain, the language server, or
                         what the server needs in its workspace
    -y, --yes            Don't ask before running or downloading
    -h, --help           Print this and exit

`install` shows what it is about to run or fetch and asks first. It installs
into TinkerPad's own folder where it can, so nothing needs administrator
rights and deleting that folder undoes it. `[tools] acquire = \"never\"` in
settings.toml refuses here as it does in the window.";

const PLUGIN_HELP: &str = "\
Plugins: check one, install one, or see what is installed.

USAGE:
    tinkerpad plugin check <dir> [--json] [--only <check>]
    tinkerpad plugin install <path|git-url>
    tinkerpad plugin update <id>
    tinkerpad plugin remove <id>
    tinkerpad plugin list [--json]

OPTIONS:
        --json           Print the report, or the list, as JSON
        --only <check>   Run only this check: manifest, toolchain, templates,
                         run, or one kind's run such as run:statements
    -h, --help           Print this and exit

`check` loads the manifest, probes the toolchain, scaffolds a playground of
every kind into a temporary folder, runs each one, and exercises whatever the
plugin's `check/` folder provides. It exits 0 unless something failed; a
toolchain this machine hasn't got is a skip, not a failure.

`install` copies a folder, or clones a URL, into TinkerPad's plugins folder —
after loading the manifest, so a folder that isn't a plugin is refused rather
than copied. A plugin runs with your privileges; `install` says so every time.
`remove` moves it to the recycle bin.";

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
    "    tinkerpad plugin install <path|git-url>
",
    "    tinkerpad plugin list
",
    "    tinkerpad tools list
",
    "    tinkerpad tools install <language>
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
