//! Maintained, source-frozen acceptance for the twelve-language SDK plan.
//!
//! This module deliberately has no dependency on codegen. The historical M3/M6
//! inventory is not extended: only its snapshot/command/seal mechanics are reused.

use super::sdk_m3_m6::{
    self as sealed, Criterion, INPUTS, Inventory, OPERATIONS, PERFORMANCE, Run, Stage,
    copy_inventory, create, hash, inventory, relative, sha, stage, strings, tree_names, write_json,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const FULL: &[&str] = &["SDK-FULL"];
const FEATURES: &str = "suspect-codegen/java-sdk,suspect-codegen/csharp-sdk,suspect-codegen/kotlin-sdk,suspect-codegen/ruby-sdk,suspect-codegen/php-sdk,suspect-codegen/dart-sdk,suspect-codegen/cpp-sdk,suspect-codegen/http-protocol";
const SWIFT_FLOOR_PAYLOAD: &str =
    "swift-6.0.3-protocol-reexpanded/swift-6.0.3-RELEASE-osx-package.pkg/Payload";
const SWIFT_FLOOR_SELECTORS: &[&str] = &[
    "SUSPECT_SWIFT_FLOOR_ROOT",
    "SUSPECT_SWIFT_FLOOR_BIN",
    "SUSPECT_SWIFTC_FLOOR_BIN",
    "SUSPECT_SWIFT_FLOOR_DOCC_BIN",
    "SUSPECT_SWIFT_FLOOR_SDKROOT",
];
const CODEGEN_LIBRARY: &str = "test:suspect-codegen/suspect_codegen";

#[derive(Clone, Copy, Serialize)]
struct LibraryNative {
    id: &'static str,
    language: &'static str,
    name: &'static str,
    source: &'static str,
    tiers: &'static [&'static str],
}

// Every ignored codegen-library test has an explicit native environment. The
// frozen harness census rejects newly ignored tests until this table covers them.
#[rustfmt::skip]
const LIBRARY_NATIVE: &[LibraryNative] = &[
    LibraryNative { id: "swift-shared-runtime", language: "swift", name: "swift_sdk::validation::tests::native_shared_runtime_contract_vectors", source: "crates/suspect-codegen/src/swift_sdk/validation.rs", tiers: &["floor", "current"] },
    LibraryNative { id: "swift-v2-runtime", language: "swift", name: "swift_sdk::validation::v2_tests::native_evaluated_applicator_vectors", source: "crates/suspect-codegen/src/swift_sdk/validation_v2.rs", tiers: &["floor", "current"] },
    LibraryNative { id: "swift-v3-runtime", language: "swift", name: "swift_sdk::validation::v3_tests::native_resource_dynamic_source_vectors", source: "crates/suspect-codegen/src/swift_sdk/validation_v3.rs", tiers: &["floor", "current"] },
    LibraryNative { id: "swift-v3-sdk", language: "swift", name: "swift_sdk::resources_tests::native_installed_v3_resources_codecs_types_wire_and_docs", source: "crates/suspect-codegen/src/swift_sdk/resources_tests.rs", tiers: &["floor", "current"] },
    LibraryNative { id: "swift-document-base", language: "swift", name: "swift_sdk::resources_tests::native_installed_physical_document_servers", source: "crates/suspect-codegen/src/swift_sdk/resources_tests.rs", tiers: &["floor", "current"] },
    LibraryNative { id: "swift-v3-aggregate-examples", language: "swift", name: "swift_sdk::aggregate_examples_tests::native_installed_declared_aggregate_examples", source: "crates/suspect-codegen/src/swift_sdk/aggregate_examples_tests.rs", tiers: &["floor", "current"] },
    LibraryNative { id: "csharp-v2-vectors", language: "csharp", name: "csharp_sdk::validation_tests::native_scoped_source_vectors", source: "crates/suspect-codegen/src/csharp_sdk/validation_tests.rs", tiers: &["matrix"] },
    LibraryNative { id: "csharp-v2-controls", language: "csharp", name: "csharp_sdk::validation_tests::native_scoped_limits_and_admission", source: "crates/suspect-codegen/src/csharp_sdk/validation_tests.rs", tiers: &["matrix"] },
    LibraryNative { id: "csharp-v2-sdk", language: "csharp", name: "csharp_sdk::validation_tests::native_scoped_sdk_packages", source: "crates/suspect-codegen/src/csharp_sdk/validation_tests.rs", tiers: &["matrix"] },
    LibraryNative { id: "csharp-document-base", language: "csharp", name: "csharp_sdk::server_tests::native_document_relative_servers", source: "crates/suspect-codegen/src/csharp_sdk/server_tests.rs", tiers: &["matrix"] },
    LibraryNative { id: "csharp-v3-vectors", language: "csharp", name: "csharp_sdk::resources_tests::native_resource_source_vectors", source: "crates/suspect-codegen/src/csharp_sdk/resources_tests.rs", tiers: &["matrix"] },
    LibraryNative { id: "csharp-v3-controls", language: "csharp", name: "csharp_sdk::resources_tests::native_resource_scope_and_admission", source: "crates/suspect-codegen/src/csharp_sdk/resources_tests.rs", tiers: &["matrix"] },
    LibraryNative { id: "csharp-v3-sdk", language: "csharp", name: "csharp_sdk::resources_tests::native_resource_sdk_packages", source: "crates/suspect-codegen/src/csharp_sdk/resources_tests.rs", tiers: &["matrix"] },
    LibraryNative { id: "csharp-credential-env-controls", language: "csharp", name: "csharp_sdk::credential_env_tests::native_environment_credentials", source: "crates/suspect-codegen/src/csharp_sdk/credential_env_tests.rs", tiers: &["matrix"] },
    LibraryNative { id: "csharp-credential-env-openrouter", language: "csharp", name: "csharp_sdk::credential_env_tests::native_openrouter_environment_client", source: "crates/suspect-codegen/src/csharp_sdk/credential_env_tests.rs", tiers: &["matrix"] },
    LibraryNative { id: "dart-v2-vectors", language: "dart", name: "dart_sdk::validation_tests::native_v2_source_vectors", source: "crates/suspect-codegen/src/dart_sdk/validation_tests.rs", tiers: &["floor", "current"] },
    LibraryNative { id: "dart-v2-sdk", language: "dart", name: "dart_sdk::sdk_v2_tests::native_v2_sdk_operations", source: "crates/suspect-codegen/src/dart_sdk/sdk_v2_tests.rs", tiers: &["floor", "current"] },
    LibraryNative { id: "dart-document-base", language: "dart", name: "dart_sdk::document_tests::native_document_relative_servers", source: "crates/suspect-codegen/src/dart_sdk/document_tests.rs", tiers: &["floor", "current"] },
    LibraryNative { id: "dart-v3-runtime", language: "dart", name: "dart_sdk::v3_tests::native_v3_source_resources", source: "crates/suspect-codegen/src/dart_sdk/v3_tests.rs", tiers: &["floor", "current"] },
    LibraryNative { id: "dart-v3-sdk", language: "dart", name: "dart_sdk::v3_sdk_tests::native_v3_sdk_operations", source: "crates/suspect-codegen/src/dart_sdk/v3_sdk_tests.rs", tiers: &["floor", "current"] },
    LibraryNative { id: "cpp-v2-vectors", language: "cpp", name: "cpp_sdk::v2_tests::native_v2_independent_32", source: "crates/suspect-codegen/src/cpp_sdk/v2_tests.rs", tiers: &["declared"] },
    LibraryNative { id: "cpp-v2-sdk", language: "cpp", name: "cpp_sdk::v2_tests::native_v2_sdk_operations", source: "crates/suspect-codegen/src/cpp_sdk/v2_tests.rs", tiers: &["declared"] },
    LibraryNative { id: "cpp-v2-controls", language: "cpp", name: "cpp_sdk::v2_tests::native_v2_scope_resource_edges", source: "crates/suspect-codegen/src/cpp_sdk/v2_tests.rs", tiers: &["declared"] },
    LibraryNative { id: "cpp-document-base", language: "cpp", name: "cpp_sdk::server_tests::native_document_relative_servers", source: "crates/suspect-codegen/src/cpp_sdk/server_tests.rs", tiers: &["declared"] },
    LibraryNative { id: "cpp-v3-vectors", language: "cpp", name: "cpp_sdk::v3_tests::native_v3_official_dynamic_ref_44", source: "crates/suspect-codegen/src/cpp_sdk/v3_tests.rs", tiers: &["declared"] },
    LibraryNative { id: "cpp-v3-controls", language: "cpp", name: "cpp_sdk::v3_tests::native_v3_scope_and_resource_controls", source: "crates/suspect-codegen/src/cpp_sdk/v3_tests.rs", tiers: &["declared"] },
    LibraryNative { id: "cpp-v3-sdk", language: "cpp", name: "cpp_sdk::v3_tests::native_v3_sdk_operations", source: "crates/suspect-codegen/src/cpp_sdk/v3_tests.rs", tiers: &["declared"] },
    LibraryNative { id: "php-v2-vectors", language: "php", name: "php_sdk::tests_v2::native_v2_source_vectors", source: "crates/suspect-codegen/src/php_sdk/tests_v2.rs", tiers: &["floor", "current"] },
    LibraryNative { id: "php-v2-sdk", language: "php", name: "php_sdk::tests_v2::native_v2_sdk_packages_models_codecs", source: "crates/suspect-codegen/src/php_sdk/tests_v2.rs", tiers: &["floor", "current"] },
    LibraryNative { id: "php-v2-controls", language: "php", name: "php_sdk::tests_v2::native_v2_scope_resource_controls", source: "crates/suspect-codegen/src/php_sdk/tests_v2.rs", tiers: &["floor", "current"] },
    LibraryNative { id: "php-v3-vectors", language: "php", name: "php_sdk::tests_v3::native_v3_source_fixtures", source: "crates/suspect-codegen/src/php_sdk/tests_v3.rs", tiers: &["floor", "current"] },
    LibraryNative { id: "php-v3-controls", language: "php", name: "php_sdk::tests_v3::native_v3_scope_resource_controls", source: "crates/suspect-codegen/src/php_sdk/tests_v3.rs", tiers: &["floor", "current"] },
    LibraryNative { id: "php-v3-sdk", language: "php", name: "php_sdk::tests_v3::native_v3_sdk_packages_models_codecs", source: "crates/suspect-codegen/src/php_sdk/tests_v3.rs", tiers: &["floor", "current"] },
];
const USAGE: &str = "usage: cargo run --locked -p xtask -- sdk-full --source OPENROUTER_ROOT --out NEW_REPORT [--editor-host-tools PINNED_TOOLS_JSON] [--functional-only | --performance-plan PINNED_PLAN_JSON]\n       cargo run --locked -p xtask -- sdk-full --list-stages [--functional-only]\n       cargo run --locked -p xtask -- sdk-full --check-syntax\nStrict by default. Functional-only success retains numerical-pending status, not whole-plan completion. Both real editor-host scenarios require explicit tool pins. See docs/SDK-FULL-EXIT.md.\n";

#[derive(Clone, Copy, Serialize)]
struct Target {
    language: &'static str,
    backend: &'static str,
    package_name: &'static str,
    import_name: Option<&'static str>,
    manifest: &'static str,
    docs: &'static str,
    toolchain_tiers: &'static [&'static str],
}

// These are real native identities, not one spelling forced on every ecosystem.
// Java/Kotlin use Maven coordinates; PHP uses vendor/package; Dart/C++ identifiers.
#[rustfmt::skip]
const TARGETS: &[Target] = &[
    Target { language: "typescript", backend: "typescript-http", package_name: "@suspect-fixtures/sdk-full", import_name: None, manifest: "package.json", docs: "docs/SDK-TYPESCRIPT-HTTP.md", toolchain_tiers: &["node22-ts55", "node24-ts59"] },
    Target { language: "rust", backend: "rust-http", package_name: "sdk-full", import_name: None, manifest: "Cargo.toml", docs: "docs/SDK-RUST-HTTP.md", toolchain_tiers: &["1.88.0", "stable"] },
    Target { language: "python", backend: "python-http", package_name: "sdk-full", import_name: Some("sdk_full"), manifest: "pyproject.toml", docs: "docs/SDK-PYTHON-HTTP.md", toolchain_tiers: &["3.11", "3.14"] },
    Target { language: "go", backend: "go-http", package_name: "example.com/sdk-full", import_name: None, manifest: "go.mod", docs: "docs/SDK-GO-HTTP.md", toolchain_tiers: &["1.23.12", "1.27.1"] },
    Target { language: "swift", backend: "swift-http", package_name: "SdkFull", import_name: Some("SdkFull"), manifest: "Package.swift", docs: "docs/SDK-SWIFT.md", toolchain_tiers: &["6.0.3-sdk15.4", "6.3.3-sdk26.5"] },
    Target { language: "java", backend: "java-http", package_name: "com.example.generated:sdk-full", import_name: Some("com.example.generated"), manifest: "pom.xml", docs: "docs/SDK-JAVA.md", toolchain_tiers: &["21.0.12.1", "25.0.4.1"] },
    Target { language: "csharp", backend: "csharp-http", package_name: "Suspect.SdkFull", import_name: Some("Suspect.SdkFull"), manifest: "Suspect.csproj", docs: "docs/SDK-CSHARP.md", toolchain_tiers: &["8.0.424-net8.0", "10.0.400-net10.0"] },
    Target { language: "kotlin", backend: "kotlin-http", package_name: "com.example:sdk-full", import_name: Some("example.sdk"), manifest: "pom.xml", docs: "docs/SDK-KOTLIN.md", toolchain_tiers: &["2.4.20-jdk21", "2.4.20-jdk25"] },
    Target { language: "ruby", backend: "ruby-http", package_name: "sdk-full", import_name: Some("SdkFull"), manifest: "sdk-full.gemspec", docs: "docs/SDK-RUBY.md", toolchain_tiers: &["3.3.12", "4.0.6"] },
    Target { language: "php", backend: "php-http", package_name: "example/sdk-full", import_name: Some("Example\\SdkFull"), manifest: "composer.json", docs: "docs/SDK-PHP.md", toolchain_tiers: &["8.3.32", "8.5.8"] },
    Target { language: "dart", backend: "dart-http", package_name: "sdk_full", import_name: None, manifest: "pubspec.yaml", docs: "docs/SDK-DART.md", toolchain_tiers: &["3.9.4", "3.13.3"] },
    Target { language: "cpp", backend: "cpp-http", package_name: "sdk_full", import_name: Some("sdk_full"), manifest: "CMakeLists.txt", docs: "docs/SDK-CPP.md", toolchain_tiers: &["apple-clang21-cpp20-libcurl8.7.1"] },
];

#[derive(Debug)]
struct Args {
    source: PathBuf,
    out: PathBuf,
    functional_only: bool,
    performance_plan: Option<PathBuf>,
    editor_host_tools: Option<PathBuf>,
}

impl Args {
    fn parse(raw: &[String]) -> Result<Self> {
        let mut args = raw.iter();
        let mut values = BTreeMap::new();
        let mut functional_only = false;
        while let Some(key) = args.next() {
            if key == "--functional-only" {
                ensure!(!functional_only, "duplicate --functional-only");
                functional_only = true;
                continue;
            }
            ensure!(
                [
                    "--source",
                    "--out",
                    "--performance-plan",
                    "--editor-host-tools"
                ]
                .contains(&key.as_str()),
                "unknown option {key}\n{USAGE}"
            );
            let value = args
                .next()
                .with_context(|| format!("{key} requires a value"))?;
            ensure!(
                !value.starts_with("--") && !value.is_empty(),
                "{key} requires a path"
            );
            ensure!(
                values.insert(key.as_str(), PathBuf::from(value)).is_none(),
                "duplicate {key}"
            );
        }
        let performance_plan = values.remove("--performance-plan");
        let editor_host_tools = values.remove("--editor-host-tools");
        ensure!(
            !functional_only || performance_plan.is_none(),
            "--functional-only and --performance-plan are mutually exclusive"
        );
        Ok(Self {
            source: values
                .remove("--source")
                .with_context(|| format!("--source is required\n{USAGE}"))?,
            out: values
                .remove("--out")
                .with_context(|| format!("--out is required\n{USAGE}"))?,
            functional_only,
            performance_plan,
            editor_host_tools,
        })
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Pin {
    path: PathBuf,
    sha256: String,
}

// No caller-selected executables, stages, environment, assertions or claim pointers.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PerformancePlan {
    format: String,
    evidence: Pin,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EditorTools {
    format: String,
    vscode: EditorCode,
    tools: EditorPackages,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EditorCode {
    executable: PathBuf,
    executable_sha256: String,
    archive: PathBuf,
    archive_sha256: String,
    version: String,
    commit: String,
    platform: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EditorPackages {
    directory: PathBuf,
    lock_sha256: String,
}

const EDITOR_CLAIMS: &[&str] = &[
    "inputPinsVerified",
    "backendInventoryExact",
    "vsixMatchesSource",
    "nativeChecksComplete",
    "screenshotsVerified",
    "hostExitedZero",
    "inputsUnchanged",
    "ownedOutputUnchanged",
];
const EDITOR_STARTUP_CHECKS: &[&str] = &[
    "installed VSIX activates in the real desktop extension host and registers production commands",
    "renderer automation attaches to the isolated native VS Code workbench",
    "packaged language-client startup uses the exact canonical stdio argv and keeps one LSP process alive",
];
const EDITOR_LIFECYCLE_CHECKS: &[&str] = &[
    "native profile discovery picker displays exactly the pinned CLI inventory and cancellation launches no SDK writer",
    "production watch opens a native read-only disk/generated diff with lexical config/source/output identities",
    "saved referenced-source changes refresh the open native diff using one canonical watch process",
    "saved A→B→A source reverts refresh the same native virtual documents without disk writes",
    "removing a target renders an empty desired side while preserving its owned disk snapshot",
    "real planning errors invalidate both native diff documents and recover in the same watch process",
    "production stop terminates the real watch and invalidates already-open native documents",
    "native output-directory dialog cancellation leaves no session process or SDK writes",
    "native progress cancellation terminates an in-flight real CLI preview",
    "actual profile/package/selector dialogs generate and open the selected native package README",
    "a final live native watch is handed to the real extension-host shutdown lifecycle",
    "native host shutdown terminates the final canonical watch process",
];
const EDITOR_COMMAND_CHECKS: &[&str] = &[
    "registered one-shot Preview resolves configured roots and opens a complete native read-only diff",
    "registered Show Latest opens another artifact from the completed native preview with no CLI process",
    "registered Check SDK Drift reports current through the native UI without opening a diff or writing",
    "registered Check SDK Drift exposes saved-source drift in the native UI while preserving owned disk bytes",
];
const EDITOR_HARNESS_FILES: &[&str] = &[
    "run.cjs",
    "runner.cjs",
    "suite.cjs",
    "contract.cjs",
    "pins.schema.json",
];

fn editor_checks(scenario: &str) -> Vec<String> {
    EDITOR_STARTUP_CHECKS
        .iter()
        .chain(if scenario == "lifecycle" {
            EDITOR_LIFECYCLE_CHECKS
        } else {
            EDITOR_COMMAND_CHECKS
        })
        .map(|name| (*name).to_owned())
        .collect()
}

fn editor_screenshots(scenario: &str) -> Vec<String> {
    strings(if scenario == "lifecycle" {
        &[
            "01-native-activation.png",
            "02-native-profile-picker.png",
            "03-native-readonly-diff-A.png",
            "04-native-live-diff-B.png",
            "05-native-removed-diff.png",
            "06-native-planning-error.png",
            "07-native-recovered-diff.png",
            "08-native-progress-cancellation.png",
            "09-native-generated-python-readme.png",
            "10-native-watch-before-shutdown.png",
        ]
    } else {
        &[
            "01-native-activation.png",
            "commands-01-one-shot-preview.png",
            "commands-02-current-check.png",
            "commands-03-drift-check.png",
        ]
    })
}

fn editor_contract_inventory() -> Value {
    json!({"reportFormat":"suspect.editor.native-host.run.v2","mode":"run","profiles":TARGETS.iter().map(|target| target.backend).collect::<Vec<_>>(),"requiredClaims":EDITOR_CLAIMS,"checks":{"lifecycle":editor_checks("lifecycle"),"commands":editor_checks("commands")},"screenshots":{"lifecycle":editor_screenshots("lifecycle"),"commands":editor_screenshots("commands")},"toolsInputFormat":"suspect.sdk.full.editor-tools.v1","sourceIdentity":"contract.cjs sourceIdentity after compilation","callerSelectableModes":false})
}

fn editor_native_stages() -> Vec<Stage> {
    let mut identity = command(
        "editor-native-source-identity",
        "{node22}",
        &[
            "-e",
            "const c=require(process.argv[1]); c.sourceIdentity(process.argv[2]).then(source=>process.stdout.write(JSON.stringify({source,formats:c.FORMATS,checks:{lifecycle:c.requiredChecks('lifecycle'),commands:c.requiredChecks('commands')},screenshots:{lifecycle:c.requiredScreenshots('lifecycle'),commands:c.requiredScreenshots('commands')}}))).catch(e=>{console.error(e);process.exitCode=1;});",
            "{work}/editor/test/native-host/contract.cjs",
            "{work}/editor",
        ],
        "{work}/editor",
    );
    identity.criterion = Criterion::Json {
        report: None,
        assertions: BTreeMap::from([
            ("/source/id".into(), json!("suspect.suspect-vscode")),
            (
                "/formats/report".into(),
                json!("suspect.editor.native-host.run.v2"),
            ),
        ]),
    };
    let mut tests = command(
        "editor-native-contract-tests",
        "{node22}",
        &[
            "--test",
            "--test-reporter=tap",
            "test/native-host/contract.test.cjs",
        ],
        "{work}/editor",
    );
    tests.criterion = Criterion::NodeTests;
    let mut steps = vec![
        tests,
        identity,
        command(
            "editor-native-production-install",
            "{npm}",
            &[
                "ci",
                "--offline",
                "--omit=dev",
                "--ignore-scripts",
                "--no-audit",
                "--no-fund",
            ],
            "{work}/editor-vsix",
        ),
        command(
            "editor-native-package-vsix",
            "{node22}",
            &[
                "{editor-host-tools}/node_modules/@vscode/vsce/vsce",
                "package",
                "--out",
                "{out}/editor-native/suspect.vsix",
                "--allow-missing-repository",
                "--skip-license",
                "--no-rewrite-relative-links",
            ],
            "{work}/editor-vsix",
        ),
    ];
    for scenario in ["lifecycle", "commands"] {
        let mut item = command(
            &format!("editor-native-{scenario}"),
            "{node22}",
            &["{work}/editor/test/native-host/run.cjs"],
            "{work}/editor",
        );
        item.environment = BTreeMap::from([
            ("SUSPECT_TEST_BINARY".into(), "{out}/bin/suspect".into()),
            (
                "SUSPECT_NATIVE_PINS".into(),
                "{out}/editor-native/pins.json".into(),
            ),
            (
                "SUSPECT_NATIVE_OUT".into(),
                format!("{{out}}/editor-native/{scenario}"),
            ),
            (
                "SUSPECT_NATIVE_SCRATCH".into(),
                "{editor-host-scratch}".into(),
            ),
            ("SUSPECT_NATIVE_SCENARIO".into(), scenario.into()),
            ("SUSPECT_NATIVE_MODE".into(), "run".into()),
        ]);
        item.criterion = Criterion::Json {
            report: Some(format!("{{out}}/editor-native/{scenario}/report.json")),
            assertions: BTreeMap::from([
                ("/format".into(), json!("suspect.editor.native-host.run.v2")),
                ("/status".into(), json!("passed")),
                ("/mode".into(), json!("run")),
                ("/scenario".into(), json!(scenario)),
                ("/exitCode".into(), json!(0)),
            ]),
        };
        steps.push(item);
    }
    steps
}

fn editor_environment(environment: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    environment
        .iter()
        .filter(|(name, _)| !name.starts_with("SUSPECT_NATIVE_"))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

fn load_editor_tools(run: &mut Run, path: Option<&Path>) -> Result<Value> {
    let path =
        path.context("--editor-host-tools is required for both real installed-host scenarios")?;
    let bytes = fs::read(path)?;
    create(&run.out.join("editor-native/tools-input.json"))?.write_all(&bytes)?;
    let tools: EditorTools = serde_json::from_slice(&bytes)?;
    ensure!(
        tools.format == "suspect.sdk.full.editor-tools.v1",
        "unsupported editor tool-pin format"
    );
    ensure!(
        tools.vscode.executable.is_absolute()
            && tools.vscode.archive.is_absolute()
            && tools.tools.directory.is_absolute(),
        "editor tools must use absolute paths"
    );
    ensure!(
        tools.vscode.commit.len() == 40
            && tools
                .vscode
                .commit
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "editor app needs an exact lowercase commit"
    );
    let platform = format!(
        "{}-{}",
        if cfg!(target_os = "macos") {
            "darwin"
        } else {
            std::env::consts::OS
        },
        if cfg!(target_arch = "aarch64") {
            "arm64"
        } else if cfg!(target_arch = "x86_64") {
            "x64"
        } else {
            std::env::consts::ARCH
        }
    );
    ensure!(
        tools.vscode.platform == platform && tools.vscode.version.split('.').count() == 3,
        "editor app platform/version differs from this host"
    );
    for (file, expected) in [
        (&tools.vscode.executable, &tools.vscode.executable_sha256),
        (&tools.vscode.archive, &tools.vscode.archive_sha256),
        (
            &tools.tools.directory.join("package-lock.json"),
            &tools.tools.lock_sha256,
        ),
    ] {
        ensure!(
            sealed::digest_text(expected) && hash(file)? == *expected,
            "editor tool pin differs: {}",
            file.display()
        );
        run.remember(file)?;
    }
    let scratch = std::env::temp_dir().join("opencode").canonicalize()?;
    sealed::require_external_root(&scratch)?;
    ensure!(
        !cfg!(target_os = "macos")
            || scratch
                .join("n-XXXXXX/u")
                .as_os_str()
                .as_encoded_bytes()
                .len()
                + 21
                <= 103,
        "editor-host scratch parent exceeds macOS IPC limit"
    );
    run.replacements.insert(
        "{editor-host-scratch}".into(),
        scratch.display().to_string(),
    );
    run.replacements.insert(
        "{editor-host-tools}".into(),
        tools.tools.directory.display().to_string(),
    );
    let executable = tools.vscode.executable.canonicalize()?;
    let app_root = if cfg!(target_os = "macos") {
        executable
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
    } else {
        executable.parent()
    }
    .context("editor application root")?;
    let payloads = vec![
        external_tree(app_root)?,
        external_tree(&tools.tools.directory.join("node_modules"))?,
    ];
    write_json(&run.out.join("editor-native/tool-payloads.json"), &payloads)?;
    run.remember(path)?;
    Ok(
        json!({"input":"editor-native/tools-input.json","app":tools.vscode,"tools":tools.tools,"payloads":"editor-native/tool-payloads.json","scratch":scratch,"callerControlsCliOrSource":false}),
    )
}

fn stage_editor_package(run: &Run) -> Result<Value> {
    ensure!(met(run, "editor-compile"), "editor compilation failed");
    let source = run.work.join("editor");
    let staged = run.work.join("editor-vsix");
    fs::create_dir(&staged)?;
    let names = tree_names(&source)?.into_iter().filter(|name| {
        ["package.json", "package-lock.json"].contains(&name.as_str())
            || name.starts_with("dist/")
            || name.starts_with("media/")
    });
    let files = inventory(&source, names)?;
    copy_inventory(&source, &staged, &files)?;
    create(&staged.join("README.md"))?
        .write_all(b"# Suspect\n\nCanonical SDK preview, watch, drift checks and generation.\n")?;
    fs::create_dir_all(run.out.join("editor-native"))?;
    Ok(
        json!({"staging":staged,"source":source,"files":files,"productionDependencies":"fresh offline npm ci from the snapshotted lock"}),
    )
}

fn prepare_editor_pins(run: &mut Run) -> Result<Value> {
    for id in [
        "editor-host-tools",
        "editor-native-source-identity",
        "editor-native-production-install",
        "editor-native-package-vsix",
    ] {
        ensure!(met(run, id), "required editor preparation failed: {id}");
    }
    let tools: EditorTools =
        serde_json::from_slice(&fs::read(run.out.join("editor-native/tools-input.json"))?)?;
    let identity: Value = serde_json::from_slice(&fs::read(
        run.out
            .join("logs/editor-native-source-identity.stdout.log"),
    )?)?;
    for scenario in ["lifecycle", "commands"] {
        ensure!(
            identity["checks"][scenario] == json!(editor_checks(scenario))
                && identity["screenshots"][scenario] == json!(editor_screenshots(scenario)),
            "editor host contract changed; maintain exact check/screenshot inventory"
        );
    }
    let source = &identity["source"];
    ensure!(
        source["directory"]
            == run
                .work
                .join("editor")
                .to_str()
                .context("editor UTF-8 path")?
            && source["id"] == "suspect.suspect-vscode",
        "editor source identity escaped the compiled execution copy"
    );
    ensure!(
        source["sha256"].as_str().is_some_and(sealed::digest_text),
        "missing compiled editor source pin"
    );
    for (name, digest) in source["hashes"]
        .as_object()
        .context("editor source hashes")?
    {
        let path = run.work.join("editor").join(relative(name)?);
        ensure!(
            digest == &json!(hash(&path)?),
            "editor source changed after compilation: {name}"
        );
        run.remember(&path)?;
    }
    let vsix = run.out.join("editor-native/suspect.vsix");
    nonempty(&vsix)?;
    let pins = json!({"format":"suspect.editor.native-host.pins.v1","cli":{"sha256":hash(&run.out.join("bin/suspect"))?,"expectedProfiles":TARGETS.iter().map(|target| target.backend).collect::<Vec<_>>()},"vscode":tools.vscode,"tools":tools.tools,"extension":{"sourceDirectory":run.work.join("editor"),"sourceSha256":source["sha256"],"vsix":{"path":vsix,"sha256":hash(&vsix)?}}});
    write_json(&run.out.join("editor-native/source-identity.json"), source)?;
    write_json(&run.out.join("editor-native/pins.json"), &pins)?;
    run.remember(&vsix)?;
    run.remember(&run.out.join("editor-native/pins.json"))?;
    Ok(
        json!({"pins":"editor-native/pins.json","sha256":hash(&run.out.join("editor-native/pins.json"))?,"source":"editor-native/source-identity.json","sourceFingerprint":run.provenance["sourceFingerprint"],"scenarios":["lifecycle","commands"],"mode":"run"}),
    )
}

fn exact_profile_ids(value: &Value) -> Result<()> {
    let ids = value.as_array().context("native host expected profiles")?;
    let actual = ids
        .iter()
        .map(|id| id.as_str().context("profile ID must be a string"))
        .collect::<Result<BTreeSet<_>>>()?;
    ensure!(
        ids.len() == TARGETS.len()
            && actual == TARGETS.iter().map(|target| target.backend).collect(),
        "native host must use all twelve expected profile IDs exactly once"
    );
    Ok(())
}

fn editor_report_contract(value: &Value, scenario: &str, pins: &Value) -> Result<()> {
    ensure!(
        ["lifecycle", "commands"].contains(&scenario),
        "unknown native editor scenario"
    );
    ensure!(
        value["format"] == "suspect.editor.native-host.run.v2"
            && value["mode"] == "run"
            && value["scenario"] == scenario
            && value["status"] == "passed"
            && value["phase"] == "complete"
            && value["exitCode"] == 0,
        "missing/partial/legacy/probe native editor verdict"
    );
    ensure!(
        value["runId"].as_str().is_some_and(|id| !id.is_empty()),
        "missing native host attempt ID"
    );
    ensure!(
        value["hostExit"]["code"] == 0
            && value["hostExit"]["timedOut"] == false
            && value["hostExit"]["signal"].is_null(),
        "native editor host did not exit cleanly"
    );
    let required = editor_checks(scenario);
    ensure!(
        value["requiredNativeChecks"] == json!(required),
        "native editor check inventory changed"
    );
    let checks = value["checks"].as_array().context("native editor checks")?;
    ensure!(
        checks.len() == required.len(),
        "native editor check count differs"
    );
    for (check, name) in checks.iter().zip(&required) {
        ensure!(
            check["name"] == *name && check["status"] == "passed",
            "native editor check missing, reordered, relabeled or skipped: {name}"
        );
    }
    ensure!(
        value["requiredClaims"] == json!(EDITOR_CLAIMS),
        "native editor claim inventory differs"
    );
    let claims = value["claims"]
        .as_object()
        .context("native editor claims")?;
    ensure!(
        claims.len() == EDITOR_CLAIMS.len()
            && EDITOR_CLAIMS
                .iter()
                .all(|name| claims.get(*name) == Some(&json!(true))),
        "native editor has missing/nonboolean/probe claims"
    );
    ensure!(
        value["inputs"]["pins"] == *pins,
        "native editor pins differ from this fresh full-runner invocation"
    );
    exact_profile_ids(&pins["cli"]["expectedProfiles"])?;
    exact_profile_ids(&value["inputs"]["cli"]["expectedProfiles"])?;
    verify_profile_inventory(&value["inventory"])?;
    ensure!(
        value["requiredScreenshots"] == json!(editor_screenshots(scenario)),
        "native editor screenshot contract differs"
    );
    ensure!(
        value["inputs"]["cli"]["sha256"] == pins["cli"]["sha256"]
            && value["inputs"]["extension"]["sha256"] == pins["extension"]["sourceSha256"],
        "native editor CLI/source identity differs"
    );
    ensure!(
        value["inputs"]["vscode"]["executable"]["sha256"] == pins["vscode"]["executableSha256"]
            && value["inputs"]["vscode"]["archive"]["sha256"] == pins["vscode"]["archiveSha256"]
            && value["inputs"]["tools"]["lock"]["sha256"] == pins["tools"]["lockSha256"],
        "native editor app/tool hashes differ"
    );
    for key in ["version", "commit", "platform"] {
        ensure!(
            value["inputs"]["vscode"][key] == pins["vscode"][key],
            "native editor app {key} differs"
        );
    }
    ensure!(
        value["vsix"]["mode"] == "supplied"
            && value["vsix"]["sha256"] == pins["extension"]["vsix"]["sha256"],
        "native editor did not use the freshly packaged, source-matching VSIX"
    );
    ensure!(
        value["baseline"]["owner"] == "native-host-fixture"
            && value["baseline"]["files"]
                .as_u64()
                .is_some_and(|count| count > 0),
        "native editor omitted its canonical owned-output baseline"
    );
    Ok(())
}

fn confined_editor_file(root: &Path, value: &Value) -> Result<PathBuf> {
    let path = PathBuf::from(value.as_str().context("native editor evidence path")?);
    ensure!(
        path.is_absolute(),
        "native editor evidence must use absolute paths"
    );
    let metadata = fs::symlink_metadata(&path)?;
    ensure!(
        metadata.is_file()
            && !metadata.file_type().is_symlink()
            && path.canonicalize()?.starts_with(root.canonicalize()?),
        "native editor evidence escaped its fresh report or is not a file"
    );
    Ok(path)
}

fn editor_evidence(run: &mut Run, scenario: &str) -> Result<Value> {
    ensure!(
        met(run, &format!("editor-native-{scenario}")),
        "native editor process failed or produced no report"
    );
    let root = run.out.join("editor-native").join(scenario);
    let report_path = root.join("report.json");
    let report: Value = serde_json::from_slice(&fs::read(&report_path)?)?;
    let pins_path = run.out.join("editor-native/pins.json");
    let pins: Value = serde_json::from_slice(&fs::read(&pins_path)?)?;
    editor_report_contract(&report, scenario, &pins)?;
    ensure!(
        report["inputs"]["cli"]["path"]
            == run.out.join("bin/suspect").to_str().context("CLI UTF-8")?
            && report["inputs"]["pinsFile"]["path"] == pins_path.to_str().context("pins UTF-8")?
            && report["inputs"]["pinsFile"]["sha256"] == hash(&pins_path)?,
        "native editor did not consume this invocation's frozen CLI/pins"
    );
    let source: Value = serde_json::from_slice(&fs::read(
        run.out.join("editor-native/source-identity.json"),
    )?)?;
    for key in ["directory", "sha256", "hashes", "runtimeHashes", "manifest"] {
        ensure!(
            report["inputs"]["extension"][key] == source[key],
            "native editor compiled-source field differs: {key}"
        );
    }
    let harness = EDITOR_HARNESS_FILES
        .iter()
        .map(|name| {
            Ok((
                (*name).to_owned(),
                hash(&run.work.join("editor/test/native-host").join(name))?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    ensure!(
        report["harnessHashes"] == json!(harness),
        "native editor harness differs from the actual source copy"
    );
    let native_path = confined_editor_file(&root, &report["native"]["path"])?;
    ensure!(
        report["native"]["sha256"] == hash(&native_path)?,
        "native editor raw observation changed"
    );
    let native: Value = serde_json::from_slice(&fs::read(&native_path)?)?;
    ensure!(
        native["format"] == "suspect.editor.native-host.observations.v2"
            && native["runId"] == report["runId"]
            && native["mode"] == "run"
            && native["scenario"] == scenario
            && native["status"] == "passed"
            && native["pinsSha256"] == hash(&pins_path)?
            && native["checks"] == report["checks"]
            && native["screenshots"] == report["screenshots"],
        "native editor raw observations do not match its verdict"
    );
    exact_profile_ids(&native["cli"]["advertisedProfiles"])?;
    ensure!(
        native["cli"]["sha256"] == pins["cli"]["sha256"]
            && native["cli"]["executable"] == report["inputs"]["cli"]["path"]
            && native["vscodeVersion"] == pins["vscode"]["version"]
            && native["appRoot"] == report["inputs"]["vscode"]["appRoot"],
        "observed native host used a different CLI/app or backend inventory"
    );
    let screenshots = report["screenshots"]
        .as_array()
        .context("native editor screenshots")?;
    let mut names = BTreeSet::new();
    ensure!(
        screenshots.len() == editor_screenshots(scenario).len(),
        "native editor screenshot count differs"
    );
    for image in screenshots {
        let path = confined_editor_file(&root, &image["file"])?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .context("screenshot filename")?;
        ensure!(
            names.insert(name.to_owned()) && image["sha256"] == hash(&path)?,
            "duplicate/changed native screenshot"
        );
        ensure!(
            image["source"] == "official VS Code Electron renderer via CDP"
                && image["dom"]["width"].as_f64().is_some_and(|n| n > 0.0)
                && image["dom"]["height"].as_f64().is_some_and(|n| n > 0.0),
            "screenshot has no real renderer viewport provenance"
        );
        let bytes = fs::read(&path)?;
        ensure!(
            bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            "native screenshot is not PNG evidence"
        );
        run.remember(&path)?;
    }
    ensure!(
        names == editor_screenshots(scenario).into_iter().collect(),
        "missing/renamed native editor screenshot"
    );
    let before = confined_editor_file(&root, &report["baseline"]["before"])?;
    let after = confined_editor_file(&root, &report["baseline"]["after"])?;
    let before: Value = serde_json::from_slice(&fs::read(&before)?)?;
    let after: Value = serde_json::from_slice(&fs::read(&after)?)?;
    ensure!(
        before == after
            && before.as_object().is_some_and(|files| !files.is_empty()
                && Some(files.len() as u64) == report["baseline"]["files"].as_u64()),
        "native editor owned-output bytes/metadata changed or inventory is incomplete"
    );
    for (name, record) in before.as_object().context("owned file census")? {
        relative(name)?;
        ensure!(
            record["sha256"].as_str().is_some_and(sealed::digest_text)
                && record["size"].as_u64().is_some()
                && record["mtimeMs"].is_number()
                && record["ino"].is_number(),
            "incomplete native owned-output record: {name}"
        );
    }
    let setup: Value = serde_json::from_slice(&fs::read(root.join("setup.json"))?)?;
    let owned = PathBuf::from(
        setup["out"]
            .as_str()
            .context("actual native owned-output path")?,
    );
    let isolation = PathBuf::from(
        report["isolation"]["root"]
            .as_str()
            .context("native isolated root")?,
    );
    sealed::require_external_root(&isolation)?;
    ensure!(
        isolation.canonicalize()?.starts_with(Path::new(
            run.replacements
                .get("{editor-host-scratch}")
                .context("native scratch identity")?
        )) && owned.canonicalize()?.starts_with(isolation.canonicalize()?),
        "native owned output escaped the explicit short scratch root"
    );
    let actual = tree_names(&owned)?.into_iter().collect::<BTreeSet<_>>();
    ensure!(
        actual == before.as_object().unwrap().keys().cloned().collect()
            && actual.contains(".suspect-artifacts.json")
            && actual.contains("typescript/models.ts"),
        "native editor baseline omitted owned files"
    );
    for name in actual {
        let path = owned.join(relative(&name)?);
        ensure!(
            before[&name]["sha256"] == hash(&path)?
                && before[&name]["size"] == fs::metadata(&path)?.len(),
            "native editor actual owned bytes differ: {name}"
        );
    }
    let owned_snapshot = copy_evidence_tree(&owned, &root.join("owned-output"))?;
    let commands = report["commands"]
        .as_array()
        .context("native subprocess commands")?;
    for label in [
        "cli-inventory",
        "vscode-version",
        "install-vsix",
        "installed-extensions",
        "owned-sdk-baseline",
    ] {
        ensure!(
            commands
                .iter()
                .filter(|command| command["label"] == label)
                .count()
                == 1,
            "missing/duplicate native subprocess {label}"
        );
    }
    for command in commands {
        let executable = Path::new(
            command["executable"]
                .as_str()
                .context("native executable")?,
        );
        ensure!(
            executable.is_absolute()
                && command["exitCode"] == 0
                && command["shell"] == false
                && command["identity"]["sha256"] == hash(executable)?,
            "native editor subprocess executable/status differs"
        );
        run.remember(executable)?;
    }
    let vsix = confined_editor_file(&root, &report["vsix"]["path"])?;
    ensure!(
        hash(&vsix)? == pins["extension"]["vsix"]["sha256"],
        "installed scenario VSIX differs from current package"
    );
    run.remember(&native_path)?;
    run.remember(&report_path)?;
    Ok(
        json!({"report":report_path,"sha256":hash(&report_path)?,"runId":report["runId"],"scenario":scenario,"mode":"run","profiles":12,"checks":editor_checks(scenario).len(),"screenshots":names,"ownedFiles":report["baseline"]["files"],"ownedSnapshot":owned_snapshot,"sourceFingerprint":run.provenance["sourceFingerprint"],"sourceSha256":source["sha256"],"cliSha256":pins["cli"]["sha256"]}),
    )
}

fn editor_native_workflow(run: &mut Run) -> Result<()> {
    let staged = stage_editor_package(run);
    verification(run, "editor-native-staging", staged)?;
    let stages = editor_native_stages();
    let original = run.env.clone();
    run.env = editor_environment(&original);
    let result = (|| -> Result<()> {
        for item in &stages[..4] {
            if item.id == "editor-native-package-vsix" && !met(run, "editor-host-tools") {
                verification(
                    run,
                    &item.id,
                    Err(anyhow::anyhow!("editor tool pins unavailable")),
                )?;
            } else {
                run.command(item)?;
            }
        }
        let pins = prepare_editor_pins(run);
        verification(run, "editor-native-pins", pins)?;
        for (scenario, item) in ["lifecycle", "commands"].into_iter().zip(&stages[4..]) {
            if met(run, "editor-native-pins") {
                run.command(item)?;
            } else {
                verification(
                    run,
                    &item.id,
                    Err(anyhow::anyhow!(
                        "native editor source/tool/VSIX pins are incomplete"
                    )),
                )?;
            }
            let evidence = editor_evidence(run, scenario);
            verification(run, &format!("editor-native-{scenario}-evidence"), evidence)?;
        }
        let integrity = (|| -> Result<Value> {
            ensure!(
                met(run, "editor-native-lifecycle-evidence")
                    && met(run, "editor-native-commands-evidence"),
                "both full native editor scenarios are required"
            );
            let payloads: Vec<Value> = serde_json::from_slice(&fs::read(
                run.out.join("editor-native/tool-payloads.json"),
            )?)?;
            for payload in payloads {
                ensure!(
                    external_tree(Path::new(
                        payload["root"].as_str().context("editor tool tree")?
                    ))? == payload,
                    "editor app/tool payload changed"
                );
            }
            let left: Value = serde_json::from_slice(&fs::read(
                run.out.join("editor-native/lifecycle/report.json"),
            )?)?;
            let right: Value = serde_json::from_slice(&fs::read(
                run.out.join("editor-native/commands/report.json"),
            )?)?;
            ensure!(
                left["runId"] != right["runId"]
                    && left["isolation"]["root"] != right["isolation"]["root"],
                "native editor scenarios reused an attempt/profile"
            );
            Ok(
                json!({"scenarios":["lifecycle","commands"],"mode":"run","nativeProfiles":12,"separateAttempts":true,"toolPayloadsUnchanged":true}),
            )
        })();
        verification(run, "editor-native-integrity", integrity)
    })();
    run.env = original;
    result
}

fn command(id: &str, program: &str, args: &[&str], cwd: &str) -> Stage {
    stage(id, FULL, program, args, cwd)
}

fn verification(run: &mut Run, id: &str, result: Result<Value>) -> Result<()> {
    run.verification(id, &strings(FULL), result)
}

fn met(run: &Run, id: &str) -> bool {
    run.checks
        .iter()
        .any(|check| check["id"] == id && check["criterionMet"] == true)
}

fn suite(package: &str, name: &str, id: &str, env: BTreeMap<String, String>) -> Stage {
    let mut item = command(
        id,
        &format!("test:{package}/{name}"),
        &["--include-ignored", "--show-output", "--test-threads=1"],
        "{workspace}",
    );
    item.environment = env;
    item.criterion = Criterion::RustTests;
    item
}

#[rustfmt::skip]
fn native_environment(language: &str, tier: &str) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    let root = format!("{{work}}/native/{language}/{tier}");
    env.insert("TMPDIR".into(), format!("{root}/tmp"));
    env.insert("SUSPECT_TEST_TMPDIR".into(), format!("{root}/tmp"));
    match language {
        "java" => {
            env.insert("JAVA_HOME".into(), format!("{{jdk-{tier}}}"));
            env.insert("SUSPECT_JAVA_HOME".into(), format!("{{jdk-{tier}}}"));
        }
        "kotlin" => {
            env.insert("JAVA_HOME".into(), format!("{{jdk-{tier}}}"));
            env.insert("SUSPECT_KOTLIN_JAVA_HOME".into(), format!("{{jdk-{tier}}}"));
        }
        "ruby" => {
            env.insert("SUSPECT_RUBY_HOME".into(), format!("{{ruby-{tier}}}"));
            env.insert("SUSPECT_RUBY_GEMS".into(), format!("{{ruby-gems-{tier}}}"));
            env.insert("GEM_HOME".into(), format!("{{ruby-gems-{tier}}}"));
            env.insert("GEM_PATH".into(), format!("{{ruby-gems-{tier}}}:{{ruby-default-gems-{tier}}}"));
        }
        "php" => { env.insert("SUSPECT_PHP_BIN".into(), format!("{{php-{tier}}}")); }
        "dart" => {
            env.insert("SUSPECT_DART_BIN".into(), format!("{{dart-{tier}}}"));
            env.insert("SUSPECT_DART_GATE_ROOT".into(), format!("{root}/gates"));
            env.insert("SUSPECT_DART_REPO_ROOT".into(), "{workspace}".into());
        }
        "swift" => {
            env.extend(sealed::swift_environment(tier));
            env.insert("SUSPECT_SWIFT_PROTOCOL_ROOT".into(), format!("{root}/protocol"));
            env.insert("SUSPECT_SWIFT_V2_ROOT".into(), format!("{root}/v2"));
        }
        "rust" => {
            let toolchain = if tier == "floor" { "1.88.0" } else { "stable" };
            env.insert("RUSTUP_TOOLCHAIN".into(), toolchain.into());
            env.insert("SUSPECT_NATIVE_RUST_TOOLCHAIN".into(), toolchain.into());
            env.insert("SUSPECT_RUST_PROTOCOL_TARGET".into(), format!("{root}/cargo"));
            env.insert("SUSPECT_RUST_V2_TARGET".into(), format!("{root}/v2/cargo"));
        }
        "python" => { env.insert("SUSPECT_PYTHON_BIN".into(), format!("{{python-{tier}}}")); }
        "go" => {
            let toolchain = if tier == "floor" { "go1.23.12" } else { "local" };
            env.insert("GOTOOLCHAIN".into(), toolchain.into());
            env.insert("SUSPECT_GO_TOOLCHAIN".into(), toolchain.into());
        }
        "typescript" => {
            let node = if tier == "current" { "node24" } else { "node22" };
            let compiler = if tier == "floor" { "typescript-floor" } else { "typescript-docs" };
            env.insert("PATH".into(), format!("{{workspace}}/crates/suspect-codegen/tools/{compiler}/node_modules/.bin:{{{node}-bin}}:{{path}}"));
        }
        "cpp" => { env.insert("SDKROOT".into(), "{swift-sdk}".into()); }
        _ => {}
    }
    env
}

#[rustfmt::skip]
fn suites() -> Vec<Stage> {
    let mut result = sealed::suite_stages();
    // The old full-library native runs become exact, census-accounted selections
    // below. Host coverage runs once; native module gates get their real tiers.
    result.retain(|item| item.program != "test:suspect_codegen");
    for item in &mut result {
        let package = if item.id.starts_with("cli-") { "suspect-cli" } else { "suspect-codegen" };
        item.program = format!("test:{package}/{}", item.program.trim_start_matches("test:"));
        item.args = strings(&["--include-ignored", "--show-output", "--test-threads=1"]);
        item.milestones = strings(FULL);
    }
    for (package, names) in [
        ("suspect-ref", &["pinned_acquisition", "pinned_provider", "pinned_transport"][..]),
        ("suspect-ir", &["contract", "contract_readers", "contract_references", "pinned_contract", "scalar_fidelity", "http_contract", "contract_oas32", "contract_resources", "contract_dual_role_scope"][..]),
        ("suspect-schema", &["owned", "owned_dialects", "owned_normative", "owned_context_regressions", "numeric", "references", "resource_uris", "evaluation_budget", "owned_applicators", "owned_applicator_conformance", "owned_resources"][..]),
        ("suspect-validate", &["http_declarations", "schema_keywords", "scoped_references"][..]),
        ("suspect-cli", &["pinned_codegen", "pinned_validation", "schema_declarations", "validation", "sdk_protocol_options", "credential_env_codegen"][..]),
        ("suspect-codegen", &["http_protocol", "protocol_examples", "pinned_generation", "sdk_model_names", "python_quickstart", "typescript_codecs", "typescript_docs", "typescript_directional", "typescript_directional_adversarial", "typescript_intersections", "sdk_native_measurements", "sdk_generation_options", "typescript_protocol_compatibility", "sdk_compatibility_context", "ruby_compatibility_credentials", "kotlin_validation_integration", "kotlin_validation_resource_integration", "credential_env", "go_credential_env_canonical", "go_credential_env_factory_capture", "swift_credential_env_canonical"][..]),
    ] {
        for name in names {
            let mut item = suite(package, name, &format!("core-{package}-{name}"), BTreeMap::new());
            if *name == "pinned_transport" {
                // environment_child is invoked twice by the real parent tests,
                // with different hostile environments. Running it bare is invalid.
                item.args = strings(&["--show-output", "--test-threads=1"]);
                item.criterion = Criterion::Exit;
            }
            if *name == "go_credential_env_factory_capture" {
                item.environment.insert("SUSPECT_GO_FACTORY_CAPTURE_EVIDENCE".into(), "{work}/native/go/credential-env-factory-capture".into());
            }
            result.push(item);
        }
    }
    for tier in ["floor", "current"] {
        let mut python_null = native_environment("python", tier);
        python_null.insert("SUSPECT_PYTHON_TOOLS".into(), "{python-tools}".into());
        python_null.insert("SUSPECT_TEST_ARTIFACT_ROOT".into(), python_null["TMPDIR"].clone());
        result.push(suite("suspect-codegen", "python_null_models", &format!("{tier}-python_null_models"), python_null));
        let mut dialect = native_environment("swift", tier);
        dialect.extend(native_environment("python", tier));
        dialect.extend(native_environment("rust", tier));
        dialect.insert("PATH".into(), format!("{{go-{tier}-bin}}:{{swift-{tier}-bin}}:{{path}}"));
        result.push(suite("suspect-codegen", "sdk_dialect_models", &format!("{tier}-sdk_dialect_models"), dialect));
        for (language, name) in [
            ("go", "go_schema_v2"), ("ruby", "ruby_schema_v2"),
            ("ruby", "ruby_schema_v3"), ("ruby", "ruby_document_servers"),
            ("ruby", "ruby_credential_env"),
            ("rust", "rust_validation_v2"),
            ("swift", "swift_validation_v2"), ("kotlin", "kotlin_validation_v2"),
            ("kotlin", "kotlin_validation_v3"), ("kotlin", "kotlin_protocol_documents"),
            ("java", "java_schema_v2"),
            ("java", "java_schema_v3"),
            ("java", "java_aggregate_examples"),
            ("dart", "dart_credential_env"),
        ] {
            let mut env = native_environment(language, tier);
            if name == "go_schema_v2" {
                env.insert("SUSPECT_SPHINX_PYTHON".into(), "{python-tools}".into());
            }
            if name == "dart_credential_env" {
                env.insert("OPENROUTER_WEB_ROOT".into(), "{out}/inputs".into());
                env.insert("PATH".into(), "{node22-bin}:{path}".into());
            }
            if name == "ruby_credential_env" {
                env.insert("OPENROUTER_WEB_ROOT".into(), "{out}/inputs".into());
                for key in ["SUSPECT_RUBY_GENERATOR_CANARY", "RUBY_ENV_BEARER", "OPENROUTER_API_KEY"] {
                    env.insert(key.into(), "RubyGeneratorCanaryMustNotBeEmitted0123456789".into());
                }
            }
            result.push(suite("suspect-codegen", name, &format!("{tier}-{name}"), env));
        }
        for name in ["go_schema_v3", "go_examples_aggregate"] {
            let mut env = native_environment("go", tier);
            let toolchain = if tier == "floor" { "go1.23.12" } else { "go1.27.1" };
            env.insert("SUSPECT_GO_TOOLCHAIN".into(), toolchain.into());
            env.insert("GOTOOLCHAIN".into(), toolchain.into());
            env.insert("SUSPECT_SPHINX_PYTHON".into(), "{python-tools}".into());
            result.push(suite("suspect-codegen", name, &format!("{tier}-{name}"), env));
        }
        for (name, selector, directory) in [
            ("rust_validation_v3", "SUSPECT_RUST_V3_TARGET", "v3/runtime"),
            ("rust_protocol_v3", "SUSPECT_RUST_V3_HTTP_TARGET", "v3/sdk"),
            ("rust_protocol_resources", "SUSPECT_RUST_RESOURCES_TARGET", "physical-servers"),
        ] {
            let mut env = native_environment("rust", tier);
            env.insert(selector.into(), format!("{{work}}/native/rust/{tier}/{directory}/cargo"));
            result.push(suite("suspect-codegen", name, &format!("{tier}-{name}"), env));
        }
        for language in ["java", "kotlin", "ruby", "php", "dart"] {
            result.push(suite("suspect-codegen", &format!("{language}_sdk"), &format!("{tier}-{language}_sdk"), native_environment(language, tier)));
        }
        for language in ["go", "rust", "swift", "java", "kotlin", "php"] {
            let mut env = native_environment(language, tier);
            env.insert("OPENROUTER_WEB_ROOT".into(), "{out}/inputs".into());
            match language {
                "go" => {
                    let toolchain = if tier == "floor" { "go1.23.12" } else { "go1.27.1" };
                    env.insert("SUSPECT_GO_TOOLCHAIN".into(), toolchain.into());
                    env.insert("GOTOOLCHAIN".into(), toolchain.into());
                    env.insert("SUSPECT_SPHINX_PYTHON".into(), "{python-tools}".into());
                    env.insert("SUSPECT_GO_CREDENTIAL_ENV_EVIDENCE".into(), format!("{{work}}/native/go/{tier}/credential-env"));
                }
                "rust" => { env.insert("SUSPECT_RUST_CREDENTIAL_ENV_TARGET".into(), format!("{{work}}/native/rust/{tier}/credential-env/cargo")); }
                "swift" => { env.insert("SUSPECT_SWIFT_CREDENTIAL_ENV_ROOT".into(), format!("{{work}}/native/swift/{tier}/credential-env")); }
                "java" => {
                    env.insert("SUSPECT_MAVEN_BIN".into(), "{maven}".into());
                    env.insert("SUSPECT_OPENROUTER_OPENAPI".into(), "{out}/inputs/projects/docs/openapi/openapi.yaml".into());
                }
                "kotlin" => {
                    env.insert("SUSPECT_KOTLIN_MAVEN".into(), "{maven}".into());
                    env.insert("SUSPECT_KOTLIN_MAVEN_REPO".into(), "{workspace}/target/sdk-kotlin-maven".into());
                }
                "php" => {
                    env.insert("SUSPECT_COMPOSER_PHAR".into(), "{composer}".into());
                    env.insert("SUSPECT_PHPSTAN_PHAR".into(), "{phpstan}".into());
                }
                _ => unreachable!(),
            }
            result.push(suite("suspect-codegen", &format!("{language}_credential_env"), &format!("{tier}-{language}_credential_env"), env));
        }
        // Ruby's complete base and expanded protocol witnesses share ruby_sdk;
        // its one invocation per tier above must execute every named native case.
        for target in TARGETS.iter().filter(|t| !["cpp", "csharp", "python", "ruby"].contains(&t.language)) {
            result.push(suite("suspect-codegen", &format!("{}_protocol", target.language), &format!("{tier}-{}_protocol", target.language), native_environment(target.language, tier)));
        }
    }
    // Each TypeScript source/SDK harness exercises both Node/compiler tiers
    // internally; its installed-package and browser gates run once per suite.
    for (name, version) in [("typescript_applicators", "V2"), ("typescript_resources", "V3")] {
        let mut env = native_environment("typescript", "matrix");
        env.extend([
            ("SUSPECT_DOCS_NODE".into(), "{node22}".into()),
            ("SUSPECT_NODE24_BIN".into(), "{node24}".into()),
            ("SUSPECT_CHROMIUM".into(), "{chromium}".into()),
            (format!("SUSPECT_TYPESCRIPT_{version}_MATRIX"), "1".into()),
            (format!("SUSPECT_TYPESCRIPT_{version}_ARTIFACTS"), format!("{{work}}/native/typescript/matrix/{}", version.to_ascii_lowercase())),
        ]);
        result.push(suite("suspect-codegen", name, &format!("matrix-{name}"), env));
    }
    let mut ignored_encoding = native_environment("typescript", "matrix");
    ignored_encoding.extend([
        ("SUSPECT_DOCS_NODE".into(), "{node22}".into()),
        ("SUSPECT_NODE24_BIN".into(), "{node24}".into()),
        ("SUSPECT_PROTOCOL_ARTIFACTS".into(), "{work}/native/typescript/matrix/ignored-encoding".into()),
    ]);
    result.push(suite("suspect-codegen", "typescript_multipart_ignored_encoding", "matrix-typescript_multipart_ignored_encoding", ignored_encoding));
    let mut typescript_env = native_environment("typescript", "matrix");
    typescript_env.extend([
        ("SUSPECT_DOCS_NODE".into(), "{node22}".into()),
        ("SUSPECT_NODE24_BIN".into(), "{node24}".into()),
        ("SUSPECT_CHROMIUM".into(), "{chromium}".into()),
        ("OPENROUTER_WEB_ROOT".into(), "{out}/inputs".into()),
        ("SUSPECT_CREDENTIAL_ENV_ARTIFACTS".into(), "{work}/native/typescript/matrix/credential-env".into()),
    ]);
    result.push(suite("suspect-codegen", "typescript_credential_env", "matrix-typescript_credential_env", typescript_env));
    result.push(suite("suspect-codegen", "python_credential_env", "matrix-python_credential_env", BTreeMap::from([
        ("OPENROUTER_WEB_ROOT".into(), "{out}/inputs".into()),
        ("SUSPECT_PYTHON_TOOLS".into(), "{python-tools}".into()),
    ])));
    let mut cpp_env = native_environment("cpp", "declared");
    cpp_env.extend([
        ("OPENROUTER_WEB_ROOT".into(), "{out}/inputs".into()),
        ("OPENROUTER_API_KEY".into(), "cpp-generator-process-canary-sdk-full".into()),
        ("SUSPECT_CPP_CXX".into(), "{cxx}".into()),
        ("SUSPECT_CPP_CMAKE".into(), "{cmake}".into()),
        ("SUSPECT_CPP_DOXYGEN".into(), "{doxygen}".into()),
    ]);
    result.push(suite("suspect-codegen", "cpp_credential_env", "matrix-cpp_credential_env", cpp_env));
    // The C# suite itself pins global.json with rollForward=disable and runs both
    // net8.0 and net10.0 consumers. C++ has one declared, verified C++20 profile.
    for language in ["csharp", "cpp"] {
        for kind in ["sdk", "protocol"] {
            result.push(suite("suspect-codegen", &format!("{language}_{kind}"), &format!("matrix-{language}_{kind}"), native_environment(language, "declared")));
        }
    }
    result.push(suite("suspect-codegen", "csharp_positional", "matrix-csharp_positional", native_environment("csharp", "matrix")));
    // The Python protocol suite builds installed consumers on both 3.11 and 3.14.
    result.push(suite("suspect-codegen", "python_protocol", "matrix-python_protocol", BTreeMap::new()));
    result.push(suite("suspect-codegen", "python_applicators", "matrix-python_applicators", BTreeMap::from([
        ("SUSPECT_PYTHON_SCOPED_ARTIFACTS".into(), "{work}/native/python/matrix/v2".into()),
    ])));
    for name in ["python_schema_v2", "python_resources", "python_schema_v3", "python_document_servers"] {
        result.push(suite("suspect-codegen", name, &format!("matrix-{name}"), BTreeMap::from([
            ("SUSPECT_PYTHON_SCOPED_VERSIONS".into(), "3.11,3.14".into()),
        ])));
    }
    for language in ["java", "csharp", "kotlin", "ruby", "php", "dart", "cpp"] {
        result.push(suite("suspect-codegen", &format!("{language}_integration"), &format!("integration-{language}"), native_environment(language, "current")));
    }
    let mut host = suite("suspect-codegen", "suspect_codegen", "library-codegen-host", BTreeMap::new());
    host.args = strings(&["--show-output", "--test-threads=1"]);
    host.criterion = Criterion::Exit;
    result.push(host);
    result.extend(library_native_stages());
    result
}

fn library_native_stages() -> Vec<Stage> {
    let mut stages = Vec::new();
    for case in LIBRARY_NATIVE {
        for tier in case.tiers {
            let mut item = suite(
                "suspect-codegen",
                "suspect_codegen",
                &format!("{tier}-{}", case.id),
                native_environment(case.language, tier),
            );
            item.args = strings(&[
                "--include-ignored",
                "--exact",
                case.name,
                "--show-output",
                "--test-threads=1",
            ]);
            if case.id.starts_with("swift-v3-") || case.id == "swift-document-base" {
                item.environment.insert(
                    "SUSPECT_SWIFT_V3_ROOT".into(),
                    format!("{{work}}/native/swift/{tier}/v3"),
                );
            }
            if case.id.starts_with("csharp-credential-env-") {
                item.environment
                    .insert("SUSPECT_DOTNET_BIN".into(), "{dotnet}".into());
                item.environment
                    .insert("OPENROUTER_WEB_ROOT".into(), "{out}/inputs".into());
            }
            item.criterion = Criterion::Exit;
            stages.push(item);
        }
    }
    stages
}

fn library_census_stages() -> Vec<Stage> {
    let mut all = command(
        "library-codegen-list",
        CODEGEN_LIBRARY,
        &["--list", "--format", "pretty"],
        "{workspace}",
    );
    all.criterion = Criterion::Exit;
    let mut ignored = command(
        "library-codegen-list-native",
        CODEGEN_LIBRARY,
        &["--list", "--ignored", "--format", "pretty"],
        "{workspace}",
    );
    ignored.criterion = Criterion::Exit;
    vec![all, ignored]
}

fn suite_groups() -> BTreeMap<String, BTreeSet<String>> {
    let mut groups = BTreeMap::<String, BTreeSet<String>>::new();
    for item in suites() {
        let (package, name) = item
            .program
            .trim_start_matches("test:")
            .split_once('/')
            .expect("maintained suite identity");
        groups
            .entry(package.into())
            .or_default()
            .insert(name.into());
    }
    groups
}

fn build_tests(package: &str) -> Stage {
    let mut item = command(
        &format!("build-tests-{package}"),
        "cargo",
        &[
            "test",
            "--locked",
            "--offline",
            "--no-run",
            "--message-format=json",
            "-p",
            package,
            "--lib",
            "--tests",
        ],
        "{workspace}",
    );
    if package == "suspect-cli" {
        item.args.extend(strings(&["--features", FEATURES]));
    } else {
        item.args.push("--all-features".into());
    }
    item
}

fn build_cli() -> Stage {
    command(
        "build-cli",
        "cargo",
        &[
            "build",
            "--locked",
            "--offline",
            "-p",
            "suspect-cli",
            "--bin",
            "suspect",
            "--features",
            FEATURES,
        ],
        "{workspace}",
    )
}

fn default_cli_stages() -> Vec<Stage> {
    let build = command(
        "build-default-cli",
        "cargo",
        &[
            "build",
            "--locked",
            "--offline",
            "-p",
            "suspect-cli",
            "--bin",
            "suspect",
        ],
        "{workspace}",
    );
    let mut profiles = command(
        "default-cli-twelve-profiles",
        "{out}/bin/suspect-default",
        &["codegen-profiles", "--format", "json"],
        "{workspace}",
    );
    profiles.criterion = Criterion::Json {
        report: None,
        assertions: BTreeMap::from([("/format".into(), json!("suspect.sdk.profiles.v1"))]),
    };
    vec![build, profiles]
}

fn verify_profile_inventory(profiles: &Value) -> Result<()> {
    let entries = profiles["profiles"]
        .as_array()
        .context("profile inventory")?;
    ensure!(
        entries.len() == TARGETS.len(),
        "default/full CLI must advertise exactly twelve SDK profiles"
    );
    for target in TARGETS {
        ensure!(
            entries
                .iter()
                .filter(|entry| entry["profile"] == target.backend
                    && entry["directory"] == target.language)
                .count()
                == 1,
            "missing/duplicate {} profile",
            target.backend
        );
    }
    Ok(())
}

fn editor_stages() -> Vec<Stage> {
    let mut items = sealed::editor_stages();
    items
        .last_mut()
        .expect("editor test stage")
        .args
        .push("test/generation-profiles.cjs".into());
    items
}

fn quality_stages() -> Vec<Stage> {
    let mut items = sealed::quality_stages();
    for item in items.iter_mut().take(3) {
        let at = item
            .args
            .iter()
            .position(|arg| arg == "--")
            .unwrap_or(item.args.len());
        item.args
            .splice(at..at, strings(&["--offline", "--all-features"]));
    }
    items
}

fn original_package_stages() -> Vec<Stage> {
    let mut items = sealed::package_stages();
    for item in &mut items {
        item.id = item.id.replace("five-op-", "twelve-five-op-");
        for value in item.args.iter_mut().chain(std::iter::once(&mut item.cwd)) {
            *value = value
                .replace("m3-m6-sdk", "sdk-full")
                .replace("m3_m6_sdk", "sdk_full");
        }
    }
    items
}

fn profile_stages() -> Vec<Stage> {
    let mut discover = command(
        "cli-twelve-profiles",
        "{out}/bin/suspect",
        &["codegen-profiles", "--format", "json"],
        "{workspace}",
    );
    discover.criterion = Criterion::Json {
        report: None,
        assertions: BTreeMap::from([("/format".into(), json!("suspect.sdk.profiles.v1"))]),
    };
    let mut compare = command(
        "cli-twelve-compatibility",
        "{out}/bin/suspect",
        &[
            "codegen-compare",
            "--before",
            "{out}/configs/five-operations.json",
            "--after",
            "{out}/configs/five-operations.json",
            "--format",
            "json",
        ],
        "{workspace}",
    );
    compare.criterion = Criterion::Json {
        report: None,
        assertions: BTreeMap::from([
            ("/format".into(), json!("suspect-sdk-compatibility-v1")),
            ("/summary/unknowns".into(), json!(0)),
            ("/summary/breaking_changes".into(), json!(0)),
            ("/summary/potentially_breaking_changes".into(), json!(0)),
        ]),
    };
    vec![discover, compare]
}

fn expected_native_tests(name: &str) -> &'static [&'static str] {
    match name {
        "credential_env" => &[
            "explicit_bearer_mapping_binds_one_source_scheme_and_only_variable_names",
            "policy_syntax_is_closed_bounded_and_duplicate_keys_are_not_silently_replaced",
            "environment_defaults_are_captured_as_client_policy_not_wire_interpretation",
            "unknown_unused_and_non_string_hooks_are_located_refusals",
            "empty_operation_capture_cannot_skip_configured_native_admission",
            "referenced_schemes_keep_physical_provenance_without_relocation_in_semantic_equality",
            "same_name_in_distinct_requirement_documents_is_ambiguous_even_with_one_terminal",
            "policy_edits_cannot_reuse_ordinary_target_artifacts_and_reverts_reuse_the_original_arc",
            "verified_native_defaults_share_generation_capture_and_policy_edit_cache_identity",
        ],
        "credential_env_codegen" => &[
            "environment_policy_is_explicit_in_reports_and_cannot_be_ignored_before_writes",
            "configured_generation_reports_names_only_and_disabling_restores_all_ordinary_bytes",
        ],
        "go_credential_env_canonical" => {
            &["canonical_go_environment_capture_retains_bound_semantics_and_fingerprinted_factory"]
        }
        "go_credential_env_factory_capture" => &[
            "additive_model_collision_reports_source_corresponding_environment_factory_break",
            "unselected_collision_and_absent_policy_keep_the_existing_surface_compatible",
            "environment_factory_signature_is_part_of_native_constructor_comparison",
        ],
        "swift_credential_env_canonical" => &[
            "canonical_generation_and_capture_match_the_bound_swift_plan",
            "canonical_session_policy_edits_reverts_and_refusals_stay_isolated",
        ],
        "typescript_credential_env" => &[
            "no_policy_http_package_keeps_its_baseline_bytes",
            "environment_plan_retains_source_binding_and_semantic_metadata",
            "environment_binding_refuses_invalid_unbound_and_unsupported_policies",
            "installed_environment_defaults_preserve_creation_snapshot_explicit_auth_and_security_choices",
            "installed_openrouter_current_key_uses_source_bearer_env_and_default_https",
            "browser_environment_absence_preserves_explicit_and_anonymous_source_clients",
        ],
        "python_credential_env" => &[
            "no_policy_python_output_retains_pre_feature_bytes",
            "python_credential_env_binds_source_schemes_and_emits_only_configured_defaults",
            "python_credential_env_canonical_capture_retains_bound_semantics",
            "installed_python_credential_env_snapshots_and_explicit_auth_precedence",
            "installed_openrouter_python_credential_env_current_key_and_optional_credits",
        ],
        "go_credential_env" => &[
            "no_policy_retains_pre_change_sdk_and_terraform_bytes",
            "policy_binds_actual_sources_and_allocates_factory_without_changing_explicit_api",
            "unsupported_env_attachment_is_refused_by_shared_binder_before_artifacts",
            "generator_environment_values_do_not_enter_configured_or_unconfigured_artifacts",
            "native_env_factory_snapshots_and_keeps_whole_explicit_credentials_authoritative",
            "native_allocated_factory_handles_symbol_collision_and_bounded_env_input",
            "native_actual_openrouter_key_and_credits_use_env_snapshot_and_source_https",
        ],
        "rust_credential_env" => &[
            "rust_no_policy_artifacts_match_pre_env_checkpoint",
            "rust_credential_env_is_source_bound_and_reserves_only_configured_helpers",
            "rust_credential_env_generation_does_not_capture_generator_values",
            "installed_rust_credential_env_snapshots_and_explicit_credentials_obey_source_auth",
            "installed_rust_openrouter_env_current_key_uses_source_default_https",
        ],
        "swift_credential_env" => &[
            "no_policy_artifacts_are_frozen",
            "bound_policy_retains_source_identity_and_reserves_only_configured_helpers",
            "shared_binding_failures_precede_artifacts",
            "generation_never_reads_credential_values",
            "native_credential_env_precedence_snapshot_and_docs",
            "native_openrouter_current_key_environment_defaults",
        ],
        "java_credential_env" => &[
            "no_policy_emission_is_byte_stable",
            "bound_variable_names_and_helpers_retain_source_identity_without_values",
            "native_policy_binding_keeps_shared_refusals_after_protocol_admission",
            "canonical_capture_retains_semantic_policy_and_actual_factory_signatures",
            "native_credential_env_controls",
            "native_credential_env_openrouter",
        ],
        "kotlin_credential_env" => &[
            "binds_declared_schemes_and_preserves_alias_credentials",
            "no_policy_emits_no_environment_helpers_or_branches",
            "unsupported_environment_kinds_and_unbound_names_are_source_findings",
            "generator_environment_canary_is_not_emitted",
            "canonical_capture_uses_semantic_env_descriptor_and_actual_native_overloads",
            "policy_edit_and_revert_are_session_identity_without_secret_values",
            "native_environment_controls",
            "native_openrouter_current_key_environment",
        ],
        "php_credential_env" => &[
            "bound_policy_and_helper_allocation_preserve_native_source_identity",
            "canonical_generation_capture_and_policy_reverts_preserve_php_semantics",
            "generation_does_not_capture_environment_values",
            "php_admission_keeps_shared_unsupported_credential_findings_located",
            "native_env_snapshot_explicit_auth_and_unavailable_platform",
            "native_openrouter_current_key_factory_uses_source_https",
        ],
        "cpp_credential_env" => &[
            "native_openrouter_credential_env_snapshot_and_explicit_precedence",
            "credential_env_binds_native_declarations_and_reserves_only_configured_helpers",
            "actual_openrouter_credential_env_generation_contract",
            "native_credential_env_security_and_portable_controls",
        ],
        "http_protocol" => &[
            "ignored_multipart_style_fields_preserve_explicit_and_default_content_plans",
            "ignored_multipart_style_cannot_bypass_the_actual_content_codec_or_metadata_validation",
            "active_form_styles_keep_their_oas31_whole_property_and_oas32_item_semantics",
            "older_oas_versions_do_not_gain_invented_positional_or_named_mixed_encodings",
        ],
        "contract_dual_role_scope" => &[
            "ignored_schema_siblings_keep_dual_role_reference_scope_lossless",
            "ignored_schema_siblings_keep_dual_role_reference_scope_fast",
            "removing_the_ignored_sibling_does_not_change_dual_role_uri_identity",
            "schema_only_and_http_only_reference_roles_keep_their_own_metadata",
            "declaring_the_http_role_first_still_preserves_the_known_schema_root",
            "equivalent_json_yaml_and_both_readers_have_the_same_dual_role_scope",
            "malformed_and_missing_references_remain_real_applicable_findings_in_both_roles",
            "an_invalid_modern_schema_scope_is_not_replaced_by_a_valid_http_scope",
            "actual_resource_boundaries_and_base_uris_still_make_dual_role_scopes_ambiguous",
            "true_cross_document_dialect_ambiguity_is_retained_with_both_origins",
        ],
        "java_sdk" => &[
            "maven_jar_consumer_exact_models_types_and_sync_async_wire",
            "native_immutable_models_unions_literals_and_shared_budgets",
            "native_five_actual_openrouter_operations_and_constructor_examples",
            "native_names_status_alternatives_and_doc_examples",
        ],
        "csharp_sdk" => &[
            "native_package_builds_and_consumes_negative_types",
            "native_actual_openrouter_five_operations",
            "native_portable_validation_executes_shared_vectors_and_failure_controls",
            "native_nullable_union_and_name_adversarial_contract",
        ],
        "kotlin_sdk" => &[
            "native_m2_installed_module_models_coroutines_wire_docs_and_types",
            "native_actual_five_openrouter_operations",
            "native_shared_17_validation_vectors",
            "native_adversarial_models_names_and_examples",
        ],
        "ruby_sdk" => &[
            "native_m2_gem_types_docs_examples_and_independent_wire",
            "native_shared_contract_exact_json_and_adversarial_models",
            "native_shared_branch_copy_equality_and_number_budgets",
            "native_openrouter_five_actual_operations",
            "native_expanded_protocol_wire_parts_streams_and_types",
            "native_additional_openrouter_binary_and_delete_operations",
            "native_oas30_nullable_reference_siblings_and_binary",
        ],
        "ruby_compatibility_credentials" => &[
            "source_relocation_alone_preserves_native_credential_equality_and_locations",
            "relocated_oauth_oidc_and_api_keys_keep_typed_values_and_url_bases",
            "credential_names_types_attachments_alternatives_and_permissions_still_change",
            "oauth_flows_endpoints_and_oidc_discovery_changes_survive_capture",
            "effective_server_base_changes_remain_real_wire_changes",
            "request_name_tightening_remains_wire_change_without_credential_noise",
        ],
        "ruby_credential_env" => &[
            "bound_policy_uses_admitted_source_names_and_keeps_no_policy_bytes",
            "credential_env_refusals_are_shared_and_follow_protocol_admission",
            "installed_credential_env_omission_precedence_choices_and_snapshot",
            "installed_openrouter_env_uses_source_default_https_and_real_key_schemas",
            "canonical_credential_env_capture_is_semantic_and_keeps_credential_surface",
        ],
        "php_sdk" => &[
            "composer_installed_m2_native_package",
            "composer_installed_actual_openrouter_five_operations",
            "installed_adversarial_models_names_docs_and_union_codecs",
            "native_shared_owned_program_vectors",
            "native_exact_numeric_math_and_json_boundaries",
            "native_resource_failures_survive_branches_and_conversion_trials",
        ],
        "dart_sdk" => &[
            "native_m2_installed_consumer",
            "native_runtime_vectors_refs_and_resource_classification",
            "native_five_actual_openrouter_operations",
            "native_adversarial_names_and_optional_body",
        ],
        "dart_credential_env" => &[
            "no_policy_output_bytes_are_preserved",
            "policy_binds_actual_source_names_and_keeps_semantics_relocation_independent",
            "canonical_policy_capture_and_session_identity_are_source_bound",
            "native_credential_env_controls",
            "native_credential_env_constructor_compatibility",
            "native_openrouter_credential_env",
        ],
        "cpp_sdk" => &[
            "native_m2_cpp_cmake_codecs_http_and_docs",
            "native_five_actual_openrouter_operations",
            "native_shared_runtime_contract_vectors",
            "native_adversarial_models_and_consumers",
        ],
        "typescript_protocol" => &[
            "native_security_alternatives_credentials_and_declared_server_choices_are_explicit",
            "native_responses_dispatch_by_actual_status_media_and_typed_header_provenance",
        ],
        "typescript_multipart_ignored_encoding" => {
            &["ignored_multipart_styles_preserve_content_in_installed_requests_and_responses"]
        }
        "rust_protocol" => &[
            "installed_native_protocol_consumer_wire_bytes_docs_and_ownership",
            "native_rootless_bytes_empty_forms_and_oas30_packages",
        ],
        "go_protocol" => &[
            "native_all_literal_parameter_vectors_and_fail_before_transport",
            "native_security_alternatives_servers_methods_and_querystring",
            "native_response_status_media_headers_links_and_byte_boundaries",
            "native_forms_named_and_positional_multipart_are_structural_and_typed",
            "native_sse_json_lines_context_owned_iteration_and_cleanup",
            "native_actual_openrouter_five_plus_crud_and_binary_operations",
        ],
        "swift_protocol" => &[
            "native_protocol_spm_types_wire_stream_lifetimes_and_docs",
            "native_remaining_standard_custom_query_positional",
        ],
        "python_null_models" => {
            &["null_only_fields_aliases_containers_and_unions_preserve_native_typing_and_values"]
        }
        "python_protocol" => &["installed_protocol_vectors_and_stream_lifetimes"],
        "csharp_protocol" => &["native_protocol_matrix"],
        "csharp_positional" => &[
            "positional_projection_uses_only_reachable_part_and_header_codecs",
            "undefined_positional_styles_are_located_refusals",
            "ignored_positional_styles_preserve_native_content_profile",
            "ignored_positional_styles_cannot_hide_invalid_content_or_metadata",
            "documented_httpclient_method_case_refusal_is_source_located",
            "canonical_generation_options_are_identical_in_generation_and_snapshot_capture",
            "native_positional_multipart_matrix",
        ],
        "php_protocol" => &[
            "native_protocol_package_types_and_injected_wire",
            "native_document_relative_servers_keep_physical_bases",
        ],
        "cpp_protocol" => &[
            "native_fresh_m2_codecs_and_installed_package",
            "native_rich_protocol_installed_wire_and_streams",
        ],
        "dart_protocol" => &["native_rich_protocol"],
        "kotlin_protocol" => &["native_rich_protocol_package"],
        "go_models" => &[
            "unimplemented_go_shapes_block_artifacts_with_source_locations",
            "pattern_properties_use_scoped_codecs_and_retain_model_obligations",
            "native_pattern_properties_validate_matching_values_without_stripping_extras",
        ],
        "go_schema_v2" => &[
            "ordinary_media_examples_have_one_entry_binding_and_rich_recipes_remain_explicit",
            "checked_program_fences_and_base_v1_program_identity",
            "scoped_32_source_vectors_execute_in_installed_consumers",
            "scoped_models_codecs_and_sdk_operations_preserve_native_data",
            "scoped_native_recursion_depth_and_call_isolation",
            "native_physical_document_base_redirect_and_encoded_path_witness",
        ],
        "go_schema_v3" => &[
            "official_44_unmodified_resource_cases_execute_with_closed_supplied_documents",
            "malformed_v3_contexts_never_emit_and_old_envelopes_reject_resources",
            "independent_dynamic_scope_branch_cycle_budget_and_annotation_controls",
            "native_codec_boundaries_preserve_outer_dynamic_context_and_base_profile_selection",
            "installed_resource_sdk_keeps_typed_fields_dynamic_wire_checks_and_examples",
            "native_distinct_resource_depth_and_concurrent_contexts_are_bounded",
        ],
        "go_examples_aggregate" => {
            &["declared_form_and_positional_groups_keep_items_extras_and_absence"]
        }
        "ruby_schema_v2" => &[
            "source_driven_v2_instructions_scopes_budgets_and_program_guards",
            "scoped_native_descriptors_retain_fields_patterns_and_real_program_identity",
            "installed_scoped_sdk_models_types_examples_and_wire",
        ],
        "ruby_schema_v3" => &[
            "source_driven_official_v3_resources_scopes_and_guards",
            "installed_resource_sdk_models_types_examples_and_wire",
            "resource_native_descriptors_and_profile_selection_preserve_physical_identity",
        ],
        "ruby_document_servers" => &[
            "physical_base_metadata_is_separate_and_schema_resource_fences_stay_closed",
            "installed_gem_uses_effective_physical_document_server_bases",
        ],
        "python_applicators" => {
            &["scoped_python_runtime_matches_source_vectors_guards_and_exact_budgets"]
        }
        "python_schema_v2" => &[
            "scoped_python_http_plan_preserves_carriers_examples_and_typed_capture",
            "installed_scoped_python_sdk_operations_mutations_examples_types_and_sphinx",
        ],
        "python_resources" => {
            &["python_v3_executes_official_sources_dynamic_scopes_guards_and_exact_budgets"]
        }
        "python_schema_v3" => &[
            "python_resource_admission_keeps_ordinary_programs_and_typed_capture",
            "installed_python_v3_dynamic_operations_native_examples_types_and_sphinx",
        ],
        "python_document_servers" => &[
            "installed_python_physical_document_servers_preserve_redirects_overrides_and_encoded_paths",
        ],
        "typescript_applicators" => &[
            "v2_instructions_cannot_enter_v1_or_unknown_program_envelopes",
            "source_driven_v2_vectors_preserve_scope_failures_and_original_locations",
            "scoped_work_identity_and_mutation_controls_are_native_failures_not_mismatches",
            "scoped_sdk_capture_tracks_checked_graphs_and_preserves_literal_instance_data",
            "scoped_http_examples_keep_valid_declared_values_and_locate_invalid_ones",
            "installed_scoped_sdk_preserves_models_values_examples_and_docs",
            "browser_scoped_sdk_executes_source_vectors_and_native_http_controls",
        ],
        "typescript_resources" => &[
            "v3_metadata_and_dynamic_ops_cannot_enter_older_or_mismatched_envelopes",
            "source_driven_v3_executes_all_44_official_cases_from_closed_supplied_documents",
            "native_v3_scope_identity_lookup_budgets_and_malformed_metadata_controls",
            "resource_selection_keeps_base_profiles_and_source_linked_refusals",
            "resource_capture_uses_typed_context_graphs_and_keeps_physical_locations_separate",
            "v3_example_admission_retains_dynamic_invalidity_and_declared_source",
            "installed_v3_sdk_preserves_dynamic_models_exact_values_examples_and_native_docs",
            "physical_document_server_bases_preserve_redirect_provenance_overrides_and_encoded_paths",
            "browser_v3_executes_official_sources_dynamic_sdk_calls_and_contextual_codecs",
        ],
        "rust_validation_v2" => &[
            "independent_v2_programs_run_through_installed_native_validation",
            "official_applicators_run_against_native_v2_with_unmodified_schema_roots",
            "v2_recursive_depth_and_codec_sessions_have_finite_isolated_scopes",
            "installed_v2_codecs_validate_mutable_native_carriers_and_negative_consumers",
            "installed_v2_codec_resource_failures_are_never_invalid_or_representation_success",
            "scoped_http_native_constructor_examples_compile_and_execute",
        ],
        "rust_validation_v3" => &[
            "v3_checked_emission_rejects_malformed_resources_and_old_envelopes",
            "installed_rust_v3_executes_all_official_dynamic_ref_source_fixtures",
            "installed_rust_v3_scope_cycles_fallbacks_and_work_failures_are_independent",
            "installed_rust_v3_default_depth_scope_restore_and_annotation_isolation",
        ],
        "rust_protocol_v3" => &[
            "rust_v3_planners_preserve_ordinary_program_bytes_and_old_resource_fences",
            "rust_v3_nested_entry_retains_idless_ancestor_resource_admission",
            "rust_v3_http_examples_and_candidate_diagnostics_keep_physical_sources",
            "installed_rust_v3_sdk_preserves_dynamic_models_examples_and_real_wire",
        ],
        "rust_protocol_resources" => &[
            "rust_document_base_capability_does_not_enable_schema_or_dynamic_execution",
            "installed_rust_relative_servers_use_effective_physical_documents",
        ],
        "swift_validation_v2" => &["native_installed_v2_codecs_types_and_docs"],
        "kotlin_validation_v2" => &["native_scoped_32_vectors"],
        "kotlin_validation_v3" => &[
            "native_resource_44_vectors",
            "native_resource_sdk_operations",
        ],
        "kotlin_protocol_documents" => &[
            "physical_document_bases_and_logical_contexts_are_distinct",
            "native_physical_document_servers",
        ],
        "java_schema_v2" => &[
            "native_schema_v2_vectors",
            "native_schema_v2_sdk_operations",
        ],
        "java_schema_v3" => &[
            "native_schema_v3_resource_conformance",
            "native_schema_v3_sdk_operations",
        ],
        "java_aggregate_examples" => &[
            "complete_declared_groups_preserve_values_and_native_roots",
            "aggregate_factories_refuse_jvm_arity_overflow_at_source",
            "native_declared_aggregate_examples",
        ],
        "dart_integration" => &["scoped_v2_capture_matches_native_carriers_and_full_codec_policy"],
        _ => &[],
    }
}

fn harness_evidence(name: &str, stdout: &str, stderr: &str) -> Result<Value> {
    let passed = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("test ")?.strip_suffix(" ... ok"))
        .collect::<BTreeSet<_>>();
    if name == "pinned_transport" {
        // The sole ignored row is a subprocess entry point, not a missing gate.
        ensure!(
            stdout
                .lines()
                .filter(|line| line.starts_with("test environment_child ... ignored"))
                .count()
                == 1,
            "missing explicit subprocess helper accounting"
        );
        let normalized = stdout.replace("; 1 ignored;", "; 0 ignored;");
        ensure!(
            sealed::complete_rust_tests(&normalized),
            "incomplete transport parent suite"
        );
        ensure!(
            passed.contains(
                "ambient_proxies_curlrc_and_credentials_are_ignored_by_the_real_transport"
            ),
            "hostile-environment parent did not execute"
        );
        ensure!(
            passed.contains(
                "https_verification_cannot_be_disabled_by_curlrc_or_ambient_certificate_settings"
            ),
            "TLS hostile-environment parent did not execute"
        );
    } else {
        ensure!(
            sealed::complete_rust_tests(stdout),
            "required native/host suite was empty, ignored, filtered or incomplete"
        );
    }
    ensure!(!passed.is_empty(), "no named tests executed");
    for required in expected_native_tests(name) {
        ensure!(
            passed.contains(required),
            "required native test did not execute: {name}::{required}"
        );
    }
    if name == "protocol_examples" {
        for witness in [
            "scoped_examples_check_all_applicators_and_keep_declared_slot_provenance",
            "explicit_scoped_example_profile_preserves_base_values_and_synthesis_identity",
        ] {
            ensure!(
                passed.contains(witness),
                "required scoped example witness did not execute: {witness}"
            );
        }
    }
    if name.ends_with("_protocol") && name != "http_protocol" {
        ensure!(
            passed
                .iter()
                .any(|name| name.starts_with("native_") || name.starts_with("installed_")),
            "protocol suite has no executed native witness"
        );
    }
    reject_early_skip(stdout, stderr)?;
    Ok(
        json!({"suite":name,"executedTests":passed,"nativeWitnesses":expected_native_tests(name),"ignoredSubprocessEntry":(name == "pinned_transport").then_some("environment_child"),"nativeOutputPolicy":"captured retained outputs and frozen harness assertions; discarded subprocess output is not a transcript"}),
    )
}

fn reject_early_skip(stdout: &str, stderr: &str) -> Result<()> {
    for line in stdout.lines().chain(stderr.lines()) {
        let text = line.trim().to_ascii_lowercase();
        ensure!(
            !text.starts_with("skipping ")
                && !text.starts_with("skipped:")
                && !text.contains("native gate skipped")
                && !text.contains("missing required tool"),
            "required suite reported an early skip: {line}"
        );
    }
    Ok(())
}

fn execute_suite(run: &mut Run, item: &Stage) -> Result<()> {
    run.command(item)?;
    let (_, name) = item
        .program
        .trim_start_matches("test:")
        .split_once('/')
        .context("suite identity")?;
    let result = (|| -> Result<Value> {
        ensure!(
            met(run, &item.id),
            "suite command failed; inspect its stdout/stderr"
        );
        let out = fs::read_to_string(run.out.join(format!("logs/{}.stdout.log", item.id)))?;
        let err = fs::read_to_string(run.out.join(format!("logs/{}.stderr.log", item.id)))?;
        if item.program == CODEGEN_LIBRARY {
            let census: LibraryCensus =
                serde_json::from_slice(&fs::read(run.out.join("library-test-census.json"))?)?;
            let exact = item
                .args
                .iter()
                .position(|arg| arg == "--exact")
                .and_then(|index| item.args.get(index + 1))
                .map(String::as_str);
            library_execution_evidence(&census, exact, &out, &err)
        } else {
            harness_evidence(name, &out, &err)
        }
    })();
    verification(run, &format!("{}-execution", item.id), result)
}

#[derive(Debug, Deserialize, Serialize)]
struct LibraryCensus {
    all: BTreeSet<String>,
    native: BTreeSet<String>,
}

fn listed_tests(text: &str) -> Result<BTreeSet<String>> {
    let mut names = BTreeSet::new();
    let mut count = None;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        if let Some(name) = line.strip_suffix(": test") {
            ensure!(
                !name.is_empty() && names.insert(name.to_owned()),
                "empty/duplicate test census entry"
            );
        } else {
            let (tests, benchmarks) = line
                .split_once(" tests, ")
                .or_else(|| line.split_once(" test, "))
                .context("unexpected libtest census output")?;
            ensure!(
                count.is_none() && benchmarks == "0 benchmarks",
                "unexpected benchmark/duplicate census footer"
            );
            count = Some(tests.parse::<usize>()?);
        }
    }
    ensure!(
        !names.is_empty() && count == Some(names.len()),
        "empty/incomplete libtest census"
    );
    Ok(names)
}

fn validate_library_census(
    all: &str,
    native: &str,
    declared: &BTreeSet<String>,
) -> Result<LibraryCensus> {
    let all = listed_tests(all)?;
    let native = listed_tests(native)?;
    ensure!(
        native.is_subset(&all) && all.len() > native.len(),
        "native census is not a proper subset of library tests"
    );
    ensure!(
        &native == declared,
        "ignored library tests differ from maintained native inventory; unassigned={:?}, missing={:?}",
        native.difference(declared).collect::<Vec<_>>(),
        declared.difference(&native).collect::<Vec<_>>()
    );
    Ok(LibraryCensus { all, native })
}

fn library_census(run: &mut Run) -> Result<Value> {
    for item in library_census_stages() {
        ensure!(met(run, &item.id), "{} failed", item.id);
    }
    let all = fs::read_to_string(run.out.join("logs/library-codegen-list.stdout.log"))?;
    let native = fs::read_to_string(run.out.join("logs/library-codegen-list-native.stdout.log"))?;
    let declared = LIBRARY_NATIVE
        .iter()
        .map(|case| case.name.to_owned())
        .collect();
    let census = validate_library_census(&all, &native, &declared)?;
    write_json(&run.out.join("library-test-census.json"), &census)?;
    run.remember(&run.out.join("library-test-census.json"))?;
    Ok(
        json!({"census":"library-test-census.json","sha256":hash(&run.out.join("library-test-census.json"))?,"tests":census.all.len(),"native":census.native,"frozenBinary":run.tests.get("suspect-codegen/suspect_codegen")}),
    )
}

fn rust_summary(text: &str) -> Result<BTreeMap<String, usize>> {
    let rows = text
        .lines()
        .filter_map(|line| line.strip_prefix("test result: ok. "))
        .collect::<Vec<_>>();
    ensure!(
        rows.len() == 1,
        "missing/ambiguous successful libtest summary"
    );
    let mut counts = BTreeMap::new();
    for label in ["passed", "failed", "ignored", "measured", "filtered out"] {
        let values = rows[0]
            .split(';')
            .filter_map(|part| part.trim().strip_suffix(label).map(str::trim))
            .collect::<Vec<_>>();
        ensure!(values.len() == 1, "missing/duplicate libtest count {label}");
        counts.insert(label.into(), values[0].parse()?);
    }
    Ok(counts)
}

fn library_execution_evidence(
    census: &LibraryCensus,
    exact: Option<&str>,
    stdout: &str,
    stderr: &str,
) -> Result<Value> {
    reject_early_skip(stdout, stderr)?;
    let counts = rust_summary(stdout)?;
    let passed = stdout
        .lines()
        .filter_map(|line| line.strip_prefix("test ")?.strip_suffix(" ... ok"))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let ignored = stdout
        .lines()
        .filter_map(|line| {
            line.strip_prefix("test ")?
                .split_once(" ... ignored")
                .map(|(name, _)| name.to_owned())
        })
        .collect::<BTreeSet<_>>();
    ensure!(
        counts["failed"] == 0 && counts["measured"] == 0,
        "library selection has failures/benchmarks"
    );
    let expected = if let Some(name) = exact {
        ensure!(
            census.native.contains(name),
            "selected native test is not in frozen census"
        );
        ensure!(
            counts["filtered out"] == census.all.len() - 1
                && counts["ignored"] == 0
                && ignored.is_empty(),
            "native exact selection filtered/ignored accounting differs"
        );
        BTreeSet::from([name.to_owned()])
    } else {
        ensure!(
            counts["filtered out"] == 0
                && counts["ignored"] == census.native.len()
                && ignored == census.native,
            "host library omitted tests outside the explicitly assigned native partition"
        );
        census.all.difference(&census.native).cloned().collect()
    };
    ensure!(
        passed == expected && counts["passed"] == expected.len() && !passed.is_empty(),
        "library selection omitted/substituted named results"
    );
    Ok(
        json!({"executedTests":passed,"counts":counts,"nativeSelection":exact,"hostDeferredToNativeSelections":if exact.is_none() {json!(census.native)} else {json!([])},"coverageAuthority":"library-test-census.json and library-codegen-coverage"}),
    )
}

fn library_coverage(run: &Run) -> Result<Value> {
    ensure!(
        met(run, "library-codegen-census") && met(run, "library-codegen-host-execution"),
        "library census/host execution is incomplete"
    );
    let stages = library_native_stages();
    for item in &stages {
        ensure!(
            met(run, &format!("{}-execution", item.id)),
            "native library witness missing: {}",
            item.id
        );
    }
    Ok(
        json!({"host":"library-codegen-host-execution","nativeSelections":stages.iter().map(|item| format!("{}-execution", item.id)).collect::<Vec<_>>(),"completePartition":true,"callerSelectableFilters":false}),
    )
}

fn freeze_file(run: &mut Run, source: &Path, destination: &Path) -> Result<Value> {
    let metadata = fs::symlink_metadata(source)?;
    ensure!(
        metadata.is_file() && metadata.len() > 0,
        "missing/nonregular executable {}",
        source.display()
    );
    let before = hash(source)?;
    let mut output = create(destination)?;
    std::io::copy(&mut fs::File::open(source)?, &mut output)?;
    let mut permissions = metadata.permissions();
    permissions.set_readonly(true);
    fs::set_permissions(destination, permissions)?;
    ensure!(
        hash(source)? == before && hash(destination)? == before,
        "executable changed while freezing"
    );
    run.remember(destination)?;
    Ok(json!({"builtPath":source,"frozenPath":destination,"sha256":before,"bytes":metadata.len()}))
}

fn freeze_tests(run: &mut Run, package: &str, wanted: &BTreeSet<String>) -> Result<Value> {
    ensure!(
        met(run, &format!("build-tests-{package}")),
        "test build did not complete"
    );
    let text = fs::read_to_string(
        run.out
            .join(format!("logs/build-tests-{package}.stdout.log")),
    )?;
    let manifest = run.work.join(format!("source/crates/{package}/Cargo.toml"));
    let mut built = BTreeMap::new();
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if value["reason"] != "compiler-artifact"
            || value["profile"]["test"] != true
            || value["manifest_path"].as_str() != manifest.to_str()
        {
            continue;
        }
        let Some(name) = value["target"]["name"].as_str() else {
            continue;
        };
        if !wanted.contains(name) {
            continue;
        }
        if let Some(path) = value["executable"].as_str() {
            let path = PathBuf::from(path).canonicalize()?;
            ensure!(
                path.starts_with(run.work.join("cargo")),
                "test executable escaped private build"
            );
            ensure!(
                built.insert(name.to_owned(), path).is_none(),
                "ambiguous test binary {name}"
            );
        }
    }
    let missing = wanted
        .iter()
        .filter(|name| !built.contains_key(*name))
        .cloned()
        .collect::<Vec<_>>();
    let mut frozen = BTreeMap::new();
    for (name, path) in built {
        let destination = run.out.join(format!("bin/tests/{package}/{name}"));
        frozen.insert(name.clone(), freeze_file(run, &path, &destination)?);
        run.tests.insert(format!("{package}/{name}"), destination);
    }
    write_json(
        &run.out.join(format!("test-binaries/{package}.json")),
        &json!({"package":package,"sourceFingerprint":run.provenance["sourceFingerprint"],"features":FEATURES,"frozen":frozen,"missing":missing}),
    )?;
    ensure!(
        missing.is_empty(),
        "missing required frozen suites for {package}: {}",
        missing.join(", ")
    );
    ensure!(
        !frozen.is_empty(),
        "test build emitted no executable evidence"
    );
    Ok(json!(frozen))
}

fn selected(name: &str, default: impl AsRef<Path>) -> String {
    std::env::var(name).unwrap_or_else(|_| default.as_ref().display().to_string())
}

fn swift_floor_selection(
    scratch: &Path,
    overrides: &BTreeMap<String, PathBuf>,
) -> BTreeMap<String, String> {
    let select = |name: &str, default: PathBuf| overrides.get(name).cloned().unwrap_or(default);
    let root = select(
        "SUSPECT_SWIFT_FLOOR_ROOT",
        scratch.join(SWIFT_FLOOR_PAYLOAD),
    );
    let driver = select("SUSPECT_SWIFT_FLOOR_BIN", root.join("usr/bin/swift"));
    [
        ("{swift-floor-root}", root),
        ("{swift-floor}", driver.clone()),
        (
            "{swiftc-floor}",
            select("SUSPECT_SWIFTC_FLOOR_BIN", driver.with_file_name("swiftc")),
        ),
        (
            "{docc-floor}",
            select(
                "SUSPECT_SWIFT_FLOOR_DOCC_BIN",
                driver.with_file_name("docc"),
            ),
        ),
        (
            "{swift-floor-sdk}",
            select(
                "SUSPECT_SWIFT_FLOOR_SDKROOT",
                "/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk".into(),
            ),
        ),
    ]
    .into_iter()
    .map(|(token, path)| (token.into(), path.display().to_string()))
    .collect()
}

fn configure_environment(run: &mut Run) -> Result<()> {
    sealed::env_values(run)?;
    // Only sdk-full selects the recovered payload by default. Resolve against
    // the caller's TMPDIR, before child environments receive private tmp paths;
    // the historical M3/M6 default and its recorded tool identities stay separate.
    let swift_overrides = SWIFT_FLOOR_SELECTORS
        .iter()
        .filter_map(|name| {
            std::env::var_os(name).map(|path| ((*name).to_owned(), PathBuf::from(path)))
        })
        .collect();
    run.replacements.extend(swift_floor_selection(
        &std::env::temp_dir().join("opencode"),
        &swift_overrides,
    ));
    run.replacements.insert(
        "{npm}".into(),
        sealed::resolve("npm", &run.env).display().to_string(),
    );
    let home = PathBuf::from(
        run.env
            .get("HOME")
            .context("HOME is required for installed tool discovery")?,
    );
    let installs = home.join(".local/share/mise/installs");
    let old_cargo = run
        .env
        .get("CARGO_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".cargo"));
    let old_uv = run
        .env
        .get("UV_CACHE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".cache/uv"));
    let old_npm = run
        .env
        .get("npm_config_cache")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".npm"));
    for (token, selector, default) in [
        (
            "{jdk-floor}",
            "SUSPECT_JAVA_FLOOR_HOME",
            installs.join("java/temurin-21.0.12+101.0.LTS"),
        ),
        (
            "{jdk-current}",
            "SUSPECT_JAVA_CURRENT_HOME",
            installs.join("java/temurin-25.0.4+101.0.LTS"),
        ),
        (
            "{maven}",
            "SUSPECT_MAVEN_BIN",
            installs.join("maven/3.9.16/apache-maven-3.9.16/bin/mvn"),
        ),
        (
            "{dotnet}",
            "SUSPECT_DOTNET_BIN",
            home.join(".local/share/mise/dotnet-root/dotnet"),
        ),
        (
            "{ruby-floor}",
            "SUSPECT_RUBY_FLOOR_HOME",
            installs.join("ruby/3.3.12"),
        ),
        (
            "{ruby-current}",
            "SUSPECT_RUBY_CURRENT_HOME",
            installs.join("ruby/4.0.6"),
        ),
        (
            "{ruby-gems-floor}",
            "SUSPECT_RUBY_FLOOR_GEMS",
            run.original.join("target/sdk-ruby-tools/gems"),
        ),
        (
            "{ruby-gems-current}",
            "SUSPECT_RUBY_CURRENT_GEMS",
            run.original.join("target/sdk-ruby-tools/gems-ruby4"),
        ),
        (
            "{php-floor}",
            "SUSPECT_PHP_FLOOR_BIN",
            run.original.join("target/sdk-php-tools/php-8.3.32/php"),
        ),
        (
            "{php-current}",
            "SUSPECT_PHP_CURRENT_BIN",
            run.original.join("target/sdk-php-tools/php-8.5.8/php"),
        ),
        (
            "{phpstan}",
            "SUSPECT_PHPSTAN_PHAR",
            run.original
                .join("target/sdk-php-tools/phpstan-2.2.13.phar"),
        ),
        (
            "{composer}",
            "SUSPECT_COMPOSER_PHAR",
            run.original
                .join("target/sdk-php-tools/composer-2.10.3.phar"),
        ),
        (
            "{dart-floor}",
            "SUSPECT_DART_FLOOR_BIN",
            run.original.join("target/sdk-dart-tools/dart-sdk/bin/dart"),
        ),
        (
            "{dart-current}",
            "SUSPECT_DART_CURRENT_BIN",
            run.original
                .join("target/sdk-dart-tools/current-3.13.3/dart-sdk/bin/dart"),
        ),
        (
            "{cmake}",
            "SUSPECT_CPP_CMAKE",
            installs.join("cmake/3.31.6/cmake-3.31.6-macos-universal/CMake.app/Contents/bin/cmake"),
        ),
        (
            "{cxx}",
            "SUSPECT_CPP_CXX",
            PathBuf::from("/usr/bin/clang++"),
        ),
        (
            "{doxygen}",
            "SUSPECT_CPP_DOXYGEN",
            run.original
                .join("target/sdk-cpp-tools/doxygen/doxygen-1.18.0/doxygen"),
        ),
        (
            "{cargo-seed}",
            "SUSPECT_FULL_CARGO_CACHE",
            old_cargo.join("registry"),
        ),
        (
            "{go-seed}",
            "SUSPECT_FULL_GO_CACHE",
            home.join("go/pkg/mod"),
        ),
        ("{npm-seed}", "SUSPECT_FULL_NPM_CACHE", old_npm),
        ("{uv-seed}", "SUSPECT_FULL_UV_CACHE", old_uv),
        (
            "{java-maven-seed}",
            "SUSPECT_FULL_JAVA_MAVEN_CACHE",
            run.original
                .join("target/sdk-java-maven-cache/java/repository"),
        ),
        (
            "{kotlin-maven-seed}",
            "SUSPECT_FULL_KOTLIN_MAVEN_CACHE",
            run.original.join("target/sdk-kotlin-maven"),
        ),
    ] {
        run.replacements
            .insert(token.into(), selected(selector, default));
    }
    run.replacements
        .insert("{caller-home}".into(), home.display().to_string());
    for tier in ["floor", "current"] {
        let ruby = PathBuf::from(run.expand(&format!("{{ruby-{tier}}}")));
        // Ruby's default gem ABI directory differs from its patch version.
        let defaults = ruby.join("lib/ruby/gems");
        let entries = fs::read_dir(&defaults)
            .with_context(|| format!("installed Ruby default gems: {}", defaults.display()))?
            .map(|entry| Ok(entry?.path()))
            .collect::<Result<Vec<_>>>()?;
        let dirs = entries
            .into_iter()
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        ensure!(
            dirs.len() == 1,
            "ambiguous Ruby default gem ABI directory {}",
            defaults.display()
        );
        run.replacements.insert(
            format!("{{ruby-default-gems-{tier}}}"),
            dirs[0].display().to_string(),
        );
    }
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
    for (key, value) in [
        ("HOME", "{work}/home"),
        ("XDG_CACHE_HOME", "{work}/caches/xdg"),
        ("XDG_CONFIG_HOME", "{work}/home/.config"),
        ("CARGO_HOME", "{work}/cargo-home"),
        ("CARGO_NET_OFFLINE", "true"),
        ("CARGO_BUILD_JOBS", "2"),
        ("GOMODCACHE", "{work}/caches/go-mod"),
        ("GOCACHE", "{work}/caches/go-build"),
        ("GOPATH", "{work}/go"),
        ("GOPROXY", "off"),
        ("GOSUMDB", "off"),
        ("GOENV", "off"),
        ("UV_CACHE_DIR", "{work}/caches/uv"),
        ("UV_OFFLINE", "1"),
        ("UV_PYTHON_DOWNLOADS", "never"),
        (
            "UV_PYTHON_INSTALL_DIR",
            "{caller-home}/.local/share/uv/python",
        ),
        ("npm_config_cache", "{work}/caches/npm"),
        ("npm_config_offline", "true"),
        ("npm_config_userconfig", "{work}/home/.npmrc"),
        ("PYTHONNOUSERSITE", "1"),
        ("MYPY_CACHE_DIR", "{work}/caches/mypy"),
        (
            "OPENROUTER_PUBLIC_SCHEMA",
            "{out}/inputs/projects/docs/openapi/openapi.yaml",
        ),
        ("MAVEN_ARGS", "--offline --no-transfer-progress"),
        ("MAVEN_USER_HOME", "{work}/home/.m2"),
        ("JAVA_HOME", "{jdk-current}"),
        ("SUSPECT_JAVA_HOME", "{jdk-current}"),
        ("SUSPECT_MAVEN_BIN", "{maven}"),
        ("SUSPECT_KOTLIN_MAVEN", "{maven}"),
        ("SUSPECT_KOTLIN_JAVA_HOME", "{jdk-current}"),
        ("SUSPECT_DOTNET_BIN", "{dotnet}"),
        ("DOTNET_CLI_HOME", "{work}/home/dotnet"),
        ("DOTNET_CLI_TELEMETRY_OPTOUT", "1"),
        ("DOTNET_CLI_WORKLOAD_UPDATE_NOTIFY_DISABLE", "true"),
        ("DOTNET_NOLOGO", "1"),
        ("MSBUILDDISABLENODEREUSE", "1"),
        ("NUGET_PACKAGES", "{work}/caches/nuget"),
        ("SUSPECT_RUBY_HOME", "{ruby-current}"),
        ("SUSPECT_RUBY_GEMS", "{ruby-gems-current}"),
        ("SUSPECT_PHP_BIN", "{php-current}"),
        ("SUSPECT_PHPSTAN_PHAR", "{phpstan}"),
        ("SUSPECT_COMPOSER_PHAR", "{composer}"),
        ("COMPOSER_DISABLE_NETWORK", "1"),
        ("COMPOSER_NO_INTERACTION", "1"),
        ("COMPOSER_HOME", "{work}/home/composer"),
        ("SUSPECT_DART_BIN", "{dart-current}"),
        ("PUB_CACHE", "{work}/caches/pub"),
        ("DART_SUPPRESS_ANALYTICS", "true"),
        ("SUSPECT_CPP_CMAKE", "{cmake}"),
        ("SUSPECT_CPP_CXX", "{cxx}"),
        ("SUSPECT_CPP_DOXYGEN", "{doxygen}"),
        ("SUSPECT_SDK_FULL_PACKAGES", "{out}/generated"),
        ("SUSPECT_SDK_FULL_NATIVE_ROOT", "{work}/native"),
        ("SUSPECT_SDK_FULL_MEASUREMENTS", "{work}/native-costs"),
        ("SUSPECT_SDK_FULL_TARGETS", "{out}/configs/targets.json"),
        (
            "SUSPECT_SDK_FULL_CONFIG",
            "{out}/configs/five-operations.json",
        ),
        (
            "SUSPECT_SDK_FULL_PINNED_CONFIG",
            "{out}/configs/pinned-session.json",
        ),
        (
            "SUSPECT_SDK_FULL_EDITOR",
            "{work}/editor/dist/generation.js",
        ),
        ("SUSPECT_SDK_FULL_EDITOR_OUT", "{work}/editor-native"),
    ] {
        run.env.insert(key.into(), run.expand(value));
    }
    run.env
        .entry("RUSTUP_HOME".into())
        .or_insert_with(|| home.join(".rustup").display().to_string());
    let python_floor = PathBuf::from(run.expand("{python-floor}"));
    let python_current = PathBuf::from(run.expand("{python-current}"));
    let path = format!(
        "{}:{}:{}",
        python_current
            .parent()
            .context("current Python directory")?
            .display(),
        python_floor
            .parent()
            .context("floor Python directory")?
            .display(),
        run.env["PATH"]
    );
    run.env.insert("PATH".into(), path.clone());
    run.replacements.insert("{path}".into(), path);
    for directory in [
        "home",
        "home/.config",
        "home/.m2",
        "cargo-home",
        "caches",
        "performance",
    ] {
        fs::create_dir_all(run.work.join(directory))?;
    }
    for item in suites() {
        for key in [
            "TMPDIR",
            "SUSPECT_TEST_TMPDIR",
            "SUSPECT_DART_GATE_ROOT",
            "SUSPECT_SWIFT_PROTOCOL_ROOT",
        ] {
            if let Some(path) = item.environment.get(key) {
                fs::create_dir_all(run.expand(path))?;
            }
        }
    }
    // A private Cargo home has no ambient config; an empty npm user config is explicit.
    create(&run.work.join("home/.npmrc"))?;
    Ok(())
}

fn seed_caches(run: &mut Run) -> Result<Value> {
    let mut seeds = BTreeMap::new();
    for (id, token, destination) in [
        ("cargo", "{cargo-seed}", "cargo-home/registry"),
        ("go", "{go-seed}", "caches/go-mod"),
        ("npm", "{npm-seed}", "caches/npm"),
        ("uv", "{uv-seed}", "caches/uv"),
        (
            "java-maven",
            "{java-maven-seed}",
            "source/target/sdk-java-maven-cache/java/repository",
        ),
        (
            "kotlin-maven",
            "{kotlin-maven-seed}",
            "source/target/sdk-kotlin-maven",
        ),
    ] {
        let from = PathBuf::from(run.expand(token))
            .canonicalize()
            .with_context(|| format!("missing required {id} cache seed"))?;
        let files = inventory(&from, tree_names(&from)?.into_iter())?;
        ensure!(!files.is_empty(), "empty required {id} cache seed");
        let to = run.work.join(destination);
        ensure!(
            !to.exists(),
            "private cache destination already exists: {}",
            to.display()
        );
        copy_inventory(&from, &to, &files)?;
        write_json(&run.out.join(format!("cache-inputs/{id}.json")), &files)?;
        seeds.insert(id, json!({"source":from,"privateCopy":to,"manifest":format!("cache-inputs/{id}.json"),"fingerprint":sha(&serde_json::to_vec(&files)?),"originalWritableByRunner":false}));
    }
    // Old native helpers resolve tooling relative to CARGO_MANIFEST_DIR. Bind only
    // immutable tools there; Maven/Composer/Dart homes remain private writable copies.
    let python = PathBuf::from(run.expand("{python-tools}"));
    let venv = python
        .parent()
        .and_then(Path::parent)
        .context("Python tool venv")?;
    ensure!(
        venv.join("pyvenv.cfg").is_file(),
        "SUSPECT_PYTHON_TOOLS must select the prepared venv"
    );
    #[cfg(unix)]
    std::os::unix::fs::symlink(venv, run.work.join("source/target/sdk-native-python-tools"))?;
    run.remember(&venv.join("pyvenv.cfg"))?;
    for (token, name) in [
        ("{composer}", "composer-2.10.3.phar"),
        ("{phpstan}", "phpstan-2.2.13.phar"),
    ] {
        let from = PathBuf::from(run.expand(token));
        let to = run.work.join("source/target/sdk-php-tools").join(name);
        create(&to)?.write_all(&fs::read(&from)?)?;
        ensure!(hash(&from)? == hash(&to)?, "PHP tool changed during copy");
        run.remember(&from)?;
        run.remember(&to)?;
    }
    fs::create_dir_all(run.work.join("source/target/sdk-php-tools/composer-cache"))?;
    fs::create_dir_all(run.work.join("source/target/sdk-dart-tools/home"))?;
    for path in [
        run.work.join("cargo-home/config"),
        run.work.join("cargo-home/config.toml"),
    ] {
        ensure!(!path.exists(), "ambient Cargo config in private home");
        run.absent_configs.insert(path);
    }
    Ok(json!(seeds))
}

#[rustfmt::skip]
fn tool_stages() -> Vec<Stage> {
    let mut items = sealed::tool_stages();
    for item in &mut items {
        if item.id == "tool-go-current-version" { item.criterion = Criterion::Contains { text: "go1.27.1".into() }; }
    }
    for (id, program, args, text) in [
        ("tool-java-floor", "{jdk-floor}/bin/java", &["--version"][..], "21.0.12"),
        ("tool-java-current", "{jdk-current}/bin/java", &["--version"][..], "25.0.4"),
        ("tool-javac-floor", "{jdk-floor}/bin/javac", &["--version"][..], "21.0.12"),
        ("tool-javac-current", "{jdk-current}/bin/javac", &["--version"][..], "25.0.4"),
        ("tool-maven", "{maven}", &["--version"][..], "Apache Maven 3.9.16"),
        ("tool-dotnet", "{dotnet}", &["--list-sdks"][..], "8.0.424"),
        ("tool-dotnet-current", "{dotnet}", &["--list-sdks"][..], "10.0.400"),
        ("tool-ruby-floor", "{ruby-floor}/bin/ruby", &["--version"][..], "ruby 3.3.12"),
        ("tool-ruby-current", "{ruby-current}/bin/ruby", &["--version"][..], "ruby 4.0.6"),
        ("tool-php-floor", "{php-floor}", &["-n", "--version"][..], "PHP 8.3.32"),
        ("tool-php-current", "{php-current}", &["-n", "--version"][..], "PHP 8.5.8"),
        ("tool-composer", "{php-current}", &["-n", "{composer}", "--version"][..], "Composer version 2.10.3"),
        ("tool-phpstan", "{php-current}", &["-n", "{phpstan}", "--version"][..], "2.2.13"),
        ("tool-dart-floor", "{dart-floor}", &["--version"][..], "3.9.4"),
        ("tool-dart-current", "{dart-current}", &["--version"][..], "3.13.3"),
        ("tool-cmake", "{cmake}", &["--version"][..], "3.31.6"),
        ("tool-cxx", "{cxx}", &["--version"][..], "clang version 21.0.0"),
        ("tool-doxygen", "{doxygen}", &["--version"][..], "1.18.0"),
        ("tool-acquire-curl", "/usr/bin/curl", &["--version"][..], "curl "),
        ("tool-openssl", "openssl", &["version", "-a"][..], "SSL"),
        ("tool-tar", "tar", &["--version"][..], "tar"),
        ("tool-unzip", "unzip", &["-v"][..], "UnZip"),
    ] {
        let mut item = command(id, program, args, "{workspace}");
        item.criterion = Criterion::Contains { text: text.into() };
        items.push(item);
    }
    for tier in ["floor", "current"] {
        let mut item = command(&format!("tool-ruby-{tier}-packages"), &format!("{{ruby-{tier}}}/bin/ruby"), &["-rjson", "-e", "pins={'yard'=>'0.9.37','redcarpet'=>'3.6.1','rbs'=>'3.9.5','steep'=>'1.10.0'}; actual=pins.keys.to_h{|name| [name,Gem::Specification.find_by_name(name,pins[name]).version.to_s]}; abort('tool gem mismatch') unless actual==pins; puts JSON.generate(actual)"], "{workspace}");
        item.environment = native_environment("ruby", tier);
        item.criterion = Criterion::Json { report: None, assertions: BTreeMap::from([("/yard".into(), json!("0.9.37")), ("/redcarpet".into(), json!("3.6.1")), ("/rbs".into(), json!("3.9.5")), ("/steep".into(), json!("1.10.0"))]) };
        items.push(item);
    }
    items
}

fn external_tree(root: &Path) -> Result<Value> {
    ensure!(
        root.is_dir(),
        "missing installed tool payload {}",
        root.display()
    );
    fn walk(
        root: &Path,
        directory: &Path,
        ancestors: &mut BTreeSet<PathBuf>,
        files: &mut BTreeMap<String, Value>,
    ) -> Result<()> {
        let real = directory.canonicalize()?;
        ensure!(
            ancestors.insert(real.clone()),
            "cyclic installed tool directory link: {}",
            directory.display()
        );
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            let metadata = fs::symlink_metadata(&path)?;
            let canonical = path
                .canonicalize()
                .with_context(|| format!("broken tool link {}", path.display()))?;
            let name = path
                .strip_prefix(root)?
                .to_str()
                .context("tool payload UTF-8")?
                .to_owned();
            let link = if metadata.file_type().is_symlink() {
                Some(fs::read_link(&path)?)
            } else {
                None
            };
            if canonical.is_dir() {
                if link.is_some() {
                    files.insert(
                        name,
                        json!({"kind":"directory-link","canonicalPath":canonical,"link":link}),
                    );
                }
                // .NET's installed root and JDK legal trees use directory links.
                // Pin their actual descendants, not just a mutable link destination.
                walk(root, &path, ancestors, files)?;
            } else {
                ensure!(
                    canonical.is_file(),
                    "nonregular tool payload {}",
                    path.display()
                );
                files.insert(name, json!({"kind":"file","canonicalPath":canonical,"link":link,"sha256":hash(&canonical)?,"bytes":fs::metadata(&canonical)?.len()}));
            }
        }
        ancestors.remove(&real);
        Ok(())
    }
    let mut files = BTreeMap::new();
    walk(root, root, &mut BTreeSet::new(), &mut files)?;
    ensure!(
        !files.is_empty(),
        "empty installed tool payload {}",
        root.display()
    );
    Ok(json!({"root":root,"files":files}))
}

fn full_tool_inputs(run: &mut Run) -> Result<Value> {
    let mut implementation_roots = Vec::new();
    for tier in ["floor", "current"] {
        let config: Value = serde_json::from_slice(&fs::read(
            run.out
                .join(format!("logs/tool-go-{tier}-config.stdout.log")),
        )?)?;
        let go = PathBuf::from(
            config["GOROOT"]
                .as_str()
                .context("actual Go toolchain root")?,
        )
        .join("bin");
        ensure!(
            go.join("go").is_file(),
            "resolved Go implementation missing"
        );
        run.replacements
            .insert(format!("{{go-{tier}-bin}}"), go.display().to_string());
        implementation_roots.push(go.parent().context("Go payload root")?.to_owned());
        let swift = PathBuf::from(run.expand(if tier == "floor" {
            "{swift-floor}"
        } else {
            "{swift}"
        }));
        let compiler = PathBuf::from(run.expand(if tier == "floor" {
            "{swiftc-floor}"
        } else {
            "{swiftc}"
        }));
        ensure!(
            swift
                .parent()
                .context("Swift driver directory")?
                .canonicalize()?
                == compiler
                    .parent()
                    .context("Swift compiler directory")?
                    .canonicalize()?,
            "Swift driver/compiler must belong to the same {tier} toolchain"
        );
        let sdk = PathBuf::from(run.expand(if tier == "floor" {
            "{swift-floor-sdk}"
        } else {
            "{swift-sdk}"
        }));
        let settings: Value = serde_json::from_slice(&fs::read(sdk.join("SDKSettings.json"))?)?;
        ensure!(
            settings["Version"] == if tier == "floor" { "15.4" } else { "26.5" },
            "the maintained Swift {tier} matrix must use its declared actual SDK version"
        );
        run.replacements.insert(
            format!("{{swift-{tier}-bin}}"),
            swift
                .parent()
                .context("Swift implementation parent")?
                .display()
                .to_string(),
        );
        implementation_roots.push(
            swift
                .parent()
                .and_then(Path::parent)
                .context("Swift payload root")?
                .join("lib"),
        );
        implementation_roots.push(
            swift
                .parent()
                .context("Swift/LLVM executable root")?
                .to_owned(),
        );
        let rust = fs::read_to_string(
            run.out
                .join(format!("logs/tool-rustc-{tier}-path.stdout.log")),
        )?;
        implementation_roots.push(
            Path::new(rust.trim())
                .parent()
                .and_then(Path::parent)
                .context("Rust payload root")?
                .join("lib"),
        );
    }
    let python = PathBuf::from(run.expand("{python-tools}"));
    let venv = python
        .parent()
        .and_then(Path::parent)
        .context("Python tools")?
        .to_owned();
    let common = sealed::tool_inputs(run, &venv)?;
    let mut roots = [
        "{jdk-floor}",
        "{jdk-current}",
        "{ruby-floor}",
        "{ruby-current}",
        "{ruby-gems-floor}",
        "{ruby-gems-current}",
    ]
    .into_iter()
    .map(|token| PathBuf::from(run.expand(token)))
    .collect::<Vec<_>>();
    roots.extend(implementation_roots);
    for token in ["{python-floor}", "{python-current}"] {
        let binary = PathBuf::from(run.expand(token)).canonicalize()?;
        roots.push(
            binary
                .parent()
                .and_then(Path::parent)
                .context("Python standard library root")?
                .join("lib"),
        );
    }
    let node = PathBuf::from(run.expand("{node22}"));
    roots.push(
        node.parent()
            .and_then(Path::parent)
            .context("npm implementation root")?
            .join("lib/node_modules/npm"),
    );
    for token in [
        "{dotnet}",
        "{maven}",
        "{dart-floor}",
        "{dart-current}",
        "{cmake}",
        "{doxygen}",
    ] {
        let path = PathBuf::from(run.expand(token));
        roots.push(
            if token == "{dotnet}" || token == "{doxygen}" {
                path.parent().context("tool executable root")?
            } else {
                path.parent()
                    .and_then(Path::parent)
                    .context("tool payload root")?
            }
            .to_owned(),
        );
    }
    let mut payloads = Vec::new();
    for root in roots {
        payloads.push(external_tree(&root)?);
    }
    for token in [
        "{php-floor}",
        "{php-current}",
        "{composer}",
        "{phpstan}",
        "{cmake}",
        "{cxx}",
        "{doxygen}",
    ] {
        run.remember(Path::new(&run.expand(token)))?;
    }
    for tier in ["floor", "current"] {
        let jdk = PathBuf::from(run.expand(&format!("{{jdk-{tier}}}")));
        for tool in ["java", "javac", "jar", "javadoc"] {
            run.remember(&jdk.join("bin").join(tool))?;
        }
    }
    for name in ["CC", "CXX", "AR"] {
        if let Some(tool) = run.env.get(name) {
            run.remember(&sealed::resolve(tool, &run.env))?;
        }
    }
    let cmake = PathBuf::from(run.expand("{cmake}"));
    run.remember(&cmake.with_file_name("ctest"))?;
    write_json(&run.out.join("tool-payloads.json"), &payloads)?;
    write_json(
        &run.out.join("full-tool-selection.json"),
        &json!({"selectors":run.replacements,"environment":run.env,"features":FEATURES}),
    )?;
    Ok(
        json!({"common":common,"payloads":"tool-payloads.json","payloadSha256":hash(&run.out.join("tool-payloads.json"))?,"selectors":"full-tool-selection.json"}),
    )
}

fn native_root(language: &str, tier: &str) -> String {
    format!("{{work}}/native/{language}/{tier}")
}

#[rustfmt::skip]
fn package_stages() -> Vec<Stage> {
    let mut items = original_package_stages();
    for tier in ["floor", "current"] {
        for language in ["java", "kotlin", "csharp", "ruby", "php", "dart", "typescript", "rust", "go"] {
            let root = native_root(language, tier);
            let sdk = format!("{root}/sdk");
            let consumer = format!("{root}/consumer");
            let env = native_environment(language, tier);
            let mut add = |name: &str, program: &str, args: &[&str], cwd: &str| {
                let mut item = command(&format!("install-{language}-{tier}-{name}"), program, args, cwd);
                item.environment = env.clone();
                items.push(item);
            };
            match language {
                "java" | "kotlin" => {
                    let repository = if language == "java" { "{workspace}/target/sdk-java-maven-cache/java/repository" } else { "{workspace}/target/sdk-kotlin-maven" };
                    add("maven", "{maven}", &["--offline", "-B", "--no-transfer-progress", &format!("-Dmaven.repo.local={repository}"), "install"], &sdk);
                    add("jar", &format!("{{jdk-{tier}}}/bin/jar"), &["--list", "--file", &format!("{sdk}/target/sdk-full-0.0.0.jar")], &root);
                    if language == "java" {
                        add("examples", &format!("{{jdk-{tier}}}/bin/java"), &["-ea", "-cp", &format!("{root}/installed/sdk-full-0.0.0.jar"), "com.example.generated.SdkExamples"], &root);
                        add("consumer-compile", &format!("{{jdk-{tier}}}/bin/javac"), &["--release", "21", "-Xlint:all", "-Werror", "-cp", "../installed/sdk-full-0.0.0.jar", "Consumer.java"], &consumer);
                        add("consumer-run", &format!("{{jdk-{tier}}}/bin/java"), &["-ea", "-cp", "../installed/sdk-full-0.0.0.jar:.", "Consumer"], &consumer);
                    } else {
                        add("consumer", "{maven}", &["--offline", "-B", "--no-transfer-progress", &format!("-Dmaven.repo.local={repository}"), "compile", "exec:exec"], &consumer);
                    }
                }
                "csharp" => {
                    add("restore", "{dotnet}", &["restore", "Suspect.csproj", "--configfile", "../NuGet.Config"], &sdk);
                    add("pack", "{dotnet}", &["pack", "Suspect.csproj", "-c", "Release", "--no-restore", "-o", "../feed", "--include-source", "--include-symbols", "-m:1"], &sdk);
                    add("examples-restore", "{dotnet}", &["restore", "--configfile", "../../NuGet.Config"], &format!("{sdk}/examples"));
                    add("examples-run", "{dotnet}", &["run", "-c", "Release", "--no-restore"], &format!("{sdk}/examples"));
                    for item in items.iter_mut().rev().take(4) {
                        item.environment.insert("NUGET_PACKAGES".into(), format!("{root}/installed"));
                        item.environment.insert("DOTNET_CLI_HOME".into(), format!("{root}/home"));
                    }
                }
                "ruby" => {
                    let ruby = format!("{{ruby-{tier}}}/bin/ruby");
                    let gem = format!("{{ruby-{tier}}}/bin/gem");
                    let installed = format!("{root}/installed/gems/sdk-full-0.0.0");
                    add("gem-build", &ruby, &[&gem, "build", "sdk-full.gemspec"], &sdk);
                    add("gem-install", &ruby, &[&gem, "install", "--local", "--no-document", "--install-dir", &format!("{root}/installed"), &format!("{sdk}/sdk-full-0.0.0.gem")], &root);
                    add("rbs", &ruby, &[&format!("{{ruby-gems-{tier}}}/bin/rbs"), "-I", &format!("{installed}/sig"), "validate"], &root);
                    add("yard", &ruby, &[&format!("{{ruby-gems-{tier}}}/bin/yard"), "doc"], &installed);
                    add("examples", &ruby, &[&format!("{installed}/examples/contract_examples.rb")], &root);
                    items.last_mut().unwrap().environment.extend([
                        ("GEM_HOME".into(), format!("{root}/installed")),
                        ("GEM_PATH".into(), format!("{root}/installed:{{ruby-default-gems-{tier}}}")),
                    ]);
                }
                "php" => {
                    let php = format!("{{php-{tier}}}");
                    add("validate", &php, &["-n", "{composer}", "validate", "--no-check-publish"], &sdk);
                    add("archive", &php, &["-n", "{composer}", "archive", "--format=zip", "--dir=build", "--file=package"], &sdk);
                    add("composer", &php, &["-n", "{composer}", "install", "--no-dev", "--no-plugins", "--no-scripts", "--no-progress"], &consumer);
                    add("types", &php, &["-n", "{phpstan}", "analyse", "--no-progress", "--memory-limit=1G", "--autoload-file", &format!("{consumer}/vendor/autoload.php")], &format!("{consumer}/vendor/example/sdk-full"));
                    for name in ["codecs", "client", "quickstart"] {
                        add(name, &php, &["-n", "-d", "error_reporting=-1", &format!("{consumer}/vendor/example/sdk-full/examples/{name}.php")], &consumer);
                    }
                    for item in items.iter_mut().rev().take(7) {
                        item.environment.insert("SUSPECT_SDK_AUTOLOAD".into(), format!("{consumer}/vendor/autoload.php"));
                        item.environment.insert("COMPOSER_HOME".into(), format!("{root}/composer-home"));
                        item.environment.insert("COMPOSER_CACHE_DIR".into(), format!("{root}/composer-cache"));
                    }
                }
                "dart" => {
                    let dart = format!("{{dart-{tier}}}");
                    // Archive extraction is the local pub consumer seam here. The
                    // required dart_sdk suite additionally proves hosted-pub install.
                    add("archive", "tar", &["-czf", &format!("{root}/sdk.tar.gz"), "-C", &sdk, "."], &root);
                    add("extract", "tar", &["-xzf", &format!("{root}/sdk.tar.gz"), "-C", &format!("{root}/installed")], &root);
                    add("pub", &dart, &["pub", "get", "--offline"], &consumer);
                    add("analyze", &dart, &["analyze", "--fatal-infos"], &consumer);
                    add("compile", &dart, &["compile", "exe", "bin/main.dart", "-o", &format!("{root}/consumer-bin")], &consumer);
                    add("run", &format!("{root}/consumer-bin"), &[], &root);
                    add("sdk-pub", &dart, &["pub", "get", "--offline"], &format!("{root}/installed"));
                    add("docs", &dart, &["doc", "--validate-links", "--output", &format!("{root}/docs")], &format!("{root}/installed"));
                    for item in items.iter_mut().rev().take(8) { item.environment.insert("PUB_CACHE".into(), format!("{root}/pub-cache")); }
                }
                "typescript" => {
                    let node = if tier == "floor" { "{node22}" } else { "{node24}" };
                    add("npm", "{npm}", &["install", "--offline", "--ignore-scripts", "--no-audit", "--no-fund", "--save-exact", "{work}/packages/typescript/suspect-fixtures-sdk-full-0.0.0.tgz"], &consumer);
                    add("import", node, &["--input-type=module", "-e", "import assert from 'node:assert/strict'; import {createClient,JsonNumber} from '@suspect-fixtures/sdk-full'; assert.equal(typeof createClient,'function'); assert.equal(JsonNumber.parse('1.0001').toString(),'1.0001'); console.log('SDK_FULL_INSTALLED_TYPESCRIPT');"], &consumer);
                }
                "rust" => {
                    add("extract", "tar", &["-xzf", &format!("{{work}}/package-rust-{tier}/package/sdk-full-0.0.0.crate"), "-C", &format!("{root}/installed")], &root);
                    add("consumer", "cargo", &["run", "--offline", "--quiet", "--manifest-path", &format!("{consumer}/Cargo.toml"), "--target-dir", &format!("{root}/consumer-target")], &root);
                    add("examples", "cargo", &["run", "--offline", "--example", "validated", "--all-features", "--manifest-path", &format!("{root}/installed/sdk-full-0.0.0/Cargo.toml"), "--target-dir", &format!("{root}/consumer-target")], &root);
                }
                "go" => {
                    add("archive", "tar", &["-czf", &format!("{root}/sdk.tar.gz"), "-C", "{work}/packages/go", "."], &root);
                    add("extract", "tar", &["-xzf", &format!("{root}/sdk.tar.gz"), "-C", &format!("{root}/installed")], &root);
                    add("consumer", "go", &["run", "."], &consumer);
                    add("module", "go", &["list", "-m", "-json", "all"], &consumer);
                }
                _ => unreachable!(),
            }
        }
    }
    let root = native_root("cpp", "declared");
    for (id, program, args) in [
        ("configure", "{cmake}", vec!["-S".into(), format!("{root}/sdk"), "-B".into(), format!("{root}/build"), "-DCMAKE_BUILD_TYPE=Release".into(), "-DCMAKE_CXX_COMPILER={cxx}".into(), "-DCMAKE_OSX_SYSROOT={swift-sdk}".into(), format!("-DCMAKE_INSTALL_PREFIX={root}/installed"), "-DSUSPECT_SDK_BUILD_DOCS=ON".into(), "-DDOXYGEN_EXECUTABLE={doxygen}".into()]),
        ("build", "{cmake}", vec!["--build".into(), format!("{root}/build"), "--parallel".into(), "2".into()]),
        ("test", "{ctest}", vec!["--test-dir".into(), format!("{root}/build"), "--output-on-failure".into(), "--no-tests=error".into()]),
        ("docs", "{cmake}", vec!["--build".into(), format!("{root}/build"), "--target".into(), "sdk_docs".into()]),
        ("cmake", "{cmake}", vec!["--install".into(), format!("{root}/build")]),
        ("consumer-configure", "{cmake}", vec!["-S".into(), format!("{root}/consumer"), "-B".into(), format!("{root}/consumer-build"), "-DCMAKE_CXX_COMPILER={cxx}".into(), "-DCMAKE_OSX_SYSROOT={swift-sdk}".into(), "-DCMAKE_FIND_USE_PACKAGE_REGISTRY=OFF".into(), format!("-DCMAKE_PREFIX_PATH={root}/installed")]),
        ("consumer-build", "{cmake}", vec!["--build".into(), format!("{root}/consumer-build")]),
        ("consumer-run", "{work}/native/cpp/declared/consumer-build/consumer", vec![]),
    ] {
        let mut item = command(&format!("install-cpp-declared-{id}"), program, &[], &root);
        item.args = args;
        items.push(item);
    }
    items.push(command("package-typescript-rendered-docs", "{node22}", &["{workspace}/crates/suspect-codegen/tools/typescript-docs/node_modules/typedoc/bin/typedoc", "--options", "typedoc.json", "--out", "{work}/native/typescript/docs", "--treatWarningsAsErrors", "--treatValidationWarningsAsErrors"], "{work}/packages/typescript"));
    for tier in ["floor", "current"] {
        for language in ["python", "go"] {
            let root = native_root(language, tier);
            let sdk = if language == "python" { "{work}/packages/python".to_owned() } else { format!("{root}/installed") };
            if language == "python" {
                items.push(command(&format!("install-python-{tier}-site"), &format!("{{work}}/python-{tier}/bin/python"), &["-c", "import site; print(site.getsitepackages()[0])"], "{work}"));
                items.push(command(&format!("install-python-{tier}-examples"), &format!("{{work}}/python-{tier}/bin/python"), &["{work}/packages/python/examples/validated.py"], "{work}"));
            }
            let mut docs = command(&format!("install-{language}-{tier}-docs"), "{python-tools}", &["-m", "sphinx", "-W", "--keep-going", "-E", "-b", "html", "docs", &format!("{root}/docs")], &sdk);
            docs.environment = native_environment(language, tier);
            if language == "python" { docs.environment.insert("PYTHONPATH".into(), format!("{{python-site-{tier}}}")); }
            items.push(docs);
        }
        let root = native_root("swift", tier);
        let swift = if tier == "floor" { "{swift-floor}" } else { "{swift}" };
        let sdk = if tier == "floor" { "{swift-floor-sdk}" } else { "{swift-sdk}" };
        let docc = if tier == "floor" { "{docc-floor}" } else { "{docc}" };
        for (name, program, args) in [
            ("archive", "tar", vec!["-czf".into(), format!("{root}/sdk.tar.gz"), "-C".into(), "{out}/generated/swift".into(), ".".into()]),
            ("extract", "tar", vec!["-xzf".into(), format!("{root}/sdk.tar.gz"), "-C".into(), format!("{root}/installed")]),
            ("consumer", swift, vec!["run".into(), "--package-path".into(), format!("{root}/consumer"), "--scratch-path".into(), format!("{root}/build"), "--cache-path".into(), format!("{root}/cache"), "--sdk".into(), sdk.into(), "-Xswiftc".into(), "-warnings-as-errors".into()]),
            ("symbolgraph", swift, vec!["package".into(), "--package-path".into(), format!("{root}/installed"), "--scratch-path".into(), format!("{root}/docs-build"), "--sdk".into(), sdk.into(), "dump-symbol-graph".into(), "--minimum-access-level".into(), "public".into()]),
            ("docs", docc, vec!["convert".into(), format!("{root}/installed/Sources/SdkFull/SdkFull.docc"), "--additional-symbol-graph-dir".into(), format!("{{swift-graph-{tier}}}"), "--output-path".into(), format!("{root}/SdkFull.doccarchive"), "--warnings-as-errors".into()]),
        ] {
            let mut item = command(&format!("install-swift-{tier}-{name}"), program, &[], &root);
            item.args = args;
            item.environment = native_environment("swift", tier);
            items.push(item);
        }
    }
    items
}

fn target_config() -> Vec<Value> {
    TARGETS.iter().map(|target| json!({"backend":target.backend,"package_name":target.package_name,"package_version":"0.0.0","import_name":target.import_name})).collect()
}

fn verify_package_identity(target: &Target, root: &Path) -> Result<()> {
    let contents = fs::read_to_string(root.join(target.manifest))
        .with_context(|| format!("{} package manifest", target.language))?;
    match target.language {
        "typescript" | "php" => {
            let value: Value = serde_json::from_str(&contents)?;
            ensure!(
                value["name"] == target.package_name && value["version"] == "0.0.0",
                "{} package identity differs",
                target.language
            );
        }
        "java" | "kotlin" => {
            let (group, artifact) = target
                .package_name
                .split_once(':')
                .context("Maven coordinates")?;
            ensure!(
                contents.contains(&format!("<groupId>{group}</groupId>"))
                    && contents.contains(&format!("<artifactId>{artifact}</artifactId>"))
                    && contents.contains("<version>0.0.0</version>"),
                "{} Maven identity differs",
                target.language
            );
        }
        _ => ensure!(
            contents.contains(target.package_name)
                && (contents.contains("0.0.0")
                    || target.language == "go"
                    || target.language == "swift"),
            "{} native identity missing from manifest",
            target.language
        ),
    }
    ensure!(
        root.join("README.md").is_file(),
        "{} README missing",
        target.language
    );
    Ok(())
}

fn generated_packages(run: &mut Run) -> Result<Value> {
    ensure!(
        met(run, "twelve-five-op-generate") && met(run, "twelve-five-op-drift"),
        "twelve-target session did not generate current artifacts"
    );
    let root = run.work.join("packages");
    let files = inventory(&root, tree_names(&root)?.into_iter())?;
    let mut roots = BTreeMap::new();
    for target in TARGETS {
        let package = root.join(target.language);
        verify_package_identity(target, &package)?;
        let package_files = files
            .keys()
            .filter(|name| name.starts_with(&format!("{}/", target.language)))
            .collect::<Vec<_>>();
        ensure!(
            package_files.len() > 3,
            "{} generation produced only placeholder files",
            target.language
        );
        roots.insert(target.language, json!({"backend":target.backend,"packageName":target.package_name,"importName":target.import_name,"version":"0.0.0","root":format!("generated/{}", target.language),"manifest":target.manifest,"artifactCount":package_files.len(),"artifacts":package_files}));
    }
    copy_inventory(&root, &run.out.join("generated"), &files)?;
    write_json(&run.out.join("generated-manifest.json"), &files)?;
    write_json(&run.out.join("package-roots.json"), &roots)?;
    run.replacements.insert(
        "{swift-package}".into(),
        root.join("swift").display().to_string(),
    );
    Ok(
        json!({"packages":roots,"manifest":"generated-manifest.json","sha256":hash(&run.out.join("generated-manifest.json"))?}),
    )
}

fn prepare_consumers(run: &mut Run) -> Result<Value> {
    for target in TARGETS {
        let tiers = if target.language == "cpp" {
            &["declared"][..]
        } else {
            &["floor", "current"][..]
        };
        for tier in tiers {
            let root = PathBuf::from(run.expand(&native_root(target.language, tier)));
            fs::create_dir_all(root.join("consumer"))?;
            fs::create_dir_all(root.join("installed"))?;
            if ["java", "kotlin", "csharp", "ruby", "php", "dart", "cpp"].contains(&target.language)
            {
                let from = run.work.join("packages").join(target.language);
                let files = inventory(&from, tree_names(&from)?.into_iter())?;
                copy_inventory(&from, &root.join("sdk"), &files)?;
            }
            match target.language {
                "java" => create(&root.join("consumer/Consumer.java"))?.write_all(b"import com.example.generated.JsonRuntime;\npublic class Consumer { public static void main(String[] args) { if (JsonRuntime.JsonNumber.parse(\"1.0001\")==null) throw new AssertionError(); System.out.println(\"SDK_FULL_INSTALLED_JAVA\"); } }\n")?,
                "kotlin" => {
                    create(&root.join("consumer/pom.xml"))?.write_all(b"<project><modelVersion>4.0.0</modelVersion><groupId>test.suspect</groupId><artifactId>sdk-full-consumer</artifactId><version>0.0.0</version><properties><project.build.sourceEncoding>UTF-8</project.build.sourceEncoding><kotlin.compiler.daemon>false</kotlin.compiler.daemon></properties><dependencies><dependency><groupId>com.example</groupId><artifactId>sdk-full</artifactId><version>0.0.0</version></dependency></dependencies><build><sourceDirectory>src/main/kotlin</sourceDirectory><plugins><plugin><groupId>org.jetbrains.kotlin</groupId><artifactId>kotlin-maven-plugin</artifactId><version>2.4.20</version><configuration><jvmTarget>21</jvmTarget><args><arg>-Werror</arg></args></configuration><executions><execution><phase>compile</phase><goals><goal>compile</goal></goals></execution></executions></plugin><plugin><groupId>org.codehaus.mojo</groupId><artifactId>exec-maven-plugin</artifactId><version>3.6.3</version><configuration><executable>${java.home}/bin/java</executable><arguments><argument>-cp</argument><classpath/><argument>consumer.Entry</argument></arguments></configuration></plugin></plugins></build></project>\n")?;
                    create(&root.join("consumer/src/main/kotlin/Entry.kt"))?.write_all(b"package consumer\nimport example.sdk.*\nobject Entry { @JvmStatic fun main(args: Array<String>) { check(JsonNumber.parse(\"1.0001\").token == \"1.0001\"); println(\"SDK_FULL_INSTALLED_KOTLIN\") } }\n")?;
                }
                "csharp" => {
                    let version = if *tier == "floor" { "8.0.424" } else { "10.0.400" };
                    write_json(&root.join("global.json"), &json!({"sdk":{"version":version,"rollForward":"disable"}}))?;
                    fs::create_dir(root.join("feed"))?;
                    create(&root.join("NuGet.Config"))?.write_all(b"<configuration><packageSources><clear/><add key=\"local\" value=\"feed\"/></packageSources><packageSourceMapping><packageSource key=\"local\"><package pattern=\"*\"/></packageSource></packageSourceMapping></configuration>\n")?;
                }
                "php" => {
                    let mut package: Value = serde_json::from_slice(&fs::read(root.join("sdk/composer.json"))?)?;
                    // Composer accepts an absolute local dist path; no network or
                    // rewritten generated source is needed for package installation.
                    package["dist"] = json!({"type":"zip","url":root.join("sdk/build/package.zip")});
                    write_json(&root.join("consumer/composer.json"), &json!({"name":"suspect/acceptance-consumer","description":"Installed SDK acceptance consumer","license":"proprietary","repositories":[{"type":"package","package":package},{"packagist.org":false}],"require":{target.package_name:"0.0.0"},"config":{"allow-plugins":false}}))?;
                }
                "typescript" => write_json(&root.join("consumer/package.json"), &json!({"name":"sdk-full-consumer","version":"0.0.0","private":true,"type":"module"}))?,
                "dart" => {
                    create(&root.join("consumer/pubspec.yaml"))?.write_all(b"name: sdk_full_consumer\nversion: 0.0.0\npublish_to: none\nenvironment:\n  sdk: '>=3.9.4 <4.0.0'\ndependencies:\n  sdk_full:\n    path: ../installed\n")?;
                    create(&root.join("consumer/bin/main.dart"))?.write_all(b"import 'package:sdk_full/sdk_full.dart';\nvoid main() { if (JsonNumber.parse('1.0001').token != '1.0001') { throw StateError('exact JSON import'); } print('SDK_FULL_INSTALLED_DART'); }\n")?;
                }
                "rust" => {
                    create(&root.join("consumer/Cargo.toml"))?.write_all(b"[package]\nname='sdk-full-consumer'\nversion='0.0.0'\nedition='2024'\n[workspace]\n[dependencies]\nsdk-full={path='../installed/sdk-full-0.0.0',features=['http']}\n")?;
                    create(&root.join("consumer/src/main.rs"))?.write_all(b"use sdk_full as _;\nfn main() { println!(\"SDK_FULL_INSTALLED_RUST\"); }\n")?;
                }
                "go" => {
                    create(&root.join("consumer/go.mod"))?.write_all(b"module example.com/sdk-full-consumer\n\ngo 1.23\n\nrequire example.com/sdk-full v0.0.0\nreplace example.com/sdk-full => ../installed\n")?;
                    create(&root.join("consumer/main.go"))?.write_all(b"package main\nimport (_ \"example.com/sdk-full\"; \"fmt\")\nfunc main(){fmt.Println(\"SDK_FULL_INSTALLED_GO\")}\n")?;
                }
                "cpp" => {
                    create(&root.join("consumer/CMakeLists.txt"))?.write_all(b"cmake_minimum_required(VERSION 3.24)\nproject(SdkFullConsumer LANGUAGES CXX)\nfind_package(sdk_full 0.0.0 EXACT CONFIG REQUIRED)\nadd_executable(consumer main.cpp)\ntarget_link_libraries(consumer PRIVATE sdk_full::sdk_full)\ntarget_compile_options(consumer PRIVATE -Wall -Wextra -Wpedantic -Werror)\n")?;
                    create(&root.join("consumer/main.cpp"))?.write_all(b"#include <sdk_full/sdk.hpp>\n#include <curl/curl.h>\n#include <iostream>\nint main(){ auto value=sdk_full::JsonNumber::parse(\"1.0001\"); if(!value)return 1; auto info=curl_version_info(CURLVERSION_NOW); if(info->version_num<0x075500 || !(info->features&CURL_VERSION_SSL) || !(info->features&CURL_VERSION_ASYNCHDNS) || !(info->features&CURL_VERSION_THREADSAFE))return 2; std::cout << \"SDK_FULL_INSTALLED_CPP \" << __cplusplus << \" curl=\" << info->version << \" tls=\" << info->ssl_version << '\\n'; }\n")?;
                }
                "swift" => {
                    create(&root.join("consumer/Package.swift"))?.write_all(b"// swift-tools-version: 6.0\nimport PackageDescription\nlet package = Package(name: \"SdkFullConsumer\", platforms: [.macOS(.v13)], dependencies: [.package(name: \"SdkFull\", path: \"../installed\")], targets: [.executableTarget(name: \"Consumer\", dependencies: [.product(name: \"SdkFull\", package: \"SdkFull\")])])\n")?;
                    create(&root.join("consumer/Sources/Consumer/main.swift"))?.write_all(b"import SdkFull\nlet number = try JsonNumber(\"1.0001\")\nprecondition(number.raw == \"1.0001\")\nprint(\"SDK_FULL_INSTALLED_SWIFT\")\n")?;
                }
                _ => {}
            }
        }
    }
    let cmake = PathBuf::from(run.expand("{cmake}"));
    run.replacements.insert(
        "{ctest}".into(),
        cmake.with_file_name("ctest").display().to_string(),
    );
    Ok(
        json!({"root":run.work.join("native"),"inputs":"generated/","configuration":"maintained native consumers in xtask/src/sdk_full.rs","repairsToEmittedSources":false}),
    )
}

fn capture_maven_install(run: &mut Run, language: &str, tier: &str) -> Result<Value> {
    ensure!(
        met(run, &format!("install-{language}-{tier}-maven")),
        "Maven install failed"
    );
    let root = PathBuf::from(run.expand(&native_root(language, tier)));
    let repository = run.work.join(if language == "java" {
        "source/target/sdk-java-maven-cache/java/repository/com/example/generated/sdk-full/0.0.0"
    } else {
        "source/target/sdk-kotlin-maven/com/example/sdk-full/0.0.0"
    });
    let suffixes = if language == "java" {
        &[".jar", "-sources.jar", "-javadoc.jar"][..]
    } else {
        &[".jar", "-sources.jar", "-javadoc.jar", "-examples.jar"][..]
    };
    let mut artifacts = Vec::new();
    for suffix in suffixes {
        let name = format!("sdk-full-0.0.0{suffix}");
        let built = root.join("sdk/target").join(&name);
        let installed = repository.join(&name);
        let bytes = fs::read(&installed)?;
        ensure!(
            bytes.starts_with(b"PK") && hash(&built)? == sha(&bytes),
            "installed Maven jar differs from build or is not a jar: {name}"
        );
        let snapshot = root.join("installed").join(name);
        create(&snapshot)?.write_all(&bytes)?;
        artifacts.push(
            json!({"built":built,"installed":installed,"snapshot":snapshot,"sha256":sha(&bytes)}),
        );
    }
    Ok(json!(artifacts))
}

fn package_hook(run: &mut Run, id: &str) -> Result<()> {
    for tier in ["floor", "current"] {
        for language in ["java", "kotlin"] {
            if id == format!("install-{language}-{tier}-maven") {
                let result = capture_maven_install(run, language, tier);
                verification(run, &format!("install-{language}-{tier}-snapshot"), result)?;
            }
        }
        if id == format!("install-python-{tier}-site") {
            let result = (|| -> Result<Value> {
                ensure!(met(run, id), "Python site query failed");
                let text = fs::read_to_string(run.out.join(format!("logs/{id}.stdout.log")))?;
                let site = PathBuf::from(text.trim()).canonicalize()?;
                ensure!(
                    site.starts_with(run.work.join(format!("python-{tier}")))
                        && site.join("sdk_full/__init__.py").is_file(),
                    "Python import must resolve inside the installed wheel environment"
                );
                run.replacements.insert(
                    format!("{{python-site-{tier}}}"),
                    site.display().to_string(),
                );
                Ok(json!({"site":site,"wheelEnvironment":run.work.join(format!("python-{tier}"))}))
            })();
            verification(run, &format!("install-python-{tier}-site-identity"), result)?;
        }
        if id == format!("install-swift-{tier}-symbolgraph") {
            let result = (|| -> Result<Value> {
                ensure!(met(run, id), "Swift public symbol graph extraction failed");
                let root =
                    PathBuf::from(run.expand(&native_root("swift", tier))).join("docs-build");
                let graphs = tree_names(&root)?
                    .into_iter()
                    .filter(|name| name.ends_with("SdkFull.symbols.json"))
                    .collect::<Vec<_>>();
                ensure!(
                    graphs.len() == 1,
                    "expected one actual SdkFull symbol graph"
                );
                let path = root.join(&graphs[0]);
                let graph: Value = serde_json::from_slice(&fs::read(&path)?)?;
                ensure!(
                    graph["symbols"]
                        .as_array()
                        .is_some_and(|symbols| !symbols.is_empty()),
                    "empty Swift public symbol graph"
                );
                run.replacements.insert(
                    format!("{{swift-graph-{tier}}}"),
                    path.parent()
                        .context("symbol graph directory")?
                        .display()
                        .to_string(),
                );
                Ok(
                    json!({"path":path,"sha256":hash(&path)?,"symbols":graph["symbols"].as_array().unwrap().len()}),
                )
            })();
            verification(
                run,
                &format!("install-swift-{tier}-symbolgraph-identity"),
                result,
            )?;
        }
    }
    Ok(())
}

fn nonempty(path: &Path) -> Result<Value> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("required output {}", path.display()))?;
    ensure!(
        metadata.is_file() && metadata.len() > 0,
        "required output is empty or not a regular file: {}",
        path.display()
    );
    let bytes = fs::read(path)?;
    match path.extension().and_then(|s| s.to_str()) {
        Some("jar" | "whl" | "zip" | "nupkg" | "vsix") => ensure!(
            bytes.len() > 22
                && bytes.starts_with(b"PK")
                && bytes[bytes.len().saturating_sub(65557)..]
                    .windows(4)
                    .any(|bytes| bytes == b"PK\x05\x06"),
            "required package is not a complete ZIP archive: {}",
            path.display()
        ),
        Some("tgz" | "gz" | "crate") => ensure!(
            bytes.starts_with(&[0x1f, 0x8b]),
            "required package is not a gzip archive: {}",
            path.display()
        ),
        Some("json") => {
            let _: Value = serde_json::from_slice(&bytes)
                .with_context(|| format!("required JSON output {}", path.display()))?;
        }
        _ => {}
    }
    Ok(json!({"path":path,"sha256":sha(&bytes),"bytes":bytes.len()}))
}

fn copy_evidence_tree(from: &Path, to: &Path) -> Result<Value> {
    let files = inventory(from, tree_names(from)?.into_iter())?;
    ensure!(
        !files.is_empty(),
        "required installed/docs tree is empty: {}",
        from.display()
    );
    copy_inventory(from, to, &files)?;
    Ok(json!({"source":from,"snapshot":to,"files":files}))
}

fn copy_evidence_file(from: &Path, to: &Path) -> Result<Value> {
    let evidence = nonempty(from)?;
    create(to)?.write_all(&fs::read(from)?)?;
    ensure!(
        hash(to)? == evidence["sha256"],
        "native output changed while archiving"
    );
    Ok(json!({"source":evidence,"snapshot":to}))
}

fn source_bindings(language: &str) -> &'static str {
    match language {
        "typescript" => "docs-manifest.json",
        "rust" => "http-manifest.json",
        "python" | "go" => "docs/source-bindings.json",
        "swift" | "dart" => "sdk-manifest.json",
        "java" => "doc-coverage.json",
        "csharp" => "docs/reference.json",
        "kotlin" => "docs/symbols.json",
        "ruby" => "source-map.json",
        "php" | "cpp" => "docs/coverage.json",
        _ => unreachable!(),
    }
}

fn source_address_count(value: &Value) -> usize {
    match value {
        Value::Object(fields) => {
            usize::from(
                fields.get("pointer").is_some_and(Value::is_string)
                    && (fields.contains_key("document") || fields.contains_key("uri")),
            ) + fields.values().map(source_address_count).sum::<usize>()
        }
        Value::Array(values) => values.iter().map(source_address_count).sum(),
        Value::String(text) => {
            usize::from(text.contains("#/paths/") || text.contains("#/components/"))
        }
        _ => 0,
    }
}

fn verify_installed_sources(run: &Run, target: &Target, installed: &Path) -> Result<Value> {
    let generated = run.out.join("generated").join(target.language);
    let mut compared = BTreeMap::new();
    for name in tree_names(&generated)? {
        let relative = if target.language == "python" {
            let Some(name) = name.strip_prefix("src/sdk_full/") else {
                continue;
            };
            name.to_owned()
        } else {
            name.clone()
        };
        let actual = installed.join(&relative);
        if actual.is_file() {
            let expected = hash(&generated.join(&name))?;
            ensure!(
                hash(&actual)? == expected,
                "installed source differs from actual CLI artifact: {}/{name}",
                target.language
            );
            compared.insert(name, expected);
        } else if ["php", "dart", "go", "swift", "python"].contains(&target.language)
            || (target.language == "rust" && relative.starts_with("src/"))
            || (target.language == "ruby" && relative.starts_with("lib/"))
        {
            anyhow::bail!(
                "installed package is missing an emitted source: {}/{relative}",
                target.language
            );
        }
    }
    ensure!(
        compared.len() >= 3,
        "installed package has no substantial linkage to actual generated sources"
    );
    Ok(json!({"installed":installed,"comparedSources":compared}))
}

fn package_evidence(run: &mut Run, target: &Target, tier: &str) -> Result<Value> {
    let language = target.language;
    let prefix = format!("install-{language}-{tier}-");
    let required = package_stages()
        .into_iter()
        .filter(|stage| stage.id.starts_with(&prefix))
        .map(|stage| stage.id)
        .collect::<Vec<_>>();
    ensure!(
        !required.is_empty() && required.iter().all(|id| met(run, id)),
        "native install/build/docs commands incomplete: {language}/{tier}"
    );
    let root = PathBuf::from(run.expand(&native_root(language, tier)));
    let evidence = run.out.join("native-artifacts").join(language).join(tier);
    let bindings = run
        .out
        .join("generated")
        .join(language)
        .join(source_bindings(language));
    let value: Value = serde_json::from_slice(
        &fs::read(&bindings)
            .with_context(|| format!("actual native source bindings: {}", bindings.display()))?,
    )?;
    ensure!(
        source_address_count(&value) > 0,
        "native documentation has no source bindings: {language}"
    );
    let mut retained = Vec::new();
    let mut files = Vec::new();
    let mut dirs: Vec<(&str, PathBuf)> = Vec::new();
    let mut installed = None;
    let mut linkage = Value::Null;
    match language {
        "java" | "kotlin" => {
            ensure!(
                met(run, &format!("install-{language}-{tier}-snapshot")),
                "Maven installed/build byte linkage is missing"
            );
            files.push(root.join("installed/sdk-full-0.0.0.jar"));
            dirs.push(("installed", root.join("installed")));
            dirs.push((
                "docs",
                root.join(if language == "java" {
                    "sdk/target/reports/apidocs"
                } else {
                    "sdk/target/dokka"
                }),
            ));
            linkage = json!({"proof":format!("checks/install-{language}-{tier}-snapshot.json"),"consumer":"separately installed Maven module"});
        }
        "csharp" => {
            let package = root.join("installed/suspect.sdkfull/0.0.0");
            files.extend([
                root.join("feed/Suspect.SdkFull.0.0.0.nupkg"),
                package.join("lib/net8.0/Suspect.SdkFull.dll"),
                package.join("lib/net8.0/Suspect.SdkFull.xml"),
            ]);
            let assets: Value = serde_json::from_slice(&fs::read(
                root.join("sdk/examples/obj/project.assets.json"),
            )?)?;
            ensure!(
                assets["libraries"]["Suspect.SdkFull/0.0.0"]["type"] == "package",
                ".NET examples did not install the NuGet package"
            );
            let xml = fs::read_to_string(package.join("lib/net8.0/Suspect.SdkFull.xml"))?;
            let symbols = value["symbols"].as_array().context("C# doc symbols")?;
            ensure!(!symbols.is_empty(), "empty C# native docs");
            for symbol in symbols {
                let id = symbol["xmlId"]
                    .as_str()
                    .context("compiler XML member identity")?
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                ensure!(
                    xml.contains(&format!("name=\"{id}\"")),
                    "installed XML documentation lacks {id}"
                );
            }
            linkage = json!({"packageReference":assets["libraries"]["Suspect.SdkFull/0.0.0"],"resolvedXmlSymbols":symbols.len()});
            dirs.push(("installed", package));
        }
        "ruby" => {
            files.push(root.join("sdk/sdk-full-0.0.0.gem"));
            installed = Some(root.join("installed/gems/sdk-full-0.0.0"));
            files.push(installed.as_ref().unwrap().join("doc/SdkFull/Client.html"));
        }
        "php" => {
            files.push(root.join("sdk/build/package.zip"));
            installed = Some(root.join("consumer/vendor/example/sdk-full"));
            files.push(root.join("consumer/vendor/autoload.php"));
        }
        "dart" => {
            files.extend([
                root.join("sdk.tar.gz"),
                root.join("consumer-bin"),
                root.join("consumer/.dart_tool/package_config.json"),
            ]);
            installed = Some(root.join("installed"));
            dirs.push(("docs", root.join("docs")));
        }
        "typescript" => {
            files.push(
                run.work
                    .join("packages/typescript/suspect-fixtures-sdk-full-0.0.0.tgz"),
            );
            installed = Some(root.join("consumer/node_modules/@suspect-fixtures/sdk-full"));
            files.push(installed.as_ref().unwrap().join("dist/source/index.js"));
            dirs.push(("docs", run.work.join("native/typescript/docs")));
        }
        "rust" => {
            files.push(
                run.work
                    .join(format!("package-rust-{tier}/package/sdk-full-0.0.0.crate")),
            );
            installed = Some(root.join("installed/sdk-full-0.0.0"));
            dirs.push((
                "docs",
                run.work.join(format!("package-rust-{tier}/doc/sdk_full")),
            ));
            files.push(root.join("consumer-target/debug/sdk-full-consumer"));
        }
        "go" | "swift" => {
            files.push(root.join("sdk.tar.gz"));
            installed = Some(root.join("installed"));
            dirs.push((
                "docs",
                root.join(if language == "go" {
                    "docs"
                } else {
                    "SdkFull.doccarchive"
                }),
            ));
        }
        "python" => {
            files.push(run.work.join("wheels/sdk_full-0.0.0-py3-none-any.whl"));
            let site = run
                .replacements
                .get(&format!("{{python-site-{tier}}}"))
                .context("verified installed Python site")?;
            installed = Some(PathBuf::from(site).join("sdk_full"));
            dirs.push(("docs", root.join("docs")));
        }
        "cpp" => {
            files.push(root.join("installed/include/sdk_full/sdk.hpp"));
            files.push(root.join("consumer-build/consumer"));
            dirs.push(("installed", root.join("installed")));
            dirs.push(("docs", root.join("build/docs")));
            let text = fs::read_to_string(
                run.out
                    .join("logs/install-cpp-declared-consumer-run.stdout.log"),
            )?;
            ensure!(
                text.contains("202002") && text.contains("curl=8.7.1"),
                "actual linked C++20/libcurl profile is missing or changed"
            );
            linkage = json!({"cmakeConsumer":"find_package(sdk_full 0.0.0 EXACT CONFIG REQUIRED)","nativeLinkedProfile":text.trim()});
        }
        _ => unreachable!(),
    }
    if let Some(installed) = installed {
        linkage = verify_installed_sources(run, target, &installed)?;
        dirs.push(("installed", installed));
    }
    for path in &files {
        nonempty(path)?;
    }
    for (label, directory) in &dirs {
        if *label == "docs" {
            ensure!(
                tree_names(directory)?
                    .iter()
                    .any(|name| name.ends_with("index.html")),
                "native reference was not rendered: {language}/{tier}"
            );
            if ["go", "python"].contains(&language) {
                let coverage: Value =
                    serde_json::from_slice(&fs::read(directory.join("coverage.json"))?)?;
                ensure!(
                    coverage["plannedSymbols"].as_u64().is_some_and(|n| n > 0),
                    "empty native symbol/link coverage"
                );
                linkage["nativeDocCoverage"] = coverage;
            }
        }
        retained.push(copy_evidence_tree(directory, &evidence.join(label))?);
    }
    for (index, path) in files.iter().enumerate() {
        retained.push(copy_evidence_file(
            path,
            &evidence.join("outputs").join(format!(
                "{index}-{}",
                path.file_name()
                    .context("native output filename")?
                    .to_string_lossy()
            )),
        )?);
    }
    let result = json!({"language":language,"tier":tier,"actualPackageRoot":format!("generated/{language}"),"bindingManifest":bindings,"sourceBindings":source_address_count(&value),"commands":required,"linkage":linkage,"retained":retained});
    write_json(
        &run.out
            .join(format!("native-artifacts/{language}/{tier}/inventory.json")),
        &result,
    )?;
    Ok(result)
}

fn profile_evidence(run: &Run) -> Result<Value> {
    ensure!(
        met(run, "cli-twelve-profiles") && met(run, "cli-twelve-compatibility"),
        "profile discovery/compatibility command failed"
    );
    let profiles: Value = serde_json::from_slice(&fs::read(
        run.out.join("logs/cli-twelve-profiles.stdout.log"),
    )?)?;
    let comparison: Value = serde_json::from_slice(&fs::read(
        run.out.join("logs/cli-twelve-compatibility.stdout.log"),
    )?)?;
    let entries = profiles["profiles"]
        .as_array()
        .context("profile inventory")?;
    let native = comparison["native"]
        .as_array()
        .context("native compatibility reports")?;
    ensure!(
        entries.len() == TARGETS.len() && native.len() == TARGETS.len(),
        "CLI/compatibility must include exactly twelve SDK backends"
    );
    for target in TARGETS {
        ensure!(
            entries
                .iter()
                .filter(|entry| entry["profile"] == target.backend
                    && entry["directory"] == target.language)
                .count()
                == 1,
            "missing/duplicate {} profile",
            target.backend
        );
        let rows = native
            .iter()
            .filter(|entry| entry["backend"] == target.backend)
            .collect::<Vec<_>>();
        ensure!(
            rows.len() == 1 && rows[0]["before"].is_object() && rows[0]["after"].is_object(),
            "{} lacks retained native compatibility descriptors",
            target.backend
        );
    }
    ensure!(
        comparison["before"]["selected_operations"] == json!(OPERATIONS) || {
            let actual = comparison["before"]["selected_operations"]
                .as_array()
                .context("selected compatibility operations")?
                .iter()
                .filter_map(Value::as_str)
                .collect::<BTreeSet<_>>();
            actual == OPERATIONS.iter().copied().collect()
        },
        "comparison did not cover the exact five actual operations"
    );
    Ok(
        json!({"profiles":profiles,"compatibility":comparison,"terraform":"separate Go-SDK-based stretch artifact, outside this SDK inventory"}),
    )
}

fn pinned_stages() -> Vec<Stage> {
    let mut acquire = command(
        "twelve-pinned-acquire",
        "{out}/bin/suspect",
        &[
            "acquire",
            "{out}/configs/pins.json",
            "--cache-dir",
            "{work}/pinned/cache",
            "--format",
            "json",
        ],
        "{workspace}",
    );
    acquire.criterion = Criterion::Json {
        report: None,
        assertions: BTreeMap::from([
            ("/format".into(), json!("suspect.acquire.v1")),
            ("/status".into(), json!("acquired")),
            ("/networkRequests".into(), json!(0)),
            ("/diagnostics".into(), json!([])),
        ]),
    };
    let watch = command(
        "twelve-pinned-session-watch",
        "{out}/bin/suspect",
        &[
            "codegen-session",
            "--config",
            "{out}/configs/pinned-session.json",
            "--out",
            "{work}/pinned/packages",
            "--watch",
            "--max-iterations",
            "2",
            "--interval-ms",
            "25",
            "--format",
            "json",
        ],
        "{workspace}",
    );
    let mut compare = command(
        "twelve-pinned-compatibility",
        "{out}/bin/suspect",
        &[
            "codegen-compare",
            "--before",
            "{out}/configs/pinned-session.json",
            "--after",
            "{out}/configs/pinned-session.json",
            "--format",
            "json",
        ],
        "{workspace}",
    );
    compare.criterion = Criterion::Json {
        report: None,
        assertions: BTreeMap::from([
            ("/format".into(), json!("suspect-sdk-compatibility-v1")),
            ("/summary/unknowns".into(), json!(0)),
            ("/summary/breaking_changes".into(), json!(0)),
        ]),
    };
    vec![acquire, watch, compare]
}

fn prepare_configs(run: &mut Run) -> Result<Value> {
    write_json(
        &run.out.join("configs/five-operations.json"),
        &json!({"spec":run.out.join("inputs").join(INPUTS[0].1),"targets":target_config(),"operation_ids":OPERATIONS,"owner":"sdk-full-twelve-five-operations"}),
    )?;
    write_json(&run.out.join("configs/targets.json"), &TARGETS)?;
    let entry = run.work.join("pinned/entry.yaml");
    let bytes = fs::read(run.out.join("inputs").join(INPUTS[0].1))?;
    create(&entry)?.write_all(&bytes)?;
    let uri = suspect_source::Uri::from_path(&entry)?.to_string();
    write_json(
        &run.out.join("configs/pins.json"),
        &json!({"manifest_version":1,"entry":uri,"resources":[{"requested_uri":uri,"effective_uri":uri,"digest":format!("sha256-{}", sha(&bytes)),"media_type":"application/yaml","via":"local","redirects":[],"retrieved_at":"2026-09-10T00:00:00Z","attempts":0}]}),
    )?;
    write_json(
        &run.out.join("configs/pinned-session.json"),
        &json!({"pins":{"manifest":run.out.join("configs/pins.json"),"cache_dir":run.work.join("pinned/cache")},"targets":target_config(),"operation_ids":OPERATIONS,"owner":"sdk-full-twelve-pinned"}),
    )?;
    create(&run.work.join("editor/test/sdk-full.cjs"))?.write_all(EDITOR_FULL_TEST.as_bytes())?;
    create(&run.out.join("configs/editor-full.cjs"))?.write_all(EDITOR_FULL_TEST.as_bytes())?;
    for path in [
        "configs/five-operations.json",
        "configs/targets.json",
        "configs/pins.json",
        "configs/pinned-session.json",
        "configs/editor-full.cjs",
    ] {
        run.remember(&run.out.join(path))?;
    }
    Ok(
        json!({"targets":TARGETS,"operations":OPERATIONS,"pinnedInputIdentity":uri,"pinRetrieval":"local copy of the byte-pinned public input; remote/offline/TLS flows are separately executed in the required core suites"}),
    )
}

const EDITOR_FULL_TEST: &str = r#"const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const {test} = require('node:test');
const {availableSdkProfiles,generationArgs,runGeneration,readSdkSessionIdentity,startSdkSession} = require(process.env.SUSPECT_SDK_FULL_EDITOR);
const binary=process.env.SUSPECT_TEST_BINARY;
const targets=require(process.env.SUSPECT_SDK_FULL_TARGETS);
const config=require(process.env.SUSPECT_SDK_FULL_CONFIG);
const root=process.env.SUSPECT_SDK_FULL_EDITOR_OUT;
assert.equal(targets.length,12);
for(const target of targets) test(`real editor generation/check: ${target.backend}`, async()=>{
  const profiles=await availableSdkProfiles(binary);
  assert.equal(profiles.filter(p=>p.profile===target.backend&&p.directory===target.language).length,1);
  const out=path.join(root,target.language);
  const options={kind:target.backend,packageName:target.package_name,packageVersion:'0.0.0',operationIds:config.operation_ids};
  if(target.import_name!==null)options.importName=target.import_name;
  await runGeneration(binary,generationArgs(config.spec,out,options));
  const manifest=path.join(out,target.language,target.manifest);
  const before=await fs.stat(manifest); const bytes=await fs.readFile(manifest);
  assert.ok(before.size>0);
  await runGeneration(binary,generationArgs(config.spec,out,{...options,check:true}));
  const after=await fs.stat(manifest);
  assert.equal(after.ino,before.ino); assert.equal(after.mtimeMs,before.mtimeMs);
  assert.deepEqual(await fs.readFile(manifest),bytes);
});
test('real editor twelve-target pinned preview is source-identified and read-only',async()=>{
  const out=path.join(root,'pinned-preview');
  const identity=await readSdkSessionIdentity(process.env.SUSPECT_SDK_FULL_PINNED_CONFIG,out);
  assert.equal(identity.sourcePath,path.join(path.dirname(process.env.SUSPECT_SDK_FULL_PINNED_CONFIG),'pins.json'));
  const record=await startSdkSession(binary,identity,{preview:true},()=>{}).done;
  assert.equal(record.status,'drift'); assert.equal(record.delta.compiles,1); assert.equal(record.delta.renders,12);
  for(const target of targets)assert.ok(record.artifacts.some(file=>file.path===`${target.language}/${target.manifest}`&&file.content.length>0));
  await assert.rejects(fs.stat(out),{code:'ENOENT'});
});
"#;

fn pinned_evidence(run: &Run) -> Result<Value> {
    for item in pinned_stages() {
        ensure!(met(run, &item.id), "{} failed", item.id);
    }
    ensure!(
        !run.work.join("pinned/entry.yaml").exists(),
        "cache-only acceptance still has its original entry"
    );
    let text = fs::read_to_string(run.out.join("logs/twelve-pinned-session-watch.stdout.log"))?;
    let records = text
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    ensure!(
        records.len() == 2,
        "pinned watch must execute cold and unchanged observations"
    );
    for record in &records {
        ensure!(
            record["format"] == "suspect.sdk.session.v1"
                && record["success"] == true
                && record["input"]["kind"] == "pinned",
            "pinned watch did not use cache-only input"
        );
    }
    ensure!(
        records[0]["delta"]["compiles"] == 1
            && records[0]["delta"]["renders"] == 12
            && records[1]["delta"]["compiles"] == 0
            && records[1]["delta"]["renders"] == 0
            && records[1]["changedArtifacts"] == json!([])
            && records[0]["revision"] == records[1]["revision"],
        "pinned twelve-target warm path did redundant work or changed identity"
    );
    for target in TARGETS {
        verify_package_identity(
            target,
            &run.work.join("pinned/packages").join(target.language),
        )?;
    }
    let comparison: Value = serde_json::from_slice(&fs::read(
        run.out.join("logs/twelve-pinned-compatibility.stdout.log"),
    )?)?;
    ensure!(
        comparison["native"]
            .as_array()
            .is_some_and(|items| items.len() == 12),
        "pinned compatibility omitted native profiles"
    );
    Ok(
        json!({"records":records,"nativeProfiles":12,"originalPrivateEntryRemoved":true,"originalInputUnchanged":true}),
    )
}

fn maintained_auxiliary_stages() -> Vec<Stage> {
    let mut editor = command(
        "editor-twelve-native-and-pinned",
        "{node22}",
        &["--test", "--test-reporter=tap", "test/sdk-full.cjs"],
        "{work}/editor",
    );
    editor.criterion = Criterion::NodeTests;
    vec![
        editor,
        command(
            "performance-harness-tests",
            "{python-current}",
            &[
                "-m",
                "unittest",
                "discover",
                "-s",
                "tools/sdk-session-perf",
                "-p",
                "test_*.py",
                "-v",
            ],
            "{workspace}",
        ),
    ]
}

fn pin_bytes(pin: &Pin) -> Result<Vec<u8>> {
    ensure!(
        pin.path.is_absolute() && sealed::digest_text(&pin.sha256),
        "performance pin must have an absolute path and lowercase SHA-256"
    );
    let bytes = fs::read(&pin.path).with_context(|| format!("read pin {}", pin.path.display()))?;
    ensure!(
        sha(&bytes) == pin.sha256,
        "performance pin changed: {}",
        pin.path.display()
    );
    Ok(bytes)
}

fn performance_stages(run: &mut Run, path: &Path) -> Result<Vec<Stage>> {
    let bytes = fs::read(path)?;
    // Retain even a malformed/unusable submitted plan in this immutable attempt.
    create(&run.out.join("performance-plan.json"))?.write_all(&bytes)?;
    let plan: PerformancePlan = serde_json::from_slice(&bytes)?;
    ensure!(
        plan.format == "suspect.sdk.full.performance-plan.v1",
        "unsupported full-plan performance input format"
    );
    let evidence_bytes = pin_bytes(&plan.evidence)?;
    let pins: Value = serde_json::from_slice(&evidence_bytes)?;
    ensure!(
        pins["format"] == "suspect-sdk-session-acceptance-inputs-v1",
        "expected actual accept.py pin manifest"
    );
    let kinds = ["baseline", "candidate", "comparison", "policy", "runner"];
    ensure!(
        pins.as_object().context("pin manifest object")?.len() == kinds.len() + 1,
        "unexpected performance input fields"
    );
    let mut inputs = Vec::new();
    for kind in kinds {
        let pin: Pin = serde_json::from_value(pins[kind].clone())?;
        let contents = pin_bytes(&pin)?;
        inputs.push((kind, pin, contents));
    }
    create(&run.out.join("performance/import-inputs/pins.json"))?.write_all(&evidence_bytes)?;
    for (kind, pin, bytes) in &inputs {
        create(
            &run.out
                .join(format!("performance/import-inputs/{kind}.json")),
        )?
        .write_all(bytes)?;
        run.remember(&pin.path)?;
    }
    run.remember(path)?;
    run.remember(&plan.evidence.path)?;
    let mut stages = Vec::new();
    for name in ["small", "split-recursive", "openrouter", "compare"] {
        let report = format!("{{out}}/performance/{name}.json");
        let mut item = command(
            &format!("performance-{name}"),
            "{python-current}",
            &[
                "{workspace}/tools/sdk-session-perf/accept.py",
                "import",
                "--stage",
                name,
                "--evidence",
                plan.evidence
                    .path
                    .to_str()
                    .context("performance path UTF-8")?,
                "--evidence-sha256",
                &plan.evidence.sha256,
                "--original",
                "{original}",
                "--snapshot",
                "{workspace}",
                "--corpus",
                run.source.to_str().context("corpus UTF-8")?,
                "--acceptance-root",
                "{out}",
                "--source-sha256",
                "{source-sha256}",
                "--cli-sha256",
                "{cli-sha256}",
                "--out",
                &report,
            ],
            "{workspace}",
        );
        let mut assertions = BTreeMap::from([
            ("/format".into(), json!("suspect-sdk-session-acceptance-v1")),
            ("/complete".into(), json!(true)),
            ("/qualification".into(), json!("qualified")),
        ]);
        for key in [
            "source_and_snapshot",
            "actual_tools",
            "actual_inputs",
            "prepared_inputs",
            "build_configuration",
            "measured_binaries",
            "runner_identity",
            "v2_qualification_recomputed",
            "comparison_recomputed",
        ] {
            assertions.insert(format!("/verification/{key}"), json!(true));
        }
        let mut claims = BTreeMap::from([("complete".into(), "/complete".into())]);
        if name == "compare" {
            for key in ["gated", "verdict", "regressions", "comparedCases"] {
                claims.insert(key.into(), format!("/{key}"));
            }
            assertions.insert("/comparedFixtureClasses".into(), json!(3));
        } else {
            for (claim, pointer) in [
                ("measurementStatus", "/performance_status"),
                ("warmCompiles", "/summary/warm/compiles"),
                ("warmRenders", "/summary/warm/renders"),
                ("warmWrites", "/summary/warm/writes"),
                ("coldSamples", "/summary/cold/samples"),
                ("warmSamples", "/summary/warm/samples"),
                ("sourceChangeSamples", "/summary/sourceChange/samples"),
                ("configChangeSamples", "/summary/configChange/samples"),
            ] {
                claims.insert(claim.into(), pointer.into());
            }
        }
        item.criterion = Criterion::Performance {
            report,
            assertions,
            claims,
            comparison: name == "compare",
        };
        stages.push(item);
    }
    Ok(stages)
}

fn load_performance(run: &mut Run, args: &Args) -> Result<Vec<Stage>> {
    let before = run.files.clone();
    let result = if args.functional_only {
        Err(anyhow::anyhow!(
            "explicit functional-only profile: original numerical M6 remains pending"
        ))
    } else {
        args.performance_plan
            .as_deref()
            .context("no qualified --performance-plan supplied")
            .and_then(|path| performance_stages(run, path))
    };
    match result {
        Ok(stages) => Ok(stages),
        Err(error) => {
            run.files = before;
            for id in PERFORMANCE {
                verification(run, id, Err(anyhow::anyhow!("{error:#}")))?;
            }
            Ok(Vec::new())
        }
    }
}

fn native_cost_evidence(run: &Run) -> Result<Value> {
    let gate = "core-suspect-codegen-sdk_native_measurements-execution";
    ensure!(
        met(run, gate),
        "Main must supply the maintained sdk_native_measurements native suite; build/import/codec/request costs cannot be inferred from host test durations"
    );
    let root = run.work.join("native-costs");
    ensure!(
        root.canonicalize()?.starts_with(run.work.canonicalize()?),
        "native measurement root escaped private scratch"
    );
    let report: Value = serde_json::from_slice(&fs::read(root.join("report.json"))?)?;
    ensure!(
        report["format"] == "suspect.sdk.native-costs.v1"
            && report["complete"] == true
            && report["sourceFingerprint"] == run.provenance["sourceFingerprint"]
            && report["cliSha256"] == run.env["SUSPECT_M3_M6_BINARY_SHA256"],
        "native cost report is not bound to this source/CLI"
    );
    let rows = report["measurements"]
        .as_array()
        .context("native measurements")?;
    let mut identities = BTreeSet::new();
    for target in TARGETS {
        for tier in target.toolchain_tiers {
            for phase in ["build", "import", "codec", "request"] {
                let matches = rows
                    .iter()
                    .filter(|row| {
                        row["language"] == target.language
                            && row["tier"] == *tier
                            && row["phase"] == phase
                    })
                    .collect::<Vec<_>>();
                ensure!(
                    matches.len() == 1,
                    "missing/duplicate native cost dimension: {}/{tier}/{phase}",
                    target.language
                );
                let row = matches[0];
                ensure!(
                    identities.insert((target.language, *tier, phase)),
                    "duplicate cost dimension"
                );
                let manifest = run
                    .out
                    .join("generated")
                    .join(target.language)
                    .join(target.manifest);
                ensure!(
                    row["packageManifestSha256"] == hash(&manifest)?
                        && row["artifactBytes"].as_u64().is_some_and(|n| n > 0),
                    "native cost lacks actual package size/identity"
                );
                let samples = row["samples"]
                    .as_array()
                    .context("native cost raw samples")?;
                ensure!(!samples.is_empty(), "native cost has no measured samples");
                for sample in samples {
                    ensure!(
                        sample["nanoseconds"].as_u64().is_some_and(|n| n > 0)
                            && sample["iterations"].as_u64().is_some_and(|n| n > 0)
                            && sample["exitCode"] == 0,
                        "untimed/failed native cost sample"
                    );
                    let argv = sample["command"]
                        .as_array()
                        .context("native cost command")?;
                    let executable = PathBuf::from(
                        argv.first()
                            .and_then(Value::as_str)
                            .context("native cost executable")?,
                    );
                    ensure!(
                        executable.is_absolute() && sample["programSha256"] == hash(&executable)?,
                        "native measurement tool identity differs"
                    );
                    for stream in ["stdout", "stderr"] {
                        let name = sample[stream]
                            .as_str()
                            .context("raw native cost log path")?;
                        let path = root.join(relative(name)?).canonicalize()?;
                        ensure!(
                            path.starts_with(root.canonicalize()?)
                                && sample[format!("{stream}Sha256")] == hash(&path)?,
                            "native cost log missing, escaped or changed"
                        );
                    }
                }
            }
        }
    }
    ensure!(
        rows.len() == identities.len(),
        "unexpected native measurement dimensions"
    );
    Ok(report)
}

fn retain_native_outputs(run: &Run) -> Result<Value> {
    let target = run.work.join("source/target");
    let mut roots = vec![
        run.work.join("native"),
        run.work.join("swift"),
        run.work.join("tmp"),
        run.work.join("native-costs"),
    ];
    if target.is_dir() {
        for entry in fs::read_dir(&target)? {
            let path = entry?.path();
            let name = path
                .file_name()
                .context("native output name")?
                .to_string_lossy();
            if path.is_dir()
                && !fs::symlink_metadata(&path)?.file_type().is_symlink()
                && (name.starts_with("sdk-") || name.starts_with("native-"))
                && !["tools", "maven", "cargo"]
                    .iter()
                    .any(|word| name.contains(word))
            {
                roots.push(path);
            }
        }
    }
    let mut retained = BTreeMap::new();
    for root in roots.into_iter().filter(|root| root.is_dir()) {
        for name in tree_names(&root)? {
            let path = root.join(&name);
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.is_file() {
                continue;
            }
            if Path::new(&name).components().any(|part| {
                [
                    "incremental",
                    ".fingerprint",
                    "deps",
                    "module-cache",
                    "package-cache",
                    "node_modules",
                ]
                .contains(&part.as_os_str().to_string_lossy().as_ref())
            }) {
                continue;
            }
            let extension = path
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or_default();
            if ![
                "log",
                "json",
                "jsonl",
                "xml",
                "html",
                "md",
                "yaml",
                "yml",
                "toml",
                "lock",
                "txt",
                "gemspec",
                "rbs",
                "rs",
                "py",
                "go",
                "swift",
                "java",
                "class",
                "kt",
                "cs",
                "php",
                "dart",
                "cpp",
                "hpp",
                "h",
                "c",
                "ts",
                "js",
                "mjs",
                "cjs",
                "whl",
                "gem",
                "jar",
                "nupkg",
                "crate",
                "tgz",
                "gz",
                "zip",
                "dll",
                "a",
                "dylib",
                "swiftmodule",
            ]
            .contains(&extension)
            {
                continue;
            }
            let relative = path
                .strip_prefix(&run.work)?
                .to_str()
                .context("native evidence UTF-8")?;
            let to = run.out.join("native-outputs").join(relative);
            create(&to)?.write_all(&fs::read(&path)?)?;
            let digest = hash(&to)?;
            ensure!(
                hash(&path)? == digest,
                "retained native output changed: {}",
                path.display()
            );
            retained.insert(relative.to_owned(), json!({"snapshot":format!("native-outputs/{relative}"),"sha256":digest,"bytes":metadata.len()}));
        }
    }
    ensure!(
        !retained.is_empty(),
        "native tools produced no retained output"
    );
    write_json(&run.out.join("native-outputs.json"), &retained)?;
    Ok(
        json!({"manifest":"native-outputs.json","files":retained.len(),"sha256":hash(&run.out.join("native-outputs.json"))?,"policy":"package/source/consumer/docs/command evidence; compiler object caches excluded; no historical native reports imported"}),
    )
}

fn runner_source(files: &Inventory) -> Result<Value> {
    let expected = [
        (
            "xtask/src/sdk_full.rs",
            include_bytes!("sdk_full.rs").as_slice(),
        ),
        (
            "xtask/src/sdk_m3_m6.rs",
            include_bytes!("sdk_m3_m6.rs").as_slice(),
        ),
        ("xtask/src/main.rs", include_bytes!("main.rs").as_slice()),
    ];
    let mut witnesses = BTreeMap::new();
    for (name, bytes) in expected {
        ensure!(
            files
                .get(name)
                .is_some_and(|file| file.kind == "file" && file.sha256 == sha(bytes)),
            "running full verifier differs from {name}; rebuild xtask"
        );
        witnesses.insert(name, sha(bytes));
    }
    Ok(json!(witnesses))
}

fn v2_source_contract(run: &mut Run) -> Result<Value> {
    let name = "crates/suspect-schema/tests/fixtures/owned-applicators-v2.json";
    let path = run.work.join("source").join(name);
    let fixture: Value = serde_json::from_slice(&fs::read(&path)?)?;
    ensure!(
        fixture["format"] == "suspect.schema.applicators.v2.fixtures.1",
        "unsupported scoped native source fixture format"
    );
    let cases = fixture["cases"].as_array().context("scoped source cases")?;
    ensure!(
        cases.len() == 32,
        "maintain the exact independent 32-case v2 source contract"
    );
    let mut ids = BTreeSet::new();
    for case in cases {
        ensure!(
            ids.insert(case["id"].as_str().context("source case ID")?),
            "duplicate v2 source case"
        );
        let _: Value =
            serde_json::from_str(case["schemaJson"].as_str().context("source schema JSON")?)?;
        let _: Value = serde_json::from_str(
            case["instanceJson"]
                .as_str()
                .context("source instance JSON")?,
        )?;
        ensure!(
            ["Valid", "Invalid", "EvaluationFailure"]
                .contains(&case["expected"].as_str().context("source outcome")?),
            "unknown v2 source outcome"
        );
    }
    run.remember(&path)?;
    // The core v3 regression recompiles the maintained schemas at their original
    // logical URIs and compares the published v2 bytes. This separate, immutable
    // expected-value input never replaces source-driven native v2 compilation.
    let checkpoint = run
        .original
        .join("target/sdk-schema-applicators-executable-v2.json");
    let checkpoint_sha = "0dcae95d213030abd6fc14ebca8d3f4bf6ce9f5bd5382730bfa403f91be53056";
    ensure!(
        hash(&checkpoint)? == checkpoint_sha,
        "published v2 checkpoint changed; preserve its original manifest/identity"
    );
    let bytes = fs::read(&checkpoint)?;
    create(&run.out.join("inputs/core/published-applicators-v2.json"))?.write_all(&bytes)?;
    create(
        &run.work
            .join("source/target/sdk-schema-applicators-executable-v2.json"),
    )?
    .write_all(&bytes)?;
    run.remember(&checkpoint)?;
    run.remember(
        &run.work
            .join("source/target/sdk-schema-applicators-executable-v2.json"),
    )?;
    Ok(
        json!({"fixture":name,"sha256":hash(&path)?,"cases":ids,"compiler":"OwnedCompiler::compile_v2","sourceDrivenNativeGates":true,"coreV3BackwardCompatibilityCheckpoint":{"snapshot":"inputs/core/published-applicators-v2.json","sha256":checkpoint_sha},"resourceDynamicProfiles":"separate core gates; not silently included in native v2 claims"}),
    )
}

fn final_integrity(run: &Run, before: &Inventory, after: &Inventory) -> Result<Value> {
    sealed::require_external_root(&run.work)?;
    ensure!(
        before == after,
        "original source changed during full acceptance"
    );
    ensure!(
        inventory(&run.out.join("source"), before.keys().cloned())? == *before,
        "immutable source snapshot changed"
    );
    ensure!(
        inventory(&run.work.join("source"), before.keys().cloned())? == *before,
        "execution source changed"
    );
    for (path, digest) in &run.files {
        ensure!(
            hash(path)? == *digest,
            "pinned executable/tool/input changed: {}",
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
    let tools: Vec<Value> = serde_json::from_slice(&fs::read(run.out.join("tool-payloads.json"))?)?;
    for payload in tools {
        let root = Path::new(payload["root"].as_str().context("tool payload root")?);
        ensure!(
            external_tree(root)? == payload,
            "native compiler/runtime payload changed: {}",
            root.display()
        );
    }
    for target in TARGETS {
        let root = run.work.join("packages").join(target.language);
        let snapshot = run.out.join("generated").join(target.language);
        let names = tree_names(&snapshot)?;
        ensure!(
            inventory(&root, names.clone().into_iter())?
                == inventory(&snapshot, names.into_iter())?,
            "native package tools repaired emitted {} sources",
            target.language
        );
    }
    Ok(
        json!({"sourceUnchanged":true,"snapshotUnchanged":true,"executionSourceUnchanged":true,"generatedSourcesUnrepaired":true,"frozenBinariesInputsAndToolsUnchanged":true,"privateCargoHome":run.env["CARGO_HOME"],"sourceFingerprint":run.provenance["sourceFingerprint"]}),
    )
}

fn workflow(run: &mut Run, args: &Args, source: &mut Option<Inventory>) -> Result<()> {
    configure_environment(run)?;
    write_json(
        &run.out.join("environment.json"),
        &json!({"environment":run.env,"selectors":run.replacements,"environmentPolicy":"env_clear; explicit native selectors; private homes and copied offline caches; no caller stage configuration"}),
    )?;
    let files = sealed::prepare(run)?;
    *source = Some(files.clone());
    let runner = runner_source(&files);
    let current = runner.is_ok();
    verification(run, "full-runner-source", runner)?;
    ensure!(current, "full-runner source provenance failed");
    let v2_sources = v2_source_contract(run);
    verification(run, "schema-v2-source-contract", v2_sources)?;
    run.replacements.insert(
        "{source-sha256}".into(),
        run.env["SUSPECT_M3_M6_SOURCE_SHA256"].clone(),
    );
    run.env.insert(
        "SUSPECT_SDK_FULL_SOURCE_SHA256".into(),
        run.env["SUSPECT_M3_M6_SOURCE_SHA256"].clone(),
    );
    let editor_files = files
        .iter()
        .filter_map(|(name, file)| {
            name.strip_prefix("editors/vscode/")
                .map(|name| (name.to_owned(), file.clone()))
        })
        .collect();
    copy_inventory(
        &run.out.join("source/editors/vscode"),
        &run.work.join("editor"),
        &editor_files,
    )?;
    sealed::require_external_root(&run.work)?;
    verification(
        run,
        "scratch-isolation",
        Ok(
            json!({"root":run.work,"ancestorCargoManifests":[],"ancestorGitDirectories":[],"originalSourceWritableByCommands":false}),
        ),
    )?;
    let caches = seed_caches(run);
    let ready = caches.is_ok();
    verification(run, "private-native-caches", caches)?;
    ensure!(ready, "native cache preparation failed");
    for item in tool_stages() {
        run.command(&item)?;
    }
    let tools = full_tool_inputs(run);
    verification(run, "full-tool-inputs", tools)?;
    let editor_tools = load_editor_tools(run, args.editor_host_tools.as_deref());
    verification(run, "editor-host-tools", editor_tools)?;
    let configs = prepare_configs(run);
    let ready = configs.is_ok();
    verification(run, "full-native-configs", configs)?;
    ensure!(ready, "native configuration preparation failed");
    let default = default_cli_stages();
    run.command(&default[0])?;
    let default_binary = (|| -> Result<Value> {
        ensure!(
            met(run, "build-default-cli"),
            "default-feature CLI build failed"
        );
        freeze_file(
            run,
            &run.work.join("cargo/debug/suspect"),
            &run.out.join("bin/suspect-default"),
        )
    })();
    verification(run, "frozen-default-cli", default_binary)?;
    run.command(&default[1])?;
    let defaults = (|| -> Result<Value> {
        ensure!(
            met(run, "default-cli-twelve-profiles"),
            "default profile discovery failed"
        );
        let profiles: Value = serde_json::from_slice(&fs::read(
            run.out.join("logs/default-cli-twelve-profiles.stdout.log"),
        )?)?;
        verify_profile_inventory(&profiles)?;
        Ok(
            json!({"profiles":profiles,"explicitFeatures":[],"binary":"bin/suspect-default","sourceFingerprint":run.provenance["sourceFingerprint"]}),
        )
    })();
    verification(run, "default-profile-coverage", defaults)?;
    run.command(&build_cli())?;
    ensure!(met(run, "build-cli"), "full CLI build failed");
    let cli = run.out.join("bin/suspect");
    let frozen = freeze_file(run, &run.work.join("cargo/debug/suspect"), &cli)?;
    let digest = hash(&cli)?;
    run.env
        .insert("SUSPECT_M3_M6_BINARY_SHA256".into(), digest.clone());
    run.env
        .insert("SUSPECT_SDK_FULL_BINARY_SHA256".into(), digest.clone());
    run.replacements.insert("{cli-sha256}".into(), digest);
    verification(run, "frozen-cli", Ok(frozen))?;
    run.command(&command(
        "frozen-cli-version",
        "{out}/bin/suspect",
        &["--version"],
        "{workspace}",
    ))?;
    let performance = load_performance(run, args)?;
    let packages = package_stages();
    for item in &packages[..3] {
        run.command(item)?;
    }
    let generated = generated_packages(run);
    let ready = generated.is_ok();
    verification(run, "twelve-package-snapshot", generated)?;
    ensure!(ready, "actual twelve-package generation failed");
    for item in profile_stages() {
        run.command(&item)?;
    }
    let profiles = profile_evidence(run);
    verification(run, "twelve-profile-coverage", profiles)?;
    let consumers = prepare_consumers(run);
    let ready = consumers.is_ok();
    verification(run, "native-install-configs", consumers)?;
    ensure!(ready, "native install configuration failed");
    for item in &packages[3..] {
        run.command(item)?;
        package_hook(run, &item.id)?;
    }
    for target in TARGETS {
        let tiers = if target.language == "cpp" {
            &["declared"][..]
        } else {
            &["floor", "current"][..]
        };
        for tier in tiers {
            let evidence = package_evidence(run, target, tier);
            verification(
                run,
                &format!("installed-{}-{tier}-evidence", target.language),
                evidence,
            )?;
        }
    }
    let pinned = pinned_stages();
    run.command(&pinned[0])?;
    if met(run, "twelve-pinned-acquire") {
        fs::remove_file(run.work.join("pinned/entry.yaml"))?;
    }
    for item in &pinned[1..] {
        run.command(item)?;
    }
    let evidence = pinned_evidence(run);
    verification(run, "twelve-pinned-evidence", evidence)?;
    for (package, names) in suite_groups() {
        run.command(&build_tests(&package))?;
        let frozen = freeze_tests(run, &package, &names);
        verification(run, &format!("frozen-tests-{package}"), frozen)?;
    }
    // CARGO_BIN_EXE_suspect is baked into CLI process tests. Preserve that actual
    // private path as well as the environment-selected CLI/editor path.
    let process_cli = run.work.join("cargo/debug/suspect");
    fs::copy(&cli, &process_cli)?;
    for item in library_census_stages() {
        run.command(&item)?;
    }
    let census = library_census(run);
    verification(run, "library-codegen-census", census)?;
    for item in suites() {
        execute_suite(run, &item)?;
    }
    let coverage = library_coverage(run);
    verification(run, "library-codegen-coverage", coverage)?;
    let process = (|| -> Result<Value> {
        ensure!(
            hash(&process_cli)? == hash(&cli)?,
            "process suites changed the frozen CLI"
        );
        Ok(
            json!({"path":process_cli,"sha256":hash(&cli)?,"scope":"all separately frozen process/native suites, before broader Cargo quality checks"}),
        )
    })();
    verification(run, "frozen-process-cli", process)?;
    for item in editor_stages() {
        run.command(&item)?;
    }
    editor_native_workflow(run)?;
    for item in maintained_auxiliary_stages()
        .into_iter()
        .chain(quality_stages())
        .chain(performance)
    {
        run.command(&item)?;
    }
    let costs = native_cost_evidence(run);
    verification(run, "native-costs-evidence", costs)?;
    let harness = (|| -> Result<Value> {
        ensure!(
            met(run, "performance-harness-tests"),
            "measurement harness tests failed"
        );
        let text = fs::read_to_string(run.out.join("logs/performance-harness-tests.stderr.log"))?;
        let count = text
            .lines()
            .find_map(|line| {
                line.strip_prefix("Ran ")?
                    .split_whitespace()
                    .next()?
                    .parse::<usize>()
                    .ok()
            })
            .context("no Python unittest result")?;
        ensure!(
            count > 0 && text.trim_end().ends_with("OK"),
            "measurement harness tests were empty/skipped/incomplete"
        );
        Ok(json!({"tests":count,"syntheticHarnessTestsOnly":true,"numericalMeasurements":false}))
    })();
    verification(run, "performance-harness-execution", harness)?;
    Ok(())
}

fn required_stages() -> Vec<Stage> {
    let mut items = suites();
    items.extend(suites().iter().map(|suite| {
        command(
            &format!("{}-execution", suite.id),
            "[runner: named native/host results]",
            &[],
            "",
        )
    }));
    items.extend(tool_stages());
    items.extend(package_stages());
    items.extend(profile_stages());
    items.extend(pinned_stages());
    items.extend(editor_stages());
    items.extend(editor_native_stages());
    items.extend(maintained_auxiliary_stages());
    items.extend(quality_stages());
    items.extend(library_census_stages());
    items.extend(default_cli_stages());
    items.push(build_cli());
    items.push(command(
        "frozen-cli-version",
        "{out}/bin/suspect",
        &["--version"],
        "{workspace}",
    ));
    for package in suite_groups().keys() {
        items.push(build_tests(package));
        items.push(command(
            &format!("frozen-tests-{package}"),
            "[runner: freeze required binaries]",
            &[],
            "",
        ));
    }
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
        "full-runner-source",
        "schema-v2-source-contract",
        "scratch-isolation",
        "private-native-caches",
        "full-tool-inputs",
        "full-native-configs",
        "frozen-cli",
        "frozen-default-cli",
        "default-profile-coverage",
        "frozen-process-cli",
        "library-codegen-census",
        "library-codegen-coverage",
        "twelve-package-snapshot",
        "twelve-profile-coverage",
        "native-install-configs",
        "editor-host-tools",
        "editor-native-staging",
        "editor-native-pins",
        "editor-native-lifecycle-evidence",
        "editor-native-commands-evidence",
        "editor-native-integrity",
        "twelve-pinned-evidence",
        "native-costs-evidence",
        "performance-harness-execution",
        "retained-native-outputs",
        "final-integrity",
    ] {
        items.push(command(id, "[runner verification]", &[], ""));
    }
    for (id, _, _) in INPUTS {
        items.push(command(
            &format!("input-tracked-{id}"),
            "[runner: tracked input]",
            &[],
            "",
        ));
    }
    for tier in ["floor", "current"] {
        for language in ["java", "kotlin"] {
            items.push(command(
                &format!("install-{language}-{tier}-snapshot"),
                "[runner: installed jar linkage]",
                &[],
                "",
            ));
        }
        items.push(command(
            &format!("install-python-{tier}-site-identity"),
            "[runner: installed wheel identity]",
            &[],
            "",
        ));
        items.push(command(
            &format!("install-swift-{tier}-symbolgraph-identity"),
            "[runner: public symbol graph]",
            &[],
            "",
        ));
    }
    for target in TARGETS {
        for tier in if target.language == "cpp" {
            &["declared"][..]
        } else {
            &["floor", "current"][..]
        } {
            items.push(command(
                &format!("installed-{}-{tier}-evidence", target.language),
                "[runner: actual archives/install/docs/links]",
                &[],
                "",
            ));
        }
    }
    for id in PERFORMANCE {
        items.push(command(
            id,
            "[maintained accept.py import; pinned plan required]",
            &[],
            "",
        ));
    }
    for item in &mut items {
        item.milestones = strings(FULL);
    }
    items
}

fn stage_inventory(functional_only: bool) -> Result<Value> {
    let items = required_stages();
    let mut ids = BTreeSet::new();
    ensure!(
        items.iter().all(|item| ids.insert(item.id.clone())),
        "duplicate full-plan stage identity"
    );
    let required = items
        .iter()
        .filter(|item| !functional_only || !PERFORMANCE.contains(&item.id.as_str()))
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    Ok(
        json!({"format":"suspect.sdk.full.stages.v1","acceptanceProfile":if functional_only { "functional-only" } else { "strict" },"numericalPerformanceRequired":!functional_only,"stageCount":items.len(),"requiredStageCount":required.len(),"requiredStages":required,"numericalStages":PERFORMANCE,"targets":TARGETS,"operations":OPERATIONS,"features":FEATURES,"libraryNativeSelections":LIBRARY_NATIVE,"editorNativeContract":editor_contract_inventory(),"stages":items,"callerSelectableStages":false,"terraform":"Go-SDK-based stretch; not a thirteenth SDK or a core gate"}),
    )
}

fn syntax_check(original: &Path) -> Result<Value> {
    let inventory = stage_inventory(false)?;
    let runner_files = sealed::inventory(
        original,
        strings(&[
            "xtask/src/sdk_full.rs",
            "xtask/src/sdk_m3_m6.rs",
            "xtask/src/main.rs",
        ])
        .into_iter(),
    )?;
    let runner_error = runner_source(&runner_files)
        .err()
        .map(|error| format!("{error:#}"));
    let mut missing = Vec::new();
    let mut checked = Vec::new();
    let mut missing_witnesses = BTreeMap::new();
    for (package, names) in suite_groups() {
        for name in names {
            let path = if name == package.replace('-', "_") {
                format!("crates/{package}/src/lib.rs")
            } else {
                format!("crates/{package}/tests/{name}.rs")
            };
            if original.join(&path).is_file() {
                let text = fs::read_to_string(original.join(&path))?;
                for witness in expected_native_tests(&name) {
                    if !text.contains(&format!("fn {witness}(")) {
                        missing_witnesses.insert(
                            format!("{package}/{name}/{witness}"),
                            json!({"source":path,"name":witness}),
                        );
                    }
                }
                checked.push(path);
            } else {
                missing.push(path);
            }
        }
    }
    for case in LIBRARY_NATIVE {
        let path = original.join(case.source);
        let text = fs::read_to_string(&path).unwrap_or_default();
        let function = case
            .name
            .rsplit("::")
            .next()
            .context("native function name")?;
        if !path.is_file() || !text.contains(&format!("fn {function}(")) {
            missing_witnesses.insert(
                case.name.to_owned(),
                json!({"source":case.source,"name":case.name}),
            );
        }
    }
    let mut missing_support = Vec::new();
    for name in EDITOR_HARNESS_FILES
        .iter()
        .copied()
        .chain(["contract.test.cjs"])
    {
        let path = format!("editors/vscode/test/native-host/{name}");
        if !original.join(&path).is_file() {
            missing_support.push(path);
        }
    }
    for name in [
        "crates/suspect-schema/tests/fixtures/owned-applicators-v2.json",
        "crates/suspect-codegen/tests/fixtures/typescript-v2-browser.mjs",
        "crates/suspect-codegen/tests/fixtures/typescript-resources-browser.mjs",
        "crates/suspect-codegen/src/java_sdk/NativeAggregateExamples.java",
        "crates/suspect-codegen/tests/fixtures/http-protocol-v1.json",
        "crates/suspect-codegen/tools/typescript-floor/package.json",
        "crates/suspect-codegen/tools/typescript-floor/package-lock.json",
        "crates/suspect-codegen/src/csharp_sdk/testdata/PositionalConsumer.cs",
        "crates/suspect-codegen/src/rust_validation/runtime_v3.rs",
        "crates/suspect-codegen/src/rust_models/resources.rs",
        "crates/suspect-codegen/src/go_validation/resources.go",
        "crates/suspect-codegen/src/go_validation/scoped.go",
        "crates/suspect-codegen/src/go_validation/scoped_pattern.go",
        "crates/suspect-codegen/src/credential_env.rs",
        "crates/suspect-codegen/src/dart_sdk/environment.rs",
        "crates/suspect-codegen/src/dart_sdk/environment.dart",
        "crates/suspect-codegen/src/dart_sdk/environment_io.dart",
        "crates/suspect-codegen/src/dart_sdk/environment_stub.dart",
        "crates/suspect-codegen/src/dart_sdk/CREDENTIAL-ENV.md",
        "crates/suspect-codegen/src/dart_sdk/native_credential_env.dart",
        "crates/suspect-codegen/src/dart_sdk/native_credential_env_io.dart",
        "crates/suspect-codegen/src/dart_sdk/native_credential_env_openrouter.dart",
        "crates/suspect-codegen/tests/fixtures/dart-credential-env-no-policy.json",
        "crates/suspect-codegen/tests/dart_support/mod.rs",
        "crates/suspect-codegen/tests/dart_support/browser.mjs",
        "crates/suspect-codegen/src/csharp_sdk/credential_env.rs",
        "crates/suspect-codegen/src/csharp_sdk/CredentialEnvironment.cs",
        "crates/suspect-codegen/src/csharp_sdk/testdata/CredentialEnvironmentConsumer.cs",
        "crates/suspect-codegen/src/csharp_sdk/testdata/OpenRouterEnvironmentConsumer.cs",
        "crates/suspect-codegen/src/csharp_sdk/testdata/credential-env.openapi.json",
        "crates/suspect-codegen/src/ruby_sdk/credential_env.rb",
        "crates/suspect-codegen/src/ruby_sdk/tests/credential_env.rb",
        "crates/suspect-codegen/src/ruby_sdk/tests/credential_env_openrouter.rb",
        "crates/suspect-codegen/src/ruby_sdk/tests/credential_env.openapi.json",
        "crates/suspect-codegen/src/typescript/http/credential-env.ts",
        "crates/suspect-codegen/tests/fixtures/typescript-credential-env-no-policy-v1.json",
        "crates/suspect-codegen/tests/fixtures/typescript-credential-env-browser.mjs",
        "crates/suspect-codegen/src/python_http/credential_env.rs",
        "crates/suspect-codegen/src/python_http/credential_env.py",
        "crates/suspect-codegen/tests/python_credential_env/api.json",
        "crates/suspect-codegen/tests/python_credential_env/consumer.py",
        "crates/suspect-codegen/tests/python_credential_env/negative.py",
        "crates/suspect-codegen/tests/python_credential_env/openrouter.py",
        "crates/suspect-codegen/tests/python_credential_env/openrouter_negative.py",
        "crates/suspect-codegen/src/go_http/credential_env.rs",
        "crates/suspect-codegen/src/rust_http/emit/credential_env.rs",
        "crates/suspect-codegen/tests/fixtures/rust-credential-env-no-policy.json",
        "crates/suspect-codegen/tests/fixtures/rust-credential-env-consumer.rs",
        "crates/suspect-codegen/src/swift_sdk/credential_env_native.swift",
        "crates/suspect-codegen/src/swift_sdk/credential_env_openrouter.swift",
        "crates/suspect-codegen/tests/fixtures/swift-credential-env-no-policy.json",
        "crates/suspect-codegen/src/swift_sdk/validation_v2_support.rs",
        "crates/suspect-codegen/src/java_sdk/readme-credential-env.md",
        "crates/suspect-codegen/src/java_sdk/NativeSupport.java",
        "crates/suspect-codegen/src/java_sdk/NativeCredentialEnv.java",
        "crates/suspect-codegen/src/java_sdk/NativeCredentialEnvOpenRouter.java",
        "crates/suspect-codegen/src/java_sdk/NativeCredentialEnvExample.java",
        "crates/suspect-codegen/src/kotlin_sdk/environment.rs",
        "crates/suspect-codegen/src/kotlin_sdk/CredentialEnvironment.kt",
        "crates/suspect-codegen/src/kotlin_sdk/guide-env.md",
        "crates/suspect-codegen/src/kotlin_sdk/native_credential_env.kt",
        "crates/suspect-codegen/src/kotlin_sdk/native_openrouter_env.kt",
        "crates/suspect-codegen/tests/kotlin_support/mod.rs",
        "crates/suspect-codegen/tests/kotlin_support/credential-env.openapi.json",
        "crates/suspect-codegen/src/cpp_sdk/emit/credential_env.rs",
        "crates/suspect-codegen/src/cpp_sdk/tests/native_credential_env_openrouter.cpp",
        "crates/suspect-codegen/src/cpp_sdk/tests/native_credential_env_security.cpp",
        "crates/suspect-codegen/src/swift_sdk/validation_v3_support.rs",
        "crates/suspect-codegen/src/swift_sdk/validation_v3_native.swift",
        "crates/suspect-codegen/src/swift_sdk/protocol_resources_native.swift",
        "crates/suspect-codegen/src/swift_sdk/aggregate_examples_native.swift",
        "crates/suspect-schema/tests/fixtures/resource-conformance/dynamicRef.json",
        "crates/suspect-schema/tests/fixtures/resource-conformance/tree.json",
        "crates/suspect-schema/tests/fixtures/resource-conformance/extendible-dynamic-ref.json",
        "crates/suspect-schema/tests/fixtures/resource-conformance/detached-dynamicref.json",
        "crates/suspect-schema/tests/conformance/draft2020-12/unevaluatedProperties.json",
        "crates/suspect-schema/tests/conformance/draft2020-12/unevaluatedItems.json",
        "docs/SDK-RUST-RESOURCES.md",
        "docs/SDK-SWIFT-RESOURCES.md",
        "docs/SDK-GO-SCHEMA-V3.md",
        "docs/SDK-HTTP-PROTOCOL.md",
        "docs/SDK-TYPESCRIPT-PROTOCOL.md",
        "docs/SDK-CREDENTIAL-ENV.md",
        "docs/SDK-SCHEMA-NATIVE-ADOPTION.md",
    ] {
        if !original.join(name).is_file() {
            missing_support.push(name.to_owned());
        }
    }
    for target in TARGETS {
        ensure!(
            original.join(target.docs).is_file(),
            "missing native selector/package documentation: {}",
            target.docs
        );
    }
    Ok(
        json!({"format":"suspect.sdk.full.syntax.v1","inventoryValid":true,"compiledRunnerMatchesSource":runner_error.is_none(),"runnerError":runner_error,"readyToBuild":missing.is_empty() && missing_witnesses.is_empty() && missing_support.is_empty() && runner_error.is_none(),"stageCount":inventory["stageCount"],"checkedSuiteSources":checked,"missingSuiteSources":missing,"missingNamedWitnesses":missing_witnesses,"missingSupportSources":missing_support,"editorHostToolPinsRequired":true,"acceptanceExecuted":false}),
    )
}

fn completion(functional_only: bool, required: &[Stage], checks: &[Value], aborted: bool) -> Value {
    let ids = required
        .iter()
        .map(|stage| stage.id.as_str())
        .collect::<BTreeSet<_>>();
    let seen = checks
        .iter()
        .filter_map(|check| check["id"].as_str())
        .collect::<BTreeSet<_>>();
    let valid_inventory = !required.is_empty()
        && ids.len() == required.len()
        && seen.len() == checks.len()
        && seen.is_subset(&ids);
    let unmet = required
        .iter()
        .filter(|stage| {
            !checks
                .iter()
                .any(|check| check["id"] == stage.id && check["criterionMet"] == true)
        })
        .map(|stage| stage.id.clone())
        .collect::<Vec<_>>();
    let functional =
        valid_inventory && !aborted && unmet.iter().all(|id| PERFORMANCE.contains(&id.as_str()));
    let numerical = PERFORMANCE.iter().all(|id| {
        checks
            .iter()
            .any(|check| check["id"] == *id && check["criterionMet"] == true)
    });
    let complete = functional && numerical;
    json!({"complete":complete,"profileComplete":if functional_only {functional} else {complete},"functionalComplete":functional,"numericalPerformanceComplete":numerical,"numericalStatus":if numerical {"qualified-original-m6-policy"} else {"pending"},"status":if complete {"passed"} else if functional {"functional-passed-numerical-pending"} else {"incomplete"},"unmet":unmet})
}

fn verify_seal(root: &Path) -> Result<String> {
    let digest = hash(&root.join("seal.json"))?;
    ensure!(
        fs::read_to_string(root.join("seal.sha256"))? == format!("{digest}  seal.json\n"),
        "evidence seal anchor differs"
    );
    let seal: Value = serde_json::from_slice(&fs::read(root.join("seal.json"))?)?;
    ensure!(
        seal["format"] == "suspect.sdk.evidence-seal.v1",
        "unknown evidence seal"
    );
    let files = seal["files"].as_object().context("seal inventory")?;
    let actual = tree_names(root)?
        .into_iter()
        .filter(|name| !["seal.json", "seal.sha256"].contains(&name.as_str()))
        .collect::<BTreeSet<_>>();
    ensure!(
        actual == files.keys().cloned().collect(),
        "evidence seal census differs"
    );
    for (name, expected) in files {
        let path = root.join(relative(name)?);
        let metadata = fs::symlink_metadata(&path)?;
        if expected["kind"] == "symlink" {
            ensure!(
                metadata.file_type().is_symlink(),
                "sealed link replaced: {name}"
            );
            let link = fs::read_link(&path)?;
            ensure!(
                expected["target"] == link.to_str().context("seal link UTF-8")?
                    && expected["sha256"] == sha(link.as_os_str().as_encoded_bytes()),
                "sealed link differs: {name}"
            );
        } else {
            ensure!(
                expected["kind"] == "file"
                    && metadata.is_file()
                    && !metadata.file_type().is_symlink()
                    && expected["sha256"] == hash(&path)?,
                "sealed file differs: {name}"
            );
        }
    }
    Ok(digest)
}

pub(super) fn run(raw: &[String]) -> Result<()> {
    if raw == ["--help"] || raw == ["-h"] {
        print!("{USAGE}");
        return Ok(());
    }
    if raw.iter().any(|arg| arg == "--list-stages") {
        let functional = raw.iter().any(|arg| arg == "--functional-only");
        ensure!(
            raw.len() == if functional { 2 } else { 1 },
            "--list-stages accepts only --functional-only"
        );
        println!(
            "{}",
            serde_json::to_string_pretty(&stage_inventory(functional)?)?
        );
        return Ok(());
    }
    let original = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("workspace root")?
        .canonicalize()?;
    if raw == ["--check-syntax"] {
        let result = syntax_check(&original)?;
        println!("{}", serde_json::to_string_pretty(&result)?);
        ensure!(
            result["readyToBuild"] == true,
            "maintained full-runner inventory is valid; listed native suite sources remain required"
        );
        return Ok(());
    }
    let args = Args::parse(raw)?;
    let source = args.source.canonicalize()?;
    let requested = if args.out.is_absolute() {
        args.out.clone()
    } else {
        original.join(&args.out)
    };
    let parent = requested
        .parent()
        .context("report parent")?
        .canonicalize()?;
    ensure!(
        parent.starts_with(original.join("target").canonicalize()?) && !parent.starts_with(&source),
        "report must be a new directory under this workspace's target/"
    );
    let out = parent.join(requested.file_name().context("report name")?);
    let work = sealed::work_path(&out, &std::env::temp_dir().join("opencode"))?;
    ensure!(
        !out.exists()
            && !work.exists()
            && fs::symlink_metadata(&out).is_err()
            && fs::symlink_metadata(&work).is_err(),
        "preserve earlier attempts; select a new --out"
    );
    let required = required_stages();
    stage_inventory(args.functional_only)?;
    fs::create_dir(&out)?;
    fs::create_dir(&work)?;
    let mut run = Run {
        replacements: BTreeMap::from([
            ("{out}".into(), out.display().to_string()),
            ("{work}".into(), work.display().to_string()),
            (
                "{workspace}".into(),
                work.join("source").display().to_string(),
            ),
            ("{original}".into(), original.display().to_string()),
        ]),
        original,
        source,
        out,
        work,
        env: BTreeMap::new(),
        checks: Vec::new(),
        tests: BTreeMap::new(),
        files: BTreeMap::new(),
        absent_configs: BTreeSet::new(),
        provenance: Value::Null,
    };
    let executable = std::env::current_exe()?;
    write_json(
        &run.out.join("invocation.json"),
        &json!({"args":raw,"runner":executable,"runnerSha256":hash(&executable)?,"sourcePolicy":"actual dirty snapshot, never a HEAD checkout","workDirectory":run.work,"acceptanceProfile":if args.functional_only {"functional-only"} else {"strict"}}),
    )?;
    run.remember(&executable)?;
    write_json(
        &run.out.join("stages.json"),
        &stage_inventory(args.functional_only)?,
    )?;
    let mut before = None;
    let outcome = workflow(&mut run, &args, &mut before);
    let error = outcome.err().map(|error| format!("{error:#}"));
    let retained = retain_native_outputs(&run);
    verification(&mut run, "retained-native-outputs", retained)?;
    let integrity = (|| -> Result<Value> {
        let before = before
            .as_ref()
            .context("source preparation did not complete")?;
        let after = sealed::source_inventory(&mut run, "after")?;
        write_json(&run.out.join("source-manifest-after.json"), &after)?;
        final_integrity(&run, before, &after)
    })();
    verification(&mut run, "final-integrity", integrity)?;
    for item in &required {
        if !run.checks.iter().any(|check| check["id"] == item.id) {
            verification(
                &mut run,
                &item.id,
                Err(anyhow::anyhow!(
                    "required gate produced no evidence{}",
                    error
                        .as_deref()
                        .map_or(String::new(), |error| format!(": {error}"))
                )),
            )?;
        }
    }
    let result = completion(
        args.functional_only,
        &required,
        &run.checks,
        error.is_some(),
    );
    let profile_complete = result["profileComplete"] == true;
    write_json(
        &run.out.join("report.json"),
        &json!({
            "format":"suspect.sdk.full.v1","acceptanceProfile":if args.functional_only {"functional-only"} else {"strict"},
            "complete":result["complete"],"profileComplete":result["profileComplete"],"functionalComplete":result["functionalComplete"],
            "status":result["status"],"numericalStatus":result["numericalStatus"],"numericalPerformanceRequired":!args.functional_only,
            "numericalPerformanceComplete":result["numericalPerformanceComplete"],"numericalStages":PERFORMANCE,
            "deferredStages":if args.functional_only {PERFORMANCE} else {&[]},"releaseReady":false,"sdkReleaseReady":false,
            "targets":TARGETS,"originalM6NumericalScope":["typescript","rust","python","go","swift"],
            "scope":["twelve default native SDK profiles","source/contract/acquisition/dialects/protocol examples","source-driven native scoped validation v2","resource/dynamic core tests with separate native admission fences","native models/codecs/types/wire/resources/cancellation/docs/quickstarts","CLI/session/pinned input/editor/compatibility","real installed VSIX lifecycle and commands","native cost observations","quality/integrity","original strict numerical M6"],
            "libraryNativeSelections":LIBRARY_NATIVE,"editorNativeContract":editor_contract_inventory(),
            "terraform":"separate stretch built on the generated Go SDK; not a core SDK gate",
            "currentFullCorpusAcceptance":null,"source":run.provenance,"requiredStageCount":required.len(),"requiredStages":required,
            "checks":run.checks,"unmet":result["unmet"],"fileFingerprints":run.files,"error":error,
            "evidenceCompleteOnlyWithVerifiedSeal":true
        }),
    )?;
    sealed::seal(&run.out)?;
    let digest = verify_seal(&run.out)?;
    println!(
        "SDK full {}: {}\nseal SHA-256: {digest}",
        result["status"].as_str().unwrap_or("incomplete"),
        run.out.join("report.json").display()
    );
    ensure!(
        profile_complete,
        "required full-plan evidence is incomplete; inspect report.json unmet gates and logs"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_run(root: &Path) -> Run {
        Run {
            original: root.into(),
            source: root.into(),
            out: root.join("report"),
            work: root.join("work"),
            env: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
            replacements: BTreeMap::new(),
            checks: Vec::new(),
            tests: BTreeMap::new(),
            files: BTreeMap::new(),
            absent_configs: BTreeSet::new(),
            provenance: Value::Null,
        }
    }

    #[test]
    fn inventory_has_twelve_native_base_protocol_and_integration_profiles_without_stretch() {
        let value = stage_inventory(false).unwrap();
        let targets = value["targets"].as_array().unwrap();
        assert_eq!(targets.len(), 12);
        let items = required_stages();
        assert_eq!(items.len(), value["stageCount"].as_u64().unwrap() as usize);
        assert_eq!(value["stageCount"], 924);
        assert_eq!(suites().len(), 296);
        assert_eq!(
            items.len(),
            items
                .iter()
                .map(|item| &item.id)
                .collect::<BTreeSet<_>>()
                .len()
        );
        for target in TARGETS {
            let native = if target.language == "ruby" {
                "test:suspect-codegen/ruby_sdk".to_owned()
            } else {
                format!("test:suspect-codegen/{}_protocol", target.language)
            };
            assert!(items.iter().any(|item| item.program == native), "{native}");
            assert!(items.iter().any(|item| {
                item.id
                    .starts_with(&format!("installed-{}-", target.language))
            }));
        }
        for language in ["java", "csharp", "kotlin", "ruby", "php", "dart", "cpp"] {
            assert!(
                items
                    .iter()
                    .any(|item| item.program == format!("test:suspect-codegen/{language}_sdk"))
            );
            assert!(
                items
                    .iter()
                    .any(|item| item.id == format!("integration-{language}"))
            );
        }
        assert!(!items.iter().any(|item| item.id.contains("terraform")));
        for name in [
            "pinned_acquisition",
            "pinned_provider",
            "pinned_transport",
            "pinned_contract",
            "contract_oas32",
            "contract_dual_role_scope",
            "owned_dialects",
            "owned_normative",
            "owned_context_regressions",
            "protocol_examples",
            "pinned_generation",
            "sdk_dialect_models",
            "python_quickstart",
            "python_null_models",
            "sdk_native_measurements",
            "sdk_generation_options",
            "sdk_protocol_options",
            "typescript_protocol_compatibility",
            "sdk_compatibility_context",
        ] {
            assert!(
                items
                    .iter()
                    .any(|item| item.program.ends_with(&format!("/{name}"))),
                "{name}"
            );
        }
        let groups = suite_groups();
        assert!(groups["suspect-ir"].contains("pinned_contract"));
        assert!(groups["suspect-schema"].contains("owned_context_regressions"));
        for (package, name) in [
            ("suspect-ir", "contract_dual_role_scope"),
            ("suspect-codegen", "sdk_generation_options"),
            ("suspect-cli", "sdk_protocol_options"),
            ("suspect-codegen", "typescript_protocol_compatibility"),
            ("suspect-codegen", "sdk_compatibility_context"),
            ("suspect-codegen", "ruby_compatibility_credentials"),
            ("suspect-codegen", "credential_env"),
            ("suspect-codegen", "go_credential_env_canonical"),
            ("suspect-codegen", "go_credential_env_factory_capture"),
            ("suspect-codegen", "swift_credential_env_canonical"),
            ("suspect-cli", "credential_env_codegen"),
            ("suspect-codegen", "kotlin_validation_integration"),
            ("suspect-codegen", "kotlin_validation_resource_integration"),
        ] {
            assert!(groups[package].contains(name));
            let id = format!("core-{package}-{name}");
            let stage = items.iter().find(|item| item.id == id).unwrap();
            assert_eq!(stage.program, format!("test:{package}/{name}"));
            assert!(matches!(stage.criterion, Criterion::RustTests));
            assert!(
                items
                    .iter()
                    .any(|item| item.id == format!("{id}-execution"))
            );
        }
    }

    #[test]
    fn ruby_base_and_protocol_require_one_complete_combined_suite_per_tier() {
        let items = suites();
        let ruby = items
            .iter()
            .filter(|item| item.program == "test:suspect-codegen/ruby_sdk")
            .collect::<Vec<_>>();
        assert_eq!(ruby.len(), 2);
        for tier in ["floor", "current"] {
            let item = ruby
                .iter()
                .find(|item| item.id == format!("{tier}-ruby_sdk"))
                .unwrap();
            assert_eq!(
                item.environment["SUSPECT_RUBY_HOME"],
                format!("{{ruby-{tier}}}")
            );
            assert_eq!(
                item.args,
                strings(&["--include-ignored", "--show-output", "--test-threads=1"])
            );
        }
        assert!(
            !items
                .iter()
                .any(|item| item.program.ends_with("/ruby_protocol"))
        );
        assert!(!suite_groups()["suspect-codegen"].contains("ruby_protocol"));

        let witnesses = [
            "native_m2_gem_types_docs_examples_and_independent_wire",
            "native_shared_contract_exact_json_and_adversarial_models",
            "native_shared_branch_copy_equality_and_number_budgets",
            "native_openrouter_five_actual_operations",
            "native_expanded_protocol_wire_parts_streams_and_types",
            "native_additional_openrouter_binary_and_delete_operations",
            "native_oas30_nullable_reference_siblings_and_binary",
        ];
        assert_eq!(expected_native_tests("ruby_sdk"), witnesses);
        let transcript = |missing: Option<&str>| {
            let included = witnesses
                .iter()
                .copied()
                .filter(|name| Some(*name) != missing)
                .collect::<Vec<_>>();
            let mut text = included
                .iter()
                .map(|name| format!("test {name} ... ok\n"))
                .collect::<String>();
            text.push_str(&format!("test result: ok. {} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n", included.len()));
            text
        };
        assert!(harness_evidence("ruby_sdk", &transcript(None), "").is_ok());
        for witness in witnesses {
            let error = harness_evidence("ruby_sdk", &transcript(Some(witness)), "").unwrap_err();
            assert!(error.to_string().contains(witness), "{error}");
        }
    }

    #[test]
    fn python_null_and_swift_protocol_regressions_require_both_tiers_and_exact_witnesses() {
        let items = suites();
        let cases: &[(&str, &[&str])] = &[
            (
                "python_null_models",
                &[
                    "null_only_fields_aliases_containers_and_unions_preserve_native_typing_and_values",
                ],
            ),
            (
                "swift_protocol",
                &[
                    "native_protocol_spm_types_wire_stream_lifetimes_and_docs",
                    "native_remaining_standard_custom_query_positional",
                ],
            ),
        ];
        let transcript = |names: &[&str]| {
            let mut text = names
                .iter()
                .map(|name| format!("test {name} ... ok\n"))
                .collect::<String>();
            text.push_str(&format!("test result: ok. {} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n", names.len()));
            text
        };
        for (name, witnesses) in cases {
            let invocations = items
                .iter()
                .filter(|item| item.program == format!("test:suspect-codegen/{name}"))
                .collect::<Vec<_>>();
            assert_eq!(invocations.len(), 2);
            assert_eq!(expected_native_tests(name), *witnesses);
            for tier in ["floor", "current"] {
                let item = invocations
                    .iter()
                    .find(|item| item.id == format!("{tier}-{name}"))
                    .unwrap();
                assert_eq!(
                    item.args,
                    strings(&["--include-ignored", "--show-output", "--test-threads=1"])
                );
                if *name == "python_null_models" {
                    assert_eq!(
                        item.environment["SUSPECT_PYTHON_BIN"],
                        format!("{{python-{tier}}}")
                    );
                    assert_eq!(item.environment["SUSPECT_PYTHON_TOOLS"], "{python-tools}");
                    assert_eq!(
                        item.environment["SUSPECT_TEST_ARTIFACT_ROOT"],
                        item.environment["TMPDIR"]
                    );
                }
            }
            assert!(harness_evidence(name, &transcript(witnesses), "").is_ok());
            for &witness in *witnesses {
                let mut incomplete = witnesses
                    .iter()
                    .copied()
                    .filter(|name| *name != witness)
                    .collect::<Vec<_>>();
                incomplete.push("host_only_planning");
                let error = harness_evidence(name, &transcript(&incomplete), "").unwrap_err();
                assert!(error.to_string().contains(witness), "{error}");
            }
        }
    }

    #[test]
    fn full_swift_floor_uses_restored_payload_and_preserves_explicit_tool_selections() {
        let scratch = Path::new("/caller-tmp/opencode");
        let expected = scratch
            .join("swift-6.0.3-protocol-reexpanded/swift-6.0.3-RELEASE-osx-package.pkg/Payload");
        let defaults = swift_floor_selection(scratch, &BTreeMap::new());
        assert_eq!(
            defaults["{swift-floor-root}"],
            expected.display().to_string()
        );
        for (token, file) in [
            ("{swift-floor}", "swift"),
            ("{swiftc-floor}", "swiftc"),
            ("{docc-floor}", "docc"),
        ] {
            assert_eq!(
                defaults[token],
                expected.join("usr/bin").join(file).display().to_string()
            );
        }
        assert_eq!(
            defaults["{swift-floor-sdk}"],
            "/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk"
        );
        let mut run = fixture_run(Path::new("/unused-test-workspace"));
        run.replacements = defaults;
        let environment = native_environment("swift", "floor");
        assert_eq!(
            run.expand(&environment["SUSPECT_SWIFT_BIN"]),
            expected.join("usr/bin/swift").display().to_string()
        );
        assert_eq!(
            run.expand(&environment["SWIFT_EXEC"]),
            expected.join("usr/bin/swiftc").display().to_string()
        );

        let mut overrides = BTreeMap::from([(
            "SUSPECT_SWIFT_FLOOR_ROOT".into(),
            PathBuf::from("/explicit/payload"),
        )]);
        let selected = swift_floor_selection(scratch, &overrides);
        assert_eq!(selected["{swift-floor}"], "/explicit/payload/usr/bin/swift");
        overrides.insert(
            "SUSPECT_SWIFT_FLOOR_BIN".into(),
            "/explicit/driver/bin/swift".into(),
        );
        let selected = swift_floor_selection(scratch, &overrides);
        assert_eq!(selected["{swift-floor-root}"], "/explicit/payload");
        assert_eq!(selected["{swiftc-floor}"], "/explicit/driver/bin/swiftc");
        assert_eq!(selected["{docc-floor}"], "/explicit/driver/bin/docc");
        overrides.extend([
            (
                "SUSPECT_SWIFTC_FLOOR_BIN".into(),
                "/explicit/compiler/swiftc".into(),
            ),
            (
                "SUSPECT_SWIFT_FLOOR_DOCC_BIN".into(),
                "/explicit/documentation/docc".into(),
            ),
            (
                "SUSPECT_SWIFT_FLOOR_SDKROOT".into(),
                "/explicit/SDKs/MacOSX15.4.sdk".into(),
            ),
        ]);
        let selected = swift_floor_selection(scratch, &overrides);
        assert_eq!(selected["{swift-floor}"], "/explicit/driver/bin/swift");
        assert_eq!(selected["{swiftc-floor}"], "/explicit/compiler/swiftc");
        assert_eq!(selected["{docc-floor}"], "/explicit/documentation/docc");
        assert_eq!(
            selected["{swift-floor-sdk}"],
            "/explicit/SDKs/MacOSX15.4.sdk"
        );
    }

    #[test]
    fn scoped_native_library_inventory_covers_real_tiers_without_cross_language_replays() {
        let stages = library_native_stages();
        let names = LIBRARY_NATIVE
            .iter()
            .map(|case| case.name)
            .collect::<BTreeSet<_>>();
        assert_eq!(names.len(), LIBRARY_NATIVE.len());
        assert_eq!(stages.len(), 50);
        for case in LIBRARY_NATIVE {
            let expected = if case.language == "csharp" || case.language == "cpp" {
                1
            } else {
                2
            };
            let selected = stages
                .iter()
                .filter(|stage| stage.args.iter().any(|arg| arg == case.name))
                .collect::<Vec<_>>();
            assert_eq!(selected.len(), expected, "{}", case.name);
            for stage in selected {
                assert_eq!(stage.program, CODEGEN_LIBRARY);
                assert_eq!(stage.args[1], "--exact");
                assert_eq!(stage.args[2], case.name);
                if case.language == "dart" {
                    assert_eq!(stage.environment["SUSPECT_DART_REPO_ROOT"], "{workspace}");
                    assert!(stage.environment["SUSPECT_DART_BIN"].contains(
                        if stage.id.starts_with("floor-") {
                            "floor"
                        } else {
                            "current"
                        }
                    ));
                }
                if case.language == "swift" {
                    assert_eq!(
                        stage.environment["SUSPECT_SWIFT_SDKROOT"],
                        if stage.id.starts_with("floor-") {
                            "{swift-floor-sdk}"
                        } else {
                            "{swift-sdk}"
                        }
                    );
                    if case.id.starts_with("swift-v3-") || case.id == "swift-document-base" {
                        assert_eq!(
                            stage.environment["SUSPECT_SWIFT_V3_ROOT"],
                            format!(
                                "{{work}}/native/swift/{}/v3",
                                if stage.id.starts_with("floor-") {
                                    "floor"
                                } else {
                                    "current"
                                }
                            )
                        );
                    }
                }
            }
        }
        assert!(names.contains("cpp_sdk::v2_tests::native_v2_independent_32"));
        assert!(names.contains("cpp_sdk::v2_tests::native_v2_sdk_operations"));
        assert!(names.contains("cpp_sdk::server_tests::native_document_relative_servers"));
        for name in [
            "cpp_sdk::v3_tests::native_v3_official_dynamic_ref_44",
            "cpp_sdk::v3_tests::native_v3_scope_and_resource_controls",
            "cpp_sdk::v3_tests::native_v3_sdk_operations",
        ] {
            assert!(names.contains(name));
        }
        assert!(names.contains("dart_sdk::v3_sdk_tests::native_v3_sdk_operations"));
        for name in [
            "swift_sdk::validation::v3_tests::native_resource_dynamic_source_vectors",
            "swift_sdk::resources_tests::native_installed_v3_resources_codecs_types_wire_and_docs",
            "swift_sdk::resources_tests::native_installed_physical_document_servers",
            "swift_sdk::aggregate_examples_tests::native_installed_declared_aggregate_examples",
        ] {
            assert!(names.contains(name));
        }
        for name in [
            "csharp_sdk::resources_tests::native_resource_source_vectors",
            "csharp_sdk::resources_tests::native_resource_scope_and_admission",
            "csharp_sdk::resources_tests::native_resource_sdk_packages",
            "csharp_sdk::credential_env_tests::native_environment_credentials",
            "csharp_sdk::credential_env_tests::native_openrouter_environment_client",
        ] {
            let case = LIBRARY_NATIVE
                .iter()
                .find(|case| case.name == name)
                .unwrap();
            assert_eq!(case.tiers, &["matrix"]);
            assert_eq!(
                stages
                    .iter()
                    .filter(|stage| stage.args.iter().any(|arg| arg == name))
                    .count(),
                1
            );
        }
        for stage in stages
            .iter()
            .filter(|stage| stage.id.starts_with("matrix-csharp-credential-env-"))
        {
            assert_eq!(stage.environment["SUSPECT_DOTNET_BIN"], "{dotnet}");
            assert_eq!(stage.environment["OPENROUTER_WEB_ROOT"], "{out}/inputs");
        }
        assert!(!names.iter().any(|name| name.contains("before_admission")));
        let items = suites();
        let ignored_encoding = items
            .iter()
            .filter(|item| {
                item.program == "test:suspect-codegen/typescript_multipart_ignored_encoding"
            })
            .collect::<Vec<_>>();
        assert_eq!(ignored_encoding.len(), 1);
        assert_eq!(
            ignored_encoding[0].id,
            "matrix-typescript_multipart_ignored_encoding"
        );
        assert_eq!(
            ignored_encoding[0].environment["SUSPECT_DOCS_NODE"],
            "{node22}"
        );
        assert_eq!(
            ignored_encoding[0].environment["SUSPECT_NODE24_BIN"],
            "{node24}"
        );
        assert_eq!(
            ignored_encoding[0].environment["SUSPECT_PROTOCOL_ARTIFACTS"],
            "{work}/native/typescript/matrix/ignored-encoding"
        );
        assert_eq!(
            expected_native_tests("typescript_multipart_ignored_encoding"),
            &["ignored_multipart_styles_preserve_content_in_installed_requests_and_responses"]
        );
        let positional = items
            .iter()
            .filter(|item| item.program == "test:suspect-codegen/csharp_positional")
            .collect::<Vec<_>>();
        assert_eq!(positional.len(), 1);
        assert_eq!(positional[0].id, "matrix-csharp_positional");
        assert_eq!(
            positional[0].environment,
            native_environment("csharp", "matrix")
        );
        assert_eq!(expected_native_tests("csharp_positional").len(), 7);
        for suite_name in [
            "go_schema_v2",
            "go_schema_v3",
            "go_examples_aggregate",
            "ruby_schema_v2",
            "ruby_credential_env",
            "rust_validation_v2",
            "rust_validation_v3",
            "rust_protocol_v3",
            "rust_protocol_resources",
            "swift_validation_v2",
            "kotlin_validation_v2",
            "kotlin_validation_v3",
            "kotlin_protocol_documents",
            "java_schema_v2",
            "java_schema_v3",
            "java_aggregate_examples",
            "dart_credential_env",
        ] {
            assert_eq!(
                items
                    .iter()
                    .filter(|item| item.program == format!("test:suspect-codegen/{suite_name}"))
                    .count(),
                2,
                "{suite_name}"
            );
            assert!(!expected_native_tests(suite_name).is_empty());
        }
        for tier in ["floor", "current"] {
            let ruby = items
                .iter()
                .find(|item| item.id == format!("{tier}-ruby_credential_env"))
                .unwrap();
            assert_eq!(
                ruby.environment["SUSPECT_RUBY_HOME"],
                format!("{{ruby-{tier}}}")
            );
            assert_eq!(
                ruby.environment["SUSPECT_RUBY_GEMS"],
                format!("{{ruby-gems-{tier}}}")
            );
            assert_eq!(ruby.environment["OPENROUTER_WEB_ROOT"], "{out}/inputs");
            for key in [
                "SUSPECT_RUBY_GENERATOR_CANARY",
                "RUBY_ENV_BEARER",
                "OPENROUTER_API_KEY",
            ] {
                assert_eq!(
                    ruby.environment[key],
                    "RubyGeneratorCanaryMustNotBeEmitted0123456789"
                );
            }
            let dart = items
                .iter()
                .find(|item| item.id == format!("{tier}-dart_credential_env"))
                .unwrap();
            assert_eq!(
                dart.environment["SUSPECT_DART_BIN"],
                format!("{{dart-{tier}}}")
            );
            assert_eq!(dart.environment["SUSPECT_DART_REPO_ROOT"], "{workspace}");
            assert_eq!(
                dart.environment["SUSPECT_DART_GATE_ROOT"],
                format!("{{work}}/native/dart/{tier}/gates")
            );
            assert_eq!(dart.environment["OPENROUTER_WEB_ROOT"], "{out}/inputs");
            assert_eq!(dart.environment["PATH"], "{node22-bin}:{path}");
            for name in ["go_schema_v3", "go_examples_aggregate"] {
                let item = items
                    .iter()
                    .find(|item| item.id == format!("{tier}-{name}"))
                    .unwrap();
                let toolchain = if tier == "floor" {
                    "go1.23.12"
                } else {
                    "go1.27.1"
                };
                assert_eq!(item.environment["SUSPECT_GO_TOOLCHAIN"], toolchain);
                assert_eq!(item.environment["GOTOOLCHAIN"], toolchain);
                assert_eq!(item.environment["SUSPECT_SPHINX_PYTHON"], "{python-tools}");
            }
            for (name, selector, directory) in [
                ("rust_validation_v3", "SUSPECT_RUST_V3_TARGET", "v3/runtime"),
                ("rust_protocol_v3", "SUSPECT_RUST_V3_HTTP_TARGET", "v3/sdk"),
                (
                    "rust_protocol_resources",
                    "SUSPECT_RUST_RESOURCES_TARGET",
                    "physical-servers",
                ),
            ] {
                let item = items
                    .iter()
                    .find(|item| item.id == format!("{tier}-{name}"))
                    .unwrap();
                assert_eq!(
                    item.environment[selector],
                    format!("{{work}}/native/rust/{tier}/{directory}/cargo")
                );
                assert_eq!(
                    item.environment["SUSPECT_NATIVE_RUST_TOOLCHAIN"],
                    if tier == "floor" { "1.88.0" } else { "stable" }
                );
                assert_eq!(
                    item.environment["RUSTUP_TOOLCHAIN"],
                    item.environment["SUSPECT_NATIVE_RUST_TOOLCHAIN"]
                );
            }
            let go = items
                .iter()
                .find(|item| item.id == format!("{tier}-go_models"))
                .unwrap();
            assert_eq!(
                go.environment["SUSPECT_GO_TOOLCHAIN"],
                if tier == "floor" {
                    "go1.23.12"
                } else {
                    "local"
                }
            );
        }
        for (name, version, witnesses) in [
            ("typescript_applicators", "V2", 7),
            ("typescript_resources", "V3", 9),
        ] {
            let matrix = items
                .iter()
                .filter(|item| item.program == format!("test:suspect-codegen/{name}"))
                .collect::<Vec<_>>();
            assert_eq!(matrix.len(), 1, "{name}");
            let item = matrix[0];
            assert_eq!(item.id, format!("matrix-{name}"));
            assert_eq!(item.environment["SUSPECT_DOCS_NODE"], "{node22}");
            assert_eq!(item.environment["SUSPECT_NODE24_BIN"], "{node24}");
            assert_eq!(item.environment["SUSPECT_CHROMIUM"], "{chromium}");
            assert_eq!(
                item.environment[&format!("SUSPECT_TYPESCRIPT_{version}_MATRIX")],
                "1"
            );
            assert_eq!(
                item.environment[&format!("SUSPECT_TYPESCRIPT_{version}_ARTIFACTS")],
                format!(
                    "{{work}}/native/typescript/matrix/{}",
                    version.to_ascii_lowercase()
                )
            );
            assert!(item.environment["PATH"].contains("{node22-bin}"));
            assert_eq!(expected_native_tests(name).len(), witnesses);
        }
        for tier in ["floor", "current"] {
            let aggregate = items
                .iter()
                .find(|item| item.id == format!("{tier}-java_aggregate_examples"))
                .unwrap();
            assert_eq!(
                aggregate.environment["JAVA_HOME"],
                format!("{{jdk-{tier}}}")
            );
            assert_eq!(
                aggregate.environment["SUSPECT_JAVA_HOME"],
                format!("{{jdk-{tier}}}")
            );
        }
        assert_eq!(
            items
                .iter()
                .filter(|item| item.program == "test:suspect-codegen/python_applicators")
                .count(),
            1
        );
        for name in [
            "python_schema_v2",
            "python_resources",
            "python_schema_v3",
            "python_document_servers",
        ] {
            let matrix = items
                .iter()
                .filter(|item| item.program == format!("test:suspect-codegen/{name}"))
                .collect::<Vec<_>>();
            assert_eq!(matrix.len(), 1, "{name}");
            assert_eq!(
                matrix[0].environment["SUSPECT_PYTHON_SCOPED_VERSIONS"],
                "3.11,3.14"
            );
            assert!(!expected_native_tests(name).is_empty());
        }
        for name in [
            "owned_applicators",
            "owned_applicator_conformance",
            "owned_resources",
            "contract_resources",
        ] {
            assert!(
                items
                    .iter()
                    .any(|item| item.program.ends_with(&format!("/{name}")))
            );
        }
    }

    #[test]
    fn ruby_resource_and_physical_server_suites_require_both_tiers_and_every_witness() {
        let cases: &[(&str, &[&str])] = &[
            (
                "ruby_schema_v3",
                &[
                    "source_driven_official_v3_resources_scopes_and_guards",
                    "installed_resource_sdk_models_types_examples_and_wire",
                    "resource_native_descriptors_and_profile_selection_preserve_physical_identity",
                ],
            ),
            (
                "ruby_document_servers",
                &[
                    "physical_base_metadata_is_separate_and_schema_resource_fences_stay_closed",
                    "installed_gem_uses_effective_physical_document_server_bases",
                ],
            ),
        ];
        let items = suites();
        for (name, witnesses) in cases {
            assert_eq!(expected_native_tests(name), *witnesses);
            assert_eq!(
                items
                    .iter()
                    .filter(|item| item.program == format!("test:suspect-codegen/{name}"))
                    .count(),
                2
            );
            for tier in ["floor", "current"] {
                let item = items
                    .iter()
                    .find(|item| item.id == format!("{tier}-{name}"))
                    .unwrap();
                assert_eq!(
                    item.environment["SUSPECT_RUBY_HOME"],
                    format!("{{ruby-{tier}}}")
                );
                assert_eq!(
                    item.environment["SUSPECT_RUBY_GEMS"],
                    format!("{{ruby-gems-{tier}}}")
                );
                assert_eq!(
                    item.args,
                    strings(&["--include-ignored", "--show-output", "--test-threads=1"])
                );
            }
            let transcript = |omitted: Option<&str>| {
                let mut text = witnesses
                    .iter()
                    .filter(|name| Some(**name) != omitted)
                    .map(|name| format!("test {name} ... ok\n"))
                    .collect::<String>();
                if omitted.is_some() {
                    text.push_str("test unrelated_host ... ok\n");
                }
                text.push_str(&format!("test result: ok. {} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n", witnesses.len()));
                text
            };
            assert!(harness_evidence(name, &transcript(None), "").is_ok());
            for &witness in *witnesses {
                let error = harness_evidence(name, &transcript(Some(witness)), "").unwrap_err();
                assert!(error.to_string().contains(witness), "{error}");
            }
        }
    }

    #[test]
    fn go_scoped_native_and_document_witnesses_are_required_with_rendered_docs_enabled() {
        let witnesses = [
            "ordinary_media_examples_have_one_entry_binding_and_rich_recipes_remain_explicit",
            "checked_program_fences_and_base_v1_program_identity",
            "scoped_32_source_vectors_execute_in_installed_consumers",
            "scoped_models_codecs_and_sdk_operations_preserve_native_data",
            "scoped_native_recursion_depth_and_call_isolation",
            "native_physical_document_base_redirect_and_encoded_path_witness",
        ];
        assert_eq!(expected_native_tests("go_schema_v2"), witnesses);
        let items = suites();
        for tier in ["floor", "current"] {
            let item = items
                .iter()
                .find(|item| item.id == format!("{tier}-go_schema_v2"))
                .unwrap();
            assert_eq!(item.environment["SUSPECT_SPHINX_PYTHON"], "{python-tools}");
            assert_eq!(
                item.environment["SUSPECT_GO_TOOLCHAIN"],
                if tier == "floor" {
                    "go1.23.12"
                } else {
                    "local"
                }
            );
        }
        let transcript = |omitted: Option<&str>| {
            let mut text = witnesses
                .iter()
                .filter(|name| Some(**name) != omitted)
                .map(|name| format!("test {name} ... ok\n"))
                .collect::<String>();
            if omitted.is_some() {
                text.push_str("test unrelated_host ... ok\n");
            }
            text.push_str("test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n");
            text
        };
        assert!(harness_evidence("go_schema_v2", &transcript(None), "").is_ok());
        for witness in witnesses {
            let error =
                harness_evidence("go_schema_v2", &transcript(Some(witness)), "").unwrap_err();
            assert!(error.to_string().contains(witness), "{error}");
        }
    }

    #[test]
    fn frozen_library_census_and_exact_results_refuse_unassigned_missing_or_filtered_work() {
        let all = "host: test\nnative_a: test\nnative_b: test\n\n3 tests, 0 benchmarks\n";
        let native = "native_a: test\nnative_b: test\n\n2 tests, 0 benchmarks\n";
        let declared = strings(&["native_a", "native_b"]).into_iter().collect();
        let census = validate_library_census(all, native, &declared).unwrap();
        assert!(validate_library_census(all, "", &declared).is_err());
        assert!(
            validate_library_census(all, "native_a: test\n\n1 test, 0 benchmarks\n", &declared)
                .is_err()
        );
        assert!(
            validate_library_census(&format!("{all}unexpected: test\n"), native, &declared)
                .is_err()
        );
        assert!(
            validate_library_census(all, native, &strings(&["native_a"]).into_iter().collect())
                .is_err()
        );
        let host = "test host ... ok\ntest native_a ... ignored, native\ntest native_b ... ignored, native\ntest result: ok. 1 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out; finished in 0.00s\n";
        assert!(library_execution_evidence(&census, None, host, "").is_ok());
        assert!(
            library_execution_evidence(&census, None, &host.replace("test host ... ok\n", ""), "")
                .is_err()
        );
        let run = "test native_a ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out; finished in 0.00s\n";
        assert!(library_execution_evidence(&census, Some("native_a"), run, "").is_ok());
        for bad in [
            run.replace("2 filtered", "1 filtered"),
            run.replace("0 ignored", "1 ignored"),
            run.replace("native_a ... ok", "native_b ... ok"),
            String::new(),
        ] {
            assert!(library_execution_evidence(&census, Some("native_a"), &bad, "").is_err());
        }
        assert!(
            library_execution_evidence(&census, Some("native_a"), run, "native gate skipped")
                .is_err()
        );
    }

    fn editor_report_fixture(scenario: &str) -> (Value, Value) {
        let profiles = TARGETS
            .iter()
            .map(|target| target.backend)
            .collect::<Vec<_>>();
        let pins = json!({"format":"suspect.editor.native-host.pins.v1","cli":{"sha256":"a".repeat(64),"expectedProfiles":profiles},"vscode":{"executableSha256":"b".repeat(64),"archiveSha256":"c".repeat(64),"version":"1.137.0","commit":"d".repeat(40),"platform":"darwin-arm64"},"tools":{"lockSha256":"e".repeat(64)},"extension":{"sourceSha256":"f".repeat(64),"vsix":{"sha256":"1".repeat(64)}}});
        let report = json!({"format":"suspect.editor.native-host.run.v2","runId":"synthetic-unit-fixture","mode":"run","scenario":scenario,"status":"passed","phase":"complete","exitCode":0,"hostExit":{"code":0,"signal":null,"timedOut":false},"requiredNativeChecks":editor_checks(scenario),"checks":editor_checks(scenario).iter().map(|name|json!({"name":name,"status":"passed"})).collect::<Vec<_>>(),"requiredClaims":EDITOR_CLAIMS,"claims":EDITOR_CLAIMS.iter().map(|name|((*name).to_owned(),json!(true))).collect::<BTreeMap<_,_>>(),"requiredScreenshots":editor_screenshots(scenario),"inputs":{"pins":pins,"cli":{"sha256":pins["cli"]["sha256"],"expectedProfiles":profiles},"extension":{"sha256":pins["extension"]["sourceSha256"]},"vscode":{"executable":{"sha256":pins["vscode"]["executableSha256"]},"archive":{"sha256":pins["vscode"]["archiveSha256"]},"version":"1.137.0","commit":"d".repeat(40),"platform":"darwin-arm64"},"tools":{"lock":{"sha256":pins["tools"]["lockSha256"]}}},"inventory":{"format":"suspect.sdk.profiles.v1","profiles":TARGETS.iter().map(|target|json!({"profile":target.backend,"directory":target.language})).collect::<Vec<_>>()},"vsix":{"mode":"supplied","sha256":pins["extension"]["vsix"]["sha256"]},"baseline":{"owner":"native-host-fixture","files":7}});
        (report, pins)
    }

    #[test]
    fn native_editor_requires_both_full_scenarios_exact_twelve_profiles_checks_and_claims() {
        let stages = editor_native_stages();
        for scenario in ["lifecycle", "commands"] {
            let step = stages
                .iter()
                .find(|stage| stage.id == format!("editor-native-{scenario}"))
                .unwrap();
            assert_eq!(step.environment["SUSPECT_NATIVE_MODE"], "run");
            assert_eq!(step.environment["SUSPECT_TEST_BINARY"], "{out}/bin/suspect");
            assert_eq!(
                step.environment["SUSPECT_NATIVE_OUT"],
                format!("{{out}}/editor-native/{scenario}")
            );
            let (good, pins) = editor_report_fixture(scenario);
            editor_report_contract(&good, scenario, &pins).unwrap();
            for (pointer, value) in [
                ("/mode", json!("selection-probe")),
                ("/format", json!("legacy")),
                ("/scenario", json!("other")),
                ("/status", json!("incomplete")),
                ("/exitCode", json!(2)),
                ("/hostExit/code", json!(1)),
                ("/hostExit/timedOut", json!(true)),
                ("/checks/0/status", json!("skipped")),
                ("/checks/0/name", json!("renamed")),
                ("/claims/inputsUnchanged", json!("true")),
                ("/baseline/files", json!(0)),
                ("/vsix/mode", json!("packaged")),
            ] {
                let mut bad = good.clone();
                *bad.pointer_mut(pointer).unwrap() = value;
                assert!(
                    editor_report_contract(&bad, scenario, &pins).is_err(),
                    "{pointer}"
                );
            }
            for count in [8, 11] {
                let mut bad = good.clone();
                let mut subset = pins.clone();
                subset["cli"]["expectedProfiles"]
                    .as_array_mut()
                    .unwrap()
                    .truncate(count);
                bad["inputs"]["pins"] = subset.clone();
                bad["inputs"]["cli"]["expectedProfiles"] =
                    subset["cli"]["expectedProfiles"].clone();
                bad["inventory"]["profiles"]
                    .as_array_mut()
                    .unwrap()
                    .truncate(count);
                assert!(editor_report_contract(&bad, scenario, &subset).is_err());
            }
            let mut bad = good.clone();
            bad["checks"].as_array_mut().unwrap().pop();
            assert!(editor_report_contract(&bad, scenario, &pins).is_err());
            let mut bad = good.clone();
            bad["claims"]["selectionProbeOnly"] = json!(true);
            assert!(editor_report_contract(&bad, scenario, &pins).is_err());
            assert!(editor_report_contract(&json!({"complete":true}), scenario, &pins).is_err());
        }
        let native = editor_environment(&BTreeMap::from([
            ("SUSPECT_NATIVE_RUST_TOOLCHAIN".into(), "stable".into()),
            ("SUSPECT_NATIVE_MODE".into(), "selection-probe".into()),
            ("SUSPECT_TEST_BINARY".into(), "/pinned/cli".into()),
            ("PATH".into(), "/pinned/node:/usr/bin".into()),
        ]));
        assert!(
            !native
                .keys()
                .any(|name| name.starts_with("SUSPECT_NATIVE_"))
        );
        assert_eq!(native["SUSPECT_TEST_BINARY"], "/pinned/cli");
        let default = default_cli_stages();
        assert!(!default[0].args.contains(&"--features".into()));
        assert!(default[1].program.ends_with("suspect-default"));
    }

    #[test]
    fn native_editor_tool_pins_cannot_choose_the_cli_source_vsix_or_acceptance_commands() {
        let mut pins = json!({"format":"suspect.sdk.full.editor-tools.v1","vscode":{"executable":"/tools/Code","executableSha256":"a".repeat(64),"archive":"/tools/Code.zip","archiveSha256":"b".repeat(64),"version":"1.137.0","commit":"c".repeat(40),"platform":"darwin-arm64"},"tools":{"directory":"/tools/native","lockSha256":"d".repeat(64)}});
        assert!(serde_json::from_value::<EditorTools>(pins.clone()).is_ok());
        for field in [
            "cli",
            "extension",
            "vsix",
            "command",
            "mode",
            "expectedProfiles",
        ] {
            let mut bad = pins.clone();
            bad[field] = json!({});
            assert!(
                serde_json::from_value::<EditorTools>(bad).is_err(),
                "{field}"
            );
        }
        pins["tools"]["skipVerification"] = json!(true);
        assert!(serde_json::from_value::<EditorTools>(pins).is_err());
        let root = tempfile::tempdir().unwrap();
        let mut run = fixture_run(root.path());
        assert!(load_editor_tools(&mut run, None).is_err());
        assert!(editor_evidence(&mut run, "lifecycle").is_err());
        assert!(editor_evidence(&mut run, "commands").is_err());
    }

    #[test]
    fn functional_profile_retains_all_native_gates_and_cannot_claim_calibration() {
        let strict = stage_inventory(false).unwrap();
        let functional = stage_inventory(true).unwrap();
        assert_eq!(strict["stageCount"], functional["stageCount"]);
        assert_eq!(functional["requiredStageCount"], 920);
        assert_eq!(
            strict["requiredStageCount"].as_u64().unwrap(),
            functional["requiredStageCount"].as_u64().unwrap() + 4
        );
        let required = required_stages();
        let mut checks = required.iter().map(|item| json!({"id":item.id,"criterionMet":!PERFORMANCE.contains(&item.id.as_str())})).collect::<Vec<_>>();
        let result = completion(true, &required, &checks, false);
        assert_eq!(result["profileComplete"], true);
        assert_eq!(result["complete"], false);
        assert_eq!(result["status"], "functional-passed-numerical-pending");
        assert_eq!(
            completion(false, &required, &checks, false)["profileComplete"],
            false
        );
        checks
            .iter_mut()
            .find(|check| check["id"] == "floor-java_sdk")
            .unwrap()["criterionMet"] = json!(false);
        assert_eq!(
            completion(true, &required, &checks, false)["profileComplete"],
            false
        );
        assert_eq!(
            completion(true, &required, &checks, true)["profileComplete"],
            false
        );
    }

    #[test]
    fn unregistered_duplicate_and_empty_evidence_inventories_fail_closed() {
        assert_eq!(completion(true, &[], &[], false)["complete"], false);
        let required = required_stages();
        let good = required
            .iter()
            .map(|item| json!({"id":item.id,"criterionMet":true}))
            .collect::<Vec<_>>();
        assert_eq!(completion(false, &required, &good, false)["complete"], true);
        let mut checks = good.clone();
        checks.push(json!({"id":"caller-provided-pass","criterionMet":true}));
        assert_eq!(
            completion(false, &required, &checks, false)["complete"],
            false
        );
        let mut checks = good;
        checks.push(checks[0].clone());
        assert_eq!(
            completion(false, &required, &checks, false)["complete"],
            false
        );
    }

    #[test]
    fn original_runner_inventory_keeps_historical_scope_and_native_opt_ins() {
        // Visibility-only reuse must not turn the old five-language demo into a
        // twelve-language claim. Its existing 208/212 regression tests also run.
        let old = sealed::suite_stages();
        assert!(!old.iter().any(|item| item.program.contains("java_sdk") || item.program.contains("_protocol")));
        for item in suites() {
            if item.program.ends_with("/pinned_transport") {
                continue;
            }
            if item.id == "library-codegen-host" {
                assert_eq!(item.program, CODEGEN_LIBRARY);
                assert_eq!(item.args, strings(&["--show-output", "--test-threads=1"]));
                assert!(
                    required_stages()
                        .iter()
                        .any(|item| item.id == "library-codegen-coverage")
                );
                continue;
            }
            assert!(item.args.contains(&"--include-ignored".into()));
            assert!(!item.args.contains(&"--ignored".into()));
            assert!(!item.args.contains(&"--skip".into()));
            assert!(item.args.contains(&"--show-output".into()));
        }
    }

    #[test]
    fn language_native_identities_and_real_tier_selectors_are_maintained() {
        let config = target_config();
        let items = suites();
        for target in TARGETS.iter().filter(|target| target.language != "csharp") {
            let name = format!("{}_credential_env", target.language);
            let matches = items
                .iter()
                .filter(|item| item.program == format!("test:suspect-codegen/{name}"))
                .collect::<Vec<_>>();
            let count = if ["typescript", "python", "cpp"].contains(&target.language) {
                1
            } else {
                2
            };
            assert_eq!(matches.len(), count, "{name}");
            assert!(!expected_native_tests(&name).is_empty(), "{name}");
            for item in matches {
                assert_eq!(item.environment["OPENROUTER_WEB_ROOT"], "{out}/inputs");
                assert_eq!(
                    item.args,
                    strings(&["--include-ignored", "--show-output", "--test-threads=1"])
                );
            }
        }
        for tier in ["floor", "current"] {
            let go = items
                .iter()
                .find(|item| item.id == format!("{tier}-go_credential_env"))
                .unwrap();
            assert_eq!(
                go.environment["SUSPECT_GO_TOOLCHAIN"],
                if tier == "floor" {
                    "go1.23.12"
                } else {
                    "go1.27.1"
                }
            );
            assert_eq!(
                go.environment["GOTOOLCHAIN"],
                go.environment["SUSPECT_GO_TOOLCHAIN"]
            );
            assert_eq!(
                go.environment["SUSPECT_GO_CREDENTIAL_ENV_EVIDENCE"],
                format!("{{work}}/native/go/{tier}/credential-env")
            );
            assert_eq!(go.environment["SUSPECT_SPHINX_PYTHON"], "{python-tools}");
            let rust = items
                .iter()
                .find(|item| item.id == format!("{tier}-rust_credential_env"))
                .unwrap();
            assert_eq!(
                rust.environment["SUSPECT_NATIVE_RUST_TOOLCHAIN"],
                if tier == "floor" { "1.88.0" } else { "stable" }
            );
            assert_eq!(
                rust.environment["SUSPECT_RUST_CREDENTIAL_ENV_TARGET"],
                format!("{{work}}/native/rust/{tier}/credential-env/cargo")
            );
            let swift = items
                .iter()
                .find(|item| item.id == format!("{tier}-swift_credential_env"))
                .unwrap();
            assert_eq!(
                swift.environment["SUSPECT_SWIFT_CREDENTIAL_ENV_ROOT"],
                format!("{{work}}/native/swift/{tier}/credential-env")
            );
            let java = items
                .iter()
                .find(|item| item.id == format!("{tier}-java_credential_env"))
                .unwrap();
            assert_eq!(java.environment["JAVA_HOME"], format!("{{jdk-{tier}}}"));
            assert_eq!(
                java.environment["SUSPECT_OPENROUTER_OPENAPI"],
                "{out}/inputs/projects/docs/openapi/openapi.yaml"
            );
            assert!(
                !java
                    .environment
                    .contains_key("SUSPECT_JAVA_CREDENTIAL_ENV_MODE")
            );
            let kotlin = items
                .iter()
                .find(|item| item.id == format!("{tier}-kotlin_credential_env"))
                .unwrap();
            assert_eq!(
                kotlin.environment["SUSPECT_KOTLIN_JAVA_HOME"],
                format!("{{jdk-{tier}}}")
            );
            assert_eq!(
                kotlin.environment["SUSPECT_KOTLIN_MAVEN_REPO"],
                "{workspace}/target/sdk-kotlin-maven"
            );
            let php = items
                .iter()
                .find(|item| item.id == format!("{tier}-php_credential_env"))
                .unwrap();
            assert_eq!(
                php.environment["SUSPECT_PHP_BIN"],
                format!("{{php-{tier}}}")
            );
            assert_eq!(php.environment["SUSPECT_COMPOSER_PHAR"], "{composer}");
            assert_eq!(php.environment["SUSPECT_PHPSTAN_PHAR"], "{phpstan}");
        }
        let typescript = items
            .iter()
            .find(|item| item.id == "matrix-typescript_credential_env")
            .unwrap();
        assert_eq!(typescript.environment["SUSPECT_DOCS_NODE"], "{node22}");
        assert_eq!(typescript.environment["SUSPECT_NODE24_BIN"], "{node24}");
        assert_eq!(typescript.environment["SUSPECT_CHROMIUM"], "{chromium}");
        assert_eq!(
            typescript.environment["SUSPECT_CREDENTIAL_ENV_ARTIFACTS"],
            "{work}/native/typescript/matrix/credential-env"
        );
        assert_eq!(
            TARGETS
                .iter()
                .map(|target| target.toolchain_tiers.len())
                .sum::<usize>()
                * 4,
            92
        );
        assert_eq!(
            config.iter().find(|c| c["backend"] == "java-http").unwrap()["package_name"],
            "com.example.generated:sdk-full"
        );
        assert_eq!(
            config
                .iter()
                .find(|c| c["backend"] == "kotlin-http")
                .unwrap()["import_name"],
            "example.sdk"
        );
        assert_eq!(
            config.iter().find(|c| c["backend"] == "php-http").unwrap()["import_name"],
            "Example\\SdkFull"
        );
        assert_eq!(
            native_environment("ruby", "floor")["SUSPECT_RUBY_HOME"],
            "{ruby-floor}"
        );
        assert_eq!(
            native_environment("kotlin", "current")["SUSPECT_KOTLIN_JAVA_HOME"],
            "{jdk-current}"
        );
        assert_eq!(
            native_environment("swift", "floor")["SUSPECT_SWIFT_SDKROOT"],
            "{swift-floor-sdk}"
        );
        assert_eq!(
            native_environment("rust", "floor")["RUSTUP_TOOLCHAIN"],
            "1.88.0"
        );
        assert_eq!(
            native_environment("go", "floor")["GOTOOLCHAIN"],
            "go1.23.12"
        );
    }

    #[test]
    fn argument_and_performance_schemas_refuse_caller_defined_gates() {
        let parse = |args: &[&str]| Args::parse(&strings(args));
        assert!(
            !parse(&["--source", "input", "--out", "output"])
                .unwrap()
                .functional_only
        );
        for args in [
            vec!["--source", "input", "--out", "output", "--stage", "pass"],
            vec!["--source", "input", "--out", "output", "--demo"],
            vec![
                "--source",
                "input",
                "--out",
                "output",
                "--functional-only",
                "--performance-plan",
                "pins",
            ],
            vec!["--source", "--out", "output"],
            vec!["--source", "input", "--source", "other", "--out", "output"],
        ] {
            assert!(parse(&args).is_err());
        }
        let plan = json!({"format":"suspect.sdk.full.performance-plan.v1","evidence":{"path":"/absolute/pins.json","sha256":"0".repeat(64)}});
        assert!(serde_json::from_value::<PerformancePlan>(plan.clone()).is_ok());
        for field in [
            "stages",
            "program",
            "args",
            "assertions",
            "environment",
            "claims",
        ] {
            let mut invalid = plan.clone();
            invalid[field] = json!([]);
            assert!(
                serde_json::from_value::<PerformancePlan>(invalid).is_err(),
                "{field}"
            );
        }
    }

    #[test]
    fn native_summaries_reject_early_returns_and_host_only_protocol_claims() {
        let result = "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n";
        for text in [
            String::new(),
            result.into(),
            format!("test plan_only ... ok\n{result}"),
            format!("test native_wire ... ok\nSkipping native tool\n{result}"),
        ] {
            assert!(
                harness_evidence("fixture_protocol", &text, "").is_err(),
                "{text}"
            );
        }
        assert!(
            harness_evidence(
                "fixture_protocol",
                &format!("test native_wire ... ok\n{result}"),
                ""
            )
            .is_ok()
        );
        assert!(
            harness_evidence(
                "java_sdk",
                &format!("test native_wire ... ok\n{result}"),
                ""
            )
            .is_err()
        );
        assert!(
            harness_evidence(
                "fixture_protocol",
                &format!("test native_wire ... ok\n{result}"),
                "native gate skipped"
            )
            .is_err()
        );
        for suite in [
            "credential_env",
            "credential_env_codegen",
            "go_credential_env_canonical",
            "go_credential_env_factory_capture",
            "swift_credential_env_canonical",
            "typescript_credential_env",
            "python_credential_env",
            "go_credential_env",
            "rust_credential_env",
            "swift_credential_env",
            "java_credential_env",
            "kotlin_credential_env",
            "php_credential_env",
            "cpp_credential_env",
            "http_protocol",
            "typescript_multipart_ignored_encoding",
            "dart_credential_env",
            "ruby_credential_env",
            "contract_dual_role_scope",
            "ruby_compatibility_credentials",
            "csharp_positional",
            "go_models",
            "go_schema_v3",
            "go_examples_aggregate",
            "rust_validation_v3",
            "rust_protocol_v3",
            "rust_protocol_resources",
            "typescript_applicators",
            "typescript_resources",
            "java_aggregate_examples",
        ] {
            let witnesses = expected_native_tests(suite);
            let transcript = |omitted: Option<&str>| {
                let mut text = witnesses
                    .iter()
                    .filter(|name| Some(**name) != omitted)
                    .map(|name| format!("test {name} ... ok\n"))
                    .collect::<String>();
                if omitted.is_some() {
                    text.push_str("test unrelated_host ... ok\n");
                }
                text.push_str(&format!("test result: ok. {} passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n", witnesses.len()));
                text
            };
            assert!(harness_evidence(suite, &transcript(None), "").is_ok());
            for witness in witnesses {
                let error = harness_evidence(suite, &transcript(Some(witness)), "").unwrap_err();
                assert!(error.to_string().contains(witness), "{error}");
            }
        }
    }

    #[test]
    fn successful_fake_command_without_native_output_is_not_acceptance() {
        let root = tempfile::tempdir().unwrap();
        let mut run = fixture_run(root.path());
        let mut fake = command(
            "fake-native",
            "/usr/bin/true",
            &[],
            root.path().to_str().unwrap(),
        );
        fake.criterion = Criterion::RustTests;
        run.command(&fake).unwrap();
        assert_eq!(run.checks[0]["success"], true);
        assert_eq!(run.checks[0]["criterionMet"], false);
        assert!(nonempty(&root.path().join("no-archive.whl")).is_err());
        fs::write(root.path().join("fake.whl"), b"PK").unwrap();
        assert!(nonempty(&root.path().join("fake.whl")).is_err());
        fs::write(root.path().join("fake.json"), b"not JSON").unwrap();
        assert!(nonempty(&root.path().join("fake.json")).is_err());
    }

    #[test]
    fn dirty_snapshot_and_running_verifier_identity_are_fail_closed() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("untracked.rs"), b"actual dirty code").unwrap();
        let files = inventory(
            root.path(),
            strings(&["untracked.rs", "deleted.rs"]).into_iter(),
        )
        .unwrap();
        assert!(runner_source(&files).is_err());
        let copy = root.path().join("snapshot");
        copy_inventory(root.path(), &copy, &files).unwrap();
        fs::write(root.path().join("untracked.rs"), b"owner edited code").unwrap();
        assert_ne!(
            inventory(root.path(), files.keys().cloned()).unwrap(),
            files
        );
        assert_eq!(inventory(&copy, files.keys().cloned()).unwrap(), files);
    }

    #[test]
    fn binary_freezing_is_an_independent_readonly_copy_with_no_replacement() {
        let root = tempfile::tempdir().unwrap();
        let mut run = fixture_run(root.path());
        let source = root.path().join("built");
        let frozen = root.path().join("frozen");
        fs::write(&source, b"first binary").unwrap();
        freeze_file(&mut run, &source, &frozen).unwrap();
        fs::write(&source, b"later Cargo rebuild").unwrap();
        assert_eq!(fs::read(&frozen).unwrap(), b"first binary");
        assert!(fs::metadata(&frozen).unwrap().permissions().readonly());
        assert!(freeze_file(&mut run, &source, &frozen).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn tool_payload_pinning_follows_directory_links_and_detects_mutation_and_cycles() {
        let root = tempfile::tempdir().unwrap();
        let selected = root.path().join("selected");
        let payload = root.path().join("payload");
        fs::create_dir(&selected).unwrap();
        fs::create_dir(&payload).unwrap();
        fs::write(payload.join("runtime.dll"), b"actual runtime").unwrap();
        std::os::unix::fs::symlink(&payload, selected.join("shared")).unwrap();
        let first = external_tree(&selected).unwrap();
        assert_eq!(
            first["files"]["shared/runtime.dll"]["sha256"],
            sha(b"actual runtime")
        );
        fs::write(payload.join("runtime.dll"), b"changed runtime").unwrap();
        assert_ne!(external_tree(&selected).unwrap(), first);
        std::os::unix::fs::symlink(&selected, payload.join("cycle")).unwrap();
        assert!(external_tree(&selected).is_err());
    }

    #[test]
    fn empty_or_unqualified_performance_never_fills_required_slots() {
        let root = tempfile::tempdir().unwrap();
        let mut run = fixture_run(root.path());
        let args = Args::parse(&strings(&["--source", "input", "--out", "output"])).unwrap();
        assert!(load_performance(&mut run, &args).unwrap().is_empty());
        assert_eq!(run.checks.len(), 4);
        assert!(
            run.checks
                .iter()
                .all(|check| check["criterionMet"] == false)
        );
        assert!(run.files.is_empty());
        let path = root.path().join("observational.json");
        fs::write(&path, b"{\"complete\":true}").unwrap();
        let invalid = Pin {
            path,
            sha256: "0".repeat(64),
        };
        assert!(pin_bytes(&invalid).is_err());
        assert!(native_cost_evidence(&run).is_err());
    }

    #[test]
    fn evidence_seal_verifies_file_census_content_and_missing_anchors() {
        let root = tempfile::tempdir().unwrap();
        let report = root.path().join("report");
        fs::create_dir(&report).unwrap();
        fs::write(report.join("result.json"), b"{\"complete\":false}").unwrap();
        assert!(verify_seal(&report).is_err());
        sealed::seal(&report).unwrap();
        assert!(verify_seal(&report).is_ok());
        // Mutate only the tiny owned test fixture, never an acceptance attempt.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                report.join("result.json"),
                fs::Permissions::from_mode(0o600),
            )
            .unwrap();
        }
        fs::write(report.join("result.json"), b"{\"complete\":true}").unwrap();
        assert!(verify_seal(&report).is_err());
    }
}
