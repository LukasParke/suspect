//! `suspect ci`: one gate across a whole workspace.
//!
//! Discovers every `suspect.project.json` (or, absent any, every
//! `suspect.project.json` parent spec) beneath a root and runs the stages
//! that are configured: validation, lint, contract drift, breaking against
//! a baseline, generated-artifact drift, and contract tests. Every stage
//! runs for every project — one failing service does not hide the state of
//! the other nine — and the aggregate is reported per project so a CI log
//! reads as a service-by-service scorecard.

use std::path::{Path, PathBuf};

use clap::Args;
use serde::Serialize;

use crate::OutputFormat;

/// One CI run.
#[derive(Debug, Args)]
pub struct CiArgs {
    /// Root to search for projects.
    #[arg(default_value = ".")]
    root: PathBuf,
    /// Git ref each project is compared against for breaking changes.
    #[arg(long, value_name = "REF")]
    baseline: Option<String>,
    /// Only run these stages (repeatable): validate, lint, contract,
    /// breaking, codegen, test.
    #[arg(long = "stage", value_name = "NAME")]
    stages: Vec<String>,
    /// Skip contract tests (they may need a live server).
    #[arg(long)]
    skip_tests: bool,
    /// Build every project before gating it: overlays, publication,
    /// validation, contract, docs and SDK generation, then the gate. One
    /// command from a clean checkout to a verified workspace.
    #[arg(long)]
    build: bool,
    /// Output format for the aggregate report.
    #[command(flatten)]
    pub text: crate::TextFormat,
}

/// One stage's outcome for one project.
#[derive(Debug, Clone, Serialize)]
pub struct StageResult {
    /// Stage name.
    pub stage: String,
    /// Whether it passed.
    pub passed: bool,
    /// Findings at Error or above.
    pub errors: usize,
    /// Findings below Error.
    pub warnings: usize,
    /// One-line summary.
    pub summary: String,
}

/// One project's outcome.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectResult {
    /// Project name from its manifest.
    pub name: String,
    /// Manifest path.
    pub manifest: String,
    /// Stage outcomes in run order.
    pub stages: Vec<StageResult>,
}

impl ProjectResult {
    /// Whether every stage passed.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.stages.iter().all(|s| s.passed)
    }

    /// Total errors across stages.
    #[must_use]
    pub fn errors(&self) -> usize {
        self.stages.iter().map(|s| s.errors).sum()
    }
}

/// The aggregate report.
#[derive(Debug, Serialize)]
pub struct CiReport {
    /// Format identifier.
    pub format: String,
    /// Root searched.
    pub root: String,
    /// Per-project outcomes.
    pub projects: Vec<ProjectResult>,
    /// Projects that passed every configured stage.
    pub passed: usize,
    /// Projects with at least one failing stage.
    pub failed: usize,
}

/// Format identifier.
const FORMAT: &str = "suspect.ci.v1";

/// Every `suspect.project.json` beneath `root`, sorted.
#[must_use]
pub fn discover_projects(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    collect(root, 0, &mut found);
    found.sort();
    found
}

fn collect(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    // Bounded so a mis-aimed root cannot walk the whole filesystem.
    if depth > 8 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            // Skip dependency and VCS directories, which are never projects.
            if matches!(
                name.as_str(),
                "node_modules" | "target" | ".git" | "vendor" | "dist" | "build" | ".next"
            ) {
                continue;
            }
            collect(&path, depth + 1, out);
        } else if name == "suspect.project.json" {
            out.push(path);
        }
    }
}

/// Runs the gate.
///
/// # Errors
/// Manifest or IO failures; a failing stage is reported, not raised.
pub fn ci(args: &CiArgs) -> anyhow::Result<i32> {
    let root = args
        .root
        .canonicalize()
        .unwrap_or_else(|_| args.root.clone());
    let projects = discover_projects(&root);
    if projects.is_empty() {
        eprintln!(
            "ci: no suspect.project.json found under {} (run `suspect project init` in a service)",
            root.display()
        );
        return Ok(2);
    }
    let wanted: Vec<&str> = if args.stages.is_empty() {
        [
            "validate", "lint", "contract", "breaking", "codegen", "test",
        ]
        .into_iter()
        .collect()
    } else {
        args.stages.iter().map(String::as_str).collect()
    };

    // A machine-readable run emits exactly one document: stage chatter is
    // redirected away so a CI consumer can parse stdout directly.
    let machine_readable = !matches!(args.text.format, OutputFormat::Text);
    let _silence = machine_readable.then(crate::silence::Silenced::start);

    let mut results = Vec::new();
    for manifest in &projects {
        // Building first is what makes this one command rather than two:
        // the gate then checks artifacts that were just produced.
        if args.build {
            build_project(manifest, args.skip_tests);
        }
        let mut stages = Vec::new();
        if wanted.contains(&"validate") {
            stages.push(validate_stage(manifest));
        }
        if wanted.contains(&"lint") {
            stages.push(lint_stage(manifest));
        }
        if wanted.contains(&"contract") {
            stages.push(contract_stage(manifest));
        }
        if wanted.contains(&"breaking") {
            stages.push(breaking_stage(manifest, args.baseline.as_deref()));
        }
        if wanted.contains(&"codegen") {
            stages.push(codegen_stage(manifest));
        }
        if wanted.contains(&"test") && !args.skip_tests {
            stages.push(test_stage(manifest));
        }
        let name = project_name(manifest);
        results.push(ProjectResult {
            name,
            manifest: manifest.display().to_string(),
            stages,
        });
    }

    let report = CiReport {
        format: FORMAT.to_owned(),
        root: root.display().to_string(),
        passed: results.iter().filter(|r| r.passed()).count(),
        failed: results.iter().filter(|r| !r.passed()).count(),
        projects: results,
    };

    // Print after the silencing guard is dropped so the report itself is
    // never redirected.
    let _ = &report;
    drop(_silence);
    match args.text.format {
        OutputFormat::Json | OutputFormat::Sarif => {
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        OutputFormat::Text => {
            for project in &report.projects {
                let verdict = if project.passed() { "PASS" } else { "FAIL" };
                println!("{verdict}  {}", project.name);
                for stage in &project.stages {
                    let mark = if stage.passed { "ok  " } else { "FAIL" };
                    println!(
                        "        [{mark}] {:<9} {} error(s), {} warning(s) — {}",
                        stage.stage, stage.errors, stage.warnings, stage.summary
                    );
                }
            }
            println!();
            println!(
                "{} project(s): {} passed, {} failed",
                report.projects.len(),
                report.passed,
                report.failed
            );
        }
    }
    Ok(i32::from(report.failed > 0))
}

/// The entry spec a manifest declares.
fn entry_spec(manifest: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(manifest).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let entry = value.get("entry")?.as_str()?;
    Some(manifest.parent()?.join(entry))
}

/// The contract package output a manifest declares.
fn contract_output(manifest: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(manifest).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    let out = value
        .get("contract")
        .and_then(|c| c.get("output"))
        .and_then(|v| v.as_str())?;
    Some(manifest.parent()?.join(out))
}

/// The project name from a manifest, falling back to its directory.
fn project_name(manifest: &Path) -> String {
    std::fs::read_to_string(manifest)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|value| {
            value
                .get("name")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| {
            manifest
                .parent()
                .and_then(|dir| dir.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "project".to_owned())
        })
}

/// The document a stage should check: the published spec once it exists
/// (that is the artifact consumers actually see), otherwise the entry
/// spec. Without this fallback, `suspect ci --stage lint` on a clean
/// checkout could only ever report "never published", which is noise
/// rather than a finding.
fn spec_to_check(manifest: &Path) -> Option<PathBuf> {
    let published = published_spec(manifest).filter(|path| path.exists());
    published.or_else(|| entry_spec(manifest).filter(|path| path.exists()))
}

/// The published spec path a manifest declares.
fn published_spec(manifest: &Path) -> Option<PathBuf> {
    let dir = manifest.parent()?;
    let text = std::fs::read_to_string(manifest).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value
        .get("publish")
        .and_then(|p| p.get("output"))
        .and_then(|v| v.as_str())
        .map(|out| dir.join(out))
}

fn stage(name: &str, exit: i32, errors: usize, warnings: usize, summary: String) -> StageResult {
    StageResult {
        stage: name.to_owned(),
        passed: exit == 0,
        errors,
        warnings,
        summary,
    }
}

/// Lints the published spec with the project's ruleset and severity floor.
///
/// A project may declare its own `lint:` section; otherwise the shared
/// `.suspect.yaml` settings apply, exactly as they do for `suspect lint`
/// run by hand — so CI and a developer see the same findings.
fn lint_stage(manifest: &Path) -> StageResult {
    let Some(spec) = spec_to_check(manifest) else {
        return stage("lint", 0, 0, 0, "no entry or published spec".to_owned());
    };
    let (ruleset, floor) = lint_policy(manifest);
    let findings = match crate::commands::lint::lint_findings(&[spec], ruleset.as_deref(), floor) {
        Ok(findings) => findings,
        Err(error) => return stage("lint", 1, 1, 0, error.to_string()),
    };
    let errors = findings
        .iter()
        .filter(|f| f.severity >= crate::output::Severity::Error)
        .count();
    let warnings = findings.len() - errors;
    stage(
        "lint",
        i32::from(errors > 0),
        errors,
        warnings,
        match (&ruleset, floor == crate::output::Severity::Hint) {
            (Some(ruleset), _) => format!("{} ruleset", ruleset.display()),
            (None, true) => "built-in ruleset".to_owned(),
            (None, false) => format!("built-in ruleset, at or above {floor:?}"),
        },
    )
}

/// A project's own lint policy, else the shared configuration file.
fn lint_policy(manifest: &Path) -> (Option<PathBuf>, crate::output::Severity) {
    let dir = manifest.parent().unwrap_or(Path::new("."));
    let declared: Option<serde_json::Value> = std::fs::read_to_string(manifest)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok());
    let ruleset = declared
        .as_ref()
        .and_then(|value| value.get("lint"))
        .and_then(|lint| lint.get("ruleset"))
        .and_then(|v| v.as_str())
        .map(|rel| dir.join(rel))
        .or_else(|| {
            suspect_config::for_invocation(Some(manifest))
                .ok()
                .and_then(|loaded| loaded.settings.lint.ruleset)
        });
    let floor = declared
        .as_ref()
        .and_then(|value| value.get("lint"))
        .and_then(|lint| lint.get("min_severity"))
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .or_else(|| {
            suspect_config::for_invocation(Some(manifest))
                .ok()
                .and_then(|loaded| loaded.settings.lint.min_severity)
        })
        .and_then(|name| match name.to_ascii_lowercase().as_str() {
            "error" => Some(crate::output::Severity::Error),
            "warning" => Some(crate::output::Severity::Warning),
            "info" => Some(crate::output::Severity::Info),
            "hint" => Some(crate::output::Severity::Hint),
            _ => None,
        })
        .unwrap_or(crate::output::Severity::Hint);
    (ruleset, floor)
}

/// Builds one project through the full pipeline, reporting as it goes.
///
/// Failures surface through the gate stages that follow, so a build that
/// fails is visible as a failing project rather than aborting the
/// workspace.
fn build_project(manifest: &Path, skip_tests: bool) {
    eprintln!("ci: building {}", manifest.display());
    match crate::commands::project::build_manifest(manifest, skip_tests) {
        Ok(_) => {}
        Err(error) => eprintln!("ci: build failed: {error}"),
    }
}

fn validate_stage(manifest: &Path) -> StageResult {
    let Some(spec) = published_spec(manifest) else {
        return stage("validate", 0, 0, 0, "no published spec declared".to_owned());
    };
    if !spec.exists() {
        return stage(
            "validate",
            1,
            1,
            0,
            format!("{} was never published", spec.display()),
        );
    }
    let findings = crate::commands::validate::validate_file(&spec, None, false);
    let errors = findings
        .iter()
        .filter(|f| f.severity >= crate::output::Severity::Error)
        .count();
    let warnings = findings.len() - errors;
    let exit = i32::from(errors > 0);
    stage(
        "validate",
        exit,
        errors,
        warnings,
        format!("{}", spec.display()),
    )
}

fn contract_stage(manifest: &Path) -> StageResult {
    if contract_output(manifest).is_none() {
        return stage(
            "contract",
            0,
            0,
            0,
            "no contract package declared".to_owned(),
        );
    }
    let Some(spec) = published_spec(manifest) else {
        return stage("contract", 0, 0, 0, "no published spec declared".to_owned());
    };
    if !spec.exists() {
        return stage(
            "contract",
            1,
            1,
            0,
            format!("{} was never published", spec.display()),
        );
    }
    let out = contract_output(manifest).unwrap_or_else(|| {
        spec.parent()
            .map(|d| d.join("contract"))
            .unwrap_or_default()
    });
    let check = crate::commands::contract::contract(&crate::commands::contract::ContractArgs {
        input: spec,
        out,
        json: false,
        overlays: Vec::new(),
        check: true,
    });
    match check {
        Ok(0) => stage("contract", 0, 0, 0, "package up to date".to_owned()),
        Ok(_) => stage("contract", 1, 1, 0, "contract package is stale".to_owned()),
        Err(error) => stage("contract", 1, 1, 0, error.to_string()),
    }
}

fn breaking_stage(manifest: &Path, baseline: Option<&str>) -> StageResult {
    // The baseline is the committed *source* spec at the ref, compared
    // against the spec the project publishes: build artifacts are not in
    // git, and the source is what a reviewer actually changed.
    let Some(source) = entry_spec(manifest) else {
        return stage("breaking", 0, 0, 0, "no entry spec declared".to_owned());
    };
    let Some(current) = published_spec(manifest).or_else(|| Some(source.clone())) else {
        return stage("breaking", 0, 0, 0, "no published spec".to_owned());
    };
    let Some(git_ref) = baseline else {
        return stage(
            "breaking",
            0,
            0,
            0,
            "no --baseline: pass a git ref to gate on".to_owned(),
        );
    };
    let staged = match crate::baseline::materialize(git_ref, &source) {
        Ok(staged) => staged,
        Err(error) => return stage("breaking", 1, 1, 0, error.to_string()),
    };
    let findings = crate::commands::breaking::break_findings(&staged, &current);
    crate::baseline::cleanup(&staged);
    match findings {
        Ok(findings) => {
            let errors = findings.iter().filter(|f| f.severity == "error").count();
            let warnings = findings.len() - errors;
            stage(
                "breaking",
                i32::from(errors > 0),
                errors,
                warnings,
                format!("against {git_ref}"),
            )
        }
        Err(error) => stage("breaking", 1, 1, 0, error.to_string()),
    }
}

fn codegen_stage(manifest: &Path) -> StageResult {
    let text = match std::fs::read_to_string(manifest) {
        Ok(text) => text,
        Err(error) => return stage("codegen", 1, 1, 0, error.to_string()),
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return stage("codegen", 1, 1, 0, "manifest is not JSON".to_owned());
    };
    let Some(targets) = value.get("codegen").and_then(|v| v.as_array()) else {
        return stage("codegen", 0, 0, 0, "no codegen targets".to_owned());
    };
    let dir = manifest.parent().unwrap_or(Path::new("."));
    let mut drifted = 0usize;
    for (index, entry) in targets.iter().enumerate() {
        let field = |name: &str| entry.get(name).and_then(|v| v.as_str()).map(str::to_owned);
        let (Some(profile), Some(package_name), Some(package_version)) = (
            field("profile"),
            field("package_name"),
            field("package_version"),
        ) else {
            drifted += 1;
            eprintln!("ci: codegen target #{index} is missing required fields");
            continue;
        };
        let Some(spec) = published_spec(manifest) else {
            drifted += 1;
            continue;
        };
        let out = field("out")
            .map(|out| dir.join(out))
            .unwrap_or_else(|| dir.join("sdk"));
        // Drift check: regenerate into a read-only comparison.
        let target = crate::commands::project::CodegenTarget {
            name: field("name").unwrap_or_else(|| profile.clone()),
            profile,
            package_name,
            package_version,
            out,
            operation_id: entry
                .get("operation_id")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            import_name: field("import_name"),
            check: true,
            generation: match crate::commands::project::generation_options_from(entry, index) {
                Ok(options) => options,
                Err(error) => {
                    drifted += 1;
                    eprintln!("ci: {error}");
                    continue;
                }
            },
        };
        if crate::commands::sdk::generate_codegen_target(&spec, &target).unwrap_or(1) != 0 {
            drifted += 1;
        }
    }
    stage(
        "codegen",
        i32::from(drifted > 0),
        drifted,
        0,
        format!(
            "{}/{} target(s) current",
            targets.len() - drifted.min(targets.len()),
            targets.len()
        ),
    )
}

fn test_stage(manifest: &Path) -> StageResult {
    let text = match std::fs::read_to_string(manifest) {
        Ok(text) => text,
        Err(error) => return stage("test", 1, 1, 0, error.to_string()),
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return stage("test", 1, 1, 0, "manifest is not JSON".to_owned());
    };
    let Some(tests) = value.get("tests") else {
        return stage("test", 0, 0, 0, "no contract tests declared".to_owned());
    };
    let Some(arazzo) = tests.get("arazzo").and_then(|v| v.as_array()) else {
        return stage("test", 0, 0, 0, "no contract tests declared".to_owned());
    };
    let dir = manifest.parent().unwrap_or(Path::new("."));
    let base_url = tests
        .get("base_url")
        .and_then(|v| v.as_str())
        .unwrap_or("http://127.0.0.1:8080");
    let mut failed = 0usize;
    for entry in arazzo {
        let Some(path) = entry.as_str() else {
            failed += 1;
            continue;
        };
        let cassette = tests
            .get("cassette")
            .and_then(|v| v.as_str())
            .map(|c| dir.join(c));
        let broker = tests
            .get("message_broker")
            .and_then(|v| v.as_str())
            .map(|b| dir.join(b));
        let credentials = tests
            .get("credentials")
            .and_then(|v| v.as_str())
            .map(|c| dir.join(c));
        let exit = crate::commands::test::test_with_messages(
            &dir.join(path),
            base_url,
            None,
            cassette.as_deref(),
            false,
            broker.as_deref(),
            credentials.as_deref(),
            None,
        )
        .unwrap_or(1);
        if exit != 0 {
            failed += 1;
        }
    }
    stage(
        "test",
        i32::from(failed > 0),
        failed,
        0,
        format!("{}/{} suite(s) green", arazzo.len() - failed, arazzo.len()),
    )
}
