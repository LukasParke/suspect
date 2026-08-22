//! suspect-source: document loading for suspect.
//!
//! Owns raw bytes (mmap or pooled), encoding detection/transcoding, line
//! indexes, and canonical URI handling. Everything above this layer is
//! zero-copy over the buffers loaded here.
//!
//! Canonical form: every [`Source`] is UTF-8. BOMs are stripped and UTF-16
//! inputs are transcoded once at load; all downstream byte offsets refer to
//! the decoded buffer, not the on-disk bytes.

mod line_index;
mod source;
mod uri;

pub use line_index::LineIndex;
pub use source::{Encoding, MMAP_THRESHOLD, Source};
pub use uri::Uri;
