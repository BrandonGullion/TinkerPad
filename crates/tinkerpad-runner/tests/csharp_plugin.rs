//! The bundled C# plugin, run for real with `dotnet run --file`: manifest,
//! templates, the scaffolded Directory.Build.props that compiles the dump
//! library in, the library itself, the compile error regex, and the output
//! decoder together.
//!
//! Needs the .NET 10 SDK on PATH (skipped without it, except in CI). Each test
//! compiles a playground, so expect a few seconds apiece.
//!
//! `shapes_match_the_captured_fixture` compares the dump library's output with
//! `tests/dump_cases/14-csharp-shapes.txt`. After an intended change:
//!
//! ```text
//! TINKERPAD_UPDATE_FIXTURES=1 cargo test -p tinkerpad-runner --test csharp_plugin
//! ```

mod common;

use common::{Scratch, check_fixture, manifest_dir, run_playground, template_text, toolchain};
use tinkerpad_core::Level;
use tinkerpad_core::dump::RichNode;

const PLUGIN: &str = "csharp";

fn table_columns(node: &RichNode) -> Vec<String> {
    match node {
        RichNode::Table { columns, .. } => columns.iter().map(|c| c.name.clone()).collect(),
        other => panic!("not a table: {other:?}"),
    }
}

#[tokio::test]
async fn statements_template_dumps_a_table() {
    let s = Scratch::new(PLUGIN, "statements");
    let Some(tc) = toolchain(PLUGIN, &s.0).await else {
        return;
    };
    let out = run_playground(&tc, &s.0, "statements", None).await;
    assert_eq!(
        out.code,
        Some(0),
        "stdout: {:?}\nstderr: {}",
        out.events,
        out.stderr
    );
    assert_eq!(out.stderr, "");
    assert_eq!(
        out.shape(),
        ["table"],
        "no build output or warnings expected"
    );
    let table = out.dumps()[0];
    assert_eq!(table.title.as_deref(), Some("people"));
    assert_eq!(table_columns(&table.node), ["Name", "Born"]);
    assert_eq!(out.last_table(), Some((2, false)));
}

#[tokio::test]
async fn program_template_prints_and_dumps() {
    let s = Scratch::new(PLUGIN, "program");
    let Some(tc) = toolchain(PLUGIN, &s.0).await else {
        return;
    };
    let out = run_playground(&tc, &s.0, "program", None).await;
    assert_eq!(out.code, Some(0), "{:?} {}", out.events, out.stderr);
    assert_eq!(out.stderr, "");
    assert_eq!(out.shape(), [r#"text "Hello from C#\n""#, "tree"]);
}

#[tokio::test]
async fn expression_template_dumps_its_locked_expression() {
    let s = Scratch::new(PLUGIN, "expression");
    let Some(tc) = toolchain(PLUGIN, &s.0).await else {
        return;
    };
    let text = template_text(&tc, "expression");
    assert!(
        text.starts_with("TinkerPad.Dump.Show(\n") && text.ends_with("\n);\n"),
        "{text}"
    );
    let out = run_playground(&tc, &s.0, "expression", None).await;
    assert_eq!(out.code, Some(0), "{:?} {}", out.events, out.stderr);
    assert_eq!(out.shape(), ["table"]);
    assert_eq!(table_columns(&out.dumps()[0].node), ["Number", "Square"]);
    assert_eq!(out.last_table(), Some((5, false)));
}

#[tokio::test]
async fn console_output_and_dumps_interleave_in_order() {
    let s = Scratch::new(PLUGIN, "interleave");
    let Some(tc) = toolchain(PLUGIN, &s.0).await else {
        return;
    };
    let source = r#"using TinkerPad;
Console.WriteLine("a");
1.Dump();
Console.WriteLine("b");
Console.Error.WriteLine("to stderr");
new[] { new { X = 1 } }.Dump();
Console.Write("c");
"inline".Dump();
Console.WriteLine("héllo 日本 🎉");
var rows = Enumerable.Range(0, 1005).Select(i => new { I = i }).ToList().Dump();
Console.WriteLine(rows.Count);
"#;
    let out = run_playground(&tc, &s.0, "statements", Some(source)).await;
    assert_eq!(out.code, Some(0), "{:?} {}", out.events, out.stderr);
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
    let source = "using TinkerPad;\n\nConsole.WriteLine(\"missing semicolon\")\nvar unused = 1;\n";
    let out = run_playground(&tc, &s.0, "statements", Some(source)).await;
    assert_ne!(out.code, Some(0));

    // The error reaches the UI as an annotation, right after the line it was
    // found in.
    let text = String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n") + &out.stderr;
    let error = out
        .annotations()
        .into_iter()
        .find(|a| a.severity == Level::Error)
        .unwrap_or_else(|| panic!("no error annotation in:\n{text}\n{:?}", out.events));
    assert!(
        error
            .file
            .as_deref()
            .is_some_and(|f| f.ends_with("main.cs")),
        "{error:?}"
    );
    assert_eq!((error.line, error.col), (3, Some(39)), "{error:?}");
    assert_eq!(error.message, "CS1002: ; expected");
    assert!(error.text.contains("error CS1002"), "{error:?}");
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
        std::fs::read_to_string(manifest_dir().join("tests/plugins/csharp/shapes.cs")).unwrap();
    let out = run_playground(&tc, &s.0, "statements", Some(&source)).await;
    assert_eq!(out.code, Some(0), "{:?} {}", out.events, out.stderr);
    assert_eq!(out.stderr, "");
    check_fixture(
        "tests/dump_cases/14-csharp-shapes.txt",
        "Captured from the C# plugin's dump library running\n\
         crates/tinkerpad-runner/tests/plugins/csharp/shapes.cs.\n\
         Regenerate: TINKERPAD_UPDATE_FIXTURES=1 cargo test -p tinkerpad-runner --test csharp_plugin",
        &out.stdout,
    );
}
