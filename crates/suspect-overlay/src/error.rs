use std::fmt;

/// Errors from parsing, validating, or applying overlays.
#[derive(Debug)]
pub enum OverlayError {
    /// The overlay document root is not an object.
    NotAnObject,
    /// A required field is missing or has the wrong type.
    MissingField {
        /// Name of the missing or mistyped field.
        field: &'static str,
    },
    /// An action violates the spec.
    InvalidAction {
        /// Zero-based position of the offending action in `actions`.
        index: usize,
        /// What is wrong with it.
        reason: String,
    },
    /// A `target` is not a valid JSONPath expression.
    InvalidTarget {
        /// Zero-based position of the offending action in `actions`.
        index: usize,
        /// The raw target expression text.
        input: String,
        /// Why the expression failed to parse.
        reason: String,
    },
    /// A target selected a node that cannot be updated or removed.
    TargetNotContainer {
        /// Zero-based position of the offending action in `actions`.
        index: usize,
        /// Pointer form of the selected scalar node.
        path: String,
    },
    /// JSONPath engine failure.
    Path(suspect_jsonpath::PathError),
}

impl fmt::Display for OverlayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAnObject => write!(f, "overlay root must be an object"),
            Self::MissingField { field } => write!(f, "overlay is missing required field `{field}`"),
            Self::InvalidAction { index, reason } => {
                write!(f, "invalid action #{index}: {reason}")
            }
            Self::InvalidTarget { index, input, reason } => {
                write!(f, "action #{index} has invalid target {input:?}: {reason}")
            }
            Self::TargetNotContainer { index, path } => {
                write!(f, "action #{index} target must select objects or arrays, got scalar at {path}")
            }
            Self::Path(e) => write!(f, "JSONPath error: {e}"),
        }
    }
}

impl std::error::Error for OverlayError {}

impl From<suspect_jsonpath::PathError> for OverlayError {
    fn from(e: suspect_jsonpath::PathError) -> Self {
        Self::Path(e)
    }
}
