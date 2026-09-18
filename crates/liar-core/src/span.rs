//! Byte ranges into a source file.

/// A half-open byte range `[start, end)` within one file.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    /// # Panics
    /// If `end < start`.
    pub fn new(start: u32, end: u32) -> Self {
        assert!(end >= start, "span end {end} precedes start {start}");
        Self { start, end }
    }

    pub fn len(self) -> u32 {
        self.end - self.start
    }

    pub fn is_empty(self) -> bool {
        self.start == self.end
    }

    pub fn contains(self, offset: u32) -> bool {
        offset >= self.start && offset < self.end
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn len_is_the_byte_length() {
        assert_eq!(Span::new(3, 8).len(), 5);
    }

    #[test]
    fn an_empty_span_is_empty() {
        assert!(Span::new(4, 4).is_empty());
        assert!(!Span::new(4, 5).is_empty());
    }

    #[test]
    fn contains_is_half_open() {
        let span = Span::new(2, 5);
        assert!(!span.contains(1));
        assert!(span.contains(2));
        assert!(span.contains(4));
        assert!(!span.contains(5), "end is exclusive");
    }

    #[test]
    fn spans_sort_by_start_then_end() {
        let mut spans = vec![Span::new(5, 6), Span::new(1, 9), Span::new(1, 2)];
        spans.sort();
        assert_eq!(
            spans,
            vec![Span::new(1, 2), Span::new(1, 9), Span::new(5, 6)]
        );
    }
}
