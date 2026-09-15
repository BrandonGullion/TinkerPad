//! Decoding a byte stream to text without mangling characters split across
//! chunks.

/// Incremental UTF-8 decoder. Feed it chunks in order; it returns the text
/// that is complete so far and holds back a trailing partial character until
/// the next chunk arrives. Invalid bytes become U+FFFD.
#[derive(Debug, Default)]
pub struct Utf8Stream {
    pending: Vec<u8>,
}

impl Utf8Stream {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decode the next chunk.
    pub fn push(&mut self, chunk: &[u8]) -> String {
        let mut out = String::with_capacity(chunk.len());
        self.push_into(chunk, &mut out);
        out
    }

    /// Decode the next chunk, appending to `out`. The chunk isn't copied
    /// except for a character split across chunks.
    pub fn push_into(&mut self, chunk: &[u8], out: &mut String) {
        let mut rest = chunk;
        // Finish a character held back from the previous chunk, one byte at
        // a time (at most three more).
        while !self.pending.is_empty() && !rest.is_empty() {
            self.pending.push(rest[0]);
            rest = &rest[1..];
            match std::str::from_utf8(&self.pending) {
                Ok(s) => {
                    out.push_str(s);
                    self.pending.clear();
                }
                Err(e) if e.error_len().is_some() => {
                    let pending = std::mem::take(&mut self.pending);
                    self.pending = decode(&pending, out).to_vec();
                }
                Err(_) => {}
            }
        }
        if self.pending.is_empty() {
            self.pending.extend_from_slice(decode(rest, out));
        }
    }

    /// Flush at end of stream. Any held-back partial character becomes U+FFFD.
    pub fn finish(&mut self) -> String {
        if self.pending.is_empty() {
            String::new()
        } else {
            self.pending.clear();
            "\u{FFFD}".to_owned()
        }
    }
}

/// Decode as much of `bytes` as is complete into `out`, replacing invalid
/// sequences with U+FFFD. Returns the incomplete character at the end, if any.
fn decode<'a>(mut bytes: &'a [u8], out: &mut String) -> &'a [u8] {
    loop {
        match std::str::from_utf8(bytes) {
            Ok(s) => {
                out.push_str(s);
                return &[];
            }
            Err(e) => {
                let valid = e.valid_up_to();
                out.push_str(std::str::from_utf8(&bytes[..valid]).expect("valid prefix"));
                match e.error_len() {
                    Some(len) => {
                        out.push('\u{FFFD}');
                        bytes = &bytes[valid + len..];
                    }
                    None => return &bytes[valid..],
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_byte_after_a_held_back_start() {
        let mut s = Utf8Stream::new();
        assert_eq!(s.push(b"a\xE6"), "a");
        assert_eq!(s.push(b"\xE6\x97\xA5b"), "\u{FFFD}\u{65E5}b");
        assert_eq!(s.finish(), "");
    }

    #[test]
    fn split_character_then_more_text() {
        let mut s = Utf8Stream::new();
        let mut out = String::new();
        s.push_into(b"x\xF0\x9F", &mut out);
        s.push_into(b"\x8E", &mut out);
        s.push_into(b"\x89 done", &mut out);
        assert_eq!(out, "x\u{1F389} done");
    }

    #[test]
    fn ascii_passes_through() {
        let mut s = Utf8Stream::new();
        assert_eq!(s.push(b"hello "), "hello ");
        assert_eq!(s.push(b"world"), "world");
        assert_eq!(s.finish(), "");
    }

    #[test]
    fn multibyte_split_at_every_position() {
        let text = "héllo wörld → 日本 🎉";
        let bytes = text.as_bytes();
        for split in 0..=bytes.len() {
            let mut s = Utf8Stream::new();
            let mut out = s.push(&bytes[..split]);
            out += &s.push(&bytes[split..]);
            out += &s.finish();
            assert_eq!(out, text, "split at {split}");
        }
    }

    #[test]
    fn one_byte_at_a_time() {
        let text = "🎉 ok 日本";
        let mut s = Utf8Stream::new();
        let mut out = String::new();
        for b in text.as_bytes() {
            out += &s.push(std::slice::from_ref(b));
        }
        out += &s.finish();
        assert_eq!(out, text);
    }

    #[test]
    fn invalid_bytes_are_replaced_not_dropped() {
        let mut s = Utf8Stream::new();
        assert_eq!(s.push(b"a\xFFb"), "a\u{FFFD}b");
    }

    #[test]
    fn truncated_character_at_end_of_stream() {
        let mut s = Utf8Stream::new();
        assert_eq!(s.push(&"日".as_bytes()[..2]), "");
        assert_eq!(s.finish(), "\u{FFFD}");
    }
}
