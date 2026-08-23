//! Shared server state: open documents and the lazily built ref workspace.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use suspect_low::LowDoc;
use suspect_ref::{Workspace, WorkspaceBuilder};
use suspect_source::{LineIndex, Source, Uri};
use tower_lsp::lsp_types::{Position, Range, SemanticToken};

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
    /// Parses the buffer text into a [`LowDoc`] (which carries the line
    /// index) and keeps the raw text alongside it.
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
    pub docs: HashMap<Uri, Arc<OpenDoc>>,
    /// Bumped on every open/change; debounce tasks publish only when their
    /// Per-document edit counters: a debounce task publishes only when its
    /// captured generation still matches the document's current one.
    pub generations: HashMap<Uri, u64>,
    /// Per-document semantic-token caches for `semanticTokens/full/delta`:
    /// the result id handed to the client plus the encoded tokens.
    pub token_cache: HashMap<Uri, (String, Vec<SemanticToken>)>,
    /// Raw initialization options captured in `initialize` for later merge.
    pub pending_init_options: Option<serde_json::Value>,
    /// Merged server configuration (initialization options < client section).
    pub config: crate::config_files::SuspectConfig,
    /// Client capabilities from `initialize`, consulted when a feature can
    /// degrade (e.g. deferring work to `codeAction/resolve` needs
    /// `codeAction.resolveSupport`).
    pub client_caps: Option<tower_lsp::lsp_types::ClientCapabilities>,
}

impl State {
    /// Inserts or replaces an open document, reparsing its `LowDoc`.
    /// Drops a closed document from the cache and forgets its generation.
    pub fn close_doc(&mut self, uri: &Uri) {
        self.docs.remove(uri);
        self.generations.remove(uri);
    }

    /// Inserts or replaces a document and reparses it.
    pub fn open_doc(&mut self, uri: Uri, text: String) {
        self.docs
            .insert(uri.clone(), Arc::new(OpenDoc::parse(uri, text)));
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
    Range {
        start: Position::new(sl, sc),
        end: Position::new(el, ec),
    }
}

/// Applies LSP content changes sequentially, returning the new buffer.
///
/// Range-less changes replace the whole text (full-sync fallback);
/// ranged edits splice against the *current* intermediate state per the
/// spec — a fresh [`LineIndex`] maps each UTF-16 range to byte offsets
/// because prior edits in the same notification shift the lines. Returns
/// `None` when a range falls outside the document (protocol violation;
/// callers keep the previous buffer rather than corrupting it).
#[must_use]
pub fn apply_content_changes(
    current: &str,
    changes: &[tower_lsp::lsp_types::TextDocumentContentChangeEvent],
) -> Option<String> {
    let mut text = current.to_owned();
    for change in changes {
        match change.range {
            None => text = change.text.clone(),
            Some(range) => {
                let bytes = text.as_bytes();
                let li = LineIndex::new(bytes);
                let start = offset_of_utf16(bytes, &li, range.start.line, range.start.character)?;
                let end = offset_of_utf16(bytes, &li, range.end.line, range.end.character)?;
                if end < start || end > text.len() {
                    return None;
                }
                text.replace_range(start..end, &change.text);
            }
        }
    }
    Some(text)
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

    #[test]
    fn incremental_edits_insert_delete_and_replace() {
        use tower_lsp::lsp_types::TextDocumentContentChangeEvent;
        let r = |sl: u32, sc: u32, el: u32, ec: u32| {
            Some(Range::new(Position::new(sl, sc), Position::new(el, ec)))
        };
        // Insert "world" after "hello ".
        let out = apply_content_changes(
            "hello \n",
            &[TextDocumentContentChangeEvent {
                range: r(0, 6, 0, 6),
                range_length: None,
                text: "world".to_owned(),
            }],
        );
        assert_eq!(out.as_deref(), Some("hello world\n"));
        // Delete " world" again (single-line delete).
        let out = apply_content_changes(
            "hello world\n",
            &[TextDocumentContentChangeEvent {
                range: r(0, 5, 0, 11),
                range_length: None,
                text: String::new(),
            }],
        );
        assert_eq!(out.as_deref(), Some("hello\n"));
        // Multi-line replace.
        let out = apply_content_changes(
            "a: 1\nb: 2\nc: 3\n",
            &[TextDocumentContentChangeEvent {
                range: r(0, 3, 2, 4),
                range_length: None,
                text: "x".to_owned(),
            }],
        );
        assert_eq!(out.as_deref(), Some("a: x\n"));
    }

    #[test]
    fn sequential_changes_apply_against_intermediate_state() {
        use tower_lsp::lsp_types::TextDocumentContentChangeEvent;
        let changes = vec![
            // Insert a line at the top; every later range shifts down one.
            TextDocumentContentChangeEvent {
                range: Some(Range::new(Position::new(0, 0), Position::new(0, 0))),
                range_length: None,
                text: "# header\n".to_owned(),
            },
            // Replace "1" in "a: 1", which shifted from line 0 to line 1.
            TextDocumentContentChangeEvent {
                range: Some(Range::new(Position::new(1, 3), Position::new(1, 4))),
                range_length: None,
                text: "9".to_owned(),
            },
        ];
        let out = apply_content_changes("a: 1\n", &changes);
        assert_eq!(out.as_deref(), Some("# header\na: 9\n"));
    }

    #[test]
    fn full_text_change_still_replaces_whole_buffer() {
        use tower_lsp::lsp_types::TextDocumentContentChangeEvent;
        let out = apply_content_changes(
            "old\n",
            &[TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: "brand new\n".to_owned(),
            }],
        );
        assert_eq!(out.as_deref(), Some("brand new\n"));
    }

    #[test]
    fn out_of_range_edit_is_rejected_not_corrupting() {
        use tower_lsp::lsp_types::TextDocumentContentChangeEvent;
        let out = apply_content_changes(
            "tiny\n",
            &[TextDocumentContentChangeEvent {
                range: Some(Range::new(Position::new(0, 0), Position::new(9, 0))),
                range_length: None,
                text: "x".to_owned(),
            }],
        );
        assert_eq!(out, None);
    }
}
