//! Structured journals and the Suspect Cassette recording format.
//!
//! Every platform component — test executor, gateway, generator, CLI — emits
//! its observations through a [`Journal`]: one JSON object per line, written
//! to any [`Sink`] (stdout, file, in-memory). Traffic observations flow
//! through [`Journal::traffic`], which applies [`Redactor`] first so
//! credentials never reach disk.
//!
//! Recorded HTTP traffic is persisted as a **Suspect Cassette**: a header
//! line followed by one [`CassetteEntry`] per exchange, JSONL throughout.
//! Cassettes are append-only, streamable, diffable in git, and serve replay,
//! offline test transports, and environment drift comparison.

#![deny(missing_docs)]

use std::collections::BTreeSet;
use std::io::Write as _;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub mod cassette;
pub mod causal;
mod record;
#[cfg(test)]
mod tests;
pub use cassette::{
    Body, BodyEncoding, CASSETTE_FORMAT, CASSETTE_VERSION, CassetteEntry, CassetteHeader,
    read_cassette, write_cassette, write_entry,
};

/// Severity of a [`LogRecord`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// Fine-grained diagnostic detail.
    Trace,
    /// Development-time information.
    Debug,
    /// Normal operational events.
    Info,
    /// Something unexpected but recoverable.
    Warn,
    /// A failure worth surfacing.
    Error,
}

/// Outcome verdict attached to a traffic observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Exchange satisfied the contract.
    Pass,
    /// Validation produced violations (request or response side).
    Invalid(Vec<Violation>),
    /// The exchange addressed no declared operation — an unknown path, or
    /// an undeclared method on a known path. In proxy-family modes the
    /// exchange is still forwarded and observed, so spec drift shows up in
    /// the journal instead of silently vanishing into a 404.
    OffSpec(Vec<Violation>),
    /// Fault injection altered the exchange deliberately.
    Fault,
}

/// One contract violation, anchored by a JSON pointer into the offending
/// document when known.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Violation {
    /// Human-readable description.
    pub message: String,
    /// JSON pointer into the request/response body (`""` when not applicable).
    pub pointer: String,
}

/// Startup/identity record: which component ran, against what, with what
/// settings summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaRecord {
    /// Unix epoch milliseconds.
    pub ts_ms: u64,
    /// Emitting component (`test`, `gateway`, `gen`, `cli`).
    pub component: String,
    /// One-line description.
    pub msg: String,
    /// Free-form structured details.
    pub fields: serde_json::Value,
}

/// A log event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogRecord {
    /// Unix epoch milliseconds.
    pub ts_ms: u64,
    /// Severity.
    pub level: Level,
    /// Emitting module path or component name.
    pub target: String,
    /// Message text.
    pub msg: String,
    /// Free-form structured details.
    pub fields: serde_json::Value,
}

/// One observed HTTP exchange.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficRecord {
    /// Unix epoch milliseconds (completion time).
    pub ts_ms: u64,
    /// Monotonic sequence number within this journal.
    pub id: u64,
    /// Correlation id tying the exchange to its workflow/step/request span.
    pub correlation: String,
    /// HTTP method.
    pub method: String,
    /// URL as requested, with userinfo passwords and secret query
    /// parameter values redacted.
    pub url: String,
    /// Response status; `None` for transport failures.
    pub status: Option<u16>,
    /// Request headers after redaction.
    pub request_headers: Vec<(String, String)>,
    /// Response headers after redaction.
    pub response_headers: Vec<(String, String)>,
    /// Wall-clock duration of the exchange in milliseconds.
    pub duration_ms: f64,
    /// Contract verdict.
    pub verdict: Verdict,
}

/// End-of-run rollup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSummaryRecord {
    /// Unix epoch milliseconds.
    pub ts_ms: u64,
    /// What kind of run (`test`, `check`, `gen`, ...). Named `run_kind`
    /// to avoid colliding with the enum tag key.
    pub run_kind: String,
    /// Passed units.
    pub passed: u32,
    /// Failed units.
    pub failed: u32,
    /// Skipped units.
    pub skipped: u32,
    /// Total wall-clock duration in milliseconds.
    pub duration_ms: f64,
}

/// Periodic load-test metrics snapshot (`suspect load`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadSnapshotRecord {
    /// Unix epoch milliseconds.
    pub ts_ms: u64,
    /// Zero-based interval index.
    pub index: usize,
    /// Active phase label.
    pub phase: String,
    /// Seconds since the run started.
    pub at_s: f64,
    /// Metrics for this interval only (`MetricsSnapshot` JSON).
    pub interval: serde_json::Value,
    /// Metrics since the run started (`MetricsSnapshot` JSON).
    pub cumulative: serde_json::Value,
}

/// Everything a journal can emit.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Record {
    /// Identity/settings record.
    Meta(MetaRecord),
    /// Log event.
    Log(LogRecord),
    /// Observed HTTP exchange.
    Traffic(TrafficRecord),
    /// End-of-run rollup.
    RunSummary(RunSummaryRecord),
    /// Load-test interval snapshot.
    LoadSnapshot(LoadSnapshotRecord),
}

/// Destination for journal lines.
pub trait Sink: Send {
    /// Writes one complete line (implementations append the newline).
    ///
    /// # Errors
    /// Propagates I/O failures from the underlying destination.
    fn write_line(&mut self, line: &str) -> std::io::Result<()>;

    /// Flushes buffered output.
    ///
    /// # Errors
    /// Propagates I/O failures from the underlying destination.
    fn flush(&mut self) -> std::io::Result<()>;
}

/// Writes journal lines to stdout.
#[derive(Default)]
pub struct StdoutSink;

impl Sink for StdoutSink {
    fn write_line(&mut self, line: &str) -> std::io::Result<()> {
        println!("{line}");
        Ok(())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        std::io::stdout().flush()
    }
}

/// Appends journal lines to a file.
pub struct FileSink {
    file: std::fs::File,
}

impl FileSink {
    /// Opens (or creates) `path` for appending.
    ///
    /// # Errors
    /// Propagates filesystem errors from opening the file.
    pub fn open(path: &std::path::Path) -> std::io::Result<Self> {
        Ok(Self {
            file: std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?,
        })
    }
}

impl Sink for FileSink {
    fn write_line(&mut self, line: &str) -> std::io::Result<()> {
        writeln!(self.file, "{line}")
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

/// Collects journal lines in memory; the test double.
///
/// Cheaply cloneable: clones share one buffer, so tests keep a handle
/// while the original moves into a [`Journal`].
#[derive(Clone, Default)]
pub struct VecSink {
    lines: std::sync::Arc<Mutex<Vec<String>>>,
}

impl VecSink {
    /// Snapshot of collected lines.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        self.lines.lock().expect("vecsink mutex").clone()
    }

    /// Parses every collected line back into a [`Record`]; panics via
    /// `expect` on malformed lines so tests fail loudly.
    #[must_use]
    pub fn records(&self) -> Vec<Record> {
        self.lines()
            .into_iter()
            .map(|l| serde_json::from_str(&l).expect("journal line must parse"))
            .collect()
    }
}

impl Sink for VecSink {
    fn write_line(&mut self, line: &str) -> std::io::Result<()> {
        self.lines
            .lock()
            .expect("vecsink mutex")
            .push(line.to_owned());
        Ok(())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Credential scrubber applied before anything reaches a sink.
///
/// Two denylists: exact header names, and secret-name suffixes matched
/// against header names, JSON body keys and URL query parameter names.
/// Names are normalized before matching (lowercased, `-`/`_` removed), and
/// a name is secret when its normalized form ends with a denylisted
/// suffix: `access_token`, `X-Auth-Token` and `client_secret` match, while
/// `max_tokens` and `token_type` do not. Values are replaced with
/// `[redacted]`; structure is preserved so diffs stay meaningful.
#[derive(Debug, Clone)]
pub struct Redactor {
    headers: BTreeSet<String>,
    secret_suffixes: BTreeSet<String>,
}

impl Default for Redactor {
    /// The same denylists as [`Redactor::new`]; a redactor never starts
    /// out redacting nothing.
    fn default() -> Self {
        Self::new()
    }
}

impl Redactor {
    /// Default denylists. Headers: `authorization`, `cookie`,
    /// `set-cookie`, `proxy-authorization`. Secret suffixes: `password`,
    /// `passwd`, `token`, `secret`, `apikey`, `privatekey`, `credential`,
    /// `credentials`, `signature`, `authorization`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            headers: [
                "authorization",
                "cookie",
                "set-cookie",
                "proxy-authorization",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            secret_suffixes: [
                "password",
                "passwd",
                "token",
                "secret",
                "apikey",
                "privatekey",
                "credential",
                "credentials",
                "signature",
                "authorization",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        }
    }

    /// Adds one header name to the exact-match denylist (case-insensitive).
    pub fn deny_header(&mut self, key: &str) {
        self.headers.insert(key.to_ascii_lowercase());
    }

    /// Adds one secret-name suffix, matched against header names, JSON
    /// keys and query parameter names after normalization.
    pub fn deny_json_key(&mut self, key: &str) {
        self.secret_suffixes.insert(normalize(key));
    }

    /// Whether a header name, JSON key or query parameter name is secret.
    #[must_use]
    pub fn is_secret_name(&self, name: &str) -> bool {
        let normalized = normalize(name);
        self.secret_suffixes
            .iter()
            .any(|suffix| normalized.ends_with(suffix.as_str()))
    }

    /// Redacts matching headers.
    #[must_use]
    pub fn headers(&self, headers: &[(String, String)]) -> Vec<(String, String)> {
        headers
            .iter()
            .map(|(k, v)| {
                if self.headers.contains(&k.to_ascii_lowercase()) || self.is_secret_name(k) {
                    (k.clone(), REDACTED.to_owned())
                } else {
                    (k.clone(), v.clone())
                }
            })
            .collect()
    }

    /// Redacts a URL's userinfo password and the values of secret query
    /// parameters (`?api_key=…`, `&access_token=…`). Everything else,
    /// including the fragment, is kept byte for byte.
    #[must_use]
    pub fn url(&self, url: &str) -> String {
        let (before_fragment, fragment) = match url.split_once('#') {
            Some((head, tail)) => (head, Some(tail)),
            None => (url, None),
        };
        let (base, query) = match before_fragment.split_once('?') {
            Some((head, tail)) => (head, Some(tail)),
            None => (before_fragment, None),
        };
        let mut out = redact_userinfo(base);
        if let Some(query) = query {
            out.push('?');
            let pairs: Vec<String> = query
                .split('&')
                .map(|pair| match pair.split_once('=') {
                    Some((name, _)) if self.is_secret_name(&percent_decode_lossy(name)) => {
                        format!("{name}={REDACTED}")
                    }
                    _ => pair.to_owned(),
                })
                .collect();
            out.push_str(&pairs.join("&"));
        }
        if let Some(fragment) = fragment {
            out.push('#');
            out.push_str(fragment);
        }
        out
    }

    /// Redacts matching keys anywhere inside a JSON body. Non-JSON bodies
    /// pass through untouched.
    #[must_use]
    pub fn json_body(&self, body: &str) -> String {
        let Ok(mut value) = serde_json::from_str::<serde_json::Value>(body) else {
            return body.to_owned();
        };
        self.json_value(&mut value);
        serde_json::to_string(&value).unwrap_or_else(|_| body.to_owned())
    }

    /// Redacts matching keys anywhere inside a JSON value, in place.
    pub fn json_value(&self, value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                for (k, v) in map.iter_mut() {
                    if self.is_secret_name(k) {
                        *v = serde_json::Value::String(REDACTED.to_owned());
                    } else {
                        self.json_value(v);
                    }
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    self.json_value(item);
                }
            }
            _ => {}
        }
    }
}

/// Lowercases and drops `-`/`_` so `X-Api-Key`, `api_key` and `apiKey`
/// compare equal.
fn normalize(name: &str) -> String {
    name.chars()
        .filter(|c| *c != '-' && *c != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

/// Replaces the password in `scheme://user:password@host…` with
/// `[redacted]`; returns the input unchanged when there is none.
fn redact_userinfo(base: &str) -> String {
    let Some(scheme_end) = base.find("://") else {
        return base.to_owned();
    };
    let authority_start = scheme_end + 3;
    let authority_end = base[authority_start..]
        .find('/')
        .map_or(base.len(), |i| authority_start + i);
    let authority = &base[authority_start..authority_end];
    let Some(at) = authority.rfind('@') else {
        return base.to_owned();
    };
    let userinfo = &authority[..at];
    let Some(colon) = userinfo.find(':') else {
        return base.to_owned();
    };
    format!(
        "{}{}:{REDACTED}{}",
        &base[..authority_start],
        &userinfo[..colon],
        &base[authority_start + at..]
    )
}

/// Percent-decodes a query parameter name for matching only; invalid
/// escapes are kept literally.
fn percent_decode_lossy(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(hi * 16 + lo);
            i += 3;
        } else {
            out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    (byte as char)
        .to_digit(16)
        .and_then(|d| u8::try_from(d).ok())
}

/// Placeholder substituted for redacted values.
pub const REDACTED: &str = "[redacted]";

/// The journal itself: sequence-numbered records serialized one per line.
pub struct Journal {
    sink: Box<dyn Sink>,
    redactor: Redactor,
    seq: u64,
}

impl Journal {
    /// Creates a journal writing unredacted-safe defaults to `sink`.
    #[must_use]
    pub fn new(sink: Box<dyn Sink>) -> Self {
        Self::with_redactor(sink, Redactor::new())
    }

    /// Creates a journal with an explicit [`Redactor`].
    #[must_use]
    pub fn with_redactor(sink: Box<dyn Sink>, redactor: Redactor) -> Self {
        Self {
            sink,
            redactor,
            seq: 0,
        }
    }

    /// Mutable access to the redactor (e.g. adding component-specific
    /// denylist entries before traffic starts flowing).
    pub fn redactor_mut(&mut self) -> &mut Redactor {
        &mut self.redactor
    }

    /// Emits one arbitrary record.
    pub fn emit(&mut self, record: Record) {
        let mut record = record;
        match &mut record {
            Record::Traffic(t) => {
                t.id = self.seq;
                t.url = self.redactor.url(&t.url);
                t.request_headers = self.redactor.headers(&t.request_headers);
                t.response_headers = self.redactor.headers(&t.response_headers);
            }
            Record::Log(l) => self.redactor.json_value(&mut l.fields),
            Record::Meta(m) => self.redactor.json_value(&mut m.fields),
            Record::RunSummary(_) | Record::LoadSnapshot(_) => {}
        }
        self.seq += 1;
        let line = serde_json::to_string(&record).expect("record serializes");
        if let Err(err) = self.sink.write_line(&line) {
            eprintln!("suspect-journal: sink write failed: {err}");
        }
    }

    /// Convenience for [`Record::Log`].
    pub fn log(&mut self, level: Level, target: &str, msg: &str, fields: serde_json::Value) {
        self.emit(Record::Log(LogRecord {
            ts_ms: Self::now_ms(),
            level,
            target: target.to_owned(),
            msg: msg.to_owned(),
            fields,
        }));
    }

    /// Convenience for [`Record::Traffic`]. [`Journal::emit`] assigns the
    /// sequence id and redacts the URL and headers.
    pub fn traffic(&mut self, t: TrafficRecord) {
        self.emit(Record::Traffic(t));
    }

    /// Convenience for [`Record::RunSummary`].
    pub fn run_summary(
        &mut self,
        kind: &str,
        passed: u32,
        failed: u32,
        skipped: u32,
        duration_ms: f64,
    ) {
        self.emit(Record::RunSummary(RunSummaryRecord {
            ts_ms: Self::now_ms(),
            run_kind: kind.to_owned(),
            passed,
            failed,
            skipped,
            duration_ms,
        }));
    }

    /// Convenience for [`Record::Meta`].
    pub fn meta(component: &str, msg: &str, fields: serde_json::Value) -> Record {
        Record::Meta(MetaRecord {
            ts_ms: Self::now_ms(),
            component: component.to_owned(),
            msg: msg.to_owned(),
            fields,
        })
    }

    /// Current unix epoch milliseconds.
    #[must_use]
    pub fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    /// Flushes the underlying sink.
    ///
    /// # Errors
    /// Propagates sink flush errors.
    pub fn flush(&mut self) -> std::io::Result<()> {
        self.sink.flush()
    }
}

/// SHA-256 hex digest of `bytes`; used for cassette body hashes.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}
