use suspect_source::{Source, Uri};
use suspect_syntax::SourceDoc;

use crate::node::NodeRef;
use crate::ValueKind;

/// Which specification family (and version) a document declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecFamily {
    /// `swagger: "2.0"`
    Oas2,
    /// `openapi: "3.0.x"`
    Oas30,
    /// `openapi: "3.1.x"`
    Oas31,
    /// `openapi: "3.2.x"`
    Oas32,
    /// `arazzo: "1.0.x"`
    Arazzo10,
    /// `overlay: "1.0.x"`
    Overlay10,
    /// Anything else (including unparseable roots).
    Unknown,
}

/// A parsed document with semantic access. Wraps a [`SourceDoc`] and hands
/// out [`NodeRef`] views; cheap to produce, immutable once built.
pub struct LowDoc {
    doc: SourceDoc,
}

impl LowDoc {
    /// Parses with format auto-detection.
    #[must_use]
    pub fn parse(uri: Uri, source: Source) -> LowDoc {
        Self { doc: SourceDoc::parse(uri, source) }
    }

    /// Parses with an explicit serialization format.
    #[must_use]
    pub fn with_format(uri: Uri, source: Source, format: suspect_syntax::Format) -> LowDoc {
        Self { doc: SourceDoc::with_format(uri, source, format) }
    }

    #[must_use]
    pub fn uri(&self) -> &Uri {
        self.doc.uri()
    }

    #[must_use]
    pub fn inner(&self) -> &SourceDoc {
        &self.doc
    }

    #[must_use]
    pub fn format(&self) -> suspect_syntax::Format {
        self.doc.format()
    }

    #[must_use]
    pub fn root(&self) -> NodeRef<'_> {
        NodeRef::new(self.doc.root())
    }

    /// Syntax-level errors from tree-sitter recovery.
    #[must_use]
    pub fn syntax_errors(&self) -> &[suspect_syntax::SyntaxError] {
        self.doc.errors()
    }

    /// Sniffs the spec family and version from the root mapping.
    ///
    /// Cheap: reads at most two root fields.
    #[must_use]
    pub fn sniff_family(&self) -> SpecFamily {
        let root = self.root();
        if root.kind() != ValueKind::Object {
            return SpecFamily::Unknown;
        }
        if let Some(v) = root.get("openapi") {
            return match v.as_str().unwrap_or("") {
                s if s.starts_with("3.0") => SpecFamily::Oas30,
                s if s.starts_with("3.1") => SpecFamily::Oas31,
                s if s.starts_with("3.2") => SpecFamily::Oas32,
                _ => SpecFamily::Unknown,
            };
        }
        if let Some(v) = root.get("swagger") {
            return if v.as_str() == Some("2.0") { SpecFamily::Oas2 } else { SpecFamily::Unknown };
        }
        if let Some(v) = root.get("arazzo") {
            return if v.as_str().is_some_and(|s| s.starts_with("1.")) {
                SpecFamily::Arazzo10
            } else {
                SpecFamily::Unknown
            };
        }
        if let Some(v) = root.get("overlay") {
            return if v.as_str().is_some_and(|s| s.starts_with("1.")) {
                SpecFamily::Overlay10
            } else {
                SpecFamily::Unknown
            };
        }
        SpecFamily::Unknown
    }
}

impl std::fmt::Debug for LowDoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LowDoc")
            .field("uri", &self.doc.uri().as_str())
            .field("family", &self.sniff_family())
            .finish()
    }
}
