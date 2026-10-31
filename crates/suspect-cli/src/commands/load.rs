//! `suspect test load` — Artillery-style load generation with Arazzo workflows as
//! virtual-user scenarios.
//!
//! Accepts either a load profile (`*.load.yaml`, `suspectLoad: "1.0"`) or an
//! Arazzo document plus `--workflow` and a shape (`--rate`/`--duration` or
//! `--vus`). Prerequisite workflows run once, users arrive on schedule,
//! metrics print every interval, and `ensure` decides the exit code.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use suspect_journal::{Journal, LoadSnapshotRecord, Record};
use suspect_low::LowDoc;
use suspect_source::Uri;
use suspect_test::duration::{format_duration, parse_duration};
use suspect_test::load::{
    self, IntervalReport, Jitter, LoadConfig, LoadProfile, LoadSink, LoadSummary, Phase, PhaseKind,
    Scenario, report,
};
use suspect_test::{HttpClient, InputMap, RunAuth};

use super::http::{HttpSettings, LiveTransport};

/// Parsed `suspect test load` arguments.
#[derive(Debug, clap::Args)]
pub struct LoadArgs {
    /// Load profile (`*.load.yaml`) or an Arazzo document (with --workflow).
    pub input: PathBuf,
    /// Base URL of the system under test; overrides the profile `target`.
    #[arg(long)]
    pub base_url: Option<String>,
    /// Named environment from the profile's `environments`.
    #[arg(short = 'e', long = "env")]
    pub env: Option<String>,
    /// Profile-less run: workflow id to use as the (only) scenario; repeatable.
    #[arg(long, value_name = "WORKFLOW_ID")]
    pub workflow: Vec<String>,
    /// Profile-less run: arrivals per second.
    #[arg(long)]
    pub rate: Option<f64>,
    /// Profile-less run: ramp the arrival rate linearly to this value.
    #[arg(long)]
    pub ramp_to: Option<f64>,
    /// Profile-less run: closed-loop concurrent users.
    #[arg(long, conflicts_with = "rate")]
    pub vus: Option<usize>,
    /// Profile-less run: total iterations for --vus (instead of a duration).
    #[arg(long, requires = "vus")]
    pub iterations: Option<u64>,
    /// Profile-less run: phase length (`30s`, `2m`).
    #[arg(long)]
    pub duration: Option<String>,
    /// Cap on concurrently running users (arrivals beyond it are skipped).
    #[arg(long)]
    pub max_vus: Option<usize>,
    /// Workflow input binding `key=value` applied to every scenario; repeatable.
    #[arg(long = "input", value_name = "KEY=VALUE")]
    pub input_pairs: Vec<String>,
    /// JSON or YAML object file of workflow inputs applied to every scenario.
    #[arg(long = "inputs-file", value_name = "FILE")]
    pub inputs_file: Option<PathBuf>,
    /// Credentials file for the security schemes the workflows exercise.
    /// Default: `.suspect/credentials.json` discovered by walking up from
    /// the Arazzo document; `SUSPECT_CREDENTIALS` overrides. Every virtual
    /// user shares one token cache.
    #[arg(long, value_name = "FILE")]
    pub credentials: Option<PathBuf>,
    /// Also validate every response body against the schema the operation
    /// declares for its status; a violation counts as a contract failure.
    /// Off by default: it costs a schema compile per response on the
    /// generator.
    #[arg(long)]
    pub validate_responses: bool,
    /// RNG seed for reproducible scenario selection, pools and jitter.
    #[arg(long)]
    pub seed: Option<u64>,
    /// Inter-arrival jitter: deterministic spacing or a Poisson process.
    #[arg(long, value_enum, default_value = "none")]
    pub jitter: JitterArg,
    /// Progress output: human text blocks or one JSON record per line.
    #[command(flatten)]
    pub stream: crate::Stream,
    /// Seconds between progress snapshots; overrides the profile.
    #[arg(long, value_name = "DURATION")]
    pub report_interval: Option<String>,
    /// How long to wait for in-flight users after the last phase.
    #[arg(long, value_name = "DURATION")]
    pub grace: Option<String>,
    /// Per-request timeout; overrides the profile.
    #[arg(long, value_name = "DURATION")]
    pub timeout: Option<String>,
    /// Disable HTTP/2 negotiation.
    #[arg(long)]
    pub http1: bool,
    /// Accept invalid TLS certificates.
    #[arg(long)]
    pub insecure: bool,
    /// JSON report and JUnit report (one testcase per ensure check) files;
    /// override the profile's `report.json` and `report.junit`.
    #[command(flatten)]
    pub report_files: crate::ReportFiles,
    /// Resolve and print the plan (phases, scenarios, prerequisites) without sending anything.
    #[arg(long)]
    pub dry_run: bool,
}

/// Inter-arrival jitter choice.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum JitterArg {
    /// Evenly spaced arrivals (Artillery behaviour).
    None,
    /// Exponential inter-arrival times.
    Poisson,
}

/// Runs `suspect test load`.
///
/// Exit codes: `0` every strict `ensure` check passed, `1` an ensure check
/// failed, `2` configuration/compile error or a failed prerequisite
/// workflow, `130` interrupted before completion.
///
/// # Errors
/// Propagates I/O, workspace and transport setup failures.
pub fn load(args: LoadArgs) -> anyhow::Result<i32> {
    let input = args
        .input
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", args.input.display()))?;
    let bytes = std::fs::read(&input)?;
    let doc = LowDoc::parse(
        Uri::from_path(&input)?,
        suspect_source::Source::from_vec(bytes),
    );

    let (mut profile, arazzo_path) = if load::profile::is_load_profile(&doc) {
        let profile = load::read_profile(&doc, &input).map_err(|e| {
            anyhow::anyhow!(
                "invalid load profile {}:\n  {}",
                input.display(),
                render_profile_error(&e)
            )
        })?;
        let arazzo = profile.arazzo.clone().ok_or_else(|| {
            anyhow::anyhow!("profile must set `arazzo:` (path to the Arazzo document)")
        })?;
        (profile, arazzo)
    } else {
        (profile_from_flags(&args)?, input.clone())
    };

    if let Some(env) = &args.env {
        profile
            .apply_environment(env)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    apply_overrides(&mut profile, &args)?;
    // `test.base_url` set anywhere (flag, env, a settings file) beats the
    // profile's `target`; its built-in default does not.
    let flags = suspect_config::Layer::flags()
        .set(
            "test.base_url",
            args.base_url.clone().map(suspect_config::Raw::Str),
            "--base-url",
        )
        .set(
            "test.credentials",
            args.credentials.clone().map(suspect_config::Raw::Path),
            "--credentials",
        );
    let resolved = crate::settings::resolve_one(&input, flags)?;
    let configured = (*resolved.origin("test.base_url") != suspect_config::Origin::Default)
        .then(|| resolved.settings.test.base_url.clone());
    let target = configured
        .or_else(|| profile.target.clone())
        .ok_or_else(|| {
            anyhow::anyhow!("no target: set `target:` in the profile or pass --base-url")
        })?;

    let ws = super::workspace_for_entry(&arazzo_path)?;
    let uri = Uri::from_path(&arazzo_path)?;
    let handle = ws
        .get(&uri)
        .ok_or_else(|| anyhow::anyhow!("arazzo document not loaded: {}", arazzo_path.display()))?;
    let plan = Arc::new(suspect_test::compile_plan(handle.doc(), &ws)?);

    let diags = load::profile::check_against_plan(&profile, &plan);
    for d in &diags {
        eprintln!(
            "{}: {} ({})",
            if d.error { "error" } else { "warning" },
            d.message,
            d.code
        );
    }
    if diags.iter().any(|d| d.error) {
        return Ok(2);
    }
    let scenarios = load::compile_scenarios(&profile, &plan).map_err(|e| anyhow::anyhow!("{e}"))?;
    drop(scenarios);

    let seed = args
        .seed
        .or(profile.seed)
        .unwrap_or_else(|| Journal::now_ms() ^ 0x5eed);
    let env_name = args.env.clone().unwrap_or_default();

    if args.dry_run {
        print!("{}", dry_run_text(&profile, &plan, &target, seed));
        return Ok(0);
    }

    let auth =
        super::test::load_credentials(&arazzo_path, resolved.settings.test.credentials.as_deref())?;

    let settings = HttpSettings {
        timeout: profile.http.timeout,
        http2: profile.http.http2,
        cookies: profile.http.cookies,
        insecure: profile.http.insecure,
        pool_max_idle_per_host: None,
        user_agent: format!("suspect-load/{}", env!("CARGO_PKG_VERSION")),
    };
    let http: Arc<dyn HttpClient> = Arc::new(LiveTransport::with_settings(&settings)?);
    let http_factory: Option<load::HttpFactory> = profile.http.per_vu_connections.then(|| {
        let settings = settings.clone();
        Arc::new(move || {
            LiveTransport::with_settings(&settings)
                .map(|t| Arc::new(t) as Arc<dyn HttpClient>)
                .map_err(|e| e.to_string())
        }) as load::HttpFactory
    });

    let ndjson = args.stream.format.is_ndjson();
    let sink: Arc<dyn LoadSink> = Arc::new(ConsoleSink { ndjson });

    let rt = tokio::runtime::Runtime::new()?;
    // The first Ctrl-C stops arrivals and drains; the second aborts with
    // 130 (`crate::interrupt`).
    let cancel_rx = crate::interrupt::graceful();
    let mut notice = cancel_rx.clone();
    rt.spawn(async move {
        if notice.wait_for(|interrupted| *interrupted).await.is_ok() {
            eprintln!("\ninterrupt: stopping arrivals and draining (Ctrl-C again to abort)");
        }
    });
    let cfg = LoadConfig {
        base_url: target.clone(),
        http,
        http_factory,
        report_interval: profile.report.interval,
        grace: profile.grace,
        seed,
        env: env_name,
        jitter: match args.jitter {
            JitterArg::None => Jitter::None,
            JitterArg::Poisson => Jitter::Poisson,
        },
        cancel: Some(cancel_rx),
        auth: Arc::new(RunAuth::new(auth)),
        validate_responses: args.validate_responses,
    };
    if !ndjson {
        println!(
            "suspect test load: {} scenario(s), {} phase(s) against {}  [seed {seed}]",
            profile.scenarios.len(),
            profile.phases.len(),
            target
        );
    }
    let summary = match rt.block_on(load::run_load(plan, &profile, cfg, sink)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return Ok(2);
        }
    };

    finish(&summary, &profile, &args, ndjson)
}

fn finish(
    summary: &LoadSummary,
    profile: &LoadProfile,
    args: &LoadArgs,
    ndjson: bool,
) -> anyhow::Result<i32> {
    if ndjson {
        println!("{}", serde_json::to_string(&report::json(summary))?);
    } else {
        println!();
        print!("{}", report::console_final(summary));
    }
    if let Some(path) = args
        .report_files
        .report_file
        .clone()
        .or_else(|| profile.report.json.clone())
    {
        write_file(
            &path,
            &serde_json::to_string_pretty(&report::json(summary))?,
        )?;
        if !ndjson {
            println!("json report: {}", path.display());
        }
    }
    if let Some(path) = args
        .report_files
        .junit
        .clone()
        .or_else(|| profile.report.junit.clone())
    {
        write_file(&path, &report::junit(summary))?;
        if !ndjson {
            println!("junit report: {}", path.display());
        }
    }
    let mut journal = Journal::new(Box::new(suspect_journal::StdoutSink));
    let checks = &summary.ensure.checks;
    journal.run_summary(
        "load",
        u32::try_from(checks.iter().filter(|c| c.passed).count()).unwrap_or(u32::MAX),
        u32::try_from(checks.iter().filter(|c| !c.passed && c.strict).count()).unwrap_or(u32::MAX),
        u32::try_from(checks.iter().filter(|c| !c.passed && !c.strict).count()).unwrap_or(u32::MAX),
        summary.duration_s * 1000.0,
    );
    if summary.interrupted && summary.aggregate.http.requests == 0 {
        return Ok(130);
    }
    Ok(i32::from(!summary.ensure.ok()))
}

fn write_file(path: &Path, text: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, text).map_err(|e| anyhow::anyhow!("cannot write {}: {e}", path.display()))
}

fn render_profile_error(e: &load::ProfileError) -> String {
    e.0.iter()
        .map(|d| {
            format!(
                "{} ({}) @{}..{}",
                d.message, d.code, d.range.start, d.range.end
            )
        })
        .collect::<Vec<_>>()
        .join("\n  ")
}

/// Builds a one-phase profile from flags when the input is an Arazzo file.
fn profile_from_flags(args: &LoadArgs) -> anyhow::Result<LoadProfile> {
    if args.workflow.is_empty() {
        anyhow::bail!(
            "{} is not a load profile; pass --workflow <id> with --rate/--duration or --vus (or write a `suspectLoad: \"1.0\"` profile)",
            args.input.display()
        );
    }
    let duration = args
        .duration
        .as_deref()
        .map(parse_duration)
        .transpose()
        .map_err(|e| anyhow::anyhow!("--duration: {e}"))?;
    let kind = match (args.rate, args.vus) {
        (Some(rate), None) => PhaseKind::ArrivalRate {
            rate,
            ramp_to: args.ramp_to,
        },
        (None, Some(vus)) => PhaseKind::ConstantVus {
            vus,
            iterations: args.iterations,
        },
        (None, None) => anyhow::bail!("profile-less runs need --rate <n> or --vus <n>"),
        (Some(_), Some(_)) => unreachable!("clap conflicts_with"),
    };
    let duration = match (duration, &kind) {
        (Some(d), _) => d,
        (
            None,
            PhaseKind::ConstantVus {
                iterations: Some(_),
                ..
            },
        ) => Duration::from_secs(24 * 3600),
        (None, _) => anyhow::bail!("profile-less runs need --duration (e.g. 60s, 5m)"),
    };
    let phases = vec![Phase {
        name: None,
        kind,
        duration,
        max_vusers: args.max_vus,
        range: 0..0,
    }];
    let scenarios = args
        .workflow
        .iter()
        .map(|w| Scenario {
            name: w.clone(),
            workflow: w.clone(),
            weight: 1.0,
            think: None,
            inputs: Default::default(),
            data: None,
            iterations: 1,
            continue_on_failure: false,
            range: 0..0,
        })
        .collect();
    Ok(LoadProfile {
        phases,
        scenarios,
        ..LoadProfile::default()
    })
}

fn apply_overrides(profile: &mut LoadProfile, args: &LoadArgs) -> anyhow::Result<()> {
    if let Some(d) = &args.report_interval {
        profile.report.interval =
            parse_duration(d).map_err(|e| anyhow::anyhow!("--report-interval: {e}"))?;
    }
    if let Some(d) = &args.grace {
        profile.grace = parse_duration(d).map_err(|e| anyhow::anyhow!("--grace: {e}"))?;
    }
    if let Some(d) = &args.timeout {
        profile.http.timeout = parse_duration(d).map_err(|e| anyhow::anyhow!("--timeout: {e}"))?;
    }
    if args.http1 {
        profile.http.http2 = false;
    }
    if args.insecure {
        profile.http.insecure = true;
    }
    if let Some(max) = args.max_vus {
        for p in &mut profile.phases {
            p.max_vusers = Some(max);
        }
    }
    let inputs: InputMap =
        super::inputs::parse_inputs(args.inputs_file.as_deref(), &args.input_pairs)?;
    if !inputs.is_empty() {
        for s in &mut profile.scenarios {
            for (k, v) in &inputs {
                s.inputs
                    .insert(k.clone(), load::profile::InputSource::Literal(v.clone()));
            }
        }
    }
    Ok(())
}

fn dry_run_text(
    profile: &LoadProfile,
    plan: &suspect_test::Plan,
    target: &str,
    seed: u64,
) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "suspect test load --dry-run");
    let _ = writeln!(out, "target: {target}");
    let _ = writeln!(out, "seed:   {seed}");
    let _ = writeln!(
        out,
        "http:   timeout {}, http2 {}, per-user connections {}, cookies {}",
        format_duration(profile.http.timeout),
        profile.http.http2,
        profile.http.per_vu_connections,
        profile.http.cookies
    );
    let prereqs = load::prerequisite_workflows(profile, plan);
    if !prereqs.is_empty() {
        let _ = writeln!(out, "\nrun once before arrivals:");
        for idx in prereqs {
            let wf = &plan.workflows[idx];
            let _ = writeln!(out, "  {}  ({} steps)", wf.workflow_id, wf.steps.len());
        }
    }
    let _ = writeln!(out, "\nphases:");
    let mut total = Duration::ZERO;
    let mut total_arrivals = 0u64;
    for (i, p) in profile.phases.iter().enumerate() {
        let arrivals = load::Arrivals::new(p, Jitter::None, 0).expected_total();
        total_arrivals += arrivals;
        let _ = writeln!(
            out,
            "  {:>2}. +{:<8} {:<24} {:<32}{}",
            i + 1,
            format_duration(total),
            p.label(),
            describe(&p.kind, p.duration),
            match (arrivals, p.max_vusers) {
                (0, None) => String::new(),
                (n, None) => format!("  {n} arrivals"),
                (0, Some(m)) => format!("  max {m} concurrent"),
                (n, Some(m)) => format!("  {n} arrivals, max {m} concurrent"),
            }
        );
        total += p.duration;
    }
    let _ = writeln!(
        out,
        "  total {}; {} scheduled arrivals (closed-loop phases excluded)",
        format_duration(total),
        total_arrivals
    );
    let weight: f64 = profile.total_weight();
    let _ = writeln!(out, "\nscenarios:");
    for s in &profile.scenarios {
        let wf = plan.workflows.iter().find(|w| w.workflow_id == s.workflow);
        let _ = writeln!(
            out,
            "  {:<24} {:>5.1}%  {} ({} steps){}{}{}",
            s.name,
            s.weight * 100.0 / weight,
            s.workflow,
            wf.map_or(0, |w| w.steps.len()),
            if s.iterations > 1 {
                format!(", {} iterations", s.iterations)
            } else {
                String::new()
            },
            s.think.map_or(String::new(), |t| format!(
                ", think {}..{}",
                format_duration(t.min),
                format_duration(t.max)
            )),
            s.data.as_ref().map_or(String::new(), |d| format!(
                ", data {} ({:?})",
                d.path.display(),
                d.order
            )),
        );
    }
    if profile.ensure.max_error_rate.is_some()
        || !profile.ensure.thresholds.is_empty()
        || !profile.ensure.conditions.is_empty()
    {
        let _ = writeln!(out, "\nensure:");
        if let Some(m) = profile.ensure.max_error_rate {
            let _ = writeln!(out, "  error rate <= {m}%");
        }
        for (metric, max, _) in &profile.ensure.thresholds {
            let _ = writeln!(out, "  {metric} <= {max}");
        }
        for c in &profile.ensure.conditions {
            let _ = writeln!(
                out,
                "  {}{}",
                c.expression,
                if c.strict { "" } else { "  [non-strict]" }
            );
        }
    }
    out
}

fn describe(kind: &PhaseKind, duration: Duration) -> String {
    let d = format_duration(duration);
    match kind {
        PhaseKind::ArrivalRate {
            rate,
            ramp_to: None,
        } => format!("{rate}/s for {d}"),
        PhaseKind::ArrivalRate {
            rate,
            ramp_to: Some(to),
        } => format!("{rate}/s → {to}/s over {d}"),
        PhaseKind::ArrivalCount { count } => format!("{count} arrivals over {d}"),
        PhaseKind::Pause => format!("pause {d}"),
        PhaseKind::ConstantVus {
            vus,
            iterations: None,
        } => format!("{vus} users looping for {d}"),
        PhaseKind::ConstantVus {
            vus,
            iterations: Some(n),
        } => format!("{vus} users, {n} iterations (≤ {d})"),
    }
}

/// Prints interval blocks (text) or journal records (ndjson).
struct ConsoleSink {
    ndjson: bool,
}

impl LoadSink for ConsoleSink {
    fn phase_started(&self, index: usize, phase: &Phase) {
        if self.ndjson {
            emit(Journal::meta(
                "load",
                "phase started",
                serde_json::json!({ "index": index, "phase": phase.label(), "duration_s": phase.duration.as_secs_f64() }),
            ));
        } else {
            println!(
                "▶ phase {} `{}` ({})",
                index + 1,
                phase.label(),
                format_duration(phase.duration)
            );
        }
    }

    fn interval(&self, report: &IntervalReport) {
        if self.ndjson {
            emit(Record::LoadSnapshot(LoadSnapshotRecord {
                ts_ms: Journal::now_ms(),
                index: report.index,
                phase: report.phase.clone(),
                at_s: report.at_s,
                interval: serde_json::to_value(&report.interval).unwrap_or_default(),
                cumulative: serde_json::to_value(&report.cumulative).unwrap_or_default(),
            }));
        } else {
            print!("{}", report::console_interval(report));
        }
    }

    fn note(&self, message: &str) {
        if self.ndjson {
            emit(Record::Log(suspect_journal::LogRecord {
                ts_ms: Journal::now_ms(),
                level: suspect_journal::Level::Info,
                target: "load".to_owned(),
                msg: message.to_owned(),
                fields: serde_json::Value::Null,
            }));
        } else {
            println!("· {message}");
        }
    }
}

fn emit(record: Record) {
    if let Ok(line) = serde_json::to_string(&record) {
        println!("{line}");
    }
}
