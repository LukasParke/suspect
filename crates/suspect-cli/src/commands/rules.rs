//! `suspect rules` — TS/JS custom rule runtime commands.
//!
//! `check` evaluates workspace TS rules against documents through the Bun
//! sidecar worker; `new` scaffolds a rule file; `bench` measures worker
//! cold start and evaluate round-trip latency.

use std::path::PathBuf;
use std::time::Instant;

use suspect_low::LowDoc;
use suspect_rules::{Error as RulesError, RuleHost, StartOptions};
use suspect_source::{Source, Uri};

use crate::output::{self, Finding, Severity};
use crate::{OutputFormat, TextFormat};

fn severity_from_str(s: Option<&str>) -> Severity {
    match s {
        Some("error") => Severity::Error,
        Some("info") => Severity::Info,
        Some("hint") => Severity::Hint,
        _ => Severity::Warning,
    }
}

fn load_doc(path: &std::path::Path) -> LowDoc {
    match std::fs::read(path) {
        Ok(bytes) => LowDoc::parse(
            Uri::from(path.to_string_lossy().into_owned().leak() as &str),
            Source::from_vec(bytes),
        ),
        Err(e) => {
            eprintln!("error: {}: {e}", path.display());
            std::process::exit(2);
        }
    }
}

fn to_findings(doc: &LowDoc, shown: &str, findings: Vec<suspect_rules::TsFinding>) -> Vec<Finding> {
    let bytes = doc.inner().bytes();
    let index = doc.inner().line_index();
    findings
        .into_iter()
        .map(|f| {
            let (line, col) = f
                .span
                .map(|(start, _)| index.line_col(bytes, start))
                .unwrap_or((1, 0));
            Finding {
                file: shown.to_owned(),
                severity: severity_from_str(f.severity.as_deref()),
                code: f.rule_id,
                message: f.message,
                line,
                col: col + 1,
            }
        })
        .collect()
}

/// `suspect rules check <documents>` — run TS rules from `.suspect/rules`
/// against each document through the Bun sidecar.
///
/// # Errors
/// Document or runtime failures.
pub fn check(
    paths: &[PathBuf],
    timeout_ms: Option<u64>,
    min_severity: Severity,
    text: TextFormat,
) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(check_async(paths, timeout_ms, min_severity, text))
}

async fn check_async(
    paths: &[PathBuf],
    timeout_ms: Option<u64>,
    min_severity: Severity,
    text: TextFormat,
) -> anyhow::Result<()> {
    let workspace_root = std::env::current_dir()?;
    let mut host = match RuleHost::start(StartOptions {
        workspace_root: workspace_root.clone(),
        rule_files: Vec::new(),
        timeout_ms,
        bun: None,
        cache_dir: None,
    })
    .await
    {
        Ok(Some(host)) => host,
        Ok(None) => {
            eprintln!(
                "no TS rules found (looked in {}) — nothing to do",
                workspace_root.join(".suspect/rules").display()
            );
            return Ok(());
        }
        Err(RulesError::BunUnavailable(msg)) => {
            eprintln!("error: {msg}");
            std::process::exit(1);
        }
        Err(e) => return Err(anyhow::anyhow!("{e}")),
    };

    eprintln!("ts rules: {} loaded (bun sidecar)", host.rules().len());

    let mut all: Vec<Finding> = Vec::new();
    for path in paths {
        let doc = load_doc(path);
        let started = Instant::now();
        let found = host
            .evaluate(&doc)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let elapsed = started.elapsed();
        eprintln!(
            "{}: {} finding(s) in {:.1}ms",
            path.display(),
            found.len(),
            elapsed.as_secs_f64() * 1000.0
        );
        all.extend(to_findings(&doc, &path.display().to_string(), found));
    }

    all.retain(|f| f.severity >= min_severity);
    match text.format {
        OutputFormat::Json => output::print_json(&all)?,
        OutputFormat::Text => output::print_findings(&all),
    }

    let disabled = host.disabled();
    if !disabled.is_empty() {
        let list: Vec<&str> = disabled.iter().map(String::as_str).collect();
        eprintln!("disabled this session: {}", list.join(", "));
    }
    host.shutdown().await.ok();
    if all.iter().any(|f| f.severity == Severity::Error) {
        std::process::exit(1);
    }
    Ok(())
}

/// `suspect rules new <name>` — scaffold a rule file under
/// `.suspect/rules`.
///
/// # Errors
/// Filesystem failures; refusing to overwrite an existing rule.
pub fn new(name: &str, dir: Option<PathBuf>) -> anyhow::Result<()> {
    let target = dir.unwrap_or_else(|| {
        std::env::current_dir()
            .unwrap_or_default()
            .join(".suspect/rules")
    });
    let path = suspect_rules::scaffold_rule(&target, name)?;
    println!("scaffolded {}", path.display());
    Ok(())
}

/// `suspect rules bench <document>` — cold start + evaluate latency.
///
/// # Errors
/// Runtime failures; missing rules.
pub fn bench(path: PathBuf, iterations: usize, timeout_ms: Option<u64>) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(bench_async(path, iterations, timeout_ms))
}

async fn bench_async(
    path: PathBuf,
    iterations: usize,
    timeout_ms: Option<u64>,
) -> anyhow::Result<()> {
    let workspace_root = std::env::current_dir()?;
    let doc = load_doc(&path);

    let started = Instant::now();
    let mut host = RuleHost::start(StartOptions {
        workspace_root: workspace_root.clone(),
        rule_files: Vec::new(),
        timeout_ms,
        bun: None,
        cache_dir: None,
    })
    .await
    .map_err(|e| anyhow::anyhow!("{e}"))?
    .ok_or_else(|| anyhow::anyhow!("no TS rules found under .suspect/rules"))?;
    let cold = started.elapsed();

    let mut samples = Vec::new();
    for _ in 0..iterations {
        let s = Instant::now();
        let n = host
            .evaluate(&doc)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .len();
        let worker_ms = host.last_worker_ms();
        samples.push((s.elapsed(), worker_ms, n));
    }
    let rule_count = host.rules().len();
    host.shutdown().await.ok();

    println!("rules: {rule_count}");
    println!(
        "cold start (stage+spawn+handshake): {:.1}ms",
        cold.as_secs_f64() * 1000.0
    );
    for (i, (elapsed, worker_ms, n)) in samples.iter().enumerate() {
        println!(
            "evaluate #{}: {:.2}ms total, {:.2}ms worker ({} findings)",
            i + 1,
            elapsed.as_secs_f64() * 1000.0,
            worker_ms,
            n
        );
    }
    Ok(())
}

/// Subcommands for `suspect rules`.
#[derive(Debug, clap::Subcommand)]
pub enum RulesCmd {
    /// Evaluate TS rules from `.suspect/rules` against documents.
    Check {
        /// Documents to evaluate.
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Per-evaluate deadline in milliseconds.
        #[arg(long, default_value_t = 250)]
        timeout_ms: u64,
        /// Report only findings at or above this severity.
        #[arg(long, default_value = "hint")]
        min_severity: Severity,
        /// Output format for the finding list.
        #[command(flatten)]
        text: TextFormat,
    },
    /// Scaffold a new TS rule file.
    New {
        /// Rule name (becomes the file name and rule id).
        name: String,
        /// Target directory (default `.suspect/rules`).
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Measure worker cold start and evaluate latency.
    Bench {
        /// Document to evaluate against.
        path: PathBuf,
        /// Warm evaluate iterations.
        #[arg(long, default_value_t = 5)]
        iterations: usize,
        /// Per-evaluate deadline (large specs need more than the 250ms
        /// interactive default).
        #[arg(long, default_value_t = 30_000)]
        timeout_ms: u64,
    },
}

impl RulesCmd {
    /// Dispatches to the subcommand handler; returns the process exit code.
    ///
    /// # Errors
    /// Whatever the selected subcommand surfaces.
    pub fn run(self) -> anyhow::Result<()> {
        match self {
            Self::Check {
                paths,
                timeout_ms,
                min_severity,
                text,
            } => check(&paths, Some(timeout_ms), min_severity, text),
            Self::New { name, dir } => new(&name, dir),
            Self::Bench {
                path,
                iterations,
                timeout_ms,
            } => bench(path, iterations, Some(timeout_ms)),
        }
    }
}
