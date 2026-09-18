//! JSON lines over a child's stdio.
//!
//! One JSON object per line, `\n` terminated, `\r\n` tolerated because a
//! helper written in a language whose `println` ends lines the Windows way
//! shouldn't have to care. A line that isn't a message is skipped, not fatal:
//! helpers print banners, warnings, and progress to stdout, and the first
//! thing many runtimes do is announce themselves.
//!
//! Chosen over LSP's `Content-Length` framing deliberately. A helper is often
//! a script, and `print(json.dumps(msg))` is a one-liner in every language a
//! plugin might be written in; counting bytes is not.

use tracing::warn;

use super::protocol::{FromHelper, ToHelper};

/// How much of an unparseable line is logged. Enough to recognise a stack
/// trace or a banner, short enough that a megabyte of base64 doesn't reach
/// the log.
pub const LOGGED_BYTES: usize = 200;

/// The longest line the decoder will assemble, 16 MiB. A run that dumps a
/// large object sends it in chunks, so a line past this is a helper that has
/// lost its mind, and buffering it would take the app with it.
pub const MAX_LINE: usize = 16 * 1024 * 1024;

/// A line that arrived but wasn't a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Garbled {
    /// The start of the line, lossily decoded and shortened.
    pub start: String,
    /// The whole line's length in bytes.
    pub bytes: usize,
    /// Why it was skipped.
    pub reason: Reason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// Not JSON, or JSON that isn't a message this version knows the shape of.
    NotAMessage,
    /// Longer than [`MAX_LINE`]; dropped without being parsed.
    TooLong,
}

impl std::fmt::Display for Garbled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.reason {
            Reason::NotAMessage => write!(f, "{} bytes, starting {:?}", self.bytes, self.start),
            Reason::TooLong => write!(f, "{} bytes, too long to read", self.bytes),
        }
    }
}

/// Bytes in, messages out.
///
/// Feed it whatever chunks arrive; it keeps the half line between calls.
#[derive(Debug, Default)]
pub struct Decoder {
    partial: Vec<u8>,
    /// Set when the current line went past [`MAX_LINE`]: the rest of it is
    /// dropped as it arrives, and the next newline starts fresh.
    dropping: bool,
    /// Length of the line being dropped, for the report.
    dropped: usize,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decode everything `bytes` completes. Lines that weren't messages come
    /// back as `Err`, in the order they arrived, so the caller can log them
    /// the way that suits where it is (banner noise before `ready`, a warning
    /// after).
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Result<FromHelper, Garbled>> {
        let mut out = Vec::new();
        for &byte in bytes {
            if byte == b'\n' {
                if let Some(line) = self.finish_line() {
                    out.push(line);
                }
            } else if self.dropping {
                self.dropped += 1;
            } else if self.partial.len() >= MAX_LINE {
                self.dropping = true;
                self.dropped = self.partial.len() + 1;
                self.partial.clear();
                self.partial.shrink_to_fit();
            } else {
                self.partial.push(byte);
            }
        }
        out
    }

    /// The last line, when the helper's stdout ended without a newline.
    pub fn end(&mut self) -> Option<Result<FromHelper, Garbled>> {
        self.finish_line()
    }

    fn finish_line(&mut self) -> Option<Result<FromHelper, Garbled>> {
        if self.dropping {
            let bytes = self.dropped;
            self.dropping = false;
            self.dropped = 0;
            return Some(Err(Garbled {
                start: String::new(),
                bytes,
                reason: Reason::TooLong,
            }));
        }
        let mut line = std::mem::take(&mut self.partial);
        // A helper whose runtime writes CRLF is still speaking the protocol.
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        if line.iter().all(|b| b.is_ascii_whitespace()) {
            return None;
        }
        Some(parse(&line))
    }
}

/// One line to one message.
pub fn parse(line: &[u8]) -> Result<FromHelper, Garbled> {
    serde_json::from_slice(line).map_err(|_| Garbled {
        start: shorten(line),
        bytes: line.len(),
        reason: Reason::NotAMessage,
    })
}

/// The first [`LOGGED_BYTES`] of a line, readable enough to recognise.
fn shorten(line: &[u8]) -> String {
    let end = line.len().min(LOGGED_BYTES);
    // Don't split a character in half on the way to the log.
    let end = (0..=end).rev().find(|&n| line.is_char_boundary_at(n));
    let end = end.unwrap_or(0);
    String::from_utf8_lossy(&line[..end]).into_owned()
}

/// `str::is_char_boundary` for bytes that may not be UTF-8 at all.
trait CharBoundary {
    fn is_char_boundary_at(&self, index: usize) -> bool;
}

impl CharBoundary for [u8] {
    fn is_char_boundary_at(&self, index: usize) -> bool {
        match self.get(index) {
            None => index == self.len(),
            Some(&byte) => (byte as i8) >= -0x40,
        }
    }
}

/// Serialise a message and its newline into `buf`.
pub fn encode_into(buf: &mut Vec<u8>, message: &ToHelper) {
    match serde_json::to_writer(&mut *buf, message) {
        Ok(()) => buf.push(b'\n'),
        Err(e) => {
            // Only reachable if a message holds something JSON can't express.
            // Dropping it beats writing half a line and desynchronising the
            // helper for good.
            warn!(error = %e, "a message could not be encoded and was dropped");
            buf.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn messages(decoder: &mut Decoder, bytes: &str) -> Vec<Result<FromHelper, Garbled>> {
        decoder.push(bytes.as_bytes())
    }

    #[test]
    fn one_object_per_line_with_either_line_ending() {
        let mut decoder = Decoder::new();
        let out = messages(
            &mut decoder,
            "{\"type\":\"ready\",\"name\":\"a\"}\r\n{\"type\":\"stdout\",\"id\":\"r1\",\"data\":\"aGk=\"}\n",
        );
        assert_eq!(out.len(), 2);
        assert!(matches!(out[0], Ok(FromHelper::Ready { .. })));
        assert!(matches!(out[1], Ok(FromHelper::Stdout { .. })));
    }

    #[test]
    fn a_message_split_across_chunks_is_assembled() {
        let mut decoder = Decoder::new();
        assert!(messages(&mut decoder, "{\"type\":\"exi").is_empty());
        assert!(messages(&mut decoder, "ted\",\"id\":\"r1\",\"co").is_empty());
        let out = messages(&mut decoder, "de\":0}\n");
        assert!(
            matches!(&out[..], [Ok(FromHelper::Exited { id, code, .. })] if id == "r1" && *code == Some(0)),
            "{out:?}",
        );
    }

    #[test]
    fn a_banner_is_skipped_and_the_next_message_still_arrives() {
        let mut decoder = Decoder::new();
        let out = messages(
            &mut decoder,
            "Python 3.12.1 starting\n\n{\"type\":\"ready\"}\n",
        );
        assert_eq!(out.len(), 2, "the blank line is not reported: {out:?}");
        let Err(garbled) = &out[0] else {
            panic!("{out:?}")
        };
        assert_eq!(garbled.reason, Reason::NotAMessage);
        assert_eq!(garbled.start, "Python 3.12.1 starting");
        assert!(matches!(out[1], Ok(FromHelper::Ready { .. })));
    }

    #[test]
    fn only_the_first_two_hundred_bytes_of_a_bad_line_are_kept() {
        let mut decoder = Decoder::new();
        let long = "x".repeat(5_000);
        let out = messages(&mut decoder, &format!("{long}\n"));
        let Err(garbled) = &out[0] else {
            panic!("{out:?}")
        };
        assert_eq!(garbled.start.len(), LOGGED_BYTES);
        assert_eq!(garbled.bytes, 5_000);
    }

    #[test]
    fn a_bad_line_is_not_cut_through_a_character() {
        let mut decoder = Decoder::new();
        // The 200th byte lands in the middle of one of these.
        let out = messages(&mut decoder, &format!("{}\n", "é".repeat(200)));
        let Err(garbled) = &out[0] else {
            panic!("{out:?}")
        };
        assert!(!garbled.start.contains('\u{fffd}'), "{:?}", garbled.start);
        assert_eq!(garbled.start.len(), LOGGED_BYTES);
    }

    #[test]
    fn a_line_past_the_cap_is_dropped_without_buffering_it_all() {
        let mut decoder = Decoder::new();
        let chunk = "x".repeat(1024 * 1024);
        for _ in 0..20 {
            assert!(decoder.push(chunk.as_bytes()).is_empty());
            assert!(decoder.partial.len() <= MAX_LINE, "the line is not kept");
        }
        let out = decoder.push(b"\n{\"type\":\"ready\"}\n");
        assert_eq!(out.len(), 2);
        let Err(garbled) = &out[0] else {
            panic!("{out:?}")
        };
        assert_eq!(garbled.reason, Reason::TooLong);
        assert!(garbled.bytes > MAX_LINE);
        assert!(
            matches!(out[1], Ok(FromHelper::Ready { .. })),
            "the next line is read normally: {out:?}",
        );
    }

    #[test]
    fn json_that_is_not_a_message_is_garbled_not_unknown() {
        let mut decoder = Decoder::new();
        let out = messages(&mut decoder, "{\"no\":\"type\"}\n[1,2,3]\n");
        assert!(out.iter().all(|m| m.is_err()), "{out:?}");
    }

    #[test]
    fn a_message_type_from_the_future_is_unknown_not_an_error() {
        let mut decoder = Decoder::new();
        let out = messages(&mut decoder, "{\"type\":\"telemetry\",\"n\":1}\n");
        assert!(matches!(out[..], [Ok(FromHelper::Unknown)]), "{out:?}");
    }

    #[test]
    fn unknown_fields_in_a_known_message_are_ignored() {
        let out = parse(
            br#"{"type":"exited","id":"r1","code":0,"duration_ms":7,"rss_kb":900,"debug":{"x":1}}"#,
        )
        .unwrap();
        assert!(
            matches!(
                out,
                FromHelper::Exited {
                    code: Some(0),
                    duration_ms: Some(7),
                    ..
                }
            ),
            "{out:?}",
        );
    }

    #[test]
    fn a_line_without_a_newline_at_the_end_still_counts() {
        let mut decoder = Decoder::new();
        assert!(messages(&mut decoder, "{\"type\":\"ready\"}").is_empty());
        assert!(matches!(decoder.end(), Some(Ok(FromHelper::Ready { .. }))));
        assert!(decoder.end().is_none(), "nothing is left over");
    }

    #[test]
    fn encoding_a_message_ends_it_with_one_newline() {
        let mut buf = Vec::new();
        encode_into(
            &mut buf,
            &ToHelper::Cancel {
                id: "r1".to_owned(),
            },
        );
        assert_eq!(buf, b"{\"type\":\"cancel\",\"id\":\"r1\"}\n");
    }
}
