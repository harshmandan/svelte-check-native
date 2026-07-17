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

    /// Skip whitespace as the Svelte compiler defines it — the ASCII run
    /// plus the rare Unicode spaces. See [`is_svelte_whitespace`].
    pub fn skip_svelte_whitespace(&mut self) {
        self.pos = skip_svelte_whitespace_at(self.bytes, self.pos as usize) as u32;
    }

    /// Jump forward to the next occurrence of byte `a` or `b`, leaving
    /// the cursor ON that byte — or at EOF when neither occurs.
    ///
    /// Replaces per-char peek/advance walks in find-next-dispatch-byte
    /// loops with one memchr2 sweep. Both needles must be ASCII: an
    /// ASCII byte can never appear inside a UTF-8 continuation
    /// sequence, so the byte-level jump always lands on a char
    /// boundary. No other scanner state exists (positions are plain
    /// byte offsets; there is no line accounting), so skipping is
    /// state-identical to advancing char by char.
    pub fn skip_until2(&mut self, a: u8, b: u8) {
        debug_assert!(a.is_ascii() && b.is_ascii());
        let start = (self.pos as usize).min(self.bytes.len());
        self.pos = match memchr::memchr2(a, b, &self.bytes[start..]) {
            Some(off) => (start + off) as u32,
            None => self.bytes.len() as u32,
        };
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
}

/// Whitespace as the Svelte compiler defines it (`is_whitespace` in
/// `phases/1-parse/index.js`): the ASCII run (space, `\t`, `\n`, `\v`,
/// `\f`, `\r`) plus NBSP, U+1680, U+2000-200A, U+2028, U+2029, U+202F,
/// U+205F, U+3000 and U+FEFF.
#[inline]
pub fn is_svelte_whitespace(c: char) -> bool {
    matches!(c,
        ' ' | '\t'..='\r'
        | '\u{a0}'
        | '\u{1680}'
        | '\u{2000}'..='\u{200a}'
        | '\u{2028}'
        | '\u{2029}'
        | '\u{202f}'
        | '\u{205f}'
        | '\u{3000}'
        | '\u{feff}')
}

/// Byte-offset variant of [`Scanner::skip_svelte_whitespace`] for the
/// byte-level scan loops: returns the first offset at/after `i` that is
/// not svelte whitespace. `bytes` must be valid UTF-8 (it always is —
/// every caller passes a `&str`'s bytes); a malformed sequence stops the
/// skip rather than panicking.
pub(crate) fn skip_svelte_whitespace_at(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        let b = bytes[i];
        if matches!(b, b' ' | 0x09..=0x0d) {
            i += 1;
            continue;
        }
        if b < 0x80 {
            break;
        }
        match char_at(bytes, i) {
            Some((c, len)) if is_svelte_whitespace(c) => i += len,
            _ => break,
        }
    }
    i
}

/// Is the char at byte offset `i` svelte whitespace?
pub(crate) fn is_svelte_whitespace_at(bytes: &[u8], i: usize) -> bool {
    skip_svelte_whitespace_at(bytes, i) > i
}

/// Is the char ENDING at byte offset `i` (exclusive) svelte whitespace?
pub(crate) fn is_svelte_whitespace_before(bytes: &[u8], i: usize) -> bool {
    if i == 0 {
        return false;
    }
    let b = bytes[i - 1];
    if b < 0x80 {
        return matches!(b, b' ' | 0x09..=0x0d);
    }
    // Walk back over UTF-8 continuation bytes to the leading byte, then
    // decode and require the char to span exactly up to `i`.
    let mut start = i - 1;
    let mut steps = 0;
    while start > 0 && bytes[start] & 0xC0 == 0x80 && steps < 3 {
        start -= 1;
        steps += 1;
    }
    matches!(char_at(bytes, start), Some((c, len)) if start + len == i && is_svelte_whitespace(c))
}

/// Length in bytes of the Unicode identifier at the start of `s` (0
/// when `s` doesn't begin with an identifier-start char). Uses oxc's
/// identifier tables — the same definition acorn's
/// isIdentifierStart/Char gives upstream's `read_identifier`.
pub(crate) fn unicode_identifier_len(s: &str) -> u32 {
    let mut chars = s.char_indices();
    match chars.next() {
        Some((_, c)) if oxc_syntax::identifier::is_identifier_start(c) => {}
        _ => return 0,
    }
    for (idx, c) in chars {
        if !oxc_syntax::identifier::is_identifier_part(c) {
            return idx as u32;
        }
    }
    s.len() as u32
}

/// Decode the char at byte offset `i` of valid-UTF-8 `bytes`. Returns
/// the char and its encoded length, or `None` on a malformed sequence.
pub(crate) fn char_at(bytes: &[u8], i: usize) -> Option<(char, usize)> {
    let b = *bytes.get(i)?;
    if b < 0x80 {
        return Some((b as char, 1));
    }
    let len = utf8_len(b) as usize;
    let slice = bytes.get(i..i + len)?;
    let c = std::str::from_utf8(slice).ok()?.chars().next()?;
    Some((c, len))
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
    fn skip_until2_lands_on_needle_or_eof() {
        let mut s = Scanner::new("plain text {x} more");
        s.skip_until2(b'<', b'{');
        assert_eq!(s.pos(), 11);
        assert_eq!(s.peek_byte(), Some(b'{'));
        // Cursor already on a needle byte: no movement.
        s.skip_until2(b'<', b'{');
        assert_eq!(s.pos(), 11);
        // No needle ahead: lands at EOF.
        s.advance_byte();
        s.skip_until2(b'<', b'`');
        assert!(s.eof());
    }

    #[test]
    fn skip_until2_steps_over_multibyte_chars() {
        let mut s = Scanner::new("a🎉b<div>");
        s.skip_until2(b'<', b'{');
        assert_eq!(s.pos(), 6);
        assert_eq!(s.peek_byte(), Some(b'<'));
    }

    #[test]
    fn find_locates_needle() {
        let s = Scanner::new("aaa</script>bbb");
        assert_eq!(s.find(b"</script>"), Some(3));
        assert_eq!(s.find(b"nope"), None);
    }
}
