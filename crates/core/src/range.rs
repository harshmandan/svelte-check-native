//! Byte-offset ranges into a source file.
//!
//! `Range` replaces `-rs`'s `Span`. The name aligns with LSP terminology and
//! signals that these are *byte* positions, not abstract token spans.
//!
//! All offsets are `u32` (4 GiB max) — well beyond any realistic source file.

use std::fmt;

/// A half-open byte range `[start, end)` into a source file.
///
/// Invariants: `start <= end`. The `new` constructor debug-asserts this; the
/// helper constructors maintain it structurally.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Range {
    pub start: u32,
    pub end: u32,
}

impl Range {
    /// Construct a range. Debug-asserts `start <= end`.
    #[inline]
    pub const fn new(start: u32, end: u32) -> Self {
        debug_assert!(start <= end, "Range::new requires start <= end");
        Self { start, end }
    }

    /// An empty range anchored at a single byte offset.
    #[inline]
    pub const fn empty_at(offset: u32) -> Self {
        Self {
            start: offset,
            end: offset,
        }
    }

    /// A range covering `[0, len)` — the entire source.
    #[inline]
    pub const fn of_source(src: &str) -> Self {
        Self::new(0, src.len() as u32)
    }

    /// Length in bytes.
    #[inline]
    pub const fn len(self) -> u32 {
        self.end - self.start
    }

    #[inline]
    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// Does `offset` lie within `[start, end)`?
    #[inline]
    pub const fn contains(self, offset: u32) -> bool {
        offset >= self.start && offset < self.end
    }

    /// Is `other` fully contained within `self`?
    #[inline]
    pub const fn contains_range(self, other: Range) -> bool {
        self.start <= other.start && other.end <= self.end
    }

    /// Does `self` overlap `other` (share at least one byte)?
    #[inline]
    pub const fn intersects(self, other: Range) -> bool {
        self.start < other.end && other.start < self.end
    }

    /// Smallest range containing both `self` and `other`.
    #[inline]
    pub fn merge(self, other: Range) -> Range {
        Range {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    /// Slice this range out of `src`. Panics if indices are out of bounds or
    /// not on a UTF-8 character boundary — callers should only pass ranges
    /// that came from well-formed parses of `src`.
    #[inline]
    pub fn slice(self, src: &str) -> &str {
        &src[self.start as usize..self.end as usize]
    }
}

impl fmt::Debug for Range {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

impl fmt::Display for Range {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_and_accessors() {
        let r = Range::new(3, 10);
        assert_eq!(r.start, 3);
        assert_eq!(r.end, 10);
        assert_eq!(r.len(), 7);
        assert!(!r.is_empty());
    }

    #[test]
    fn empty_at() {
        let r = Range::empty_at(5);
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
        assert_eq!(r.start, 5);
        assert_eq!(r.end, 5);
    }

    #[test]
    fn contains_byte() {
        let r = Range::new(3, 7);
        assert!(!r.contains(2));
        assert!(r.contains(3));
        assert!(r.contains(6));
        // Half-open: `end` itself is NOT contained.
        assert!(!r.contains(7));
        assert!(!r.contains(8));
    }

    #[test]
    fn contains_range_cases() {
        let outer = Range::new(0, 10);
        assert!(outer.contains_range(Range::new(2, 5)));
        assert!(outer.contains_range(Range::new(0, 10))); // equal bounds
        assert!(!outer.contains_range(Range::new(5, 11)));
        assert!(!outer.contains_range(Range::new(11, 12)));
    }

    #[test]
    fn intersects_cases() {
        let a = Range::new(3, 7);
        assert!(a.intersects(Range::new(0, 5)));
        assert!(a.intersects(Range::new(5, 10)));
        assert!(a.intersects(Range::new(4, 6)));
        assert!(!a.intersects(Range::new(0, 3))); // touching but not overlapping
        assert!(!a.intersects(Range::new(7, 10)));
    }

    #[test]
    fn merge_combines_extents() {
        let a = Range::new(3, 5);
        let b = Range::new(4, 10);
        assert_eq!(a.merge(b), Range::new(3, 10));
        assert_eq!(a.merge(Range::new(0, 1)), Range::new(0, 5));
    }

    #[test]
    fn slice_returns_substring() {
        let src = "hello world";
        let r = Range::new(6, 11);
        assert_eq!(r.slice(src), "world");
    }

    #[test]
    fn of_source_covers_all() {
        let src = "abcdef";
        assert_eq!(Range::of_source(src), Range::new(0, 6));
    }

    #[test]
    #[should_panic(expected = "Range::new requires start <= end")]
    fn debug_assert_start_le_end() {
        let _ = Range::new(5, 3);
    }
}
