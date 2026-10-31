//! `suspect check`: every check the editor runs, under one policy.
//!
//! The pass is `suspect_check::check_document` plus TypeScript rules,
//! then `suspect_check::apply_policy_report` per document: the same
//! functions the language server calls, so the terminal, CI and the
//! editor agree on codes, severities and ranges. `validate` and `lint`
//! run this pass with a check filter ([`VALIDATE`], [`LINT`]), and so do
//! the `ci` check stages. A findings baseline ([`baseline`]) then hides
//! recorded findings before the exit status is computed.

pub mod baseline;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{Args, ValueEnum};
use rayon::prelude::*;
use serde_json::{Value, json};
use suspect_check::{CheckSet, Context, Policy, Registry};
use suspect_config::{Layer, Raw, Resolved};
use suspect_diag::render::{self, Sources};
use suspect_diag::{CheckId, Finding, Severity};
use suspect_ref::{Workspace, WorkspaceBuilder};
use suspect_source::Uri;

use crate::{Findings, FindingsFormat};
use baseline::Baseline;

/// What `suspect validate` runs: document structure and semantics.
pub const VALIDATE: &[CheckId] = &[
    CheckId::Syntax,
    CheckId::Ref,
    CheckId::Validate,
    CheckId::Swagger,
    CheckId::Arazzo,
    CheckId::Overlay,
    CheckId::LoadProfile,
    CheckId::Config,
    CheckId::Extension,
];

/// What `suspect lint` runs: style rules, markdown and TypeScript rules.
pub const LINT: &[CheckId] = &[CheckId::Lint, CheckId::Markdown, CheckId::Rules];

/// What `--parse-only` runs.
pub const PARSE: &[CheckId] = &[CheckId::Syntax, CheckId::Ref];

/// The checks `--only` accepts, in help order.
const SELECTABLE: [CheckId; 13] = [
    CheckId::Syntax,
    CheckId::Ref,
    CheckId::Validate,
    CheckId::Swagger,
    CheckId::Arazzo,
    CheckId::Overlay,
    CheckId::LoadProfile,
    CheckId::Config,
    CheckId::Extension,
    CheckId::Lint,
    CheckId::Markdown,
    CheckId::Rules,
    CheckId::Admission,
];

/// One `--only` value: a check by its kebab-case name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckName(pub CheckId);

impl ValueEnum for CheckName {
    fn value_variants<'a>() -> &'a [Self] {
        const NAMES: [CheckName; SELECTABLE.len()] = {
            let mut names = [CheckName(CheckId::Syntax); SELECTABLE.len()];
            let mut index = 0;
            while index < names.len() {
                names[index] = CheckName(SELECTABLE[index]);
                index += 1;
            }
            names
        };
        &NAMES
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        Some(clap::builder::PossibleValue::new(self.0.name()))
    }
}

/// `suspect check` arguments.
#[derive(Debug, Args)]
pub struct CheckArgs {
    /// Documents to check.
    #[arg(required = true)]
    pub paths: Vec<PathBuf>,
    /// Run only these checks (comma-separated or repeated).
    #[arg(long, value_enum, value_delimiter = ',', value_name = "CHECK")]
    pub only: Vec<CheckName>,
    /// Only parse and resolve `$ref`s: `--only syntax,ref`.
    #[arg(long, conflicts_with = "only")]
    pub parse_only: bool,
    /// Report SDK-readiness findings and run SDK admission (default: when
    /// `.suspect.yaml` or the project manifest configures codegen).
    #[arg(long, overrides_with = "no_sdk")]
    pub sdk: bool,
    /// Do not report SDK-readiness findings, even with codegen configured.
    #[arg(long, overrides_with = "sdk")]
    pub no_sdk: bool,
    /// Ruleset document (default: `lint.ruleset`, else the built-in rules).
    #[arg(long)]
    pub ruleset: Option<PathBuf>,
    /// Report only findings at or above this severity (default: hint, or
    /// `lint.min_severity`).
    #[arg(long)]
    pub min_severity: Option<crate::output::Severity>,
    /// Assert JSON-Schema `format` keywords (`validate.strict_format`).
    #[arg(long)]
    pub strict_format: bool,
    /// JSON array of absolute document paths; deny every unlisted reference load.
    #[arg(long)]
    pub reference_allowlist: Option<PathBuf>,
    /// Findings baseline: findings recorded there are hidden and do not
    /// fail the run (default: `check.baseline`).
    #[arg(long, value_name = "FILE")]
    pub baseline: Option<PathBuf>,
    /// Write every current finding to this baseline file and exit 0.
    #[arg(long, value_name = "FILE", conflicts_with = "baseline")]
    pub write_baseline: Option<PathBuf>,
    /// Output format for the finding list.
    #[command(flatten)]
    pub findings: Findings,
}

/// `suspect check`.
///
/// # Errors
/// Configuration, ruleset, TypeScript rule and baseline errors (exit 2).
pub fn check(args: CheckArgs) -> anyhow::Result<i32> {
    let only: Vec<CheckId> = if args.parse_only {
        PARSE.to_vec()
    } else {
        args.only.iter().map(|c| c.0).collect()
    };
    let flags = Layer::flags()
        .set("lint.ruleset", args.ruleset.map(Raw::Path), "--ruleset")
        .set(
            "lint.min_severity",
            args.min_severity.map(|s| Raw::Str(s.label().to_owned())),
            "--min-severity",
        )
        // A clap bool cannot say "explicitly false": only a passed flag
        // enters the flag layer.
        .set(
            "validate.strict_format",
            args.strict_format.then_some(Raw::Bool(true)),
            "--strict-format",
        )
        .set(
            "validate.reference_allowlist",
            args.reference_allowlist.map(Raw::Path),
            "--reference-allowlist",
        )
        .set("check.baseline", args.baseline.map(Raw::Path), "--baseline");
    let sdk = if args.sdk {
        Some(true)
    } else if args.no_sdk {
        Some(false)
    } else {
        None
    };
    command(&Request {
        paths: &args.paths,
        flags,
        rules: only.contains(&CheckId::Rules).then_some(true),
        checks: (!only.is_empty()).then_some(only),
        sdk,
        write_baseline: args.write_baseline,
        format: args.findings.format,
    })
}

/// One run of the check pass from the command line.
pub struct Request<'a> {
    /// Documents to check.
    pub paths: &'a [PathBuf],
    /// The flag layer, on top of the resolved configuration.
    pub flags: Layer,
    /// Checks to run; `None` runs every check.
    pub checks: Option<Vec<CheckId>>,
    /// Whether TypeScript rules run; `None` follows `rules.enabled`.
    pub rules: Option<bool>,
    /// `--sdk` / `--no-sdk`; `None` follows the configuration.
    pub sdk: Option<bool>,
    /// `--write-baseline`.
    pub write_baseline: Option<PathBuf>,
    /// Output format.
    pub format: FindingsFormat,
}

/// Resolves the configuration, runs the pass, applies the baseline and
/// prints. Returns 1 when an error-severity finding remains.
///
/// # Errors
/// Configuration, ruleset, TypeScript rule and baseline errors (exit 2).
pub fn command(request: &Request<'_>) -> anyhow::Result<i32> {
    let resolved = crate::settings::resolve(request.paths, request.flags.clone())?;
    let options = Options::new(
        &resolved,
        request.checks.as_deref(),
        request.rules,
        request.sdk,
    );
    let outcome = run(request.paths, &resolved, &options)?;
    if let Some(path) = &request.write_baseline {
        let count = baseline::write(path, &outcome.findings)?;
        eprintln!("wrote {count} finding(s) to {}", path.display());
        return Ok(0);
    }
    let baseline = resolved
        .settings
        .check
        .baseline
        .as_deref()
        .map(Baseline::read)
        .transpose()?;
    let shown = Shown::new(&outcome, baseline.as_ref());
    print(&outcome, &shown, request.format)?;
    Ok(i32::from(shown.failed()))
}

/// What one run checks and how.
#[derive(Debug, Clone)]
pub struct Options {
    /// Checks that run.
    pub checks: CheckSet,
    /// Whether TypeScript rules run.
    pub rules: bool,
    /// Overrides [`Policy::sdk`] when set.
    pub sdk: Option<bool>,
}

impl Options {
    /// Options for `checks` (every check when `None`) under `resolved`:
    /// TypeScript rules run when `rules` says so, else when `rules.enabled`
    /// is set and the filter includes them.
    #[must_use]
    pub fn new(
        resolved: &Resolved,
        checks: Option<&[CheckId]>,
        rules: Option<bool>,
        sdk: Option<bool>,
    ) -> Self {
        let checks = checks.map_or_else(CheckSet::all, |c| CheckSet::only(c.iter().copied()));
        let rules =
            rules.unwrap_or(resolved.settings.rules.enabled) && checks.contains(CheckId::Rules);
        Self { checks, rules, sdk }
    }
}

/// The findings of one run, after policy and before the baseline.
pub struct Outcome {
    /// Findings to report, sorted by path, position and code.
    pub findings: Vec<Finding>,
    /// Findings inline or extension suppressions hid.
    pub suppressed: Vec<Finding>,
    /// Display paths and positions for every document.
    pub documents: Documents,
    /// The registry the run used (builtin codes plus TypeScript rule ids).
    pub registry: Arc<Registry>,
}

impl Outcome {
    /// Whether any finding is an error.
    #[must_use]
    pub fn has_error(&self) -> bool {
        self.findings.iter().any(|f| f.severity == Severity::Error)
    }
}

/// Runs the check pass over `paths` under `resolved`.
///
/// Each input gets its own reference workspace, checked in parallel. An
/// input that cannot be read is an `io-error` finding, so one missing file
/// does not hide the others' results.
///
/// # Errors
/// A ruleset that does not compile, a reference allowlist that cannot be
/// read, TypeScript rule ids that collide with builtin codes, and a rule
/// runtime that fails to start.
pub fn run(paths: &[PathBuf], resolved: &Resolved, options: &Options) -> anyhow::Result<Outcome> {
    let mut policy = Policy::from_resolved(resolved);
    policy.checks = options.checks.clone();
    if let Some(sdk) = options.sdk {
        policy.sdk = sdk;
    }
    // The editor falls back to the builtin packs on a broken ruleset; the
    // command line says so instead.
    if let Some(ruleset) = &resolved.settings.lint.ruleset
        && (policy.checks.contains(CheckId::Lint) || policy.checks.contains(CheckId::Markdown))
    {
        policy.linter = Arc::new(crate::commands::lint::build_linter(Some(ruleset))?);
    }
    let allowed = resolved
        .settings
        .validate
        .reference_allowlist
        .as_deref()
        .map(crate::commands::validate::load_reference_allowlist)
        .transpose()?;
    let settings = &resolved.settings;
    let inputs: Vec<Result<Input, Box<Finding>>> = paths
        .par_iter()
        .map(|path| {
            Input::load(
                path,
                &resolved.root,
                settings.refs.max_docs,
                allowed.as_deref(),
            )
        })
        .collect();
    let mut raw: Vec<Vec<Finding>> = inputs
        .par_iter()
        .map(|input| match input {
            Ok(input) => {
                let cx = Context {
                    workspace: Some(&input.ws),
                    policy: &policy,
                };
                let mut found = suspect_check::check_document(&cx, input.doc());
                // Syntax errors anywhere in the loaded `$ref` closure, as
                // `validate` always reported them; the editor reports
                // them when each document is open.
                if policy.checks.contains(CheckId::Syntax) {
                    for uri in input.ws.uris() {
                        if uri != input.uri
                            && let Some(handle) = input.ws.get(&uri)
                        {
                            found.extend(suspect_check::checks::syntax(handle.doc()));
                        }
                    }
                }
                found
            }
            Err(finding) => vec![(**finding).clone()],
        })
        .collect();
    if options.rules {
        let loaded: Vec<(usize, &Input)> = inputs
            .iter()
            .enumerate()
            .filter_map(|(i, input)| input.as_ref().ok().map(|input| (i, input)))
            .collect();
        let ts = ts_rules(resolved, &loaded)?;
        for (index, findings) in ts.findings {
            raw[index].extend(findings);
        }
        if let Some(registry) = ts.registry {
            policy.registry = registry;
        }
    }
    let mut documents = Documents::default();
    let mut findings = Vec::new();
    let mut suppressed = Vec::new();
    for ((path, input), raw) in paths.iter().zip(&inputs).zip(raw) {
        let Ok(input) = input else {
            for f in &raw {
                documents
                    .shown
                    .insert(f.document.clone(), path.display().to_string());
            }
            findings.extend(raw);
            continue;
        };
        documents.add(input);
        let (kept, hidden) = input.apply_policy(raw, &policy);
        findings.extend(kept);
        suppressed.extend(hidden);
    }
    let mut seen = HashSet::new();
    findings.retain(|f| seen.insert(identity(f)));
    let mut seen = HashSet::new();
    suppressed.retain(|f| seen.insert(identity(f)));
    documents.sort(&mut findings);
    documents.sort(&mut suppressed);
    Ok(Outcome {
        findings,
        suppressed,
        documents,
        registry: policy.registry,
    })
}

/// Identical findings from two inputs (a shared referenced document) are
/// reported once.
fn identity(f: &Finding) -> (String, Option<(usize, usize)>, String, String, Severity) {
    (
        f.document.as_str().to_owned(),
        f.range.as_ref().map(|r| (r.start, r.end)),
        f.code.to_string(),
        f.message.clone(),
        f.severity,
    )
}

/// One loaded input document.
struct Input {
    /// The path as given.
    shown: String,
    ws: Arc<Workspace>,
    uri: Uri,
}

impl Input {
    fn load(
        path: &Path,
        root: &Path,
        max_docs: usize,
        allowed: Option<&[Uri]>,
    ) -> Result<Self, Box<Finding>> {
        let shown = path.display().to_string();
        let fail = |message: String| {
            Box::new(Finding::new(
                CheckId::Command,
                "io-error",
                Severity::Error,
                Uri::from(shown.as_str()),
                message,
            ))
        };
        let absolute = path
            .canonicalize()
            .map_err(|e| fail(format!("cannot read {shown}: {e}")))?;
        let entry = absolute
            .to_str()
            .ok_or_else(|| fail(format!("{shown}: path is not UTF-8")))?;
        let mut builder = WorkspaceBuilder::new().root(root).max_docs(max_docs);
        if let Some(documents) = allowed {
            builder = builder.allowed_documents(documents.iter().cloned());
        }
        let ws = Arc::new(builder.build().map_err(|e| fail(e.to_string()))?);
        let uri = ws
            .open(entry)
            .map_err(|e| fail(format!("cannot load {shown}: {e}")))?
            .uri()
            .clone();
        Ok(Self { shown, ws, uri })
    }

    fn doc(&self) -> &suspect_low::LowDoc {
        // Loaded by `load`; a workspace never unloads a document.
        self.ws
            .get(&self.uri)
            .map(|handle| handle.doc())
            .expect("the entry is loaded")
    }

    /// Policy per document: a finding validation reported in a referenced
    /// document is canonicalized, suppressed and pointed against that
    /// document.
    fn apply_policy(&self, raw: Vec<Finding>, policy: &Policy) -> (Vec<Finding>, Vec<Finding>) {
        let mut by_doc: Vec<(Uri, Vec<Finding>)> = Vec::new();
        for finding in raw {
            match by_doc.iter_mut().find(|(uri, _)| *uri == finding.document) {
                Some((_, group)) => group.push(finding),
                None => by_doc.push((finding.document.clone(), vec![finding])),
            }
        }
        let mut kept = Vec::new();
        let mut hidden = Vec::new();
        for (uri, group) in by_doc {
            let Some(handle) = self.ws.get(&uri) else {
                kept.extend(group);
                continue;
            };
            let doc = handle.doc();
            let outcome = suspect_check::apply_policy_report(group, doc, policy);
            for mut finding in outcome.findings {
                point(&mut finding, doc);
                kept.push(finding);
            }
            for mut finding in outcome.suppressed {
                point(&mut finding, doc);
                hidden.push(finding);
            }
        }
        (kept, hidden)
    }
}

/// Gives a located finding without a producer pointer the pointer of its
/// range; a document-level finding gets `""`.
fn point(finding: &mut Finding, doc: &suspect_low::LowDoc) {
    if finding.pointer.is_none() {
        finding.pointer = Some(
            finding
                .range
                .as_ref()
                .map_or_else(|| "".into(), |range| suspect_check::pointer_at(doc, range)),
        );
    }
}

/// Per-document deadline for TypeScript rules in a batch run.
const RULE_TIMEOUT_MS: u64 = 10_000;

struct TsOutcome {
    findings: Vec<(usize, Vec<Finding>)>,
    registry: Option<Arc<Registry>>,
}

/// TypeScript rules over every loaded input. No rule files: nothing. No
/// Bun: a `rules-runtime-unavailable` warning per input, and the run goes
/// on.
fn ts_rules(resolved: &Resolved, inputs: &[(usize, &Input)]) -> anyhow::Result<TsOutcome> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let roots = &resolved.settings.rules.roots;
        let mut out = TsOutcome {
            findings: Vec::new(),
            registry: None,
        };
        // A batch run is not latency-bound: allow a cold worker cache.
        match suspect_check::rules::start(&resolved.root, roots, Some(RULE_TIMEOUT_MS)).await {
            suspect_check::rules::Startup::NoRules => {}
            suspect_check::rules::Startup::Unavailable { files } => {
                for (index, input) in inputs {
                    out.findings.push((
                        *index,
                        vec![suspect_check::rules::runtime_unavailable(
                            input.uri.clone(),
                            files.len(),
                        )],
                    ));
                }
            }
            suspect_check::rules::Startup::Failed(error) => {
                anyhow::bail!("TypeScript rules failed to start: {error}");
            }
            suspect_check::rules::Startup::Started(mut host) => {
                let collisions = suspect_check::rules::collisions(
                    suspect_check::registry::builtin(),
                    host.rules(),
                );
                if !collisions.is_empty() {
                    host.shutdown().await.ok();
                    let list: Vec<String> = collisions.iter().map(ToString::to_string).collect();
                    anyhow::bail!("{}", list.join("\n"));
                }
                let ids = host
                    .rules()
                    .iter()
                    .map(|rule| (rule.id.clone(), CheckId::Rules))
                    .collect::<Vec<_>>();
                out.registry = Some(Arc::new(Registry::builtin().with_dynamic(ids)?));
                for (index, input) in inputs {
                    let found = suspect_check::check_rules(host.as_mut(), input.doc())
                        .await
                        .map_err(|e| anyhow::anyhow!("TypeScript rules: {e}"))?;
                    out.findings.push((*index, found));
                }
                let disabled = host.disabled();
                if !disabled.is_empty() {
                    let list: Vec<&str> = disabled.iter().map(String::as_str).collect();
                    eprintln!("TypeScript rules disabled this run: {}", list.join(", "));
                }
                host.shutdown().await.ok();
            }
        }
        Ok(out)
    })
}

/// Display paths and positions for the documents of a run.
#[derive(Default)]
pub struct Documents {
    shown: HashMap<Uri, String>,
    owner: HashMap<Uri, Arc<Workspace>>,
}

impl Documents {
    fn add(&mut self, input: &Input) {
        self.shown.insert(input.uri.clone(), input.shown.clone());
        for uri in input.ws.uris() {
            self.owner
                .entry(uri)
                .or_insert_with(|| Arc::clone(&input.ws));
        }
    }

    /// 1-based start line and column.
    #[must_use]
    pub fn start(&self, finding: &Finding) -> (u32, u32) {
        render::start(finding, self)
    }

    fn sort(&self, findings: &mut [Finding]) {
        findings.sort_by_cached_key(|f| {
            let (line, col) = self.start(f);
            (
                self.display_path(&f.document),
                line,
                col,
                f.code.to_string(),
                f.message.clone(),
            )
        });
    }
}

impl Sources for Documents {
    fn line_col(&self, doc: &Uri, offset: usize) -> Option<(u32, u32)> {
        let handle = self.owner.get(doc)?.get(doc)?;
        let low = handle.doc();
        Some(
            low.inner()
                .line_index()
                .line_col(low.inner().bytes(), offset),
        )
    }

    fn display_path(&self, doc: &Uri) -> String {
        if let Some(shown) = self.shown.get(doc) {
            return shown.clone();
        }
        doc.as_path()
            .map_or_else(|| doc.to_string(), |path| shown_path(&path))
    }
}

/// A path relative to the working directory when it lies below it.
fn shown_path(path: &Path) -> String {
    std::env::current_dir()
        .ok()
        .and_then(|cwd| cwd.canonicalize().ok())
        .and_then(|cwd| path.strip_prefix(cwd).ok().map(Path::to_path_buf))
        .unwrap_or_else(|| path.to_path_buf())
        .display()
        .to_string()
}

/// The findings a run reports once the baseline is applied.
pub struct Shown<'a> {
    outcome: &'a Outcome,
    /// Per finding: its baseline key, and whether the baseline covers it.
    marks: Vec<(baseline::Key, bool)>,
    with_baseline: bool,
}

impl<'a> Shown<'a> {
    /// Marks the findings `baseline` covers.
    #[must_use]
    pub fn new(outcome: &'a Outcome, baseline: Option<&Baseline>) -> Self {
        let base = baseline.map_or_else(baseline::default_base, Baseline::base);
        let keys: Vec<baseline::Key> = outcome
            .findings
            .iter()
            .map(|f| baseline::Key::of(f, &base))
            .collect();
        let covered = baseline.map_or_else(|| vec![false; keys.len()], |b| b.covers(&keys));
        Self {
            outcome,
            marks: keys.into_iter().zip(covered).collect(),
            with_baseline: baseline.is_some(),
        }
    }

    /// Findings the baseline does not cover.
    pub fn new_findings(&self) -> impl Iterator<Item = &'a Finding> + '_ {
        self.outcome
            .findings
            .iter()
            .zip(&self.marks)
            .filter(|(_, (_, covered))| !covered)
            .map(|(f, _)| f)
    }

    /// How many findings the baseline hides.
    #[must_use]
    pub fn baselined(&self) -> usize {
        self.marks.iter().filter(|(_, covered)| *covered).count()
    }

    /// Whether a finding the baseline does not cover is an error.
    #[must_use]
    pub fn failed(&self) -> bool {
        self.new_findings().any(|f| f.severity == Severity::Error)
    }

    /// Error and lower counts of the findings the baseline does not cover.
    #[must_use]
    pub fn counts(&self) -> (usize, usize) {
        let errors = self
            .new_findings()
            .filter(|f| f.severity == Severity::Error)
            .count();
        (errors, self.new_findings().count() - errors)
    }
}

/// Prints the findings `shown` reports in `format`.
///
/// # Errors
/// Serialization and write failures.
pub fn print(outcome: &Outcome, shown: &Shown<'_>, format: FindingsFormat) -> anyhow::Result<()> {
    let docs = &outcome.documents;
    let fresh: Vec<Finding> = shown.new_findings().cloned().collect();
    match format {
        FindingsFormat::Text => {
            let mut out = std::io::stdout().lock();
            text(&fresh, docs, &outcome.registry, &mut out)?;
            if !outcome.suppressed.is_empty() {
                eprintln!("{} suppressed finding(s) hidden", outcome.suppressed.len());
            }
            if shown.baselined() > 0 {
                eprintln!("{} baselined finding(s) hidden", shown.baselined());
            }
        }
        FindingsFormat::Json => {
            let mut report = render::json_report(&fresh, &outcome.suppressed, docs);
            if shown.with_baseline {
                report["baselined"] = json!(shown.baselined());
            }
            crate::output::print_json(&report)?;
        }
        FindingsFormat::Sarif => {
            crate::output::print_json(&sarif(outcome, shown))?;
        }
        FindingsFormat::Junit => print!("{}", render::junit(&fresh, docs)),
    }
    Ok(())
}

/// `file:line:col [SEVERITY] code: message` lines, each followed by the
/// code's `how to fix:` line when the registry has one.
///
/// # Errors
/// Write failures.
pub fn text(
    findings: &[Finding],
    docs: &Documents,
    registry: &Registry,
    out: &mut dyn std::io::Write,
) -> std::io::Result<()> {
    for finding in findings {
        render::text(std::slice::from_ref(finding), docs, out)?;
        if let Some(info) = registry.info(&finding.code)
            && !info.how_to_fix.is_empty()
        {
            writeln!(out, "how to fix: {}", info.how_to_fix)?;
        }
    }
    Ok(())
}

/// The SARIF log: every finding, with its baseline key as
/// `partialFingerprints["suspect/v1"]`, and with a baseline in use,
/// `baselineState` `new` or `unchanged` (baselined findings stay in the
/// log so code scanning keeps their alerts open).
fn sarif(outcome: &Outcome, shown: &Shown<'_>) -> Value {
    let mut log = render::sarif(&outcome.findings, &outcome.documents, &*outcome.registry);
    if let Some(results) = log["runs"][0]["results"].as_array_mut() {
        for (result, (key, covered)) in results.iter_mut().zip(&shown.marks) {
            result["partialFingerprints"] = json!({ "suspect/v1": key.fingerprint() });
            if shown.with_baseline {
                result["baselineState"] = json!(if *covered { "unchanged" } else { "new" });
            }
        }
    }
    log
}
