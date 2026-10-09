//! `suspect drift`: cross-reference recorded traffic against the spec and
//! report the gaps — endpoints the traffic exercised that the spec does
//! not declare, undeclared query parameters on declared endpoints, and
//! declared operations the traffic never touched.
//!
//! The traffic file is either a Suspect Cassette (gateway `--mode record`
//! output) or a Suspect Journal (gateway `--journal` / `suspect test`
//! output); the two are distinguished line by line, so mixed files and
//! interleaved records work too.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::OutputFormat;
use suspect_ir::{IrSpec, ParamIn};

/// One observed exchange, normalized.
#[derive(Debug, Clone)]
pub struct Exchange {
    /// Uppercase HTTP method.
    pub method: String,
    /// Path component of the URL, no query string.
    pub path: String,
    /// Query parameter names present on the request.
    pub query: BTreeSet<String>,
}

/// The full drift report.
#[derive(Debug, serde::Serialize, PartialEq)]
pub struct DriftReport {
    /// The rollup counts.
    pub summary: DriftSummary,
    /// Exchanges that addressed no declared operation, with the query
    /// parameters they carried.
    pub missing_endpoints: Vec<MissingEndpoint>,
    /// Declared operations the recorded traffic never touched.
    pub untested_endpoints: Vec<UntestedEndpoint>,
    /// Declared operations exercised with undeclared query parameters.
    pub query_param_gaps: Vec<QueryParamGap>,
}

/// The rollup counts of one drift report.
#[derive(Debug, serde::Serialize, PartialEq)]
pub struct DriftSummary {
    /// Distinct `(method, path)` operations the spec declares.
    pub endpoints_in_spec: usize,
    /// Distinct `(method, path)` operations (or undocumented paths) the
    /// traffic addressed.
    pub endpoints_captured: usize,
    /// Traffic-exercised endpoints the spec does not declare.
    pub missing_from_spec: usize,
    /// Declared operations the traffic never touched.
    pub untested_in_spec: usize,
    /// Declared operations exercised with undeclared query parameters.
    pub query_param_gaps: usize,
}

/// Traffic addressed an endpoint the spec does not declare.
#[derive(Debug, serde::Serialize, PartialEq)]
pub struct MissingEndpoint {
    /// The path as requested (or the matching template, when the path
    /// matched a template the method does not cover).
    pub path: String,
    /// Uppercase HTTP method.
    pub method: String,
    /// Query parameter names the traffic carried (candidate spec entries).
    pub query_params_seen: Vec<String>,
}

/// A declared operation the recorded traffic never touched.
#[derive(Debug, serde::Serialize, PartialEq)]
pub struct UntestedEndpoint {
    /// The spec's path template.
    pub path: String,
    /// Uppercase HTTP method.
    pub method: String,
}

/// A declared operation exercised with query parameters it does not
/// declare.
#[derive(Debug, serde::Serialize, PartialEq)]
pub struct QueryParamGap {
    /// The spec's path template.
    pub path: String,
    /// Uppercase HTTP method.
    pub method: String,
    /// The undeclared query parameter names, sorted.
    pub missing_query_params: Vec<String>,
}

/// Reads one traffic line into an [`Exchange`]; `None` for headers, meta
/// records, log lines, and anything that is not an observed request.
#[must_use]
pub fn exchange_from_line(line: &serde_json::Value) -> Option<Exchange> {
    // Journal traffic record: {"kind": "traffic", "method", "url", ...}
    if line.get("kind").and_then(serde_json::Value::as_str) == Some("traffic") {
        let method = line.get("method")?.as_str()?.to_owned();
        let url = line.get("url")?.as_str()?;
        return Some(from_method_url(&method, url));
    }
    // Cassette entry: {"id", "method", "url", "status", ...}
    if line.get("method").is_some() && line.get("url").is_some() && line.get("kind").is_none() {
        let method = line.get("method")?.as_str()?.to_owned();
        let url = line.get("url")?.as_str()?;
        return Some(from_method_url(&method, url));
    }
    None
}

/// Splits a full URL into path and query parameter names.
pub fn from_method_url(method: &str, url: &str) -> Exchange {
    // URLs are absolute (scheme://host/path?query) or absolute-path
    // (/path?query); both split identically on the first `?`.
    let (before_query, query) = match url.split_once('?') {
        Some((b, q)) => (b, Some(q)),
        None => (url, None),
    };
    // Trim the scheme+authority if present.
    let path = before_query
        .split_once("://")
        .map_or(before_query, |(_, rest)| match rest.find('/') {
            Some(slash) => &rest[slash..],
            None => "/",
        });
    let mut names = BTreeSet::new();
    if let Some(query) = query {
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            let key = pair.split('=').next().unwrap_or(pair);
            // Percent-decoding only affects values in practice; keys are
            // conventionally plain, but decode defensively.
            let decoded = percent_decode(key);
            if !decoded.is_empty() {
                names.insert(decoded);
            }
        }
    }
    Exchange {
        method: method.to_ascii_uppercase(),
        path: path.to_owned(),
        query: names,
    }
}

/// Minimal percent-decoding for query keys.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Whether one concrete path segment matches one template segment.
/// `{param}` matches any non-empty segment; a segment with embedded
/// captures (`start.{extension}`) matches when every literal lines up.
fn segment_matches(template: &str, concrete: &str) -> bool {
    if !template.contains('{') {
        return template == concrete;
    }
    // Split the template around captures; literal gaps must match exactly
    // and concrete must be consumed fully.
    let mut parts = template.split('{');
    let Some(first) = parts.next() else {
        return false;
    };
    if !concrete.starts_with(first) {
        return false;
    }
    let mut rest = &concrete[first.len()..];
    for part in parts {
        // part is `name}` or `name}.suffix` — the capture ends at `}`.
        let Some((after_capture_prefix, literal)) = part.split_once('}') else {
            return false;
        };
        let _ = after_capture_prefix; // capture name; content is arbitrary
        // The captured segment may be empty; find the next literal.
        if literal.is_empty() {
            // Capture runs to the segment end (last capture).
            return !rest.is_empty() || part.ends_with('}');
        }
        match rest.find(literal) {
            Some(at) => rest = &rest[at + literal.len()..],
            None => return false,
        }
    }
    // A trailing literal means the concrete segment must end with it.
    true
}

/// One trailing slash is dropped (`/library/sections/` →
/// `/library/sections`), except on the root path itself — the same
/// normalization the gateway applies on both routing and recording, so a
/// recorded `/library/sections` meets the spec's `/library/sections/`.
fn normalize_trailing_slash(path: &str) -> &str {
    if path.len() > 1 {
        path.strip_suffix('/').unwrap_or(path)
    } else {
        path
    }
}

/// The spec template a concrete path matched, if any.
#[must_use]
fn match_template<'a>(templates: &'a [&'a str], path: &str) -> Option<&'a str> {
    let normalized = normalize_trailing_slash(path);
    let concrete: Vec<&str> = normalized.split('/').collect();
    templates.iter().copied().find(|t| {
        let segments: Vec<&str> = normalize_trailing_slash(t).split('/').collect();
        segments.len() == concrete.len()
            && segments
                .iter()
                .zip(concrete.iter())
                .all(|(ts, cs)| segment_matches(ts, cs))
    })
}

/// Computes the drift report for one spec and one set of exchanges.
#[must_use]
pub fn drift_report(spec: &IrSpec, exchanges: &[Exchange]) -> DriftReport {
    // Spec side: every (method, template), plus declared query params.
    let mut declared: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    let mut templates: Vec<&str> = Vec::new();
    for op in &spec.operations {
        let key = (op.method.as_str().to_owned(), op.path.clone());
        let entry = declared.entry(key).or_default();
        for param in &op.parameters {
            if param.location == ParamIn::Query {
                entry.insert(param.name.clone());
            }
        }
        if !templates.contains(&op.path.as_str()) {
            templates.push(op.path.as_str());
        }
    }

    // Traffic side: aggregate per (method, matched-template or raw path).
    let mut captured: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for exchange in exchanges {
        let path = match_template(&templates, &exchange.path)
            .map_or_else(|| exchange.path.clone(), |t| t.to_owned());
        let entry = captured.entry((exchange.method.clone(), path)).or_default();
        entry.extend(exchange.query.iter().cloned());
    }

    let mut missing_endpoints = Vec::new();
    let mut query_param_gaps = Vec::new();
    for ((method, path), seen) in &captured {
        match declared.get(&(method.clone(), path.clone())) {
            None => {
                missing_endpoints.push(MissingEndpoint {
                    path: path.clone(),
                    method: method.clone(),
                    query_params_seen: seen.iter().cloned().collect(),
                });
            }
            Some(declared_params) => {
                let missing: Vec<String> = seen
                    .difference(declared_params)
                    .cloned()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                if !missing.is_empty() {
                    query_param_gaps.push(QueryParamGap {
                        path: path.clone(),
                        method: method.clone(),
                        missing_query_params: missing,
                    });
                }
            }
        }
    }

    let mut untested_endpoints = Vec::new();
    for (method, path) in declared.keys() {
        if !captured.contains_key(&(method.clone(), path.clone())) {
            untested_endpoints.push(UntestedEndpoint {
                path: path.clone(),
                method: method.clone(),
            });
        }
    }

    DriftReport {
        summary: DriftSummary {
            endpoints_in_spec: declared.len(),
            endpoints_captured: captured.len(),
            missing_from_spec: missing_endpoints.len(),
            untested_in_spec: untested_endpoints.len(),
            query_param_gaps: query_param_gaps.len(),
        },
        missing_endpoints,
        untested_endpoints,
        query_param_gaps,
    }
}

/// `suspect drift <SPEC> <TRAFFIC> [--format text|json] [--exit-on-gap]
/// [--ignore-param NAME]`.
///
/// # Errors
/// IO or parse failures on either file.
pub fn drift(
    spec_path: &Path,
    traffic_path: &Path,
    ignored_params: &[String],
    exit_on_gap: bool,
    format: OutputFormat,
) -> anyhow::Result<i32> {
    let spec = IrSpec::from_file(spec_path).map_err(|e| anyhow::anyhow!("{e}"))?;

    let mut exchanges = Vec::new();
    for line in std::fs::read_to_string(traffic_path)?.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(mut exchange) = exchange_from_line(&value) {
            for name in ignored_params {
                exchange.query.remove(name);
            }
            exchanges.push(exchange);
        }
    }
    let report = drift_report(&spec, &exchanges);

    match format {
        OutputFormat::Text => print_text(&report),
        OutputFormat::Json => serde_json::to_writer_pretty(std::io::stdout(), &report)?,
        _ => unimplemented!(),
    }
    println!();

    if exit_on_gap {
        let gaps = report.summary.missing_from_spec + report.summary.query_param_gaps;
        if gaps > 0 {
            eprintln!(
                "[DRIFT] {gaps} gap(s): {} missing endpoint(s), {} query param gap(s)",
                report.summary.missing_from_spec, report.summary.query_param_gaps
            );
            return Ok(2);
        }
    }
    Ok(0)
}

/// Human-readable rendering; the `[DRIFT]` lines are the stable markers
/// CI greps for, mirroring the log contract the retired proxy published.
fn print_text(report: &DriftReport) {
    let s = &report.summary;
    println!(
        "spec: {} operations | captured: {} | missing from spec: {} | untested: {} | query param gaps: {}",
        s.endpoints_in_spec,
        s.endpoints_captured,
        s.missing_from_spec,
        s.untested_in_spec,
        s.query_param_gaps
    );
    for m in &report.missing_endpoints {
        let params = if m.query_params_seen.is_empty() {
            String::new()
        } else {
            format!(" (query: {})", m.query_params_seen.join(", "))
        };
        println!("[DRIFT] missing endpoint: {} {}{params}", m.method, m.path);
    }
    for g in &report.query_param_gaps {
        println!(
            "[DRIFT] query param gap: {} {} undeclares: {}",
            g.method,
            g.path,
            g.missing_query_params.join(", ")
        );
    }
    for u in &report.untested_endpoints {
        println!("[DRIFT] untested: {} {}", u.method, u.path);
    }
}
