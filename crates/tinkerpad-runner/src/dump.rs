//! Finding `dump()` values in a program's stdout.
//!
//! A program writes `ESC ] 1337 ; TinkerPad= <json> BEL` (or `ESC \` in place
//! of `BEL`) for each value. [`DumpParser`] is a byte-level state machine that
//! splits stdout into plain text and parsed [`RichValue`]s, in order, however
//! the bytes are chunked. It works on bytes, not text: output may be invalid
//! UTF-8 and must still pass through untouched.
//!
//! Nothing is ever dropped. A payload that isn't valid JSON comes back as
//! [`Dumped::Malformed`] with its bytes; a sentinel still open when the program
//! exits comes back as text; a payload over the size cap becomes a
//! [`RichNode::Truncated`] marker.

use std::sync::LazyLock;

use memchr::memmem;
use tinkerpad_core::dump::{RichNode, RichValue};

/// What starts a dump payload.
pub const PREFIX: &[u8] = b"\x1b]1337;TinkerPad=";
const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
/// The second byte of the `ST` terminator, `ESC \`.
const ST_END: u8 = b'\\';

/// Payloads larger than this are skipped and replaced with a marker.
pub const DEFAULT_PAYLOAD_CAP: usize = 8 * 1024 * 1024;

/// Payload buffers up to this size are kept for reuse between dumps.
const REUSE_CAPACITY: usize = 64 * 1024;

/// One piece of parsed stdout. `Text` and `Malformed` borrow from the input or
/// the parser, so plain output is never copied.
#[derive(Debug, PartialEq)]
pub enum Dumped<'a> {
    /// Plain output bytes. One stretch of text may arrive in several pieces.
    Text(&'a [u8]),
    /// A complete dump value, or a [`RichNode::Truncated`] marker.
    Rich(RichValue),
    /// A terminated payload that didn't parse. `payload` excludes the prefix
    /// and terminator.
    Malformed { payload: &'a [u8], error: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Plain text.
    Text,
    /// Matched this many bytes of [`PREFIX`], at least one.
    Prefix(usize),
    /// Inside a payload. `esc`: the previous byte was an ESC, not yet known to
    /// be part of an `ESC \` terminator.
    Payload { esc: bool },
}

#[derive(Debug)]
pub struct DumpParser {
    state: State,
    payload: Vec<u8>,
    /// Set once the current payload passes the cap: its size so far. The bytes
    /// themselves are no longer kept.
    skipped: Option<u64>,
    cap: usize,
}

impl Default for DumpParser {
    fn default() -> Self {
        Self::new()
    }
}

impl DumpParser {
    pub fn new() -> Self {
        Self::with_cap(DEFAULT_PAYLOAD_CAP)
    }

    /// A parser with a payload cap other than 8 MiB.
    pub fn with_cap(cap: usize) -> Self {
        Self {
            state: State::Text,
            payload: Vec::new(),
            skipped: None,
            cap,
        }
    }

    /// Parse the next chunk, calling `emit` for each piece in order. A
    /// sentinel cut off at the end of the chunk is held until the next one.
    pub fn push(&mut self, chunk: &[u8], mut emit: impl FnMut(Dumped<'_>)) {
        let mut i = 0;
        while i < chunk.len() {
            match self.state {
                State::Text => {
                    let rest = &chunk[i..];
                    let Some((at, matched)) = find_prefix(rest) else {
                        emit(Dumped::Text(rest));
                        return;
                    };
                    if at > 0 {
                        emit(Dumped::Text(&rest[..at]));
                    }
                    self.state = if matched == PREFIX.len() {
                        State::Payload { esc: false }
                    } else {
                        State::Prefix(matched)
                    };
                    i += at + matched;
                }
                State::Prefix(n) => {
                    if chunk[i] == PREFIX[n] {
                        i += 1;
                        self.state = if n + 1 == PREFIX.len() {
                            State::Payload { esc: false }
                        } else {
                            State::Prefix(n + 1)
                        };
                    } else {
                        // Not a sentinel after all. The matched bytes are text,
                        // and this byte is looked at again as text (it may be
                        // the ESC of a real sentinel).
                        emit(Dumped::Text(&PREFIX[..n]));
                        self.state = State::Text;
                    }
                }
                State::Payload { esc: true } => {
                    self.state = State::Payload { esc: false };
                    if chunk[i] == ST_END {
                        i += 1;
                        self.complete(&mut emit);
                    } else {
                        // A lone ESC inside the payload. JSON can't contain a
                        // raw one, so this payload will be reported as
                        // malformed, but keep the byte so nothing is lost.
                        self.store(&[ESC]);
                    }
                }
                State::Payload { esc: false } => {
                    let rest = &chunk[i..];
                    let Some(at) = memchr::memchr2(BEL, ESC, rest) else {
                        self.store(rest);
                        return;
                    };
                    self.store(&rest[..at]);
                    i += at + 1;
                    if rest[at] == BEL {
                        self.complete(&mut emit);
                    } else {
                        self.state = State::Payload { esc: true };
                    }
                }
            }
        }
    }

    /// End of stream. Anything held back is released: a partial prefix or an
    /// unterminated payload as text, an oversized one as a truncation marker.
    pub fn finish(&mut self, mut emit: impl FnMut(Dumped<'_>)) {
        match std::mem::replace(&mut self.state, State::Text) {
            State::Text => {}
            State::Prefix(n) => emit(Dumped::Text(&PREFIX[..n])),
            State::Payload { esc } => {
                if let Some(bytes) = self.skipped.take() {
                    emit(Dumped::Rich(truncated(bytes + u64::from(esc))));
                } else {
                    emit(Dumped::Text(PREFIX));
                    if !self.payload.is_empty() {
                        emit(Dumped::Text(&self.payload));
                    }
                    if esc {
                        emit(Dumped::Text(&[ESC]));
                    }
                    self.payload.clear();
                }
            }
        }
    }

    /// Append payload bytes, or just count them once over the cap.
    fn store(&mut self, bytes: &[u8]) {
        if let Some(skipped) = &mut self.skipped {
            *skipped += bytes.len() as u64;
        } else if self.payload.len() + bytes.len() > self.cap {
            self.skipped = Some((self.payload.len() + bytes.len()) as u64);
            // Free the buffer: the point of the cap is bounded memory.
            self.payload = Vec::new();
        } else {
            self.payload.extend_from_slice(bytes);
        }
    }

    /// A terminator arrived: emit the value, marker, or malformed payload.
    fn complete(&mut self, emit: &mut impl FnMut(Dumped<'_>)) {
        self.state = State::Text;
        if let Some(bytes) = self.skipped.take() {
            emit(Dumped::Rich(truncated(bytes)));
            return;
        }
        match RichValue::from_json(&self.payload) {
            Ok(value) => emit(Dumped::Rich(value)),
            Err(e) => emit(Dumped::Malformed {
                payload: &self.payload,
                error: e.to_string(),
            }),
        }
        if self.payload.capacity() > REUSE_CAPACITY {
            self.payload = Vec::new();
        } else {
            self.payload.clear();
        }
    }
}

/// Find where a sentinel starts in `text`: the offset of its ESC and how many
/// prefix bytes follow it in `text` (all of them, or fewer when `text` ends
/// first). ESC bytes that start something else, like ANSI colour codes, are
/// skipped here, so ordinary output stays one piece.
fn find_prefix(text: &[u8]) -> Option<(usize, usize)> {
    static FINDER: LazyLock<memmem::Finder<'static>> =
        LazyLock::new(|| memmem::Finder::new(PREFIX));
    if let Some(at) = FINDER.find(text) {
        return Some((at, PREFIX.len()));
    }
    // No whole prefix; the chunk may still end with the start of one.
    let tail = text.len().saturating_sub(PREFIX.len() - 1);
    (tail..text.len())
        .find(|&at| PREFIX.starts_with(&text[at..]))
        .map(|at| (at, text.len() - at))
}

fn truncated(bytes: u64) -> RichValue {
    RichValue::new(RichNode::Truncated { bytes })
}
