//! Shared server state: open documents and the lazily built ref workspace.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use suspect_low::LowDoc;
use suspect_ref::{Workspace, WorkspaceBuilder};
use suspect_source::{LineIndex, Source, Uri};
use tower_lsp::lsp_types::{Position, Range};

/// One editor-open document: the live buffer text plus the `LowDoc` parsed
/// from it. The line index lives inside the parsed document
/// (`SourceDoc::line_index`), so no separate copy is kept.
pub struct OpenDoc {
    /// Raw buffer text as last reported by the editor.
    pub text: String,
    /// [`LowDoc`] parsed from [`OpenDoc::text`]; carries the line index.
    pub low: LowDoc,
}

impl OpenDoc {
    pub fn parse(uri: Uri, text: String) -> OpenDoc {
        let low = LowDoc::parse(uri.clone(), Source::from_vec(text.clone().into_bytes()));
        OpenDoc { text, low }
    }
}

/// Everything the backend knows, guarded by one async `RwLock`.
#[derive(Default)]
pub struct State {
    /// Workspace root directory (from `initialize`), if any.
    pub root: Option<PathBuf>,
    /// Lazily built `$ref` workspace; dropped on
    /// `workspace/didChangeWatchedFiles` so the next query reloads from disk.
    pub workspace: Option<Arc<Workspace>>,
    /// Per-open-document cache keyed by canonical URI.
    pub docs: HashMap<Uri, OpenDoc>,
    /// Bumped on every open/change; debounce tasks publish only when their
    /// captured generation is still current.
    pub generation: u64,
}

impl State {
    /// Inserts or replaces an open document, reparsing its `LowDoc`.
    pub fn open_doc(&mut self, uri: Uri, text: String) {
        self.docs.insert(uri.clone(), OpenDoc::parse(uri, text));
    }

    /// Returns the cached workspace, building it against the workspace root
    /// on first use.
    pub fn ensure_workspace(&mut self) -> Option<Arc<Workspace>> {
        if let Some(ws) = &self.workspace {
            return Some(Arc::clone(ws));
        }
        let root = self.root.clone()?;
        match WorkspaceBuilder::new().root(root).build() {
            Ok(ws) => {
                let arc = Arc::new(ws);
                self.workspace = Some(Arc::clone(&arc));
                Some(arc)
            }
            Err(_) => None,
        }
    }

}

/// Byte offset for an LSP position (UTF-16 line/column). Columns that land
/// mid-character round down to the containing character's start; positions
/// past the end of a line clamp to the line end.
#[must_use]
pub fn offset_of_utf16(bytes: &[u8], li: &LineIndex, line: u32, col_utf16: u32) -> Option<usize> {
    let r = li.line_range(bytes, line)?;
    let text = std::str::from_utf8(&bytes[r.start..r.end]).ok()?;
    let mut seen = 0u32;
    for (i, ch) in text.char_indices() {
        let width = u32::try_from(ch.len_utf16()).ok()?;
        if seen + width > col_utf16 {
            // `col_utf16` is at or inside this character: round to its start.
            return Some(r.start + i);
        }
        seen += width;
    }
    Some(r.start + text.len())
}

/// Converts a byte range into an LSP range with UTF-16 positions.
#[must_use]
pub fn lsp_range(bytes: &[u8], li: &LineIndex, range: std::ops::Range<usize>) -> Range {
    let (sl, sc) = li.line_col_utf16(bytes, range.start);
    let (el, ec) = li.line_col_utf16(bytes, range.end);
    Range { start: Position::new(sl, sc), end: Position::new(el, ec) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_offset_roundtrip() {
        // "aé😀\nxyz" — 😀 is 4 UTF-8 bytes, 2 UTF-16 units.
        let bytes = "aé😀\nxyz".as_bytes();
        let li = LineIndex::new(bytes);
        assert_eq!(offset_of_utf16(bytes, &li, 0, 0), Some(0));
        assert_eq!(offset_of_utf16(bytes, &li, 0, 1), Some(1));
        // Column 2 is the start of the 4-byte emoji; column 3 rounds down
        // to the same character start.
        assert_eq!(offset_of_utf16(bytes, &li, 0, 2), Some(3));
        assert_eq!(offset_of_utf16(bytes, &li, 0, 3), Some(3));
        assert_eq!(offset_of_utf16(bytes, &li, 0, 5), Some(7)); // end of line
        assert_eq!(offset_of_utf16(bytes, &li, 1, 1), Some(9));
        assert_eq!(offset_of_utf16(bytes, &li, 9, 0), None);
    }

    #[test]
    fn lsp_range_uses_utf16_columns() {
        let bytes = "aé😀: x\n".as_bytes();
        let li = LineIndex::new(bytes);
        let r = lsp_range(bytes, &li, 0..bytes.len() - 1);
        assert_eq!(r.start, Position::new(0, 0));
        assert_eq!(r.end, Position::new(0, 7)); // 1 + 1 + 2 + 3 UTF-16 units
    }
}
