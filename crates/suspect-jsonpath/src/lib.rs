//! suspect-jsonpath: RFC 9535 JSONPath queries over suspect-low nodes.
//!
//! Compile once with [`Path::parse`], evaluate many times with
//! [`Path::query`] against any [`suspect_low::NodeRef`] subtree (typically a
//! document root from [`suspect_low::LowDoc::root`]). Results are
//! normalized: deduplicated by source position and ordered by document
//! position. Evaluation is iterative (explicit stacks for descendant
//! segments), so deeply nested documents cannot overflow the call stack.
//!
//! ```no_run
//! use suspect_jsonpath::Path;
//!
//! let path = Path::parse("$.store.book[?(@.price < 10)].title").unwrap();
//! // let results = path.query(doc.root());
//! // for node in results.iter() { /* ... */ }
//! ```

mod ast;
mod eval;
mod functions;
mod parser;

pub use eval::{NodeList, Path};

/// A JSONPath syntax error with its byte offset and reason.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid JSONPath query at offset {offset}: {reason}")]
pub struct PathError {
    /// The full input that failed to parse.
    pub input: String,
    /// Byte offset of the failure.
    pub offset: usize,
    /// Human-readable explanation.
    pub reason: String,
}
