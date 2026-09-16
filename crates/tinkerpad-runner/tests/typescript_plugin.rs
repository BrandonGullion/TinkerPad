//! The bundled TypeScript plugin, run for real: manifest, templates, the
//! "tinkerpad" import hook, the dump library, and the output decoder together.
//!
//! Needs Node 22.7+ on PATH (skipped without it, except in CI).
//!
//! `shapes_match_the_captured_fixture` compares the dump library's output with
//! `tests/dump_cases/13-typescript-shapes.txt`, which the dump parser's
//! table-driven tests also run. After an intended change to the library:
//!
//! ```text
//! TINKERPAD_UPDATE_FIXTURES=1 cargo test -p tinkerpad-runner --test typescript_plugin
//! ```

mod common;

use common::{PLAYGROUND_NAME, Scratch, check_fixture, manifest_dir, run_playground, toolchain};
use tinkerpad_core::RunEvent as UiEvent;
use tinkerpad_core::dump::RichNode;

const PLUGIN: &str = "typescript";

#[tokio::test]
async fn statements_template_dumps_a_table() {
    let s = Scratch::new(PLUGIN, "statements");
    let Some(tc) = toolchain(PLUGIN, &s.0).await else {
        return;
    };
    let out = run_playground(&tc, &s.0, "statements", None).await;
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stderr, "", "no warnings expected");
    match &out.events[..] {
        [UiEvent::Rich { value }] => {
            assert_eq!(value.title.as_deref(), Some("people"));
            let RichNode::Table {
                columns,
                rows,
                truncated,
            } = &value.node
            else {
                panic!("{value:?}")
            };
            let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
            assert_eq!(names, ["name", "born"]);
            assert_eq!(rows.len(), 2);
            assert!(!truncated);
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn program_template_prints_and_dumps() {
    let s = Scratch::new(PLUGIN, "program");
    let Some(tc) = toolchain(PLUGIN, &s.0).await else {
        return;
    };
    let out = run_playground(&tc, &s.0, "program", None).await;
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stderr, "");
    match &out.events[..] {
        [UiEvent::Stdout { text }, UiEvent::Rich { value }] => {
            assert_eq!(text, &format!("Hello from {PLAYGROUND_NAME}\n"));
            assert!(matches!(value.node, RichNode::Tree { .. }), "{value:?}");
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn console_log_and_dump_interleave_in_order() {
    let s = Scratch::new(PLUGIN, "interleave");
    let Some(tc) = toolchain(PLUGIN, &s.0).await else {
        return;
    };
    let source = r#"import { dump } from "tinkerpad";
console.log("a");
dump(1);
console.log("b");
console.error("to stderr");
dump([{ x: 1 }]);
process.stdout.write("c");
dump("inline");
console.log("d");
const rows = dump(Array.from({ length: 1005 }, (_, i) => ({ i })));
console.log(rows.length);
"#;
    let out = run_playground(&tc, &s.0, "statements", Some(source)).await;
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
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
            r#"text "d\n""#,
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
async fn transform_types_keeps_line_numbers() {
    let s = Scratch::new(PLUGIN, "lines");
    let Some(tc) = toolchain(PLUGIN, &s.0).await else {
        return;
    };
    // `enum` and parameter properties need the transform, not just stripping.
    let source = "import { dump } from \"tinkerpad\";\n\
                  enum Color {\n  Red,\n  Green,\n}\n\
                  class P { constructor(public x: number) {} }\n\
                  dump(new P(Color.Green));\n\
                  throw new Error(\"on line 8\");\n";
    assert_eq!(source.lines().position(|l| l.starts_with("throw")), Some(7));
    let out = run_playground(&tc, &s.0, "statements", Some(source)).await;
    assert_eq!(out.code, Some(1));
    assert!(
        out.stderr.contains("main.ts:8"),
        "stack trace should point at line 8:\n{}",
        out.stderr
    );
    assert!(
        !out.stderr.contains("ExperimentalWarning"),
        "{}",
        out.stderr
    );

    // The first stack frame reaches the UI as an annotation.
    let error = out
        .annotations()
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("no annotation in:\n{}", out.stderr));
    assert_eq!(error.message, "Error: on line 8", "{error:?}");
    assert!(
        error
            .file
            .as_deref()
            .is_some_and(|f| f.replace('\\', "/").ends_with("main.ts")),
        "{error:?}"
    );
    assert_eq!((error.line, error.col), (8, Some(7)), "{error:?}");
    assert_eq!(
        out.annotations().len(),
        1,
        "later frames have no message line, so they don't match: {:?}",
        out.annotations()
    );
}

#[tokio::test]
async fn ordinary_output_is_not_annotated() {
    let s = Scratch::new(PLUGIN, "no-errors");
    let Some(tc) = toolchain(PLUGIN, &s.0).await else {
        return;
    };
    let out = run_playground(&tc, &s.0, "program", None).await;
    assert_eq!(out.code, Some(0));
    assert!(out.annotations().is_empty(), "{:?}", out.annotations());
}

#[tokio::test]
async fn shapes_match_the_captured_fixture() {
    let s = Scratch::new(PLUGIN, "shapes");
    let Some(tc) = toolchain(PLUGIN, &s.0).await else {
        return;
    };
    let source =
        std::fs::read_to_string(manifest_dir().join("tests/plugins/typescript/shapes.ts")).unwrap();
    let out = run_playground(&tc, &s.0, "statements", Some(&source)).await;
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stderr, "");
    check_fixture(
        "tests/dump_cases/13-typescript-shapes.txt",
        "Captured from the TypeScript plugin's dump library running\n\
         crates/tinkerpad-runner/tests/plugins/typescript/shapes.ts.\n\
         Regenerate: TINKERPAD_UPDATE_FIXTURES=1 cargo test -p tinkerpad-runner --test typescript_plugin",
        &out.stdout,
    );
}
