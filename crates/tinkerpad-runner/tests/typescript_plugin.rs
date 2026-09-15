//! The bundled TypeScript plugin, run for real: manifest, templates, the
//! "tinkerpad" import hook, the dump library, and the output decoder together.
//!
//! Needs Node 22.7+ on PATH. Without it the tests skip locally but fail in CI
//! (where `CI` is set), so a missing toolchain can't hide a broken plugin.
//!
//! `shapes_match_the_captured_fixture` compares the dump library's output with
//! `tests/dump_cases/13-typescript-shapes.txt`, which the dump parser's
//! table-driven tests also run. After an intended change to the library,
//! regenerate it:
//!
//! ```text
//! TINKERPAD_UPDATE_FIXTURES=1 cargo test -p tinkerpad-runner --test typescript_plugin
//! ```

use std::path::{Path, PathBuf};

use tinkerpad_core::dump::RichNode;
use tinkerpad_core::{
    KindId, LanguageId, Playground, PlaygroundId, RunEvent as UiEvent, ToolchainState,
};
use tinkerpad_plugin::{PROBE_TIMEOUT, Plugin, Resolver, ToolchainOverrides, probe, scaffold};
use tinkerpad_runner::dump::PREFIX;
use tinkerpad_runner::{OutputDecoder, RunContext, RunEvent, run, run_spec};

const FIXTURE: &str = "tests/dump_cases/13-typescript-shapes.txt";

fn manifest_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("tinkerpad-ts-{}-{name}", std::process::id()));
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

struct Node {
    plugin: Plugin,
    resolver: Resolver,
    program: String,
    version: String,
}

/// The plugin and a ready Node, or `None` to skip (outside CI only).
async fn node(scratch: &Path) -> Option<Node> {
    let plugin = Plugin::load(manifest_dir().join("../../plugins/typescript")).unwrap();
    let resolver = Resolver::new(
        std::env::var_os("PATH").unwrap_or_default(),
        ToolchainOverrides::default(),
    );
    match probe(&plugin, &resolver, scratch, PROBE_TIMEOUT).await {
        ToolchainState::Ready {
            program, version, ..
        } => Some(Node {
            plugin,
            resolver,
            program,
            version: version.unwrap_or_default(),
        }),
        other => {
            assert!(
                std::env::var_os("CI").is_none(),
                "Node isn't ready in CI: {other:?}"
            );
            eprintln!("skipping: Node 22.7+ isn't available ({other:?})");
            None
        }
    }
}

struct Output {
    events: Vec<UiEvent>,
    stdout: Vec<u8>,
    stderr: String,
    code: Option<i32>,
}

/// Scaffold a playground of `kind`, optionally replace its main file, and run it.
async fn run_playground(node: &Node, scratch: &Path, kind: &str, source: Option<&str>) -> Output {
    let kind = node.plugin.kind(kind).unwrap();
    let dir = scratch.join("library").join(&kind.id);
    let name = "TS Test";
    for (path, bytes) in scaffold::playground_files(&node.plugin, kind, name, "2026-09-15").unwrap()
    {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    let main_file = dir.join(&node.plugin.main_path);
    if let Some(source) = source {
        std::fs::write(&main_file, source).unwrap();
    }
    let playground = Playground {
        id: PlaygroundId::new("01K0000000000000000000TSTS"),
        name: name.into(),
        language: LanguageId::new("typescript"),
        kind: KindId::new(kind.id.clone()),
        main_file,
        dir,
        created_ms: 0,
        tags: Vec::new(),
    };
    let scratch_root = scratch.join("scratch");
    let ctx = RunContext {
        resolver: &node.resolver,
        scratch_root: &scratch_root,
        toolchain_program: &node.program,
        toolchain_version: &node.version,
    };
    let spec = run_spec(&node.plugin, &playground, &ctx).unwrap();

    let mut handle = run(spec).unwrap();
    let mut decoder = OutputDecoder::new();
    let mut out = Output {
        events: Vec::new(),
        stdout: Vec::new(),
        stderr: String::new(),
        code: None,
    };
    while let Some(event) = handle.recv().await {
        match event {
            RunEvent::Stdout(b) => {
                out.stdout.extend_from_slice(&b);
                decoder.stdout(&b, &mut out.events);
            }
            RunEvent::Stderr(b) => {
                out.stderr.push_str(&String::from_utf8_lossy(&b));
                decoder.stderr(&b, &mut out.events);
            }
            RunEvent::Exited { code, .. } => {
                decoder.finish(&mut out.events);
                out.code = code;
            }
            RunEvent::Cancelled { .. } => panic!("cancelled"),
            RunEvent::Started { .. } => {}
        }
    }
    // Merge adjacent text so chunking doesn't matter.
    let mut merged: Vec<UiEvent> = Vec::new();
    for event in out.events.drain(..) {
        match (merged.last_mut(), event) {
            (Some(UiEvent::Stdout { text: a }), UiEvent::Stdout { text: b }) => a.push_str(&b),
            (Some(UiEvent::Stderr { text: a }), UiEvent::Stderr { text: b }) => a.push_str(&b),
            (_, e) => merged.push(e),
        }
    }
    out.events = merged;
    out
}

fn stdout_only(events: &[UiEvent]) -> Vec<&UiEvent> {
    events
        .iter()
        .filter(|e| !matches!(e, UiEvent::Stderr { .. }))
        .collect()
}

#[tokio::test]
async fn statements_template_dumps_a_table() {
    let s = Scratch::new("statements");
    let Some(node) = node(&s.0).await else { return };
    let out = run_playground(&node, &s.0, "statements", None).await;
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
    let s = Scratch::new("program");
    let Some(node) = node(&s.0).await else { return };
    let out = run_playground(&node, &s.0, "program", None).await;
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stderr, "");
    match &out.events[..] {
        [UiEvent::Stdout { text }, UiEvent::Rich { value }] => {
            assert_eq!(text, "Hello from TS Test\n");
            assert!(matches!(value.node, RichNode::Tree { .. }), "{value:?}");
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn console_log_and_dump_interleave_in_order() {
    let s = Scratch::new("interleave");
    let Some(node) = node(&s.0).await else { return };
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
    let out = run_playground(&node, &s.0, "statements", Some(source)).await;
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stderr, "to stderr\n");

    let shape: Vec<String> = stdout_only(&out.events)
        .iter()
        .map(|e| match e {
            UiEvent::Stdout { text } => format!("text {text:?}"),
            UiEvent::Rich { value } => serde_json::to_value(value).unwrap()["kind"]
                .as_str()
                .unwrap()
                .to_owned(),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(
        shape,
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
    let big = out.events.iter().rev().find_map(|e| match e {
        UiEvent::Rich { value } => match &value.node {
            RichNode::Table {
                rows, truncated, ..
            } => Some((rows.len(), *truncated)),
            _ => None,
        },
        _ => None,
    });
    assert_eq!(big, Some((1000, true)), "tables keep 1,000 rows");
}

#[tokio::test]
async fn transform_types_keeps_line_numbers() {
    let s = Scratch::new("lines");
    let Some(node) = node(&s.0).await else { return };
    // `enum` and parameter properties need the transform, not just stripping.
    let source = "import { dump } from \"tinkerpad\";\n\
                  enum Color {\n  Red,\n  Green,\n}\n\
                  class P { constructor(public x: number) {} }\n\
                  dump(new P(Color.Green));\n\
                  throw new Error(\"on line 8\");\n";
    assert_eq!(source.lines().position(|l| l.starts_with("throw")), Some(7));
    let out = run_playground(&node, &s.0, "statements", Some(source)).await;
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
}

#[tokio::test]
async fn shapes_match_the_captured_fixture() {
    let s = Scratch::new("shapes");
    let Some(node) = node(&s.0).await else { return };
    let source =
        std::fs::read_to_string(manifest_dir().join("tests/plugins/typescript/shapes.ts")).unwrap();
    let out = run_playground(&node, &s.0, "statements", Some(&source)).await;
    assert_eq!(out.code, Some(0), "stderr: {}", out.stderr);
    assert_eq!(out.stderr, "");

    let captured = case_file(&out.stdout);
    let path = manifest_dir().join(FIXTURE);
    if std::env::var_os("TINKERPAD_UPDATE_FIXTURES").is_some() {
        std::fs::write(&path, &captured).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap()
        .replace("\r\n", "\n");
    assert!(
        captured == expected,
        "the dump library's output changed. If that's intended, regenerate {FIXTURE} (see this file's docs).\n--- captured ---\n{captured}"
    );
}

/// Turn raw stdout into a `dump_cases` file: the bytes as `in` lines, then the
/// expected text and rich events.
fn case_file(stdout: &[u8]) -> String {
    let mut file = String::from(
        "# Captured from the TypeScript plugin's dump library running\n\
         # crates/tinkerpad-runner/tests/plugins/typescript/shapes.ts.\n\
         # Regenerate: TINKERPAD_UPDATE_FIXTURES=1 cargo test -p tinkerpad-runner --test typescript_plugin\n",
    );
    for line in stdout.split_inclusive(|&b| b == b'\n') {
        file.push_str("in ");
        file.push_str(&escape(line));
        file.push('\n');
    }
    let mut rest = stdout;
    while !rest.is_empty() {
        match memchr::memmem::find(rest, PREFIX) {
            Some(at) => {
                if at > 0 {
                    file.push_str(&format!("out text {}\n", escape(&rest[..at])));
                }
                let payload = &rest[at + PREFIX.len()..];
                let end = memchr::memchr(0x07, payload).expect("terminated dump");
                file.push_str("out rich ");
                file.push_str(std::str::from_utf8(&payload[..end]).unwrap());
                file.push('\n');
                rest = &payload[end + 1..];
            }
            None => {
                file.push_str(&format!("out text {}\n", escape(rest)));
                rest = &[];
            }
        }
    }
    file
}

/// Escape bytes for a case file (see `dump_cases.rs`).
fn escape(bytes: &[u8]) -> String {
    let text = std::str::from_utf8(bytes).expect("UTF-8 output");
    let mut out = String::new();
    let chars: Vec<char> = text.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\u{1b}' => out.push_str("\\e"),
            '\u{7}' => out.push_str("\\a"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ' ' if chars[i + 1..].iter().all(|&c| c == ' ') => out.push_str("\\s"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}
