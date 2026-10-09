//! `suspect project`: one manifest that wires the toolchain together.
//!
//! A `suspect.project.json` manifest declares the entry OpenAPI document,
//! an ordered overlay pipeline, publication profiles, and the derived
//! targets (docs, contract tests). `check` validates the manifest and its
//! inputs; `build` executes the whole pipeline — overlays in order,
//! published spec, per-profile views, validation, docs, contract tests —
//! failing on the first stage with findings.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use clap::Subcommand;

use crate::output::{Finding, Severity};

/// A `suspect.project.json` manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectManifest {
    /// Manifest schema version; must be `1`.
    pub version: u32,
    /// Project name (defaults to the directory name).
    pub name: String,
    /// Entry OpenAPI document, relative to the manifest directory.
    pub entry: PathBuf,
    /// Overlay documents applied in order to produce the published spec.
    pub overlays: Vec<PathBuf>,
    /// Where the published (post-overlay) spec is written.
    pub publish_output: PathBuf,
    /// Named publication profiles: each is an extra overlay list applied
    /// on top of the published spec (e.g. `public` strips internals).
    pub profiles: BTreeMap<String, Vec<PathBuf>>,
    /// Documentation target: style + output directory.
    pub docs: Option<(String, PathBuf)>,
    /// SDK generation targets built from the published spec.
    pub codegen: Vec<CodegenTarget>,
    /// Where the contract package is written, when the project declares one.
    pub contract: Option<PathBuf>,
    /// Contract-test targets: Arazzo documents run against `base_url`
    /// (or offline from `cassette`).
    pub tests: Option<ProjectTests>,
    /// The manifest's committed policy sections — `lint`, `validate`,
    /// `editor` — carried raw: the config loader interprets them with the
    /// settings file's key spellings, so one schema serves both files and
    /// `.suspect.yaml` remains a local override on top.
    pub policy: serde_json::Value,
}

/// One SDK generation target.
#[derive(Debug, Clone, PartialEq)]
pub struct CodegenTarget {
    /// Target name, used in progress output.
    pub name: String,
    /// The native profile id (`typescript-http`, `python-http`, …).
    pub profile: String,
    /// Native package identity.
    pub package_name: String,
    /// Package SemVer.
    pub package_version: String,
    /// Output root, relative to the manifest.
    pub out: PathBuf,
    /// Exact operationId selectors; empty selects all outgoing operations.
    pub operation_id: Vec<String>,
    /// Explicit import/module/namespace identity where the profile needs one.
    pub import_name: Option<String>,
    /// When true, only ownership/drift is checked; nothing is written.
    pub check: bool,
    /// Generation options: interpretation profiles, credential env mapping,
    /// and SDK defaults (pagination, OAuth lifecycle) — the same schema
    /// `suspect codegen --defaults` accepts.
    pub generation: suspect_codegen::backend::GenerationOptions,
}

/// Lifts the `compatibility_profiles`/`credential_env`/`sdk_defaults`
/// keys of one manifest codegen target into its [`GenerationOptions`].
///
/// # Errors
/// Fails when a present key does not parse as generation options.
pub fn generation_options_from(
    entry: &serde_json::Value,
    index: usize,
) -> anyhow::Result<suspect_codegen::backend::GenerationOptions> {
    // The three keys form one GenerationOptions document; lift whichever
    // are present together.
    let mut options_json = serde_json::Map::new();
    for key in ["compatibility_profiles", "credential_env", "sdk_defaults"] {
        if let Some(value) = entry.get(key) {
            options_json.insert(key.to_owned(), value.clone());
        }
    }
    if options_json.is_empty() {
        return Ok(suspect_codegen::backend::GenerationOptions::default());
    }
    serde_json::from_value::<suspect_codegen::backend::GenerationOptions>(
        serde_json::Value::Object(options_json),
    )
    .map_err(|e| anyhow::anyhow!("codegen target #{index}: invalid generation options: {e}"))
}

/// Contract-test target configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectTests {
    /// Arazzo documents to compile and run.
    pub arazzo: Vec<PathBuf>,
    /// Base URL prepended to operation paths.
    pub base_url: String,
    /// When set, run offline against this cassette instead of live HTTP.
    pub cassette: Option<PathBuf>,
    /// Message broker directory for Arazzo 1.1 AsyncAPI steps.
    pub message_broker: Option<PathBuf>,
    /// Credentials file for the security schemes the suites exercise
    /// (default: discovered `.suspect/credentials.json`).
    pub credentials: Option<PathBuf>,
    /// Workflow inputs supplied to every declared suite: name → value.
    pub inputs: serde_json::Map<String, serde_json::Value>,
}

/// `suspect project` subcommands.
#[derive(Debug, Subcommand)]
pub enum ProjectCmd {
    /// Write a starter `suspect.project.json` in the given directory.
    Init {
        /// Project directory (defaults to the current directory).
        #[arg(default_value = ".")]
        dir: PathBuf,
    },
    /// Validate the manifest and every declared input.
    Check {
        /// Manifest path (defaults to `suspect.project.json`).
        #[arg(long, default_value = "suspect.project.json")]
        manifest: PathBuf,
    },
    /// Execute the build pipeline: overlays → publish → profiles →
    /// validate → docs → tests.
    Build {
        /// Manifest path (defaults to `suspect.project.json`).
        #[arg(long, default_value = "suspect.project.json")]
        manifest: PathBuf,
        /// Skip contract tests (docs/validate still run).
        #[arg(long)]
        skip_tests: bool,
    },
}

/// Parses a manifest document into a [`ProjectManifest`].
///
/// # Errors
/// IO or schema failures with stable field names in the message.
pub fn parse_manifest(path: &Path) -> anyhow::Result<ProjectManifest> {
    let text = std::fs::read_to_string(path)?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| anyhow::anyhow!("{}: invalid JSON: {e}", path.display()))?;
    let dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("manifest root must be a JSON object"))?;
    let version = object
        .get("version")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow::anyhow!("manifest `version` must be 1"))?;
    if version != 1 {
        return Err(anyhow::anyhow!("unsupported manifest version {version}"));
    }
    let entry = object
        .get("entry")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("manifest `entry` is required"))?;
    let resolve = |rel: &str| dir.join(rel);
    let overlays: Vec<PathBuf> = object
        .get("overlays")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).map(resolve).collect())
        .unwrap_or_default();
    let publish_output = object
        .get("publish")
        .and_then(|p| p.get("output"))
        .and_then(|v| v.as_str())
        .map(resolve)
        .unwrap_or_else(|| dir.join("build/spec.yaml"));
    let mut profiles = BTreeMap::new();
    if let Some(list) = object.get("publish").and_then(|p| p.get("profiles"))
        && let Some(map) = list.as_object()
    {
        for (name, overlays) in map {
            let list = overlays
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str()).map(resolve).collect())
                .unwrap_or_default();
            profiles.insert(name.clone(), list);
        }
    }
    let contract = object
        .get("contract")
        .and_then(|c| c.get("output"))
        .and_then(|v| v.as_str())
        .map(resolve);
    let docs = object.get("docs").and_then(|d| {
        let style = d.get("style").and_then(|v| v.as_str())?.to_owned();
        let out = d.get("output").and_then(|v| v.as_str()).map(resolve)?;
        Some((style, out))
    });
    let codegen = object
        .get("codegen")
        .and_then(|v| v.as_array())
        .map(|targets| {
            targets
                .iter()
                .enumerate()
                .map(|(index, entry)| {
                    // A codegen target is a closed object: unknown keys are
                    // a typo about to be silently ignored, and a manifest
                    // that says something the builder never reads is worse
                    // than an error.
                    const KNOWN: &[&str] = &[
                        "name",
                        "profile",
                        "package_name",
                        "package_version",
                        "out",
                        "operation_id",
                        "import_name",
                        "check",
                        "compatibility_profiles",
                        "credential_env",
                        "sdk_defaults",
                    ];
                    if let Some(map) = entry.as_object() {
                        for key in map.keys() {
                            anyhow::ensure!(
                                KNOWN.contains(&key.as_str()),
                                "codegen target #{index}: unknown key `{key}`"
                            );
                        }
                    }
                    let field = |name: &str| -> anyhow::Result<String> {
                        entry
                            .get(name)
                            .and_then(|v| v.as_str())
                            .map(str::to_owned)
                            .ok_or_else(|| {
                                anyhow::anyhow!("codegen target #{index} is missing `{name}`")
                            })
                    };
                    let generation = generation_options_from(entry, index)?;
                    let profile = field("profile")?;
                    Ok(CodegenTarget {
                        name: entry
                            .get("name")
                            .and_then(|v| v.as_str())
                            .map(str::to_owned)
                            .unwrap_or_else(|| profile.clone()),
                        profile,
                        package_name: field("package_name")?,
                        package_version: field("package_version")?,
                        out: entry
                            .get("out")
                            .and_then(|v| v.as_str())
                            .map(resolve)
                            .unwrap_or_else(|| dir.join("sdk")),
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
                        import_name: entry
                            .get("import_name")
                            .and_then(|v| v.as_str())
                            .map(str::to_owned),
                        check: entry
                            .get("check")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false),
                        generation,
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>()
        })
        .transpose()?;
    let tests = object.get("tests").and_then(|t| {
        let arazzo: Vec<PathBuf> = t
            .get("arazzo")?
            .as_array()?
            .iter()
            .filter_map(|v| v.as_str())
            .map(resolve)
            .collect();
        Some(ProjectTests {
            arazzo,
            base_url: t
                .get("base_url")
                .and_then(|v| v.as_str())
                .unwrap_or("http://127.0.0.1:8080")
                .to_owned(),
            cassette: t.get("cassette").and_then(|v| v.as_str()).map(resolve),
            message_broker: t
                .get("message_broker")
                .and_then(|v| v.as_str())
                .map(resolve),
            credentials: t.get("credentials").and_then(|v| v.as_str()).map(resolve),
            inputs: t
                .get("inputs")
                .and_then(|v| v.as_object())
                .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                .unwrap_or_default(),
        })
    });

    Ok(ProjectManifest {
        version: 1,
        name: object
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                dir.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "project".to_owned())
            }),
        entry: resolve(entry),
        overlays,
        publish_output,
        profiles,
        docs,
        codegen: codegen.unwrap_or_default(),
        contract,
        tests,
        policy: ["lint", "validate", "editor"]
            .into_iter()
            .filter_map(|key| {
                object
                    .get(key)
                    .cloned()
                    .map(|value| (key.to_owned(), value))
            })
            .collect::<serde_json::Map<String, serde_json::Value>>()
            .into(),
    })
}

/// Builds one project through the whole pipeline.
///
/// Split out so `suspect ci --build` reuses exactly this pipeline rather
/// than a second copy of it: one implementation, one set of behaviors.
///
/// # Errors
/// Manifest or IO failures.
pub fn build_manifest(manifest: &Path, skip_tests: bool) -> anyhow::Result<i32> {
    build(&parse_manifest(manifest)?, skip_tests)
}

/// Runs the project subcommands.
///
/// # Errors
/// IO, manifest, document, or pipeline failures.
pub fn run(cmd: ProjectCmd) -> anyhow::Result<i32> {
    match cmd {
        ProjectCmd::Init { dir } => {
            let manifest = dir.join("suspect.project.json");
            if manifest.exists() {
                anyhow::bail!("{} already exists", manifest.display());
            }
            let template = serde_json::json!({
                "version": 1,
                "name": dir.file_name().map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "my-api".to_owned()),
                "entry": "openapi.yaml",
                "overlays": [],
                "publish": {"output": "build/spec.yaml", "profiles": {}},
                "docs": {"style": "markdown", "output": "build/docs"},
                "tests": {"arazzo": [], "base_url": "http://127.0.0.1:8080"},
                "codegen": [],
                // Committed policy: the editor, a shell and CI read the
                // same values; `.suspect.yaml` and client settings layer
                // on top as local overrides. Projects documenting an API
                // they do not own add "design": "off" here.
                "lint": {
                    "min_severity": "hint",
                    "rules": {},
                    "recommended": true
                },
                "validate": {"strict_format": false},
                "editor": {
                    "inlay_hints": {"refs": true, "properties": true},
                    "ref": {"max_docs": 500},
                    "formatting": {"sort_keys": true}
                },
                "contract": {"output": "build/contract"}
            });
            std::fs::create_dir_all(&dir)?;
            std::fs::write(
                &manifest,
                format!("{}\n", serde_json::to_string_pretty(&template)?),
            )?;
            eprintln!("wrote {}", manifest.display());
            Ok(0)
        }
        ProjectCmd::Check { manifest } => {
            let project = parse_manifest(&manifest)?;
            let mut findings: Vec<Finding> = Vec::new();
            check_inputs(&project, &mut findings);
            report(&format!("project {}", project.name), &findings)
        }
        ProjectCmd::Build {
            manifest,
            skip_tests,
        } => {
            let project = parse_manifest(&manifest)?;
            build(&project, skip_tests)
        }
    }
}

/// Validates every declared input document exists and parses.
fn check_inputs(project: &ProjectManifest, findings: &mut Vec<Finding>) {
    fn check(path: &Path, kind: &str, findings: &mut Vec<Finding>) {
        if !path.exists() {
            findings.push(Finding {
                file: path.display().to_string(),
                severity: Severity::Error,
                code: "project-missing-input".into(),
                message: format!("{kind} document not found"),
                line: 1,
                col: 1,
                range: None,
            });
        }
    }
    check(&project.entry, "entry spec", findings);
    for overlay in &project.overlays {
        check(overlay, "overlay", findings);
    }
    for overlays in project.profiles.values() {
        for overlay in overlays {
            check(overlay, "profile overlay", findings);
        }
    }
    for target in &project.codegen {
        if crate::commands::sdk::profile_by_name(&target.profile).is_none() {
            findings.push(Finding {
                file: path_label(&target.out),
                severity: Severity::Error,
                code: "project-unknown-profile".into(),
                message: format!("unknown SDK profile `{}`", target.profile),
                line: 1,
                col: 1,
                range: None,
            });
        }
    }
    if let Some(tests) = &project.tests {
        for arazzo in &tests.arazzo {
            check(arazzo, "arazzo", findings);
        }
    }
}

/// Executes the build pipeline.
/// Per-build workspace cache: each distinct spec file is parsed exactly
/// once and every stage — validate, contract, docs, SDKs — consumes the
/// same `Arc<Workspace>`. Before this, one `project build` re-parsed the
/// same published document five times.
struct WorkspaceLoader {
    cache: std::collections::HashMap<PathBuf, std::sync::Arc<suspect_ref::Workspace>>,
}

impl WorkspaceLoader {
    fn new() -> Self {
        Self {
            cache: std::collections::HashMap::new(),
        }
    }

    /// The workspace for `spec`, loaded on first request and shared after.
    ///
    /// # Errors
    /// Workspace loading failures.
    fn load(&mut self, spec: &Path) -> anyhow::Result<std::sync::Arc<suspect_ref::Workspace>> {
        let absolute = spec.canonicalize()?;
        if let Some(cached) = self.cache.get(&absolute) {
            return Ok(std::sync::Arc::clone(cached));
        }
        let ws = std::sync::Arc::new(
            suspect_ref::WorkspaceBuilder::new()
                .root(absolute.parent().unwrap_or(Path::new(".")))
                .build()?,
        );
        ws.load_all(&absolute.display().to_string())?;
        self.cache.insert(absolute, std::sync::Arc::clone(&ws));
        Ok(ws)
    }
}

fn build(project: &ProjectManifest, skip_tests: bool) -> anyhow::Result<i32> {
    let mut loader = WorkspaceLoader::new();
    let profile = std::env::var_os("SUSPECT_BUILD_PROFILE").is_some();
    let mut stage_clock = std::time::Instant::now();
    let mark = |profile: bool, name: &str, clock: &mut std::time::Instant| {
        if profile {
            eprintln!("[build-profile] {name}: {:?}", clock.elapsed());
        }
        *clock = std::time::Instant::now();
    };
    let mut failures = 0usize;

    // Stage 1: overlays in order → published spec.
    let entry = crate::load_doc(&project.entry)?;
    let mut tree = suspect_overlay::Value::from_node(entry.root());
    for overlay_path in &project.overlays {
        let ov_doc = crate::load_doc(overlay_path)?;
        let ov_doc: &'static suspect_low::LowDoc = Box::leak(Box::new(ov_doc));
        let parsed = suspect_overlay::OverlayDoc::parse(ov_doc)?;
        let applied = suspect_overlay::apply(&parsed, doc_root(&tree)?)?;
        tree = applied.output;
        eprintln!(
            "overlay {}: {} applied, {} unmatched",
            overlay_path.display(),
            applied.applied_actions,
            applied.unmatched_targets.len()
        );
    }
    if let Some(parent) = project.publish_output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&project.publish_output, tree.to_yaml())?;
    eprintln!("published {}", project.publish_output.display());

    mark(profile, "publish", &mut stage_clock);
    // Stage 2: publication profiles.
    for (name, overlays) in &project.profiles {
        let mut profile_tree = tree.clone();
        for overlay_path in overlays {
            let ov_doc = crate::load_doc(overlay_path)?;
            let ov_doc: &'static suspect_low::LowDoc = Box::leak(Box::new(ov_doc));
            let parsed = suspect_overlay::OverlayDoc::parse(ov_doc)?;
            let applied = suspect_overlay::apply(&parsed, doc_root(&profile_tree)?)?;
            profile_tree = applied.output;
        }
        let out = project
            .publish_output
            .with_file_name(format!("spec.{name}.yaml"));
        std::fs::write(&out, profile_tree.to_yaml())?;
        eprintln!("profile {name}: {}", out.display());
    }

    mark(profile, "profiles", &mut stage_clock);
    // Stage 3: validate the published spec (and each profile).
    let mut specs = vec![project.publish_output.clone()];
    for name in project.profiles.keys() {
        specs.push(
            project
                .publish_output
                .with_file_name(format!("spec.{name}.yaml")),
        );
    }
    for spec in &specs {
        let ws = loader.load(spec)?;
        let findings = crate::commands::validate::validate_workspace(&ws, spec, false);
        let errors = findings
            .iter()
            .filter(|f| f.severity >= Severity::Error)
            .count();
        eprintln!(
            "validate {}: {errors} error(s), {} finding(s)",
            spec.display(),
            findings.len()
        );
        failures += errors;
    }

    mark(profile, "validate", &mut stage_clock);
    // Stage 3b: contract package, when the project declares one.
    if let Some(out) = &project.contract {
        let ws = loader.load(&project.publish_output)?;
        let exit = crate::commands::contract::contract_on_workspace(
            &ws,
            &project.publish_output,
            &crate::commands::contract::ContractArgs {
                input: project.publish_output.clone(),
                out: out.clone(),
                json: false,
                overlays: Vec::new(),
                check: false,
            },
        )?;
        if exit != 0 {
            failures += 1;
        }
    }

    mark(profile, "contract", &mut stage_clock);
    // Stage 4: docs. An unrecognized style is a typo, not a silent
    // fallback to HTML.
    if let Some((style, out)) = &project.docs {
        let style = match style.as_str() {
            "markdown" => crate::commands::docs_gen_cmd::DocsStyle::Markdown,
            "sveltekit" => crate::commands::docs_gen_cmd::DocsStyle::Sveltekit,
            "html" => crate::commands::docs_gen_cmd::DocsStyle::Html,
            other => {
                anyhow::bail!("docs style `{other}` is not one of: markdown, sveltekit, html");
            }
        };
        let args = crate::commands::docs_gen_cmd::DocsGenArgs {
            input: project.publish_output.clone(),
            style: Some(style),
            output: Some(out.clone()),
            title: None,
        };
        let ws = loader.load(&project.publish_output)?;
        let entry = suspect_source::Uri::from_path(
            &project
                .publish_output
                .canonicalize()
                .unwrap_or_else(|_| project.publish_output.clone()),
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?;
        // The workspace's own parse of the published spec: the same
        // document every other stage consumes, borrowed — no re-read, no
        // copy, no re-parse.
        let handle = ws
            .get(&entry)
            .ok_or_else(|| anyhow::anyhow!("published spec is not loaded"))?;
        crate::commands::docs_gen_cmd::docs_gen_parsed(handle.doc(), &args)?;
    }

    mark(profile, "docs", &mut stage_clock);
    // Stage 4b: SDK generation targets. The published spec is compiled
    // once — scoped to the union of the targets' operation selectors —
    // and every target's render runs in parallel against that one
    // contract; the commits stay serial and in manifest order.
    if !project.codegen.is_empty() {
        let ws = loader.load(&project.publish_output)?;
        let shared = std::sync::Arc::new(crate::commands::sdk::SharedSpec::from_workspace_scoped(
            ws,
            &project.publish_output,
            &project.codegen,
        )?);
        let exit =
            crate::commands::sdk::generate_codegen_targets_shared(&shared, &project.codegen)?;
        if exit != 0 {
            failures += 1;
        }
    }

    mark(profile, "sdks", &mut stage_clock);
    // Stage 5: contract tests.
    if !skip_tests
        && let Some(tests) = &project.tests
        && !tests.arazzo.is_empty()
    {
        for arazzo in &tests.arazzo {
            let exit = crate::commands::test::test_with_messages(
                arazzo,
                &tests.base_url,
                None,
                &tests.inputs,
                tests.cassette.as_deref(),
                false,
                tests.message_broker.as_deref(),
                tests.credentials.as_deref(),
                None,
            )?;
            if exit != 0 {
                failures += 1;
            }
        }
    }

    mark(profile, "tests", &mut stage_clock);
    mark(profile, "tests", &mut stage_clock);
    if failures > 0 {
        eprintln!("project build: {failures} failure(s)");
        Ok(1)
    } else {
        eprintln!("project build: ok");
        Ok(0)
    }
}

/// Re-parses the owned tree into a scratch doc for overlay application.
fn doc_root(tree: &suspect_overlay::Value) -> anyhow::Result<suspect_low::NodeRef<'static>> {
    let yaml = tree.to_yaml();
    let doc: &'static suspect_low::LowDoc = Box::leak(Box::new(suspect_low::LowDoc::parse(
        "mem://project-target.yaml".into(),
        suspect_source::Source::from_vec(yaml.into_bytes()),
    )));
    Ok(doc.root())
}

/// A stable display label for a path inside a finding.
fn path_label(path: &Path) -> String {
    path.display().to_string()
}

/// Human label for a finding severity.
fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
        Severity::Hint => "hint",
    }
}

/// Prints findings and returns the exit code.
fn report(label: &str, findings: &[Finding]) -> anyhow::Result<i32> {
    eprintln!("{label}: {} finding(s)", findings.len());
    for f in findings {
        eprintln!(
            "  {}: {} [{}] {}",
            f.file,
            severity_label(f.severity),
            f.code,
            f.message
        );
    }
    Ok(i32::from(
        findings.iter().any(|f| f.severity >= Severity::Error),
    ))
}
