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
        self.pending.extend_from_slice(chunk);
        let mut out = String::with_capacity(self.pending.len());
        let mut rest: &[u8] = &self.pending;
        loop {
            match std::str::from_utf8(rest) {
                Ok(s) => {
                    out.push_str(s);
                    rest = &[];
                    break;
                }
                Err(e) => {
                    let valid = e.valid_up_to();
                    // SAFETY-free: from_utf8 guarantees this prefix is valid.
                    out.push_str(std::str::from_utf8(&rest[..valid]).expect("valid prefix"));
                    match e.error_len() {
                        // Invalid sequence: replace it and keep going.
                        Some(len) => {
                            out.push('\u{FFFD}');
                            rest = &rest[valid + len..];
                        }
                        // Incomplete sequence at the end: wait for more bytes.
                        None => {
                            rest = &rest[valid..];
                            break;
                        }
                    }
                }
            }
        }
        self.pending = rest.to_vec();
        out
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

#[cfg(test)]
mod tests {
    use super::*;

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
