//! HTTP contract gateway for the suspect platform.
//!
//! One server, five [`Mode`]s over the same spec-driven router:
//!
//! - **Mock** — serves deterministic examples synthesized from the IR at
//!   startup (see [`mock`]).
//! - **Proxy** — forwards to an upstream unchanged (see [`proxy`]).
//! - **Validate** — proxies plus structural request/response validation;
//!   response violations are journaled but passed through unchanged.
//! - **Record** — proxies and appends every exchange to a Suspect Cassette.
//! - **Replay** — serves a cassette back (see [`replay`]).
//!
//! Every served exchange emits one journal traffic record with correlation
//! id `gw/<seq>`. Fault injection (delay + error) is available in all
//! modes except replay and uses a **hash-based deterministic roll**
//! (`hash(method, path+query) % 100 < pct`) instead of an RNG: injected
//! faults must be reproducible run-to-run so failing tests can be
//! debugged offline.
//!
//! Unknown paths get `404` problem+json `{"title":"Operation not found"}`;
//! known paths with an undeclared method get a journaled `405`
//! problem+json.
#![deny(missing_docs)]

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{MatchedPath, Request as AxumRequest, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use suspect_ir::{IrSpec, Method};

use suspect_journal::{Journal, Level, Redactor, TrafficRecord, Verdict};

pub mod bridge;
pub mod mock;
pub mod playground;
pub mod proxy;
pub mod replay;
pub mod scenario;

#[cfg(test)]
mod tests;

pub use replay::ReplayIndex;

/// Address the gateway binds.
const HOST: &str = "127.0.0.1";

/// Gateway operating mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// Serve synthesized examples from the spec; no upstream traffic.
    Mock,
    /// Forward every request to `upstream` (`http://host[:port]`).
    Proxy {
        /// Upstream base authority.
        upstream: String,
    },
    /// Forward to `upstream`, validating requests and responses against
    /// the operation contract.
    Validate {
        /// Upstream base authority.
        upstream: String,
        /// When `true`, invalid requests are rejected with `400` before
        /// reaching the upstream; when `false` they are only journaled.
        enforce: bool,
    },
    /// Forward to `upstream` and append each exchange to `cassette`.
    Record {
        /// Upstream base authority.
        upstream: String,
        /// Cassette file to create and append to.
        cassette: PathBuf,
    },
    /// Serve previously recorded exchanges from `cassette`.
    Replay {
        /// Cassette file to load at startup.
        cassette: PathBuf,
    },
}

impl Mode {
    /// Short lowercase name used in journal metadata.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Mode::Mock => "mock",
            Mode::Proxy { .. } => "proxy",
            Mode::Validate { .. } => "validate",
            Mode::Record { .. } => "record",
            Mode::Replay { .. } => "replay",
        }
    }
}

/// Deterministic fault injection knobs.
/// Percentages are evaluated against a hash of the request's method and
/// path-plus-query modulo 100 rather than a random source — see the crate
/// docs for why reproducibility beats entropy here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FaultConfig {
    /// Delay applied to faulted requests, in milliseconds.
    pub delay_ms: u64,
    /// Percentage of requests that receive the delay (`0..=100`).
    pub delay_pct: u8,
    /// Status code returned by injected errors; `None` disables them.
    pub error_status: Option<u16>,
    /// Percentage of requests that receive the error (`0..=100`).
    pub error_pct: u8,
}

/// Full gateway configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayConfig {
    /// Operating mode.
    pub mode: Mode,
    /// Path of the OpenAPI entry document; its whole directory is loaded.
    pub spec: PathBuf,
    /// TCP port on `127.0.0.1` (`0` is allowed only when callers drive
    /// [`build_router`] themselves and bind their own listener).
    pub port: u16,
    /// Fault injection settings.
    pub faults: FaultConfig,
}

/// Shared per-server state handed to every handler.
struct GatewayState {
    spec: Arc<IrSpec>,
    mode: Mode,
    faults: FaultConfig,
    seq: AtomicU64,
    journal: Arc<tokio::sync::Mutex<Journal>>,
    mocks: HashMap<(Method, String), Vec<mock::CompiledResponse>>,
    replay: Option<ReplayIndex>,
    recorder: Option<Arc<tokio::sync::Mutex<proxy::CassetteAppender>>>,
    redactor: Arc<Redactor>,
}

/// Serves the gateway on `127.0.0.1:<cfg.port>`, blocking until failure.
///
/// # Errors
/// Router construction (spec/cassette loading), bind failures, or runtime
/// server failures, rendered as strings.
pub async fn serve(
    cfg: GatewayConfig,
    journal: Arc<tokio::sync::Mutex<Journal>>,
) -> Result<(), String> {
    let app = build_router(&cfg, Arc::clone(&journal)).await?;
    let listener = tokio::net::TcpListener::bind((HOST, cfg.port))
        .await
        .map_err(|e| format!("cannot bind {HOST}:{}: {e}", cfg.port))?;
    axum::serve(listener, app)
        .await
        .map_err(|e| format!("gateway server error: {e}"))
}

/// Builds the gateway router without binding a socket.
///
/// Exposed for benches and tests that drive the router in-process via
/// `tower::ServiceExt::oneshot`.
///
/// # Errors
/// Spec or cassette loading failures, rendered as strings.
pub async fn build_router(
    cfg: &GatewayConfig,
    journal: Arc<tokio::sync::Mutex<Journal>>,
) -> Result<Router, String> {
    // Phase profiler, gated like suspect-lsp's SUSPECT_HLDBG: set
    // SUSPECT_GW_PROFILE=1 to print per-phase startup timings on stderr.
    let profile = std::env::var_os("SUSPECT_GW_PROFILE").is_some_and(|v| !v.is_empty());
    let t_startup = Instant::now();

    let spec = {
        let t = Instant::now();
        let spec = load_ir(&cfg.spec)?;
        if profile {
            eprintln!("[gw-profile] load_ir: {:?}", t.elapsed());
        }
        spec
    };
    let mocks = {
        let t = Instant::now();
        let mocks = mock::compile_all(&spec);
        if profile {
            eprintln!(
                "[gw-profile] mock::compile_all ({} ops): {:?}",
                spec.operations.len(),
                t.elapsed()
            );
        }
        mocks
    };

    let replay = match &cfg.mode {
        Mode::Replay { cassette } => {
            let file = std::fs::File::open(cassette)
                .map_err(|e| format!("open cassette {}: {e}", cassette.display()))?;
            let (_header, entries) =
                suspect_journal::read_cassette(file).map_err(|e| format!("read cassette: {e}"))?;
            Some(ReplayIndex::new(&entries))
        }
        _ => None,
    };
    let recorder = match &cfg.mode {
        Mode::Record { cassette, .. } => {
            let source = format!("gateway {}", cfg.spec.display());
            Some(Arc::new(tokio::sync::Mutex::new(
                proxy::CassetteAppender::create(cassette, source)
                    .map_err(|e| format!("create cassette {}: {e}", cassette.display()))?,
            )))
        }
        _ => None,
    };

    // One shared credential scrubber for the journal and cassette
    // recording, so both sinks redact identically.
    let mut faults = cfg.faults;
    faults.delay_pct = faults.delay_pct.min(100);
    faults.error_pct = faults.error_pct.min(100);
    let redactor = Arc::new(Redactor::new());
    *journal.lock().await.redactor_mut() = (*redactor).clone();
    journal.lock().await.emit(Journal::meta(
        "gateway",
        "starting",
        serde_json::json!({
            "mode": cfg.mode.name(),
            "spec": cfg.spec.display().to_string(),
            "operations": spec.operations.len(),
        }),
    ));

    let router = router_for_state(Arc::new(GatewayState {
        spec: Arc::new(spec),
        mode: cfg.mode.clone(),
        faults,
        seq: AtomicU64::new(0),
        journal,
        mocks,
        replay,
        recorder,
        redactor,
    }));
    if profile {
        eprintln!("[gw-profile] total startup: {:?}", t_startup.elapsed());
    }
    Ok(router)
}

/// Wires routes for every operation path (all declared methods per path)
/// plus the fallback, all sharing one dispatch handler.
fn router_for_state(state: Arc<GatewayState>) -> Router {
    let mut paths: Vec<&str> = state
        .spec
        .operations
        .iter()
        .map(|op| op.path.as_str())
        .collect();
    paths.sort_unstable();
    paths.dedup();

    // Plex-style action paths (`/:/timeline`, `/:/prefs`) use `:` as a
    // literal segment prefix. Axum 0.8 panics on these unless the v0.7
    // compatibility check is disabled (matchit v0.8 treats `{param}` as
    // captures and `:segment` as a plain path).
    let mut router = Router::new().without_v07_checks();
    for path in paths {
        let mut method_router = axum::routing::MethodRouter::new();
        for op in &state.spec.operations {
            if op.path != path {
                continue;
            }
            method_router = match op.method {
                Method::Get => method_router.get(dispatch),
                Method::Put => method_router.put(dispatch),
                Method::Post => method_router.post(dispatch),
                Method::Delete => method_router.delete(dispatch),
                Method::Options => method_router.options(dispatch),
                Method::Head => method_router.head(dispatch),
                Method::Patch => method_router.patch(dispatch),
                Method::Trace => method_router.trace(dispatch),
            };
        }
        // Undeclared methods on a known path get an explicit journaled
        // 405 instead of axum's silent default.
        let method_router = method_router.fallback(method_not_allowed);
        // Some real-world specs have paths that matchit cannot express
        // (e.g. `{channel}.json` mixes a capture with a literal suffix).
        // Skip those; requests to them fall through to the 404 handler.
        let registrable = !path.split('/').any(|seg| {
            let inner = seg.trim_start_matches('{').trim_end_matches('}');
            seg.starts_with('{')
                && (inner.contains('.') || !inner.is_empty() && seg.ends_with("}"))
                && seg.matches('{').count() != 1
        }) && path.split('/').all(|seg| {
            !seg.starts_with('{') || (seg.ends_with('}') && seg.matches('{').count() == 1)
        });
        if registrable {
            router = router.route(path, method_router);
        } else {
            eprintln!("[gw] skipping unregistrable path: {path}");
        }
    }
    let pg_state = Arc::clone(&state);
    let router = router.route(
        "/playground",
        axum::routing::get(move || {
            let state = Arc::clone(&pg_state);
            async move { axum::response::Html(playground::playground_html(&state)).into_response() }
        }),
    );
    router.fallback(fallback_dispatch).with_state(state)
}

/// Handler for matched operations.
async fn dispatch(
    State(state): State<Arc<GatewayState>>,
    matched: MatchedPath,
    request: AxumRequest,
) -> Response {
    process(&state, request, Some(matched.as_str())).await
}

/// Fallback handler: replay lookup in replay mode, else plain 404.
async fn fallback_dispatch(
    State(state): State<Arc<GatewayState>>,
    request: AxumRequest,
) -> Response {
    if matches!(state.mode, Mode::Replay { .. }) {
        return process(&state, request, None).await;
    }
    process_not_found(&state, request).await
}

/// Handler for a matched path with no declared method: journaled 405.
async fn method_not_allowed(
    State(state): State<Arc<GatewayState>>,
    request: AxumRequest,
) -> Response {
    process_rejection(
        &state,
        request,
        StatusCode::METHOD_NOT_ALLOWED,
        "Method not allowed",
    )
    .await
}

/// Runs one full exchange: faults, mode dispatch, journaling.
///
/// `template` is the matched route pattern (an OpenAPI path template) when
/// the request hit a registered operation.
async fn process(
    state: &Arc<GatewayState>,
    request: AxumRequest,
    template: Option<&str>,
) -> Response {
    let started = Instant::now();
    let method = request.method().as_str().to_owned();
    let target = request.uri().path_and_query().map_or_else(
        || request.uri().path().to_owned(),
        |pq| pq.as_str().to_owned(),
    );
    let host = request
        .headers()
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost")
        .to_owned();
    let req_headers = collect_headers(request.headers());
    let body = match read_body(request.into_body()).await {
        BodyRead::Ok(bytes) => bytes,
        other => {
            // Oversize and disconnect failures must never masquerade as
            // empty bodies: reject before mode dispatch.
            let (status, title, detail) = match &other {
                BodyRead::Oversize => (
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "Payload too large",
                    Some(format!(
                        "request body exceeds the {} byte limit",
                        proxy::MAX_BODY
                    )),
                ),
                BodyRead::Failed(detail) => (
                    StatusCode::BAD_REQUEST,
                    "Request body unreadable",
                    Some(detail.clone()),
                ),
                BodyRead::Ok(_) => unreachable!("handled above"),
            };
            let response = problem(status, title, detail);
            let exchange = Exchange {
                method: method.clone(),
                host: host.clone(),
                target: target.clone(),
                request_headers: req_headers.clone(),
                response_status: response.status().as_u16(),
                response_headers: collect_headers(response.headers()),
                started,
            };
            journal_exchange(state, &exchange, Verdict::Pass).await;
            return response;
        }
    };

    // Fault middleware: skipped in replay mode (a recording replays as it
    // was captured, warts included). The roll is a pure function of the
    // request, so delay and error patterns are stable across runs and
    // independent of concurrency.
    if !matches!(state.mode, Mode::Replay { .. }) {
        let roll = fault_roll(&method, &target);
        if state.faults.delay_ms > 0 && roll % 100 < u64::from(state.faults.delay_pct) {
            tokio::time::sleep(Duration::from_millis(state.faults.delay_ms)).await;
        }
        if let Some(status) = state.faults.error_status
            && state.faults.error_pct > 0
            && roll % 100 < u64::from(state.faults.error_pct)
        {
            let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            let response = problem(status, "Injected fault", Some(format!("{method} {target}")));
            let exchange = Exchange {
                method: method.clone(),
                host: host.clone(),
                target: target.clone(),
                request_headers: req_headers.clone(),
                response_status: response.status().as_u16(),
                response_headers: collect_headers(response.headers()),
                started,
            };
            journal_exchange(state, &exchange, Verdict::Fault).await;
            return response;
        }
    }

    let (response, violations) = match &state.mode {
        Mode::Mock => {
            let ir_method = Method::from_key(method.to_ascii_lowercase().as_str());
            let compiled =
                template.and_then(|t| ir_method.and_then(|m| state.mocks.get(&(m, t.to_owned()))));
            let response = compiled.map_or_else(
                || {
                    problem(
                        StatusCode::NOT_IMPLEMENTED,
                        "No synthesized response",
                        Some(format!("no mock compiled for {method} {template:?}")),
                    )
                },
                |c| mock::respond(c),
            );
            (response, Vec::new())
        }
        Mode::Proxy { upstream } => {
            proxy::forward(upstream, &method, &target, &req_headers, body.clone()).await
        }
        Mode::Validate { upstream, enforce } => {
            let ir_method = Method::from_key(method.to_ascii_lowercase().as_str());
            let op = template.and_then(|t| {
                ir_method.and_then(|m| {
                    state
                        .spec
                        .operation(suspect_ir::OpSelector::MethodPath(m, t))
                })
            });
            match op {
                Some(op) => {
                    let refs = mock::schema_refs(&state.spec);
                    let ctx = proxy::ForwardCtx {
                        method: &method,
                        target: &target,
                        headers: &req_headers,
                    };
                    proxy::validate_forward(upstream, op, &refs, ctx, body.clone(), *enforce).await
                }
                None => (
                    problem(
                        StatusCode::NOT_FOUND,
                        "Operation not found",
                        Some(format!("{method} {target}")),
                    ),
                    Vec::new(),
                ),
            }
        }
        Mode::Record { upstream, .. } => {
            match proxy::fetch_upstream(upstream, &method, &target, &req_headers, body.clone())
                .await
            {
                Ok(reply) => {
                    let duration_ms = started.elapsed().as_secs_f64() * 1000.0;
                    let entry = suspect_journal::CassetteEntry {
                        id: 0,
                        method: method.clone(),
                        url: format!("http://{host}{target}"),
                        status: reply.status,
                        request_headers: state.redactor.headers(&req_headers),
                        request_body: redact_body(&state.redactor, &body),
                        response_headers: state.redactor.headers(&reply.headers),
                        response_body: redact_body(&state.redactor, &reply.body),
                        duration_ms,
                    };
                    let response = proxy::reply_to_response(&reply);
                    if let Some(recorder) = &state.recorder {
                        let mut appender = recorder.lock().await;
                        let first_failure = !appender.is_poisoned();
                        if let Err(err) = appender.append(entry) {
                            // The appender is sticky-poisoned after its
                            // first write failure; log that failure once
                            // at error level and stay quiet afterwards so
                            // one bad cassette cannot flood the journal.
                            if first_failure {
                                state.journal.lock().await.log(
                                    Level::Error,
                                    "gateway",
                                    "cassette append failed; recording stopped",
                                    serde_json::json!({ "error": err.to_string() }),
                                );
                            }
                        }
                    }
                    (response, Vec::new())
                }
                Err(err) => (
                    problem(StatusCode::BAD_GATEWAY, "Bad gateway", Some(err)),
                    Vec::new(),
                ),
            }
        }
        Mode::Replay { .. } => {
            let response = state.replay.as_ref().map_or_else(
                || problem(StatusCode::NOT_FOUND, "No cassette loaded", None),
                |index| replay::respond(index, &method, &target),
            );
            (response, Vec::new())
        }
    };

    let verdict = if violations.is_empty() {
        Verdict::Pass
    } else {
        Verdict::Invalid(violations)
    };
    let exchange = Exchange {
        method: method.clone(),
        host: host.clone(),
        target: target.clone(),
        request_headers: req_headers.clone(),
        response_status: response.status().as_u16(),
        response_headers: collect_headers(response.headers()),
        started,
    };
    journal_exchange(state, &exchange, verdict).await;
    response
}

/// Owned wire-level facts about one served exchange.
///
/// Journaling copies what it needs out of the `Response` *before*
/// suspending: holding a `&Response` across `.await` would make handler
/// futures non-`Send` (response bodies are not `Sync`).
struct Exchange {
    method: String,
    host: String,
    target: String,
    request_headers: Vec<(String, String)>,
    response_status: u16,
    response_headers: Vec<(String, String)>,
    started: Instant,
}

/// Serves a plain problem+json rejection (unknown-path `404`, undeclared
/// method `405`) and journals it.
async fn process_rejection(
    state: &Arc<GatewayState>,
    request: AxumRequest,
    status: StatusCode,
    title: &str,
) -> Response {
    let started = Instant::now();
    let method = request.method().as_str().to_owned();
    let target = request.uri().path_and_query().map_or_else(
        || request.uri().path().to_owned(),
        |pq| pq.as_str().to_owned(),
    );
    let host = request
        .headers()
        .get("host")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost")
        .to_owned();
    let req_headers = collect_headers(request.headers());
    let response = problem(status, title, Some(format!("{method} {target}")));
    let exchange = Exchange {
        method,
        host,
        target,
        request_headers: req_headers,
        response_status: response.status().as_u16(),
        response_headers: collect_headers(response.headers()),
        started,
    };
    journal_exchange(state, &exchange, Verdict::Pass).await;
    response
}

/// Serves the standard unknown-path 404.
async fn process_not_found(state: &Arc<GatewayState>, request: AxumRequest) -> Response {
    process_rejection(state, request, StatusCode::NOT_FOUND, "Operation not found").await
}

/// Emits one traffic record for a completed exchange.
async fn journal_exchange(state: &Arc<GatewayState>, exchange: &Exchange, verdict: Verdict) {
    let record = TrafficRecord {
        ts_ms: Journal::now_ms(),
        id: 0,
        correlation: format!("gw/{}", state.seq.fetch_add(1, Ordering::Relaxed)),
        method: exchange.method.clone(),
        url: format!("http://{}{}", exchange.host, exchange.target),
        status: Some(exchange.response_status),
        request_headers: exchange.request_headers.clone(),
        response_headers: exchange.response_headers.clone(),
        duration_ms: exchange.started.elapsed().as_secs_f64() * 1000.0,
        verdict,
    };
    state.journal.lock().await.traffic(record);
}

/// Collects headers into ordered `(name, value)` string pairs.
///
/// Header values are decoded lossily (`String::from_utf8_lossy`) so
/// non-UTF-8 bytes are preserved as replacement characters instead of
/// silently dropping the header from journals and cassettes.
#[must_use]
fn collect_headers(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect()
}

/// Outcome of reading an inbound request body to bytes, capped at
/// [`proxy::MAX_BODY`].
enum BodyRead {
    /// Body within the cap.
    Ok(Bytes),
    /// Body exceeded the cap (`413` for the client).
    Oversize,
    /// Read failed mid-body — client disconnect or malformed framing
    /// (`400` for the client).
    Failed(String),
}

/// Reads an axum request body to bytes, capping at [`proxy::MAX_BODY`].
///
/// Failures are propagated rather than swallowed: an oversize or
/// truncated body must never silently become an empty one.
async fn read_body(body: axum::body::Body) -> BodyRead {
    use http_body_util::{BodyExt as _, LengthLimitError, Limited};
    match Limited::new(body, proxy::MAX_BODY).collect().await {
        Ok(collected) => BodyRead::Ok(collected.to_bytes()),
        Err(err) if err.is::<LengthLimitError>() => BodyRead::Oversize,
        Err(err) => BodyRead::Failed(err.to_string()),
    }
}

/// Deterministic fault roll in `0..100`: a pure hash of the request's
/// method and path-plus-query (fixed-key `DefaultHasher`). The same
/// request faults identically no matter how many concurrent exchanges
/// interleave — unlike a shared counter.
#[must_use]
pub(crate) fn fault_roll(method: &str, path_and_query: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    hasher.write(method.as_bytes());
    hasher.write_u8(0);
    hasher.write(path_and_query.as_bytes());
    hasher.finish() % 100
}

/// Builds a cassette body, scrubbing sensitive keys from UTF-8 JSON.
///
/// Non-UTF-8 payloads are stored untouched (base64 in the cassette).
fn redact_body(redactor: &Redactor, bytes: &[u8]) -> suspect_journal::Body {
    match std::str::from_utf8(bytes) {
        Ok(text) => suspect_journal::Body::from_bytes(redactor.json_body(text).as_bytes()),
        Err(_) => suspect_journal::Body::from_bytes(bytes),
    }
}

/// Builds an RFC 7807 problem+json response.
#[must_use]
pub fn problem(status: StatusCode, title: &str, detail: Option<String>) -> Response {
    let mut body = serde_json::json!({
        "type": "about:blank",
        "title": title,
        "status": status.as_u16(),
    });
    if let Some(detail) = detail {
        body["detail"] = Value::String(detail);
    }
    (
        status,
        [("content-type", "application/problem+json")],
        body.to_string(),
    )
        .into_response()
}

/// Loads `spec` plus its transitive `$ref` closure into one workspace.
///
/// [`suspect_ref::Workspace::load_all`] walks the entry document's
/// external-`$ref` frontier breadth-first, so only documents the spec can
/// actually reach are parsed — startup cost scales with the reference
/// closure instead of the size of the containing directory (the previous
/// directory scan loaded every sibling YAML/JSON file, which dominated
/// gateway startup on shared corpus directories).
///
/// # Errors
/// Workspace build failures; unreadable or unloadable documents degrade
/// to a partial closure (matching the CLI's lenient behavior) — a missing
/// *entry* document is reported precisely by [`IrSpec::from_workspace`].
pub fn load_workspace(spec: &Path) -> Result<Arc<suspect_ref::Workspace>, String> {
    use suspect_ref::WorkspaceBuilder;
    let dir = spec
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let name = spec
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| format!("spec path {} has no file name", spec.display()))?;
    let ws = WorkspaceBuilder::new()
        .root(&dir)
        .build()
        .map_err(|e| format!("workspace build failed: {e}"))?;
    // Best-effort closure load: a failing *referenced* document leaves a
    // partial workspace behind rather than aborting startup, matching the
    // old directory scan's skip-on-error policy.
    let _ = ws.load_all(name);
    Ok(Arc::new(ws))
}

/// Loads the IR snapshot for `spec` through [`load_workspace`].
///
/// # Errors
/// Workspace/document loading or non-OAS documents.
fn load_ir(spec: &Path) -> Result<IrSpec, String> {
    let ws = load_workspace(spec)?;
    let uri = suspect_source::Uri::from_path(spec)
        .map_err(|e| format!("invalid spec path {}: {e}", spec.display()))?;
    IrSpec::from_workspace(&ws, &uri)
}
