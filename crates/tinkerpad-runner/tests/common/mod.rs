//! Helpers for tests that run a bundled language plugin for real: probe its
//! toolchain, scaffold a playground, run it through `run_spec` and the output
//! decoder, and compare dump output with a captured fixture.

#![allow(dead_code)] // Each test crate uses a different subset.

use std::path::{Path, PathBuf};

use tinkerpad_core::dump::RichNode;
use tinkerpad_core::{
    KindId, LanguageId, Playground, PlaygroundId, RichValue, RunEvent as UiEvent, ToolchainState,
};
use tinkerpad_plugin::scaffold::{self, ScaffoldVars};
use tinkerpad_plugin::{Plugin, Resolver, ToolchainOverrides, probe};
use tinkerpad_runner::dump::PREFIX;
use tinkerpad_runner::{OutputDecoder, RunContext, RunEvent, run, run_spec};

/// The name every test playground gets.
pub const PLAYGROUND_NAME: &str = "Test Playground";

pub fn manifest_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(plugin: &str, name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("tinkerpad-{plugin}-{}-{name}", std::process::id()));
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

/// The app gives a probe 3 s, because startup shouldn't wait on it. These
/// tests are about runs, not about how quickly `dotnet --version` answers on
/// a busy CI runner with several of them starting at once, which is where
/// 3 s has run out.
const TEST_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

pub struct Toolchain {
    pub plugin: Plugin,
    pub resolver: Resolver,
    pub program: String,
    pub version: String,
}

/// The bundled plugin `folder` with its toolchain ready, or `None` to skip.
/// In CI (where `CI` is set) a missing toolchain fails instead, so it can't
/// hide a broken plugin.
pub async fn toolchain(folder: &str, scratch: &Path) -> Option<Toolchain> {
    let plugin = Plugin::load(manifest_dir().join("../../plugins").join(folder)).unwrap();
    let resolver = Resolver::new(
        std::env::var_os("PATH").unwrap_or_default(),
        ToolchainOverrides::default(),
    );
    match probe(&plugin, &resolver, scratch, TEST_PROBE_TIMEOUT).await {
        ToolchainState::Ready {
            program, version, ..
        } => Some(Toolchain {
            plugin,
            resolver,
            program,
            version: version.unwrap_or_default(),
        }),
        other => {
            assert!(
                std::env::var_os("CI").is_none(),
                "{folder}'s toolchain isn't ready in CI: {other:?}"
            );
            eprintln!("skipping: {folder}'s toolchain isn't available ({other:?})");
            None
        }
    }
}

pub struct Output {
    /// Decoded events, adjacent text merged and `\r\n` normalised to `\n`.
    pub events: Vec<UiEvent>,
    /// Raw stdout bytes.
    pub stdout: Vec<u8>,
    pub stderr: String,
    pub code: Option<i32>,
}

impl Output {
    /// Events as short strings: `text "..."` for stdout, the kind for dumps.
    /// Stderr is left out.
    pub fn shape(&self) -> Vec<String> {
        self.events
            .iter()
            .filter(|e| !matches!(e, UiEvent::Stderr { .. }))
            .map(|e| match e {
                UiEvent::Stdout { text } => format!("text {text:?}"),
                UiEvent::Rich { value } => serde_json::to_value(value).unwrap()["kind"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
                other => format!("{other:?}"),
            })
            .collect()
    }

    pub fn dumps(&self) -> Vec<&RichValue> {
        self.events
            .iter()
            .filter_map(|e| match e {
                UiEvent::Rich { value } => Some(value),
                _ => None,
            })
            .collect()
    }

    /// Positions the plugin's `[output]` pattern found in the output.
    pub fn annotations(&self) -> Vec<&tinkerpad_core::Annotation> {
        self.events
            .iter()
            .filter_map(|e| match e {
                UiEvent::Annotation(a) => Some(a),
                _ => None,
            })
            .collect()
    }

    /// `(rows, truncated)` of the last table dumped.
    pub fn last_table(&self) -> Option<(usize, bool)> {
        self.dumps().iter().rev().find_map(|v| match &v.node {
            RichNode::Table {
                rows, truncated, ..
            } => Some((rows.len(), *truncated)),
            _ => None,
        })
    }
}

/// The playground's main file as a new playground of `kind` would start.
pub fn template_text(tc: &Toolchain, kind: &str) -> String {
    let kind = tc.plugin.kind(kind).unwrap();
    let scratch = Path::new("/scratch");
    scaffold::main_file(
        kind,
        &vars(tc, "test-playground", "test-playground", scratch),
    )
    .unwrap()
}

fn vars<'a>(
    tc: &'a Toolchain,
    folder: &'a str,
    package: &'a str,
    scratch: &'a Path,
) -> ScaffoldVars<'a> {
    ScaffoldVars {
        name: PLAYGROUND_NAME,
        date: "2026-09-15",
        folder,
        package,
        plugin_dir: &tc.plugin.dir,
        scratch,
    }
}

/// Write a new playground of `kind` into `library/<folder>` as the app would,
/// with the plugin's scratch folder under `scratch_root`.
pub fn scaffold_playground(
    tc: &Toolchain,
    library: &Path,
    folder: &str,
    kind: &str,
    scratch_root: &Path,
) -> Playground {
    let kind = tc.plugin.kind(kind).unwrap();
    let dir = library.join(folder);
    let id = PlaygroundId::new("01K0000000000000000000TEST");
    let package = scaffold::package_name(folder, id.as_str());
    let scratch = tinkerpad_runner::spec::scratch_dir(scratch_root, &tc.plugin);
    let out = scaffold::playground_files(&tc.plugin, kind, &vars(tc, folder, &package, &scratch))
        .unwrap();
    for (path, bytes) in out.files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    Playground {
        id,
        name: PLAYGROUND_NAME.into(),
        language: LanguageId::new(tc.plugin.id.as_str()),
        kind: KindId::new(kind.id.clone()),
        main_file: dir.join(&tc.plugin.main_path),
        dir,
        created_ms: 0,
        tags: Vec::new(),
        pinned: false,
    }
}

/// Scaffold a playground of `kind` in `scratch`, optionally replace its main
/// file, and run it to the end.
pub async fn run_playground(
    tc: &Toolchain,
    scratch: &Path,
    kind: &str,
    source: Option<&str>,
) -> Output {
    let scratch_root = scratch.join("scratch");
    let playground = scaffold_playground(tc, &scratch.join("library"), kind, kind, &scratch_root);
    if let Some(source) = source {
        std::fs::write(&playground.main_file, source).unwrap();
    }
    run_scaffolded(tc, &playground, &scratch_root).await
}

/// Run an existing playground to the end.
pub async fn run_scaffolded(
    tc: &Toolchain,
    playground: &Playground,
    scratch_root: &Path,
) -> Output {
    let ctx = RunContext {
        resolver: &tc.resolver,
        scratch_root,
        toolchain_program: &tc.program,
        toolchain_version: &tc.version,
    };
    let spec = run_spec(&tc.plugin, playground, &ctx).unwrap();

    let mut handle = run(spec).unwrap();
    let mut decoder = OutputDecoder::with_output(tc.plugin.output.clone());
    let mut events = Vec::new();
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
                decoder.stdout(&b, &mut events);
            }
            RunEvent::Stderr(b) => {
                out.stderr.push_str(&String::from_utf8_lossy(&b));
                decoder.stderr(&b, &mut events);
            }
            RunEvent::Exited { code, .. } => {
                decoder.finish(&mut events);
                out.code = code;
            }
            RunEvent::Cancelled { .. } => panic!("cancelled"),
            RunEvent::Failed { message, .. } => panic!("failed: {message}"),
            RunEvent::Started { .. } => {}
        }
    }
    out.stderr = out.stderr.replace("\r\n", "\n");
    // Merge adjacent text so chunking doesn't matter.
    for event in events {
        match (out.events.last_mut(), event) {
            (Some(UiEvent::Stdout { text: a }), UiEvent::Stdout { text: b })
            | (Some(UiEvent::Stderr { text: a }), UiEvent::Stderr { text: b }) => a.push_str(&b),
            (_, e) => out.events.push(e),
        }
    }
    for event in &mut out.events {
        if let UiEvent::Stdout { text } | UiEvent::Stderr { text } = event {
            *text = text.replace("\r\n", "\n");
        }
    }
    out
}

/// Compare `stdout` with the case file at `fixture` (relative to the crate),
/// or rewrite the file when `TINKERPAD_UPDATE_FIXTURES` is set.
pub fn check_fixture(fixture: &str, header: &str, stdout: &[u8]) {
    let captured = case_file(header, stdout);
    let path = manifest_dir().join(fixture);
    if std::env::var_os("TINKERPAD_UPDATE_FIXTURES").is_some() {
        std::fs::write(&path, &captured).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        captured == expected,
        "the dump library's output changed. If that's intended, regenerate {fixture} \
         with TINKERPAD_UPDATE_FIXTURES=1.\n--- captured ---\n{captured}"
    );
}

/// Turn raw stdout into a `dump_cases` file: the bytes as `in` lines, then the
/// expected text and rich events.
fn case_file(header: &str, stdout: &[u8]) -> String {
    let mut file = String::new();
    for line in header.lines() {
        file.push_str("# ");
        file.push_str(line);
        file.push('\n');
    }
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
