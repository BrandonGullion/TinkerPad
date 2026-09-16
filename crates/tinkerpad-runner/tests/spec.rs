//! `run_spec` end to end: a fixture plugin whose `[run]` launches
//! `tinkerpad-echoargs`, which prints what it received. Proves placeholder
//! substitution, program resolution, working directory, and environment on
//! every OS without a shell.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use tinkerpad_core::{KindId, LanguageId, Playground, PlaygroundId};
use tinkerpad_plugin::{Os, Plugin, Resolver, ToolchainOverrides};
use tinkerpad_runner::{RunContext, RunEvent, SpecError, run, run_spec};

const ECHO: &str = env!("CARGO_BIN_EXE_tinkerpad-echoargs");

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("tinkerpad-spec-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A TOML literal string, so Windows backslashes need no escaping.
fn lit(s: &str) -> String {
    format!("'{s}'")
}

/// Write a plugin with the given `[run]` body and load it.
fn plugin(root: &Path, run_section: &str) -> Plugin {
    let dir = root.join("plugin");
    std::fs::create_dir_all(dir.join("templates")).unwrap();
    std::fs::write(dir.join("templates/main.x"), "x\n").unwrap();
    std::fs::write(
        dir.join("plugin.toml"),
        format!(
            r#"api = 1

[language]
id = "echo"
name = "Echo"
extension = "x"

[toolchain]
probe = [{{ command = "echo-tool" }}]
install_hint = "Install Echo from example.com"

[[kind]]
id = "k"
name = "K"
template = "main.x"
default = true

{run_section}
"#
        ),
    )
    .unwrap();
    Plugin::load_for(&dir, Os::current()).unwrap_or_else(|e| panic!("{e}"))
}

fn playground(root: &Path) -> Playground {
    let dir = root.join("library").join("my-playground");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.x"), "x\n").unwrap();
    Playground {
        id: PlaygroundId::new("01K0000000000000000000TEST"),
        name: "My Playground".into(),
        language: LanguageId::new("echo"),
        kind: KindId::new("k"),
        main_file: dir.join("main.x"),
        dir,
        created_ms: 0,
        tags: Vec::new(),
    }
}

fn resolver() -> Resolver {
    Resolver::new(
        std::env::var_os("PATH").unwrap_or_default(),
        ToolchainOverrides::default(),
    )
}

/// Run a spec and collect `key=value` output lines.
async fn outputs(spec: tinkerpad_runner::RunSpec) -> (Vec<String>, BTreeMap<String, String>) {
    let mut handle = run(spec).unwrap();
    let mut out = Vec::new();
    loop {
        match handle.recv().await.unwrap() {
            RunEvent::Stdout(b) => out.extend_from_slice(&b),
            RunEvent::Exited { code, .. } => {
                assert_eq!(code, Some(0));
                break;
            }
            RunEvent::Cancelled { .. } => panic!("cancelled"),
            _ => {}
        }
    }
    let text = String::from_utf8(out).unwrap();
    let mut args = Vec::new();
    let mut other = BTreeMap::new();
    for line in text.lines() {
        if let Some(a) = line.strip_prefix("arg=") {
            args.push(a.to_owned());
        } else if let Some((k, v)) = line.split_once('=') {
            other.insert(k.to_owned(), v.to_owned());
        }
    }
    (args, other)
}

#[tokio::test]
async fn every_placeholder_is_filled_end_to_end() {
    let s = Scratch::new("placeholders");
    let p = plugin(
        &s.0,
        &format!(
            r#"[run]
command = {}
args = ["{{main}}", "{{dir}}", "{{id}}", "{{name}}", "{{scratch}}", "{{plugin_dir}}", "{{version}}", "{{toolchain}}", "{{{{literal}}}}"]
env = {{ TINKERPAD_ECHO_ENV = "ECHO_MAIN,ECHO_PLAIN,PATH_SEEN", ECHO_MAIN = "main is {{main}}", ECHO_PLAIN = "plain" }}"#,
            lit(ECHO)
        ),
    );
    let pg = playground(&s.0);
    let scratch_root = s.0.join("scratch");
    let r = resolver();
    let ctx = RunContext {
        resolver: &r,
        scratch_root: &scratch_root,
        toolchain_program: "echo-tool",
        toolchain_version: "1.2.3",
    };

    let spec = run_spec(&p, &pg, &ctx).unwrap();
    assert_eq!(Path::new(&spec.program), Path::new(ECHO));
    assert_eq!(
        spec.cwd.as_deref(),
        Some(pg.dir.as_path()),
        "cwd defaults to the playground"
    );

    let (args, other) = outputs(spec).await;
    let main = pg.main_file.display().to_string();
    let dir = pg.dir.display().to_string();
    let scratch = scratch_root.join("echo").display().to_string();
    let plugin_dir = p.dir.display().to_string();
    assert_eq!(
        args,
        [
            main.as_str(),
            dir.as_str(),
            "01K0000000000000000000TEST",
            "My Playground",
            scratch.as_str(),
            plugin_dir.as_str(),
            "1.2.3",
            "echo-tool",
            "{literal}",
        ]
    );
    assert_eq!(
        Path::new(&other["cwd"]).canonicalize().unwrap(),
        pg.dir.canonicalize().unwrap()
    );
    assert_eq!(other["env:ECHO_MAIN"], format!("main is {main}"));
    assert_eq!(other["env:ECHO_PLAIN"], "plain");
}

#[tokio::test]
async fn toolchain_placeholder_as_the_command() {
    let s = Scratch::new("toolchain");
    let p = plugin(
        &s.0,
        "[run]\ncommand = \"{toolchain}\"\nargs = [\"{main}\"]\ncwd = \"{scratch}\"",
    );
    let pg = playground(&s.0);
    let scratch_root = s.0.join("scratch");
    std::fs::create_dir_all(scratch_root.join("echo")).unwrap();
    let r = resolver();
    let ctx = RunContext {
        resolver: &r,
        scratch_root: &scratch_root,
        toolchain_program: ECHO,
        toolchain_version: "",
    };
    let spec = run_spec(&p, &pg, &ctx).unwrap();
    assert_eq!(Path::new(&spec.program), Path::new(ECHO));
    assert_eq!(
        spec.cwd.as_deref(),
        Some(scratch_root.join("echo").as_path())
    );
    let (args, _) = outputs(spec).await;
    assert_eq!(args, [pg.main_file.display().to_string()]);
}

#[tokio::test]
async fn settings_override_decides_the_program() {
    let s = Scratch::new("override");
    // `echo-tool` isn't on PATH; only the override makes it runnable.
    let p = plugin(
        &s.0,
        "[run]\ncommand = \"echo-tool\"\nargs = [\"overridden\"]",
    );
    let pg = playground(&s.0);
    let overrides = ToolchainOverrides::from_settings(&format!(
        "[toolchains.echo]\necho-tool = {}\n",
        lit(ECHO)
    ))
    .unwrap();
    let r = Resolver::new(std::env::var_os("PATH").unwrap_or_default(), overrides);
    let scratch_root = s.0.join("scratch");
    let ctx = RunContext {
        resolver: &r,
        scratch_root: &scratch_root,
        toolchain_program: "echo-tool",
        toolchain_version: "",
    };
    let spec = run_spec(&p, &pg, &ctx).unwrap();
    assert_eq!(Path::new(&spec.program), Path::new(ECHO));
    let (args, _) = outputs(spec).await;
    assert_eq!(args, ["overridden"]);
}

#[test]
fn missing_program_carries_the_install_hint() {
    let s = Scratch::new("missing");
    let p = plugin(&s.0, "[run]\ncommand = \"tinkerpad-no-such-runtime\"");
    let pg = playground(&s.0);
    let r = resolver();
    let scratch_root = s.0.join("scratch");
    let ctx = RunContext {
        resolver: &r,
        scratch_root: &scratch_root,
        toolchain_program: "x",
        toolchain_version: "",
    };
    match run_spec(&p, &pg, &ctx) {
        Err(SpecError::NotFound {
            message,
            install_hint,
        }) => {
            assert!(message.contains("tinkerpad-no-such-runtime"), "{message}");
            assert_eq!(
                install_hint.as_deref(),
                Some("Install Echo from example.com")
            );
        }
        other => panic!("expected NotFound, got {other:?}"),
    }
}

#[test]
fn spec_is_plain_data_a_debugger_can_extend() {
    let s = Scratch::new("extend");
    let p = plugin(
        &s.0,
        &format!("[run]\ncommand = {}\nargs = [\"{{main}}\"]", lit(ECHO)),
    );
    let pg = playground(&s.0);
    let r = resolver();
    let scratch_root = s.0.join("scratch");
    let ctx = RunContext {
        resolver: &r,
        scratch_root: &scratch_root,
        toolchain_program: "x",
        toolchain_version: "",
    };
    let mut spec = run_spec(&p, &pg, &ctx).unwrap();
    // What a debug launch would do (DEBUGGING.md D13).
    spec.args.insert(0, "--inspect=0".into());
    spec.env.push(("EXTRA".into(), "1".into()));
    assert_eq!(spec.args[0], "--inspect=0");
    assert!(
        spec.env.iter().any(|(k, _)| k == "PATH"),
        "child gets the discovered PATH"
    );
}

// `[deps]`: adding a package either runs a command or gives back a line to
// insert (docs/PLUGINS.md §2).

/// A `[deps]` section on top of a minimal `[run]`.
fn deps_plugin(root: &Path, deps_section: &str) -> Plugin {
    plugin(
        root,
        &format!("[run]\ncommand = {}\n\n{deps_section}", lit(ECHO)),
    )
}

fn package<'a>(name: &'a str, version: &'a str) -> tinkerpad_runner::Package<'a> {
    tinkerpad_runner::Package { name, version }
}

#[test]
fn deps_command_runs_in_the_playground_folder() {
    let scratch = Scratch::new("deps-command");
    let plugin = deps_plugin(
        &scratch.0,
        &format!(
            "[deps]\nkind = \"command\"\nadd = [{}, \"add\", \"{{name}}@{{version}}\"]",
            lit(ECHO)
        ),
    );
    let playground = playground(&scratch.0);
    let resolver = resolver();
    let ctx = RunContext {
        resolver: &resolver,
        scratch_root: &scratch.0,
        toolchain_program: "",
        toolchain_version: "",
    };

    let spec = tinkerpad_runner::deps_command(&plugin, &playground, &ctx, package("serde", "1.0"))
        .unwrap()
        .expect("a command plugin");
    assert_eq!(spec.program, PathBuf::from(ECHO));
    assert_eq!(spec.args, ["add", "serde@1.0"]);
    assert_eq!(spec.cwd.as_deref(), Some(playground.dir.as_path()));

    // No version: the trailing `@` would be a syntax error for cargo and npm.
    let spec = tinkerpad_runner::deps_command(&plugin, &playground, &ctx, package("serde", ""))
        .unwrap()
        .unwrap();
    assert_eq!(spec.args, ["add", "serde"]);

    // This plugin adds packages by command, so there is no directive.
    assert_eq!(
        tinkerpad_runner::deps_directive(&plugin, package("serde", "1.0")).unwrap(),
        None
    );
}

#[test]
fn deps_directive_gives_a_line_and_the_prefix_that_identifies_it() {
    let scratch = Scratch::new("deps-directive");
    let plugin = deps_plugin(
        &scratch.0,
        "[deps]\nkind = \"inline-directive\"\ndirective = \"#:package {name}@{version}\"",
    );

    let (line, prefix) =
        tinkerpad_runner::deps_directive(&plugin, package("Newtonsoft.Json", "13.0.3"))
            .unwrap()
            .expect("a directive plugin");
    assert_eq!(line, "#:package Newtonsoft.Json@13.0.3");
    // The prefix plus the name is what an existing line for this package
    // starts with, so the UI can replace it instead of adding a second one.
    assert_eq!(prefix, "#:package ");
    assert!(line.starts_with(&format!("{prefix}Newtonsoft.Json")));

    let (line, _) = tinkerpad_runner::deps_directive(&plugin, package("Newtonsoft.Json", ""))
        .unwrap()
        .unwrap();
    assert_eq!(line, "#:package Newtonsoft.Json");

    let playground = playground(&scratch.0);
    let resolver = resolver();
    let ctx = RunContext {
        resolver: &resolver,
        scratch_root: &scratch.0,
        toolchain_program: "",
        toolchain_version: "",
    };
    assert!(
        tinkerpad_runner::deps_command(&plugin, &playground, &ctx, package("x", ""))
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_plugin_without_deps_says_so() {
    let scratch = Scratch::new("deps-none");
    let plugin = deps_plugin(&scratch.0, "");
    assert_eq!(
        tinkerpad_runner::deps_directive(&plugin, package("serde", "")),
        Err(SpecError::NoDeps {
            plugin: "Echo".into()
        })
    );
}

#[test]
fn every_bundled_plugin_can_add_a_package() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("plugins");
    for (id, expected) in [
        ("typescript", "npm install zod@4.0.0"),
        ("rust", "cargo add serde@1.0.0"),
        ("csharp", "#:package Newtonsoft.Json@13.0.3"),
    ] {
        let plugin = Plugin::load_for(root.join(id), Os::current()).unwrap();
        let package = match id {
            "typescript" => package("zod", "4.0.0"),
            "rust" => package("serde", "1.0.0"),
            _ => package("Newtonsoft.Json", "13.0.3"),
        };
        let got = match tinkerpad_runner::deps_directive(&plugin, package).unwrap() {
            Some((line, _)) => line,
            None => {
                let scratch = Scratch::new(&format!("deps-{id}"));
                let playground = playground(&scratch.0);
                let resolver = resolver();
                let ctx = RunContext {
                    resolver: &resolver,
                    scratch_root: &scratch.0,
                    toolchain_program: "",
                    toolchain_version: "",
                };
                // Resolution needs the real tool on PATH; skip if it isn't.
                let Ok(Some(spec)) =
                    tinkerpad_runner::deps_command(&plugin, &playground, &ctx, package)
                else {
                    continue;
                };
                let program = Path::new(&spec.program)
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                std::iter::once(program)
                    .chain(spec.args.iter().map(|a| a.to_string_lossy().into_owned()))
                    .collect::<Vec<_>>()
                    .join(" ")
            }
        };
        assert_eq!(got, expected, "{id}");
    }
}
