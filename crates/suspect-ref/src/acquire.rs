//! Explicit acquisition of an immutable, SHA-256-pinned retrieval manifest.
//!
//! Acquisition is the only network-capable entry point. An [`AcquiredClosure`]
//! provides a complete verified in-memory snapshot to ordinary offline
//! [`crate::Workspace`] consumers. Neither identifiers nor `$ref` text expand the
//! retrieval list. See `docs/SDK-PINNED-CLOSURE-DESIGN.md` for the versioned format.

mod cache;
mod manifest;
mod transport;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use suspect_source::Uri;

use crate::{
    DocumentMetadata, DocumentProvider, ProvidedDocument, WorkspaceBuilder, sha256_digest,
};

pub use manifest::{PinManifest, RedirectHop, ResourcePin, parse_utc_timestamp};

/// Caller-selected refresh policy. No policy ever rewrites a declared manifest.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum RefreshPolicy {
    /// Reverify cached bytes; online acquisition fills only cache misses.
    #[default]
    Never,
    /// Reacquire pins whose declared retrieval timestamp predates this cutoff.
    StaleOnly {
        /// Caller-owned UTC staleness threshold.
        before: SystemTime,
    },
    /// Reacquire every pin, rejecting any byte or retrieval-provenance drift.
    All,
}

/// Cooperative cancellation shared with a caller or CLI signal handler.
/// Active curl children are killed and reaped when this token is cancelled.
#[derive(Debug, Default, Clone)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    /// Creates an uncancelled token.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Cancels the operation; cancellation cannot be reset on this token.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    /// Whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// One explicitly supplied credential header. Debug output redacts its value.
#[derive(Clone)]
pub struct CredentialHeader {
    name: String,
    value: String,
}

impl CredentialHeader {
    /// Stores one credential for the current request only. Acquisition validates
    /// header syntax, reserved transport headers, duplicates, and byte limits
    /// before starting the child process. Values never enter manifests or argv.
    #[must_use]
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

impl std::fmt::Debug for CredentialHeader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CredentialHeader([redacted])")
    }
}

/// Credential selection failed. Provider-specific error text is never logged.
#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("credential selection failed")]
pub struct CredentialsError;

/// Per-call credential selection, invoked afresh for every actual HTTP hop.
///
/// This synchronous hook should return promptly and perform no I/O. Credentials
/// are never copied from a prior hop, including same-authority redirects.
pub trait Credentials: Send + Sync {
    /// Selects credentials using the original requested URI and current origin
    /// (`scheme://host[:port]`). Returned values are sent only to this origin.
    ///
    /// # Errors
    /// Return [`CredentialsError`] when no credential decision can be made.
    fn headers(
        &self,
        requested_uri: &Uri,
        current_origin: &str,
    ) -> Result<Vec<CredentialHeader>, CredentialsError>;
}

/// Bounds and explicitly granted network policy for one acquisition call.
#[derive(Clone)]
pub struct AcquireOptions {
    /// Explicit cache root (default `.suspect-cache`); no ambient cache lookup.
    pub cache_dir: PathBuf,
    /// Maximum exact bytes in one document (default 64 MiB).
    pub max_bytes: u64,
    /// Maximum total bytes across all declared documents (default 256 MiB).
    pub max_total_bytes: u64,
    /// Maximum declared retrieval requests (default 10,000).
    pub max_docs: usize,
    /// Maximum manifest bytes (default 4 MiB).
    pub max_manifest_bytes: u64,
    /// Maximum response headers, chunk framing/extensions and trailers (64 KiB).
    pub max_header_bytes: usize,
    /// Maximum response header fields/blocks (default 128).
    pub max_header_count: usize,
    /// Maximum outgoing headers including credentials (default 16 KiB).
    pub max_request_header_bytes: usize,
    /// Maximum redirect hops for a resource (default 3).
    pub max_redirects: usize,
    /// End-to-end wall-clock deadline, including all resources/hops (30 seconds).
    pub timeout: Duration,
    /// Cache verification only: no source-file reads, child processes or network.
    pub offline: bool,
    /// Explicit refresh policy. Offline mode requires [`RefreshPolicy::Never`].
    pub refresh: RefreshPolicy,
    /// Exact destination origins permitted for cross-authority redirects.
    /// Every hop must also exactly match the manifest's declared redirect ledger.
    pub allowed_redirect_origins: Vec<String>,
    /// Test-only HTTP exceptions: exact `http://<numeric-loopback>[:port]` origins.
    /// Non-loopback hosts are rejected. This never disables HTTPS verification.
    pub insecure_test_origins: Vec<String>,
    /// Per-hop credentials; none are read from the environment or stored on disk.
    pub credentials: Option<Arc<dyn Credentials>>,
    /// Shared cancellation signal; checked before each I/O and while curl runs.
    pub cancellation: CancellationToken,
    /// Trusted curl executable. Defaults to `/usr/bin/curl` on Unix, `curl.exe`
    /// on Windows. The adapter disables config files and clears the environment.
    pub curl_program: PathBuf,
}

impl Default for AcquireOptions {
    fn default() -> Self {
        Self {
            cache_dir: PathBuf::from(".suspect-cache"),
            max_bytes: 64 << 20,
            max_total_bytes: 256 << 20,
            max_docs: 10_000,
            max_manifest_bytes: 4 << 20,
            max_header_bytes: 64 << 10,
            max_header_count: 128,
            max_request_header_bytes: 16 << 10,
            max_redirects: 3,
            timeout: Duration::from_secs(30),
            offline: false,
            refresh: RefreshPolicy::Never,
            allowed_redirect_origins: Vec::new(),
            insecure_test_origins: Vec::new(),
            credentials: None,
            cancellation: CancellationToken::new(),
            curl_program: PathBuf::from(if cfg!(windows) {
                "curl.exe"
            } else {
                "/usr/bin/curl"
            }),
        }
    }
}

impl std::fmt::Debug for AcquireOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcquireOptions")
            .field("cache_dir", &self.cache_dir)
            .field("offline", &self.offline)
            .field("refresh", &self.refresh)
            .field("max_bytes", &self.max_bytes)
            .field("max_total_bytes", &self.max_total_bytes)
            .field("max_docs", &self.max_docs)
            .field("timeout", &self.timeout)
            .field(
                "credentials",
                &self.credentials.as_ref().map(|_| "[redacted]"),
            )
            .finish_non_exhaustive()
    }
}

/// Structured, value-redacted acquisition failure classification.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AcquireErrorKind {
    /// The declared JSON manifest is invalid or unsupported.
    #[error("invalid pin manifest: {reason}")]
    InvalidManifest {
        /// Static explanation, never an untrusted scalar value.
        reason: &'static str,
    },
    /// Invalid or contradictory caller options.
    #[error("invalid acquisition options: {reason}")]
    InvalidOptions {
        /// Static explanation without credential values.
        reason: &'static str,
    },
    /// A required cache entry is absent.
    #[error("pinned cache entry is missing; explicitly acquire it before offline compilation")]
    CacheMiss,
    /// Cached or first-acquired bytes disagree with a declared digest.
    #[error("pinned bytes do not match {expected} (found {actual})")]
    DigestMismatch {
        /// Declared SHA-256.
        expected: String,
        /// Observed SHA-256.
        actual: String,
    },
    /// An explicit refresh observed different source bytes.
    #[error("refresh changed pinned bytes from {expected} to {actual}; manifest was not rewritten")]
    DigestDrift {
        /// Declared SHA-256.
        expected: String,
        /// Observed SHA-256.
        actual: String,
    },
    /// An alias names different bytes or incompatible reference bases.
    #[error("duplicate document identity has inconsistent pins")]
    ConflictingIdentity,
    /// Too many declared documents.
    #[error("pin manifest exceeds maximum document count ({limit})")]
    TooManyDocuments {
        /// Configured document count cap.
        limit: usize,
    },
    /// Manifest input exceeded its byte limit.
    #[error("pin manifest exceeds maximum byte size ({limit})")]
    ManifestTooLarge {
        /// Configured manifest byte cap.
        limit: u64,
    },
    /// A single response or cached/local document exceeded its exact-byte limit.
    #[error("document exceeds maximum byte size ({limit})")]
    TooLarge {
        /// Configured per-document byte cap.
        limit: u64,
    },
    /// The complete closure would exceed the aggregate memory/disk budget.
    #[error("pinned closure exceeds maximum total bytes ({limit})")]
    TotalTooLarge {
        /// Configured aggregate byte cap.
        limit: u64,
    },
    /// HTTP header bytes exceeded the bound before body acquisition.
    #[error("HTTP headers exceed maximum byte size ({limit})")]
    HeadersTooLarge {
        /// Configured header byte cap.
        limit: usize,
    },
    /// HTTP fields/informational responses exceeded their count bound.
    #[error("HTTP headers exceed maximum field count ({limit})")]
    TooManyHeaders {
        /// Configured header field cap.
        limit: usize,
    },
    /// A scheme other than file/HTTPS (or explicit test-loopback HTTP) was used.
    #[error("unsupported retrieval scheme")]
    UnsupportedScheme,
    /// Cleartext HTTP was not explicitly permitted for this test-loopback origin.
    #[error("HTTPS is required; HTTP requires an explicit numeric-loopback test origin")]
    InsecureScheme,
    /// Actual redirects departed from declared provenance.
    #[error("redirect ledger differs from the immutable manifest")]
    RedirectDrift,
    /// A redirect crossed authority without an explicit destination allowance.
    #[error("cross-authority redirect denied")]
    RedirectDenied,
    /// Too many redirects were observed or declared.
    #[error("redirects exceed maximum hop count ({limit})")]
    TooManyRedirects {
        /// Configured hop cap.
        limit: usize,
    },
    /// A response did not end at the declared effective URI.
    #[error("effective retrieval URI differs from the immutable manifest")]
    EffectiveUriDrift,
    /// Media type is absent, unsupported, or differs from the declared pin.
    #[error("expected the pinned UTF-8 JSON/YAML media type")]
    BadMediaType,
    /// Compressed content/transfer encodings are deliberately unsupported.
    #[error("compressed HTTP representations are not accepted; identity encoding is required")]
    UnsupportedEncoding,
    /// Byte-for-byte pins do not license lossy source transcoding.
    #[error("pinned document is not UTF-8")]
    InvalidUtf8,
    /// Invalid, ambiguous or incomplete HTTP framing/header data.
    #[error("invalid or incomplete HTTP response")]
    InvalidResponse,
    /// The server returned a non-success, non-redirect status.
    #[error("retrieval returned HTTP {status}")]
    HttpStatus {
        /// Actual response status code.
        status: u16,
    },
    /// The configured curl executable could not be started.
    #[error("could not start the configured curl transport")]
    TransportUnavailable,
    /// TLS certificate/hostname verification or handshake failed.
    #[error("verified HTTPS connection failed")]
    Tls,
    /// curl failed; stderr and server text are not copied into diagnostics.
    #[error("HTTP transport failed (exit status {status:?})")]
    Transport {
        /// Process exit code, if one was available.
        status: Option<i32>,
    },
    /// The operation-wide deadline expired.
    #[error("pinned acquisition timed out")]
    Timeout,
    /// The caller cancelled acquisition.
    #[error("pinned acquisition was cancelled")]
    Cancelled,
    /// Credential selection failed without exposing provider-specific details.
    #[error("credential selection failed")]
    Credentials,
    /// Credential headers were malformed, duplicated or overrode transport policy.
    #[error("invalid or reserved credential header")]
    InvalidHeader,
    /// Filesystem failure with the original untrusted error text omitted.
    #[error("I/O failed while {operation}: {kind}")]
    Io {
        /// Static operation description.
        operation: &'static str,
        /// OS error class; paths are supplied separately as provenance.
        kind: std::io::ErrorKind,
    },
}

impl AcquireErrorKind {
    /// Stable machine-readable diagnostic code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidManifest { .. } => "pin-manifest-invalid",
            Self::InvalidOptions { .. } => "pin-options-invalid",
            Self::CacheMiss => "pin-cache-missing",
            Self::DigestMismatch { .. } => "pin-digest-mismatch",
            Self::DigestDrift { .. } => "pin-digest-drift",
            Self::ConflictingIdentity => "pin-identity-conflict",
            Self::TooManyDocuments { .. } => "pin-document-limit",
            Self::ManifestTooLarge { .. } => "pin-manifest-limit",
            Self::TooLarge { .. } => "pin-byte-limit",
            Self::TotalTooLarge { .. } => "pin-total-byte-limit",
            Self::HeadersTooLarge { .. } | Self::TooManyHeaders { .. } => "pin-header-limit",
            Self::UnsupportedScheme => "pin-scheme-unsupported",
            Self::InsecureScheme => "pin-https-required",
            Self::RedirectDrift => "pin-redirect-drift",
            Self::RedirectDenied => "pin-redirect-denied",
            Self::TooManyRedirects { .. } => "pin-redirect-limit",
            Self::EffectiveUriDrift => "pin-effective-uri-drift",
            Self::BadMediaType => "pin-media-type",
            Self::UnsupportedEncoding => "pin-content-encoding",
            Self::InvalidUtf8 => "pin-utf8",
            Self::InvalidResponse => "pin-http-response",
            Self::HttpStatus { .. } => "pin-http-status",
            Self::TransportUnavailable | Self::Transport { .. } => "pin-transport",
            Self::Tls => "pin-tls",
            Self::Timeout => "pin-timeout",
            Self::Cancelled => "pin-cancelled",
            Self::Credentials | Self::InvalidHeader => "pin-credentials",
            Self::Io { .. } => "pin-io",
        }
    }
}

/// Structured source-linked failure. Display and Debug redact URI queries and
/// never contain credential/header values, server bodies, or curl stderr.
pub struct AcquireError {
    kind: AcquireErrorKind,
    context: Box<Context>,
    line: Option<usize>,
    redirects: Vec<RedirectHop>,
}

impl AcquireError {
    /// Structured error classification and any expected/actual digests.
    #[must_use]
    pub fn kind(&self) -> &AcquireErrorKind {
        &self.kind
    }
    /// Stable diagnostic code suitable for CLI and editor reports.
    #[must_use]
    pub fn code(&self) -> &'static str {
        self.kind.code()
    }
    /// The declared pin manifest, never a cache path substituted for a URI.
    #[must_use]
    pub fn manifest_path(&self) -> &Path {
        &self.context.manifest
    }
    /// Original logical retrieval URI, when a resource has been identified.
    #[must_use]
    pub fn uri(&self) -> Option<&Uri> {
        self.context.uri.as_ref()
    }
    /// Diagnostic-safe URI spelling with userinfo/query/fragment removed.
    #[must_use]
    pub fn redacted_uri(&self) -> Option<String> {
        self.uri().map(redact_uri)
    }
    /// Zero-based manifest resource index, when available.
    #[must_use]
    pub fn resource_index(&self) -> Option<usize> {
        self.context.index
    }
    /// One-based JSON parse-error line, when available.
    #[must_use]
    pub fn line(&self) -> Option<usize> {
        self.line
    }
    /// Cache or source path associated with an I/O or verification error.
    #[must_use]
    pub fn file_path(&self) -> Option<&Path> {
        self.context.file.as_deref()
    }
    /// Bounded observed redirect ledger, including a final denied hop.
    #[must_use]
    pub fn redirects(&self) -> &[RedirectHop] {
        &self.redirects
    }
}

impl std::fmt::Display for AcquireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{}] {} ({}",
            self.code(),
            self.kind,
            self.manifest_path().display()
        )?;
        if let Some(line) = self.line {
            write!(f, ":{line}")?;
        }
        if let Some(index) = self.resource_index() {
            write!(f, " /resources/{index}")?;
        }
        if let Some(uri) = self.redacted_uri() {
            write!(f, "; {uri}")?;
        }
        write!(f, ")")
    }
}

impl std::fmt::Debug for AcquireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcquireError")
            .field("kind", &self.kind)
            .field("manifest", &self.manifest_path())
            .field("uri", &self.redacted_uri())
            .field("resource_index", &self.resource_index())
            .field("line", &self.line)
            .field("redirects", &self.redirects)
            .finish()
    }
}

impl std::error::Error for AcquireError {}

#[derive(Clone)]
struct Context {
    manifest: PathBuf,
    uri: Option<Uri>,
    index: Option<usize>,
    file: Option<PathBuf>,
}

impl Context {
    fn error(&self, kind: AcquireErrorKind) -> AcquireError {
        AcquireError {
            kind,
            context: Box::new(self.clone()),
            line: None,
            redirects: Vec::new(),
        }
    }

    fn io(&self, operation: &'static str, error: &std::io::Error) -> AcquireError {
        self.error(AcquireErrorKind::Io {
            operation,
            kind: error.kind(),
        })
    }

    fn for_resource(&self, index: usize, resource: &ResourcePin) -> Self {
        Self {
            uri: Some(resource.requested_uri().clone()),
            index: Some(index),
            file: None,
            ..self.clone()
        }
    }

    fn for_file(&self, path: &Path) -> Self {
        Self {
            file: Some(path.to_path_buf()),
            ..self.clone()
        }
    }
}

struct Budget<'a> {
    options: &'a AcquireOptions,
    deadline: Instant,
}

impl Budget<'_> {
    fn check(&self, context: &Context) -> Result<(), AcquireError> {
        if self.options.cancellation.is_cancelled() {
            return Err(context.error(AcquireErrorKind::Cancelled));
        }
        if Instant::now() >= self.deadline {
            return Err(context.error(AcquireErrorKind::Timeout));
        }
        Ok(())
    }
}

/// Execution evidence for one declared retrieval, separate from immutable pins.
#[derive(Debug, Clone)]
pub struct AcquisitionRecord {
    requested_uri: Uri,
    from_cache: bool,
    attempts: usize,
    redirects: Vec<RedirectHop>,
}

impl AcquisitionRecord {
    /// Original declared retrieval request.
    #[must_use]
    pub fn requested_uri(&self) -> &Uri {
        &self.requested_uri
    }
    /// Whether all bytes came from a verified cache entry on this call.
    #[must_use]
    pub fn from_cache(&self) -> bool {
        self.from_cache
    }
    /// Actual HTTP requests on this call (zero for cached and local resources).
    #[must_use]
    pub fn attempts(&self) -> usize {
        self.attempts
    }
    /// Observed hops for this call; cached calls contain no new hop evidence.
    #[must_use]
    pub fn redirects(&self) -> &[RedirectHop] {
        &self.redirects
    }
}

/// A complete verified immutable snapshot, ready for an offline compiler.
#[derive(Debug, Clone)]
pub struct AcquiredClosure {
    manifest: Arc<PinManifest>,
    provider: Arc<DocumentProvider>,
    documents: Vec<DocumentMetadata>,
    records: Vec<AcquisitionRecord>,
    cache_manifest_path: PathBuf,
    entry: Uri,
    max_bytes: u64,
    max_docs: usize,
}

impl AcquiredClosure {
    /// Effective logical entry URI. Pass this to `Contract::from_workspace` so
    /// redirected entries use the same source identity as their parsed nodes.
    #[must_use]
    pub fn entry(&self) -> &Uri {
        &self.entry
    }
    /// Requested entry alias exactly as normalized in the declared manifest.
    #[must_use]
    pub fn requested_entry(&self) -> &Uri {
        self.manifest.entry()
    }
    /// Immutable declared pin metadata and its exact-byte manifest fingerprint.
    #[must_use]
    pub fn manifest(&self) -> &PinManifest {
        &self.manifest
    }
    /// Shared memory-only provider; this cannot reacquire or mutate bytes.
    #[must_use]
    pub fn provider(&self) -> Arc<DocumentProvider> {
        self.provider.clone()
    }
    /// Requested and effective lookup aliases admitted by the pin manifest.
    #[must_use]
    pub fn logical_uris(&self) -> Vec<Uri> {
        self.provider.logical_uris()
    }
    /// Immutable metadata including exact digests and cache paths for session
    /// invalidation, dependency watching, and compatibility-closure reporting.
    #[must_use]
    pub fn documents(&self) -> &[DocumentMetadata] {
        &self.documents
    }
    /// Execution evidence. Refresh timestamps never mutate the original manifest.
    #[must_use]
    pub fn records(&self) -> &[AcquisitionRecord] {
        &self.records
    }
    /// SHA-256 of the exact declared manifest; pins and aliases are included.
    #[must_use]
    pub fn fingerprint(&self) -> &str {
        self.manifest.fingerprint()
    }
    /// Content-addressed copy of the manifest, also reverified on offline loads.
    #[must_use]
    pub fn cache_manifest_path(&self) -> &Path {
        &self.cache_manifest_path
    }
    /// Constructs a closed offline workspace builder using the verified snapshot
    /// and the same document/byte limits. Callers can also set a reference depth cap.
    #[must_use]
    pub fn workspace_builder(&self) -> WorkspaceBuilder {
        WorkspaceBuilder::new()
            .allowed_documents(self.logical_uris())
            .document_provider(self.provider())
            .max_docs(self.max_docs)
            .max_doc_size(self.max_bytes)
    }
}

/// Acquires exactly the retrieval requests declared by a versioned pin manifest.
///
/// All existing cache bytes are verified before any source is contacted. Online
/// `Never` fills misses; explicit refreshes must reproduce both bytes and the
/// declared redirect ledger. Offline mode verifies every cache entry, including
/// the local entry, and never falls back to sources. No partial provider is returned.
///
/// # Errors
/// Structured missing/tampered/drifting pins, invalid manifests/policies, resource
/// limits, media/UTF-8 failures, transport failures, deadline, or cancellation.
pub fn acquire(
    manifest_path: &Path,
    options: AcquireOptions,
) -> Result<AcquiredClosure, AcquireError> {
    let context = Context {
        manifest: manifest_path.to_path_buf(),
        uri: None,
        index: None,
        file: None,
    };
    let deadline = Instant::now().checked_add(options.timeout).ok_or_else(|| {
        context.error(AcquireErrorKind::InvalidOptions {
            reason: "timeout is out of range",
        })
    })?;
    let budget = Budget {
        options: &options,
        deadline,
    };
    budget.check(&context)?;
    if options.offline && options.refresh != RefreshPolicy::Never {
        return Err(context.error(AcquireErrorKind::InvalidOptions {
            reason: "offline mode cannot refresh sources",
        }));
    }
    transport::validate_options(&options).map_err(|kind| context.error(kind))?;
    let manifest = Arc::new(manifest::read(&context, &budget)?);
    budget.check(&context)?;
    let cache_base = options
        .cache_dir
        .join("pins")
        .join(&manifest.fingerprint()[7..]);
    let cache_manifest_path = cache_base.join("manifest.json");
    let archived = cache::read_verified(
        &context.for_file(&cache_manifest_path),
        &cache_manifest_path,
        manifest.fingerprint(),
        options.max_manifest_bytes,
        &budget,
    )?;
    if options.offline && archived.is_none() {
        return Err(context
            .for_file(&cache_manifest_path)
            .error(AcquireErrorKind::CacheMiss));
    }
    // Verify the whole cache before any refresh or miss can contact a source.
    let mut cached = Vec::with_capacity(manifest.resources().len());
    let mut total = 0u64;
    for (index, resource) in manifest.resources().iter().enumerate() {
        let path = cache::document_path(&cache_base, resource);
        let ctx = context.for_resource(index, resource).for_file(&path);
        let limit = options
            .max_bytes
            .min(options.max_total_bytes.saturating_sub(total));
        let bytes = cache::read_verified(&ctx, &path, resource.digest(), limit, &budget)
            .map_err(|error| total_limit_error(error, limit, &options))?;
        if options.offline && bytes.is_none() {
            return Err(ctx.error(AcquireErrorKind::CacheMiss));
        }
        if let Some(bytes) = &bytes {
            total = total.checked_add(bytes.len() as u64).ok_or_else(|| {
                ctx.error(AcquireErrorKind::TotalTooLarge {
                    limit: options.max_total_bytes,
                })
            })?;
            if total > options.max_total_bytes {
                return Err(ctx.error(AcquireErrorKind::TotalTooLarge {
                    limit: options.max_total_bytes,
                }));
            }
            validate_utf8(bytes).map_err(|kind| ctx.error(kind))?;
        }
        cached.push((path, bytes));
    }
    let mut provided = Vec::with_capacity(cached.len());
    let mut records = Vec::with_capacity(cached.len());
    for (index, (resource, (path, bytes))) in manifest.resources().iter().zip(cached).enumerate() {
        let ctx = context.for_resource(index, resource);
        budget.check(&ctx)?;
        let refreshing = match options.refresh {
            RefreshPolicy::Never => false,
            RefreshPolicy::All => true,
            RefreshPolicy::StaleOnly { before } => resource.retrieved_time < before,
        };
        let (bytes, record) = match (bytes, refreshing) {
            (Some(bytes), false) => (
                bytes,
                AcquisitionRecord {
                    requested_uri: resource.requested_uri().clone(),
                    from_cache: true,
                    attempts: 0,
                    redirects: Vec::new(),
                },
            ),
            (bytes, _) => {
                if let Some(previous) = bytes {
                    total -= previous.len() as u64;
                }
                let limit = options
                    .max_bytes
                    .min(options.max_total_bytes.saturating_sub(total));
                let (bytes, redirects, attempts) = if let Some(source) =
                    resource.requested_uri().as_path()
                {
                    let bytes = cache::read_source(&ctx.for_file(&source), &source, limit, &budget)
                        .map_err(|error| total_limit_error(error, limit, &options))?;
                    (bytes, Vec::new(), 0)
                } else {
                    transport::retrieve(resource, &ctx, &budget, limit)
                        .map_err(|error| total_limit_error(error, limit, &options))?
                };
                let actual = sha256_digest(&bytes);
                if actual != resource.digest() {
                    let kind = if refreshing {
                        AcquireErrorKind::DigestDrift {
                            expected: resource.digest().into(),
                            actual,
                        }
                    } else {
                        AcquireErrorKind::DigestMismatch {
                            expected: resource.digest().into(),
                            actual,
                        }
                    };
                    return Err(ctx.error(kind));
                }
                validate_utf8(&bytes).map_err(|kind| ctx.error(kind))?;
                total = total.checked_add(bytes.len() as u64).ok_or_else(|| {
                    ctx.error(AcquireErrorKind::TotalTooLarge {
                        limit: options.max_total_bytes,
                    })
                })?;
                if total > options.max_total_bytes {
                    return Err(ctx.error(AcquireErrorKind::TotalTooLarge {
                        limit: options.max_total_bytes,
                    }));
                }
                (
                    bytes,
                    AcquisitionRecord {
                        requested_uri: resource.requested_uri().clone(),
                        from_cache: false,
                        attempts,
                        redirects,
                    },
                )
            }
        };
        let mut document = ProvidedDocument::new(
            resource.requested_uri().clone(),
            resource.effective_uri().clone(),
            bytes,
        )
        .map_err(|_| ctx.error(AcquireErrorKind::InvalidUtf8))?;
        document.metadata.media_type = Some(resource.media_type().into());
        document.metadata.cache_path = Some(path);
        document.metadata.manifest_path = Some(manifest_path.to_path_buf());
        provided.push(document);
        records.push(record);
    }
    let provider = Arc::new(
        DocumentProvider::new(provided)
            .map_err(|_| context.error(AcquireErrorKind::ConflictingIdentity))?,
    );
    // Only fully verified closures reach cache publication. Each immutable file
    // is atomically installed without replacing any existing cache entry.
    if !options.offline {
        for document in provider.documents() {
            let path = document
                .metadata()
                .cache_path()
                .expect("acquired cache path");
            cache::write_immutable(&context.for_file(path), path, document.bytes(), &budget)?;
        }
        cache::write_immutable(
            &context.for_file(&cache_manifest_path),
            &cache_manifest_path,
            &manifest.bytes,
            &budget,
        )?;
    }
    budget.check(&context)?;
    let entry = provider
        .document(manifest.entry())
        .expect("validated manifest entry")
        .metadata()
        .effective_uri()
        .clone();
    let documents = provider
        .documents()
        .iter()
        .map(|document| document.metadata().clone())
        .collect();
    Ok(AcquiredClosure {
        manifest,
        provider,
        documents,
        records,
        cache_manifest_path,
        entry,
        max_bytes: options.max_bytes,
        max_docs: options.max_docs,
    })
}

fn total_limit_error(
    mut error: AcquireError,
    limit: u64,
    options: &AcquireOptions,
) -> AcquireError {
    if limit < options.max_bytes && matches!(error.kind, AcquireErrorKind::TooLarge { .. }) {
        error.kind = AcquireErrorKind::TotalTooLarge {
            limit: options.max_total_bytes,
        };
    }
    error
}

fn validate_utf8(bytes: &[u8]) -> Result<(), AcquireErrorKind> {
    std::str::from_utf8(bytes)
        .map(|_| ())
        .map_err(|_| AcquireErrorKind::InvalidUtf8)
}

/// Diagnostic URI spelling with userinfo, query, and fragment removed. Logical
/// identity remains available separately on manifest/provider metadata.
#[must_use]
pub fn redact_uri(uri: &Uri) -> String {
    let Ok(mut url) = url::Url::parse(uri.as_str()) else {
        return "<invalid URI>".into();
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
}
