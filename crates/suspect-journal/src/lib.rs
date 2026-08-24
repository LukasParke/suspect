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

use std::collections::BTreeSet;
use std::io::Write as _;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub mod cassette;
#[cfg(test)]
mod tests;
pub use cassette::{
    Body, BodyEncoding, CASSETTE_FORMAT, CASSETTE_VERSION, CassetteEntry, CassetteHeader,
    read_cassette, write_cassette,
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
    /// Full URL as requested.
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

/// Everything a journal can emit.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
/// Two denylists: header names and JSON body keys (both case-insensitive).
/// Values are replaced with `[redacted]`; structure is preserved so diffs
/// stay meaningful.
#[derive(Debug, Clone, Default)]
pub struct Redactor {
    headers: BTreeSet<String>,
    json_keys: BTreeSet<String>,
}

impl Redactor {
    /// Default denylist: `authorization`, `cookie`, `set-cookie`,
    /// `proxy-authorization`, `x-api-key`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            headers: [
                "authorization",
                "cookie",
                "set-cookie",
                "proxy-authorization",
                "x-api-key",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            json_keys: ["password", "token", "secret", "api_key"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        }
    }

    /// Adds one header name to the denylist (matched case-insensitively).
    pub fn deny_header(&mut self, key: &str) {
        self.headers.insert(key.to_ascii_lowercase());
    }

    /// Adds one JSON key to the body denylist (matched case-insensitively).
    pub fn deny_json_key(&mut self, key: &str) {
        self.json_keys.insert(key.to_lowercase());
    }

    /// Redacts matching headers.
    #[must_use]
    pub fn headers(&self, headers: &[(String, String)]) -> Vec<(String, String)> {
        headers
            .iter()
            .map(|(k, v)| {
                if self.headers.contains(&k.to_ascii_lowercase()) {
                    (k.clone(), REDACTED.to_owned())
                } else {
                    (k.clone(), v.clone())
                }
            })
            .collect()
    }

    /// Redacts matching keys anywhere inside a JSON body. Non-JSON bodies
    /// pass through untouched.
    #[must_use]
    pub fn json_body(&self, body: &str) -> String {
        let Ok(mut value) = serde_json::from_str::<serde_json::Value>(body) else {
            return body.to_owned();
        };
        Self::redact_value(&mut value, &self.json_keys);
        serde_json::to_string(&value).unwrap_or_else(|_| body.to_owned())
    }

    /// Redacts matching keys anywhere inside a JSON value, in place.
    pub fn json_value(&self, value: &mut serde_json::Value) {
        Self::redact_value(value, &self.json_keys);
    }

    fn redact_value(value: &mut serde_json::Value, keys: &BTreeSet<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (k, v) in map.iter_mut() {
                    if keys.contains(&k.to_lowercase()) {
                        *v = serde_json::Value::String(REDACTED.to_owned());
                    } else {
                        Self::redact_value(v, keys);
                    }
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    Self::redact_value(item, keys);
                }
            }
            _ => {}
        }
    }
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
                t.request_headers = self.redactor.headers(&t.request_headers);
                t.response_headers = self.redactor.headers(&t.response_headers);
            }
            Record::Log(l) => self.redactor.json_value(&mut l.fields),
            Record::Meta(m) => self.redactor.json_value(&mut m.fields),
            Record::RunSummary(_) => {}
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

    /// Convenience for [`Record::Traffic`] with automatic redaction.
    pub fn traffic(&mut self, mut t: TrafficRecord) {
        t.request_headers = self.redactor.headers(&t.request_headers);
        t.response_headers = self.redactor.headers(&t.response_headers);
        if t.id == 0 && self.seq > 0 {
            t.id = self.seq;
        }
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
