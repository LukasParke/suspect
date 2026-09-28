//! `suspect impact`: traffic-backed compatibility analysis.
//!
//! Replays a recorded cassette against two spec revisions and reports the
//! recorded exchanges that pass the old contract but fail the new one,
//! grouped by consumer (user-agent). This grounds "will this change break
//! consumers?" in actual recorded traffic — with the observation window
//! the evidence comes from stated up front.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Args;
use serde::Serialize;

use suspect_ir::IrSpec;

use crate::OutputFormat;

/// One impact review.
#[derive(Debug, Args)]
pub struct ImpactArgs {
    /// The previous spec revision.
    #[arg(required = true)]
    pub old: PathBuf,
    /// The candidate spec revision.
    #[arg(required = true)]
    pub new: PathBuf,
    /// Recorded Suspect Cassette of production-like traffic.
    #[arg(required = true)]
    pub cassette: PathBuf,
    /// Output format for the report.
    #[command(flatten)]
    pub text: crate::TextFormat,
}

/// One evaluated recorded exchange.
#[derive(Debug, Clone, Serialize)]
pub struct ExchangeVerdict {
    /// Recorded method.
    pub method: String,
    /// Request path (host stripped).
    pub path: String,
    /// Recorded response status.
    pub status: u16,
    /// Consumer fingerprint (`user-agent`, `x-consumer-id`, else `unknown`).
    pub consumer: String,
    /// Passed the old contract.
    pub passes_old: bool,
    /// Passed the new contract.
    pub passes_new: bool,
    /// Why the new contract rejects it (empty when it passes).
    pub reason: String,
}

/// The complete impact report.
#[derive(Debug, Serialize)]
pub struct ImpactReport {
    /// How many recorded exchanges were evaluated.
    pub evaluated: usize,
    /// Exchanges that pass both revisions.
    pub unaffected: usize,
    /// Exchanges that pass old but fail new.
    pub broken: usize,
    /// Exchanges that failed both (pre-existing violations; not new impact).
    pub already_failing: usize,
    /// Broken exchanges grouped by consumer.
    pub by_consumer: BTreeMap<String, Vec<ExchangeVerdict>>,
    /// Every evaluated exchange, for drill-down.
    pub exchanges: Vec<ExchangeVerdict>,
}

/// Runs the impact analysis.
///
/// # Errors
/// File IO, workspace, or cassette failures.
pub fn impact(args: &ImpactArgs) -> anyhow::Result<i32> {
    let report = analyze(&args.old, &args.new, &args.cassette)?;
    match args.text.format {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
        OutputFormat::Sarif => anyhow::bail!("SARIF output is not defined for impact reports"),
        OutputFormat::Text => {
            println!(
                "evaluated {} recorded exchange(s): {} unaffected, {} broken by the new contract, {} already failing",
                report.evaluated, report.unaffected, report.broken, report.already_failing
            );
            if report.broken > 0 {
                println!();
                for (consumer, exchanges) in &report.by_consumer {
                    println!("{consumer}: {} broken exchange(s)", exchanges.len());
                    for e in exchanges {
                        println!("  {} {} → {}: {}", e.method, e.path, e.status, e.reason);
                    }
                }
            }
        }
    }
    Ok(i32::from(report.broken > 0))
}

/// Evaluates every cassette exchange against both revisions.
///
/// # Errors
/// File IO, workspace, or cassette parse failures.
pub fn analyze(old: &Path, new: &Path, cassette: &Path) -> anyhow::Result<ImpactReport> {
    let old_spec = load_ir(old)?;
    let new_spec = load_ir(new)?;
    let file = std::fs::File::open(cassette)?;
    let (_header, entries) = suspect_journal::read_cassette(file)?;

    let mut exchanges = Vec::new();
    for entry in &entries {
        let path = path_of(&entry.url);
        let consumer = consumer_of(&entry.request_headers);
        // Only evaluate exchanges the candidate contract still serves.
        let (passes_old, reason_old) = match match_op(&old_spec, &entry.method, &path) {
            Some(op) => {
                let verdict = validate_response(&old_spec, op, entry.status, &entry.response_body);
                match verdict {
                    None => (true, String::new()),
                    Some(reason) => (false, reason),
                }
            }
            None => (false, "operation absent from old contract".to_owned()),
        };
        let (passes_new, reason) = match match_op(&new_spec, &entry.method, &path) {
            Some(op) => {
                let verdict = validate_response(&new_spec, op, entry.status, &entry.response_body);
                match verdict {
                    None => (true, String::new()),
                    Some(reason) => (false, reason),
                }
            }
            None => (false, "operation removed from the new contract".to_owned()),
        };
        let reason = if !passes_new { reason } else { reason_old };
        exchanges.push(ExchangeVerdict {
            method: entry.method.clone(),
            path,
            status: entry.status,
            consumer,
            passes_old,
            passes_new,
            reason,
        });
    }

    let evaluated = exchanges.len();
    let broken = exchanges
        .iter()
        .filter(|e| e.passes_old && !e.passes_new)
        .cloned()
        .collect::<Vec<_>>();
    let unaffected = exchanges
        .iter()
        .filter(|e| e.passes_old && e.passes_new)
        .count();
    let already_failing = exchanges.iter().filter(|e| !e.passes_old).count();

    let mut by_consumer: BTreeMap<String, Vec<ExchangeVerdict>> = BTreeMap::new();
    for verdict in broken {
        by_consumer
            .entry(verdict.consumer.clone())
            .or_default()
            .push(verdict);
    }

    Ok(ImpactReport {
        evaluated,
        unaffected,
        broken: by_consumer.values().map(Vec::len).sum(),
        already_failing,
        by_consumer,
        exchanges,
    })
}

fn load_ir(path: &Path) -> anyhow::Result<IrSpec> {
    let ws = Arc::new(crate::commands::workspace_for_entry(path)?);
    // Same lexical identity as the workspace load: no canonicalization.
    let uri = suspect_source::Uri::from_path(path)?;
    IrSpec::from_workspace(&ws, &uri).map_err(|e| anyhow::anyhow!(e))
}

/// Strips scheme/authority/query from a recorded URL.
fn path_of(url: &str) -> String {
    let without_scheme = url.split("://").nth(1).unwrap_or(url);
    let start = without_scheme.find('/').unwrap_or(without_scheme.len());
    let rest = &without_scheme[start..];
    rest.split('?').next().unwrap_or(rest).to_owned()
}

/// The consumer fingerprint: user-agent, then x-consumer-id, else unknown.
fn consumer_of(headers: &[(String, String)]) -> String {
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("user-agent") {
            return format!("user-agent:{value}");
        }
    }
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("x-consumer-id") {
            return format!("consumer:{value}");
        }
    }
    "unknown".to_owned()
}

/// Matches a concrete request path against an operation path template
/// (`/pets/{petId}` matches `/pets/42`).
fn match_op<'s>(spec: &'s IrSpec, method: &str, path: &str) -> Option<&'s suspect_ir::IrOperation> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    spec.operations.iter().find(|op| {
        op.method.as_str().eq_ignore_ascii_case(method) && {
            let template: Vec<&str> = op.path.split('/').filter(|s| !s.is_empty()).collect();
            template.len() == segments.len()
                && template
                    .iter()
                    .zip(&segments)
                    .all(|(t, s)| t.starts_with('{') || *t == *s)
        }
    })
}

/// Validates a recorded response body against the operation's declared
/// response schema (status-exact, then `default`).
fn validate_response(
    spec: &IrSpec,
    op: &suspect_ir::IrOperation,
    status: u16,
    body: &suspect_journal::Body,
) -> Option<String> {
    let schema_name = op
        .responses
        .iter()
        .find(|r| r.status == Some(status))
        .or_else(|| op.responses.iter().find(|r| r.status.is_none()))
        .and_then(|r| r.schema.as_deref());
    let Some(name) = schema_name else {
        return None; // no declared schema: nothing to violate
    };
    let schema_json = spec.schemas.iter().find(|s| s.name == name)?;
    let bytes = body_bytes(body);
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return None; // binary bodies are not JSON-schema checkable here
    };
    let Ok(instance) = serde_json::from_str::<serde_json::Value>(text) else {
        return Some("recorded body is not valid JSON against the declared schema".to_owned());
    };
    let wrapper = serde_json::json!({"schema": schema_json.json, "instance": instance});
    let Ok(uri) = suspect_source::Uri::parse("mem://impact-check.json") else {
        return None;
    };
    let doc = suspect_low::LowDoc::parse(
        uri,
        suspect_source::Source::from_vec(wrapper.to_string().into_bytes()),
    );
    let (Some(schema_node), Some(instance_node)) =
        (doc.root().get("schema"), doc.root().get("instance"))
    else {
        return None;
    };
    let Ok(compiled) =
        suspect_schema::Compiler::new(suspect_schema::Config::default()).compile(schema_node)
    else {
        return None;
    };
    compiled
        .validate(instance_node)
        .first()
        .map(|e| e.message.clone())
}

fn body_bytes(body: &suspect_journal::Body) -> Vec<u8> {
    match body.encoding {
        suspect_journal::BodyEncoding::Utf8 => body.content.clone().into_bytes(),
        suspect_journal::BodyEncoding::Base64 => {
            // The journal base64-decodes via the same helper used at write
            // time; decode here with a minimal standard-alphabet decoder.
            base64_decode(&body.content)
        }
    }
}

/// Minimal standard-alphabet base64 decoder.
fn base64_decode(text: &str) -> Vec<u8> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for c in text.bytes() {
        let Some(v) = TABLE.iter().position(|t| *t == c) else {
            continue;
        };
        let v = v as u32;
        buffer = (buffer << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xFF) as u8);
        }
    }
    out
}
