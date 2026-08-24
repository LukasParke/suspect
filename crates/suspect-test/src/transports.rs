//! Deterministic in-process [`HttpClient`](crate::exec::HttpClient)
//! transports used for testing and offline runs.
//!
//! No real network transport ships in this crate: HTTP clients arrive with
//! the CLI. These transports make plan execution fully deterministic —
//! [`CannedTransport`] matches requests against declared rules, and
//! [`ReplayTransport`] serves recorded cassette entries.

use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;

use suspect_journal::CassetteEntry;

use crate::exec::{HttpClient, HttpRequest, HttpResponse, TransportError};

/// Request-matching rule of a canned route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// Required method (`"GET"`); `None` matches any method.
    pub method: Option<String>,
    /// Suffix the request URL must end with (e.g. `/pets`).
    pub path_suffix: String,
}

/// Transport serving pre-declared responses.
///
/// The first rule whose method matches (when set) and whose
/// [`Match::path_suffix`] is a suffix of the request URL wins; a request
/// matching no rule is a transport error.
#[derive(Debug, Clone, Default)]
pub struct CannedTransport {
    /// Ordered `(match, response)` routes.
    pub rules: Vec<(Match, HttpResponse)>,
}

impl CannedTransport {
    /// Creates an empty transport.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends one canned route (builder style).
    #[must_use]
    pub fn route(mut self, matcher: Match, response: HttpResponse) -> Self {
        self.rules.push((matcher, response));
        self
    }
}

#[async_trait]
impl HttpClient for CannedTransport {
    async fn execute(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
        let matched = self.rules.iter().find(|(m, _)| {
            m.method
                .as_ref()
                .is_none_or(|want| want.eq_ignore_ascii_case(&req.method))
                // Suffix matching ignores the query string.
                && req
                    .url
                    .split('?')
                    .next()
                    .unwrap_or(&req.url)
                    .ends_with(&m.path_suffix)
        });
        match matched {
            Some((_, response)) => Ok(response.clone()),
            None => Err(TransportError(format!(
                "no canned response for {} {}",
                req.method, req.url
            ))),
        }
    }
}

/// Transport replaying recorded [`CassetteEntry`] exchanges.
///
/// Simplification: entries are served strictly in cassette order and the
/// incoming request is ignored entirely (no method/URL/body matching).
/// Request-aware replay lives in the gateway crate's `ReplayIndex`.
pub struct ReplayTransport {
    entries: Vec<CassetteEntry>,
    next: AtomicUsize,
}

impl ReplayTransport {
    /// Wraps already-read cassette entries in serving order.
    #[must_use]
    pub fn new(entries: Vec<CassetteEntry>) -> Self {
        Self {
            entries,
            next: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl HttpClient for ReplayTransport {
    async fn execute(&self, _req: HttpRequest) -> Result<HttpResponse, TransportError> {
        let idx = self.next.fetch_add(1, Ordering::SeqCst);
        let Some(entry) = self.entries.get(idx) else {
            return Err(TransportError(format!(
                "cassette exhausted after {idx} entries"
            )));
        };
        Ok(HttpResponse {
            status: entry.status,
            headers: entry.response_headers.clone(),
            body: bytes::Bytes::from(entry.response_body.bytes()),
        })
    }
}
