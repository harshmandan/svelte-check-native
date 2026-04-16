//! Low-level byte scanner used by the structural parser.
//!
//! Operates on UTF-8 bytes with safe advance helpers. Keeps an offset
//! cursor; all predicates are zero-copy.

/// A cursor into a source string.
pub struct Scanner<'src> {
    source: &'src str,
    bytes: &'src [u8],
    pos: u32,
}

// Several helpers here are designed for the upcoming template parser; the
// structural parser uses only a subset. Allow dead_code at the impl level
// so we can land the full API incrementally without warnings.
#[allow(dead_code)]
impl<'src> Scanner<'src> {
    pub fn new(source: &'src str) -> Self {
        Self {
            source,
            bytes: source.as_bytes(),
            pos: 0,
        }
    }

    #[inline]
    pub fn source(&self) -> &'src str {
        self.source
    }

    #[inline]
    pub fn pos(&self) -> u32 {
        self.pos
    }

    #[inline]
    pub fn set_pos(&mut self, pos: u32) {
        self.pos = pos;
    }

    #[inline]
    pub fn eof(&self) -> bool {
        self.pos as usize >= self.bytes.len()
    }

    /// Byte at current position, or `None` at EOF.
    #[inline]
    pub fn peek_byte(&self) -> Option<u8> {
        self.bytes.get(self.pos as usize).copied()
    }

    /// Byte at `pos + offset`, or `None`.
    #[inline]
    pub fn peek_byte_at(&self, offset: u32) -> Option<u8> {
        self.bytes.get((self.pos + offset) as usize).copied()
    }

    /// Does the source at the current position start with `prefix`?
    ///
    /// ASCII-only comparison on bytes — prefixes we pass are all ASCII so
    /// this is safe and fast.
    pub fn starts_with(&self, prefix: &str) -> bool {
        let start = self.pos as usize;
        let end = start.saturating_add(prefix.len());
        if end > self.bytes.len() {
            return false;
        }
        &self.bytes[start..end] == prefix.as_bytes()
    }

    /// Case-insensitive variant of [`starts_with`] for ASCII prefixes.
    pub fn starts_with_ignore_case(&self, prefix: &str) -> bool {
        let start = self.pos as usize;
        let end = start.saturating_add(prefix.len());
        if end > self.bytes.len() {
            return false;
        }
        self.bytes[start..end]
            .iter()
            .zip(prefix.as_bytes())
            .all(|(a, b)| a.eq_ignore_ascii_case(b))
    }

    /// Advance by one byte. No-op at EOF.
    #[inline]
    pub fn advance_byte(&mut self) {
        if !self.eof() {
            self.pos += 1;
        }
    }

    /// Advance by one full UTF-8 code point. No-op at EOF.
    pub fn advance_char(&mut self) {
        let Some(b) = self.peek_byte() else { return };
        let step = utf8_len(b);
        self.pos = (self.pos + step).min(self.bytes.len() as u32);
    }

    /// Advance by `n` bytes.
    #[inline]
    pub fn advance(&mut self, n: u32) {
        self.pos = (self.pos + n).min(self.bytes.len() as u32);
    }

    /// Skip ASCII whitespace (spaces, tabs, newlines, `\r`).
    pub fn skip_ascii_whitespace(&mut self) {
        while let Some(b) = self.peek_byte() {
            if matches!(b, b' ' | b'\t' | b'\n' | b'\r') {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    /// Advance through `target` if it matches at the current position;
    /// return true on match.
    pub fn eat(&mut self, target: &str) -> bool {
        if self.starts_with(target) {
            self.pos += target.len() as u32;
            true
        } else {
            false
        }
    }

    /// Find the byte position of the next occurrence of `needle` starting at
    /// or after `pos`. Returns the source-relative byte offset, or `None`.
    pub fn find(&self, needle: &[u8]) -> Option<u32> {
        let start = self.pos as usize;
        // Short-circuit trivial cases.
        if needle.is_empty() || start >= self.bytes.len() {
            return None;
        }
        memchr::memmem::find(&self.bytes[start..], needle).map(|off| (start + off) as u32)
    }

    /// Find the next occurrence of any of several needles; return the
    /// earliest one together with its index into `needles`.
    pub fn find_any(&self, needles: &[&[u8]]) -> Option<(u32, usize)> {
        let mut earliest: Option<(u32, usize)> = None;
        for (i, n) in needles.iter().enumerate() {
            if let Some(p) = self.find(n) {
                match earliest {
                    Some((cur, _)) if cur <= p => {}
                    _ => earliest = Some((p, i)),
                }
            }
        }
        earliest
    }
}

/// UTF-8 sequence length from the leading byte.
///
/// Returns 1 for ASCII and also for continuation bytes (shouldn't appear as
/// leading; we fall through safely so the scanner never infinite-loops on
/// malformed input).
#[inline]
fn utf8_len(b: u8) -> u32 {
    if b < 0xC0 {
        1
    } else if b < 0xE0 {
        2
    } else if b < 0xF0 {
        3
    } else {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peek_and_advance() {
        let mut s = Scanner::new("abc");
        assert_eq!(s.peek_byte(), Some(b'a'));
        s.advance_byte();
        assert_eq!(s.peek_byte(), Some(b'b'));
        s.advance_byte();
        s.advance_byte();
        assert!(s.eof());
    }

    #[test]
    fn starts_with_matches_ascii() {
        let s = Scanner::new("hello world");
        assert!(s.starts_with("hello"));
        assert!(!s.starts_with("world"));
        assert!(!s.starts_with("hello world and more"));
    }

    #[test]
    fn starts_with_ignore_case_works() {
        let s = Scanner::new("<SCRIPT>");
        assert!(s.starts_with_ignore_case("<script"));
        assert!(!s.starts_with_ignore_case("<style"));
    }

    #[test]
    fn eat_consumes_match() {
        let mut s = Scanner::new("<script>body");
        assert!(s.eat("<script>"));
        assert_eq!(s.pos(), 8);
        assert_eq!(s.peek_byte(), Some(b'b'));
        assert!(!s.eat("nope"));
    }

    #[test]
    fn skip_ascii_whitespace_stops_at_non_ws() {
        let mut s = Scanner::new("  \n\t hello");
        s.skip_ascii_whitespace();
        assert_eq!(s.peek_byte(), Some(b'h'));
    }

    #[test]
    fn advance_char_handles_multi_byte() {
        let mut s = Scanner::new("a🎉b");
        s.advance_char();
        assert_eq!(s.pos(), 1);
        s.advance_char();
        assert_eq!(s.pos(), 5); // skipped 4 bytes of 🎉
        s.advance_char();
        assert_eq!(s.pos(), 6);
        assert!(s.eof());
    }

    #[test]
    fn find_locates_needle() {
        let s = Scanner::new("aaa</script>bbb");
        assert_eq!(s.find(b"</script>"), Some(3));
        assert_eq!(s.find(b"nope"), None);
    }

    #[test]
    fn find_any_returns_earliest_with_index() {
        let s = Scanner::new("first <style> then <script>");
        assert_eq!(s.find_any(&[b"<script", b"<style"]), Some((6, 1)));
    }
}
