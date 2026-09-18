//! Observational canonical SDK workflow measurements, not a release/performance gate.
//!
//! Run with a fresh `--out target/...`, an explicit source and exact operation IDs.
//! `--native` additionally builds, packs, installs and imports the actual npm package.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use anyhow::{Context, Result, bail, ensure};
use clap::Parser;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use suspect_codegen::typescript::http::{HttpConfig, HttpPlan, plan_http};
use suspect_codegen::typescript::package::{PackageConfig, emit_http};
use suspect_codegen::{OutFile, check_files_with_owner, write_files};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

const PACKAGE_NAME: &str = "@suspect/sdk-workflow-bench";

#[derive(Parser)]
#[command(about = "Measure actual canonical SDK planning, writing and optional native consumption")]
struct Options {
    /// Entry OpenAPI document. The benchmark never modifies source documents.
    #[arg(long)]
    spec: PathBuf,
    /// New directory beneath this repository's target directory; parent must exist.
    #[arg(long)]
    out: PathBuf,
    /// Exact operationId to include (repeatable). Omit to attempt all operations.
    #[arg(long)]
    operation_id: Vec<String>,
    /// Independent cold-compiler and shared-contract samples; OS caches are not flushed.
    #[arg(long, default_value_t = 3, value_parser = clap::value_parser!(u16).range(1..=100))]
    iterations: u16,
    /// Require the pinned native Node/npm tools and perform real package consumption.
    #[arg(long)]
    native: bool,
}

#[derive(Serialize)]
struct Phase {
    stage: &'static str,
    milliseconds: f64,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct Input {
    uri: String,
    bytes: u64,
    sha256: String,
}

#[derive(Debug, PartialEq, Eq)]
struct Stamp {
    bytes: u64,
    modified: SystemTime,
    #[cfg(unix)]
    inode: u64,
}

fn measure<T>(
    phases: &mut Vec<Phase>,
    stage: &'static str,
    run: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let start = Instant::now();
    let result = run();
    phases.push(Phase {
        stage,
        milliseconds: start.elapsed().as_secs_f64() * 1000.0,
    });
    result
}

fn fingerprint(path: &Path) -> Result<Input> {
    let mut file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    let mut bytes = 0;
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        hasher.update(&buffer[..count]);
    }
    Ok(Input {
        uri: Uri::from_path(path)?.to_string(),
        bytes,
        sha256: format!("{:x}", hasher.finalize()),
    })
}

fn inputs(contract: &Contract) -> Result<Vec<Input>> {
    contract
        .documents()
        .map(|(uri, _)| {
            let path = uri
                .as_path()
                .with_context(|| format!("cannot fingerprint non-file source {uri}"))?;
            fingerprint(&path)
        })
        .collect()
}

fn selected(contract: &Contract, names: &[String]) -> Result<Vec<SourceId>> {
    let available: Vec<_> = contract.operations().collect();
    let mut selected = Vec::new();
    if names.is_empty() {
        selected.extend(available.iter().map(|operation| operation.source().clone()));
    } else {
        for name in names {
            let matches: Vec<_> = available
                .iter()
                .filter(|operation| operation.operation_id() == Some(name.as_str()))
                .collect();
            ensure!(
                matches.len() == 1,
                "operationId {name:?} has {} matches, expected exactly one",
                matches.len()
            );
            selected.push(matches[0].source().clone());
        }
    }
    selected.sort();
    selected.dedup();
    ensure!(!selected.is_empty(), "no outgoing operations selected");
    Ok(selected)
}

fn plan(contract: &Arc<Contract>, selected: &[SourceId]) -> Result<HttpPlan> {
    plan_http(Arc::clone(contract), selected, HttpConfig::default()).map_err(|findings| {
        anyhow::anyhow!(
            "HTTP admission failed:\n{}",
            findings
                .into_iter()
                .map(|finding| format!(
                    "{}#{} {}: {}",
                    finding.source.document(),
                    finding.source.pointer(),
                    finding.code,
                    finding.message
                ))
                .collect::<Vec<_>>()
                .join("\n")
        )
    })
}

fn package(plan: &HttpPlan) -> Result<Vec<OutFile>> {
    Ok(emit_http(
        plan,
        &PackageConfig {
            name: PACKAGE_NAME.into(),
            version: "0.0.0".into(),
        },
    )?)
}

fn stamps(root: &Path, files: &[OutFile]) -> Result<BTreeMap<PathBuf, Stamp>> {
    files
        .iter()
        .map(|file| PathBuf::from(&file.path))
        .chain([PathBuf::from(suspect_artifact::OWNERSHIP_MANIFEST)])
        .map(|path| {
            let metadata = fs::metadata(root.join(&path))?;
            Ok((
                path,
                Stamp {
                    bytes: metadata.len(),
                    modified: metadata.modified()?,
                    #[cfg(unix)]
                    inode: {
                        use std::os::unix::fs::MetadataExt;
                        metadata.ino()
                    },
                },
            ))
        })
        .collect()
}

fn run_command(
    phases: &mut Vec<Phase>,
    stage: &'static str,
    executable: &Path,
    args: &[&str],
    cwd: &Path,
    logs: &Path,
) -> Result<String> {
    measure(phases, stage, || {
        let output = Command::new(executable)
            .args(args)
            .current_dir(cwd)
            .output()
            .with_context(|| format!("run {} {args:?}", executable.display()))?;
        fs::write(logs.join(format!("{stage}.stdout")), &output.stdout)?;
        fs::write(logs.join(format!("{stage}.stderr")), &output.stderr)?;
        ensure!(
            output.status.success(),
            "{stage} failed ({}); see {}",
            output.status,
            logs.display()
        );
        String::from_utf8(output.stdout).context("native tool emitted non-UTF8 output")
    })
}

fn native(root: &Path) -> Result<Value> {
    let logs = root.join("native-logs");
    fs::create_dir(&logs)?;
    let node = std::env::var_os("SUSPECT_DOCS_NODE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("node"));
    let npm = Path::new("npm");
    let mut phases = Vec::new();
    let node_version = run_command(
        &mut phases,
        "node-version",
        &node,
        &["--version"],
        root,
        &logs,
    )?;
    let npm_version = run_command(&mut phases, "npm-version", npm, &["--version"], root, &logs)?;
    ensure!(
        node_version.trim() == "v22.23.1",
        "native measurements require Node 22.23.1, got {}",
        node_version.trim()
    );
    ensure!(
        npm_version.trim() == "10.9.8",
        "native measurements require npm 10.9.8, got {}",
        npm_version.trim()
    );
    let package = root.join("artifacts/typescript");
    run_command(
        &mut phases,
        "npm-ci",
        npm,
        &[
            "ci",
            "--ignore-scripts",
            "--offline",
            "--no-audit",
            "--no-fund",
        ],
        &package,
        &logs,
    )?;
    run_command(
        &mut phases,
        "native-build",
        npm,
        &["run", "build"],
        &package,
        &logs,
    )?;
    let packed: Value = serde_json::from_str(&run_command(
        &mut phases,
        "native-pack",
        npm,
        &["pack", "--ignore-scripts", "--json"],
        &package,
        &logs,
    )?)?;
    let filename = packed[0]["filename"]
        .as_str()
        .context("npm pack omitted filename")?;
    ensure!(
        Path::new(filename).components().count() == 1 && Path::new(filename).file_name().is_some(),
        "npm pack returned a non-basename path"
    );
    let tarball = fingerprint(&package.join(filename))?;
    let consumer = root.join("consumer");
    fs::create_dir(&consumer)?;
    fs::write(
        consumer.join("package.json"),
        serde_json::to_vec_pretty(&json!({
            "name": "sdk-workflow-consumer", "version": "0.0.0", "private": true, "type": "module",
            "dependencies": { PACKAGE_NAME: format!("file:../artifacts/typescript/{filename}") }
        }))?,
    )?;
    run_command(
        &mut phases,
        "consumer-install",
        npm,
        &[
            "install",
            "--ignore-scripts",
            "--offline",
            "--no-audit",
            "--no-fund",
        ],
        &consumer,
        &logs,
    )?;
    fs::write(
        consumer.join("measure.mjs"),
        r#"import { performance } from 'node:perf_hooks';
if (typeof globalThis.gc !== 'function') throw new Error('explicit GC unavailable');
globalThis.gc();
const heapBefore = process.memoryUsage().heapUsed;
const start = performance.now();
const sdk = await import('@suspect/sdk-workflow-bench');
const importMs = performance.now() - start;
globalThis.gc();
const heapAfterImport = process.memoryUsage().heapUsed;
const text = '[9007199254740993,1e400,1.00,null]';
const firstStart = performance.now();
const value = sdk.parseJson(text);
if (sdk.stringifyJson(value) !== text) throw new Error('exact JSON roundtrip changed numeric tokens');
const firstRoundtripMs = performance.now() - firstStart;
const iterations = 10000;
const warmStart = performance.now();
for (let i = 0; i < iterations; i++) {
    if (sdk.stringifyJson(sdk.parseJson(text)) !== text) throw new Error('warm roundtrip failed');
}
const warmRoundtripMs = performance.now() - warmStart;
console.log(JSON.stringify({ importMs, firstRoundtripMs, iterations, warmRoundtripMs,
    heapBefore, heapAfterImport, retainedImportHeapDelta: heapAfterImport - heapBefore,
    peakRssKiB: process.resourceUsage().maxRSS,
    scope: 'unbundled installed package import and exact JSON representation; not model-codec or HTTP throughput' }));
"#,
    )?;
    let consumption: Value = serde_json::from_str(&run_command(
        &mut phases,
        "native-consumer",
        &node,
        &["--expose-gc", "measure.mjs"],
        &consumer,
        &logs,
    )?)?;
    Ok(
        json!({ "status": "passed", "node": node_version.trim(), "npm": npm_version.trim(), "phases": phases, "tarball": tarball, "consumption": consumption }),
    )
}

fn main() -> Result<()> {
    let options = Options::parse();
    let target = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .canonicalize()?;
    let parent = options
        .out
        .parent()
        .context("output needs a parent directory")?
        .canonicalize()
        .context("output parent must already exist")?;
    ensure!(
        parent.starts_with(&target),
        "benchmark output must be beneath {}",
        target.display()
    );
    let name = options
        .out
        .file_name()
        .context("output must name a new directory")?;
    let root = parent.join(name);
    fs::create_dir(&root).with_context(|| format!("output must be new: {}", root.display()))?;
    let entry = Uri::from_path(&options.spec)?;
    let executable = fingerprint(&std::env::current_exe()?)?;
    let entry_before = fingerprint(&options.spec)?;
    let mut cold_samples = Vec::new();
    let mut baseline = None;
    for _ in 0..options.iterations {
        let mut phases = Vec::new();
        let workspace = Arc::new(WorkspaceBuilder::new().build()?);
        measure(&mut phases, "entry-read-and-parse", || {
            Ok(workspace.open(entry.as_str())?)
        })?;
        let contract = Arc::new(measure(
            &mut phases,
            "reference-closure-and-contract",
            || Ok(Contract::from_workspace(&workspace, &entry)?),
        )?);
        let selected = selected(&contract, &options.operation_id)?;
        let plan = measure(
            &mut phases,
            "http-model-codec-validation-and-docs-plan",
            || plan(&contract, &selected),
        )?;
        let files = measure(&mut phases, "package-and-artifact-materialization", || {
            package(&plan)
        })?;
        let source_inputs = inputs(&contract)?;
        if let Some((_, expected, expected_inputs, _)) = &baseline {
            ensure!(
                &files == expected,
                "cold compiler samples produced different artifacts"
            );
            ensure!(
                &source_inputs == expected_inputs,
                "source closure changed between samples"
            );
        } else {
            baseline = Some((contract, files, source_inputs, selected));
        }
        cold_samples.push(phases);
    }
    let (contract, files, source_inputs, selected) = baseline.context("no benchmark samples")?;
    let mut shared_contract_samples = Vec::new();
    for _ in 0..options.iterations {
        let mut phases = Vec::new();
        let plan = measure(
            &mut phases,
            "http-model-codec-validation-and-docs-plan",
            || plan(&contract, &selected),
        )?;
        let next = measure(&mut phases, "package-and-artifact-materialization", || {
            package(&plan)
        })?;
        ensure!(next == files, "shared-contract planning changed artifacts");
        shared_contract_samples.push(phases);
    }
    let artifacts = root.join("artifacts");
    let mut writes = Vec::new();
    measure(&mut writes, "initial-owned-write", || {
        write_files(&files, &artifacts).map_err(anyhow::Error::msg)
    })?;
    let before = stamps(&artifacts, &files)?;
    for _ in 0..options.iterations {
        measure(&mut writes, "unchanged-ownership-check", || {
            let report = check_files_with_owner(&files, &artifacts, "suspect-codegen")
                .map_err(anyhow::Error::msg)?;
            ensure!(
                report.is_current(),
                "unchanged artifact set has drift: {report:?}"
            );
            Ok(())
        })?;
        measure(&mut writes, "unchanged-owned-write", || {
            write_files(&files, &artifacts).map_err(anyhow::Error::msg)
        })?;
        ensure!(
            stamps(&artifacts, &files)? == before,
            "unchanged generation rewrote bytes, mtimes or inodes"
        );
    }
    let native_report = if options.native {
        native(&root)?
    } else {
        json!({ "status": "not-requested" })
    };
    if fingerprint(&options.spec)? != entry_before || inputs(&contract)? != source_inputs {
        bail!(
            "source changed during the benchmark; measurements are not attributable to one closure"
        );
    }
    let report = json!({
        "format": "suspect-sdk-workflow-bench-v1", "observational": true, "releaseReady": false,
        "generatorVersion": env!("CARGO_PKG_VERSION"), "executable": executable,
        "entry": entry.to_string(), "inputs": source_inputs, "iterations": options.iterations,
        "operationIds": options.operation_id,
        "coldCompilerSamples": cold_samples, "sharedContractSamples": shared_contract_samples,
        "writes": writes, "unchangedRewrites": 0,
        "artifacts": { "files": files.len(), "bytes": files.iter().map(|file| file.content.len() as u64).sum::<u64>() },
        "native": native_report,
        "measurementScope": [
            "Cold means a fresh compiler/workspace; filesystem and OS page caches are not flushed.",
            "Reference closure and normalization share one compiler seam; model, codec, validation and docs planning share one HTTP seam.",
            "Shared-contract samples reuse one immutable Contract, not a persistent invalidation cache.",
            "Native installation uses pinned offline npm artifacts; missing tools/cache entries fail the command.",
            "Timings and heap are observations, not calibrated p95 regression budgets or complete SDK performance certification."
        ]
    });
    let encoded = serde_json::to_string_pretty(&report)?;
    fs::write(root.join("report.json"), &encoded)?;
    println!("{encoded}");
    Ok(())
}
