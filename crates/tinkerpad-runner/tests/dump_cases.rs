//! Table-driven tests for the dump parser, read from `tests/dump_cases/*.txt`.
//!
//! Each case file is a list of directives, one per line:
//!
//! ```text
//! # a comment
//! cap 16                  payload cap in bytes (default 8 MiB)
//! in <escaped bytes>      appended to the input; repeat for more
//! split 3,17              extra chunk split points; repeat for more sets
//! out text <escaped>      expected events, in order
//! out rich <json>
//! out malformed <escaped payload>
//! out truncated <bytes>
//! ```
//!
//! Escapes: `\e` ESC, `\a` BEL, `\n`, `\r`, `\t`, `\\`, `\s` space (for
//! trailing spaces), `\xHH`. Adjacent text is merged before comparing, since
//! chunking decides where text is cut.
//!
//! Every case runs as one chunk, one byte at a time, split in two at every
//! position, and at each `split` set.

use std::path::{Path, PathBuf};

use tinkerpad_core::dump::{RichNode, RichValue};
use tinkerpad_runner::dump::{DEFAULT_PAYLOAD_CAP, DumpParser, Dumped};

#[derive(Debug, PartialEq)]
enum Event {
    Text(Vec<u8>),
    Rich(RichValue),
    Malformed(Vec<u8>),
}

impl std::fmt::Display for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Event::Text(t) => write!(f, "text {:?}", String::from_utf8_lossy(t)),
            Event::Rich(v) => write!(f, "rich {}", serde_json::to_string(v).unwrap()),
            Event::Malformed(p) => write!(f, "malformed {:?}", String::from_utf8_lossy(p)),
        }
    }
}

struct Case {
    name: String,
    cap: usize,
    input: Vec<u8>,
    splits: Vec<Vec<usize>>,
    expected: Vec<Event>,
}

fn unescape(s: &str, at: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        let esc = *bytes
            .get(i + 1)
            .unwrap_or_else(|| panic!("{at}: trailing backslash"));
        i += 2;
        out.push(match esc {
            b'e' => 0x1b,
            b'a' => 0x07,
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            b's' => b' ',
            b'\\' => b'\\',
            b'x' => {
                let hex = s.get(i..i + 2).unwrap_or_else(|| panic!("{at}: short \\x"));
                i += 2;
                u8::from_str_radix(hex, 16).unwrap_or_else(|_| panic!("{at}: bad \\x{hex}"))
            }
            other => panic!("{at}: unknown escape \\{}", other as char),
        });
    }
    out
}

fn load(path: &Path) -> Case {
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let source = std::fs::read_to_string(path).unwrap();
    let mut case = Case {
        name: name.clone(),
        cap: DEFAULT_PAYLOAD_CAP,
        input: Vec::new(),
        splits: Vec::new(),
        expected: Vec::new(),
    };
    for (n, line) in source.lines().enumerate() {
        let at = format!("{name}:{}", n + 1);
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let (directive, rest) = line.split_once(' ').unwrap_or((line, ""));
        match directive {
            "cap" => {
                case.cap = rest
                    .trim()
                    .parse()
                    .unwrap_or_else(|_| panic!("{at}: bad cap"))
            }
            "in" => case.input.extend(unescape(rest, &at)),
            "split" => case.splits.push(
                rest.split(',')
                    .map(|p| {
                        p.trim()
                            .parse()
                            .unwrap_or_else(|_| panic!("{at}: bad split"))
                    })
                    .collect(),
            ),
            "out" => {
                let (kind, value) = rest.split_once(' ').unwrap_or((rest, ""));
                case.expected.push(match kind {
                    "text" => Event::Text(unescape(value, &at)),
                    "rich" => {
                        Event::Rich(RichValue::from_json(value.as_bytes()).unwrap_or_else(|e| {
                            panic!("{at}: expected rich value doesn't parse: {e}")
                        }))
                    }
                    "malformed" => Event::Malformed(unescape(value, &at)),
                    "truncated" => Event::Rich(RichValue::new(RichNode::Truncated {
                        bytes: value
                            .trim()
                            .parse()
                            .unwrap_or_else(|_| panic!("{at}: bad byte count")),
                    })),
                    other => panic!("{at}: unknown output kind {other}"),
                });
            }
            other => panic!("{at}: unknown directive {other}"),
        }
    }
    case.expected = merge(std::mem::take(&mut case.expected));
    case
}

fn merge(events: Vec<Event>) -> Vec<Event> {
    let mut out: Vec<Event> = Vec::new();
    for event in events {
        match (out.last_mut(), event) {
            (Some(Event::Text(prev)), Event::Text(more)) => prev.extend(more),
            (_, Event::Text(t)) if t.is_empty() => {}
            (_, event) => out.push(event),
        }
    }
    out
}

fn parse(cap: usize, chunks: &[&[u8]]) -> Vec<Event> {
    let mut parser = DumpParser::with_cap(cap);
    let mut events = Vec::new();
    let mut record = |piece: Dumped<'_>| {
        events.push(match piece {
            Dumped::Text(t) => {
                assert!(!t.is_empty(), "empty text piece");
                Event::Text(t.to_vec())
            }
            Dumped::Rich(v) => Event::Rich(v),
            Dumped::Malformed { payload, error } => {
                assert!(!error.is_empty());
                Event::Malformed(payload.to_vec())
            }
        })
    };
    for chunk in chunks {
        parser.push(chunk, &mut record);
    }
    parser.finish(&mut record);
    merge(events)
}

fn chunks_at<'a>(input: &'a [u8], points: &[usize]) -> Vec<&'a [u8]> {
    let mut chunks = Vec::new();
    let mut start = 0;
    for &p in points {
        let p = p.clamp(start, input.len());
        chunks.push(&input[start..p]);
        start = p;
    }
    chunks.push(&input[start..]);
    chunks
}

fn check(case: &Case, how: &str, chunks: &[&[u8]]) -> Result<(), String> {
    let actual = parse(case.cap, chunks);
    if actual == case.expected {
        return Ok(());
    }
    let list = |events: &[Event]| {
        events
            .iter()
            .map(|e| format!("    {e}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    Err(format!(
        "{} ({how}):\n  expected:\n{}\n  actual:\n{}",
        case.name,
        list(&case.expected),
        list(&actual)
    ))
}

fn case_files() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/dump_cases");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "txt"))
        .collect();
    files.sort();
    files
}

#[test]
fn dump_cases() {
    let files = case_files();
    assert!(
        files.len() >= 10,
        "expected the dump case set, found {}",
        files.len()
    );
    let mut failures = Vec::new();
    for path in files {
        let case = load(&path);
        assert!(!case.input.is_empty(), "{}: no input", case.name);
        let input = &case.input[..];

        let mut runs: Vec<(String, Vec<&[u8]>)> = vec![
            ("one chunk".into(), vec![input]),
            ("byte at a time".into(), input.chunks(1).collect()),
        ];
        for at in 0..=input.len() {
            runs.push((format!("split at {at}"), chunks_at(input, &[at])));
        }
        for points in &case.splits {
            runs.push((format!("split at {points:?}"), chunks_at(input, points)));
        }
        for (how, chunks) in runs {
            if let Err(e) = check(&case, &how, &chunks) {
                failures.push(e);
                break;
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n\n"));
}

/// The real default cap, which the case files can't reach cheaply.
#[test]
fn default_cap_is_8_mib() {
    assert_eq!(DEFAULT_PAYLOAD_CAP, 8 * 1024 * 1024);
    let wrap = |len: usize| {
        // {"kind":"text","text":"aaa…"} sized to exactly `len` bytes.
        let overhead = r#"{"kind":"text","text":""}"#.len();
        let mut payload = br#"{"kind":"text","text":""#.to_vec();
        payload.resize(len - 2, b'a');
        payload.extend_from_slice(br#""}"#);
        assert_eq!(payload.len(), len);
        assert!(len > overhead);
        let mut input = b"before".to_vec();
        input.extend_from_slice(tinkerpad_runner::dump::PREFIX);
        input.extend_from_slice(&payload);
        input.extend_from_slice(b"\x07after");
        input
    };

    let at_cap = wrap(DEFAULT_PAYLOAD_CAP);
    let events = parse(
        DEFAULT_PAYLOAD_CAP,
        &at_cap.chunks(64 * 1024).collect::<Vec<_>>(),
    );
    assert!(
        matches!(&events[..], [Event::Text(_), Event::Rich(RichValue { node: RichNode::Text { text, .. }, .. }), Event::Text(_)] if text.len() == DEFAULT_PAYLOAD_CAP - 25),
        "at the cap the value is kept"
    );

    let over = wrap(DEFAULT_PAYLOAD_CAP + 1);
    let events = parse(
        DEFAULT_PAYLOAD_CAP,
        &over.chunks(64 * 1024).collect::<Vec<_>>(),
    );
    assert_eq!(
        events,
        vec![
            Event::Text(b"before".to_vec()),
            Event::Rich(RichValue::new(RichNode::Truncated {
                bytes: DEFAULT_PAYLOAD_CAP as u64 + 1
            })),
            Event::Text(b"after".to_vec()),
        ]
    );
}
