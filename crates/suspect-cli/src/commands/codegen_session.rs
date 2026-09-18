//! Persistent canonical SDK generation, readonly preview and content-based watch.
use crate::OutputFormat;
use anyhow::{Context, Result};
use clap::Args;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use suspect_codegen::{
    OwnershipChangeKind,
    backend::{GenerationOptions, TargetConfig},
    generation_session::{Session, SessionConfig, SessionError, Stats},
    http_protocol::CompatibilityProfile,
};

#[derive(Debug, Args)]
/// Persistent multi-backend generation and readonly preview options.
pub struct SessionArgs {
    /// JSON configuration file; relative source paths resolve from its directory.
    #[arg(long)]
    pub config: PathBuf,
    /// Owned output root.
    #[arg(long, default_value = "sdk-out")]
    pub out: PathBuf,
    /// Preserve a process/session across input and configuration changes.
    #[arg(long)]
    pub watch: bool,
    /// Check disk drift and conflicts without writing.
    #[arg(long)]
    pub check: bool,
    /// Return complete generated contents as readonly preview artifacts.
    #[arg(long)]
    pub preview: bool,
    /// Human-readable status or versioned newline-delimited JSON records.
    #[arg(long, value_enum, default_value = "text")]
    pub format: OutputFormat,
    /// Content-fingerprint polling interval, in milliseconds.
    #[arg(long,default_value_t=250,value_parser=clap::value_parser!(u64).range(25..=60000))]
    pub interval_ms: u64,
    /// Optional finite watch limit for automated process-level consumers.
    #[arg(long, hide = true, requires = "watch")]
    pub max_iterations: Option<usize>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
    spec: Option<PathBuf>,
    pins: Option<PinnedConfiguration>,
    targets: Vec<TargetConfig>,
    #[serde(default)]
    operation_ids: Vec<String>,
    #[serde(default)]
    compatibility_profiles: std::collections::BTreeSet<CompatibilityProfile>,
    #[serde(default)]
    credential_env: Option<suspect_codegen::credential_env::CredentialEnv>,
    #[serde(default)]
    sdk_defaults: Option<suspect_codegen::sdk_defaults::SdkDefaults>,
    owner: Option<String>,
    cache_entries: Option<usize>,
    cache_bytes: Option<usize>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PinnedConfiguration {
    manifest: PathBuf,
    cache_dir: Option<PathBuf>,
    #[serde(default)]
    insecure_test_origins: Vec<String>,
}

pub(super) fn read_config(
    path: &Path,
) -> Result<(suspect_codegen::generation_session::Input, SessionConfig)> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .with_context(|| format!("read configuration {}", path.display()))?
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= 4 * 1024 * 1024,
        "configuration exceeds 4 MiB"
    );
    let config: Configuration =
        serde_json::from_slice(&bytes).context("invalid SDK session configuration")?;
    let relative = |value: PathBuf| {
        if value.is_absolute() {
            value
        } else {
            path.parent().unwrap_or(Path::new(".")).join(value)
        }
    };
    let source = match (config.spec, config.pins) {
        (Some(path), None) => suspect_codegen::generation_session::Input::File {
            path: relative(path),
        },
        (None, Some(pins)) => suspect_codegen::generation_session::Input::Pinned {
            manifest: relative(pins.manifest),
            cache_dir: relative(
                pins.cache_dir
                    .unwrap_or_else(|| PathBuf::from(".suspect-cache")),
            ),
            insecure_test_origins: pins.insecure_test_origins,
        },
        _ => anyhow::bail!("SDK configuration requires exactly one of spec or pins"),
    }
    .normalized()?;
    let mut options = SessionConfig {
        targets: config.targets,
        operation_ids: config.operation_ids,
        generation: GenerationOptions {
            compatibility_profiles: config.compatibility_profiles,
            credential_env: config.credential_env,
            sdk_defaults: config.sdk_defaults,
        },
        ..Default::default()
    };
    if let Some(owner) = config.owner {
        options.owner = owner;
    }
    if let Some(entries) = config.cache_entries {
        options.cache_entries = entries;
    }
    if let Some(bytes) = config.cache_bytes {
        options.cache_bytes = bytes;
    }
    Ok((source, options))
}
fn stats(stats: Stats) -> Value {
    json!({"compiles":stats.compiles,"renders":stats.renders,"cache_hits":stats.cache_hits})
}
fn diagnostics(error: &anyhow::Error) -> Value {
    if let Some(SessionError::Acquisition(error)) = error.downcast_ref::<SessionError>() {
        return json!([{"code":error.code(),"message":error.to_string(),"manifest":error.manifest_path(),
            "resourceIndex":error.resource_index(),"line":error.line(),"uri":error.redacted_uri(),"file":error.file_path()}]);
    }
    if let Some(SessionError::Backend(findings)) = error.downcast_ref::<SessionError>() {
        return json!(findings.iter().map(|finding|json!({"code":finding.code,"message":finding.message,"source":finding.source.as_ref().map(|source|json!({"document":source.document().as_str(),"pointer":source.pointer()})),"range":finding.at})).collect::<Vec<_>>());
    }
    json!([{"code":"sdk-session","message":format!("{error:#}")}])
}
fn output(args: &SessionArgs, value: &Value) -> Result<()> {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    if matches!(args.format, OutputFormat::Json) {
        serde_json::to_writer(&mut stdout, value)?;
        writeln!(stdout)?;
    } else {
        writeln!(
            stdout,
            "SDK generation {}: {} ({} changed artifacts; {} compiles, {} renders)",
            value["generation"],
            value["status"].as_str().unwrap_or("error"),
            value["changedArtifacts"].as_array().map_or(0, Vec::len),
            value["delta"]["compiles"],
            value["delta"]["renders"]
        )?;
        for diagnostic in value["diagnostics"].as_array().into_iter().flatten() {
            writeln!(
                stdout,
                "{}",
                diagnostic["message"].as_str().unwrap_or("unknown error")
            )?;
        }
    }
    stdout.flush()?;
    Ok(())
}
/// Run one canonical generation/preview, or maintain a content-fingerprinted watch.
///
/// # Errors
/// Returns process/output failures. Configuration, planning and ownership findings
/// are emitted as structured results and return status 1 for a finite run.
pub fn generate(args: SessionArgs) -> Result<i32> {
    anyhow::ensure!(
        args.max_iterations != Some(0),
        "max-iterations must be positive"
    );
    let config_path = if args.config.is_absolute() {
        args.config.clone()
    } else {
        std::env::current_dir()?.join(&args.config)
    };
    let mut session: Option<Session> = None;
    let mut source: Option<suspect_codegen::generation_session::Input> = None;
    let mut previous_signature = String::new();
    let mut generation = 0usize;
    let mut iterations = 0usize;
    loop {
        iterations += 1;
        let result = (|| -> Result<Value> {
            let (entry, options) = read_config(&config_path)?;
            let owner = options.owner.clone();
            let generation_options = options.generation.clone();
            if source.as_ref() != Some(&entry) {
                session = Some(Session::with_input(entry.clone(), options)?);
                source = Some(entry.clone());
            } else {
                session
                    .as_mut()
                    .expect("session for source")
                    .set_config(options)?;
            }
            let session = session.as_mut().expect("configured session");
            let generated = session.generate()?;
            let drift =
                suspect_codegen::check_files_with_owner(&generated.files, &args.out, &owner)
                    .map_err(anyhow::Error::msg)?;
            let changed = drift
                .changes
                .iter()
                .filter(|change| change.kind != OwnershipChangeKind::Unchanged)
                .map(|change| change.path.display().to_string())
                .collect::<Vec<_>>();
            let diagnostics=drift.conflicts().map(|change|json!({"code":"ownership-conflict","path":change.path,"message":change.conflict})).collect::<Vec<_>>();
            let readonly = args.check || args.preview;
            let current = drift.is_current();
            let success = if readonly {
                current
            } else {
                diagnostics.is_empty()
            };
            let status = if !diagnostics.is_empty() {
                "write-conflict"
            } else if current {
                "current"
            } else if readonly {
                "drift"
            } else {
                session.write(&generated, &args.out)?;
                "written"
            };
            let mut record = json!({"format":"suspect.sdk.session.v1","success":success,"status":status,"source":entry.path(),"sourceDocument":generated.contract.entry().as_str(),"input":entry,"output":args.out,"config":config_path,"revision":generated.revision,"compatibilityProfiles":generation_options.compatibility_profiles,"changedArtifacts":changed,"newDocuments":generated.new_documents,"delta":stats(generated.delta),"stats":stats(generated.stats),"diagnostics":diagnostics});
            if let Some(policy) = generation_options.credential_env {
                record["credentialEnv"] = json!(policy);
            }
            if let Some(defaults) = generation_options.sdk_defaults {
                record["sdkDefaults"] = json!(defaults);
            }
            if args.preview {
                record["artifacts"] = json!(
                    generated
                        .files
                        .iter()
                        .map(|file| json!({"path":file.path,"content":file.content}))
                        .collect::<Vec<_>>()
                );
            }
            Ok(record)
        })();
        let mut record = match result {
            Ok(record) => record,
            Err(error) => {
                json!({"format":"suspect.sdk.session.v1","success":false,"status":"planning-error","source":source.as_ref().map(|source|source.path()),"input":source,"output":args.out,"config":config_path,"changedArtifacts":[],"newDocuments":[],"delta":stats(Stats::default()),"stats":stats(session.as_ref().map_or(Stats::default(),Session::stats)),"diagnostics":diagnostics(&error)})
            }
        };
        let signature=json!({"status":record["status"],"revision":record["revision"],"changed":record["changedArtifacts"],"diagnostics":record["diagnostics"]}).to_string();
        let activity = record["delta"]["compiles"].as_u64().unwrap_or(0) > 0
            || record["delta"]["renders"].as_u64().unwrap_or(0) > 0;
        if generation == 0 || activity || signature != previous_signature {
            generation += 1;
            record["generation"] = json!(generation);
            output(&args, &record)?;
            previous_signature = signature;
        }
        let exit = if record["success"] == true { 0 } else { 1 };
        if !args.watch || args.max_iterations.is_some_and(|limit| iterations >= limit) {
            return Ok(exit);
        }
        std::thread::sleep(Duration::from_millis(args.interval_ms));
    }
}
