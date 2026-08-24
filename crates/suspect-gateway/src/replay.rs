//! Cassette-backed replay serving.
//!
//! [`ReplayIndex`] loads a recorded cassette at startup and answers
//! requests from it: an exact hit on `(method, path+query)` wins; when no
//! exchange matches with its query string, the first entry whose URL path
//! matches ignoring the query is served instead. Misses produce `404`
//! problem+json. Recorded bodies are replayed byte-for-byte (base64
//! entries decode back to raw bytes), so binary payloads pass through.

use std::collections::HashMap;

use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use suspect_journal::{Body, CassetteEntry};

use crate::problem;

/// Strips scheme and authority from a recorded full URL, leaving
/// path-plus-query (fragments dropped). Bare paths pass through unchanged.
#[must_use]
fn url_key(url: &str) -> &str {
    let rest = match url.find("://") {
        Some(idx) => {
            let after = &url[idx + 3..];
            after.find('/').map_or("/", |s| &after[s..])
        }
        None => url,
    };
    match rest.split_once('#') {
        Some((path, _)) => path,
        None => rest,
    }
}

/// Drops the query portion of a path-plus-query string.
#[must_use]
fn path_only(path_and_query: &str) -> &str {
    path_and_query
        .split(['?', '#'])
        .next()
        .unwrap_or(path_and_query)
}

/// Startup index over one cassette's entries.
///
/// Built once by the gateway (or via [`replay_index`]); lookups are
/// allocation-free.
#[derive(Debug, Clone)]
pub struct ReplayIndex {
    /// `(METHOD, path+query)` -> index into [`ReplayIndex::ordered`].
    exact: HashMap<(String, String), usize>,
    /// All entries in cassette order (fallback search order).
    ordered: Vec<CassetteEntry>,
}

impl ReplayIndex {
    /// Builds an index from cassette entries.
    ///
    /// Later entries with an identical `(method, path+query)` key win,
    /// mirroring "last recording of an endpoint is freshest" intuition.
    #[must_use]
    pub fn new(entries: &[CassetteEntry]) -> Self {
        let mut exact = HashMap::with_capacity(entries.len());
        for (idx, entry) in entries.iter().enumerate() {
            exact.insert(
                (
                    entry.method.to_ascii_uppercase(),
                    url_key(&entry.url).to_owned(),
                ),
                idx,
            );
        }
        Self {
            exact,
            ordered: entries.to_vec(),
        }
    }

    /// Number of indexed exchanges.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ordered.len()
    }

    /// Whether the index holds no exchanges.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ordered.is_empty()
    }

    /// The indexed entries in cassette order.
    #[must_use]
    pub fn entries(&self) -> &[CassetteEntry] {
        &self.ordered
    }

    /// Looks up the exchange for a request.
    ///
    /// `url_path_and_query` is the request target (`/pets/42?full=1`).
    /// Exact `(method, path+query)` hits are preferred; otherwise the
    /// first entry in cassette order whose URL path matches ignoring the
    /// query is returned.
    #[must_use]
    pub fn lookup(&self, method: &str, url_path_and_query: &str) -> Option<&CassetteEntry> {
        if let Some(&idx) = self
            .exact
            .get(&(method.to_ascii_uppercase(), url_path_and_query.to_owned()))
        {
            return self.ordered.get(idx);
        }
        let want = path_only(url_path_and_query);
        self.ordered
            .iter()
            .find(|e| path_only(url_key(&e.url)) == want)
    }
}

/// Free-function constructor matching the platform contract:
/// `replay_index(entries) -> ReplayIndex`.
#[must_use]
pub fn replay_index(entries: &[CassetteEntry]) -> ReplayIndex {
    ReplayIndex::new(entries)
}

/// Serves one request from the index.
///
/// Recorded response headers are copied except hop-by-hop framing headers;
/// the body is decoded to raw bytes so non-UTF-8 (base64-encoded) entries
/// replay identically.
#[must_use]
pub fn respond(index: &ReplayIndex, method: &str, url_path_and_query: &str) -> Response {
    match index.lookup(method, url_path_and_query) {
        Some(entry) => {
            let mut builder = axum::http::Response::builder().status(entry.status);
            for (name, value) in &entry.response_headers {
                if matches!(
                    name.to_ascii_lowercase().as_str(),
                    "content-length" | "transfer-encoding" | "connection"
                ) {
                    continue;
                }
                builder = builder.header(name.as_str(), value.as_str());
            }
            builder
                .body(axum::body::Body::from(Bytes::from(
                    entry.response_body.bytes(),
                )))
                .unwrap_or_else(|_| problem_fallback())
        }
        None => problem(
            axum::http::StatusCode::NOT_FOUND,
            "Replay miss",
            Some(format!(
                "no recorded exchange for {method} {url_path_and_query}"
            )),
        ),
    }
}

/// Last-resort empty response used only if header reconstruction fails.
fn problem_fallback() -> Response {
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        [("content-type", "application/problem+json")],
        r#"{"title":"Internal error"}"#,
    )
        .into_response()
}

/// Replay-drift comparison: true when `live` bytes hash to the recorded
/// body's SHA-256 digest. Encoding-agnostic — both sides compare on raw
/// bytes, so UTF-8 and base64-recorded bodies are treated identically.
#[must_use]
pub fn body_matches(recorded: &Body, live: &[u8]) -> bool {
    suspect_journal::sha256_hex(live) == recorded.sha256
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_identical_bytes_across_encodings() {
        let text = Body::from_bytes(b"{\"ok\":true}");
        assert!(body_matches(&text, b"{\"ok\":true}"));

        // A binary (base64-stored) body still compares on raw bytes.
        let binary = Body::from_bytes(&[0u8, 159, 146, 150, 255]);
        assert!(body_matches(&binary, &[0u8, 159, 146, 150, 255]));
    }

    #[test]
    fn flags_drifted_bytes() {
        let text = Body::from_bytes(b"alpha");
        assert!(!body_matches(&text, b"beta"));
        assert!(!body_matches(&text, b""));
    }
}
