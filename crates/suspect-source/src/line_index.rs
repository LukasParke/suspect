use memchr::memchr_iter;

/// Maps byte offsets to line/column and back.
///
/// Built once per document with a SIMD line-scan; queries are binary search.
/// The index does not own the buffer — accessors take it explicitly.
/// Lines and columns are zero-based; columns count Unicode scalar values
/// within the line unless a `_utf16` accessor says otherwise (LSP wants
/// UTF-16 code units).
#[derive(Debug, Clone)]
pub struct LineIndex {
    line_starts: Vec<u32>,
}

impl LineIndex {
    #[must_use]
    pub fn new(bytes: &[u8]) -> Self {
        let mut line_starts = Vec::with_capacity(64);
        line_starts.push(0u32);
        for i in memchr_iter(b'\n', bytes) {
            line_starts.push(i as u32 + 1);
        }
        Self { line_starts }
    }

    #[must_use]
    pub fn len_lines(&self) -> usize {
        self.line_starts.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.line_starts.len() <= 1
    }

    fn line_for(&self, offset: usize, len: usize) -> usize {
        let offset = offset.min(len);
        match self.line_starts.binary_search(&(offset as u32)) {
            Ok(l) => l,
            Err(ins) => ins.saturating_sub(1),
        }
    }

    /// Zero-based `(line, column-in-scalars)` for a byte offset. Offsets past
    /// the end clamp to the final position.
    #[must_use]
    pub fn line_col(&self, bytes: &[u8], offset: usize) -> (u32, u32) {
        let clamped = offset.min(bytes.len());
        let line = self.line_for(clamped, bytes.len());
        let start = self.line_starts[line] as usize;
        let col = String::from_utf8_lossy(&bytes[start..clamped.max(start)])
            .chars()
            .count() as u32;
        (line as u32, col)
    }

    /// Zero-based `(line, column-in-UTF-16-code-units)` — the LSP flavor.
    #[must_use]
    pub fn line_col_utf16(&self, bytes: &[u8], offset: usize) -> (u32, u32) {
        let clamped = offset.min(bytes.len());
        let line = self.line_for(clamped, bytes.len());
        let start = self.line_starts[line] as usize;
        let col = String::from_utf8_lossy(&bytes[start..clamped.max(start)])
            .chars()
            .map(char::len_utf16)
            .sum::<usize>() as u32;
        (line as u32, col)
    }

    /// Zero-based `(line, column-in-bytes)` — matches tree-sitter points.
    #[must_use]
    pub fn line_col_bytes(&self, bytes: &[u8], offset: usize) -> (usize, usize) {
        let clamped = offset.min(bytes.len());
        let line = self.line_for(clamped, bytes.len());
        let start = self.line_starts[line] as usize;
        (line, clamped - start)
    }

    /// Byte offset for a zero-based line plus column counted in Unicode
    /// scalars.
    #[must_use]
    pub fn offset_of(&self, bytes: &[u8], line: u32, col: u32) -> Option<usize> {
        let start = *self.line_starts.get(line as usize)? as usize;
        if start > bytes.len() {
            return None;
        }
        let line_bytes = &bytes[start..];
        let line_end = line_bytes
            .iter()
            .position(|&b| b == b'\n')
            .unwrap_or(line_bytes.len());
        for (scalars, (i, _)) in String::from_utf8_lossy(&line_bytes[..line_end])
            .char_indices()
            .enumerate()
        {
            if scalars as u32 == col {
                return Some(start + i);
            }
        }
        Some(start + line_end)
    }

    /// Byte range of a line, excluding the trailing `\r\n` or `\n`.
    #[must_use]
    pub fn line_range(&self, bytes: &[u8], line: u32) -> Option<std::ops::Range<usize>> {
        let start = *self.line_starts.get(line as usize)? as usize;
        if start > bytes.len() {
            return None;
        }
        let mut end = bytes[start..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(bytes.len(), |i| start + i);
        if end > start && bytes[end - 1] == b'\r' {
            end -= 1;
        }
        Some(start..end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "alpha: 1\nbeta: \"x\u{1F600}y\"\n\ngamma\n";

    #[test]
    fn line_col_basic() {
        let idx = LineIndex::new(TEXT.as_bytes());
        assert_eq!(idx.len_lines(), 5);
        let off = TEXT.find("beta").unwrap();
        assert_eq!(idx.line_col(TEXT.as_bytes(), off), (1, 0));
        // emoji is one scalar, two UTF-16 units
        let off = TEXT.find('\u{1F600}').unwrap();
        assert_eq!(idx.line_col(TEXT.as_bytes(), off), (1, 8));
        assert_eq!(idx.line_col_utf16(TEXT.as_bytes(), off), (1, 8));
        let off = TEXT.find("y\"").unwrap();
        assert_eq!(idx.line_col_utf16(TEXT.as_bytes(), off), (1, 10));
        assert_eq!(idx.line_col(TEXT.as_bytes(), off), (1, 9));
    }

    #[test]
    fn offsets_clamp() {
        let idx = LineIndex::new(TEXT.as_bytes());
        let (l, c) = idx.line_col(TEXT.as_bytes(), usize::MAX);
        assert_eq!((l, c), (4, 0));
    }

    #[test]
    fn offset_of_round_trips() {
        let idx = LineIndex::new(TEXT.as_bytes());
        for off in [0usize, 9, 10, 25, 26, 27] {
            if off > TEXT.len() {
                continue;
            }
            let (l, c) = idx.line_col(TEXT.as_bytes(), off);
            assert_eq!(
                idx.offset_of(TEXT.as_bytes(), l, c),
                Some(off),
                "offset {off}"
            );
        }
    }

    #[test]
    fn crlf_and_empty() {
        let bytes = b"a\r\nbb\r\n".as_slice();
        let idx = LineIndex::new(bytes);
        assert_eq!(idx.line_col(bytes, 5), (1, 2));
        assert_eq!(idx.line_range(bytes, 0).unwrap(), 0..1);
        let empty = LineIndex::new(b"");
        assert!(empty.is_empty());
    }
}
