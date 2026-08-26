//! Wire frames shared with `rules-runtime/src/protocol.ts`. One JSON object
//! per line; every frame carries a `t` discriminator (applied via serde
//! enum tags).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Protocol version; bumped on any breaking frame change.
pub const PROTOCOL_VERSION: u32 = 1;

/// Which document kind a rule (or run) evaluates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    /// The OpenAPI document.
    Spec,
    /// Code-extraction facts (fold: `suspect-code`).
    Facts,
    /// Spec ∪ facts merged graph.
    Union,
}

/// Worker → host: per-rule metadata; the host compiles `given` natively.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleMeta {
    /// Unique rule id from `meta.id`.
    pub id: String,
    /// File the rule was loaded from.
    pub file: String,
    /// Which document kinds the rule evaluates.
    pub targets: Vec<TargetKind>,
    /// JSONPath `given` selector (point rules).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub given: Option<String>,
    /// Point (per-node `check`) or walk (visitor traversal) shape.
    pub shape: Shape,
    /// Visitor keys declared by walk rules (host informational).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub visitors: Vec<String>,
    /// True when the rule file imports zod (host informational).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub uses_zod: bool,
}

/// Point (per-node `check`) or walk (visitor traversal) rule shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    /// `given` + `check(node, ctx)` per selected node.
    Point,
    /// Visitors traversing the document with shared state.
    Walk,
}

/// Document root shipped once per evaluate run; point rules receive
/// pointer lists and resolve against it worker-side. The value is raw JSON
/// text (`RawValue`) so the host never re-serializes it inside the frame.
#[derive(Debug, Clone)]
pub struct RunDocument {
    /// Whole document as pre-serialized JSON (spliced verbatim into the
    /// frame — never re-serialized host-side).
    pub value: Box<serde_json::value::RawValue>,
}

impl Serialize for RunDocument {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        // Shape must stay `{"value": <raw doc>}` — the worker reads
        // `frame.document.value`. RawValue splices the JSON verbatim.
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry("value", &self.value)?;
        map.end()
    }
}

/// Walk-rule entry: the shared document plus the walk start.
#[derive(Debug, Clone, Serialize)]
pub struct WalkEntry {
    /// Pointer the walk starts from (usually empty).
    pub root_pointer: String,
}

/// Worker → host: one violation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FindingBody {
    /// Run the finding belongs to.
    pub run_id: u64,
    /// Producing rule.
    pub rule_id: String,
    /// RFC 6901 pointer of the offending node.
    pub pointer: String,
    /// Human message.
    pub message: String,
    /// Severity override (rule default when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    /// Structured fix intent (pass-through; host executes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<serde_json::Value>,
}

/// Worker → host: batched findings for one run (the worker buffers a
/// run's findings and emits them in one frame to avoid per-frame
/// round-trip costs at finding volume).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FindingsBatchBody {
    /// Run the batch belongs to.
    pub run_id: u64,
    /// All findings for the run, emission order.
    pub findings: Vec<FindingBody>,
}

/// Worker → host: a rule threw during a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleErrorBody {
    /// Run the error occurred in.
    pub run_id: u64,
    /// Rule that threw.
    pub rule_id: String,
    /// Error message.
    pub message: String,
}

/// Worker → host: run completed.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DoneBody {
    /// Completed run id.
    pub run_id: u64,
    /// Worker-measured duration in milliseconds.
    pub ms: f64,
    /// Findings emitted during the run.
    pub findings: u32,
}

/// Payload of [`WorkerFrame::Ready`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadyPayload {
    /// Bun version string.
    pub bun: String,
    /// SDK version the worker compiled against.
    pub sdk: String,
    /// Loaded rules in declaration order.
    pub rules: Vec<RuleMeta>,
}

/// Payload of [`WorkerFrame::Fatal`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FatalBody {
    /// What went wrong, worker-side.
    pub message: String,
}

/// Worker → host: any frame the worker sends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum WorkerFrame {
    /// Rules loaded after hello/reload.
    Ready(ReadyPayload),
    /// One violation (legacy single-frame path).
    Finding(FindingBody),
    /// Batched findings for a run.
    FindingsBatch(FindingsBatchBody),
    /// Run finished.
    Done(DoneBody),
    /// A rule threw.
    RuleError(RuleErrorBody),
    /// Fatal worker problem.
    Fatal(FatalBody),
    /// Liveness reply.
    Pong,
}

/// Host → worker frames, tagged the same way.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum HostFrame {
    /// Handshake: protocol version, workspace, rule files to load.
    Hello {
        /// Wire protocol version.
        protocol: u32,
        /// Host-side SDK version.
        sdk_version: String,
        /// Workspace root (worker cwd, read jail).
        workspace_root: String,
        /// Absolute rule file paths to import.
        rule_files: Vec<String>,
    },
    /// Evaluate: one document payload + per-rule pointer lists.
    Evaluate {
        /// Monotonic run id.
        run_id: u64,
        /// Deadline the host enforces.
        timeout_ms: u64,
        /// Document kind under evaluation.
        doc_kind: TargetKind,
        /// Document uri when known.
        #[serde(skip_serializing_if = "Option::is_none")]
        doc_uri: Option<String>,
        /// The document, shipped once for the whole run.
        document: RunDocument,
        /// Per-point-rule selected pointers.
        selections: BTreeMap<String, Vec<String>>,
        /// Per-walk-rule walk entries.
        walks: BTreeMap<String, WalkEntry>,
    },
    /// Re-import changed rule files.
    Reload {
        /// Files that changed.
        files: Vec<String>,
    },
    /// Liveness probe.
    Ping,
}
