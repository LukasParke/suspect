//! Error types for the `$ref` resolution engine.

/// An error produced while resolving a single `$ref` edge or pointer.
#[derive(Debug, thiserror::Error)]
pub enum RefError {
    /// An RFC 6901 pointer did not designate a node inside its document.
    #[error("pointer `{pointer}` not found in {doc_uri}")]
    MissingPointer {
        /// URI of the document that was searched.
        doc_uri: String,
        /// The pointer that missed (serialized `/a/b` form).
        pointer: String,
    },
    /// A referenced document could not be loaded (missing file, unsupported
    /// scheme) or was re-entered while still loading.
    #[error("referenced document not available: {uri}")]
    MissingDoc {
        /// URI that could not be loaded.
        uri: String,
    },
    /// Filesystem I/O failed while loading a referenced document.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// A remote reference was encountered without an explicit byte provider.
    /// Ordinary workspace loads never perform network fetches.
    #[error("remote reference requires explicitly acquired pinned bytes: {uri}")]
    RemoteDenied {
        /// The denied remote URI.
        uri: String,
    },
    /// The retrieval URI was not admitted by the workspace's explicit allowlist.
    #[error("referenced document is outside the allowed source manifest: {uri}")]
    OutsideAllowlist {
        /// The denied retrieval URI; no document I/O was performed for it.
        uri: String,
    },
    /// A lazy or direct load would exceed the workspace's document cap.
    #[error("workspace exceeded maximum document count ({max})")]
    TooManyDocs {
        /// Configured maximum number of distinct documents (aliases do not count).
        max: usize,
    },
    /// Source bytes exceed the workspace's per-document limit.
    #[error("document {uri} exceeds max_doc_size ({limit})")]
    TooLarge {
        /// Logical URI of the oversized document.
        uri: String,
        /// Maximum permitted byte length.
        limit: u64,
    },
    /// A resolution chain or census walk exceeded its depth cap.
    #[error("resolution exceeded depth cap of {cap}")]
    TooDeep {
        /// The cap that was exceeded (see [`crate::WorkspaceBuilder::depth_cap`]).
        cap: usize,
    },
    /// The `$ref` value itself is malformed (unparseable URI, invalid
    /// percent-escape, unknown plain-name anchor).
    #[error("invalid $ref `{raw}`: {reason}")]
    InvalidRef {
        /// The raw `$ref` string.
        raw: String,
        /// Why it could not be used.
        reason: String,
    },
    /// Eager operations that must materialize a value hit a resolution
    /// cycle. Lazy chain resolution reports [`crate::Resolution::Cycle`] instead.
    #[error("resolution cycle detected")]
    CycleDetected,
}

/// An error produced by workspace-level operations (opening entries,
/// breadth-first loading).
#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    /// Filesystem I/O failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// The entry string could not be interpreted as a path or URI.
    #[error("invalid workspace entry: {0}")]
    InvalidEntry(String),
    /// Breadth-first loading would exceed the configured document cap.
    #[error("workspace exceeded maximum document count ({max})")]
    TooManyDocs {
        /// The configured `max_docs`.
        max: usize,
    },
    /// A `$ref` error occurred while walking the graph.
    #[error(transparent)]
    Ref(#[from] RefError),
}
