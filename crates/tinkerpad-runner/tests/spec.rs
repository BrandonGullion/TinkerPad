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
