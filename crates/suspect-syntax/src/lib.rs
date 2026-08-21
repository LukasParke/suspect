//! suspect-syntax: lossless tree-sitter CSTs for JSON and YAML.
//!
use tree_sitter::Language;
use tree_sitter_language::LanguageFn;

unsafe extern "C" {
    fn tree_sitter_json() -> *const ();
    fn tree_sitter_yaml() -> *const ();
}

/// The vendored JSON grammar.
#[must_use]
pub fn json_language() -> Language {
    unsafe { LanguageFn::from_raw(tree_sitter_json).into() }
}

/// The vendored YAML grammar.
#[must_use]
pub fn yaml_language() -> Language {
    unsafe { LanguageFn::from_raw(tree_sitter_yaml).into() }
}


mod doc;
mod node;

pub use doc::{Edit, Point, SourceDoc, SyntaxError};
pub use node::SNode;

use std::fmt;

/// Serialization format of a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Json,
    Yaml,
}

/// Normalized CST node kinds, unified across both grammars.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SyntaxKind {
    /// Multi-document YAML stream root.
    Stream,
    /// A single document (JSON root object/array or one YAML document).
    Document,
    /// Object / block mapping / flow mapping.
    Mapping,
    /// Array / block sequence / flow sequence.
    Sequence,
    /// One key-value entry of a mapping.
    Pair,
    /// Any scalar leaf: plain, quoted, block, number, bool, null.
    Scalar,
    Anchor,
    Alias,
    Tag,
    Comment,
    Directive,
    /// tree-sitter error node; downstream must tolerate these everywhere.
    Error,
}

/// How a scalar was written — drives type inference in `suspect-low`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScalarStyle {
    /// Bare token (`abc`, `3.14`, `true`) or JSON literal.
    Plain,
    /// `'single quoted'`
    SingleQuoted,
    /// `"double quoted"` (may contain escapes)
    DoubleQuoted,
    /// `|` or `>` block scalar
    Block,
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Format::Json => f.write_str("json"),
            Format::Yaml => f.write_str("yaml"),
        }
    }
}

#[cfg(test)]
mod probe;
