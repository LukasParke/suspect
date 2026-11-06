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
    // `usize`, not `u32`: offsets in documents over 4 GiB must not wrap.
    line_starts: Vec<usize>,
}

impl LineIndex {
    /// Scans `bytes` once for `\n` and records every line start.
    #[must_use]
    pub fn new(bytes: &[u8]) -> Self {
        let mut line_starts = Vec::with_capacity(64);
        line_starts.push(0);
        for i in memchr_iter(b'\n', bytes) {
            line_starts.push(i + 1);
        }
        Self { line_starts }
    }

    /// Number of lines; a trailing newline starts an extra empty line.
    #[must_use]
    pub fn len_lines(&self) -> usize {
        self.line_starts.len()
    }

    /// True when the buffer contains no `\n`, i.e. at most one line (not
    /// necessarily zero bytes).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.line_starts.len() <= 1
    }

    fn line_for(&self, offset: usize, len: usize) -> usize {
        let offset = offset.min(len);
        match self.line_starts.binary_search(&offset) {
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
        let start = self.line_starts[line];
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
        let start = self.line_starts[line];
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
        let start = self.line_starts[line];
        (line, clamped - start)
    }

    /// Byte offset for a zero-based line plus column counted in Unicode
    /// scalars.
    #[must_use]
    pub fn offset_of(&self, bytes: &[u8], line: u32, col: u32) -> Option<usize> {
        let start = *self.line_starts.get(line as usize)?;
        if start > bytes.len() {
            return None;
        }
        let line_bytes = &bytes[start..];
        let line_end = line_bytes
            .iter()
            .position(|&b| b == b'\n')
            .unwrap_or(line_bytes.len());
        // Walk scalar boundaries in the original bytes, counting an invalid
        // UTF-8 sequence as one scalar exactly as `from_utf8_lossy` (and so
        // `line_col`) does; offsets into a lossily decoded copy would drift
        // after the first invalid byte.
        let mut scalars = 0u32;
        let mut position = 0usize;
        for chunk in line_bytes[..line_end].utf8_chunks() {
            for (i, _) in chunk.valid().char_indices() {
                if scalars == col {
                    return Some(start + position + i);
                }
                scalars += 1;
            }
            position += chunk.valid().len();
            if !chunk.invalid().is_empty() {
                if scalars == col {
                    return Some(start + position);
                }
                scalars += 1;
                position += chunk.invalid().len();
            }
        }
        Some(start + line_end)
    }

    /// Byte range of a line, excluding the trailing `\r\n` or `\n`.
    #[must_use]
    pub fn line_range(&self, bytes: &[u8], line: u32) -> Option<std::ops::Range<usize>> {
        let start = *self.line_starts.get(line as usize)?;
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

    #[test]
    fn offset_of_counts_invalid_utf8_like_line_col() {
        // 0xFF is one replacement scalar; `b` is the second scalar.
        let bytes = b"\xFFb\xF0\x9F\x98\x80c\n";
        let idx = LineIndex::new(bytes);
        assert_eq!(idx.offset_of(bytes, 0, 1), Some(1));
        assert_eq!(idx.offset_of(bytes, 0, 2), Some(2));
        assert_eq!(idx.offset_of(bytes, 0, 3), Some(6));
        for offset in [0, 1, 2, 6] {
            let (line, col) = idx.line_col(bytes, offset);
            assert_eq!(idx.offset_of(bytes, line, col), Some(offset));
        }
    }
}
