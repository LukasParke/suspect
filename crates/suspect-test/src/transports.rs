//! Deterministic in-process [`HttpClient`]
//! transports used for testing and offline runs.
//!
//! No real network transport ships in this crate: HTTP clients arrive with
//! the CLI. These transports make plan execution fully deterministic —
//! [`CannedTransport`] matches requests against declared rules,
//! [`ReplayTransport`] answers each request from its recorded exchange, and
//! [`DelayTransport`] adds simulated latency in front of any other
//! transport (used to exercise schedulers and metrics under `tokio::time`
//! control).

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;

use suspect_journal::{CassetteEntry, key_path, request_key};

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
/// Each request is answered by the recording of *that* request, not by
/// the next line of the cassette: workflows run concurrently, so requests
/// arrive in an order the recording cannot predict. A recording matches on
/// method and [`request_key`] (path and query; scheme and host are
/// ignored, so a cassette recorded through a gateway replays against any
/// base URL). Among the recordings that match:
///
/// 1. one not yet served whose request body is the same wins,
/// 2. else the earliest not yet served, so a request repeated within a run
///    (read, delete, read again) walks its recordings in order,
/// 3. else the earliest not yet served with the same path and another
///    query,
/// 4. else the latest one already served — a re-sent request gets the
///    answer that stood last.
///
/// A request with no recording at all is a transport error naming it.
/// The gateway's `ReplayIndex` serves a long-lived process and is
/// stateless instead: there the latest recording of a request always
/// wins.
pub struct ReplayTransport {
    entries: Vec<Recording>,
    /// `served[i]` once `entries[i]` has answered a request.
    served: Mutex<Vec<bool>>,
}

/// A cassette entry with its match keys computed once.
struct Recording {
    method: String,
    key: String,
    entry: CassetteEntry,
}

impl ReplayTransport {
    /// Wraps already-read cassette entries.
    #[must_use]
    pub fn new(entries: Vec<CassetteEntry>) -> Self {
        let served = Mutex::new(vec![false; entries.len()]);
        Self {
            entries: entries
                .into_iter()
                .map(|entry| Recording {
                    method: entry.method.to_ascii_uppercase(),
                    key: request_key(&entry.url),
                    entry,
                })
                .collect(),
            served,
        }
    }

    /// The recording that answers `req`, marked served.
    fn pick(&self, req: &HttpRequest) -> Option<&CassetteEntry> {
        let method = req.method.to_ascii_uppercase();
        let key = request_key(&req.url);
        let path = key_path(&key);
        let body = suspect_journal::sha256_hex(&req.body);
        let mut served = self
            .served
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let same_request =
            |recording: &Recording| recording.method == method && recording.key == key;
        let same_path =
            |recording: &Recording| recording.method == method && key_path(&recording.key) == path;
        let fresh = |index: &usize| !served[*index];
        let indices = || 0..self.entries.len();
        let chosen = indices()
            .filter(fresh)
            .find(|index| {
                let recording = &self.entries[*index];
                same_request(recording) && recording.entry.request_body.sha256 == body
            })
            .or_else(|| {
                indices()
                    .filter(fresh)
                    .find(|index| same_request(&self.entries[*index]))
            })
            .or_else(|| {
                indices()
                    .filter(fresh)
                    .find(|index| same_path(&self.entries[*index]))
            })
            .or_else(|| {
                indices()
                    .rev()
                    .find(|index| same_request(&self.entries[*index]))
            })
            .or_else(|| {
                indices()
                    .rev()
                    .find(|index| same_path(&self.entries[*index]))
            })?;
        served[chosen] = true;
        Some(&self.entries[chosen].entry)
    }
}

#[async_trait]
impl HttpClient for ReplayTransport {
    async fn execute(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
        let Some(entry) = self.pick(&req) else {
            let count = self.entries.len();
            return Err(TransportError(format!(
                "no recorded exchange for {} {} (the cassette holds {count} recorded exchange{})",
                req.method,
                req.url,
                if count == 1 { "" } else { "s" }
            )));
        };
        Ok(HttpResponse {
            status: entry.status,
            headers: entry.response_headers.clone(),
            body: bytes::Bytes::from(entry.response_body.bytes()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use suspect_journal::Body;

    fn recorded(
        id: u64,
        method: &str,
        url: &str,
        request: &str,
        status: u16,
        response: &str,
    ) -> CassetteEntry {
        CassetteEntry {
            id,
            method: method.to_owned(),
            url: url.to_owned(),
            status,
            request_headers: Vec::new(),
            request_body: Body::from_bytes(request.as_bytes()),
            response_headers: vec![("content-type".to_owned(), "application/json".to_owned())],
            response_body: Body::from_bytes(response.as_bytes()),
            duration_ms: 1.0,
        }
    }

    async fn send(
        transport: &ReplayTransport,
        method: &str,
        url: &str,
        body: &str,
    ) -> Result<(u16, String), String> {
        transport
            .execute(HttpRequest {
                method: method.to_owned(),
                url: url.to_owned(),
                headers: Vec::new(),
                body: Bytes::from(body.to_owned()),
            })
            .await
            .map(|response| {
                (
                    response.status,
                    String::from_utf8_lossy(&response.body).into_owned(),
                )
            })
            .map_err(|error| error.to_string())
    }

    #[tokio::test]
    async fn each_request_is_answered_by_its_own_recording() {
        let transport = ReplayTransport::new(vec![
            recorded(1, "POST", "http://rec/login", "{}", 200, "token"),
            recorded(2, "GET", "http://rec/pets", "", 200, "pets"),
        ]);
        // The requests arrive in the opposite order to the recording.
        assert_eq!(
            send(&transport, "GET", "http://live/pets", "").await,
            Ok((200, "pets".to_owned()))
        );
        assert_eq!(
            send(&transport, "POST", "http://live/login", "{}").await,
            Ok((200, "token".to_owned()))
        );
    }

    #[tokio::test]
    async fn scheme_host_and_escape_case_do_not_matter() {
        let transport = ReplayTransport::new(vec![recorded(
            1,
            "GET",
            "http://127.0.0.1:18084/pets/a%2fb?limit=20",
            "",
            200,
            "page",
        )]);
        assert_eq!(
            send(
                &transport,
                "get",
                "https://api.example.com/pets/a%2Fb?limit=20",
                ""
            )
            .await,
            Ok((200, "page".to_owned()))
        );
    }

    #[tokio::test]
    async fn a_repeated_request_walks_its_recordings_in_order() {
        let transport = ReplayTransport::new(vec![
            recorded(1, "GET", "http://rec/pets/1", "", 200, "alive"),
            recorded(2, "DELETE", "http://rec/pets/1", "", 204, ""),
            recorded(3, "GET", "http://rec/pets/1", "", 404, "gone"),
        ]);
        assert_eq!(
            send(&transport, "GET", "http://live/pets/1", "").await,
            Ok((200, "alive".to_owned()))
        );
        assert_eq!(
            send(&transport, "DELETE", "http://live/pets/1", "").await,
            Ok((204, String::new()))
        );
        assert_eq!(
            send(&transport, "GET", "http://live/pets/1", "").await,
            Ok((404, "gone".to_owned()))
        );
        // Past the last recording, the latest answer stands.
        assert_eq!(
            send(&transport, "GET", "http://live/pets/1", "").await,
            Ok((404, "gone".to_owned()))
        );
    }

    #[tokio::test]
    async fn the_recording_with_the_same_body_wins() {
        let transport = ReplayTransport::new(vec![
            recorded(
                1,
                "POST",
                "http://rec/pets",
                r#"{"name":"Rex"}"#,
                201,
                "rex",
            ),
            recorded(
                2,
                "POST",
                "http://rec/pets",
                r#"{"name":"Tom"}"#,
                201,
                "tom",
            ),
        ]);
        assert_eq!(
            send(&transport, "POST", "http://live/pets", r#"{"name":"Tom"}"#).await,
            Ok((201, "tom".to_owned()))
        );
        assert_eq!(
            send(&transport, "POST", "http://live/pets", r#"{"name":"Rex"}"#).await,
            Ok((201, "rex".to_owned()))
        );
    }

    #[tokio::test]
    async fn a_different_query_falls_back_to_the_path() {
        let transport = ReplayTransport::new(vec![recorded(
            1,
            "GET",
            "http://rec/pets?limit=20",
            "",
            200,
            "page",
        )]);
        assert_eq!(
            send(&transport, "GET", "http://live/pets?limit=5", "").await,
            Ok((200, "page".to_owned()))
        );
    }

    #[tokio::test]
    async fn an_unrecorded_request_is_a_named_transport_error() {
        let transport =
            ReplayTransport::new(vec![recorded(1, "GET", "http://rec/pets", "", 200, "page")]);
        let error = send(&transport, "POST", "http://live/pets", "{}")
            .await
            .expect_err("nothing recorded for POST");
        assert!(
            error.contains("POST http://live/pets") && error.contains("1 recorded exchange"),
            "{error}"
        );
    }
}
