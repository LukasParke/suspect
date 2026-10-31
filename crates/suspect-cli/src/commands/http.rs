//! Shared live-HTTP plumbing for CLI commands that talk to a real server
//! (`test run`, `test fuzz`, `test replay`, `test load`). The core crates stay dependency-free;
//! this is the single CLI-side [`HttpClient`] implementation.

use std::time::{Duration, Instant};

use suspect_test::{ErrorClass, HttpClient, HttpRequest, HttpResponse, TransportError};

/// Connection settings for a [`LiveTransport`].
#[derive(Debug, Clone)]
pub struct HttpSettings {
    /// Whole-request timeout (connect + headers + body).
    pub timeout: Duration,
    /// Negotiate HTTP/2 via ALPN when the server offers it.
    pub http2: bool,
    /// Keep a cookie jar across requests (one per transport instance).
    pub cookies: bool,
    /// Accept invalid/self-signed TLS certificates.
    pub insecure: bool,
    /// Maximum idle pooled connections per host.
    pub pool_max_idle_per_host: Option<usize>,
    /// `User-Agent` header.
    pub user_agent: String,
}

impl Default for HttpSettings {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            http2: true,
            cookies: false,
            insecure: false,
            pool_max_idle_per_host: None,
            user_agent: format!("suspect/{}", env!("CARGO_PKG_VERSION")),
        }
    }
}

/// Live HTTP transport backed by `reqwest`; the CLI-side implementation of
/// [`HttpClient`] (the core crate stays dependency-free).
pub struct LiveTransport {
    client: reqwest::Client,
}

impl LiveTransport {
    /// Builds a transport whose requests time out after `timeout`, with all
    /// other settings at their defaults.
    ///
    /// # Errors
    /// Propagates `reqwest` client-construction failures.
    pub fn new(timeout: Duration) -> reqwest::Result<Self> {
        Self::with_settings(&HttpSettings {
            timeout,
            ..HttpSettings::default()
        })
    }

    /// Builds a transport from explicit settings.
    ///
    /// # Errors
    /// Propagates `reqwest` client-construction failures.
    pub fn with_settings(settings: &HttpSettings) -> reqwest::Result<Self> {
        let mut builder = reqwest::Client::builder()
            .timeout(settings.timeout)
            .user_agent(settings.user_agent.clone())
            .tcp_nodelay(true)
            .cookie_store(settings.cookies)
            .danger_accept_invalid_certs(settings.insecure);
        if !settings.http2 {
            builder = builder.http1_only();
        }
        if let Some(max) = settings.pool_max_idle_per_host {
            builder = builder.pool_max_idle_per_host(max);
        }
        Ok(Self {
            client: builder.build()?,
        })
    }

    fn classify(e: &reqwest::Error) -> TransportError {
        let class = if e.is_timeout() {
            ErrorClass::Timeout
        } else if e.is_connect() {
            ErrorClass::Connect
        } else {
            // Walk the source chain for TLS / reset signatures.
            let mut text = e.to_string();
            let mut src = std::error::Error::source(e);
            while let Some(s) = src {
                text.push_str(" | ");
                text.push_str(&s.to_string());
                src = s.source();
            }
            TransportError(text).class()
        };
        TransportError::classified(class, e)
    }

    async fn send(
        &self,
        req: HttpRequest,
    ) -> Result<(HttpResponse, Option<Duration>), TransportError> {
        let method = reqwest::Method::from_bytes(req.method.as_bytes())
            .map_err(|e| TransportError(format!("bad method {}: {e}", req.method)))?;
        let mut builder = self.client.request(method, &req.url);
        for (k, v) in &req.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }
        if !req.body.is_empty() {
            builder = builder.body(req.body.clone());
        }
        let started = Instant::now();
        let resp = builder.send().await.map_err(|e| Self::classify(&e))?;
        let ttfb = started.elapsed();
        let status = resp.status().as_u16();
        let headers: Vec<(String, String)> = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str().to_owned(), v.to_str().unwrap_or("").to_owned()))
            .collect();
        let body = resp.bytes().await.map_err(|e| Self::classify(&e))?;
        Ok((
            HttpResponse {
                status,
                headers,
                body,
            },
            Some(ttfb),
        ))
    }
}

#[async_trait::async_trait]
impl HttpClient for LiveTransport {
    async fn execute(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
        self.send(req).await.map(|(resp, _)| resp)
    }

    async fn execute_timed(
        &self,
        req: HttpRequest,
    ) -> Result<(HttpResponse, Option<Duration>), TransportError> {
        self.send(req).await
    }
}
