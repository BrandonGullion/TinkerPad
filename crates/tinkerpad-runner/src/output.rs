//! Turning a run's stdout and stderr bytes into the events the UI shows.
//!
//! Stdout goes through the [`DumpParser`] first, so dumped values come out as
//! `rich` events between the text around them. Text is decoded to UTF-8 only
//! here, at the UI boundary.

use tinkerpad_core::RunEvent;

use crate::dump::{DumpParser, Dumped};
use crate::utf8::Utf8Stream;

#[derive(Debug, Default)]
pub struct OutputDecoder {
    dump: DumpParser,
    stdout: Utf8Stream,
    stderr: Utf8Stream,
}

impl OutputDecoder {
    pub fn new() -> Self {
        Self::default()
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
        let Self { dump, stdout, .. } = self;
        let mut text = String::with_capacity(chunk.len());
        dump.push(chunk, |piece| handle(piece, stdout, &mut text, out));
        flush_stdout(&mut text, out);
    }

    /// Decode a stderr chunk. Stderr is never scanned for dumps.
    pub fn stderr(&mut self, chunk: &[u8], out: &mut Vec<RunEvent>) {
        let text = self.stderr.push(chunk);
        if !text.is_empty() {
            out.push(RunEvent::Stderr { text });
        }
    }

    /// End of both streams: release anything held back.
    pub fn finish(&mut self, out: &mut Vec<RunEvent>) {
        let Self {
            dump,
            stdout,
            stderr,
        } = self;
        let mut text = String::new();
        dump.finish(|piece| handle(piece, stdout, &mut text, out));
        text.push_str(&stdout.finish());
        flush_stdout(&mut text, out);
        let text = stderr.finish();
        if !text.is_empty() {
            out.push(RunEvent::Stderr { text });
        }
    }
}

fn handle(piece: Dumped<'_>, utf8: &mut Utf8Stream, text: &mut String, out: &mut Vec<RunEvent>) {
    match piece {
        Dumped::Text(bytes) => utf8.push_into(bytes, text),
        Dumped::Rich(value) => {
            // A character can't continue across a sentinel, so anything held
            // back is already broken; release it before the value.
            text.push_str(&utf8.finish());
            flush_stdout(text, out);
            out.push(RunEvent::Rich { value });
        }
        Dumped::Malformed { payload, error } => {
            text.push_str(&utf8.finish());
            flush_stdout(text, out);
            out.push(RunEvent::Stderr {
                text: String::from_utf8_lossy(payload).into_owned(),
            });
            out.push(RunEvent::Warning {
                message: format!("A dump() value couldn't be read, so it's shown as text: {error}"),
            });
        }
    }
}

fn flush_stdout(text: &mut String, out: &mut Vec<RunEvent>) {
    if !text.is_empty() {
        out.push(RunEvent::Stdout {
            text: std::mem::take(text),
        });
    }
}

#[cfg(test)]
mod tests {
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
}
