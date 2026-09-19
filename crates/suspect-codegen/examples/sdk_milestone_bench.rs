//! M0/M2 observational milestone measurements over one canonical contract and
//! two targets (canonical TypeScript and native Rust).
//!
//! Run with a fresh `--out target/...`, an explicit source entry document and
//! exact operation IDs. `--native` additionally builds, packs, installs and
//! imports the actual npm package, and builds the actual Rust package.
//! Timings are observations, not calibrated budgets; persistent caches are
//! M6 scope and are not claimed here.

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
use suspect_codegen::rust_http::{
    PackageConfig as RsPackageConfig, emit_http as emit_rust_http, plan_http as plan_rust_http,
};
use suspect_codegen::typescript::http::plan_http as plan_ts_http;
use suspect_codegen::typescript::package::{
    PackageConfig as TsPackageConfig, emit_http as emit_ts_http,
};
use suspect_codegen::{OutFile, check_files_with_owner, write_files};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

#[path = "sdk_milestone_bench/compare.rs"]
mod compare;

const TS_PACKAGE: &str = "@suspect/sdk-milestone-bench";
const RUST_PACKAGE: &str = "sdk-milestone-bench";
const REPORT_FORMAT: &str = "suspect-sdk-milestone-bench-v1";

#[derive(Parser)]
#[command(about = "Measure canonical TS and Rust SDK generation from one shared contract")]
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
    /// Require pinned native tools and perform real package consumption.
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
            fingerprint(
                &uri.as_path()
                    .with_context(|| format!("cannot fingerprint non-file source {uri}"))?,
            )
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

fn stamp(root: &Path, files: &[OutFile]) -> Result<BTreeMap<PathBuf, Stamp>> {
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
fn ts_files(
    contract: &Arc<Contract>,
    selected: &[SourceId],
    phases: &mut Vec<Phase>,
) -> Result<Vec<OutFile>> {
    let plan = measure(phases, "typescript-plan-http", || {
        plan_ts_http(Arc::clone(contract), selected, Default::default()).map_err(|findings| {
            anyhow::anyhow!(
                "TypeScript HTTP admission failed:\n{}",
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
    })?;
    let config = TsPackageConfig {
        name: TS_PACKAGE.into(),
        version: "0.0.0".into(),
    };
    measure(phases, "typescript-emit-package-and-docs", || {
        emit_ts_http(&plan, &config)
            .map_err(|error| anyhow::anyhow!("TypeScript emission failed: {error}"))
    })
}

fn rust_files(
    contract: &Arc<Contract>,
    selected: &[SourceId],
    phases: &mut Vec<Phase>,
) -> Result<Vec<OutFile>> {
    let plan = measure(phases, "rust-plan-http", || {
        plan_rust_http(Arc::clone(contract), selected, Default::default()).map_err(|findings| {
            anyhow::anyhow!(
                "Rust HTTP admission failed:\n{}",
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
    })?;
    let config = RsPackageConfig {
        name: RUST_PACKAGE.into(),
        version: "0.0.0".into(),
    };
    measure(phases, "rust-emit-package-and-docs", || {
        emit_rust_http(&plan, &config)
            .map_err(|error| anyhow::anyhow!("Rust emission failed: {error}"))
    })
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

fn tool_version(
    phases: &mut Vec<Phase>,
    stage: &'static str,
    executable: &str,
    logs: &Path,
    cwd: &Path,
) -> Result<String> {
    run_command(
        phases,
        stage,
        Path::new(executable),
        &["--version"],
        cwd,
        logs,
    )
    .map(|output| output.trim().to_owned())
}

fn native_typescript(root: &Path, logs: &Path) -> Result<Value> {
    let npm = Path::new("npm");
    let mut phases = Vec::new();
    let node = std::env::var_os("SUSPECT_DOCS_NODE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("node"));
    let node_version = tool_version(
        &mut phases,
        "node-version",
        node.to_str().context("node path")?,
        logs,
        root,
    )?;
    let npm_version = tool_version(&mut phases, "npm-version", "npm", logs, root)?;
    ensure!(
        node_version == "v22.23.1",
        "native requires Node 22.23.1, got {node_version}"
    );
    ensure!(
        npm_version == "10.9.8",
        "native requires npm 10.9.8, got {npm_version}"
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
        logs,
    )?;
    run_command(
        &mut phases,
        "native-build",
        npm,
        &["run", "build"],
        &package,
        logs,
    )?;
    let packed: Value = serde_json::from_str(&run_command(
        &mut phases,
        "native-pack",
        npm,
        &["pack", "--ignore-scripts", "--json"],
        &package,
        logs,
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
            "name": "sdk-milestone-consumer", "version": "0.0.0", "private": true, "type": "module",
            "dependencies": { TS_PACKAGE: format!("file:../artifacts/typescript/{filename}") }
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
        logs,
    )?;
    fs::write(
        consumer.join("measure.mjs"),
        r#"import { performance } from 'node:perf_hooks';
globalThis.gc();
const start = performance.now();
const sdk = await import('@suspect/sdk-milestone-bench');
const importMs = performance.now() - start;
const text = '[9007199254740993,1e400,1.00,null]';
const loopStart = performance.now();
for (let i = 0; i < 10000; i++) {
    if (sdk.stringifyJson(sdk.parseJson(text)) !== text) throw new Error('exact JSON roundtrip failed');
}
console.log(JSON.stringify({ importMs, codecLoopMs: performance.now() - loopStart, peakRssKiB:process.resourceUsage().maxRSS, retainedHeapBytes:process.memoryUsage().heapUsed,
    scope: 'installed package import and exact JSON codec loop; not HTTP throughput' }));
"#,
    )?;
    let consumption: Value = serde_json::from_str(&run_command(
        &mut phases,
        "native-consumer",
        &node,
        &["--expose-gc", "measure.mjs"],
        &consumer,
        logs,
    )?)?;
    Ok(
        json!({ "status": "passed", "node": node_version, "npm": npm_version,
        "phases": phases, "tarball": tarball, "consumption": consumption }),
    )
}

fn native_rust(root: &Path, logs: &Path) -> Result<Value> {
    let mut phases = Vec::new();
    let rustc_version = tool_version(&mut phases, "rustc-version", "rustc", logs, root)?;
    let package = root.join("artifacts/rust");
    let target = root.join("native-rust-target");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let cargo = Path::new(cargo.to_str().context("non-UTF8 CARGO")?);
    let target_arg = target.to_string_lossy().into_owned();
    run_command(
        &mut phases,
        "native-cargo-build",
        cargo,
        &[
            "build",
            "--offline",
            "--quiet",
            "--features",
            "http",
            "--target-dir",
            &target_arg,
        ],
        &package,
        logs,
    )?;
    run_command(
        &mut phases,
        "native-cargo-pack",
        cargo,
        &[
            "package",
            "--offline",
            "--quiet",
            "--allow-dirty",
            "--no-verify",
            "--target-dir",
            &target_arg,
        ],
        &package,
        logs,
    )?;
    let archive = target.join(format!("package/{RUST_PACKAGE}-0.0.0.crate"));
    let archive_pin = fingerprint(&archive)?;
    let vendor = root.join("vendor");
    fs::create_dir(&vendor)?;
    run_command(
        &mut phases,
        "native-cargo-install",
        Path::new("tar"),
        &[
            "-xzf",
            archive.to_str().context("archive path")?,
            "-C",
            vendor.to_str().context("vendor path")?,
        ],
        root,
        logs,
    )?;
    let consumer = root.join("rust-consumer");
    fs::create_dir_all(consumer.join("src"))?;
    fs::write(
        consumer.join("Cargo.toml"),
        format!(
            "[package]\nname=\"milestone-consumer\"\nversion=\"0.0.0\"\nedition=\"2024\"\n[workspace]\n[dependencies]\nsdk={{package=\"{RUST_PACKAGE}\",path=\"../vendor/{RUST_PACKAGE}-0.0.0\"}}\n"
        ),
    )?;
    fs::write(
        consumer.join("src/main.rs"),
        r#"fn main() {
    let text = "[9007199254740993,1e400,1.00,null]";
    let start=std::time::Instant::now();
    for _ in 0..10000 {
        let value=sdk::parse_json(text,sdk::JsonLimits::default()).unwrap();
        assert_eq!(sdk::stringify_json(&value,sdk::JsonLimits::default()).unwrap(),text);
    }
    println!("{{\"codecLoopMs\":{},\"iterations\":10000}}",start.elapsed().as_secs_f64()*1000.0);
}
"#,
    )?;
    run_command(
        &mut phases,
        "native-cargo-consumer-build",
        cargo,
        &["build", "--offline", "--quiet", "--target-dir", &target_arg],
        &consumer,
        logs,
    )?;
    let binary = target.join("debug/milestone-consumer");
    let binary_pin = fingerprint(&binary)?;
    let consumption: Value = if cfg!(target_os = "macos") {
        serde_json::from_str(&run_command(
            &mut phases,
            "native-rust-consumer",
            Path::new("/usr/bin/time"),
            &["-l", binary.to_str().context("consumer executable")?],
            &consumer,
            logs,
        )?)?
    } else {
        serde_json::from_str(&run_command(
            &mut phases,
            "native-rust-consumer",
            &binary,
            &[],
            &consumer,
            logs,
        )?)?
    };
    let peak_rss = if cfg!(target_os = "macos") {
        fs::read_to_string(logs.join("native-rust-consumer.stderr"))?
            .lines()
            .find(|line| line.contains("maximum resident set size"))
            .and_then(|line| line.split_whitespace().next())
            .and_then(|value| value.parse::<u64>().ok())
    } else {
        None
    };
    Ok(
        json!({ "status": "passed", "rustc": rustc_version, "phases": phases,
        "targetDir": target.to_string_lossy(),"archive":archive_pin,"consumerBinary":binary_pin,"consumption":consumption,"peakRssBytes":peak_rss,
        "scope": "private Cargo build/pack/install, installed default-feature JSON codec loop and process wall time; HTTP throughput is measured separately" }),
    )
}

fn file_hashes(files: &[OutFile]) -> BTreeMap<String, String> {
    files
        .iter()
        .map(|file| {
            let mut hasher = Sha256::new();
            hasher.update(&file.content);
            (file.path.clone(), format!("{:x}", hasher.finalize()))
        })
        .collect()
}

fn diffs(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let changed: Vec<_> = before
        .iter()
        .filter(|(path, hash)| after.get(*path).is_some_and(|next| next != *hash))
        .map(|(path, _)| path.clone())
        .collect();
    let added: Vec<_> = after
        .keys()
        .filter(|path| !before.contains_key(*path))
        .cloned()
        .collect();
    let removed: Vec<_> = before
        .keys()
        .filter(|path| !after.contains_key(*path))
        .cloned()
        .collect();
    (changed, added, removed)
}

/// Copies the resolved source closure into a private directory, preserving
/// relative layout so intra-closure references stay valid. Never touches sources.
fn private_copy(
    contract: &Contract,
    entry: &Path,
    destination: &Path,
) -> Result<(PathBuf, BTreeMap<Uri, PathBuf>)> {
    let documents: Vec<PathBuf> = contract
        .documents()
        .filter_map(|(uri, _)| uri.as_path())
        .collect();
    let common = documents
        .first()
        .and_then(|first| {
            first
                .parent()?
                .ancestors()
                .find(|root| documents.iter().all(|doc| doc.starts_with(root)))
        })
        .context("documents share no common ancestor")?;
    let mut copied_entry = None;
    let mut mapping = BTreeMap::new();
    for document in &documents {
        let relative = document
            .strip_prefix(common)
            .context("document outside closure root")?;
        let target = destination.join(relative);
        fs::create_dir_all(target.parent().context("document without parent")?)?;
        fs::copy(document, &target)?;
        mapping.insert(Uri::from_path(document)?, target.clone());
        if document.canonicalize()? == entry.canonicalize()? {
            copied_entry = Some(target);
        }
    }
    Ok((
        copied_entry.context("entry document missing from the resolved closure")?,
        mapping,
    ))
}

fn run_scenario(
    name: &'static str,
    destination: &Path,
    entry: &Path,
    contract: &Arc<Contract>,
    selected_names: &[String],
    edited_source: &SourceId,
    mutate: impl FnOnce(&mut Value) -> Result<()>,
) -> Result<Value> {
    let (entry_copy, mapping) = private_copy(contract, entry, destination)
        .with_context(|| format!("private copy for {name} failed"))?;
    let edited_file = &mapping[edited_source.document()];
    let mut document = contract
        .document(edited_source.document())
        .context("edited document absent")?
        .clone();
    fs::write(edited_file, serde_json::to_vec_pretty(&document)?)?;
    let before_workspace = Arc::new(WorkspaceBuilder::new().build()?);
    let before_contract = Arc::new(Contract::from_workspace(
        &before_workspace,
        &Uri::from_path(&entry_copy)?,
    )?);
    let before_selected = selected(&before_contract, selected_names)?;
    let mut untimed = Vec::new();
    let before_ts = ts_files(&before_contract, &before_selected, &mut untimed)?;
    let before_rs = rust_files(&before_contract, &before_selected, &mut untimed)?;
    let allowed = destination.canonicalize()?;
    ensure!(
        before_contract
            .documents()
            .all(|(uri, _)| uri.as_path().is_some_and(|path| path.starts_with(&allowed))),
        "private change scenarios require a self-contained relative reference closure"
    );
    mutate(
        document
            .pointer_mut(edited_source.pointer())
            .context("edited source value absent")?,
    )?;
    fs::write(edited_file, serde_json::to_vec_pretty(&document)?)?;
    let regenerate = Instant::now();
    let mut phases = Vec::new();
    let (ts, rust, selected_ids) = (|| -> Result<_> {
        let workspace = Arc::new(WorkspaceBuilder::new().build()?);
        measure(&mut phases, "entry-read-and-parse", || {
            Ok(workspace.open(Uri::from_path(&entry_copy)?.as_str())?)
        })?;
        let copied = Arc::new(measure(
            &mut phases,
            "reference-closure-and-contract",
            || {
                Ok(Contract::from_workspace(
                    &workspace,
                    &Uri::from_path(&entry_copy)?,
                )?)
            },
        )?);
        ensure!(
            copied
                .documents()
                .all(|(uri, _)| uri.as_path().is_some_and(|path| path.starts_with(&allowed))),
            "changed closure escaped private files"
        );
        let selected = selected(&copied, selected_names)?;
        let ts = ts_files(&copied, &selected, &mut phases)?;
        let rust = rust_files(&copied, &selected, &mut phases)?;
        Ok((ts, rust, selected))
    })()?;
    phases.push(Phase {
        stage: "regenerate",
        milliseconds: regenerate.elapsed().as_secs_f64() * 1000.0,
    });
    let (changed, added, removed) = diffs(&file_hashes(&before_ts), &file_hashes(&ts));
    let (rust_changed, rust_added, rust_removed) =
        diffs(&file_hashes(&before_rs), &file_hashes(&rust));
    let ts_executable_changed = compare::executable_changes(&before_ts, &ts);
    let rust_executable_changed = compare::executable_changes(&before_rs, &rust);
    let artifact_root = destination.join("artifacts");
    let mut before = before_ts;
    before.extend(before_rs);
    let mut after = ts;
    after.extend(rust);
    if name == "docs-only" {
        let document = Uri::from_path(edited_file)?;
        let source = before_contract
            .operations()
            .find(|operation| {
                operation.source().document() == &document
                    && operation.source().pointer() == edited_source.pointer()
            })
            .context("edited operation source")?
            .source()
            .clone();
        compare::docs_only(&before, &after, &source)?;
    }
    write_files(&before, &artifact_root).map_err(anyhow::Error::msg)?;
    let old = stamp(&artifact_root, &before)?;
    measure(&mut phases, "changed-ownership-comparison", || {
        check_files_with_owner(&after, &artifact_root, "suspect-codegen")
            .map_err(anyhow::Error::msg)
    })?;
    measure(&mut phases, "changed-owned-write", || {
        write_files(&after, &artifact_root).map_err(anyhow::Error::msg)
    })?;
    let new = stamp(&artifact_root, &after)?;
    for (path, prior) in &old {
        let relative = path.to_str().context("artifact path must be UTF-8")?;
        if relative != suspect_artifact::OWNERSHIP_MANIFEST
            && !changed.iter().any(|changed| changed == relative)
            && !rust_changed.iter().any(|changed| changed == relative)
            && let Some(next) = new.get(path)
        {
            ensure!(
                prior == next,
                "unchanged change-scenario artifact was rewritten: {relative}"
            );
        }
    }
    Ok(json!({
        "scenario": name, "phases": phases, "selectedOperations": selected_ids.len(),
        "typescript": { "changed": changed, "added": added, "removed": removed,
            "executableChanged": ts_executable_changed },
        "rust": { "changed": rust_changed, "added": rust_added, "removed": rust_removed,
            "executableChanged": rust_executable_changed },
    }))
}

/// Docs-only scenario contract: no added/removed artifacts, every changed
/// executable language file differs only in generated doc comments (semantic
/// hashes equal the baseline), and at least one artifact changed so the
/// docs-only edit demonstrably reached the generated documentation.
fn docs_only_stable(report: &Value) -> Result<()> {
    for target in ["typescript", "rust"] {
        for key in ["added", "removed"] {
            ensure!(
                report[target][key]
                    .as_array()
                    .context("missing diff")?
                    .is_empty(),
                "docs-only scenario has unexpected {target} {key} files: {report}"
            );
        }
        let executable = report[target]["executableChanged"]
            .as_array()
            .context("missing executableChanged")?;
        ensure!(
            executable.is_empty(),
            "docs-only scenario changed executable declarations in {target}: {report}"
        );
    }
    ensure!(
        report["typescript"]["changed"]
            .as_array()
            .context("missing diff")?
            .iter()
            .chain(
                report["rust"]["changed"]
                    .as_array()
                    .context("missing diff")?
            )
            .count()
            > 0,
        "docs-only scenario changed no generated artifact; documentation propagation unproven: {report}"
    );
    Ok(())
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
    let root = parent.join(
        options
            .out
            .file_name()
            .context("output must name a new directory")?,
    );
    fs::create_dir(&root).with_context(|| format!("output must be new: {}", root.display()))?;
    let entry = Uri::from_path(&options.spec)?;
    let entry_before = fingerprint(&options.spec)?;
    let executable = fingerprint(&std::env::current_exe()?)?;
    let mut cold_samples = Vec::new();
    let mut baseline = None;
    let mut retained_contract = None;
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
        let ts = ts_files(&contract, &selected, &mut phases)?;
        let rust = rust_files(&contract, &selected, &mut phases)?;
        let inputs = inputs(&contract)?;
        if let Some((expected, expected_inputs, _)) = &baseline {
            ensure!(
                &(ts, rust) == expected,
                "cold compiler samples produced different artifacts"
            );
            ensure!(
                &inputs == expected_inputs,
                "source closure changed between samples"
            );
        } else {
            baseline = Some(((ts, rust), inputs, selected));
        }
        cold_samples.push(phases);
        retained_contract = Some(contract);
    }
    let (files, source_inputs, selected) = baseline.context("no benchmark samples")?;
    let contract = retained_contract.context("no shared contract")?;
    let mut shared_contract_samples = Vec::new();
    for _ in 0..options.iterations {
        let mut phases = Vec::new();
        let ts = ts_files(&contract, &selected, &mut phases)?;
        let rust = rust_files(&contract, &selected, &mut phases)?;
        ensure!(
            (ts, rust) == files,
            "shared-contract planning changed artifacts"
        );
        shared_contract_samples.push(phases);
    }
    let artifacts = root.join("artifacts");
    let mut all = files.0.clone();
    all.extend(files.1.clone());
    let mut writes = Vec::new();
    measure(&mut writes, "initial-owned-write", || {
        write_files(&all, &artifacts).map_err(anyhow::Error::msg)
    })?;
    let before = stamp(&artifacts, &all)?;
    for _ in 0..options.iterations {
        measure(&mut writes, "unchanged-ownership-check", || {
            let report = check_files_with_owner(&all, &artifacts, "suspect-codegen")
                .map_err(anyhow::Error::msg)?;
            ensure!(
                report.is_current(),
                "unchanged artifact set has drift: {report:?}"
            );
            Ok(())
        })?;
        measure(&mut writes, "unchanged-owned-write", || {
            write_files(&all, &artifacts).map_err(anyhow::Error::msg)
        })?;
        ensure!(
            stamp(&artifacts, &all)? == before,
            "unchanged generation rewrote bytes, mtimes or inodes"
        );
    }
    let private = root.join("private");
    let operation_source = selected
        .first()
        .context("select an operation for change scenarios")?
        .clone();
    let docs_only = run_scenario(
        "docs-only",
        &private.join("docs"),
        &options.spec,
        &contract,
        &options.operation_id,
        &operation_source,
        |operation| {
            operation["description"] = json!("milestone bench documentation edit");
            Ok(())
        },
    )?;
    docs_only_stable(&docs_only)?;
    let mut schema_roots = Vec::new();
    for operation in contract
        .operations()
        .filter(|operation| selected.contains(operation.source()))
    {
        if let Some(body) = operation.request_body() {
            schema_roots.extend(
                body.content()
                    .into_iter()
                    .filter_map(|media| media.schema())
                    .map(|schema| schema.id().clone()),
            );
        }
        for response in operation.responses() {
            schema_roots.extend(
                response
                    .content()
                    .into_iter()
                    .filter_map(|media| media.schema())
                    .map(|schema| schema.id().clone()),
            );
        }
    }
    let schema_source = contract
        .reachable_from(&schema_roots)
        .into_iter()
        .find(|id| {
            contract
                .source(id)
                .is_some_and(|raw| raw["type"] == "object" && raw.get("$ref").is_none())
        })
        .context("schema edit requires a reachable object schema")?;
    let one_schema = run_scenario(
        "one-schema",
        &private.join("schema"),
        &options.spec,
        &contract,
        &options.operation_id,
        &schema_source,
        |schema| {
            if schema.get("properties").is_none() {
                schema["properties"] = json!({});
            }
            schema["properties"]["milestoneBenchProbe"] = json!({"type": "string"});
            if schema.get("required").is_none() {
                schema["required"] = json!([]);
            }
            schema["required"]
                .as_array_mut()
                .context("schema required list")?
                .push(json!("milestoneBenchProbe"));
            Ok(())
        },
    )?;
    let one_operation = run_scenario(
        "one-operation",
        &private.join("operation"),
        &options.spec,
        &contract,
        &options.operation_id,
        &operation_source,
        |operation| {
            let responses = operation["responses"]
                .as_object_mut()
                .context("operation responses")?;
            let value = responses
                .iter()
                .find(|(key, _)| key.parse::<u16>().is_ok())
                .context("exact response")?
                .1
                .clone();
            let code = (400..600)
                .map(|code| code.to_string())
                .find(|code| !responses.contains_key(code))
                .context("available response code")?;
            responses.insert(code, value);
            Ok(())
        },
    )?;
    for scenario in [&one_schema, &one_operation] {
        ensure!(
            !scenario["rust"]["executableChanged"]
                .as_array()
                .context("executable diff")?
                .is_empty()
                && !scenario["typescript"]["executableChanged"]
                    .as_array()
                    .context("executable diff")?
                    .is_empty(),
            "semantic edit did not reach both target plans: {scenario}"
        );
    }
    let logs = root.join("native-logs");
    let native = if options.native {
        fs::create_dir(&logs)?;
        json!({
            "typescript": native_typescript(&root, &logs)?,
            "rust": native_rust(&root, &logs)?,
        })
    } else {
        json!({"status": "not-requested"})
    };
    if fingerprint(&options.spec)? != entry_before || inputs(&contract)? != source_inputs {
        bail!(
            "source changed during the benchmark; measurements are not attributable to one closure"
        );
    }
    ensure!(
        fingerprint(&std::env::current_exe()?)? == executable,
        "benchmark executable changed during measurement"
    );
    let configuration = json!({
        "operationIds":options.operation_id,"iterations":options.iterations,"native":options.native,
        "profiles":["typescript-http","rust-http"],"packageVersion":"0.0.0",
        "typescriptPackage":TS_PACKAGE,"rustPackage":RUST_PACKAGE,
        "policies":"generator-versioned default HTTP, codec and example policies",
    });
    let configuration_sha256 = format!("{:x}", Sha256::digest(serde_json::to_vec(&configuration)?));
    let report = json!({
        "format": REPORT_FORMAT, "observational": true, "releaseReady": false,
        "generatorVersion": env!("CARGO_PKG_VERSION"), "executable": executable,
        "generatorBuild":if cfg!(debug_assertions) {"debug"} else {"release"},
        "configuration":configuration,"configurationSha256":configuration_sha256,
        "entry": entry.to_string(), "inputs": source_inputs, "iterations": options.iterations,
        "schemaNodes":contract.schemas().count(),"referenceEdges":contract.schemas().map(|schema|schema.references().len()).sum::<usize>(),
        "operationIds": options.operation_id,
        "coldCompilerSamples": cold_samples, "sharedContractSamples": shared_contract_samples,
        "writes": writes, "unchangedRewrites": 0,
        "artifacts": {
            "typescript": {"files": files.0.len(),
                "bytes": files.0.iter().map(|file| file.content.len() as u64).sum::<u64>()},
            "rust": {"files": files.1.len(),
                "bytes": files.1.iter().map(|file| file.content.len() as u64).sum::<u64>()}
        },
        "scenarios": {"docsOnly": docs_only, "oneSchema": one_schema, "oneOperation": one_operation},
        "native": native,
        "measurementScope": [
            "One canonical Arc<Contract> per snapshot feeds both targets; cold samples rebuild it, shared-contract samples reuse one immutable instance.",
            "Cold means a fresh compiler/workspace; filesystem and OS page caches are not flushed.",
            "Zero rewrites is verified byte-, mtime- and inode-exact for both targets' artifact sets.",
            "Change scenarios run on private copies; source documents are never modified and are re-fingerprinted afterwards.",
            "Native TS measures pinned offline npm pack/install/import and a JSON codec loop; native Rust measures Cargo build/pack/install and the installed JSON codec loop. HTTP throughput is a separate workload, not inferred from these timings.",
            "Timings are observations, not calibrated p95 regression budgets or complete SDK performance certification."
        ]
    });
    let encoded = serde_json::to_string_pretty(&report)?;
    fs::write(root.join("report.json"), &encoded)?;
    println!("{encoded}");
    Ok(())
}
