//! `suspect har`: export recorded traffic as an HTTP Archive (HAR 1.2),
//! the interchange format every profiling and debugging tool reads.
//!
//! Accepts the same traffic files `suspect drift` does — a Suspect
//! Cassette (gateway `--mode record` output, with bodies) or a Suspect
//! Journal (gateway `--journal` / `suspect test` output, without bodies)
//! — recognized line by line.

use std::path::Path;

use suspect_journal::BodyEncoding;

use super::traffic_file::{iso8601, read_traffic_lines};

/// `suspect har <TRAFFIC> [--output FILE]`.
///
/// # Errors
/// IO or parse failures on the traffic file, or writing the archive.
pub fn har(traffic: &Path, output: Option<&Path>) -> anyhow::Result<i32> {
    let lines = read_traffic_lines(traffic)?;
    let entries: Vec<HarEntry> = lines
        .into_iter()
        .map(|l| HarEntry {
            method: l.method,
            url: l.url,
            status: l.status,
            request_headers: l.request_headers,
            response_headers: l.response_headers,
            response_body: l.response_body,
            duration_ms: l.duration_ms,
            ts_ms: l.ts_ms,
        })
        .collect();
    let archive = build_archive(&entries);

    let text = serde_json::to_string_pretty(&archive)?;
    match output {
        Some(path) => std::fs::write(path, format!("{text}\n"))?,
        None => println!("{text}"),
    }
    Ok(0)
}

/// One recorded exchange in the shape the HAR builder needs.
pub struct HarEntry {
    /// Uppercase HTTP method.
    pub method: String,
    /// Full request URL.
    pub url: String,
    /// Response status (0 when unknown).
    pub status: u16,
    /// Request headers (name, value), already redacted.
    pub request_headers: Vec<(String, String)>,
    /// Response headers (name, value), already redacted.
    pub response_headers: Vec<(String, String)>,
    /// Response body content when recorded, with its encoding.
    pub response_body: Option<(BodyEncoding, String)>,
    /// Exchange duration in milliseconds.
    pub duration_ms: f64,
    /// Completion time, Unix epoch milliseconds.
    pub ts_ms: u64,
}

/// Builds the HAR document.
#[must_use]
pub fn build_archive(entries: &[HarEntry]) -> serde_json::Value {
    let entries: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            let content = match &e.response_body {
                Some((BodyEncoding::Utf8, text)) => serde_json::json!({
                    "size": text.len(),
                    "mimeType": content_type(&e.response_headers),
                    "text": text,
                }),
                Some((BodyEncoding::Base64, b64)) => serde_json::json!({
                    "size": b64.len(),
                    "mimeType": content_type(&e.response_headers),
                    "text": b64,
                    "encoding": "base64",
                }),
                None => serde_json::json!({
                    "size": 0,
                    "mimeType": content_type(&e.response_headers),
                }),
            };
            serde_json::json!({
                "startedDateTime": iso8601(e.ts_ms),
                "time": e.duration_ms,
                "request": {
                    "method": e.method,
                    "url": e.url,
                    "httpVersion": "HTTP/1.1",
                    "cookies": [],
                    "headers": pairs(&e.request_headers),
                    "queryString": query_pairs(&e.url),
                    "headersSize": -1,
                    "bodySize": -1,
                },
                "response": {
                    "status": e.status,
                    "statusText": "",
                    "httpVersion": "HTTP/1.1",
                    "cookies": [],
                    "headers": pairs(&e.response_headers),
                    "content": content,
                    "redirectURL": "",
                    "headersSize": -1,
                    "bodySize": -1,
                },
                "cache": {},
                "timings": { "send": -1, "wait": e.duration_ms, "receive": -1 },
            })
        })
        .collect();
    serde_json::json!({
        "log": {
            "version": "1.2",
            "creator": { "name": "suspect", "version": env!("CARGO_PKG_VERSION") },
            "entries": entries,
        }
    })
}

/// Header pairs in HAR shape.
fn pairs(headers: &[(String, String)]) -> Vec<serde_json::Value> {
    headers
        .iter()
        .map(|(name, value)| serde_json::json!({ "name": name, "value": value }))
        .collect()
}

/// Query pairs from the URL, in HAR shape.
fn query_pairs(url: &str) -> Vec<serde_json::Value> {
    url.split_once('?').map_or(Vec::new(), |(_, query)| {
        query
            .split('&')
            .filter(|p| !p.is_empty())
            .map(|pair| match pair.split_once('=') {
                Some((name, value)) => {
                    serde_json::json!({ "name": name, "value": value })
                }
                None => serde_json::json!({ "name": pair, "value": "" }),
            })
            .collect()
    })
}

/// The response's declared content type, when recorded.
fn content_type(headers: &[(String, String)]) -> String {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.clone())
        .unwrap_or_default()
}
