//! Turning a run's stdout and stderr bytes into the events the UI shows.
//!
//! Stdout goes through the [`DumpParser`] first, so dumped values come out as
//! `rich` events between the text around them. Text is decoded to UTF-8 only
//! here, at the UI boundary. Each completed line of either stream is offered to
//! the plugin's [`Annotator`], and a match becomes an `annotation` event right
//! after the text it was found in.

use tinkerpad_core::RunEvent;
use tinkerpad_plugin::OutputSpec;

use crate::annotate::Annotator;
use crate::dump::{DumpParser, Dumped};
use crate::utf8::Utf8Stream;

#[derive(Debug, Default)]
pub struct OutputDecoder {
    dump: DumpParser,
    stdout: Stream,
    stderr: Stream,
    annotator: Option<Annotator>,
}

/// One stream's UTF-8 decoder and the line it's part way through.
#[derive(Debug, Default)]
struct Stream {
    utf8: Utf8Stream,
    partial: String,
}

impl OutputDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// A decoder that also looks for positions with the plugin's `[output]`
    /// pattern.
    pub fn with_output(spec: Option<OutputSpec>) -> Self {
        Self {
            annotator: spec.map(Annotator::new),
            ..Self::default()
        }
    }

    /// A decoder whose dump payload cap isn't the default 8 MiB.
    pub fn with_payload_cap(cap: usize) -> Self {
        Self {
            dump: DumpParser::with_cap(cap),
            ..Self::default()
        }
    }

    /// Decode a stdout chunk, appending events to `out` in order.
    pub fn stdout(&mut self, chunk: &[u8], out: &mut Vec<RunEvent>) {
        let Self {
            dump,
            stdout,
            annotator,
            ..
        } = self;
        let mut text = String::with_capacity(chunk.len());
        dump.push(chunk, |piece| {
            handle(piece, stdout, annotator, &mut text, out);
        });
        flush(&mut text, stdout, annotator, false, out);
    }

    /// Decode a stderr chunk. Stderr is never scanned for dumps.
    pub fn stderr(&mut self, chunk: &[u8], out: &mut Vec<RunEvent>) {
        let Self {
            stderr, annotator, ..
        } = self;
        let mut text = stderr.utf8.push(chunk);
        flush(&mut text, stderr, annotator, true, out);
    }

    /// End of both streams: release anything held back.
    pub fn finish(&mut self, out: &mut Vec<RunEvent>) {
        let Self {
            dump,
            stdout,
            stderr,
            annotator,
        } = self;
        let mut text = String::new();
        dump.finish(|piece| handle(piece, stdout, annotator, &mut text, out));
        text.push_str(&stdout.utf8.finish());
        flush(&mut text, stdout, annotator, false, out);
        let mut text = stderr.utf8.finish();
        flush(&mut text, stderr, annotator, true, out);

        // A last line with no newline still counts.
        for stream in [&mut *stdout, &mut *stderr] {
            let line = std::mem::take(&mut stream.partial);
            if line.is_empty() {
                continue;
            }
            if let Some(annotation) = annotator.as_mut().and_then(|a| a.line(line.trim_end())) {
                out.push(RunEvent::Annotation(annotation));
            }
        }
        if let Some(annotator) = annotator {
            annotator.finish();
        }
    }
}

fn handle(
    piece: Dumped<'_>,
    stream: &mut Stream,
    annotator: &mut Option<Annotator>,
    text: &mut String,
    out: &mut Vec<RunEvent>,
) {
    match piece {
        Dumped::Text(bytes) => stream.utf8.push_into(bytes, text),
        Dumped::Rich(value) => {
            // A character can't continue across a sentinel, so anything held
            // back is already broken; release it before the value.
            text.push_str(&stream.utf8.finish());
            flush(text, stream, annotator, false, out);
            out.push(RunEvent::Rich { value });
        }
        Dumped::Malformed { payload, error } => {
            text.push_str(&stream.utf8.finish());
            flush(text, stream, annotator, false, out);
            out.push(RunEvent::Stderr {
                text: String::from_utf8_lossy(payload).into_owned(),
            });
            out.push(RunEvent::Warning {
                message: format!("A dump() value couldn't be read, so it's shown as text: {error}"),
            });
        }
    }
}

/// Send `text` on, with an annotation event after the text of any line the
/// pattern matched.
fn flush(
    text: &mut String,
    stream: &mut Stream,
    annotator: &mut Option<Annotator>,
    is_err: bool,
    out: &mut Vec<RunEvent>,
) {
    if text.is_empty() {
        return;
    }
    let event = |text: String| {
        if is_err {
            RunEvent::Stderr { text }
        } else {
            RunEvent::Stdout { text }
        }
    };
    let Some(annotator) = annotator.as_mut() else {
        out.push(event(std::mem::take(text)));
        return;
    };

    let mut batch = String::with_capacity(text.len());
    for piece in text.split_inclusive('\n') {
        batch.push_str(piece);
        stream.partial.push_str(piece);
        if !piece.ends_with('\n') {
            continue;
        }
        let line = std::mem::take(&mut stream.partial);
        if let Some(annotation) = annotator.line(line.trim_end_matches(['\n', '\r'])) {
            out.push(event(std::mem::take(&mut batch)));
            out.push(RunEvent::Annotation(annotation));
        }
    }
    text.clear();
    if !batch.is_empty() {
        out.push(event(batch));
    }
}

#[cfg(test)]
mod tests {
    use regex::Regex;
    use tinkerpad_core::dump::RichNode;

    use super::*;

    fn decode(chunks: &[&[u8]]) -> Vec<RunEvent> {
        let mut decoder = OutputDecoder::new();
        let mut out = Vec::new();
        for chunk in chunks {
            decoder.stdout(chunk, &mut out);
        }
        decoder.finish(&mut out);
        out
    }

    fn annotating() -> OutputDecoder {
        OutputDecoder::with_output(Some(OutputSpec {
            error_regex: Regex::new(r"^(?<file>.+?):(?<line>\d+): (?<message>.*)$").unwrap(),
            multiline: false,
        }))
    }

    #[test]
    fn text_value_text() {
        let events = decode(&[b"a \x1b]1337;TinkerPad={\"kind\":\"value\",\"value\":42}\x07 b\n"]);
        assert_eq!(events.len(), 3, "{events:?}");
        assert_eq!(events[0], RunEvent::Stdout { text: "a ".into() });
        assert!(
            matches!(&events[1], RunEvent::Rich { value } if matches!(value.node, RichNode::Value { .. }))
        );
        assert_eq!(
            events[2],
            RunEvent::Stdout {
                text: " b\n".into()
            }
        );
    }

    #[test]
    fn malformed_becomes_stderr_and_warning() {
        let events = decode(&[b"x\x1b]1337;TinkerPad={oops\x07y"]);
        assert_eq!(events[0], RunEvent::Stdout { text: "x".into() });
        assert_eq!(
            events[1],
            RunEvent::Stderr {
                text: "{oops".into()
            }
        );
        assert!(
            matches!(&events[2], RunEvent::Warning { message } if message.contains("couldn't be read"))
        );
        assert_eq!(events[3], RunEvent::Stdout { text: "y".into() });
        assert_eq!(events.len(), 4);
    }

    #[test]
    fn utf8_split_around_chunks_is_intact() {
        let text = "日本 🎉";
        let bytes = text.as_bytes();
        for split in 0..=bytes.len() {
            let events = decode(&[&bytes[..split], &bytes[split..]]);
            let joined: String = events
                .iter()
                .map(|e| match e {
                    RunEvent::Stdout { text } => text.as_str(),
                    other => panic!("{other:?}"),
                })
                .collect();
            assert_eq!(joined, text, "split at {split}");
        }
    }

    #[test]
    fn stderr_is_not_scanned() {
        let mut decoder = OutputDecoder::new();
        let mut out = Vec::new();
        decoder.stderr(
            b"\x1b]1337;TinkerPad={\"kind\":\"value\",\"value\":1}\x07",
            &mut out,
        );
        decoder.finish(&mut out);
        assert!(matches!(&out[..], [RunEvent::Stderr { .. }]), "{out:?}");
    }

    #[test]
    fn an_annotation_follows_the_text_it_was_found_in() {
        let mut decoder = annotating();
        let mut out = Vec::new();
        decoder.stdout(b"fine\nmain.ts:8: boom\nafter\n", &mut out);
        decoder.finish(&mut out);
        match &out[..] {
            [
                RunEvent::Stdout { text: first },
                RunEvent::Annotation(a),
                RunEvent::Stdout { text: rest },
            ] => {
                assert_eq!(first, "fine\nmain.ts:8: boom\n");
                assert_eq!(
                    (a.file.as_deref(), a.line, a.message.as_str()),
                    (Some("main.ts"), 8, "boom")
                );
                assert_eq!(a.text, "main.ts:8: boom");
                assert_eq!(rest, "after\n");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn lines_split_across_chunks_and_streams_are_still_matched() {
        let mut decoder = annotating();
        let mut out = Vec::new();
        decoder.stdout(b"main.ts:", &mut out);
        decoder.stderr(b"err.rs:2: from stderr\n", &mut out);
        decoder.stdout(b"9: late\n", &mut out);
        decoder.finish(&mut out);
        let annotations: Vec<_> = out
            .iter()
            .filter_map(|e| match e {
                RunEvent::Annotation(a) => Some((a.file.clone().unwrap_or_default(), a.line)),
                _ => None,
            })
            .collect();
        assert_eq!(
            annotations,
            [("err.rs".to_owned(), 2), ("main.ts".to_owned(), 9)]
        );
    }

    #[test]
    fn a_last_line_without_a_newline_still_annotates() {
        let mut decoder = annotating();
        let mut out = Vec::new();
        decoder.stdout(b"main.ts:3: no newline", &mut out);
        decoder.finish(&mut out);
        assert!(
            matches!(out.last(), Some(RunEvent::Annotation(a)) if a.line == 3),
            "{out:?}"
        );
    }
}
