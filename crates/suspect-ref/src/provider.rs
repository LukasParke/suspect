//! Immutable byte snapshots supplied to an offline workspace.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};
use suspect_source::Uri;

/// Computes the manifest spelling of an exact byte digest (`sha256-<64 hex>`).
#[must_use]
pub fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256-{:x}", Sha256::digest(bytes))
}

/// Invalid input to a read-only [`DocumentProvider`].
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// A URI was not a canonical, fragment-free document identifier.
    #[error("invalid provider document URI")]
    InvalidUri,
    /// Documents must contain UTF-8 source bytes, rather than a lossy conversion.
    #[error("pinned document is not UTF-8: {uri}")]
    InvalidUtf8 {
        /// The requested logical retrieval URI.
        uri: Uri,
    },
    /// An alias identified incompatible bytes or incompatible retrieval bases.
    #[error("inconsistent pinned document identity: {uri}")]
    ConflictingIdentity {
        /// The logical identity claimed by both documents.
        uri: Uri,
    },
}

/// Immutable provenance of one supplied document. Cache paths are never URIs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentMetadata {
    pub(crate) requested_uri: Uri,
    pub(crate) effective_uri: Uri,
    pub(crate) digest: String,
    pub(crate) byte_len: u64,
    pub(crate) media_type: Option<String>,
    pub(crate) cache_path: Option<PathBuf>,
    pub(crate) manifest_path: Option<PathBuf>,
}

impl DocumentMetadata {
    /// Original logical lookup alias.
    #[must_use]
    pub fn requested_uri(&self) -> &Uri {
        &self.requested_uri
    }

    /// Final logical retrieval URI: source identity and relative-reference base.
    #[must_use]
    pub fn effective_uri(&self) -> &Uri {
        &self.effective_uri
    }

    /// SHA-256 of the exact bytes, including any UTF-8 BOM.
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// Exact byte length before parser BOM handling.
    #[must_use]
    pub fn byte_len(&self) -> u64 {
        self.byte_len
    }

    /// Validated JSON/YAML media type, if this snapshot came from acquisition.
    #[must_use]
    pub fn media_type(&self) -> Option<&str> {
        self.media_type.as_deref()
    }

    /// Content-addressed cache file to watch or reverify; never a source base.
    #[must_use]
    pub fn cache_path(&self) -> Option<&Path> {
        self.cache_path.as_deref()
    }

    /// Declared manifest whose pin authorized these bytes, if any.
    #[must_use]
    pub fn manifest_path(&self) -> Option<&Path> {
        self.manifest_path.as_deref()
    }

    /// Stable semantic input fingerprint, excluding cache location and timestamps.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        let mut hash = Sha256::new();
        for field in [
            self.requested_uri.as_str(),
            self.effective_uri.as_str(),
            &self.digest,
            self.media_type.as_deref().unwrap_or(""),
        ] {
            hash.update((field.len() as u64).to_be_bytes());
            hash.update(field.as_bytes());
        }
        format!("sha256-{:x}", hash.finalize())
    }
}

/// An owned byte snapshot with an original lookup alias and a retrieval base.
/// Clones share immutable bytes; no file, fetch callback, or mutable buffer is held.
#[derive(Clone)]
pub struct ProvidedDocument {
    pub(crate) metadata: DocumentMetadata,
    bytes: Arc<[u8]>,
}

impl ProvidedDocument {
    /// Pins in-memory UTF-8 bytes to explicit logical URIs without performing I/O.
    ///
    /// # Errors
    /// Invalid URI spellings or invalid UTF-8. Conflicting aliases are checked
    /// when the complete [`DocumentProvider`] is constructed.
    pub fn new(
        requested_uri: Uri,
        effective_uri: Uri,
        bytes: impl Into<Arc<[u8]>>,
    ) -> Result<Self, ProviderError> {
        for uri in [&requested_uri, &effective_uri] {
            if Uri::parse(uri.as_str()).ok().as_ref() != Some(uri) {
                return Err(ProviderError::InvalidUri);
            }
        }
        let bytes = bytes.into();
        std::str::from_utf8(&bytes).map_err(|_| ProviderError::InvalidUtf8 {
            uri: requested_uri.clone(),
        })?;
        Ok(Self {
            metadata: DocumentMetadata {
                requested_uri,
                effective_uri,
                digest: sha256_digest(&bytes),
                byte_len: bytes.len() as u64,
                media_type: None,
                cache_path: None,
                manifest_path: None,
            },
            bytes,
        })
    }

    /// Exact verified bytes. Only a shared read-only slice is exposed.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Logical source identity, digest, and optional cache/manifest provenance.
    #[must_use]
    pub fn metadata(&self) -> &DocumentMetadata {
        &self.metadata
    }
}

impl std::fmt::Debug for ProvidedDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProvidedDocument")
            .field("metadata", &self.metadata)
            .finish_non_exhaustive()
    }
}

/// A complete immutable document lookup table with no I/O capability.
///
/// Both requested aliases and effective retrieval URIs address the same bytes.
/// A workspace using this provider is closed: misses never fall back to the
/// filesystem or network. Construction rejects ambiguous aliases before any
/// document is made available to a consumer.
#[derive(Debug, Clone)]
pub struct DocumentProvider {
    documents: Arc<[ProvidedDocument]>,
    aliases: BTreeMap<Uri, usize>,
    fingerprint: String,
}

impl DocumentProvider {
    /// Constructs a snapshot from already-owned bytes; performs no I/O.
    ///
    /// # Errors
    /// Any URI identifies differing bytes or differing effective reference bases.
    pub fn new(
        documents: impl IntoIterator<Item = ProvidedDocument>,
    ) -> Result<Self, ProviderError> {
        let mut documents: Vec<_> = documents.into_iter().collect();
        documents.sort_by(|a, b| {
            a.metadata
                .requested_uri
                .cmp(&b.metadata.requested_uri)
                .then(a.metadata.effective_uri.cmp(&b.metadata.effective_uri))
        });
        let mut aliases = BTreeMap::<Uri, usize>::new();
        for (index, document) in documents.iter().enumerate() {
            for uri in [
                &document.metadata.requested_uri,
                &document.metadata.effective_uri,
            ] {
                if let Some(&previous) = aliases.get(uri) {
                    let previous = &documents[previous];
                    if previous.metadata.effective_uri != document.metadata.effective_uri
                        || previous.bytes != document.bytes
                        || previous.metadata.media_type != document.metadata.media_type
                    {
                        return Err(ProviderError::ConflictingIdentity { uri: uri.clone() });
                    }
                } else {
                    aliases.insert(uri.clone(), index);
                }
            }
        }
        let mut hash = Sha256::new();
        for (alias, &index) in &aliases {
            hash.update((alias.as_str().len() as u64).to_be_bytes());
            hash.update(alias.as_str().as_bytes());
            hash.update(documents[index].metadata.fingerprint().as_bytes());
        }
        Ok(Self {
            documents: documents.into(),
            aliases,
            fingerprint: format!("sha256-{:x}", hash.finalize()),
        })
    }

    /// Looks up either a requested alias or an effective logical URI in memory.
    #[must_use]
    pub fn document(&self, uri: &Uri) -> Option<&ProvidedDocument> {
        self.aliases.get(uri).map(|&index| &self.documents[index])
    }

    /// All declared byte snapshots, in stable requested-URI order.
    #[must_use]
    pub fn documents(&self) -> &[ProvidedDocument] {
        &self.documents
    }

    /// Exact logical lookup allowlist, including effective aliases.
    #[must_use]
    pub fn logical_uris(&self) -> Vec<Uri> {
        self.aliases.keys().cloned().collect()
    }

    /// Stable fingerprint of bytes, aliases, media types, and reference bases.
    #[must_use]
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}
