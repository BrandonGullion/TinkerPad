//! The bundled Rust plugin, run for real with `cargo run`: manifest, the
//! scaffolded Cargo.toml and .cargo/config.toml with absolute paths, the shared
//! target folder, the dump crate, the compile error regex, and the output
//! decoder together.
//!
//! Needs Cargo 1.85+ on PATH (skipped without it, except in CI). The first
//! build downloads and compiles serde, so most tests share one target folder
//! under the system temp folder; `build_times_with_a_shared_target` starts
//! from an empty one and prints its timings.
//!
//! `shapes_match_the_captured_fixture` compares the dump crate's output with
//! `tests/dump_cases/15-rust-shapes.txt`. After an intended change:
//!
//! ```text
//! TINKERPAD_UPDATE_FIXTURES=1 cargo test -p tinkerpad-runner --test rust_plugin
//! ```

mod common;

use std::path::{Path, PathBuf};
use std::time::Instant;

use common::{
    Scratch, check_fixture, manifest_dir, run_scaffolded, scaffold_playground, template_text,
    toolchain,
};
use tinkerpad_core::dump::RichNode;

const PLUGIN: &str = "rust";

/// Shared by the tests in this file (and kept between test runs), so serde
/// compiles once. Cargo's own lock serialises builds that overlap.
fn shared_scratch_root() -> PathBuf {
    std::env::temp_dir().join("tinkerpad-rust-plugin-tests-scratch")
}

async fn run_new(
    tc: &common::Toolchain,
    s: &Scratch,
    folder: &str,
    kind: &str,
    source: Option<&str>,
) -> common::Output {
    let root = shared_scratch_root();
    let p = scaffold_playground(tc, &s.0.join("library"), folder, kind, &root);
    if let Some(source) = source {
        std::fs::write(&p.main_file, source).unwrap();
    }
    run_scaffolded(tc, &p, &root).await
}

#[tokio::test]
async fn scaffold_writes_absolute_paths_that_exist() {
    let s = Scratch::new(PLUGIN, "scaffold");
    let Some(tc) = toolchain(PLUGIN, &s.0).await else {
        return;
    };
    let root = s.0.join("scratch");
    let p = scaffold_playground(&tc, &s.0.join("library"), "fizz-buzz", "program", &root);
    assert_eq!(p.main_file, p.dir.join("src").join("main.rs"));

    let cargo = std::fs::read_to_string(p.dir.join("Cargo.toml")).unwrap();
    assert!(cargo.contains("name = \"fizz-buzz\""), "{cargo}");
    assert!(cargo.contains("edition = \"2024\""), "{cargo}");
    assert!(cargo.contains("debug = \"line-tables-only\""), "{cargo}");
    assert!(cargo.contains("[profile.tp-debug]"), "{cargo}");
    let manifest: toml::Table = toml::from_str(&cargo).unwrap();
    let dump_path = manifest["dependencies"]["tinkerpad"]["path"]
        .as_str()
        .unwrap();
    assert!(Path::new(dump_path).is_absolute(), "{dump_path}");
    assert!(
        Path::new(dump_path).join("Cargo.toml").is_file(),
        "{dump_path}"
    );
    assert!(
        !dump_path.contains('\\'),
        "paths use forward slashes: {dump_path}"
    );

    let config: toml::Table =
        toml::from_str(&std::fs::read_to_string(p.dir.join(".cargo/config.toml")).unwrap())
            .unwrap();
    let target = config["build"]["target-dir"].as_str().unwrap();
    assert!(Path::new(target).is_absolute(), "{target}");
    assert!(
        Path::new(target).starts_with(root.join("rust"))
            || target.starts_with(&common_path(&root.join("rust"))),
        "{target} should be in the plugin's scratch folder"
    );
}

fn common_path(p: &Path) -> String {
    tinkerpad_plugin::scaffold::template_path(p)
}

#[tokio::test]
async fn program_template_prints_and_dumps() {
    let s = Scratch::new(PLUGIN, "program");
    let Some(tc) = toolchain(PLUGIN, &s.0).await else {
        return;
    };
    let out = run_new(&tc, &s, "program-test", "program", None).await;
    assert_eq!(out.code, Some(0), "{:?}\n{}", out.events, out.stderr);
    assert_eq!(out.stderr, "", "no warnings or build output expected");
    assert_eq!(out.shape(), [r#"text "Hello from Rust\n""#, "table"]);
    let table = out.dumps()[0];
    assert_eq!(table.title.as_deref(), Some("people"));
    match &table.node {
        RichNode::Table { columns, rows, .. } => {
            let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
            assert_eq!(names, ["name", "born"], "fields in declaration order");
            assert_eq!(rows.len(), 2);
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn expression_template_dumps_its_locked_expression() {
    let s = Scratch::new(PLUGIN, "expression");
    let Some(tc) = toolchain(PLUGIN, &s.0).await else {
        return;
    };
    let text = template_text(&tc, "expression");
    assert!(
        text.starts_with("fn main() {\n    tinkerpad::dump(&(\n")
            && text.ends_with("\n    ));\n}\n"),
        "{text}"
    );
    let out = run_new(&tc, &s, "expression-test", "expression", None).await;
    assert_eq!(out.code, Some(0), "{:?}\n{}", out.events, out.stderr);
    assert_eq!(out.stderr, "");
    assert_eq!(out.shape(), ["tree"]);
    assert_eq!(
        out.dumps()[0].node,
        RichNode::Tree {
            value: serde_json::json!([1, 4, 9, 16, 25, 36, 49, 64, 81, 100]),
            ty: Some("Vec<u64>".into())
        }
    );
}

#[tokio::test]
async fn output_and_dumps_interleave_in_order() {
    let s = Scratch::new(PLUGIN, "interleave");
    let Some(tc) = toolchain(PLUGIN, &s.0).await else {
        return;
    };
    let source = r#"use std::collections::BTreeMap;
use tinkerpad::Dump;

fn main() {
    println!("a");
    1.dump();
    println!("b");
    eprintln!("to stderr");
    vec![BTreeMap::from([("x", 1)])].dump();
    print!("c");
    "inline".dump();
    println!("héllo 日本 🎉");
    let rows: Vec<_> = (0..1005).map(|i| BTreeMap::from([("i", i)])).collect();
    println!("{}", rows.dump().len());
}
"#;
    let out = run_new(&tc, &s, "interleave-test", "program", Some(source)).await;
    assert_eq!(out.code, Some(0), "{:?}\n{}", out.events, out.stderr);
    assert_eq!(out.stderr, "to stderr\n");
    assert_eq!(
        out.shape(),
        [
            r#"text "a\n""#,
            "value",
            r#"text "b\n""#,
            "table",
            r#"text "c""#,
            "value",
            r#"text "héllo 日本 🎉\n""#,
            "table",
            r#"text "1005\n""#,
        ]
    );
    assert_eq!(
        out.last_table(),
        Some((1000, true)),
        "tables keep 1,000 rows"
    );
}

#[tokio::test]
async fn compile_errors_match_the_output_regex() {
    let s = Scratch::new(PLUGIN, "errors");
    let Some(tc) = toolchain(PLUGIN, &s.0).await else {
        return;
    };
    let source =
        "fn main() {\n    let n: u32 = 1;\n    let s: String = n;\n    println!(\"{s}\");\n}\n";
    let out = run_new(&tc, &s, "errors-test", "program", Some(source)).await;
    assert_ne!(out.code, Some(0));

    let output = tc.plugin.output.as_ref().expect("[output]");
    assert!(
        output.multiline,
        "rustc puts the location on the line after the message"
    );
    let text = out.stderr.clone();
    let error = output
        .error_regex
        .captures_iter(&text)
        .find(|c| &c["severity"] == "error")
        .unwrap_or_else(|| panic!("no error matched in:\n{text}"));
    assert_eq!(&error["message"], "mismatched types", "{text}");
    assert!(
        error["file"].replace('\\', "/").ends_with("src/main.rs"),
        "{}",
        &error["file"]
    );
    assert_eq!((&error["line"], &error["col"]), ("3", "21"), "{text}");
}

#[tokio::test]
async fn shapes_match_the_captured_fixture() {
    let s = Scratch::new(PLUGIN, "shapes");
    let Some(tc) = toolchain(PLUGIN, &s.0).await else {
        return;
    };
    let source =
        std::fs::read_to_string(manifest_dir().join("tests/plugins/rust/shapes.rs")).unwrap();
    let out = run_new(&tc, &s, "shapes-test", "program", Some(&source)).await;
    assert_eq!(out.code, Some(0), "{:?}\n{}", out.events, out.stderr);
    assert_eq!(out.stderr, "");
    check_fixture(
        "tests/dump_cases/15-rust-shapes.txt",
        "Captured from the Rust plugin's dump crate running\n\
         crates/tinkerpad-runner/tests/plugins/rust/shapes.rs.\n\
         Regenerate: TINKERPAD_UPDATE_FIXTURES=1 cargo test -p tinkerpad-runner --test rust_plugin",
        &out.stdout,
    );
}

/// The four timings the work item asks for, from an empty target folder.
#[tokio::test]
async fn build_times_with_a_shared_target() {
    let s = Scratch::new(PLUGIN, "timings");
    let Some(tc) = toolchain(PLUGIN, &s.0).await else {
        return;
    };
    let root = s.0.join("scratch");
    let library = s.0.join("library");
    let timed = |label: &'static str| {
        let start = Instant::now();
        move |out: &common::Output| {
            assert_eq!(
                out.code,
                Some(0),
                "{label}: {:?}\n{}",
                out.events,
                out.stderr
            );
            eprintln!("timing: {label}: {} ms", start.elapsed().as_millis());
        }
    };

    let first = scaffold_playground(&tc, &library, "first", "program", &root);
    let done = timed("first build of a fresh playground (empty target folder)");
    done(&run_scaffolded(&tc, &first, &root).await);

    let done = timed("second run, unchanged");
    done(&run_scaffolded(&tc, &first, &root).await);

    let main = std::fs::read_to_string(&first.main_file).unwrap();
    std::fs::write(
        &first.main_file,
        main.replace("Hello from Rust", "Hello again"),
    )
    .unwrap();
    let done = timed("second run after a one-line edit");
    let out = run_scaffolded(&tc, &first, &root).await;
    done(&out);
    assert!(out.shape()[0].contains("Hello again"));

    let second = scaffold_playground(&tc, &library, "second", "program", &root);
    let done = timed("first run of a second playground (shared target folder)");
    done(&run_scaffolded(&tc, &second, &root).await);

    assert!(
        root.join("rust").join("target").is_dir(),
        "both built into the shared target"
    );
    assert!(!first.dir.join("target").exists() && !second.dir.join("target").exists());
}
