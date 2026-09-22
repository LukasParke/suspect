//! Error types for schema compilation and validation.

use std::ops::Range;
use suspect_low::Pointer;
use thiserror::Error;

/// A single validation failure.
///
/// `instance_path` locates the offending value inside the validated
/// document; `schema_path` locates the keyword that rejected it inside the
/// schema (e.g. `#/properties/name/maxLength`).
#[derive(Debug, Clone, PartialEq, Error)]
#[error("{message} (at instance `{}`; schema `{}`)", instance_path.to_path(), schema_path.to_path())]
pub struct SchemaError {
    /// Distinguishes a schema assertion failure from an incomplete evaluation.
    pub kind: SchemaErrorKind,
    /// RFC 6901 pointer to the failing instance location.
    pub instance_path: Pointer,
    /// RFC 6901 pointer to the failing keyword in the schema.
    pub schema_path: Pointer,
    /// Human-readable explanation.
    pub message: String,
}

/// The meaning of a [`SchemaError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaErrorKind {
    /// A schema assertion rejected the instance.
    Invalid,
    /// Evaluation could not determine validity, for example because an exact
    /// arithmetic or equality resource limit was exceeded. Logical schema
    /// branches cannot suppress or invert this failure.
    Evaluation,
}

/// Failure to compile a schema.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum CompileError {
    /// An exact numeric operand exceeded the configured resource limit.
    #[error("{message}")]
    ResourceLimit {
        /// Explanation including the configured limit.
        message: String,
        /// Byte range of the operand in the schema source.
        at: Range<usize>,
    },
    /// Schema nesting exceeded [`Config::max_depth`](crate::Config::max_depth) during eager
    /// compilation.
    #[error("schema nesting exceeds maximum depth {cap}")]
    TooDeep {
        /// The configured depth cap.
        cap: usize,
    },
    /// The schema document is not a valid 2020-12 schema at this point
    /// (wrong keyword value type, unknown type name, unresolvable anchor…).
    #[error("{message}")]
    Invalid {
        /// Explanation.
        message: String,
        /// Byte range of the offending node in the source document.
        at: Range<usize>,
    },
    /// A valid schema requests semantics outside the exact portable subset.
    #[error("{message}")]
    Unsupported {
        /// Explanation of the unsupported construct.
        message: String,
        /// Byte range of the unsupported keyword value.
        at: Range<usize>,
    },
    /// A `pattern`/`patternProperties` regular expression failed to compile.
    #[error("invalid regular expression: {0}")]
    Regex(String),
}
