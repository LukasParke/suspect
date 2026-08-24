//! Proxy transport, contract validation, and cassette recording.
//!
//! Proxy/Validate/Record modes forward each request to an upstream over a
//! fresh hyper HTTP/1.1 connection (simple and stateless; the gateway is
//! not a load-balancing reverse proxy). Validate mode additionally runs
//! structural checks — required fields present, JSON types matching,
//! enums respected — against the operation's parameter and body schemas on
//! the way in and the declared response schema on the way out.
//!
//! **Response violations are journaled but passed through unchanged**: the
//! gateway observes upstream drift, it does not rewrite upstream traffic.
//! Request-side violations in `enforce` mode short-circuit with `400`
//! problem+json before anything reaches the upstream.

use std::io::Write as _;
use std::path::Path;
use std::time::Duration;

use axum::response::IntoResponse;
use bytes::Bytes;
use http_body_util::{BodyExt, Full, LengthLimitError, Limited};
use suspect_ir::{IrOperation, ParamIn};
use suspect_journal::{
    CASSETTE_FORMAT, CASSETTE_VERSION, CassetteEntry, CassetteHeader, Journal, Violation,
};
use tokio::io::{AsyncRead as _, AsyncWrite as _};

use crate::{mock, problem};
/// Maximum proxied request/response body size (32 MiB).
pub(crate) const MAX_BODY: usize = 32 * 1024 * 1024;

/// Wall-clock budget for each upstream connect/handshake/send/read step.
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(30);

/// Headers that must not be copied onto proxied requests verbatim.
fn is_hop_by_hop_request(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "host" | "content-length" | "connection" | "transfer-encoding" | "upgrade"
    )
}

/// Headers that must not be copied onto gateway responses verbatim.
fn is_hop_by_hop_response(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "content-length" | "transfer-encoding" | "connection"
    )
}

/// Parses `http://host[:port]` into connectable parts.
///
/// TLS upstreams are rejected explicitly rather than silently downgrade.
pub(crate) fn parse_upstream(upstream: &str) -> Result<(String, u16), String> {
    let rest = if let Some(r) = upstream.strip_prefix("http://") {
        r
    } else if upstream.starts_with("https://") {
        return Err("https upstreams are not supported by the gateway proxy".to_owned());
    } else {
        upstream
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    if authority.is_empty() {
        return Err(format!("upstream `{upstream}` has no host"));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) => port
            .parse::<u16>()
            .map(|p| (host.to_owned(), p))
            .map_err(|_| format!("invalid port in upstream `{upstream}`")),
        None => Ok((authority.to_owned(), 80)),
    }
}

/// Bridges a tokio TCP stream to hyper 1.x runtime traits.
///
/// hyper 1 moved its tokio adapters out to `hyper-util`; this tiny adapter
/// keeps the dependency footprint to plain `hyper`.
struct TokioStream(tokio::net::TcpStream);

impl hyper::rt::Read for TokioStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        mut buf: hyper::rt::ReadBufCursor<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        // SAFETY: hyper hands out a `ReadBufCursor` over memory it owns and
        // promises the region is writable; this mirrors hyper-util's TokioIo.
        let mut tbuf = tokio::io::ReadBuf::uninit(unsafe { buf.as_mut() });
        match std::pin::Pin::new(&mut self.get_mut().0).poll_read(cx, &mut tbuf) {
            std::task::Poll::Ready(Ok(())) => {
                let filled = tbuf.filled().len();
                unsafe { buf.advance(filled) };
                std::task::Poll::Ready(Ok(()))
            }
            std::task::Poll::Ready(Err(err)) => std::task::Poll::Ready(Err(err)),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

impl hyper::rt::Write for TokioStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        std::pin::Pin::new(&mut self.get_mut().0).poll_write(cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        std::pin::Pin::new(&mut self.get_mut().0).poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        std::pin::Pin::new(&mut self.get_mut().0).poll_shutdown(cx)
    }
}

/// One upstream exchange reduced to wire-level facts.
#[derive(Debug, Clone)]
pub(crate) struct UpstreamReply {
    /// Response status code.
    pub status: u16,
    /// Response headers in wire order (hop-by-hop framing stripped).
    pub headers: Vec<(String, String)>,
    /// Response body bytes.
    pub body: Bytes,
}

/// Forwards one exchange to the upstream over a fresh HTTP/1.1 connection.
pub(crate) async fn fetch_upstream(
    upstream: &str,
    method: &str,
    url_path_and_query: &str,
    headers: &[(String, String)],
    body: Bytes,
) -> Result<UpstreamReply, String> {
    let (host, port) = parse_upstream(upstream)?;
    let stream = tokio::time::timeout(
        UPSTREAM_TIMEOUT,
        tokio::net::TcpStream::connect((host.as_str(), port)),
    )
    .await
    .map_err(|_| format!("connect {host}:{port} timed out after 30s"))?
    .map_err(|e| format!("connect {host}:{port} failed: {e}"))?;
    let (mut sender, conn) = tokio::time::timeout(
        UPSTREAM_TIMEOUT,
        hyper::client::conn::http1::handshake(TokioStream(stream)),
    )
    .await
    .map_err(|_| "upstream handshake timed out after 30s".to_owned())?
    .map_err(|e| format!("upstream handshake failed: {e}"))?;
    // Drive the connection to completion alongside the request.
    tokio::spawn(async move {
        let _ = conn.await;
    });

    // hyper's HTTP/1.1 client does not synthesize `Host`; the upstream
    // needs the origin-form request target plus an explicit authority.
    let authority = if port == 80 {
        host.clone()
    } else {
        format!("{host}:{port}")
    };
    let mut builder = hyper::http::Request::builder()
        .method(method)
        .uri(url_path_and_query)
        .header("host", authority);
    for (name, value) in headers {
        if !is_hop_by_hop_request(name) {
            builder = builder.header(name.as_str(), value.as_str());
        }
    }
    let request = builder
        .body(Full::new(body.clone()))
        .map_err(|e| format!("build upstream request failed: {e}"))?;
    let response = tokio::time::timeout(UPSTREAM_TIMEOUT, sender.send_request(request))
        .await
        .map_err(|_| "upstream send timed out after 30s".to_owned())?
        .map_err(|e| format!("upstream send failed: {e}"))?;

    let status = response.status().as_u16();
    let reply_headers = response
        .headers()
        .iter()
        .filter(|(name, _)| !is_hop_by_hop_response(name.as_str()))
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect::<Vec<_>>();
    let reply_body = tokio::time::timeout(
        UPSTREAM_TIMEOUT,
        Limited::new(response.into_body(), MAX_BODY).collect(),
    )
    .await
    .map_err(|_| "read upstream body timed out after 30s".to_owned())?
    .map_err(|err| {
        if err.is::<LengthLimitError>() {
            format!(
                "upstream response body exceeds the {} MiB limit",
                MAX_BODY / (1024 * 1024)
            )
        } else {
            format!("read upstream body failed: {err}")
        }
    })?
    .to_bytes();

    Ok(UpstreamReply {
        status,
        headers: reply_headers,
        body: reply_body,
    })
}

/// Converts an [`UpstreamReply`] into a served axum response.
#[must_use]
pub(crate) fn reply_to_response(reply: &UpstreamReply) -> axum::response::Response {
    let status = axum::http::StatusCode::from_u16(reply.status)
        .unwrap_or(axum::http::StatusCode::BAD_GATEWAY);
    let mut builder = axum::http::Response::builder().status(status);
    for (name, value) in &reply.headers {
        builder = builder.header(name.as_str(), value.as_str());
    }
    builder
        .body(axum::body::Body::from(reply.body.clone()))
        .unwrap_or_else(|_| {
            problem(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Internal error",
                None,
            )
        })
}

/// Proxy mode: forward and return; never rewrites upstream responses.
pub(crate) async fn forward(
    upstream: &str,
    method: &str,
    url_path_and_query: &str,
    headers: &[(String, String)],
    body: Bytes,
) -> (axum::response::Response, Vec<Violation>) {
    match fetch_upstream(upstream, method, url_path_and_query, headers, body).await {
        Ok(reply) => (reply_to_response(&reply), Vec::new()),
        Err(err) => (
            problem(
                axum::http::StatusCode::BAD_GATEWAY,
                "Bad gateway",
                Some(err),
            ),
            Vec::new(),
        ),
    }
}

// ------------------------------------------------------------- validation

/// Resolves a local `$ref` through the schema map (depth-guarded).
fn resolve<'a>(
    schema: &'a serde_json::Value,
    refs: &'a mock::SchemaRefs,
    depth: u8,
) -> &'a serde_json::Value {
    if depth > mock::DEPTH_CAP {
        return &serde_json::Value::Null;
    }
    match schema.get("$ref").and_then(serde_json::Value::as_str) {
        Some(target) => {
            let name = target.rsplit('/').next().unwrap_or(target);
            refs.get(name)
                .map_or(schema, |next| resolve(next, refs, depth + 1))
        }
        None => schema,
    }
}

/// Structural type check of one JSON value against one schema.
fn check_value(
    value: &serde_json::Value,
    schema: &serde_json::Value,
    refs: &mock::SchemaRefs,
    pointer: &str,
    out: &mut Vec<Violation>,
) {
    let schema = resolve(schema, refs, 0);
    if let Some(serde_json::Value::Array(allowed)) = schema.get("enum")
        && !allowed.contains(value)
    {
        out.push(Violation {
            message: format!("value {} is not one of the enum values", value),
            pointer: pointer.to_owned(),
        });
    }
    let expected = schema
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    let ok = match expected {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.is_i64() || value.is_u64(),
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        _ => true,
    };
    if !ok {
        out.push(Violation {
            message: format!("expected {expected}, got {}", kind_of(value)),
            pointer: pointer.to_owned(),
        });
        return;
    }
    match (expected, value) {
        ("object", serde_json::Value::Object(map)) => {
            if let Some(required) = schema.get("required").and_then(serde_json::Value::as_array) {
                for name in required {
                    if let Some(key) = name.as_str()
                        && !map.contains_key(key)
                    {
                        out.push(Violation {
                            message: format!("missing required property `{key}`"),
                            pointer: format!("{pointer}/{key}"),
                        });
                    }
                }
            }
            if let Some(props) = schema
                .get("properties")
                .and_then(serde_json::Value::as_object)
            {
                for (key, prop_schema) in props {
                    if let Some(child) = map.get(key) {
                        check_value(child, prop_schema, refs, &format!("{pointer}/{key}"), out);
                    }
                }
            }
        }
        ("array", serde_json::Value::Array(items)) => {
            if let Some(item_schema) = schema.get("items") {
                for (idx, item) in items.iter().enumerate() {
                    check_value(item, item_schema, refs, &format!("{pointer}/{idx}"), out);
                }
            }
        }
        _ => {}
    }
}

/// Human-readable JSON kind for violation messages.
fn kind_of(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

/// Minimal percent-decoding of query components (`%XX` and `+`→space).
fn percent_decode_query(text: &str) -> String {
    percent_decode_impl(text, true)
}

/// Percent-decoding of path segments (`%XX` only; `+` is literal in
/// paths, only the query grammar treats it as an encoded space).
fn percent_decode_path(text: &str) -> String {
    percent_decode_impl(text, false)
}

/// Shared decoder core; `plus_as_space` selects the query grammar.
fn percent_decode_impl(text: &str, plus_as_space: bool) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut idx = 0;
    while idx < bytes.len() {
        match bytes[idx] {
            b'+' if plus_as_space => {
                out.push(b' ');
                idx += 1;
            }
            b'%' if idx + 2 < bytes.len() => {
                let hi = (bytes[idx + 1] as char).to_digit(16);
                let lo = (bytes[idx + 2] as char).to_digit(16);
                match (hi, lo) {
                    (Some(hi), Some(lo)) => {
                        out.push((hi * 16 + lo) as u8);
                        idx += 3;
                    }
                    _ => {
                        out.push(bytes[idx]);
                        idx += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                idx += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// First query-string value for `name`, if present.
fn query_lookup(query: Option<&str>, name: &str) -> Option<String> {
    let query = query?;
    for pair in query.split('&') {
        let (key, value) = match pair.split_once('=') {
            Some((k, v)) => (k, v),
            None => (pair, ""),
        };
        if percent_decode_query(key) == name {
            return Some(percent_decode_query(value));
        }
    }
    None
}

/// Validates a scalar parameter string against its declared schema.
fn check_scalar(
    raw: &str,
    schema: &serde_json::Value,
    refs: &mock::SchemaRefs,
    pointer: &str,
    out: &mut Vec<Violation>,
) {
    let resolved = resolve(schema, refs, 0);
    let expected = resolved
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("string");
    let value = match expected {
        "integer" => raw.parse::<i64>().map_or_else(
            |_| serde_json::Value::String(raw.to_owned()),
            |n| serde_json::json!(n),
        ),
        "number" => raw.parse::<f64>().map_or_else(
            |_| serde_json::Value::String(raw.to_owned()),
            |n| serde_json::json!(n),
        ),
        "boolean" => match raw {
            "true" => serde_json::Value::Bool(true),
            "false" => serde_json::Value::Bool(false),
            _ => serde_json::Value::String(raw.to_owned()),
        },
        _ => serde_json::Value::String(raw.to_owned()),
    };
    check_value(&value, resolved, refs, pointer, out);
}

/// Extracts `{param}` values by aligning template and actual segments.
fn path_param_values<'t>(
    template: &'t str,
    actual_path: &str,
) -> impl Iterator<Item = (&'t str, String)> {
    let t_segs: Vec<_> = template.split('/').collect();
    let a_segs: Vec<_> = actual_path.split('/').collect();
    t_segs.into_iter().zip(a_segs).filter_map(|(t, a)| {
        let name = t.strip_prefix('{').and_then(|n| n.strip_suffix('}'))?;
        Some((name, percent_decode_path(a)))
    })
}

/// Validates the incoming request against the operation contract.
///
/// Checks every parameter that carries a schema (query/header/path) plus
/// the JSON request body when the operation declares one. Cookie
/// parameters are skipped (the gateway parses no cookie jar). An operation
/// with a declared body schema treats an empty or non-JSON body as a
/// violation (`requestBody` is required unless explicitly optional, which
/// the IR does not model separately).
fn validate_request(
    op: &IrOperation,
    refs: &mock::SchemaRefs,
    url_path_and_query: &str,
    headers: &[(String, String)],
    body: &[u8],
    out: &mut Vec<Violation>,
) {
    let (actual_path, query) = match url_path_and_query.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (url_path_and_query, None),
    };

    for param in &op.parameters {
        let Some(schema) = &param.schema else {
            continue;
        };
        match param.location {
            ParamIn::Query => match query_lookup(query, &param.name) {
                Some(raw) => {
                    check_scalar(&raw, schema, refs, &format!("/{}", param.name), out);
                }
                None if param.required => out.push(Violation {
                    message: format!("missing required query parameter `{}`", param.name),
                    pointer: format!("/{}", param.name),
                }),
                None => {}
            },
            ParamIn::Header => {
                let found = headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(&param.name));
                match found {
                    Some((_, value)) => {
                        check_scalar(value, schema, refs, &format!("/{}", param.name), out);
                    }
                    None if param.required => out.push(Violation {
                        message: format!("missing required header `{}`", param.name),
                        pointer: format!("/{}", param.name),
                    }),
                    None => {}
                }
            }
            ParamIn::Path => {
                // Routing already proved presence; still type-check values.
                if let Some((_, value)) =
                    path_param_values(&op.path, actual_path).find(|(name, _)| *name == param.name)
                {
                    check_scalar(&value, schema, refs, &format!("/{}", param.name), out);
                }
            }
            ParamIn::Cookie => {}
        }
    }

    if let Some(component) = &op.body_schema {
        if body.is_empty() {
            out.push(Violation {
                message: "required request body is missing".to_owned(),
                pointer: "/body".to_owned(),
            });
        } else {
            match serde_json::from_slice::<serde_json::Value>(body) {
                Ok(parsed) => {
                    if let Some(schema) = refs.get(component.as_str()) {
                        check_value(&parsed, schema, refs, "", out);
                    }
                }
                Err(err) => out.push(Violation {
                    message: format!("request body is not valid JSON: {err}"),
                    pointer: "/body".to_owned(),
                }),
            }
        }
    }
}

/// Borrowed view of an incoming request shared by proxying modes.
pub(crate) struct ForwardCtx<'a> {
    /// HTTP method.
    pub method: &'a str,
    /// Path plus query as received.
    pub target: &'a str,
    /// Request headers.
    pub headers: &'a [(String, String)],
}

/// Validate mode: optionally enforce on the way in, observe on the way out.
///
/// With `enforce`, request violations short-circuit as `400` problem+json
/// carrying a `violations` array. Response violations are always journaled
/// but never alter the upstream response.
pub(crate) async fn validate_forward(
    upstream: &str,
    op: &IrOperation,
    refs: &mock::SchemaRefs,
    ctx: ForwardCtx<'_>,
    body: Bytes,
    enforce: bool,
) -> (axum::response::Response, Vec<Violation>) {
    let mut violations = Vec::new();
    validate_request(op, refs, ctx.target, ctx.headers, &body, &mut violations);

    if enforce && !violations.is_empty() {
        let detail = serde_json::json!({
            "title": "Request failed validation",
            "status": 400,
            "violations": violations.iter().map(|v| serde_json::json!({
                "message": v.message,
                "pointer": v.pointer,
            })).collect::<Vec<_>>(),
        })
        .to_string();
        return (
            (
                axum::http::StatusCode::BAD_REQUEST,
                [("content-type", "application/problem+json")],
                detail,
            )
                .into_response(),
            violations,
        );
    }

    let reply = match fetch_upstream(upstream, ctx.method, ctx.target, ctx.headers, body).await {
        Ok(reply) => reply,
        Err(err) => {
            return (
                problem(
                    axum::http::StatusCode::BAD_GATEWAY,
                    "Bad gateway",
                    Some(err),
                ),
                violations,
            );
        }
    };

    // Response side: validate only when the status matches a declared
    // response whose schema is locally resolvable. Passed through unchanged.
    if let Some(declared) = op.responses.iter().find(|r| r.status == Some(reply.status))
        && let Some(component) = &declared.schema
        && let Some(schema) = refs.get(component.as_str())
        && let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(&reply.body)
    {
        check_value(&parsed, schema, refs, "", &mut violations);
    }

    (reply_to_response(&reply), violations)
}

// ---------------------------------------------------------------- record

/// Append-only cassette writer used by Record mode.
///
/// Wraps a [`std::fs::File`]; the header line is written lazily on the
/// first entry so an aborted recording leaves no misleadingly empty
/// cassette. Entry ids are sequential starting at 1, exactly what
/// `suspect_journal::read_cassette` validates.
///
/// After the first write failure the appender is **sticky-poisoned**:
/// every subsequent [`CassetteAppender::append`] fails immediately, so a
/// broken sink can never interleave partial entries and poison the whole
/// file. The failed entry's id is not consumed.
pub struct CassetteAppender {
    file: std::fs::File,
    next_id: u64,
    wrote_header: bool,
    poisoned: bool,
    source: String,
}

impl CassetteAppender {
    /// Creates (or truncates) the cassette file at `path`.
    ///
    /// # Errors
    /// Propagates file-creation errors.
    pub fn create(path: &Path, source: String) -> std::io::Result<Self> {
        Ok(Self {
            file: std::fs::File::create(path)?,
            next_id: 1,
            wrote_header: false,
            poisoned: false,
            source,
        })
    }

    /// Whether a previous append failed and the writer refuses further
    /// entries.
    #[must_use]
    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    /// Appends one exchange, writing the header line first when needed.
    ///
    /// The entry is fully serialized (id tentatively assigned from
    /// [`CassetteAppender`]'s sequence) before any bytes are written; the
    /// id counter only advances after the line hits the file successfully.
    ///
    /// # Errors
    /// Propagates serialization or I/O errors. The first failure poisons
    /// the writer; all later appends fail immediately with
    /// [`std::io::ErrorKind::Other`] until the recorder is recreated.
    pub fn append(&mut self, mut entry: CassetteEntry) -> std::io::Result<()> {
        if self.poisoned {
            return Err(std::io::Error::other(
                "cassette writer poisoned by an earlier write failure",
            ));
        }
        let result = self.append_inner(&mut entry);
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }

    fn append_inner(&mut self, entry: &mut CassetteEntry) -> std::io::Result<()> {
        if !self.wrote_header {
            let header = CassetteHeader {
                format: CASSETTE_FORMAT.to_owned(),
                version: CASSETTE_VERSION,
                recorded_at_ms: Journal::now_ms(),
                source: self.source.clone(),
            };
            writeln!(
                self.file,
                "{}",
                serde_json::to_string(&header)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?
            )?;
            self.wrote_header = true;
        }
        entry.id = self.next_id;
        let line = serde_json::to_string(entry)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        writeln!(self.file, "{line}")?;
        self.file.flush()?;
        // Commit the id only once the line is durably written.
        self.next_id += 1;
        Ok(())
    }
}
#[cfg(test)]
mod append_tests {
    use super::*;

    fn sample_entry() -> CassetteEntry {
        CassetteEntry {
            id: 0,
            method: "GET".to_owned(),
            url: "http://x/y".to_owned(),
            status: 200,
            request_headers: vec![],
            request_body: suspect_journal::Body::from_bytes(b""),
            response_headers: vec![],
            response_body: suspect_journal::Body::from_bytes(b"{}"),
            duration_ms: 1.0,
        }
    }

    #[test]
    fn io_failure_poisons_writer_and_preserves_id_sequence() {
        let dir = tempfile::tempdir().expect("tempdir");

        // A read-only backing file makes every write fail (EBADF).
        let bad_path = dir.path().join("bad.cassette");
        let mut broken = CassetteAppender::create(&bad_path, "t".to_owned()).expect("create");
        broken.file = std::fs::File::open(&bad_path).expect("reopen read-only");
        assert!(!broken.is_poisoned());

        assert!(broken.append(sample_entry()).is_err());
        assert!(broken.is_poisoned(), "first write failure must poison");
        assert_eq!(
            broken.next_id, 1,
            "failed append must not consume the entry id"
        );

        // Sticky: every subsequent append is refused outright, so no
        // partial entries can interleave into the cassette.
        assert!(broken.append(sample_entry()).is_err());

        // A healthy writer keeps ids strictly sequential.
        let ok_path = dir.path().join("ok.cassette");
        let mut good = CassetteAppender::create(&ok_path, "t".to_owned()).expect("create");
        good.append(sample_entry()).expect("append 1");
        good.append(sample_entry()).expect("append 2");
        let (_, entries) =
            suspect_journal::read_cassette(std::fs::File::open(&ok_path).expect("open"))
                .expect("readable");
        assert_eq!(entries.iter().map(|e| e.id).collect::<Vec<_>>(), vec![1, 2]);
    }
}
