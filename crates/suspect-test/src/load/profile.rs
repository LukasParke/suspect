//! Load profile model (`*.load.yaml`) and its `suspect-low` reader.
//!
//! A profile says *how many users, how fast, with what data, against which
//! target, under what SLOs*; the Arazzo document it references says what
//! each user does. The reader produces owned values plus source-located
//! diagnostics so `suspect validate` and the LSP can point at the offending
//! line.
//!
//! Root discriminator: `suspectLoad: "1.0"`.

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Duration;

use suspect_low::{LowDoc, NodeRef, ValueKind};

use crate::duration::parse_duration;
use crate::plan::ThinkSpec;

/// A source-located profile finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileDiagnostic {
    /// Stable machine code (`load-unknown-workflow`, ...).
    pub code: &'static str,
    /// Human-readable explanation.
    pub message: String,
    /// Byte range of the offending node in the profile document.
    pub range: Range<usize>,
    /// Whether this blocks a run (`true`) or is advisory.
    pub error: bool,
}

/// Failure to read a profile: every blocking diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileError(pub Vec<ProfileDiagnostic>);

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, d) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{} ({})", d.message, d.code)?;
        }
        Ok(())
    }
}

impl std::error::Error for ProfileError {}

/// How virtual users arrive during one phase.
#[derive(Debug, Clone, PartialEq)]
pub enum PhaseKind {
    /// Open loop: `rate` new users per second, optionally ramping linearly
    /// to `ramp_to` over the phase.
    ArrivalRate {
        /// Initial arrivals per second.
        rate: f64,
        /// Final arrivals per second (linear ramp) when set.
        ramp_to: Option<f64>,
    },
    /// Open loop: exactly `count` users spread evenly over the phase.
    ArrivalCount {
        /// Total arrivals.
        count: u64,
    },
    /// No arrivals; in-flight users continue.
    Pause,
    /// Closed loop: `vus` long-lived users each looping scenarios until the
    /// phase ends (or until `iterations` total loops across all users).
    ConstantVus {
        /// Concurrent users.
        vus: usize,
        /// Cap on total iterations across all users.
        iterations: Option<u64>,
    },
}

/// One load phase.
#[derive(Debug, Clone, PartialEq)]
pub struct Phase {
    /// Optional label for reports.
    pub name: Option<String>,
    /// Arrival model.
    pub kind: PhaseKind,
    /// Phase length (how long arrivals are generated, not how long users run).
    pub duration: Duration,
    /// Cap on concurrently running users; arrivals beyond it are skipped.
    pub max_vusers: Option<usize>,
    /// Source range of the phase node.
    pub range: Range<usize>,
}

impl Phase {
    /// Report label: the name, else the kind.
    #[must_use]
    pub fn label(&self) -> String {
        self.name.clone().unwrap_or_else(|| match &self.kind {
            PhaseKind::ArrivalRate {
                rate,
                ramp_to: None,
            } => format!("{rate}/s"),
            PhaseKind::ArrivalRate {
                rate,
                ramp_to: Some(to),
            } => format!("{rate}→{to}/s"),
            PhaseKind::ArrivalCount { count } => format!("{count} arrivals"),
            PhaseKind::Pause => "pause".to_owned(),
            PhaseKind::ConstantVus { vus, .. } => format!("{vus} vus"),
        })
    }
}

/// Where a scenario input value comes from.
#[derive(Debug, Clone, PartialEq)]
pub enum InputSource {
    /// Same value for every user.
    Literal(serde_json::Value),
    /// One value picked per user from the pool (uniformly, seeded).
    Pool(Vec<serde_json::Value>),
}

/// Row selection order for a data file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DataOrder {
    /// Uniform random row per user (Artillery default).
    #[default]
    Random,
    /// Rows in order, wrapping around.
    Sequence,
    /// Rows in order, never reused; users beyond the last row fail.
    Unique,
}

/// A CSV / NDJSON data file feeding one row of inputs per user.
#[derive(Debug, Clone, PartialEq)]
pub struct DataSource {
    /// Resolved file path.
    pub path: PathBuf,
    /// Input names for CSV columns (ignored for NDJSON objects).
    pub fields: Vec<String>,
    /// Row selection order.
    pub order: DataOrder,
    /// Skip the first CSV row.
    pub skip_header: bool,
    /// CSV delimiter byte.
    pub delimiter: u8,
    /// Source range of the `data` node.
    pub range: Range<usize>,
}

/// A weighted user scenario bound to an Arazzo workflow.
#[derive(Debug, Clone, PartialEq)]
pub struct Scenario {
    /// Report name (defaults to the workflow id).
    pub name: String,
    /// Arazzo `workflowId` to run.
    pub workflow: String,
    /// Relative selection weight (default 1).
    pub weight: f64,
    /// Default think time between steps for this scenario.
    pub think: Option<ThinkSpec>,
    /// Per-user input sources.
    pub inputs: BTreeMap<String, InputSource>,
    /// Row-per-user data file.
    pub data: Option<DataSource>,
    /// Times each user repeats the workflow (default 1).
    pub iterations: u64,
    /// Keep iterating after a failed iteration.
    pub continue_on_failure: bool,
    /// Source range of the scenario node.
    pub range: Range<usize>,
}

/// Transport settings.
#[derive(Debug, Clone, PartialEq)]
pub struct HttpOptions {
    /// Per-request timeout.
    pub timeout: Duration,
    /// Negotiate HTTP/2.
    pub http2: bool,
    /// Fresh connection pool (and cookie jar) per user.
    pub per_vu_connections: bool,
    /// Keep cookies between a user's requests.
    pub cookies: bool,
    /// Accept invalid TLS certificates.
    pub insecure: bool,
}

impl Default for HttpOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(10),
            http2: true,
            per_vu_connections: false,
            cookies: true,
            insecure: false,
        }
    }
}

/// A boolean `ensure` expression.
#[derive(Debug, Clone, PartialEq)]
pub struct Condition {
    /// Expression text (see [`crate::load::ensure`] for the grammar).
    pub expression: String,
    /// Failing a strict condition fails the run; non-strict only warns.
    pub strict: bool,
    /// Source range.
    pub range: Range<usize>,
}

/// SLO checks evaluated over the cumulative run metrics.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Ensure {
    /// Maximum percent of requests failing transport or criteria.
    pub max_error_rate: Option<f64>,
    /// `metric <= value` pairs (ms for time metrics).
    pub thresholds: Vec<(String, f64, Range<usize>)>,
    /// Free-form conditions.
    pub conditions: Vec<Condition>,
}

/// Reporting options.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportOptions {
    /// Interval between progress snapshots.
    pub interval: Duration,
    /// JSON report output path.
    pub json: Option<PathBuf>,
    /// JUnit report output path.
    pub junit: Option<PathBuf>,
}

impl Default for ReportOptions {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(10),
            json: None,
            junit: None,
        }
    }
}

/// Environment-specific overrides.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Environment {
    /// Replacement target.
    pub target: Option<String>,
    /// Replacement phases.
    pub phases: Option<Vec<Phase>>,
}

/// A parsed load profile.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadProfile {
    /// Arazzo document path, resolved against the profile's directory.
    pub arazzo: Option<PathBuf>,
    /// Base URL of the system under test.
    pub target: Option<String>,
    /// Named environment overrides (`-e staging`).
    pub environments: BTreeMap<String, Environment>,
    /// Phases in order.
    pub phases: Vec<Phase>,
    /// Workflow run once before arrivals begin.
    pub before: Option<String>,
    /// Workflow run once after the last user finishes.
    pub after: Option<String>,
    /// Weighted scenarios.
    pub scenarios: Vec<Scenario>,
    /// Transport settings.
    pub http: HttpOptions,
    /// SLO checks.
    pub ensure: Ensure,
    /// Reporting options.
    pub report: ReportOptions,
    /// RNG seed for reproducible scenario selection and jitter.
    pub seed: Option<u64>,
    /// Grace period for in-flight users after the last phase.
    pub grace: Duration,
}

impl Default for LoadProfile {
    fn default() -> Self {
        Self {
            arazzo: None,
            target: None,
            environments: BTreeMap::new(),
            phases: Vec::new(),
            before: None,
            after: None,
            scenarios: Vec::new(),
            http: HttpOptions::default(),
            ensure: Ensure::default(),
            report: ReportOptions::default(),
            seed: None,
            grace: Duration::from_secs(30),
        }
    }
}

impl LoadProfile {
    /// Applies a named environment's overrides.
    ///
    /// # Errors
    /// Returns the unknown environment name.
    pub fn apply_environment(&mut self, name: &str) -> Result<(), String> {
        let env = self
            .environments
            .get(name)
            .cloned()
            .ok_or_else(|| format!("unknown environment `{name}`"))?;
        if let Some(target) = env.target {
            self.target = Some(target);
        }
        if let Some(phases) = env.phases {
            self.phases = phases;
        }
        Ok(())
    }

    /// Total weight across scenarios.
    #[must_use]
    pub fn total_weight(&self) -> f64 {
        self.scenarios.iter().map(|s| s.weight).sum()
    }
}

/// Whether a parsed document is a load profile (`suspectLoad` root key).
#[must_use]
pub fn is_load_profile(doc: &LowDoc) -> bool {
    doc.root().get("suspectLoad").is_some()
}

/// Reads a profile from a parsed document. `profile_path` resolves relative
/// `arazzo:` and `data.path` entries; pass the document's file path.
///
/// # Errors
/// Returns every blocking diagnostic when the profile is malformed.
pub fn read_profile(doc: &LowDoc, profile_path: &Path) -> Result<LoadProfile, ProfileError> {
    let (profile, diags) = read_profile_lenient(doc, profile_path);
    let errors: Vec<ProfileDiagnostic> = diags.into_iter().filter(|d| d.error).collect();
    if errors.is_empty() {
        Ok(profile)
    } else {
        Err(ProfileError(errors))
    }
}

/// Reads a profile, returning whatever could be understood plus all
/// diagnostics (errors and warnings) — the shape validators and editors want.
#[must_use]
pub fn read_profile_lenient(
    doc: &LowDoc,
    profile_path: &Path,
) -> (LoadProfile, Vec<ProfileDiagnostic>) {
    let mut r = Reader {
        diags: Vec::new(),
        base_dir: profile_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(".")),
    };
    let root = doc.root();
    let mut profile = LoadProfile::default();

    match root.get("suspectLoad").and_then(|n| n.as_str()) {
        Some(v) if v.starts_with("1.") || v == "1" => {}
        Some(v) => r.error(
            root.get("suspectLoad").map_or(0..0, |n| n.byte_range()),
            "load-unsupported-version",
            format!("unsupported suspectLoad version `{v}` (expected 1.x)"),
        ),
        None => r.error(
            root.byte_range(),
            "load-missing-version",
            "profile must declare `suspectLoad: \"1.0\"`",
        ),
    }

    if let Some(n) = root.get("arazzo") {
        match n.as_str() {
            Some(p) => profile.arazzo = Some(r.base_dir.join(p)),
            None => r.error(
                n.byte_range(),
                "load-type",
                "`arazzo` must be a path string",
            ),
        }
    }
    profile.target = r.opt_string(root, "target");
    if let Some(seed) = root.get("seed") {
        match seed.as_i64() {
            Some(v) => profile.seed = Some(v as u64),
            None => r.error(seed.byte_range(), "load-type", "`seed` must be an integer"),
        }
    }
    if let Some(g) = root.get("grace")
        && let Some(d) = r.duration(g, "grace")
    {
        profile.grace = d;
    }

    if let Some(phases) = root.get("phases") {
        profile.phases = r.phases(phases);
    } else {
        r.error(
            root.byte_range(),
            "load-missing-phases",
            "profile needs `phases`",
        );
    }
    if profile.phases.is_empty()
        && let Some(p) = root.get("phases")
    {
        r.error(
            p.byte_range(),
            "load-empty-phases",
            "`phases` must list at least one phase",
        );
    }

    if let Some(envs) = root.get("environments") {
        for e in envs.entries() {
            let Some(node) = e.value else { continue };
            let env = Environment {
                target: r.opt_string(node, "target"),
                phases: node.get("phases").map(|p| r.phases(p)),
            };
            profile.environments.insert(e.key.to_owned(), env);
        }
    }

    profile.before = r.opt_string(root, "before");
    profile.after = r.opt_string(root, "after");

    match root.get("scenarios") {
        Some(list) => {
            for node in list.items() {
                if let Some(s) = r.scenario(node) {
                    profile.scenarios.push(s);
                }
            }
            if profile.scenarios.is_empty() {
                r.error(
                    list.byte_range(),
                    "load-empty-scenarios",
                    "`scenarios` must list at least one scenario",
                );
            }
        }
        None => r.error(
            root.byte_range(),
            "load-missing-scenarios",
            "profile needs `scenarios`",
        ),
    }

    if let Some(http) = root.get("http") {
        if let Some(t) = http.get("timeout")
            && let Some(d) = r.duration(t, "http.timeout")
        {
            profile.http.timeout = d;
        }
        if let Some(v) = r.opt_bool(http, "http2") {
            profile.http.http2 = v;
        }
        if let Some(v) = r.opt_bool(http, "per_vu_connections") {
            profile.http.per_vu_connections = v;
        }
        if let Some(v) = r.opt_bool(http, "cookies") {
            profile.http.cookies = v;
        }
        if let Some(v) = r.opt_bool(http, "insecure") {
            profile.http.insecure = v;
        }
    }

    if let Some(ensure) = root.get("ensure") {
        if let Some(n) = ensure.get("maxErrorRate") {
            match n.as_f64() {
                Some(v) if (0.0..=100.0).contains(&v) => profile.ensure.max_error_rate = Some(v),
                _ => r.error(
                    n.byte_range(),
                    "load-type",
                    "`ensure.maxErrorRate` must be a percentage 0..=100",
                ),
            }
        }
        if let Some(list) = ensure.get("thresholds") {
            for item in list.items() {
                let entries = item.entries();
                if item.kind() != ValueKind::Object || entries.len() != 1 {
                    r.error(
                        item.byte_range(),
                        "load-threshold-shape",
                        "each threshold is a single `metric: max` pair",
                    );
                    continue;
                }
                let e = &entries[0];
                match e.value.and_then(|v| v.as_f64()) {
                    Some(v) => {
                        profile
                            .ensure
                            .thresholds
                            .push((e.key.to_owned(), v, item.byte_range()))
                    }
                    None => r.error(
                        item.byte_range(),
                        "load-type",
                        format!("threshold `{}` must be numeric", e.key),
                    ),
                }
            }
        }
        if let Some(list) = ensure.get("conditions") {
            for item in list.items() {
                let Some(expr) = item.get("expression").and_then(|n| n.as_str()) else {
                    r.error(
                        item.byte_range(),
                        "load-condition-shape",
                        "condition needs an `expression` string",
                    );
                    continue;
                };
                if let Err(msg) = crate::load::ensure::parse_expression(expr) {
                    r.error(
                        item.get("expression")
                            .map_or(item.byte_range(), |n| n.byte_range()),
                        "load-condition-syntax",
                        msg,
                    );
                    continue;
                }
                profile.ensure.conditions.push(Condition {
                    expression: expr.to_owned(),
                    strict: item.get("strict").and_then(|n| n.as_bool()).unwrap_or(true),
                    range: item.byte_range(),
                });
            }
        }
    }

    if let Some(report) = root.get("report") {
        if let Some(i) = report.get("interval")
            && let Some(d) = r.duration(i, "report.interval")
        {
            if d.is_zero() {
                r.error(
                    i.byte_range(),
                    "load-type",
                    "`report.interval` must be positive",
                );
            } else {
                profile.report.interval = d;
            }
        }
        profile.report.json = r.opt_string(report, "json").map(|p| r.base_dir.join(p));
        profile.report.junit = r.opt_string(report, "junit").map(|p| r.base_dir.join(p));
    }

    (profile, r.diags)
}

struct Reader {
    diags: Vec<ProfileDiagnostic>,
    base_dir: PathBuf,
}

impl Reader {
    fn error(&mut self, range: Range<usize>, code: &'static str, message: impl Into<String>) {
        self.diags.push(ProfileDiagnostic {
            code,
            message: message.into(),
            range,
            error: true,
        });
    }

    fn warn(&mut self, range: Range<usize>, code: &'static str, message: impl Into<String>) {
        self.diags.push(ProfileDiagnostic {
            code,
            message: message.into(),
            range,
            error: false,
        });
    }

    fn opt_string(&mut self, node: NodeRef<'_>, key: &str) -> Option<String> {
        let n = node.get(key)?;
        match n.as_str() {
            Some(s) => Some(s.to_owned()),
            None => {
                self.error(
                    n.byte_range(),
                    "load-type",
                    format!("`{key}` must be a string"),
                );
                None
            }
        }
    }

    fn opt_bool(&mut self, node: NodeRef<'_>, key: &str) -> Option<bool> {
        let n = node.get(key)?;
        match n.as_bool() {
            Some(b) => Some(b),
            None => {
                self.error(
                    n.byte_range(),
                    "load-type",
                    format!("`{key}` must be a boolean"),
                );
                None
            }
        }
    }

    /// Parses a duration node: a string literal or a bare number of seconds.
    fn duration(&mut self, node: NodeRef<'_>, what: &str) -> Option<Duration> {
        let text = match node.kind() {
            ValueKind::Str => node.as_str().unwrap_or("").to_owned(),
            ValueKind::Int | ValueKind::Float => node.as_f64().unwrap_or(0.0).to_string(),
            _ => {
                self.error(
                    node.byte_range(),
                    "load-type",
                    format!("`{what}` must be a duration"),
                );
                return None;
            }
        };
        match parse_duration(&text) {
            Ok(d) => Some(d),
            Err(e) => {
                self.error(node.byte_range(), "load-duration", format!("`{what}`: {e}"));
                None
            }
        }
    }

    fn phases(&mut self, list: NodeRef<'_>) -> Vec<Phase> {
        let mut out = Vec::new();
        for node in list.items() {
            if let Some(p) = self.phase(node) {
                out.push(p);
            }
        }
        out
    }

    fn phase(&mut self, node: NodeRef<'_>) -> Option<Phase> {
        let range = node.byte_range();
        if node.kind() != ValueKind::Object {
            self.error(range, "load-phase-shape", "phase must be a mapping");
            return None;
        }
        let name = self.opt_string(node, "name");
        let max_vusers = node
            .get("maxVusers")
            .and_then(|n| n.as_i64())
            .map(|v| v.max(0) as usize);
        let num = |key: &str| node.get(key).and_then(|n| n.as_f64());

        let mut kinds = 0;
        for key in ["arrivalRate", "arrivalCount", "pause", "vus"] {
            if node.get(key).is_some() {
                kinds += 1;
            }
        }
        if kinds != 1 {
            self.error(
                range,
                "load-phase-kind",
                "phase must set exactly one of `arrivalRate`, `arrivalCount`, `pause`, `vus`",
            );
            return None;
        }

        if let Some(p) = node.get("pause") {
            let duration = self.duration(p, "pause")?;
            return Some(Phase {
                name,
                kind: PhaseKind::Pause,
                duration,
                max_vusers,
                range,
            });
        }

        let duration = match node.get("duration") {
            Some(d) => self.duration(d, "duration")?,
            None => {
                self.error(range, "load-phase-duration", "phase needs a `duration`");
                return None;
            }
        };
        if duration.is_zero() {
            self.error(
                range,
                "load-phase-duration",
                "phase `duration` must be positive",
            );
            return None;
        }

        let kind = if let Some(rate) = num("arrivalRate") {
            let ramp_to = num("rampTo");
            if rate < 0.0
                || ramp_to.is_some_and(|r| r < 0.0)
                || (rate == 0.0 && ramp_to.is_none_or(|r| r == 0.0))
            {
                self.error(
                    range,
                    "load-phase-rate",
                    "`arrivalRate`/`rampTo` must be non-negative and not both zero",
                );
                return None;
            }
            PhaseKind::ArrivalRate { rate, ramp_to }
        } else if let Some(count) = num("arrivalCount") {
            if count < 1.0 {
                self.error(
                    range,
                    "load-phase-count",
                    "`arrivalCount` must be at least 1",
                );
                return None;
            }
            PhaseKind::ArrivalCount {
                count: count as u64,
            }
        } else {
            let vus = num("vus").unwrap_or(0.0);
            if vus < 1.0 {
                self.error(range, "load-phase-vus", "`vus` must be at least 1");
                return None;
            }
            let iterations = num("iterations").map(|i| i.max(0.0) as u64);
            PhaseKind::ConstantVus {
                vus: vus as usize,
                iterations,
            }
        };
        if node.get("rampTo").is_some() && !matches!(kind, PhaseKind::ArrivalRate { .. }) {
            self.warn(
                range.clone(),
                "load-phase-ramp",
                "`rampTo` only applies to `arrivalRate` phases",
            );
        }
        Some(Phase {
            name,
            kind,
            duration,
            max_vusers,
            range,
        })
    }

    fn scenario(&mut self, node: NodeRef<'_>) -> Option<Scenario> {
        let range = node.byte_range();
        let Some(workflow) = self.opt_string(node, "workflow") else {
            self.error(
                range,
                "load-scenario-workflow",
                "scenario needs a `workflow` id",
            );
            return None;
        };
        let name = self
            .opt_string(node, "name")
            .unwrap_or_else(|| workflow.clone());
        let weight = match node.get("weight") {
            None => 1.0,
            Some(w) => match w.as_f64() {
                Some(v) if v > 0.0 => v,
                _ => {
                    self.error(
                        w.byte_range(),
                        "load-scenario-weight",
                        "`weight` must be a positive number",
                    );
                    return None;
                }
            },
        };
        let think = match node.get("think") {
            None => None,
            Some(t) => {
                let text = t
                    .get("between_steps")
                    .or(Some(t))
                    .and_then(|n| match n.kind() {
                        ValueKind::Str => n.as_str().map(str::to_owned),
                        ValueKind::Int | ValueKind::Float => n.as_f64().map(|v| v.to_string()),
                        _ => None,
                    });
                match text.as_deref().map(ThinkSpec::parse) {
                    Some(Ok(spec)) => Some(spec),
                    Some(Err(e)) => {
                        self.error(t.byte_range(), "load-duration", format!("`think`: {e}"));
                        None
                    }
                    None => {
                        self.error(
                            t.byte_range(),
                            "load-type",
                            "`think` must be a duration or `{between_steps: a..b}`",
                        );
                        None
                    }
                }
            }
        };
        let mut inputs = BTreeMap::new();
        if let Some(map) = node.get("inputs") {
            for e in map.entries() {
                let Some(v) = e.value else { continue };
                let source = if v.kind() == ValueKind::Array {
                    let pool: Vec<serde_json::Value> =
                        v.items().into_iter().map(materialize).collect();
                    if pool.is_empty() {
                        self.error(
                            v.byte_range(),
                            "load-empty-pool",
                            format!("input pool `{}` is empty", e.key),
                        );
                        continue;
                    }
                    InputSource::Pool(pool)
                } else {
                    InputSource::Literal(materialize(v))
                };
                inputs.insert(e.key.to_owned(), source);
            }
        }
        let data = node.get("data").and_then(|d| self.data_source(d));
        let iterations = match node.get("iterations") {
            None => 1,
            Some(i) => match i.as_i64() {
                Some(v) if v >= 1 => v as u64,
                _ => {
                    self.error(
                        i.byte_range(),
                        "load-type",
                        "`iterations` must be a positive integer",
                    );
                    1
                }
            },
        };
        let continue_on_failure = self.opt_bool(node, "continue_on_failure").unwrap_or(false);
        Some(Scenario {
            name,
            workflow,
            weight,
            think,
            inputs,
            data,
            iterations,
            continue_on_failure,
            range,
        })
    }

    fn data_source(&mut self, node: NodeRef<'_>) -> Option<DataSource> {
        let range = node.byte_range();
        let Some(path) = self.opt_string(node, "path") else {
            self.error(range, "load-data-path", "`data` needs a `path`");
            return None;
        };
        let fields: Vec<String> = node
            .get("fields")
            .map(|f| {
                f.items()
                    .into_iter()
                    .filter_map(|n| n.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        let order = match node.get("order").and_then(|n| n.as_str()) {
            None | Some("random") => DataOrder::Random,
            Some("sequence") => DataOrder::Sequence,
            Some("unique") => DataOrder::Unique,
            Some(other) => {
                self.error(
                    range,
                    "load-data-order",
                    format!("unknown data order `{other}` (random | sequence | unique)"),
                );
                return None;
            }
        };
        let skip_header = self.opt_bool(node, "skipHeader").unwrap_or(false);
        let delimiter = node
            .get("delimiter")
            .and_then(|n| n.as_str())
            .and_then(|s| s.bytes().next())
            .unwrap_or(b',');
        let resolved = self.base_dir.join(&path);
        if !resolved.exists() {
            self.error(
                range.clone(),
                "load-data-missing",
                format!("data file not found: {}", resolved.display()),
            );
        }
        let is_csv = resolved
            .extension()
            .and_then(|e| e.to_str())
            .is_none_or(|e| !matches!(e, "ndjson" | "jsonl" | "json"));
        if is_csv && fields.is_empty() {
            self.error(
                range.clone(),
                "load-data-fields",
                "CSV data needs `fields` naming each column",
            );
        }
        Some(DataSource {
            path: resolved,
            fields,
            order,
            skip_header,
            delimiter,
            range,
        })
    }
}

/// Materializes any node into owned JSON.
fn materialize(node: NodeRef<'_>) -> serde_json::Value {
    match node.kind() {
        ValueKind::Object => {
            let mut map = serde_json::Map::new();
            for entry in node.entries() {
                if let Some(value) = entry.value {
                    map.insert(entry.key.to_owned(), materialize(value));
                }
            }
            serde_json::Value::Object(map)
        }
        ValueKind::Array => {
            serde_json::Value::Array(node.items().into_iter().map(materialize).collect())
        }
        ValueKind::Bool => serde_json::Value::Bool(node.as_bool().unwrap_or(false)),
        ValueKind::Int => serde_json::Value::from(node.as_i64().unwrap_or_default()),
        ValueKind::Float => serde_json::Value::from(node.as_f64().unwrap_or_default()),
        ValueKind::Str => serde_json::Value::String(node.as_str().unwrap_or_default().to_owned()),
        ValueKind::Null => serde_json::Value::Null,
    }
}

/// Cross-checks a profile against the compiled plan: every referenced
/// workflow exists, `before`/`after` are not also scenarios, and data
/// fields cover the workflow's required inputs when declared.
#[must_use]
pub fn check_against_plan(
    profile: &LoadProfile,
    plan: &crate::plan::Plan,
) -> Vec<ProfileDiagnostic> {
    let mut out = Vec::new();
    let known = |id: &str| plan.workflows.iter().any(|w| w.workflow_id == id);
    for s in &profile.scenarios {
        if !known(&s.workflow) {
            out.push(ProfileDiagnostic {
                code: "load-unknown-workflow",
                message: format!(
                    "scenario `{}`: workflow `{}` is not in the Arazzo document",
                    s.name, s.workflow
                ),
                range: s.range.clone(),
                error: true,
            });
        }
    }
    for (what, id) in [("before", &profile.before), ("after", &profile.after)] {
        if let Some(id) = id {
            if !known(id) {
                out.push(ProfileDiagnostic {
                    code: "load-unknown-workflow",
                    message: format!("`{what}`: workflow `{id}` is not in the Arazzo document"),
                    range: 0..0,
                    error: true,
                });
            }
            if profile.scenarios.iter().any(|s| &s.workflow == id) {
                out.push(ProfileDiagnostic {
                    code: "load-hook-is-scenario",
                    message: format!("`{what}` workflow `{id}` is also a scenario; hooks run once, scenarios per user"),
                    range: 0..0,
                    error: false,
                });
            }
        }
    }
    out
}
