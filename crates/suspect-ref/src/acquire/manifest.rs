use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use serde::Deserialize;
use suspect_source::Uri;
use url::Url;

use super::{AcquireError, AcquireErrorKind, Budget, Context, cache, redact_uri};
use crate::sha256_digest;

/// One declared or observed HTTP redirect, with both logical addresses retained.
#[derive(Clone, PartialEq, Eq)]
pub struct RedirectHop {
    pub(super) from_uri: Uri,
    pub(super) to_uri: Uri,
    pub(super) status: u16,
}

impl RedirectHop {
    /// Address that produced the redirect response.
    #[must_use]
    pub fn from_uri(&self) -> &Uri {
        &self.from_uri
    }
    /// Resolved target address, before any I/O to that address.
    #[must_use]
    pub fn to_uri(&self) -> &Uri {
        &self.to_uri
    }
    /// Redirect status (301, 302, 303, 307 or 308).
    #[must_use]
    pub fn status(&self) -> u16 {
        self.status
    }
}

impl std::fmt::Debug for RedirectHop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedirectHop")
            .field("from_uri", &redact_uri(&self.from_uri))
            .field("to_uri", &redact_uri(&self.to_uri))
            .field("status", &self.status)
            .finish()
    }
}

/// Immutable validated declaration of one exact document retrieval.
#[derive(Debug, Clone)]
pub struct ResourcePin {
    requested_uri: Uri,
    effective_uri: Uri,
    digest: String,
    media_type: String,
    via: String,
    redirects: Vec<RedirectHop>,
    retrieved_at: String,
    pub(super) retrieved_time: SystemTime,
    attempts: usize,
}

impl ResourcePin {
    /// Original logical retrieval request, not a JSON Schema identifier guess.
    #[must_use]
    pub fn requested_uri(&self) -> &Uri {
        &self.requested_uri
    }
    /// Declared final retrieval URI and reference base.
    #[must_use]
    pub fn effective_uri(&self) -> &Uri {
        &self.effective_uri
    }
    /// Declared exact-byte SHA-256.
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }
    /// Normalized supported JSON/YAML media type (without parameters).
    #[must_use]
    pub fn media_type(&self) -> &str {
        &self.media_type
    }
    /// Declared provenance: `local`, `direct`, or `redirect`.
    #[must_use]
    pub fn via(&self) -> &str {
        &self.via
    }
    /// Exact declared hop ledger. Acquisition verifies it without rewriting it.
    #[must_use]
    pub fn redirects(&self) -> &[RedirectHop] {
        &self.redirects
    }
    /// Caller-declared retrieval timestamp, in `YYYY-MM-DDTHH:MM:SSZ` form.
    #[must_use]
    pub fn retrieved_at(&self) -> &str {
        &self.retrieved_at
    }
    /// Declared request count: zero locally, otherwise hops plus one, no retries.
    #[must_use]
    pub fn attempts(&self) -> usize {
        self.attempts
    }
}

/// Validated immutable version-1 pin manifest. Unknown/duplicate JSON fields,
/// unpinned entries, ambiguous identities, and malformed provenance are rejected.
#[derive(Debug, Clone)]
pub struct PinManifest {
    entry: Uri,
    resources: Vec<ResourcePin>,
    fingerprint: String,
    pub(super) bytes: Arc<[u8]>,
}

impl PinManifest {
    /// Supported manifest version (currently 1).
    #[must_use]
    pub fn manifest_version(&self) -> u32 {
        1
    }
    /// Requested entry alias, which must itself have a resource declaration.
    #[must_use]
    pub fn entry(&self) -> &Uri {
        &self.entry
    }
    /// Every declared request, including local entries and dependencies.
    #[must_use]
    pub fn resources(&self) -> &[ResourcePin] {
        &self.resources
    }
    /// SHA-256 of the exact original JSON bytes; never a mutable "latest" key.
    #[must_use]
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    manifest_version: u32,
    entry: String,
    resources: Vec<RawResource>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawResource {
    requested_uri: String,
    effective_uri: String,
    digest: String,
    media_type: String,
    via: String,
    redirects: Vec<RawRedirect>,
    retrieved_at: String,
    attempts: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRedirect {
    from_uri: String,
    to_uri: String,
    status: u16,
}

pub(super) fn read(context: &Context, budget: &Budget<'_>) -> Result<PinManifest, AcquireError> {
    let bytes = cache::read_source(
        context,
        &context.manifest,
        budget.options.max_manifest_bytes,
        budget,
    )
    .map_err(|mut error| {
        if matches!(error.kind, AcquireErrorKind::TooLarge { .. }) {
            error.kind = AcquireErrorKind::ManifestTooLarge {
                limit: budget.options.max_manifest_bytes,
            };
        }
        error
    })?;
    let raw: RawManifest = serde_json::from_slice(&bytes).map_err(|error| {
        let mut result = context.error(invalid(
            "expected a versioned JSON pin manifest with only declared fields",
        ));
        result.line = Some(error.line());
        result
    })?;
    if raw.manifest_version != 1 {
        return Err(context.error(invalid("unsupported manifest_version")));
    }
    if raw.resources.len() > budget.options.max_docs {
        return Err(context.error(AcquireErrorKind::TooManyDocuments {
            limit: budget.options.max_docs,
        }));
    }
    let entry = parse_uri(&raw.entry).map_err(|kind| context.error(kind))?;
    let mut resources = Vec::with_capacity(raw.resources.len());
    let mut identities = BTreeMap::<Uri, (Uri, String, String)>::new();
    for (index, raw) in raw.resources.into_iter().enumerate() {
        budget.check(context)?;
        let ctx = Context {
            index: Some(index),
            ..context.clone()
        };
        let requested_uri = parse_uri(&raw.requested_uri).map_err(|kind| ctx.error(kind))?;
        let ctx = Context {
            uri: Some(requested_uri.clone()),
            ..ctx
        };
        let effective_uri = parse_uri(&raw.effective_uri).map_err(|kind| ctx.error(kind))?;
        if raw.digest.len() != 71
            || !raw.digest.starts_with("sha256-")
            || !raw.digest[7..]
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(ctx.error(invalid(
                "digest must be sha256- followed by 64 lowercase hexadecimal digits",
            )));
        }
        let media_type = media_type(&raw.media_type).map_err(|kind| ctx.error(kind))?;
        let retrieved_time =
            parse_utc_timestamp(&raw.retrieved_at).map_err(|kind| ctx.error(kind))?;
        if raw.redirects.len() > budget.options.max_redirects {
            return Err(ctx.error(AcquireErrorKind::TooManyRedirects {
                limit: budget.options.max_redirects,
            }));
        }
        let mut redirects = Vec::with_capacity(raw.redirects.len());
        let mut current = requested_uri.clone();
        for hop in raw.redirects {
            let from_uri = parse_uri(&hop.from_uri).map_err(|kind| ctx.error(kind))?;
            let to_uri = parse_uri(&hop.to_uri).map_err(|kind| ctx.error(kind))?;
            if from_uri != current
                || !is_redirect(hop.status)
                || !from_uri.is_remote()
                || !to_uri.is_remote()
            {
                return Err(ctx.error(invalid("redirects must be a continuous HTTP hop ledger")));
            }
            current = to_uri.clone();
            redirects.push(RedirectHop {
                from_uri,
                to_uri,
                status: hop.status,
            });
        }
        if current != effective_uri {
            return Err(ctx.error(invalid(
                "effective_uri must match the final declared redirect target",
            )));
        }
        let expected_via = if requested_uri.scheme() == "file" {
            "local"
        } else if redirects.is_empty() {
            "direct"
        } else {
            "redirect"
        };
        let expected_attempts = if expected_via == "local" {
            0
        } else {
            redirects.len() + 1
        };
        if raw.via != expected_via || raw.attempts != expected_attempts {
            return Err(ctx.error(invalid(
                "via/attempts must match local or single-attempt-per-hop retrieval",
            )));
        }
        for uri in [&requested_uri, &effective_uri] {
            let identity = (
                effective_uri.clone(),
                raw.digest.clone(),
                media_type.clone(),
            );
            if let Some(previous) = identities.get(uri) {
                if previous != &identity {
                    return Err(ctx.error(AcquireErrorKind::ConflictingIdentity));
                }
            } else {
                identities.insert(uri.clone(), identity);
            }
        }
        resources.push(ResourcePin {
            requested_uri,
            effective_uri,
            digest: raw.digest,
            media_type,
            via: raw.via,
            redirects,
            retrieved_at: raw.retrieved_at,
            retrieved_time,
            attempts: raw.attempts,
        });
    }
    if !resources
        .iter()
        .any(|resource| resource.requested_uri == entry)
    {
        return Err(context.error(invalid(
            "entry must have its own requested_uri resource pin",
        )));
    }
    Ok(PinManifest {
        entry,
        resources,
        fingerprint: sha256_digest(&bytes),
        bytes: bytes.into(),
    })
}

fn invalid(reason: &'static str) -> AcquireErrorKind {
    AcquireErrorKind::InvalidManifest { reason }
}

pub(super) fn parse_uri(raw: &str) -> Result<Uri, AcquireErrorKind> {
    if raw.len() > 8192
        || raw
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(invalid(
            "retrieval URI is oversized or contains whitespace/control characters",
        ));
    }
    // Uri::parse also accepts absolute filesystem paths. Manifests deliberately
    // require URI spellings so no source identity depends on the caller's cwd.
    let url =
        Url::parse(raw).map_err(|_| invalid("expected an absolute fragment-free retrieval URI"))?;
    if url.fragment().is_some() || !url.username().is_empty() || url.password().is_some() {
        return Err(invalid(
            "retrieval URIs must not contain fragments or embedded credentials",
        ));
    }
    match url.scheme() {
        "https" | "http" if url.host_str().is_some() => {}
        "file"
            if url.query().is_none()
                && url
                    .host_str()
                    .is_none_or(|host| host.is_empty() || host == "localhost")
                && url.to_file_path().is_ok() => {}
        _ => return Err(AcquireErrorKind::UnsupportedScheme),
    }
    let bytes = raw.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'%'
            && (!bytes.get(index + 1).is_some_and(u8::is_ascii_hexdigit)
                || !bytes.get(index + 2).is_some_and(u8::is_ascii_hexdigit))
        {
            return Err(invalid("retrieval URI contains an invalid percent escape"));
        }
    }
    Uri::parse(raw).map_err(|_| invalid("expected an absolute fragment-free retrieval URI"))
}

pub(super) fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

pub(super) fn media_type(value: &str) -> Result<String, AcquireErrorKind> {
    if value.len() > 256 || value.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(AcquireErrorKind::BadMediaType);
    }
    let mut parts = value.split(';');
    let essence = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
    let Some((top, sub)) = essence.split_once('/') else {
        return Err(AcquireErrorKind::BadMediaType);
    };
    if !matches!(top, "application" | "text")
        || sub.is_empty()
        || !sub
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b'-'))
        || !(matches!(
            essence.as_str(),
            "application/json"
                | "application/yaml"
                | "application/x-yaml"
                | "text/yaml"
                | "text/x-yaml"
        ) || sub.ends_with("+json")
            || sub.ends_with("+yaml"))
    {
        return Err(AcquireErrorKind::BadMediaType);
    }
    let mut charset = false;
    for parameter in parts {
        let Some((name, value)) = parameter.trim().split_once('=') else {
            return Err(AcquireErrorKind::BadMediaType);
        };
        if name.trim().eq_ignore_ascii_case("charset") {
            if charset || !value.trim().trim_matches('"').eq_ignore_ascii_case("utf-8") {
                return Err(AcquireErrorKind::BadMediaType);
            }
            charset = true;
        }
    }
    Ok(essence)
}

/// Parses the manifest's bounded UTC timestamp syntax, `YYYY-MM-DDTHH:MM:SSZ`.
/// This is also suitable for constructing a caller-owned stale-only cutoff.
///
/// # Errors
/// Invalid calendar dates/times, non-UTC spellings, or years before 1970.
pub fn parse_utc_timestamp(value: &str) -> Result<SystemTime, AcquireErrorKind> {
    let bad = || {
        invalid(
            "retrieved_at/cutoff must be a valid UTC YYYY-MM-DDTHH:MM:SSZ timestamp (year >= 1970)",
        )
    };
    if value.len() != 20 || !value.is_ascii() {
        return Err(bad());
    }
    let bytes = value.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        let valid = match index {
            4 | 7 => *byte == b'-',
            10 => *byte == b'T',
            13 | 16 => *byte == b':',
            19 => *byte == b'Z',
            _ => byte.is_ascii_digit(),
        };
        if !valid {
            return Err(bad());
        }
    }
    let number = |range: std::ops::Range<usize>| value[range].parse::<i64>().map_err(|_| bad());
    let year = number(0..4)?;
    let month = number(5..7)?;
    let day = number(8..10)?;
    let hour = number(11..13)?;
    let minute = number(14..16)?;
    let second = number(17..19)?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => 0,
    };
    if year < 1970 || day < 1 || day > days_in_month || hour > 23 || minute > 59 || second > 59 {
        return Err(bad());
    }
    // Civil date to days since 1970-01-01, with Gregorian leap-year handling.
    let y = year - i64::from(month <= 2);
    let era = y / 400;
    let yoe = y - era * 400;
    let adjusted_month = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * adjusted_month + 2) / 5 + day - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    let seconds = days * 86_400 + hour * 3600 + minute * 60 + second;
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds as u64))
        .ok_or_else(bad)
}
