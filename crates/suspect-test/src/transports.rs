//! Deterministic in-process [`HttpClient`]
//! transports used for testing and offline runs.
//!
//! No real network transport ships in this crate: HTTP clients arrive with
//! the CLI. These transports make plan execution fully deterministic —
//! [`CannedTransport`] matches requests against declared rules,
//! [`ReplayTransport`] serves recorded cassette entries, and
//! [`DelayTransport`] adds simulated latency in front of any other
//! transport (used to exercise schedulers and metrics under `tokio::time`
//! control).

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;

use suspect_journal::CassetteEntry;

use crate::exec::{HttpClient, HttpRequest, HttpResponse, TransportError};

/// Decorator adding a deterministic sleep before delegating to `inner`.
///
/// With `jitter` set, successive requests cycle through
/// `base, base + jitter/n, ...` so a sequence of calls fans out across the
/// range reproducibly (no RNG). The sleep uses `tokio::time`, so tests can
/// `pause()`/`advance()` the clock.
pub struct DelayTransport<T> {
    inner: T,
    base: Duration,
    jitter: Duration,
    calls: AtomicU64,
}

impl<T> DelayTransport<T> {
    /// Wraps `inner` with a fixed delay.
    #[must_use]
    pub fn new(inner: T, delay: Duration) -> Self {
        Self {
            inner,
            base: delay,
            jitter: Duration::ZERO,
            calls: AtomicU64::new(0),
        }
    }

    /// Adds up to `jitter` on top of the base delay, spread across calls.
    #[must_use]
    pub fn with_jitter(mut self, jitter: Duration) -> Self {
        self.jitter = jitter;
        self
    }

    /// Number of requests seen so far.
    #[must_use]
    pub fn calls(&self) -> u64 {
        self.calls.load(Ordering::SeqCst)
    }

    fn delay_for(&self, call: u64) -> Duration {
        if self.jitter.is_zero() {
            return self.base;
        }
        // Eight-slot cycle: 0/8, 1/8, ..., 7/8 of the jitter range.
        let slot = call % 8;
        self.base + self.jitter.mul_f64(slot as f64 / 8.0)
    }
}

#[async_trait]
impl<T: HttpClient> HttpClient for DelayTransport<T> {
    async fn execute(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.delay_for(call)).await;
        self.inner.execute(req).await
    }
}

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
