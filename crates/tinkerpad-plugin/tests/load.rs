//! Loading real plugin folders. Every folder under `fixtures/valid` must load;
//! every folder under `fixtures/invalid` must fail with the error its test
//! names.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tinkerpad_plugin::{
    DepsSpec, IdleShutdown, ManifestError, Os, Plugin, ProblemKind, Version, Workspace,
    example_manifest, placeholders,
};

fn fixture(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(path)
}

fn load(path: &str) -> Plugin {
    Plugin::load_for(fixture(path), Os::Linux)
        .unwrap_or_else(|e| panic!("{path} should load:\n{e}"))
}

fn fail(path: &str) -> ManifestError {
    match Plugin::load_for(fixture(path), Os::Linux) {
        Ok(_) => panic!("{path} should fail to load"),
        Err(e) => {
            // Every error must render to something a person can act on.
            let text = e.to_string();
            assert!(
                text.contains("plugin.toml"),
                "error doesn't name the file: {text}"
            );
            e
        }
    }
}

fn problem_kinds(e: &ManifestError) -> Vec<ProblemKind> {
    e.problems().iter().map(|p| p.kind).collect()
}

// ---------------------------------------------------------------- valid ----

#[test]
fn every_valid_fixture_loads_on_every_os() {
    let dir = fixture("valid");
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        for os in Os::ALL {
            if let Err(e) = Plugin::load_for(&path, os) {
                panic!("{} failed on {os}:\n{e}", path.display());
            }
        }
    }
}

#[test]
fn minimal_plugin_gets_defaults() {
    let p = load("valid/minimal");
    assert_eq!(p.id, "mini");
    assert_eq!(p.main_path, PathBuf::from("main.mini"));
    assert_eq!(p.default_kind().id, "statements");
    assert!(p.default_kind().template.is_absolute());
    assert!(
        p.default_kind()
            .template
            .ends_with("templates/statements.mini")
    );
    let run = p.run.as_ref().unwrap();
    assert_eq!(run.command, "mini");
    assert_eq!(run.args, ["{main}"]);
    assert!(p.helper.is_none() && p.lsp.is_none() && p.deps.is_none() && p.output.is_none());
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
}

#[test]
fn full_plugin_resolves_every_section() {
    let p = load("valid/full");
    assert_eq!(p.main_path, PathBuf::from("src/main.rs"));
    assert_eq!(p.probes[0].min_version, Version::parse("1.88"));
    assert_eq!(
        p.install_hint.as_deref(),
        Some("Install Rust from https://rustup.rs")
    );

    let expression = p.kind("expression").unwrap();
    assert_eq!(
        expression.prelude.as_deref(),
        Some("fn main() { tinkerpad::dump(&(\n")
    );
    assert_eq!(p.scaffold.len(), 2);
    assert_eq!(p.scaffold[1].path, PathBuf::from(".cargo/config.toml"));

    let run = p.run.as_ref().unwrap();
    assert_eq!(run.cwd.as_deref(), Some("{dir}"));
    assert_eq!(run.env["CARGO_TERM_COLOR"], "always");

    let helper = p.helper.as_ref().unwrap();
    assert_eq!(helper.command.args, ["--stdio"]);
    assert_eq!(
        helper.build.as_ref().unwrap().cwd.as_deref(),
        Some("{plugin_dir}/helper")
    );
    assert_eq!(
        helper.idle_shutdown,
        IdleShutdown::After(Duration::from_secs(1800))
    );

    let lsp = p.lsp.as_ref().unwrap();
    assert_eq!(lsp.workspace, Workspace::ScratchWorkspace);
    assert_eq!(
        lsp.init_options,
        Some(serde_json::json!({ "checkOnSave": true, "cargo": { "targetDir": true } }))
    );
    let register = lsp.register.as_ref().unwrap();
    assert_eq!(register.setting, "rust-analyzer.linkedProjects");

    assert_eq!(
        p.deps,
        Some(DepsSpec::Command {
            add: vec!["cargo".into(), "add".into(), "{name}@{version}".into()]
        })
    );

    let output = p.output.as_ref().unwrap();
    assert!(output.multiline);
    let caps = output
        .error_regex
        .captures("--> src/main.rs:3:9\nexpected `;`")
        .unwrap();
    assert_eq!(&caps["file"], "src/main.rs");
    assert_eq!(&caps["line"], "3");
}

#[test]
fn per_os_sections_resolve_for_each_os() {
    let fixture = fixture("valid/per-os");

    let linux = Plugin::load_for(&fixture, Os::Linux).unwrap();
    assert_eq!(
        linux
            .probes
            .iter()
            .map(|p| p.command.as_str())
            .collect::<Vec<_>>(),
        ["python3"]
    );
    assert_eq!(linux.run.as_ref().unwrap().args, ["{main}"]);

    let macos = Plugin::load_for(&fixture, Os::Macos).unwrap();
    assert_eq!(macos.probes[0].command, "python3");
    assert_eq!(macos.run.as_ref().unwrap().args, ["{main}"]);

    let windows = Plugin::load_for(&fixture, Os::Windows).unwrap();
    assert_eq!(
        windows
            .probes
            .iter()
            .map(|p| p.command.as_str())
            .collect::<Vec<_>>(),
        ["py", "python"]
    );
    // The windows-specific [[run]] beats the generic one.
    assert_eq!(windows.run.as_ref().unwrap().args, ["-X", "utf8", "{main}"]);
}

#[test]
fn unknown_top_level_section_is_a_warning_not_an_error() {
    let p = load("valid/future-section");
    assert_eq!(p.warnings.len(), 1, "{:?}", p.warnings);
    assert!(p.warnings[0].contains("`debug`"), "{}", p.warnings[0]);
}

#[test]
fn example_manifest_matches_the_docs() {
    let docs = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/PLUGINS.md"),
    )
    .unwrap();
    let section = docs.split("## 2. Manifest").nth(1).expect("§2 exists");
    let block = section
        .split("```toml\n")
        .nth(1)
        .and_then(|b| b.split("\n```").next())
        .expect("§2 has a toml block");
    assert_eq!(
        example_manifest().replace("\r\n", "\n").trim_end(),
        block.replace("\r\n", "\n").trim_end(),
        "src/example_manifest.toml has drifted from docs/PLUGINS.md §2"
    );
}

#[test]
fn example_manifest_loads_as_a_plugin() {
    let dir = std::env::temp_dir().join(format!("tinkerpad-example-{}", std::process::id()));
    let templates = dir.join("templates");
    std::fs::create_dir_all(&templates).unwrap();
    std::fs::write(dir.join("plugin.toml"), example_manifest()).unwrap();
    for t in ["statements.cs", "program.cs", "expression.cs"] {
        std::fs::write(templates.join(t), "// template\n").unwrap();
    }

    let result = Plugin::load_for(&dir, Os::Windows);
    let _ = std::fs::remove_dir_all(&dir);
    let p = result.unwrap_or_else(|e| panic!("example manifest should load:\n{e}"));

    assert_eq!(p.id, "csharp");
    assert_eq!(p.default_kind().id, "statements");
    assert_eq!(p.kinds.len(), 3);
    assert_eq!(p.lsp.as_ref().unwrap().workspace, Workspace::None);
    assert_eq!(
        p.deps,
        Some(DepsSpec::InlineDirective {
            directive: "#:package {name}@{version}".into()
        })
    );
    let caps = p
        .output
        .as_ref()
        .unwrap()
        .error_regex
        .captures("main.cs(3,9): error CS1002: ; expected")
        .unwrap();
    assert_eq!(&caps["line"], "3");
    assert_eq!(&caps["message"], "; expected");
}

#[test]
fn placeholders_substitute_into_resolved_commands() {
    let p = load("valid/minimal");
    let run = p.run.unwrap();
    let args: Vec<String> = run
        .args
        .iter()
        .map(|a| placeholders::substitute(a, &[("main", "/tmp/pg/main.mini")]).unwrap())
        .collect();
    assert_eq!(args, ["/tmp/pg/main.mini"]);
}

// -------------------------------------------------------------- invalid ----

#[test]
fn unknown_key_names_the_key_location_and_suggestion() {
    match fail("invalid/unknown-key") {
        ManifestError::UnknownKey(u) => {
            assert_eq!(u.name, "idle_shutdwon");
            assert_eq!(u.suggestion.as_deref(), Some("idle_shutdown"));
            assert_eq!(u.location.section.as_deref(), Some("lsp"));
            assert_eq!(u.location.line, 22);
            assert!(
                u.expected.contains(&"workspace".to_owned()),
                "{:?}",
                u.expected
            );
        }
        other => panic!("expected UnknownKey, got {other:?}"),
    }
    let text = fail("invalid/unknown-key").to_string();
    assert!(text.contains("did you mean `idle_shutdown`?"), "{text}");
    assert!(text.contains(":22:"), "{text}");
}

#[test]
fn unknown_value_suggests_the_right_variant() {
    match fail("invalid/unknown-value") {
        ManifestError::UnknownValue(u) => {
            assert_eq!(u.name, "playgrounddir");
            assert_eq!(u.suggestion.as_deref(), Some("playground_dir"));
        }
        other => panic!("expected UnknownValue, got {other:?}"),
    }
}

#[test]
fn wrong_os_name_is_an_unknown_value() {
    match fail("invalid/wrong-os-name") {
        ManifestError::UnknownValue(u) => {
            assert_eq!(u.name, "mac");
            assert_eq!(u.suggestion.as_deref(), Some("macos"));
        }
        other => panic!("expected UnknownValue, got {other:?}"),
    }
}

#[test]
fn missing_template() {
    let e = fail("invalid/missing-template");
    assert_eq!(problem_kinds(&e), [ProblemKind::MissingTemplate]);
    assert_eq!(e.problems()[0].key, "kind[0].template");
    assert!(e.to_string().contains("templates/nope.x does not exist"));
}

#[test]
fn two_default_kinds() {
    let e = fail("invalid/two-defaults");
    assert_eq!(problem_kinds(&e), [ProblemKind::DefaultKind]);
}

#[test]
fn bad_regex() {
    let e = fail("invalid/bad-regex");
    assert_eq!(problem_kinds(&e), [ProblemKind::InvalidRegex]);
    // The multi-line regex error stays indented under its list item.
    assert!(
        e.to_string().lines().skip(1).all(|l| l.starts_with("  ")),
        "{e}"
    );
}

#[test]
fn regex_missing_required_group() {
    let e = fail("invalid/regex-missing-group");
    assert_eq!(problem_kinds(&e), [ProblemKind::MissingRegexGroup]);
    assert!(e.to_string().contains("(?<message>"), "{e}");
}

#[test]
fn bad_placeholder_suggests_the_right_one() {
    let e = fail("invalid/bad-placeholder");
    assert_eq!(problem_kinds(&e), [ProblemKind::UnknownPlaceholder]);
    assert_eq!(e.problems()[0].key, "run.args[0]");
    assert!(e.to_string().contains("did you mean `{main}`?"), "{e}");
}

#[test]
fn unsupported_api_is_reported_before_anything_else() {
    // The fixture is otherwise incomplete; only the api problem is reported.
    let e = fail("invalid/bad-api");
    assert_eq!(problem_kinds(&e), [ProblemKind::UnsupportedApi]);
}

#[test]
fn bad_id_and_extension_are_both_reported() {
    let e = fail("invalid/bad-id");
    assert_eq!(
        problem_kinds(&e),
        [ProblemKind::InvalidId, ProblemKind::InvalidValue]
    );
}

#[test]
fn no_runner_for_this_os() {
    assert_eq!(
        problem_kinds(&fail("invalid/no-runner")),
        [ProblemKind::NoRunner]
    );
}

#[test]
fn overlapping_os_lists() {
    let e = fail("invalid/os-conflict");
    assert_eq!(problem_kinds(&e), [ProblemKind::OsConflict]);
    assert!(e.to_string().contains("`windows`"), "{e}");
}

#[test]
fn paths_must_stay_inside_their_folder() {
    let e = fail("invalid/unsafe-paths");
    assert!(e.to_string().contains("C:/Windows/win.ini"), "{e}");
    assert_eq!(
        problem_kinds(&e),
        [
            ProblemKind::InvalidPath,
            ProblemKind::InvalidPath,
            ProblemKind::MissingScaffoldSource
        ]
    );
}

#[test]
fn deps_with_the_wrong_shape() {
    assert_eq!(
        problem_kinds(&fail("invalid/deps-shape")),
        [ProblemKind::DepsShape]
    );
}

#[test]
fn syntax_error_has_a_location() {
    match fail("invalid/syntax") {
        ManifestError::Syntax(s) => {
            assert!(s.message.contains("name"), "{}", s.message);
            assert_eq!(s.location.section.as_deref(), Some("language"));
        }
        other => panic!("expected Syntax, got {other:?}"),
    }
}

#[test]
fn missing_manifest_is_an_io_error() {
    match Plugin::load(fixture("does-not-exist")) {
        Err(ManifestError::Io { path, .. }) => assert!(path.ends_with("plugin.toml")),
        other => panic!("expected Io, got {other:?}"),
    }
}
