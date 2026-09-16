//! `Content-Length` framing over a byte stream.
//!
//! This module knows nothing about JSON-RPC: it turns a stream of bytes into
//! message bodies and back. The debug adapter protocol frames its messages the
//! same way with a different body shape, so it reuses this as-is
//! (`docs/DEBUGGING.md` D4).
//!
//! A frame is headers, a blank line, then exactly `Content-Length` bytes:
//!
//! ```text
//! Content-Length: 27\r\n
//! Content-Type: application/vscode-jsonrpc; charset=utf-8\r\n
//! \r\n
//! {"jsonrpc":"2.0","id":1,...}
//! ```
//!
//! Real servers vary, so the decoder is forgiving about everything except the
//! length: header names are matched case-insensitively, headers may come in any
//! order, `Content-Type` may be missing, a lone `\n` is accepted as a line
//! ending, and a body may arrive split across any number of chunks.

use bytes::{Buf, BytesMut};

/// The one header that matters, lowercased for comparison.
const CONTENT_LENGTH: &[u8] = b"content-length";

/// Bodies larger than this are refused rather than buffered. Real messages are
/// kilobytes; rust-analyzer's largest are a few megabytes.
pub const MAX_BODY: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    #[error("no Content-Length header in {headers:?}")]
    NoContentLength { headers: String },
    #[error("Content-Length is not a length: {value:?}")]
    BadContentLength { value: String },
    #[error("message of {len} bytes is over the {MAX_BODY} byte limit")]
    TooLarge { len: usize },
    #[error("stream ended {got} bytes into a {expected} byte message")]
    Truncated { expected: usize, got: usize },
}

/// Frame `body` for sending: one `Content-Length` header, a blank line, then
/// the body unchanged.
pub fn encode(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 32);
    encode_into(&mut out, body);
    out
}

/// [`encode`] into an existing buffer, for a writer that reuses one.
pub fn encode_into(out: &mut Vec<u8>, body: &[u8]) {
    out.extend_from_slice(b"Content-Length: ");
    out.extend_from_slice(body.len().to_string().as_bytes());
    out.extend_from_slice(b"\r\n\r\n");
    out.extend_from_slice(body);
}

#[derive(Debug, PartialEq, Eq)]
enum State {
    /// Looking for the end of the header block.
    Headers,
    /// Waiting for this many body bytes.
    Body(usize),
}

/// Turns pushed bytes into message bodies.
///
/// Feed it whatever arrives with [`push`](Decoder::push), then call
/// [`take`](Decoder::take) until it returns `None`; one chunk may hold several
/// messages, or a fraction of one.
#[derive(Debug)]
pub struct Decoder {
    buf: BytesMut,
    state: State,
    /// Set once the stream is over, so a half-read body can be reported.
    ended: bool,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub fn new() -> Self {
        Self {
            buf: BytesMut::new(),
            state: State::Headers,
            ended: false,
        }
    }

    /// Add bytes as they arrive from the stream.
    pub fn push(&mut self, chunk: &[u8]) {
        self.buf.extend_from_slice(chunk);
    }

    /// Note that no more bytes are coming, so [`take`](Decoder::take) can
    /// report a message that was cut in half.
    pub fn end(&mut self) {
        self.ended = true;
    }

    /// Bytes held but not yet part of a message.
    pub fn pending(&self) -> usize {
        self.buf.len()
    }

    /// The next complete body, or `None` when more bytes are needed.
    ///
    /// A header block that makes no sense is reported and skipped: the decoder
    /// resyncs by looking for the next `Content-Length` in what is left, which
    /// is the only way back from a server that wrote something unframed (a
    /// panic message, say) into the stream.
    pub fn take(&mut self) -> Option<Result<Vec<u8>, FrameError>> {
        loop {
            match self.state {
                State::Body(len) => {
                    if self.buf.len() < len {
                        if self.ended && !self.buf.is_empty() {
                            let got = self.buf.len();
                            self.buf.clear();
                            self.state = State::Headers;
                            return Some(Err(FrameError::Truncated { expected: len, got }));
                        }
                        return None;
                    }
                    let body = self.buf.split_to(len).to_vec();
                    self.state = State::Headers;
                    return Some(Ok(body));
                }
                State::Headers => {
                    let (end, body_at) = find_header_end(&self.buf)?;
                    let headers = self.buf.split_to(body_at);
                    match content_length(&headers[..end]) {
                        Ok(len) if len > MAX_BODY => {
                            // Reading it would mean buffering it, so drop the
                            // frame and carry on with whatever follows.
                            return Some(Err(FrameError::TooLarge { len }));
                        }
                        Ok(len) => self.state = State::Body(len),
                        Err(e) => {
                            self.resync();
                            return Some(Err(e));
                        }
                    }
                }
            }
        }
    }

    /// Drop everything up to the next `Content-Length` header, if there is one.
    fn resync(&mut self) {
        match find_ignore_case(&self.buf, CONTENT_LENGTH) {
            Some(at) => self.buf.advance(at),
            None => self.buf.clear(),
        }
    }
}

/// `(length of the headers, offset of the body)`, once the blank line that ends
/// the header block has arrived.
fn find_header_end(buf: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0;
    while i < buf.len() {
        // A blank line is the end: `\r\n\r\n`, or `\n\n` from a lenient server.
        if buf[i..].starts_with(b"\r\n\r\n") {
            return Some((i, i + 4));
        }
        if buf[i..].starts_with(b"\n\n") {
            return Some((i, i + 2));
        }
        i += 1;
    }
    None
}

/// The `Content-Length` value from a header block.
fn content_length(headers: &[u8]) -> Result<usize, FrameError> {
    for line in headers.split(|&b| b == b'\n') {
        let line = trim(line);
        let Some(colon) = line.iter().position(|&b| b == b':') else {
            continue;
        };
        let (name, value) = line.split_at(colon);
        if !name.eq_ignore_ascii_case(CONTENT_LENGTH) {
            continue;
        }
        let value = String::from_utf8_lossy(trim(&value[1..])).into_owned();
        return value
            .parse::<usize>()
            .map_err(|_| FrameError::BadContentLength { value });
    }
    Err(FrameError::NoContentLength {
        headers: String::from_utf8_lossy(headers).into_owned(),
    })
}

fn trim(mut s: &[u8]) -> &[u8] {
    while let [first, rest @ ..] = s {
        if first.is_ascii_whitespace() {
            s = rest;
        } else {
            break;
        }
    }
    while let [rest @ .., last] = s {
        if last.is_ascii_whitespace() {
            s = rest;
        } else {
            break;
        }
    }
    s
}

/// Where `needle` next appears in `haystack`, ignoring ASCII case.
fn find_ignore_case(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=haystack.len() - needle.len())
        .find(|&i| haystack[i..i + needle.len()].eq_ignore_ascii_case(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bodies(decoder: &mut Decoder) -> Vec<Result<Vec<u8>, FrameError>> {
        let mut out = Vec::new();
        while let Some(item) = decoder.take() {
            out.push(item);
        }
        out
    }

    fn ok(decoder: &mut Decoder) -> Vec<String> {
        bodies(decoder)
            .into_iter()
            .map(|b| String::from_utf8(b.expect("a body")).unwrap())
            .collect()
    }

    #[test]
    fn round_trips_a_message() {
        let mut decoder = Decoder::new();
        decoder.push(&encode(b"hello"));
        assert_eq!(ok(&mut decoder), ["hello"]);
        assert_eq!(decoder.pending(), 0);
    }

    #[test]
    fn encodes_one_header_and_a_blank_line() {
        assert_eq!(encode(b"{}"), b"Content-Length: 2\r\n\r\n{}");
        assert_eq!(encode(b""), b"Content-Length: 0\r\n\r\n");
    }

    #[test]
    fn reads_headers_in_any_order_and_case() {
        let mut decoder = Decoder::new();
        decoder.push(
            b"Content-Type: application/vscode-jsonrpc; charset=utf-8\r\ncOnTeNt-LeNgTh:  4 \r\n\r\nbody",
        );
        assert_eq!(ok(&mut decoder), ["body"]);
    }

    #[test]
    fn accepts_bare_newlines() {
        let mut decoder = Decoder::new();
        decoder.push(b"Content-Length: 2\n\nhi");
        assert_eq!(ok(&mut decoder), ["hi"]);
    }

    #[test]
    fn reads_several_messages_from_one_chunk() {
        let mut decoder = Decoder::new();
        let mut chunk = encode(b"one");
        chunk.extend_from_slice(&encode(b"two"));
        chunk.extend_from_slice(&encode(b"three"));
        decoder.push(&chunk);
        assert_eq!(ok(&mut decoder), ["one", "two", "three"]);
    }

    #[test]
    fn reads_a_message_split_across_chunks() {
        let frame = encode(b"a longer body than one byte");
        for split in 1..frame.len() {
            let mut decoder = Decoder::new();
            decoder.push(&frame[..split]);
            let before = decoder.take();
            decoder.push(&frame[split..]);
            assert!(
                before.is_none(),
                "split at {split} produced a message early"
            );
            assert_eq!(
                ok(&mut decoder),
                ["a longer body than one byte"],
                "split at {split}"
            );
        }
    }

    #[test]
    fn a_zero_length_body_is_a_message() {
        let mut decoder = Decoder::new();
        decoder.push(&encode(b""));
        assert_eq!(ok(&mut decoder), [""]);
    }

    #[test]
    fn resyncs_after_a_header_block_with_no_length() {
        let mut decoder = Decoder::new();
        let mut chunk = b"thread 'main' panicked at src/main.rs:1:1\r\n\r\n".to_vec();
        chunk.extend_from_slice(&encode(b"after"));
        decoder.push(&chunk);
        let out = bodies(&mut decoder);
        assert!(
            matches!(out[0], Err(FrameError::NoContentLength { .. })),
            "{out:?}"
        );
        assert_eq!(String::from_utf8(out[1].clone().unwrap()).unwrap(), "after");
    }

    #[test]
    fn resyncs_after_a_length_that_is_not_a_number() {
        let mut decoder = Decoder::new();
        let mut chunk = b"Content-Length: banana\r\n\r\n".to_vec();
        chunk.extend_from_slice(&encode(b"after"));
        decoder.push(&chunk);
        let out = bodies(&mut decoder);
        assert_eq!(
            out[0],
            Err(FrameError::BadContentLength {
                value: "banana".into()
            })
        );
        assert_eq!(String::from_utf8(out[1].clone().unwrap()).unwrap(), "after");
    }

    #[test]
    fn refuses_a_body_too_large_to_buffer() {
        let mut decoder = Decoder::new();
        decoder.push(format!("Content-Length: {}\r\n\r\n", MAX_BODY + 1).as_bytes());
        assert_eq!(
            decoder.take(),
            Some(Err(FrameError::TooLarge { len: MAX_BODY + 1 }))
        );
    }

    #[test]
    fn a_body_cut_short_by_the_end_of_the_stream_is_an_error() {
        let mut decoder = Decoder::new();
        decoder.push(b"Content-Length: 10\r\n\r\nfour");
        assert_eq!(decoder.take(), None);
        decoder.end();
        assert_eq!(
            decoder.take(),
            Some(Err(FrameError::Truncated {
                expected: 10,
                got: 4
            }))
        );
        assert_eq!(decoder.take(), None);
    }

    #[test]
    fn a_clean_end_of_stream_is_not_an_error() {
        let mut decoder = Decoder::new();
        decoder.push(&encode(b"last"));
        decoder.end();
        assert_eq!(ok(&mut decoder), ["last"]);
    }

    #[test]
    fn carries_a_two_megabyte_body() {
        let body = vec![b'x'; 2 * 1024 * 1024];
        let frame = encode(&body);
        let mut decoder = Decoder::new();
        // In chunks, the way a pipe delivers it.
        for chunk in frame.chunks(8 * 1024) {
            decoder.push(chunk);
        }
        assert_eq!(decoder.take().unwrap().unwrap().len(), body.len());
    }
}
