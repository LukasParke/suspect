//! Shared live-HTTP plumbing for CLI commands that talk to a real server
//! (`test`, `fuzz`, `replay`). The core crates stay dependency-free; this is
//! the single CLI-side [`HttpClient`] implementation.

use suspect_test::{HttpClient, HttpRequest, HttpResponse, TransportError};

/// Live HTTP transport backed by `reqwest`; the CLI-side implementation of
/// [`HttpClient`] (the core crate stays dependency-free).
pub struct LiveTransport {
    client: reqwest::Client,
}

impl LiveTransport {
    /// Builds a transport whose requests time out after `timeout`.
    ///
    /// # Errors
    /// Propagates `reqwest` client-construction failures.
    pub fn new(timeout: std::time::Duration) -> reqwest::Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder().timeout(timeout).build()?,
        })
    }
}

#[async_trait::async_trait]
impl HttpClient for LiveTransport {
    async fn execute(&self, req: HttpRequest) -> Result<HttpResponse, TransportError> {
        let method = reqwest::Method::from_bytes(req.method.as_bytes())
            .map_err(|e| TransportError(format!("bad method {}: {e}", req.method)))?;
        let mut builder = self.client.request(method, &req.url);
        for (k, v) in &req.headers {
            builder = builder.header(k.as_str(), v.as_str());
        }
        if !req.body.is_empty() {
            builder = builder.body(req.body.clone());
        }
        let resp = builder
            .send()
            .await
            .map_err(|e| TransportError(e.to_string()))?;
        let status = resp.status().as_u16();
        let headers: Vec<(String, String)> = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str().to_owned(), v.to_str().unwrap_or("").to_owned()))
            .collect();
        let body = resp
            .bytes()
            .await
            .map_err(|e| TransportError(e.to_string()))?;
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}
