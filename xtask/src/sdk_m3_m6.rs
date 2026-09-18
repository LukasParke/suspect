//! Greenfield canonical SDK evidence for the revised M3/M6 scope.
//!
//! Keep this runner independent of codegen so its acceptance logic can be checked
//! in isolation while backend owners are editing. `main` only calls `run(&args)`.

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const USAGE: &str = "usage: cargo run -p xtask -- sdk-m3-m6 --source OPENROUTER_ROOT --out NEW_TARGET_DIRECTORY [--demo | --performance-plan PLAN_JSON]\n       cargo run -p xtask -- sdk-m3-m6 --list-stages [--demo]\n--demo verifies generation, native SDKs and iteration tools; latency remains observational. Full acceptance requires qualified performance evidence. See docs/SDK-M3-M6-EXIT.md.\n";
const BOTH: &[&str] = &["M3", "M6"];
const M3: &[&str] = &["M3"];
const M6: &[&str] = &["M6"];
#[rustfmt::skip]
pub(super) const INPUTS: &[(&str, &str, &str)] = &[
    ("public", "projects/docs/openapi/openapi.yaml", "bd4953b29f34de134ed4be27b2803c5622a756c3c63bc8617636b6abe5de1821"),
    ("management", "openrouter-management.openapi.yaml", "c1ed00af257808f00c2a070b28ace135954f9cc1f97111a48364a35eae0505f2"),
    ("provider", "projects/docs/assets/provider-monitor-schema-v2.openapi.json", "5e8d14ed38af861e2c0d4827367d7f00d1a55722a5f62199e821ecc84241a355"),
    ("temporal", "packages/temporal/benchmarks.openapi.json", "e0fea9dc837eb24a3d82e8d5334fffd154d83cf22c44782b9651a36416b54ea7"),
];
pub(super) const OPERATIONS: &[&str] = &[
    "getCredits",
    "createKeys",
    "updateKeys",
    "listContainerFiles",
    "getContainerFile",
];
pub(super) const PERFORMANCE: &[&str] = &[
    "performance-small",
    "performance-split-recursive",
    "performance-openrouter",
    "performance-compare",
];
const CODEGEN_LIB: &str = "suspect_codegen";
const SWIFT_VECTORS: &str = "swift_sdk::validation::tests::native_shared_runtime_contract_vectors";

#[derive(Debug)]
struct Args {
    source: PathBuf,
    out: PathBuf,
    performance_plan: Option<PathBuf>,
    demo: bool,
}

impl Args {
    fn parse(args: &[String]) -> Result<Self> {
        let mut values = BTreeMap::new();
        let mut demo = false;
        let mut args = args.iter();
        while let Some(key) = args.next() {
            if key == "--demo" {
                ensure!(!demo, "duplicate --demo");
                demo = true;
                continue;
            }
            ensure!(
                ["--source", "--out", "--performance-plan"].contains(&key.as_str()),
                "unknown option {key}\n{USAGE}"
            );
            let value = args
                .next()
                .with_context(|| format!("{key} requires a value"))?;
            ensure!(
                values.insert(key.as_str(), value.clone()).is_none(),
                "duplicate {key}"
            );
        }
        let performance_plan = values.remove("--performance-plan").map(PathBuf::from);
        ensure!(
            !demo || performance_plan.is_none(),
            "--demo keeps latency observational; use --performance-plan for full acceptance"
        );
        let mut take = |key| {
            values
                .remove(key)
                .with_context(|| format!("{key} is required\n{USAGE}"))
        };
        let result = Self {
            source: take("--source")?.into(),
            out: take("--out")?.into(),
            performance_plan,
            demo,
        };
        Ok(result)
    }
}

/// A Main-wired performance command. Each report must be newly produced inside
/// `{out}/performance/`; assertions use RFC 6901 pointers, not text grep.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PerformanceStage {
    id: String,
    program: String,
    args: Vec<String>,
    cwd: String,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    report: String,
    assertions: BTreeMap<String, Value>,
    /// Named obligations -> RFC 6901 pointers in the tool's actual report.
    claims: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PerformancePlan {
    format: String,
    /// Every external baseline/config consumed by these commands is hash-pinned.
    inputs: Vec<PinnedInput>,
    stages: Vec<PerformanceStage>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PinnedInput {
    path: PathBuf,
    sha256: String,
}

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(super) enum Criterion {
    Exit,
    RustTests,
    NamedRustTest {
        name: String,
    },
    WorkspaceTests,
    NodeTests,
    Contains {
        text: String,
    },
    Json {
        report: Option<String>,
        assertions: BTreeMap<String, Value>,
    },
    Performance {
        report: String,
        assertions: BTreeMap<String, Value>,
        claims: BTreeMap<String, String>,
        comparison: bool,
    },
    TestBuild {
        suites: Vec<String>,
    },
    AppleTool {
        name: String,
    },
    SdkSettings {
        root: String,
    },
    CurrentSwiftSdk,
    ToolPath,
}

#[derive(Clone, Serialize)]
pub(super) struct Stage {
    pub(super) id: String,
    pub(super) milestones: Vec<String>,
    pub(super) program: String,
    pub(super) args: Vec<String>,
    pub(super) cwd: String,
    pub(super) environment: BTreeMap<String, String>,
    pub(super) expected_exit: i32,
    pub(super) criterion: Criterion,
}

pub(super) fn stage(
    id: &str,
    milestones: &[&str],
    program: &str,
    args: &[&str],
    cwd: &str,
) -> Stage {
    Stage {
        id: id.into(),
        milestones: strings(milestones),
        program: program.into(),
        args: strings(args),
        cwd: cwd.into(),
        environment: BTreeMap::new(),
        expected_exit: 0,
        criterion: Criterion::Exit,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct FileRecord {
    pub(super) kind: String,
    pub(super) sha256: String,
    pub(super) bytes: u64,
    pub(super) mode: u32,
    pub(super) link: Option<String>,
}

pub(super) type Inventory = BTreeMap<String, FileRecord>;

pub(super) struct Run {
    pub(super) original: PathBuf,
    pub(super) source: PathBuf,
    pub(super) out: PathBuf,
    pub(super) work: PathBuf,
    pub(super) env: BTreeMap<String, String>,
    pub(super) replacements: BTreeMap<String, String>,
    pub(super) checks: Vec<Value>,
    pub(super) tests: BTreeMap<String, PathBuf>,
    pub(super) files: BTreeMap<PathBuf, String>,
    pub(super) absent_configs: BTreeSet<PathBuf>,
    pub(super) provenance: Value,
}

impl Run {
    pub(super) fn expand(&self, text: &str) -> String {
        self.replacements
            .iter()
            .fold(text.into(), |text, (key, value)| text.replace(key, value))
    }

    pub(super) fn remember(&mut self, path: &Path) -> Result<()> {
        // Cargo legitimately rebuilds harnesses under broader feature unification.
        // Each invocation records their before/after digest; the frozen CLI lives
        // separately in the report and remains an end-to-end integrity input.
        if path.starts_with(self.work.join("cargo")) {
            return Ok(());
        }
        let digest = hash(path)?;
        if let Some(previous) = self.files.insert(path.to_owned(), digest.clone()) {
            ensure!(
                previous == digest,
                "evidence/tool changed during execution: {}",
                path.display()
            );
        }
        Ok(())
    }

    pub(super) fn record(&mut self, check: Value) -> Result<()> {
        let id = check["id"].as_str().context("check ID")?;
        ensure!(
            !self.checks.iter().any(|old| old["id"] == id),
            "duplicate check {id}"
        );
        write_json(&self.out.join(format!("checks/{id}.json")), &check)?;
        println!(
            "{id}: {}",
            if check["criterionMet"] == true {
                "met"
            } else {
                "NOT MET"
            }
        );
        self.checks.push(check);
        Ok(())
    }

    fn internal(&mut self, id: &str, result: Result<Value>) -> Result<()> {
        self.verification(id, &strings(BOTH), result)
    }

    pub(super) fn verification(
        &mut self,
        id: &str,
        milestones: &[String],
        result: Result<Value>,
    ) -> Result<()> {
        let (met, detail) = match result {
            Ok(value) => (true, value),
            Err(error) => (false, json!({"error":format!("{error:#}")})),
        };
        self.record(json!({"id":id,"kind":"verification","required":true,"milestones":milestones,"success":met,"criterionMet":met,"detail":detail}))
    }

    pub(super) fn command(&mut self, item: &Stage) -> Result<()> {
        if matches!(item.criterion, Criterion::CurrentSwiftSdk)
            && let Some(requested) = self.replacements.get("{swift-sdk-requested}").cloned()
        {
            let result = self.pin_current_sdk(Path::new(&requested), true);
            return self.verification(&item.id, &item.milestones, result);
        }
        if let Criterion::SdkSettings { root } = &item.criterion {
            let path = PathBuf::from(self.expand(root)).join("SDKSettings.json");
            let result = (|| -> Result<Value> {
                let bytes = fs::read(&path)
                    .with_context(|| format!("read actual SDK settings {}", path.display()))?;
                let metadata = sdk_settings(&bytes)?;
                let snapshot = format!("tool-metadata/{}.json", item.id);
                create(&self.out.join(&snapshot))?.write_all(&bytes)?;
                self.remember(&path)?;
                Ok(
                    json!({"path":path,"sha256":sha(&bytes),"snapshot":snapshot,"metadata":metadata}),
                )
            })();
            return self.verification(&item.id, &item.milestones, result);
        }
        let mut env = self.env.clone();
        env.extend(
            item.environment
                .iter()
                .map(|(k, v)| (k.clone(), self.expand(v))),
        );
        if matches!(item.criterion, Criterion::CurrentSwiftSdk) {
            match self
                .current_tool_directory()
                .and_then(|directory| developer_directory(&directory))
            {
                Ok(developer) => {
                    env.insert("DEVELOPER_DIR".into(), developer.display().to_string());
                }
                Err(error) => return self.verification(&item.id, &item.milestones, Err(error)),
            }
        }
        let requested = self.expand(&item.program);
        let program = if let Some(suite) = requested.strip_prefix("test:") {
            self.tests
                .get(suite)
                .cloned()
                .unwrap_or_else(|| self.work.join("missing-test").join(suite))
        } else {
            resolve(&requested, &env)
        };
        let args = item
            .args
            .iter()
            .map(|arg| self.expand(arg))
            .collect::<Vec<_>>();
        let cwd = self.expand(&item.cwd);
        let stdout = format!("logs/{}.stdout.log", item.id);
        let stderr = format!("logs/{}.stderr.log", item.id);
        let mut note = None;
        let before = hash(&program).ok();
        if program.is_file()
            && let Err(error) = self.remember(&program)
        {
            note = Some(error.to_string());
        }
        if let Criterion::Performance { report, .. } = &item.criterion {
            let report = PathBuf::from(self.expand(report));
            let safe = report
                .strip_prefix(self.out.join("performance"))
                .ok()
                .and_then(Path::to_str)
                .is_some_and(|s| relative(s).is_ok());
            if report.exists() || !safe {
                note = Some(
                    "performance evidence must be a new file under this report's performance/"
                        .into(),
                );
            }
        }
        let deadline = if matches!(item.criterion, Criterion::Performance { .. }) {
            21600
        } else {
            1800
        };
        let started = Instant::now();
        let mut command = Command::new(&program);
        command
            .args(&args)
            .current_dir(&cwd)
            .env_clear()
            .envs(&env)
            .stdin(Stdio::null())
            .stdout(create(&self.out.join(&stdout))?)
            .stderr(create(&self.out.join(&stderr))?);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let status = if note.is_some() {
            None
        } else {
            match command.spawn() {
                Err(error) => {
                    note = Some(format!("required command unavailable: {error}"));
                    None
                }
                Ok(mut child) => loop {
                    if let Some(status) = child.try_wait()? {
                        break Some(status);
                    }
                    if started.elapsed() > Duration::from_secs(deadline) {
                        #[cfg(unix)]
                        {
                            let _ = Command::new("/bin/kill")
                                .args(["-KILL", "--", &format!("-{}", child.id())])
                                .status();
                        }
                        let _ = child.kill();
                        note = Some(format!(
                            "required command exceeded its {deadline} second deadline"
                        ));
                        break Some(child.wait()?);
                    }
                    std::thread::sleep(Duration::from_millis(50));
                },
            }
        };
        let text = fs::read_to_string(self.out.join(&stdout)).unwrap_or_default();
        let success = status.is_some_and(|s| s.success()); // Never relabel exit 1 as success.
        let mut criterion_met =
            note.is_none() && status.and_then(|s| s.code()) == Some(item.expected_exit);
        if criterion_met {
            let evaluated = self.evaluate(&item.criterion, &text);
            if let Err(error) = evaluated {
                criterion_met = false;
                note = Some(format!("{error:#}"));
            }
        }
        if before.is_none() || hash(&program).ok() != before {
            criterion_met = false;
            note = Some(format!(
                "command executable missing or changed while running; {}",
                note.unwrap_or_default()
            ));
        }
        self.record(json!({"id":item.id,"kind":"command","required":true,"milestones":item.milestones,
            "program":program,"programSha256":before,"args":args,"cwd":cwd,"environment":env,
            "environmentPolicy":"env_clear plus exactly these entries","success":success,"criterionMet":criterion_met,
            "criterion":item.criterion,"expectedExit":item.expected_exit,"exitCode":status.and_then(|s|s.code()),
            "milliseconds":started.elapsed().as_secs_f64()*1000.0,"stdout":stdout,"stderr":stderr,
            "stdoutSha256":hash(&self.out.join(&stdout))?,"stderrSha256":hash(&self.out.join(&stderr))?,"note":note}))
    }

    fn evaluate(&mut self, criterion: &Criterion, stdout: &str) -> Result<()> {
        match criterion {
            Criterion::Exit => {}
            Criterion::Contains { text } => ensure!(
                stdout.contains(text),
                "required tool version/evidence missing: {text}"
            ),
            Criterion::RustTests => ensure!(
                complete_rust_tests(stdout),
                "required suite omitted tests, filtered tests, or has no nonempty complete result"
            ),
            Criterion::NamedRustTest { name } => {
                self.evaluate(&Criterion::RustTests, stdout)?;
                ensure!(
                    stdout
                        .lines()
                        .any(|line| line == format!("test {name} ... ok")),
                    "required native test did not execute: {name}"
                );
            }
            Criterion::WorkspaceTests => ensure!(
                stdout
                    .lines()
                    .any(|line| line.starts_with("test result: ok. ")
                        && !line.starts_with("test result: ok. 0 passed;")),
                "workspace command did not produce a nonempty test result"
            ),
            Criterion::NodeTests => ensure!(
                complete_node_tests(stdout),
                "required editor tests omitted/skipped/cancelled tests or produced no complete TAP result"
            ),
            Criterion::Json { report, assertions } => {
                let bytes = match report {
                    Some(path) => fs::read(self.expand(path))?,
                    None => stdout.as_bytes().to_vec(),
                };
                let value: Value =
                    serde_json::from_slice(&bytes).context("required JSON evidence")?;
                ensure!(
                    !has_skip(&value),
                    "skipped/missing-tool evidence cannot satisfy a required gate"
                );
                for (pointer, expected) in assertions {
                    ensure!(
                        value.pointer(pointer) == Some(expected),
                        "criterion {pointer}: expected {expected}, got {:?}",
                        value.pointer(pointer)
                    );
                }
                if let Some(path) = report {
                    self.remember(Path::new(&self.expand(path)))?;
                }
            }
            Criterion::Performance {
                report,
                assertions,
                claims,
                comparison,
            } => {
                self.evaluate(
                    &Criterion::Json {
                        report: Some(report.clone()),
                        assertions: assertions.clone(),
                    },
                    stdout,
                )?;
                let path = PathBuf::from(self.expand(report)).canonicalize()?;
                ensure!(
                    path.starts_with(self.out.join("performance")),
                    "performance report escaped its evidence directory"
                );
                let value: Value = serde_json::from_slice(&fs::read(path)?)?;
                performance_claims(&value, claims, *comparison)?;
            }
            Criterion::ToolPath => self.remember(Path::new(stdout.trim()))?,
            Criterion::AppleTool { name } => {
                let (token, selector) = match name.as_str() {
                    "swift" => ("{swift}", "SUSPECT_SWIFT_BIN"),
                    "swiftc" => ("{swiftc}", "SUSPECT_SWIFTC_BIN"),
                    "docc" => ("{docc}", "SUSPECT_SWIFT_DOCC_BIN"),
                    _ => anyhow::bail!("unknown Apple tool {name}"),
                };
                let requested = self.replacements.get(token).map(PathBuf::from);
                let path = requested
                    .filter(|path| path.components().count() > 1 && !path.starts_with("/usr/bin"))
                    .unwrap_or_else(|| PathBuf::from(stdout.trim()));
                ensure!(
                    path.is_absolute() && !path.starts_with("/usr/bin"),
                    "xcrun did not resolve an actual {name} implementation"
                );
                self.remember(&path)?;
                if name == "swiftc" {
                    self.remember(&path.with_file_name("swift-symbolgraph-extract"))
                        .context("selected Swift compiler lacks its symbol-graph companion")?;
                }
                self.replacements
                    .insert(token.into(), path.display().to_string());
                self.env.insert(selector.into(), path.display().to_string());
            }
            Criterion::CurrentSwiftSdk => {
                self.pin_current_sdk(Path::new(stdout.trim()), false)?;
            }
            Criterion::SdkSettings { .. } => {
                anyhow::bail!("SDK settings must be read as file evidence")
            }
            Criterion::TestBuild { suites } => {
                let mut found = BTreeMap::new();
                for line in stdout.lines() {
                    let Ok(value) = serde_json::from_str::<Value>(line) else {
                        continue;
                    };
                    if value["reason"] != "compiler-artifact" || value["profile"]["test"] != true {
                        continue;
                    }
                    let Some(name) = value["target"]["name"].as_str() else {
                        continue;
                    };
                    if !suites.iter().any(|s| s == name) {
                        continue;
                    }
                    if let Some(path) = value["executable"].as_str() {
                        let path = PathBuf::from(path);
                        ensure!(
                            path.starts_with(self.work.join("cargo")),
                            "test executable outside private build directory"
                        );
                        ensure!(
                            found.insert(name.to_owned(), path).is_none(),
                            "duplicate test executable {name}"
                        );
                    }
                }
                ensure!(
                    found.len() == suites.len(),
                    "some required test executables were not built"
                );
                for (suite, path) in found {
                    self.remember(&path)?;
                    self.tests.insert(suite, path);
                }
            }
        }
        Ok(())
    }

    fn current_tool_directory(&self) -> Result<PathBuf> {
        let directory = |token: &str| -> Result<PathBuf> {
            let tool = PathBuf::from(self.expand(token));
            ensure!(
                tool.is_file(),
                "selected Swift implementation is missing: {}",
                tool.display()
            );
            Ok(tool
                .parent()
                .context("Swift tool directory")?
                .canonicalize()?)
        };
        let compiler = directory("{swiftc}")?;
        ensure!(
            compiler == directory("{swift}")?,
            "current Swift driver/compiler belong to different toolchains"
        );
        Ok(compiler)
    }

    fn pin_current_sdk(&mut self, requested: &Path, explicit: bool) -> Result<Value> {
        let tools = self.current_tool_directory()?;
        let sdk = requested
            .canonicalize()
            .with_context(|| format!("resolve selected current SDK {}", requested.display()))?;
        ensure!(sdk.is_dir(), "current SDK must be a directory");
        if !explicit {
            let developer = developer_directory(&tools)?.canonicalize()?;
            ensure!(
                sdk.starts_with(&developer),
                "discovered SDK {} does not belong to the selected Swift developer directory {}",
                sdk.display(),
                developer.display()
            );
        }
        let settings = sdk.join("SDKSettings.json");
        let metadata = sdk_settings(
            &fs::read(&settings).with_context(|| format!("read {}", settings.display()))?,
        )?;
        ensure!(
            metadata["CanonicalName"]
                .as_str()
                .is_some_and(|name| name.starts_with("macosx")),
            "current Swift probes require a macOS SDK"
        );
        self.remember(&settings)?;
        self.replacements
            .insert("{swift-sdk}".into(), sdk.display().to_string());
        self.env
            .insert("SUSPECT_SWIFT_SDKROOT".into(), sdk.display().to_string());
        Ok(
            json!({"sdkRoot":sdk,"version":metadata["Version"],"toolDirectory":tools,"explicitOverride":explicit,"settingsSha256":hash(&settings)?}),
        )
    }
}

fn developer_directory(tools: &Path) -> Result<PathBuf> {
    tools.ancestors().find(|path| path.file_name().is_some_and(|name| name == "Developer" || name == "CommandLineTools"))
        .map(Path::to_owned).context("selected Swift toolchain has no developer directory; provide an explicit SUSPECT_SWIFT_SDKROOT for standalone toolchains")
}

fn sdk_settings(bytes: &[u8]) -> Result<Value> {
    let metadata: Value = serde_json::from_slice(bytes).context("invalid SDKSettings.json")?;
    for field in ["Version", "CanonicalName", "DisplayName"] {
        ensure!(
            metadata[field]
                .as_str()
                .is_some_and(|value| !value.is_empty()),
            "SDKSettings.json lacks {field}"
        );
    }
    Ok(metadata)
}

fn performance_claims(
    value: &Value,
    claims: &BTreeMap<String, String>,
    comparison: bool,
) -> Result<()> {
    let get = |key: &str| -> Result<&Value> {
        value
            .pointer(
                claims
                    .get(key)
                    .with_context(|| format!("missing performance claim {key}"))?,
            )
            .with_context(|| format!("missing performance evidence for {key}"))
    };
    ensure!(
        get("complete")? == &json!(true),
        "performance collection/comparison is incomplete"
    );
    if comparison {
        ensure!(
            get("gated")? == &json!(true) && get("verdict")? == &json!("passed"),
            "observational, inconclusive or unqualified comparisons cannot complete M6"
        );
        ensure!(
            get("regressions")? == &json!(0) || get("regressions")? == &json!([]),
            "performance regressions remain"
        );
        ensure!(
            get("comparedCases")?.as_u64().is_some_and(|n| n >= 3),
            "comparator must cover all three fixture classes"
        );
    } else {
        ensure!(
            get("measurementStatus")? == &json!("observational"),
            "untimed or failed runs are not measurement evidence"
        );
        for key in ["warmCompiles", "warmRenders", "warmWrites"] {
            ensure!(
                get(key)? == &json!(0),
                "unchanged accepted snapshots must have zero {key}"
            );
        }
        for key in [
            "coldSamples",
            "warmSamples",
            "sourceChangeSamples",
            "configChangeSamples",
        ] {
            ensure!(
                get(key)?.as_u64().is_some_and(|n| n > 0),
                "required performance scenario has no samples: {key}"
            );
        }
    }
    Ok(())
}

pub(super) fn complete_rust_tests(text: &str) -> bool {
    let rows = text
        .lines()
        .filter_map(|line| line.strip_prefix("test result: ok. "))
        .collect::<Vec<_>>();
    rows.len() == 1
        && ["passed", "failed", "ignored", "filtered out"]
            .iter()
            .all(|label| {
                let count = rows[0].split(';').find_map(|part| {
                    part.trim()
                        .strip_suffix(label)?
                        .trim()
                        .parse::<usize>()
                        .ok()
                });
                if *label == "passed" {
                    count.is_some_and(|n| n > 0)
                } else {
                    count == Some(0)
                }
            })
}

fn complete_node_tests(text: &str) -> bool {
    let count = |label: &str| {
        text.lines()
            .filter_map(|line| {
                line.strip_prefix(&format!("# {label} "))?
                    .parse::<usize>()
                    .ok()
            })
            .next_back()
    };
    count("tests").is_some_and(|n| n > 0)
        && count("tests") == count("pass")
        && ["fail", "cancelled", "skipped", "todo"]
            .iter()
            .all(|label| count(label) == Some(0))
}

fn has_skip(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, value)| {
            (["skipped", "skip", "missingTools", "missing_tools"].contains(&key.as_str())
                && !matches!(value, Value::Bool(false) | Value::Null)
                && value != &json!(0)
                && value != &json!([]))
                || has_skip(value)
        }),
        Value::Array(values) => values.iter().any(has_skip),
        Value::String(value) => {
            ["skipped", "skip", "missing-tool", "unavailable"].contains(&value.as_str())
        }
        _ => false,
    }
}

pub(super) fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).into()).collect()
}
pub(super) fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub(super) fn digest_text(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(super) fn hash(path: &Path) -> Result<String> {
    let mut reader = fs::File::open(path).with_context(|| format!("hash {}", path.display()))?;
    let mut state = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        state.update(&buffer[..count]);
    }
    Ok(format!("{:x}", state.finalize()))
}
pub(super) fn create(path: &Path) -> Result<fs::File> {
    fs::create_dir_all(path.parent().context("file parent")?)?;
    Ok(fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?)
}
pub(super) fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    create(path)?.write_all(&serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
pub(super) fn resolve(name: &str, env: &BTreeMap<String, String>) -> PathBuf {
    if Path::new(name).components().count() > 1 {
        return name.into();
    }
    std::env::split_paths(env.get("PATH").map(String::as_str).unwrap_or_default())
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
        .unwrap_or_else(|| name.into())
}
pub(super) fn relative(name: &str) -> Result<&Path> {
    let path = Path::new(name);
    ensure!(
        !name.is_empty() && path.components().all(|c| matches!(c, Component::Normal(_))),
        "nonportable relative evidence path {name:?}"
    );
    Ok(path)
}

fn file_record(root: &Path, name: &str) -> Result<FileRecord> {
    let path = root.join(relative(name)?);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileRecord {
                kind: "deleted".into(),
                sha256: sha(b""),
                bytes: 0,
                mode: 0,
                link: None,
            });
        }
        Err(error) => return Err(error.into()),
    };
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o777
    };
    #[cfg(not(unix))]
    let mode = u32::from(metadata.permissions().readonly());
    let (kind, link, bytes, digest) = if metadata.file_type().is_symlink() {
        let link = fs::read_link(&path)?;
        ensure!(
            !link.is_absolute() && path.canonicalize()?.starts_with(root),
            "source link escapes relocatable snapshot: {name}"
        );
        let text = link.to_str().context("non-UTF8 source link")?.to_owned();
        (
            "symlink",
            Some(text.clone()),
            text.len() as u64,
            sha(text.as_bytes()),
        )
    } else {
        ensure!(
            metadata.is_file(),
            "source input is not a regular file: {name}"
        );
        ("file", None, metadata.len(), hash(&path)?)
    };
    Ok(FileRecord {
        kind: kind.into(),
        sha256: digest,
        bytes,
        mode,
        link,
    })
}

pub(super) fn inventory(root: &Path, names: impl Iterator<Item = String>) -> Result<Inventory> {
    names
        .map(|name| Ok((name.clone(), file_record(root, &name)?)))
        .collect()
}

pub(super) fn copy_inventory(from: &Path, to: &Path, files: &Inventory) -> Result<()> {
    fs::create_dir_all(to)?;
    for (name, record) in files {
        if record.kind == "deleted" {
            continue;
        }
        let target = to.join(relative(name)?);
        fs::create_dir_all(target.parent().context("snapshot parent")?)?;
        if let Some(link) = &record.link {
            #[cfg(unix)]
            std::os::unix::fs::symlink(link, &target)?;
            #[cfg(not(unix))]
            anyhow::bail!("source symlink snapshots require Unix");
        } else {
            ensure!(!target.exists(), "snapshot destination exists");
            fs::copy(from.join(name), &target)?;
        }
    }
    ensure!(
        &inventory(to, files.keys().cloned())? == files,
        "source changed while taking the snapshot"
    );
    Ok(())
}

pub(super) fn tree_names(root: &Path) -> Result<Vec<String>> {
    fn walk(root: &Path, directory: &Path, names: &mut Vec<String>) -> Result<()> {
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if fs::symlink_metadata(&path)?.is_dir() {
                walk(root, &path, names)?;
            } else {
                names.push(
                    path.strip_prefix(root)?
                        .to_str()
                        .context("UTF-8 file path")?
                        .to_owned(),
                );
            }
        }
        Ok(())
    }
    let mut names = Vec::new();
    walk(root, root, &mut names)?;
    names.sort();
    Ok(names)
}

fn git(run: &mut Run, id: &str, cwd: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let item = stage(id, BOTH, "git", args, &cwd.display().to_string());
    run.command(&item)?;
    ensure!(
        run.checks.last().context("git check")?["criterionMet"] == true,
        "required Git provenance failed: {id}"
    );
    Ok(fs::read(run.out.join(format!("logs/{id}.stdout.log")))?)
}

pub(super) fn source_inventory(run: &mut Run, suffix: &str) -> Result<Inventory> {
    let original = run.original.clone();
    let bytes = git(
        run,
        &format!("source-files-{suffix}"),
        &original,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    )?;
    let names = bytes
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| Ok(std::str::from_utf8(s)?.to_owned()))
        .collect::<Result<BTreeSet<_>>>()?;
    ensure!(!names.is_empty(), "source census is empty");
    inventory(&original, names.into_iter())
}

pub(super) fn prepare(run: &mut Run) -> Result<Inventory> {
    let original = run.original.clone();
    let head = git(run, "source-head", &original, &["rev-parse", "HEAD"])?;
    git(
        run,
        "source-status",
        &original,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    git(
        run,
        "source-index",
        &original,
        &["ls-files", "--stage", "-z"],
    )?;
    git(
        run,
        "source-unstaged-patch",
        &original,
        &["diff", "--binary", "--no-ext-diff"],
    )?;
    git(
        run,
        "source-staged-patch",
        &original,
        &["diff", "--cached", "--binary", "--no-ext-diff"],
    )?;
    let files = source_inventory(run, "before")?;
    ensure!(
        files
            .get("xtask/src/sdk_m3_m6.rs")
            .is_some_and(|file| file.sha256 == sha(include_bytes!("sdk_m3_m6.rs"))),
        "running verifier does not match its current source; rebuild xtask"
    );
    copy_inventory(&original, &run.out.join("source"), &files)?;
    copy_inventory(&run.out.join("source"), &run.work.join("source"), &files)?;
    write_json(&run.out.join("source-manifest.json"), &files)?;
    let fingerprint = sha(&serde_json::to_vec(&files)?);
    run.env
        .insert("SUSPECT_M3_M6_SOURCE_SHA256".into(), fingerprint.clone());
    run.provenance = json!({"repository":original,"head":String::from_utf8(head)?.trim(),"sourceFingerprint":fingerprint,
        "snapshot":"source/","manifest":"source-manifest.json","manifestSha256":hash(&run.out.join("source-manifest.json"))?,
        "selection":"git ls-files --cached --others --exclude-standard -z; includes actual modified, staged, untracked and deleted paths",
        "executionRoot":run.work.join("source"),"checkoutOfHead":false});
    run.internal("source-snapshot", Ok(run.provenance.clone()))?;

    let source = run.source.clone();
    let revision = git(run, "input-head", &source, &["rev-parse", "HEAD"])?;
    let mut inputs = Vec::new();
    for (id, name, expected) in INPUTS {
        git(
            run,
            &format!("input-tracked-{id}"),
            &source,
            &["ls-files", "--error-unmatch", "--", name],
        )?;
        ensure!(
            hash(&source.join(name))? == *expected,
            "tracked OpenRouter bytes differ: {name}"
        );
        let snapshot = run.out.join("inputs").join(name);
        create(&snapshot)?.write_all(&fs::read(source.join(name))?)?;
        run.remember(&source.join(name))?;
        run.remember(&snapshot)?;
        inputs.push(json!({"id":id,"sourcePath":name,"snapshot":format!("inputs/{name}"),"sha256":expected,"tracked":true}));
    }
    write_json(
        &run.out.join("inputs.json"),
        &json!({"root":source,"revision":String::from_utf8(revision)?.trim(),"inputs":inputs}),
    )?;
    run.internal("tracked-inputs", Ok(json!(inputs)))?;
    Ok(files)
}

pub(super) fn env_values(run: &mut Run) -> Result<()> {
    for name in [
        "HOME",
        "PATH",
        "CARGO_HOME",
        "RUSTUP_HOME",
        "UV_CACHE_DIR",
        "npm_config_cache",
        "DEVELOPER_DIR",
        "SDKROOT",
        "MACOSX_DEPLOYMENT_TARGET",
        "CC",
        "CXX",
        "AR",
        "CFLAGS",
        "CXXFLAGS",
    ] {
        if let Ok(value) = std::env::var(name) {
            run.env.insert(name.into(), value);
        }
    }
    let selected =
        |name: &str, default: &str| std::env::var(name).unwrap_or_else(|_| default.into());
    for (token, name, default) in [
        (
            "{python-floor}",
            "SUSPECT_PYTHON_FLOOR_BIN",
            "/Users/luke/.local/share/uv/python/cpython-3.11-macos-aarch64-none/bin/python3.11",
        ),
        (
            "{python-current}",
            "SUSPECT_PYTHON_CURRENT_BIN",
            "/opt/homebrew/opt/python@3.14/bin/python3.14",
        ),
        (
            "{node22}",
            "SUSPECT_DOCS_NODE",
            "/Users/luke/.local/share/mise/installs/node/22.23.1/bin/node",
        ),
        (
            "{node24}",
            "SUSPECT_NODE24_BIN",
            "/Users/luke/.local/share/mise/installs/node/24.21.0/bin/node",
        ),
        (
            "{python-tools}",
            "SUSPECT_PYTHON_TOOLS",
            &run.original
                .join("target/sdk-native-python-tools/bin/python")
                .display()
                .to_string(),
        ),
        ("{swift}", "SUSPECT_SWIFT_BIN", "/usr/bin/swift"),
        ("{docc}", "SUSPECT_SWIFT_DOCC_BIN", "/usr/bin/docc"),
        (
            "{swiftc}",
            "SUSPECT_SWIFTC_BIN",
            &PathBuf::from(selected("SUSPECT_SWIFT_BIN", "/usr/bin/swift"))
                .with_file_name("swiftc")
                .display()
                .to_string(),
        ),
        (
            "{chromium}",
            "SUSPECT_CHROMIUM",
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        ),
    ] {
        run.replacements
            .insert(token.into(), selected(name, default));
    }
    if let Ok(sdk) = std::env::var("SUSPECT_SWIFT_SDKROOT") {
        run.replacements.insert("{swift-sdk-requested}".into(), sdk);
    }
    // Resolve from the caller's TMPDIR, before child commands receive private tmp paths.
    let floor = std::env::var_os("SUSPECT_SWIFT_FLOOR_ROOT").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("opencode/swift-6.0.3-toolchain/expanded/swift-6.0.3-RELEASE-osx-package.pkg/Payload"));
    let driver = std::env::var_os("SUSPECT_SWIFT_FLOOR_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| floor.join("usr/bin/swift"));
    for (token, value) in [
        ("{swift-floor-root}", floor),
        ("{swift-floor}", driver.clone()),
        (
            "{swiftc-floor}",
            std::env::var_os("SUSPECT_SWIFTC_FLOOR_BIN")
                .map(PathBuf::from)
                .unwrap_or_else(|| driver.with_file_name("swiftc")),
        ),
        (
            "{docc-floor}",
            std::env::var_os("SUSPECT_SWIFT_FLOOR_DOCC_BIN")
                .map(PathBuf::from)
                .unwrap_or_else(|| driver.with_file_name("docc")),
        ),
        (
            "{swift-floor-sdk}",
            std::env::var_os("SUSPECT_SWIFT_FLOOR_SDKROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    "/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk".into()
                }),
        ),
    ] {
        run.replacements
            .insert(token.into(), value.display().to_string());
    }
    for (key, value) in [
        ("RUSTUP_TOOLCHAIN", "stable"),
        ("SUSPECT_NATIVE_RUST_TOOLCHAIN", "stable"),
        ("GOTOOLCHAIN", "local"),
        ("SUSPECT_GO_TOOLCHAIN", "local"),
        ("GOWORK", "off"),
        ("OPENROUTER_WEB_ROOT", "{out}/inputs"),
        (
            "SUSPECT_OPENROUTER_YAML",
            "{out}/inputs/projects/docs/openapi/openapi.yaml",
        ),
        (
            "SUSPECT_OPENROUTER_SPEC",
            "{out}/inputs/projects/docs/openapi/openapi.yaml",
        ),
        ("SUSPECT_PYTHON_BIN", "{python-current}"),
        ("SUSPECT_PYTHON_TOOLS", "{python-tools}"),
        ("SUSPECT_PYTHON_FLOOR_BIN", "{python-floor}"),
        ("SUSPECT_PYTHON_CURRENT_BIN", "{python-current}"),
        ("SUSPECT_DOCS_NODE", "{node22}"),
        ("SUSPECT_PACKAGE_NODE", "{node22}"),
        ("SUSPECT_NODE24_BIN", "{node24}"),
        ("SUSPECT_SWIFT_BIN", "{swift}"),
        ("SUSPECT_SWIFTC_BIN", "{swiftc}"),
        ("SUSPECT_SWIFT_DOCC_BIN", "{docc}"),
        ("SUSPECT_SWIFT_GATE_ROOT", "{work}/swift/current/gates"),
        ("SUSPECT_CHROMIUM", "{chromium}"),
        ("SUSPECT_TEST_BINARY", "{out}/bin/suspect"),
        ("SUSPECT_COMPARE_BIN", "{out}/bin/suspect"),
        ("CARGO_TARGET_DIR", "{work}/cargo"),
        ("CARGO_TERM_COLOR", "never"),
        ("TMPDIR", "{work}/tmp"),
        ("SUSPECT_TEST_TMPDIR", "{work}/tmp"),
        ("PYTHONDONTWRITEBYTECODE", "1"),
        ("LC_ALL", "C"),
        ("LANG", "C"),
        ("GIT_OPTIONAL_LOCKS", "0"),
    ] {
        run.env.insert(key.into(), run.expand(value));
    }
    let node = PathBuf::from(run.expand("{node22}"));
    let paths = [
        node.parent().context("Node bin directory")?.to_owned(),
        run.work
            .join("source/crates/suspect-codegen/tools/typescript-docs/node_modules/.bin"),
    ];
    let path = std::env::join_paths(
        paths
            .into_iter()
            .chain(std::env::split_paths(run.env.get("PATH").context("PATH")?)),
    )?;
    run.env.insert(
        "PATH".into(),
        path.into_string()
            .map_err(|_| anyhow::anyhow!("PATH must be UTF-8"))?,
    );
    for name in ["tmp", "source/target", "packages", "editor"] {
        fs::create_dir_all(run.work.join(name))?;
    }
    for tier in ["floor", "current"] {
        for name in ["tmp", "gates", "module-cache", "package-cache"] {
            fs::create_dir_all(run.work.join("swift").join(tier).join(name))?;
        }
    }
    Ok(())
}

#[rustfmt::skip]
pub(super) fn swift_environment(tier: &str) -> BTreeMap<String, String> {
    let floor = tier == "floor";
    let compiler = if floor { "{swiftc-floor}" } else { "{swiftc}" };
    let mut env = BTreeMap::from([
        ("SUSPECT_SWIFT_BIN".into(), if floor { "{swift-floor}" } else { "{swift}" }.into()),
        ("SUSPECT_SWIFTC_BIN".into(), compiler.into()), ("SWIFT_EXEC".into(), compiler.into()),
        ("SUSPECT_SWIFT_GATE_ROOT".into(), format!("{{work}}/swift/{tier}/gates")),
        ("TMPDIR".into(), format!("{{work}}/swift/{tier}/tmp")),
        ("SWIFTPM_MODULECACHE_OVERRIDE".into(), format!("{{work}}/swift/{tier}/module-cache")),
        ("CLANG_MODULE_CACHE_PATH".into(), format!("{{work}}/swift/{tier}/module-cache")),
    ]);
    let sdk = if floor { "{swift-floor-sdk}" } else { "{swift-sdk}" };
    env.insert("SUSPECT_SWIFT_SDKROOT".into(), sdk.into());
    env.insert("SDKROOT".into(), sdk.into());
    env.insert("SUSPECT_SWIFT_DOCC_BIN".into(), if floor { "{docc-floor}" } else { "{docc}" }.into());
    env
}

// Keep command matrices compact and reviewable beside the exit document.
#[rustfmt::skip]
pub(super) fn suite_stages() -> Vec<Stage> {
    let mut stages = Vec::new();
    let mut add = |prefix: &str, milestones: &[&str], suites: &[&str], environment: BTreeMap<String, String>| {
        for suite in suites {
            let mut item = stage(&format!("{prefix}-{suite}"), milestones, &format!("test:{suite}"), &["--include-ignored", "--nocapture", "--test-threads=1"], "{workspace}");
            item.criterion = Criterion::RustTests;
            item.environment = environment.clone();
            stages.push(item);
        }
    };
    add("library", BOTH, &["generation_session", "sdk_compatibility", "artifact_safety", "http_admission", "sdk_examples", "sdk_workflow_contract"], BTreeMap::new());
    add("cli", BOTH, &["codegen_session", "codegen_compare", "sdk", "rust_sdk", "generation_ownership", "entry_loading"], BTreeMap::new());
    for (tier, python, go, rust) in [("floor", "{python-floor}", "go1.23.12", "1.88.0"), ("current", "{python-current}", "local", "stable")] {
        let env = BTreeMap::from([("SUSPECT_PYTHON_BIN".into(), python.into()), ("SUSPECT_GO_TOOLCHAIN".into(), go.into()), ("GOTOOLCHAIN".into(), go.into())]);
        add(tier, M3, &["python_json", "python_validation", "python_models", "python_codecs", "python_http", "python_runtime_regressions", "go_json", "go_validation", "go_models", "go_codecs", "go_http", "go_http_names", "go_runtime_regressions", "wave_a_openrouter", "m3_native_docs"], env);
        // Execute already-built harnesses directly: *all* child Cargo invocations
        // inherit this toolchain, including older helpers without a selector.
        let env = BTreeMap::from([("RUSTUP_TOOLCHAIN".into(), rust.into()), ("SUSPECT_NATIVE_RUST_TOOLCHAIN".into(), rust.into())]);
        add(tier, M3, &["rust_json", "rust_validation", "rust_contract", "rust_codecs", "rust_codecs_json", "rust_http", "rust_http_runtime", "rust_http_openrouter", "rust_http_security", "rust_http_docs", "rust_serde", "m2_vertical", "sdk_example_packages"], env);
    }
    add("names", M3, &["python_http_names"], BTreeMap::new());
    add("native", M3, &["typescript_contract", "typescript_http", "typescript_http_runtime", "typescript_http_docs", "typescript_http_bundle", "typescript_package", "typescript_m2_toolchains", "typescript_directional_package", "typescript_http_directional", "typescript_codecs_docs", "m2_browser"], BTreeMap::new());
    for tier in ["floor", "current"] { add(tier, M3, &["swift_sdk"], swift_environment(tier)); }
    for node in ["node22", "node24"] {
        add(node, M3, &["typescript_json", "typescript_validation", "typescript_codecs_runtime"], BTreeMap::from([("PATH".into(), format!("{{{node}-bin}}:{{path}}"))]));
    }
    for tier in ["floor", "current"] {
        let mut item = stage(&format!("{tier}-swift-shared-runtime"), M3, &format!("test:{CODEGEN_LIB}"), &["--include-ignored", "--show-output", "--test-threads=1"], "{workspace}");
        item.environment = swift_environment(tier);
        item.criterion = Criterion::NamedRustTest { name: SWIFT_VECTORS.into() };
        stages.push(item);
    }
    stages
}

fn test_build(package: &str, suites: &[String]) -> Stage {
    let mut item = stage(
        &format!("build-tests-{package}"),
        BOTH,
        "cargo",
        &[
            "test",
            "--locked",
            "--no-run",
            "--message-format=json",
            "-p",
            package,
        ],
        "{workspace}",
    );
    for suite in suites {
        if suite == CODEGEN_LIB {
            item.args.push("--lib".into());
        } else {
            item.args.extend(["--test".into(), suite.clone()]);
        }
    }
    item.criterion = Criterion::TestBuild {
        suites: suites.into(),
    };
    item.environment
        .insert("RUSTUP_TOOLCHAIN".into(), "stable".into());
    item
}

#[rustfmt::skip]
pub(super) fn tool_stages() -> Vec<Stage> {
    let mut stages = Vec::new();
    // Resolve before any current Swift command or SWIFT_EXEC consumer runs.
    for name in ["docc", "swift", "swiftc"] {
        let mut item = stage(&format!("tool-{name}-path"), M3, "xcrun", &["--find", name], "{workspace}");
        item.criterion = Criterion::AppleTool { name: name.into() };
        stages.push(item);
    }
    let mut current_sdk = stage("tool-swift-current-sdk-path", M3, "xcrun", &["--sdk", "macosx", "--show-sdk-path"], "{workspace}");
    current_sdk.criterion = Criterion::CurrentSwiftSdk;
    stages.push(current_sdk);
    for (id, program, args, wanted) in [
        ("tool-node22", "{node22}", &["--version"][..], "v22.23.1"),
        ("tool-node24", "{node24}", &["--version"][..], "v24.21.0"),
        ("tool-python-floor", "{python-floor}", &["--version"][..], "Python 3.11."),
        ("tool-python-current", "{python-current}", &["--version"][..], "Python 3.14."),
        ("tool-npm", "npm", &["--version"][..], "10.9.8"),
        ("tool-uv", "uv", &["--version"][..], "uv "),
        ("tool-swift", "{swift}", &["--version"][..], "Swift version 6.3.3 ("),
        ("tool-swiftc", "{swiftc}", &["--version"][..], "Swift version 6.3.3 ("),
        ("tool-swift-floor", "{swift-floor}", &["--version"][..], "Swift version 6.0.3 ("),
        ("tool-swiftc-floor", "{swiftc-floor}", &["--version"][..], "Swift version 6.0.3 ("),
        ("tool-docc-floor", "{docc-floor}", &["--help"][..], "convert"),
        ("tool-docc", "{docc}", &["--help"][..], "convert"),
        ("tool-chromium", "{chromium}", &["--version"][..], ""),
        ("tool-clippy", "cargo", &["clippy", "--version"][..], "clippy"),
        ("tool-rustfmt", "rustfmt", &["--version"][..], "rustfmt"),
    ] {
        let mut item = stage(id, M3, program, args, "{workspace}");
        if !wanted.is_empty() { item.criterion = Criterion::Contains { text: wanted.into() }; }
        stages.push(item);
    }
    for (tier, root) in [("floor", "{swift-floor-sdk}"), ("current", "{swift-sdk}")] {
        let mut sdk = stage(&format!("tool-swift-{tier}-sdk"), M3, "[read SDKSettings.json]", &[], "{workspace}");
        sdk.criterion = Criterion::SdkSettings { root: root.into() };
        stages.push(sdk);
    }
    for (tier, toolchain) in [("floor", "go1.23.12"), ("current", "local")] {
        for (name, args) in [("version", &["version"][..]), ("config", &["env", "-json"][..])] {
            let mut item = stage(&format!("tool-go-{tier}-{name}"), M3, "go", args, "{workspace}");
            item.environment.insert("GOTOOLCHAIN".into(), toolchain.into());
            if tier == "floor" && name == "version" { item.criterion = Criterion::Contains { text: "go1.23.12".into() }; }
            stages.push(item);
        }
    }
    for (tier, toolchain) in [("floor", "1.88.0"), ("current", "stable")] {
        for (name, args) in [("rustc", &["-Vv"][..]), ("cargo", &["-Vv"][..]), ("rustdoc", &["-Vv"][..])] {
            let mut item = stage(&format!("tool-{name}-{tier}"), M3, name, args, "{workspace}");
            item.environment.insert("RUSTUP_TOOLCHAIN".into(), toolchain.into());
            if tier == "floor" { item.criterion = Criterion::Contains { text: format!("{name} 1.88.0") }; }
            stages.push(item);
            let mut path = stage(&format!("tool-{name}-{tier}-path"), M3, "rustup", &["which", "--toolchain", toolchain, name], "{workspace}");
            path.criterion = Criterion::ToolPath;
            stages.push(path);
        }
    }
    stages.push(stage("tool-python-packages", M3, "{python-tools}", &["-c", "import importlib.metadata as m,json,sys; p={n:m.version(n) for n in ['build','hatchling','httpx','mypy','sphinx']}; print(json.dumps({'python':sys.version,'executable':sys.executable,'prefix':sys.prefix,'packages':p},sort_keys=True)); assert p=={'build':'1.4.0','hatchling':'1.29.0','httpx':'0.28.1','mypy':'1.19.1','sphinx':'8.2.3'}"], "{workspace}"));
    for (name, version) in [("typescript-docs", "5.9.3"), ("typescript-floor", "5.5.4"), ("typescript-http-bundle", "0.28.2")] {
        let cwd = format!("{{workspace}}/crates/suspect-codegen/tools/{name}");
        stages.push(stage(&format!("install-{name}"), M3, "npm", &["ci", "--offline", "--ignore-scripts", "--no-audit", "--no-fund"], &cwd));
        let script = if name == "typescript-http-bundle" { "node_modules/esbuild/bin/esbuild" } else { "node_modules/typescript/bin/tsc" };
        let mut item = stage(&format!("tool-{name}"), M3, "{node22}", &[script, "--version"], &cwd);
        item.criterion = Criterion::Contains { text: version.into() };
        stages.push(item);
    }
    stages
}

fn unmet_performance(run: &mut Run, id: &str, reason: &str) -> Result<()> {
    run.record(
        json!({"id":id,"kind":"unmet","state":"unmet","required":true,"milestones":M6,
        "success":false,"criterionMet":false,"exitCode":null,"note":reason}),
    )
}

fn load_performance(
    run: &mut Run,
    path: Option<&Path>,
    required: &mut Vec<Stage>,
) -> Result<Vec<Stage>> {
    let before = run.files.clone();
    let result = path
        .context("no --performance-plan; measured and qualified M6 comparison evidence is required")
        .and_then(|path| performance_stages(run, path));
    match result {
        Ok(stages) => {
            for item in &stages {
                if let Some(slot) = required.iter_mut().find(|s| s.id == item.id) {
                    *slot = item.clone();
                } else {
                    required.push(item.clone());
                }
            }
            Ok(stages)
        }
        Err(error) => {
            // An unusable optional plan must not add unrelated integrity prerequisites to M3.
            run.files = before;
            let reason = format!("performance plan {path:?} is unusable: {error:#}");
            for id in PERFORMANCE {
                unmet_performance(run, id, &reason)?;
            }
            Ok(Vec::new())
        }
    }
}

fn performance_stages(run: &mut Run, path: &Path) -> Result<Vec<Stage>> {
    let bytes = fs::read(path).context("Main must supply the integrated performance plan")?;
    create(&run.out.join("performance-plan.json"))?.write_all(&bytes)?;
    let plan: PerformancePlan = serde_json::from_slice(&bytes)?;
    ensure!(
        plan.format == "suspect.sdk.m3-m6.performance-plan.v1",
        "unsupported performance-plan format"
    );
    let ids = plan
        .stages
        .iter()
        .map(|s| s.id.as_str())
        .collect::<BTreeSet<_>>();
    ensure!(
        plan.stages.len() == PERFORMANCE.len() && ids == PERFORMANCE.iter().copied().collect(),
        "all four performance stages are mandatory"
    );
    ensure!(
        !plan.inputs.is_empty(),
        "pin the performance comparator's baseline/config inputs"
    );
    for (index, input) in plan.inputs.iter().enumerate() {
        ensure!(
            input.path.is_absolute()
                && digest_text(&input.sha256)
                && hash(&input.path)? == input.sha256,
            "performance baseline/config digest mismatch"
        );
        create(&run.out.join(format!("links/performance-input-{index}")))?
            .write_all(&fs::read(&input.path)?)?;
        run.remember(&input.path)?;
    }
    run.remember(path)?;
    let mut stages = Vec::new();
    // Run in the declared order so collection precedes comparison.
    ensure!(
        plan.stages
            .last()
            .is_some_and(|s| s.id == "performance-compare"),
        "performance comparison must follow collection"
    );
    for item in plan.stages {
        ensure!(
            item.assertions.len() >= 2
                && item.assertions.contains_key("/format")
                && item.assertions.keys().all(|k| k.starts_with('/')),
            "performance criteria need an exact format and nonempty semantic assertions"
        );
        let comparison = item.id == "performance-compare";
        let claim_keys = if comparison {
            &[
                "complete",
                "regressions",
                "comparedCases",
                "gated",
                "verdict",
            ][..]
        } else {
            &[
                "complete",
                "measurementStatus",
                "warmCompiles",
                "warmRenders",
                "warmWrites",
                "coldSamples",
                "warmSamples",
                "sourceChangeSamples",
                "configChangeSamples",
            ][..]
        };
        ensure!(
            claim_keys
                .iter()
                .all(|key| item.claims.get(*key).is_some_and(|p| p.starts_with('/'))),
            "performance stage {} lacks required semantic claim pointers",
            item.id
        );
        ensure!(
            item.cwd == "{workspace}",
            "performance commands must run against the snapshotted implementation"
        );
        let mut step = stage(&item.id, M6, &item.program, &[], &item.cwd);
        step.args = item.args;
        step.environment = item.environment;
        step.criterion = Criterion::Performance {
            report: item.report,
            assertions: item.assertions,
            claims: item.claims,
            comparison,
        };
        stages.push(step);
    }
    Ok(stages)
}

#[rustfmt::skip]
pub(super) fn quality_stages() -> Vec<Stage> {
    let mut stages = vec![
        stage("workspace-tests", BOTH, "cargo", &["test", "--workspace", "--locked"], "{workspace}"),
        stage("workspace-clippy", BOTH, "cargo", &["clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"], "{workspace}"),
        stage("workspace-rustdoc", BOTH, "cargo", &["doc", "--workspace", "--no-deps", "--locked"], "{workspace}"),
        stage("workspace-format", BOTH, "cargo", &["fmt", "--all", "--", "--check"], "{workspace}"),
        stage("patch-whitespace", BOTH, "git", &["diff", "--check"], "{original}"),
        stage("staged-whitespace", BOTH, "git", &["diff", "--cached", "--check"], "{original}"),
    ];
    stages[0].criterion = Criterion::WorkspaceTests;
    stages[2].environment.insert("RUSTDOCFLAGS".into(), "-D warnings".into());
    stages
}

#[rustfmt::skip]
pub(super) fn editor_stages() -> Vec<Stage> {
    let mut stages = vec![
        stage("editor-install", M6, "npm", &["ci", "--offline", "--ignore-scripts", "--no-audit", "--no-fund"], "{work}/editor"),
        stage("editor-compile", M6, "npm", &["run", "compile"], "{work}/editor"),
        stage("editor-real-cli-and-protocol", M6, "{node22}", &["--test", "--test-reporter=tap", "test/generation.cjs", "test/generation-native-profiles.cjs", "test/generation-native-session.cjs", "test/generation-session.cjs", "test/generation-ui.cjs"], "{work}/editor"),
    ];
    stages[2].criterion = Criterion::NodeTests;
    stages
}

#[rustfmt::skip]
pub(super) fn package_stages() -> Vec<Stage> {
    let mut steps = Vec::new();
    for (id, flag, status, success, code) in [("preview", "--preview", "drift", false, 1), ("generate", "", "written", true, 0), ("drift", "--check", "current", true, 0)] {
        let mut item = stage(&format!("five-op-{id}"), BOTH, "{out}/bin/suspect", &["codegen-session", "--config", "{out}/configs/five-operations.json", "--out", "{work}/packages", "--format", "json"], "{workspace}");
        if !flag.is_empty() { item.args.push(flag.into()); }
        item.expected_exit = code;
        item.criterion = Criterion::Json { report: None, assertions: BTreeMap::from([("/format".into(), json!("suspect.sdk.session.v1")), ("/status".into(), json!(status)), ("/success".into(), json!(success))]) };
        steps.push(item);
    }
    steps.push(stage("package-python-wheel", M3, "{python-tools}", &["-m", "build", "--wheel", "--no-isolation", "--outdir", "{work}/wheels"], "{work}/packages/python"));
    steps.push(stage("package-python-types", M3, "{python-tools}", &["-m", "mypy", "--strict", "--python-version", "3.11", "--cache-dir", "{work}/package-mypy", "src/m3_m6_sdk"], "{work}/packages/python"));
    for (tier, python, go, rust) in [("floor", "{python-floor}", "go1.23.12", "1.88.0"), ("current", "{python-current}", "local", "stable")] {
        let venv = format!("{{work}}/python-{tier}");
        let interpreter = format!("{venv}/bin/python");
        steps.push(stage(&format!("package-python-{tier}-venv"), M3, "uv", &["venv", "--python", python, &venv], "{work}"));
        steps.push(stage(&format!("package-python-{tier}-install"), M3, "uv", &["pip", "install", "--offline", "--python", &interpreter, "{work}/wheels/m3_m6_sdk-0.0.0-py3-none-any.whl"], "{work}"));
        steps.push(stage(&format!("package-python-{tier}-dependencies"), M3, "uv", &["pip", "check", "--python", &interpreter], "{work}"));
        steps.push(stage(&format!("package-python-{tier}-inventory"), M3, "uv", &["pip", "list", "--format", "json", "--python", &interpreter], "{work}"));
        steps.push(stage(&format!("package-python-{tier}-import"), M3, &interpreter, &["-c", "import sys,importlib.metadata as m; from m3_m6_sdk import Client,AsyncClient; print(sys.version); print(m.version('m3-m6-sdk')); assert m.version('m3-m6-sdk')=='0.0.0'"], "{work}"));
        for (name, args) in [("build", &["build", "./..."][..]), ("vet", &["vet", "./..."][..]), ("docs", &["doc", "-all"][..]), ("config", &["list", "-m", "-json", "all"][..])] {
            let mut item = stage(&format!("package-go-{tier}-{name}"), M3, "go", args, "{work}/packages/go");
            item.environment.insert("GOTOOLCHAIN".into(), go.into()); steps.push(item);
        }
        for (name, args) in [
            ("lock", &["generate-lockfile", "--offline"][..]),
            ("models", &["check", "--offline", "--locked", "--no-default-features"][..]),
            ("http", &["check", "--offline", "--locked", "--features", "http"][..]),
            ("all-features", &["check", "--offline", "--locked", "--all-features"][..]),
            ("docs", &["doc", "--offline", "--locked", "--no-deps", "--all-features"][..]),
            ("archive", &["package", "--offline", "--locked", "--allow-dirty", "--no-verify"][..]),
        ] {
            let mut item = stage(&format!("package-rust-{tier}-{name}"), M3, "cargo", args, "{work}/packages/rust");
            item.environment.extend([("RUSTUP_TOOLCHAIN".into(), rust.into()), ("CARGO_TARGET_DIR".into(), format!("{{work}}/package-rust-{tier}")), ("RUSTDOCFLAGS".into(), "-D warnings".into())]); steps.push(item);
        }
    }
    for (name, args) in [("install", &["ci", "--offline", "--ignore-scripts", "--no-audit", "--no-fund"][..]), ("build", &["run", "build"][..]), ("archive", &["pack", "--offline", "--ignore-scripts", "--json"][..])] {
        steps.push(stage(&format!("package-typescript-{name}"), M3, "npm", args, "{work}/packages/typescript"));
    }
    for tier in ["floor", "current"] {
        let mut item = stage(&format!("package-swift-{tier}-build"), M3, if tier == "floor" { "{swift-floor}" } else { "{swift}" }, &["build", "--package-path", "{swift-package}", "--scratch-path", &format!("{{work}}/swift/{tier}/build/package"), "--cache-path", &format!("{{work}}/swift/{tier}/package-cache"), "-Xswiftc", "-warnings-as-errors"], "{work}");
        item.environment = swift_environment(tier);
        if let Some(sdk) = item.environment.get("SUSPECT_SWIFT_SDKROOT").cloned() { item.args.extend(["--sdk".into(), sdk]); }
        steps.push(item);
    }
    steps
}

pub(super) fn tool_inputs(run: &mut Run, venv: &Path) -> Result<Value> {
    // Installed compiler/runtime packages are outside the source census. Hash
    // their actual bytes as well as the lockfiles and recorded version commands.
    write_json(
        &run.out.join("tool-selection.json"),
        &json!({"selectors":run.replacements,"resolutionChecks":run.checks.iter().filter(|check| ["tool-swift-path", "tool-swiftc-path", "tool-docc-path", "tool-swift-current-sdk-path", "tool-swift-current-sdk"].iter().any(|id| check["id"] == *id)).collect::<Vec<_>>()}),
    )?;
    let failed = run
        .checks
        .iter()
        .filter(|check| {
            check["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("install-typescript-"))
                && check["criterionMet"] != true
        })
        .map(|check| {
            format!(
                "{} (stderr: {})",
                check["id"].as_str().unwrap(),
                check["stderr"].as_str().unwrap_or("missing log")
            )
        })
        .collect::<Vec<_>>();
    ensure!(
        failed.is_empty(),
        "installed tool inventory blocked by failed preparation: {}",
        failed.join(", ")
    );
    run.remember(&PathBuf::from(run.expand("{swift-floor-sdk}")).join("SDKSettings.json"))
        .context("floor SDK metadata unavailable; inspect tool-swift-floor-sdk")?;
    run.remember(&PathBuf::from(run.expand("{swiftc-floor}")).with_file_name("swift-frontend"))?;
    let mut roots = vec![venv.join("lib")];
    for tool in [
        "typescript-docs",
        "typescript-floor",
        "typescript-http-bundle",
    ] {
        roots.push(
            run.work
                .join("source/crates/suspect-codegen/tools")
                .join(tool)
                .join("node_modules"),
        );
    }
    for root in &roots {
        ensure!(
            root.is_dir(),
            "required installed tool directory is missing: {}; inspect its preparation check and logs",
            root.display()
        );
        for name in tree_names(root)
            .with_context(|| format!("inventory installed tool files under {}", root.display()))?
        {
            let path = root.join(name);
            if path.is_file() {
                run.remember(&path)?;
            }
        }
    }
    for tier in ["floor", "current"] {
        let go: Value = serde_json::from_slice(&fs::read(
            run.out
                .join(format!("logs/tool-go-{tier}-config.stdout.log")),
        )?)?;
        run.remember(&Path::new(go["GOROOT"].as_str().context("Go root")?).join("bin/go"))?;
        for binary in ["compile", "link"] {
            run.remember(
                &Path::new(go["GOTOOLDIR"].as_str().context("Go tool directory")?).join(binary),
            )?;
        }
    }
    let home = PathBuf::from(run.env.get("HOME").context("HOME")?);
    let cargo = run
        .env
        .get("CARGO_HOME")
        .map_or_else(|| home.join(".cargo"), PathBuf::from);
    let rustup = run
        .env
        .get("RUSTUP_HOME")
        .map_or_else(|| home.join(".rustup"), PathBuf::from);
    let mut configs = BTreeMap::new();
    for path in [
        cargo.join("config"),
        cargo.join("config.toml"),
        rustup.join("settings.toml"),
        home.join(".npmrc"),
        home.join(".gitconfig"),
    ] {
        if path.exists() {
            run.remember(&path)?;
            configs.insert(path, true);
        } else {
            run.absent_configs.insert(path.clone());
            configs.insert(path, false);
        }
    }
    write_json(&run.out.join("tool-files.json"), &run.files)?;
    Ok(
        json!({"installedToolRoots":roots,"configPresence":configs,"fingerprints":"tool-files.json","sha256":hash(&run.out.join("tool-files.json"))?}),
    )
}

fn workflow(run: &mut Run, args: &Args, required: &mut Vec<Stage>) -> Result<()> {
    let files = prepare(run)?;
    let perf = if args.demo {
        Vec::new()
    } else {
        load_performance(run, args.performance_plan.as_deref(), required)?
    };
    let editor = files
        .iter()
        .filter_map(|(name, file)| {
            name.strip_prefix("editors/vscode/")
                .map(|name| (name.to_owned(), file.clone()))
        })
        .collect();
    copy_inventory(
        &run.out.join("source/editors/vscode"),
        &run.work.join("editor"),
        &editor,
    )?;
    // Older native helpers locate this venv relative to CARGO_MANIFEST_DIR.
    // The execution copy gets that compatibility link; the real workspace stays untouched.
    let tools = PathBuf::from(run.expand("{python-tools}"));
    let venv = tools
        .parent()
        .and_then(Path::parent)
        .context("Python tools venv")?;
    ensure!(
        venv.join("pyvenv.cfg").is_file(),
        "SUSPECT_PYTHON_TOOLS must select the prepared tool venv"
    );
    #[cfg(unix)]
    std::os::unix::fs::symlink(venv, run.work.join("source/target/sdk-native-python-tools"))?;
    run.remember(&venv.join("pyvenv.cfg"))?;
    require_external_root(&run.work)?;
    run.internal("scratch-isolation", Ok(json!({"workDirectory":run.work,"ancestorCargoManifests":[],"ancestorGitDirectories":[],"nativeTemporaryRoot":run.work.join("tmp")})))?;
    for item in tool_stages() {
        run.command(&item)?;
    }
    let tool_evidence = tool_inputs(run, venv);
    run.internal("tool-config-inputs", tool_evidence)?;
    let build = stage(
        "build-cli",
        BOTH,
        "cargo",
        &["build", "--locked", "-p", "suspect-cli", "--bin", "suspect"],
        "{workspace}",
    );
    run.command(&build)?;
    ensure!(
        run.checks.last().context("CLI build")?["criterionMet"] == true,
        "frozen CLI build failed"
    );
    let pinned = run.out.join("bin/suspect");
    fs::create_dir_all(pinned.parent().context("binary parent")?)?;
    fs::copy(run.work.join("cargo/debug/suspect"), &pinned)?;
    run.remember(&pinned)?;
    run.env
        .insert("SUSPECT_M3_M6_BINARY_SHA256".into(), hash(&pinned)?);
    run.internal("frozen-cli", Ok(json!({"path":pinned,"sha256":hash(&pinned)?,"builtFromSourceFingerprint":run.env["SUSPECT_M3_M6_SOURCE_SHA256"]})))?;
    run.command(&stage(
        "frozen-cli-version",
        BOTH,
        "{out}/bin/suspect",
        &["--version"],
        "{workspace}",
    ))?;
    let targets = [
        ("typescript-http", "@suspect-fixtures/m3-m6-sdk"),
        ("rust-http", "m3-m6-sdk"),
        ("python-http", "m3-m6-sdk"),
        ("go-http", "example.com/m3-m6-sdk"),
        ("swift-http", "M3M6SDK"),
    ]
    .map(
        |(backend, name)| json!({"backend":backend,"package_name":name,"package_version":"0.0.0"}),
    );
    write_json(
        &run.out.join("configs/five-operations.json"),
        &json!({"spec":run.out.join("inputs").join(INPUTS[0].1),"targets":targets,"operation_ids":OPERATIONS,"owner":"sdk-m3-m6-five-operations"}),
    )?;
    let packages = package_stages();
    for item in &packages[..3] {
        run.command(item)?;
    }
    let generated = run.work.join("packages");
    let package_snapshot = (|| -> Result<Value> {
        let native_files = inventory(&generated, tree_names(&generated)?.into_iter())?;
        ensure!(
            !native_files.is_empty(),
            "frozen CLI produced no package sources"
        );
        let manifests = native_files
            .keys()
            .filter(|name| {
                Path::new(name)
                    .file_name()
                    .is_some_and(|name| name == "Package.swift")
            })
            .collect::<Vec<_>>();
        ensure!(
            manifests.len() == 1,
            "integrated Swift backend must emit one Package.swift"
        );
        run.replacements.insert(
            "{swift-package}".into(),
            generated
                .join(manifests[0])
                .parent()
                .context("Swift package")?
                .display()
                .to_string(),
        );
        copy_inventory(&generated, &run.out.join("generated"), &native_files)?;
        write_json(&run.out.join("generated-manifest.json"), &native_files)?;
        Ok(
            json!({"snapshot":"generated/","manifest":"generated-manifest.json","sha256":hash(&run.out.join("generated-manifest.json"))?}),
        )
    })();
    run.internal("native-package-snapshot", package_snapshot)?;
    for item in &packages[3..] {
        run.command(item)?;
    }
    let archives = (|| -> Result<Value> {
        let paths = [
            "wheels/m3_m6_sdk-0.0.0-py3-none-any.whl",
            "packages/rust/Cargo.lock",
            "packages/typescript/suspect-fixtures-m3-m6-sdk-0.0.0.tgz",
            "package-rust-floor/package/m3-m6-sdk-0.0.0.crate",
            "package-rust-current/package/m3-m6-sdk-0.0.0.crate",
        ];
        let mut copied = BTreeMap::new();
        for path in paths {
            let source = run.work.join(path);
            let dest = run.out.join("native-artifacts").join(path);
            create(&dest)?.write_all(&fs::read(&source)?)?;
            copied.insert(path, hash(&dest)?);
            run.remember(&dest)?;
        }
        Ok(json!(copied))
    })();
    run.internal("native-archives", archives)?;
    let suites = suite_stages();
    for package in ["suspect-codegen", "suspect-cli"] {
        let names = suites
            .iter()
            .filter(|s| s.id.starts_with("cli-") == (package == "suspect-cli"))
            .map(|s| s.program.trim_start_matches("test:").to_owned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        run.command(&test_build(package, &names))?;
    }
    // Existing CLI tests embed CARGO_BIN_EXE_suspect at compile time. Freeze that
    // private path too, rather than pretending an environment override changes it.
    fs::copy(&pinned, run.work.join("cargo/debug/suspect"))?;
    for item in &suites {
        run.command(item)?;
    }
    let process_cli = run.work.join("cargo/debug/suspect");
    run.internal(
        "frozen-process-cli",
        (|| -> Result<Value> {
            ensure!(
                hash(&process_cli)? == hash(&pinned)?,
                "CLI process tests did not retain the frozen binary"
            );
            Ok(json!({"compiledProcessPath":process_cli,"sha256":hash(&pinned)?}))
        })(),
    )?;
    for item in editor_stages()
        .iter()
        .chain(perf.iter())
        .chain(quality_stages().iter())
    {
        run.command(item)?;
    }
    let after = source_inventory(run, "after")?;
    write_json(&run.out.join("source-manifest-after.json"), &after)?;
    let integrity = (|| -> Result<Value> {
        require_external_root(&run.work)?;
        ensure!(
            files == after,
            "original source changed while acceptance ran"
        );
        ensure!(
            inventory(&run.out.join("source"), files.keys().cloned())? == files,
            "source snapshot was modified"
        );
        ensure!(
            inventory(&run.work.join("source"), files.keys().cloned())? == files,
            "execution source was modified"
        );
        for (path, digest) in &run.files {
            ensure!(
                hash(path)? == *digest,
                "binary, tool, corpus or linked evidence changed: {}",
                path.display()
            );
        }
        for path in &run.absent_configs {
            ensure!(
                !path.exists(),
                "new ambient configuration appeared: {}",
                path.display()
            );
        }
        Ok(
            json!({"sourceUnchanged":true,"snapshotUnchanged":true,"executionSourceUnchanged":true,"inputsBinaryToolsAndLinksUnchanged":true,"files":run.files}),
        )
    })();
    run.internal("final-integrity", integrity)?;
    Ok(())
}

pub(super) fn seal(root: &Path) -> Result<String> {
    let mut files = BTreeMap::new();
    for name in tree_names(root)? {
        let path = root.join(&name);
        let value = if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            let target = fs::read_link(&path)?;
            json!({"kind":"symlink","target":target,"sha256":sha(target.to_str().context("UTF-8 link")?.as_bytes())})
        } else {
            json!({"kind":"file","sha256":hash(&path)?})
        };
        files.insert(name, value);
    }
    write_json(
        &root.join("seal.json"),
        &json!({"format":"suspect.sdk.evidence-seal.v1","files":files}),
    )?;
    let digest = hash(&root.join("seal.json"))?;
    create(&root.join("seal.sha256"))?.write_all(format!("{digest}  seal.json\n").as_bytes())?;
    fn readonly(directory: &Path) -> Result<()> {
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                readonly(&path)?;
            }
            let mut permissions = metadata.permissions();
            permissions.set_readonly(true);
            fs::set_permissions(path, permissions)?;
        }
        let mut permissions = fs::metadata(directory)?.permissions();
        permissions.set_readonly(true);
        fs::set_permissions(directory, permissions)?;
        Ok(())
    }
    readonly(root)?;
    Ok(digest)
}

fn required_stages() -> Vec<Stage> {
    let mut required = suite_stages();
    required.extend(tool_stages());
    required.extend(editor_stages());
    required.extend(quality_stages());
    required.extend(package_stages());
    for id in [
        "source-head",
        "source-status",
        "source-index",
        "source-unstaged-patch",
        "source-staged-patch",
        "source-files-before",
        "source-files-after",
        "input-head",
        "source-snapshot",
        "tracked-inputs",
        "scratch-isolation",
        "tool-config-inputs",
        "build-cli",
        "frozen-cli",
        "frozen-cli-version",
        "build-tests-suspect-codegen",
        "build-tests-suspect-cli",
        "frozen-process-cli",
        "native-package-snapshot",
        "native-archives",
        "final-integrity",
    ] {
        required.push(stage(id, BOTH, "", &[], ""));
    }
    for (id, _, _) in INPUTS {
        required.push(stage(&format!("input-tracked-{id}"), BOTH, "", &[], ""));
    }
    required.extend(PERFORMANCE.iter().map(|id| stage(id, M6, "", &[], "")));
    required
}

fn acceptance_profile(demo: bool) -> &'static str {
    if demo {
        "hackathon-demo"
    } else {
        "full-calibrated"
    }
}

fn required_for_profile(demo: bool) -> Vec<Stage> {
    required_stages()
        .into_iter()
        .filter(|stage| !demo || !PERFORMANCE.contains(&stage.id.as_str()))
        .collect()
}

fn stage_inventory(demo: bool) -> Result<Value> {
    let required = required_for_profile(demo);
    let mut ids = BTreeSet::new();
    ensure!(
        required.iter().all(|s| ids.insert(&s.id)),
        "duplicate required stage identity"
    );
    let counts = ["M3", "M6"]
        .into_iter()
        .map(|id| {
            (
                id,
                required
                    .iter()
                    .filter(|s| s.milestones.iter().any(|m| m == id))
                    .count(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    Ok(
        json!({"format":"suspect.sdk.m3-m6.stages.v1","acceptanceProfile":acceptance_profile(demo),"numericalPerformanceRequired":!demo,"stageCount":required.len(),"milestoneCounts":counts,"stages":required.iter().map(|s| json!({"id":s.id,"milestones":s.milestones})).collect::<Vec<_>>()}),
    )
}

fn milestone_results(required: &[Stage], checks: &[Value], aborted: bool) -> Vec<Value> {
    ["M3", "M6"].into_iter().map(|id| {
        let evidence = required.iter().filter(|s| s.milestones.iter().any(|m| m == id)).map(|s| s.id.clone()).collect::<Vec<_>>();
        let missing = evidence.iter().filter(|id| !checks.iter().any(|c| c["id"] == id.as_str() && c["criterionMet"] == true)).cloned().collect::<Vec<_>>();
        json!({"id":id,"complete":!aborted && missing.is_empty() && !evidence.is_empty(),"requiredEvidence":evidence,"unmet":missing})
    }).collect()
}

pub(super) fn require_external_root(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        for marker in ["Cargo.toml", ".git"] {
            ensure!(
                !ancestor.join(marker).exists(),
                "scratch must be outside repositories and Cargo workspaces; found {}",
                ancestor.join(marker).display()
            );
        }
    }
    Ok(())
}

pub(super) fn work_path(out: &Path, scratch: &Path) -> Result<PathBuf> {
    let existing = scratch
        .ancestors()
        .find(|path| path.exists())
        .context("scratch has no existing ancestor")?;
    require_external_root(&existing.canonicalize()?)?;
    fs::create_dir_all(scratch)
        .with_context(|| format!("create external scratch parent {}", scratch.display()))?;
    let scratch = scratch.canonicalize()?;
    require_external_root(&scratch)?;
    let name = out
        .file_name()
        .context("output name")?
        .to_str()
        .context("UTF-8 output name")?;
    let identity = sha(out.as_os_str().as_encoded_bytes());
    Ok(scratch.join(format!("sdk-{name}-{}.work", &identity[..12])))
}

/// One immutable run of the revised scope; never implies whole-SDK readiness.
pub(super) fn run(raw: &[String]) -> Result<()> {
    if raw.iter().any(|arg| arg == "--list-stages") {
        let demo = raw.iter().any(|arg| arg == "--demo");
        ensure!(
            raw.len() == if demo { 2 } else { 1 },
            "--list-stages accepts only --demo"
        );
        println!("{}", serde_json::to_string_pretty(&stage_inventory(demo)?)?);
        return Ok(());
    }
    if raw == ["--help"] || raw == ["-h"] {
        print!("{USAGE}");
        return Ok(());
    }
    let args = Args::parse(raw)?;
    let original = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("workspace")?
        .canonicalize()?;
    let source = args.source.canonicalize()?;
    let requested = if args.out.is_absolute() {
        args.out.clone()
    } else {
        original.join(&args.out)
    };
    let parent = requested
        .parent()
        .context("output parent")?
        .canonicalize()?;
    ensure!(
        parent.starts_with(original.join("target").canonicalize()?) && !parent.starts_with(&source),
        "output must be a new directory under this workspace's target/"
    );
    let out = parent.join(requested.file_name().context("output name")?);
    let work = work_path(&out, &std::env::temp_dir().join("opencode"))?;
    ensure!(
        !out.exists() && !work.exists(),
        "preserve prior evidence/work directories; choose a new --out"
    );
    fs::create_dir(&out)?;
    fs::create_dir(&work)?;
    let replacements = BTreeMap::from([
        ("{out}".into(), out.display().to_string()),
        ("{work}".into(), work.display().to_string()),
        (
            "{workspace}".into(),
            work.join("source").display().to_string(),
        ),
        ("{original}".into(), original.display().to_string()),
    ]);
    let mut run = Run {
        original,
        source,
        out,
        work,
        env: BTreeMap::new(),
        replacements,
        checks: Vec::new(),
        tests: BTreeMap::new(),
        files: BTreeMap::new(),
        absent_configs: BTreeSet::new(),
        provenance: Value::Null,
    };
    env_values(&mut run)?;
    for node in ["node22", "node24"] {
        let path = PathBuf::from(run.expand(&format!("{{{node}}}")));
        run.replacements.insert(
            format!("{{{node}-bin}}"),
            path.parent()
                .context("Node directory")?
                .display()
                .to_string(),
        );
    }
    run.replacements
        .insert("{path}".into(), run.env["PATH"].clone());
    fs::create_dir_all(run.out.join("performance"))?;
    let mut required = required_for_profile(args.demo);
    write_json(
        &run.out.join("invocation.json"),
        &json!({"args":raw,"acceptanceProfile":acceptance_profile(args.demo),"runner":std::env::current_exe()?,"runnerSha256":hash(&std::env::current_exe()?)?,"environment":run.env,"selectors":run.replacements,"sourcePolicy":"copy the actual dirty tree, never a fresh HEAD checkout","workDirectory":run.work}),
    )?;
    let outcome = workflow(&mut run, &args, &mut required);
    let error = outcome.err().map(|e| format!("{e:#}"));
    for id in PERFORMANCE.iter().filter(|_| !args.demo) {
        if !run.checks.iter().any(|check| check["id"] == *id) {
            unmet_performance(
                &mut run,
                id,
                error
                    .as_deref()
                    .unwrap_or("required performance stage produced no evidence"),
            )?;
        }
    }
    let mut unique = BTreeSet::new();
    ensure!(
        required.iter().all(|s| unique.insert(s.id.clone())),
        "duplicate required stage identity"
    );
    let milestones = milestone_results(&required, &run.checks, error.is_some());
    let complete = milestones.iter().all(|m| m["complete"] == true)
        && run.checks.iter().all(|c| c["criterionMet"] == true);
    write_json(
        &run.out.join("report.json"),
        &json!({"format":"suspect.sdk.m3-m6.v1","acceptanceProfile":acceptance_profile(args.demo),"numericalPerformanceRequired":!args.demo,"deferredStages":if args.demo { PERFORMANCE } else { &[] },"complete":complete,"releaseReady":false,"sdkReleaseReady":false,
        "scope":["M3","M6"],"targets":["python","go","swift","rust","typescript","javascript"],"milestones":milestones,
        "source":run.provenance,
        "currentFullCorpusAcceptance":null,"checks":run.checks,"requiredStageCount":required.len(),"requiredStages":required,"fileFingerprints":run.files,"error":error}),
    )?;
    let digest = seal(&run.out)?;
    println!(
        "M3/M6 {} {}: {}\nseal SHA-256: {digest}",
        acceptance_profile(args.demo),
        if complete { "complete" } else { "INCOMPLETE" },
        run.out.join("report.json").display()
    );
    ensure!(
        complete,
        "required M3/M6 evidence is incomplete; inspect report.json and logs"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isolated_run() -> (tempfile::TempDir, Run) {
        let root = tempfile::tempdir().unwrap();
        let run = Run {
            original: root.path().into(),
            source: root.path().into(),
            out: root.path().join("report"),
            work: root.path().join("work"),
            env: BTreeMap::new(),
            replacements: BTreeMap::new(),
            checks: Vec::new(),
            tests: BTreeMap::new(),
            files: BTreeMap::new(),
            absent_configs: BTreeSet::new(),
            provenance: Value::Null,
        };
        (root, run)
    }

    #[test]
    fn required_test_results_reject_false_green_summaries() {
        let good = "test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n";
        assert!(complete_rust_tests(good));
        for bad in [
            String::new(),
            good.replace("3 passed", "0 passed"),
            good.replace("0 ignored", "1 ignored"),
            good.replace("0 filtered out", "2 filtered out"),
            good.repeat(2),
        ] {
            assert!(!complete_rust_tests(&bad), "{bad}");
        }
        let tap = "# tests 2\n# pass 2\n# fail 0\n# cancelled 0\n# skipped 0\n# todo 0\n";
        assert!(complete_node_tests(tap));
        assert!(!complete_node_tests(
            &tap.replace("# skipped 0", "# skipped 1")
        ));
        assert!(!complete_node_tests(&tap.replace("# pass 2", "# pass 1")));
        assert!(has_skip(
            &json!({"complete":true,"stages":[{"status":"skipped"}]})
        ));
        assert!(has_skip(&json!({"complete":true,"missingTools":["swift"]})));
        assert!(!has_skip(&json!({"complete":true,"skipped":0})));
    }

    #[test]
    fn actual_source_inventory_includes_untracked_bytes_and_deleted_paths() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("tracked.rs"), "original").unwrap();
        fs::write(root.path().join("untracked.rs"), "implementation").unwrap();
        let names = strings(&["tracked.rs", "untracked.rs", "deleted.rs"]);
        let first = inventory(root.path(), names.clone().into_iter()).unwrap();
        assert_eq!(first["deleted.rs"].kind, "deleted");
        let snapshot = tempfile::tempdir().unwrap();
        copy_inventory(root.path(), snapshot.path(), &first).unwrap();
        fs::write(root.path().join("untracked.rs"), "new implementation").unwrap();
        assert_ne!(first, inventory(root.path(), names.into_iter()).unwrap());
        assert_eq!(
            fs::read_to_string(snapshot.path().join("untracked.rs")).unwrap(),
            "implementation"
        );
        assert!(relative("../outside").is_err());
    }

    #[test]
    fn stage_inventory_covers_every_required_identity_once() {
        let inventory = stage_inventory(false).unwrap();
        let stages = inventory["stages"].as_array().unwrap();
        assert_eq!(stages.len(), required_stages().len());
        assert_eq!(inventory["stageCount"], stages.len());
        for id in PERFORMANCE {
            assert!(stages.iter().any(|s| s["id"] == *id));
        }
        assert!(stages.iter().any(|s| s["id"] == "source-files-after"));
    }

    #[test]
    fn demo_acceptance_defers_only_numerical_gates_and_keeps_native_failures_blocking() {
        let full = required_for_profile(false);
        let demo = required_for_profile(true);
        let full_ids = full
            .iter()
            .map(|stage| stage.id.as_str())
            .collect::<BTreeSet<_>>();
        let demo_ids = demo
            .iter()
            .map(|stage| stage.id.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            full_ids
                .difference(&demo_ids)
                .copied()
                .collect::<BTreeSet<_>>(),
            PERFORMANCE.iter().copied().collect()
        );
        assert_eq!(demo.len(), 208);
        let mut checks=full.iter().map(|stage|json!({"id":stage.id,"criterionMet":!PERFORMANCE.contains(&stage.id.as_str())})).collect::<Vec<_>>();
        assert!(
            milestone_results(&demo, &checks, false)
                .iter()
                .all(|result| result["complete"] == true)
        );
        assert_eq!(
            milestone_results(&full, &checks, false)[1]["complete"],
            false
        );
        checks
            .iter_mut()
            .find(|check| check["id"] == "package-python-wheel")
            .unwrap()["criterionMet"] = json!(false);
        assert_eq!(
            milestone_results(&demo, &checks, false)[0]["complete"],
            false
        );
        assert_eq!(
            stage_inventory(true).unwrap()["acceptanceProfile"],
            "hackathon-demo"
        );
    }

    #[test]
    fn demo_scope_is_explicit_and_cannot_claim_calibrated_evidence() {
        let parse = |args: &[&str]| {
            Args::parse(&args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>())
        };
        assert!(
            parse(&["--demo", "--source", "source", "--out", "output"])
                .unwrap()
                .demo
        );
        assert!(
            !parse(&["--source", "source", "--out", "output"])
                .unwrap()
                .demo
        );
        assert!(parse(&["--demo", "--demo", "--source", "source", "--out", "output"]).is_err());
        assert!(
            parse(&[
                "--demo",
                "--source",
                "source",
                "--out",
                "output",
                "--performance-plan",
                "plan.json"
            ])
            .is_err()
        );
    }

    #[test]
    fn performance_success_requires_real_scenarios_and_zero_redundant_work() {
        let claims = [
            "complete",
            "measurementStatus",
            "warmCompiles",
            "warmRenders",
            "warmWrites",
            "coldSamples",
            "warmSamples",
            "sourceChangeSamples",
            "configChangeSamples",
        ]
        .map(|key| (key.into(), format!("/{key}")))
        .into_iter()
        .collect();
        let mut result = json!({"complete":true,"measurementStatus":"observational","warmCompiles":0,"warmRenders":0,"warmWrites":0,"coldSamples":5,"warmSamples":5,"sourceChangeSamples":5,"configChangeSamples":5});
        assert!(performance_claims(&result, &claims, false).is_ok());
        result["measurementStatus"] = json!("not-measured");
        assert!(performance_claims(&result, &claims, false).is_err());
        result["measurementStatus"] = json!("observational");
        result["warmWrites"] = json!(1);
        assert!(performance_claims(&result, &claims, false).is_err());
        result["warmWrites"] = json!(0);
        result
            .as_object_mut()
            .unwrap()
            .remove("configChangeSamples");
        assert!(performance_claims(&result, &claims, false).is_err());
        let claims = [
            "complete",
            "regressions",
            "comparedCases",
            "gated",
            "verdict",
        ]
        .map(|key| (key.into(), format!("/{key}")))
        .into_iter()
        .collect();
        assert!(
            performance_claims(
                &json!({"complete":true,"regressions":[],"comparedCases":3,"gated":true,"verdict":"passed"}),
                &claims,
                true
            )
            .is_ok()
        );
        assert!(
            performance_claims(
                &json!({"complete":true,"regressions":["warm"],"comparedCases":3,"gated":true,"verdict":"passed"}),
                &claims,
                true
            )
            .is_err()
        );
        assert!(
            performance_claims(
                &json!({"complete":true,"regressions":[],"comparedCases":0,"gated":true,"verdict":"passed"}),
                &claims,
                true
            )
            .is_err()
        );
        for (gated, verdict) in [
            (false, "observational"),
            (true, "inconclusive"),
            (false, "passed"),
        ] {
            assert!(performance_claims(&json!({"complete":true,"regressions":[],"comparedCases":3,"gated":gated,"verdict":verdict}), &claims, true).is_err());
        }
    }

    #[test]
    fn absent_or_unusable_performance_does_not_block_other_milestone_evidence() {
        let args = Args::parse(&strings(&["--source", "source", "--out", "out"])).unwrap();
        assert!(args.performance_plan.is_none());
        for case in ["absent", "missing", "malformed", "bad-pin"] {
            let (root, mut run) = isolated_run();
            let plan = root.path().join("plan.json");
            let pin = root.path().join("pin.json");
            fs::write(&pin, "baseline").unwrap();
            if case == "malformed" {
                fs::write(&plan, "{invalid").unwrap();
            }
            if case == "bad-pin" {
                let stages = PERFORMANCE.iter().map(|id| json!({"id":id,"program":"never-run","args":[],"cwd":"{workspace}","report":"{out}/performance/new.json","assertions":{},"claims":{}})).collect::<Vec<_>>();
                fs::write(&plan, json!({"format":"suspect.sdk.m3-m6.performance-plan.v1","inputs":[{"path":pin,"sha256":hash(&pin).unwrap()},{"path":pin,"sha256":"0".repeat(64)}],"stages":stages}).to_string()).unwrap();
            }
            let mut required = vec![stage("other-evidence", BOTH, "", &[], "")];
            assert!(
                load_performance(
                    &mut run,
                    (case != "absent").then_some(plan.as_path()),
                    &mut required
                )
                .unwrap()
                .is_empty()
            );
            assert!(
                run.files.is_empty(),
                "invalid performance pins leaked into common integrity"
            );
            assert_eq!(run.checks.len(), PERFORMANCE.len());
            assert!(run.checks.iter().all(|c| c["milestones"] == json!(M6)
                && c["state"] == "unmet"
                && c["criterionMet"] == false));
            run.internal("other-evidence", Ok(json!({"testFixture":true})))
                .unwrap();
            required.extend(PERFORMANCE.iter().map(|id| stage(id, M6, "", &[], "")));
            let results = milestone_results(&required, &run.checks, false);
            assert_eq!(results[0]["complete"], true);
            assert_eq!(results[1]["complete"], false);
            assert_eq!(results[1]["unmet"], json!(PERFORMANCE));
        }
    }

    #[test]
    fn swift_floor_and_shared_vectors_require_the_actual_native_test() {
        let (_root, mut run) = isolated_run();
        let suites = suite_stages();
        for tier in ["floor", "current"] {
            let item = suites
                .iter()
                .find(|s| s.id == format!("{tier}-swift-shared-runtime"))
                .unwrap();
            assert_eq!(item.program, format!("test:{CODEGEN_LIB}"));
            assert!(item.args.contains(&"--include-ignored".into()));
            assert!(
                !item.args.contains(&SWIFT_VECTORS.into()),
                "native gate must not filter other library tests"
            );
            let summary = "test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n";
            assert!(run.evaluate(&item.criterion, summary).is_err());
            assert!(
                run.evaluate(
                    &item.criterion,
                    &format!("test {SWIFT_VECTORS} ... ok\n{summary}")
                )
                .is_ok()
            );
        }
        let floor = swift_environment("floor");
        assert_eq!(floor["SUSPECT_SWIFT_BIN"], "{swift-floor}");
        assert_eq!(floor["SWIFT_EXEC"], "{swiftc-floor}");
        assert_eq!(floor["SDKROOT"], "{swift-floor-sdk}");
        assert_eq!(floor["SUSPECT_SWIFT_DOCC_BIN"], "{docc-floor}");
        let build = test_build("suspect-codegen", &strings(&[CODEGEN_LIB, "swift_sdk"]));
        assert!(build.args.contains(&"--lib".into()));
        assert!(!build.args.contains(&CODEGEN_LIB.into()));
        assert_eq!(build.environment["RUSTUP_TOOLCHAIN"], "stable");
    }

    #[test]
    fn missing_executables_never_satisfy_even_an_expected_failure() {
        let (root, mut run) = isolated_run();
        let mut missing = stage(
            "missing",
            M3,
            &root.path().join("missing-tool").display().to_string(),
            &[],
            root.path().to_str().unwrap(),
        );
        missing.expected_exit = 1;
        run.command(&missing).unwrap();
        assert_eq!(run.checks[0]["success"], false);
        assert_eq!(run.checks[0]["criterionMet"], false);
        assert_eq!(run.checks[0]["exitCode"], Value::Null);
    }

    #[test]
    fn scratch_parent_cannot_inherit_a_workspace_or_git_repository() {
        for marker in ["Cargo.toml", ".git"] {
            let root = tempfile::tempdir().unwrap();
            fs::write(root.path().join(marker), "marker").unwrap();
            let error = work_path(
                &root.path().join("target/report"),
                &root.path().join("tmp/opencode"),
            )
            .unwrap_err();
            assert!(error.to_string().contains(marker));
        }
    }

    #[test]
    fn sdk_version_requires_actual_settings_and_no_inferred_fallback() {
        assert!(
            sdk_settings(br#"{"DisplayName":"macOS 15.4","CanonicalName":"macosx15.4"}"#).is_err()
        );
        let metadata = sdk_settings(
            br#"{"DisplayName":"macOS 15.4","CanonicalName":"macosx15.4","Version":"15.4"}"#,
        )
        .unwrap();
        assert_eq!(metadata["Version"], "15.4");
    }

    #[test]
    fn swift_selection_uses_implementation_and_requires_symbolgraph_companion() {
        let (root, mut run) = isolated_run();
        let compiler = root.path().join("toolchain/usr/bin/swiftc");
        fs::create_dir_all(compiler.parent().unwrap()).unwrap();
        fs::write(&compiler, "compiler").unwrap();
        run.replacements
            .insert("{swiftc}".into(), "/usr/bin/swiftc".into());
        let rule = Criterion::AppleTool {
            name: "swiftc".into(),
        };
        assert!(run.evaluate(&rule, compiler.to_str().unwrap()).is_err());
        fs::write(
            compiler.with_file_name("swift-symbolgraph-extract"),
            "symbolgraph",
        )
        .unwrap();
        run.evaluate(&rule, compiler.to_str().unwrap()).unwrap();
        assert_eq!(run.env["SUSPECT_SWIFTC_BIN"], compiler.to_str().unwrap());
        assert_eq!(run.expand("{swiftc}"), compiler.to_str().unwrap());
    }

    #[test]
    fn current_sdk_defaults_match_the_selected_toolchain_and_reach_every_swift_gate() {
        let (root, mut run) = isolated_run();
        let developer = root.path().join("Xcode.app/Contents/Developer");
        let tools = developer.join("Toolchains/Current.xctoolchain/usr/bin");
        fs::create_dir_all(&tools).unwrap();
        for name in ["swift", "swiftc"] {
            fs::write(tools.join(name), name).unwrap();
            run.replacements.insert(
                format!("{{{name}}}"),
                tools.join(name).display().to_string(),
            );
        }
        let sdk = developer.join("Platforms/MacOSX.platform/Developer/SDKs/Current.sdk");
        let other = root.path().join("OtherXcode/SDKs/Other.sdk");
        for directory in [&sdk, &other] {
            fs::create_dir_all(directory).unwrap();
            fs::write(
                directory.join("SDKSettings.json"),
                br#"{"Version":"99.2","CanonicalName":"macosx99.2","DisplayName":"Fixture SDK"}"#,
            )
            .unwrap();
        }
        assert!(run.pin_current_sdk(&other, false).is_err());
        let selected = run.pin_current_sdk(&sdk, false).unwrap();
        assert_eq!(selected["version"], "99.2");
        assert_eq!(
            run.env["SUSPECT_SWIFT_SDKROOT"],
            sdk.canonicalize().unwrap().display().to_string()
        );
        assert!(
            run.pin_current_sdk(&other, true).is_ok(),
            "an explicit SDK override is recorded separately from discovery"
        );
        for item in suite_stages()
            .into_iter()
            .chain(package_stages())
            .filter(|s| {
                matches!(
                    s.id.as_str(),
                    "current-swift_sdk"
                        | "current-swift-shared-runtime"
                        | "package-swift-current-build"
                )
            })
        {
            assert_eq!(item.environment["SUSPECT_SWIFT_SDKROOT"], "{swift-sdk}");
            assert_eq!(item.environment["SDKROOT"], "{swift-sdk}");
            if item.id == "package-swift-current-build" {
                assert!(item.args.windows(2).any(|a| a == ["--sdk", "{swift-sdk}"]));
            }
        }
        let lookup = tool_stages()
            .into_iter()
            .find(|s| s.id == "tool-swift-current-sdk-path")
            .unwrap();
        assert_eq!(
            lookup.args,
            strings(&["--sdk", "macosx", "--show-sdk-path"])
        );
    }

    #[test]
    fn tool_inventory_names_the_failed_preparation_and_its_log() {
        let (root, mut run) = isolated_run();
        run.checks.push(json!({"id":"install-typescript-docs","criterionMet":false,"stderr":"logs/install-typescript-docs.stderr.log"}));
        let error = tool_inputs(&mut run, root.path()).unwrap_err().to_string();
        assert!(
            error.contains("install-typescript-docs.stderr.log"),
            "{error}"
        );
        assert!(error.contains("blocked by failed preparation"));
    }

    #[test]
    #[ignore = "requires installed current/floor Swift tools; tiny symbol-graph/tool-identity proof"]
    fn swift_tool_metadata_and_symbolgraph_use_real_implementations() {
        let scratch = std::env::temp_dir().join("opencode");
        fs::create_dir_all(&scratch).unwrap();
        let root = tempfile::Builder::new()
            .prefix("sdk-swift-tool-proof-")
            .tempdir_in(scratch)
            .unwrap()
            .keep();
        let mut run = Run {
            original: root.clone(),
            source: root.clone(),
            out: root.join("report"),
            work: root.join("work"),
            env: BTreeMap::new(),
            replacements: BTreeMap::new(),
            checks: Vec::new(),
            tests: BTreeMap::new(),
            files: BTreeMap::new(),
            absent_configs: BTreeSet::new(),
            provenance: Value::Null,
        };
        for (key, path) in [
            ("{out}", run.out.clone()),
            ("{work}", run.work.clone()),
            ("{workspace}", run.work.join("source")),
        ] {
            run.replacements
                .insert(key.into(), path.display().to_string());
        }
        env_values(&mut run).unwrap();
        run.replacements.remove("{swift-sdk-requested}"); // Prove automatic selection even if the caller exported an override.
        println!("Swift tool proof retained at {}", root.display());
        let wanted = [
            "tool-docc-path",
            "tool-swift-path",
            "tool-swiftc-path",
            "tool-docc",
            "tool-docc-floor",
            "tool-swift",
            "tool-swiftc",
            "tool-swift-floor-sdk",
            "tool-swift-current-sdk-path",
            "tool-swift-current-sdk",
        ];
        for item in tool_stages()
            .iter()
            .filter(|s| wanted.contains(&s.id.as_str()))
        {
            run.command(item).unwrap();
        }
        assert_eq!(run.checks.len(), wanted.len());
        assert!(
            run.checks.iter().all(|c| c["criterionMet"] == true),
            "{:?}",
            run.checks
                .iter()
                .filter(|c| c["criterionMet"] != true)
                .collect::<Vec<_>>()
        );
        let probe = run.work.join("stdlib.swift");
        fs::write(&probe, "import Foundation\nlet values: [Int] = [1, 2, 3]\nlet url = URL(string: \"https://example.invalid\")!\n").unwrap();
        let mut standalone = stage(
            "stdlib",
            M3,
            "{swiftc}",
            &[
                "-typecheck",
                "-swift-version",
                "6",
                "-module-cache-path",
                "{work}/swift/current/standalone-cache",
                probe.to_str().unwrap(),
            ],
            "{work}",
        );
        standalone.environment = swift_environment("current");
        if let Some(sdk) = standalone.environment.get("SUSPECT_SWIFT_SDKROOT").cloned() {
            standalone.args.extend(["-sdk".into(), sdk]);
        }
        run.command(&standalone).unwrap();
        assert_eq!(
            run.checks.last().unwrap()["criterionMet"],
            true,
            "standalone standard library probe failed; inspect {root:?}"
        );
        let sdk = run.work.join("swift-probe");
        fs::create_dir_all(sdk.join("Sources/IsolationProbe")).unwrap();
        fs::write(sdk.join("Package.swift"), "// swift-tools-version: 6.0\nimport PackageDescription\nlet package = Package(name: \"IsolationProbe\", targets: [.target(name: \"IsolationProbe\")])\n").unwrap();
        fs::write(
            sdk.join("Sources/IsolationProbe/Probe.swift"),
            "public struct Probe { public init() {} }\n",
        )
        .unwrap();
        let mut item = stage(
            "symbolgraph",
            M3,
            "{swift}",
            &[
                "package",
                "--sdk",
                "{swift-sdk}",
                "--package-path",
                sdk.to_str().unwrap(),
                "--scratch-path",
                "{work}/swift/current/build",
                "--cache-path",
                "{work}/swift/current/package-cache",
                "dump-symbol-graph",
                "--minimum-access-level",
                "public",
            ],
            "{work}",
        );
        item.environment = swift_environment("current");
        run.command(&item).unwrap();
        assert_eq!(
            run.checks.last().unwrap()["criterionMet"],
            true,
            "inspect {root:?}"
        );
        assert!(
            tree_names(&run.work)
                .unwrap()
                .iter()
                .any(|name| name.ends_with("IsolationProbe.symbols.json"))
        );
    }

    #[test]
    #[ignore = "requires Cargo 1.88.0 and tar; small packaging/isolation proof, not SDK acceptance"]
    fn cargo_floor_standalone_and_extracted_example_have_no_outer_workspace() {
        let scratch = std::env::temp_dir().join("opencode");
        fs::create_dir_all(&scratch).unwrap();
        let root = tempfile::Builder::new()
            .prefix("sdk-isolation-proof-")
            .tempdir_in(&scratch)
            .unwrap()
            .keep();
        println!("isolation proof retained at {}", root.display());
        let checkout = root.join("checkout");
        fs::create_dir_all(checkout.join("target")).unwrap();
        fs::write(
            checkout.join("Cargo.toml"),
            "[workspace]\nmembers=[]\nresolver='3'\n",
        )
        .unwrap();
        let work = work_path(&checkout.join("target/evidence"), &scratch).unwrap();
        let sdk = work.join("packages/sdk");
        fs::create_dir_all(sdk.join("src")).unwrap();
        fs::create_dir_all(sdk.join("examples")).unwrap();
        fs::write(sdk.join("Cargo.toml"), "[package]\nname='sdk-isolation-probe'\nversion='0.0.0'\nedition='2024'\nrust-version='1.88'\npublish=false\n[workspace]\n").unwrap();
        fs::write(sdk.join("src/lib.rs"), "pub fn answer() -> u8 { 42 }\n").unwrap();
        fs::write(sdk.join("examples/validated.rs"), "fn main() { assert_eq!(sdk_isolation_probe::answer(), 42); println!(\"isolated-example-ok\"); }\n").unwrap();
        let checked = |id: &str, command: &mut Command| {
            let argv = format!("{command:?}");
            let output = command.output().unwrap();
            write_json(
                &root.join(format!("{id}.json")),
                &json!({"command":argv,"exitCode":output.status.code()}),
            )
            .unwrap();
            fs::write(root.join(format!("{id}.stdout.log")), &output.stdout).unwrap();
            fs::write(root.join(format!("{id}.stderr.log")), &output.stderr).unwrap();
            assert!(
                output.status.success(),
                "{id}: {argv}\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
            output
        };
        let cargo = |action: &str, manifest: &Path| {
            let mut command = Command::new("cargo");
            command
                .args(["+1.88.0", action, "--offline", "--manifest-path"])
                .arg(manifest)
                .arg("--target-dir")
                .arg(work.join("build"))
                .current_dir(&work);
            command
        };
        checked("standalone", &mut cargo("check", &sdk.join("Cargo.toml")));
        checked(
            "package",
            cargo("package", &sdk.join("Cargo.toml")).args(["--allow-dirty", "--no-verify"]),
        );
        let vendor = work.join("vendor");
        fs::create_dir(&vendor).unwrap();
        checked(
            "extract",
            Command::new("tar")
                .arg("-xzf")
                .arg(work.join("build/package/sdk-isolation-probe-0.0.0.crate"))
                .arg("-C")
                .arg(&vendor),
        );
        let installed = vendor.join("sdk-isolation-probe-0.0.0/Cargo.toml");
        let result = checked(
            "example",
            cargo("run", &installed).args(["--locked", "--example", "validated"]),
        );
        assert_eq!(
            String::from_utf8(result.stdout).unwrap().trim(),
            "isolated-example-ok"
        );
        assert!(!work.starts_with(checkout));
        assert!(
            !work
                .ancestors()
                .any(|p| p.join(".git").exists() || p.join("Cargo.toml").exists())
        );
    }
}
