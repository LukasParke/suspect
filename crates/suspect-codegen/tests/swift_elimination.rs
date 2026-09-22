//! M2 per-language elimination gate for the Swift HTTP SDK (`swift-http`):
//! real `swift build` consumer binaries measured by this test, mirroring the
//! TypeScript bundler gate's property — a single-operation consumer artifact
//! must not retain unrelated operations, and adding an unrelated operation to
//! the source must not grow the single-op artifact beyond a negligible delta.
//!
//! Two consumer shapes are measured against both the canonical fixture and
//! the same fixture plus one unrelated operation, in both SwiftPM build
//! configurations (debug and release/whole-module):
//! - `one-operation`: an executable that only references `getGadget` (one
//!   plain JSON operation) through the generated `Client` struct.
//! - `codec-only`: an executable that only references the `Codecs.gadget`
//!   model codec, never touching the client, OAuth, pagination or streams.
//!
//! Measured finding (asserted here as a recorded defeat, per the
//! typescript_elimination.rs convention that a divergence is recorded rather
//! than a marker weakened): the Swift linker retains every selected
//! operation in every consumer artifact. Operation methods, pagination
//! walker code and OAuth grant code survive even in a codec-only consumer
//! that never references the client, in debug, in release (whole-module
//! optimization), and with an explicit `-Xlinker -dead_strip`. The
//! unrelated operation costs roughly 84–88 KB in every artifact, so the
//! plan's additivity property does not hold for Swift at link granularity.
//! Swift nominal-type and reflective metadata records appear to root the
//! module's public API; the emitter-side fix (per-operation SwiftPM targets
//! or products, mirroring the Go per-operation functions) is out of scope
//! for this measurement task and is documented in
//! `docs/SDK-ELIMINATION-AUDIT.md`.
//!
//! Retained artifacts live under `target/sdk-swift-elimination/` and a
//! machine-readable record of the last run is written to
//! `target/tmp/swift-elimination-report.json`.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::Arc,
};

use serde_json::{Value, json};
use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
    sdk_defaults::{OAuthDefaults, OAuthSchemeConfig, SdkDefaults},
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

static NATIVE_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Fixed logical source URI for both fixture documents: emitted source
/// references stay byte-comparable across the two generations.
const ENTRY: &str = "https://source.elimination.test/openapi.json";

/// Same-length retained-artifact labels keep emitted source paths
/// length-identical across generations, so binary sizes stay comparable.
const BASE_LABEL: &str = "base";
const EXTENDED_LABEL: &str = "extd";

/// The measured per-unrelated-operation cost bound. The plan's additivity
/// property (≤64 B, the TypeScript slack) does NOT hold for Swift; this gate
/// records the measured defeat: the unrelated operation costs 64–128 KB in
/// every consumer artifact. If an emitter/toolchain change ever achieves
/// link-level elimination, this assertion fails loudly and forces an audit
/// update.
const MEASURED_GROWTH_FLOOR_BYTES: u64 = 64_000;
const MEASURED_GROWTH_CEILING_BYTES: u64 = 128_000;

fn contract_with_document(document: Value) -> Arc<Contract> {
    let entry = Uri::parse(ENTRY).unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            serde_json::to_vec(&document).unwrap(),
        )
        .unwrap()])
        .unwrap(),
    );
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &entry).unwrap())
}

/// The canonical elimination fixture: one limit/offset paginated list
/// operation, one discriminated SSE stream operation, one plain JSON read
/// operation, one OAuth2 client-credentials operation, and one device-flow
/// operation, all under an API-key document policy. Identical to the
/// TypeScript elimination fixture so measurements stay cross-language
/// comparable.
fn elimination_document() -> Value {
    json!({
        "openapi": "3.2.0",
        "info": {"title": "Elimination fixture", "version": "1.0.0"},
        "servers": [{"url": "https://api.elimination.test/v1"}],
        "security": [{"apiKey": []}],
        "paths": {
            "/widgets": {"get": {
                "operationId": "listWidgets",
                "summary": "List widgets with limit/offset pagination.",
                "parameters": [
                    {"name": "limit", "in": "query", "schema": {"type": "integer", "minimum": 1}},
                    {"name": "offset", "in": "query", "schema": {"type": "integer", "minimum": 0}},
                    {"name": "filter", "in": "query", "schema": {"type": "string"}}
                ],
                "responses": {"200": {"description": "Widget page", "content": {"application/json": {"schema": {
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "data": {"type": "array", "items": {"type": "string"}},
                        "total": {"type": "integer"}
                    },
                    "required": ["data", "total"]
                }}}}}
            }},
            "/chat": {"post": {
                "operationId": "streamChat",
                "summary": "Stream chat completion events.",
                "responses": {"200": {"description": "Chat events", "content": {"text/event-stream": {"itemSchema": {
                    "type": "object", "additionalProperties": false,
                    "properties": {
                        "event": {"type": "string", "enum": ["message", "done"]},
                        "data": {"type": "string"}
                    },
                    "required": ["event", "data"]
                }}}}}
            }},
            "/gadgets/{gadgetId}": {"get": {
                "operationId": "getGadget",
                "summary": "Fetch one gadget.",
                "parameters": [{"name": "gadgetId", "in": "path", "required": true, "schema": {"type": "string"}}],
                "responses": {"200": {"description": "Gadget", "content": {"application/json": {
                    "schema": {"$ref": "#/components/schemas/Gadget"}
                }}}}
            }},
            "/banners": {"post": {
                "operationId": "createBanner",
                "summary": "Create a banner with OAuth2 client credentials.",
                "security": [{"serviceOAuth": ["read"]}],
                "requestBody": {"required": true, "content": {"application/json": {
                    "schema": {"$ref": "#/components/schemas/BannerCreate"}
                }}},
                "responses": {"200": {"description": "Banner acknowledged", "content": {"application/json": {"schema": {
                    "type": "object", "additionalProperties": false,
                    "properties": {"ok": {"type": "boolean"}}, "required": ["ok"]
                }}}}}
            }},
            "/licenses": {"get": {
                "operationId": "listLicenses",
                "summary": "List licenses with the device-authorization scheme.",
                "security": [{"deviceOAuth": []}],
                "responses": {"200": {"description": "Licenses", "content": {"application/json": {"schema": {
                    "type": "object", "additionalProperties": false,
                    "properties": {"items": {"type": "array", "items": {"type": "string"}}},
                    "required": ["items"]
                }}}}}
            }}
        },
        "components": {
            "securitySchemes": {
                "apiKey": {"type": "http", "scheme": "bearer"},
                "serviceOAuth": {"type": "oauth2", "flows": {
                    "authorizationCode": {
                        "authorizationUrl": "https://auth.elimination.test/authorize",
                        "tokenUrl": "https://auth.elimination.test/token",
                        "refreshUrl": "https://auth.elimination.test/token-refresh",
                        "scopes": {"read": "Read access", "write": "Write access"}
                    },
                    "clientCredentials": {
                        "tokenUrl": "https://auth.elimination.test/token",
                        "refreshUrl": "https://auth.elimination.test/token-refresh",
                        "scopes": {"read": "Read access"}
                    }
                }},
                "deviceOAuth": {"type": "oauth2", "flows": {"deviceAuthorization": {
                    "deviceAuthorizationUrl": "https://auth.elimination.test/device",
                    "tokenUrl": "https://auth.elimination.test/token",
                    "scopes": {}
                }}}
            },
            "schemas": {
                "BannerCreate": {"type": "object", "additionalProperties": false,
                    "properties": {"text": {"type": "string"}, "weight": {"type": "integer"}},
                    "required": ["text"]},
                "Gadget": {"type": "object", "additionalProperties": false,
                    "properties": {
                        "kind": {"type": "string", "enum": ["standard", "compact"]},
                        "label": {"type": "string"}
                    },
                    "required": ["kind", "label"]}
            }
        }
    })
}

/// The same contract plus one operation that no consumer references. Its
/// pointers sort strictly between existing ones, so every shared source
/// pointer is stable and the two generations are comparable.
fn document_with_unrelated_operation() -> Value {
    let mut document = elimination_document();
    let root = document.as_object_mut().unwrap();
    root.get_mut("paths")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert(
            "/gizmos".to_owned(),
            json!({"get": {
                "operationId": "listGizmos",
                "summary": "Unrelated gizmo inventory probe.",
                "responses": {"200": {"description": "Gizmos", "content": {"application/json": {
                    "schema": {"$ref": "#/components/schemas/UnrelatedGizmo"}
                }}}}
            }}),
        );
    root.get_mut("components")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .get_mut("schemas")
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert(
            "UnrelatedGizmo".to_owned(),
            json!({"type": "object", "additionalProperties": false,
                "properties": {
                    "flavor": {"type": "string", "enum": ["zeta-quantum"]},
                    "serial": {"type": "string"}
                },
                "required": ["flavor", "serial"]}),
        );
    document
}

fn elimination_options() -> GenerationOptions {
    GenerationOptions {
        sdk_defaults: Some(SdkDefaults {
            oauth: OAuthDefaults {
                schemes: BTreeMap::from([
                    (
                        "serviceOAuth".to_owned(),
                        OAuthSchemeConfig {
                            client_id_env: Some("SUSPECT_ELIMINATION_CLIENT_ID".into()),
                            client_secret_env: Some("SUSPECT_ELIMINATION_CLIENT_SECRET".into()),
                            revocation_endpoint: Some(
                                "https://auth.elimination.test/revoke".into(),
                            ),
                            ..OAuthSchemeConfig::default()
                        },
                    ),
                    (
                        "deviceOAuth".to_owned(),
                        OAuthSchemeConfig {
                            client_id_env: Some("SUSPECT_ELIMINATION_DEVICE_ID".into()),
                            client_secret_env: Some("SUSPECT_ELIMINATION_DEVICE_SECRET".into()),
                            ..OAuthSchemeConfig::default()
                        },
                    ),
                ]),
                ..OAuthDefaults::default()
            },
            ..SdkDefaults::v1()
        }),
        ..GenerationOptions::default()
    }
}

fn generate(document: Value) -> Vec<OutFile> {
    let contract = contract_with_document(document);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    generate_with_options(
        contract,
        &selected,
        &TargetConfig {
            backend: Backend::SwiftHttp,
            package_name: "EliminationFixture".into(),
            package_version: "0.0.0".into(),
            import_name: None,
        },
        &elimination_options(),
    )
    .unwrap()
}

/// Retention markers: identifiers and wire literals that survive only inside
/// retained code or retained descriptor data.
const MARKER_OWN_OPERATION: &str = "getGadget";
const MARKER_UNRELATED_OPERATION: &[&str] = &["listGizmos", "zeta-quantum"];
const MARKER_OTHER_OPERATIONS: &[&str] = &[
    "listWidgets",
    "streamChat",
    "createBanner",
    "listLicenses",
    "text/event-stream",
    "PaginationError",
    "client_credentials",
    "urn:ietf:params:oauth:grant-type:device_code",
];

fn swift() -> PathBuf {
    std::env::var_os("SUSPECT_SWIFT_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/usr/bin/swift".into())
}

fn swift_compiler() -> PathBuf {
    std::env::var_os("SUSPECT_SWIFTC_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| swift().with_file_name("swiftc"))
}

fn swift_command(action: &str) -> Command {
    let mut command = Command::new(swift());
    command.arg(action).env("SWIFT_EXEC", swift_compiler());
    if let Some(sdk) = std::env::var_os("SUSPECT_SWIFT_SDKROOT") {
        command.arg("--sdk").arg(&sdk).env("SDKROOT", sdk);
    }
    command
}

fn checked(command: &mut Command, retained: &Path, label: &str) -> Output {
    let output = command.output().unwrap_or_else(|error| {
        panic!("{label}: required Swift toolchain is unavailable: {error}")
    });
    std::fs::create_dir_all(retained.join("logs")).unwrap();
    std::fs::write(
        retained.join(format!("logs/{label}.log")),
        format!(
            "$ {command:?}\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{label} failed; artifacts retained at {}\n{}{}",
        retained.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// A consumer package: an executable target that references exactly one SDK
/// surface, never executed — the gate measures its linked binary.
struct Consumer {
    label: &'static str,
    directory: &'static str,
    main: &'static str,
}

const ONE_OPERATION_CONSUMER: Consumer = Consumer {
    label: "one-operation",
    directory: "ops",
    main: r#"import EliminationFixture

@main
enum EliminationGate {
    static func main() async throws {
        let client = Client(credentials: .init(apiKey: "gate"))
        let result = try await client.getGadget(GetGadgetInput(gadgetId: "g"))
        print(result.status)
    }
}
"#,
};

const CODEC_ONLY_CONSUMER: Consumer = Consumer {
    label: "codec-only",
    directory: "codec",
    main: r#"import EliminationFixture

@main
enum EliminationGate {
    static func main() {
        let codec = Codecs.gadget
        print(codec)
    }
}
"#,
};

fn write_consumer_package(generation_root: &Path, consumer: &Consumer) -> PathBuf {
    let package = generation_root.join(format!("consumer-{}", consumer.directory));
    std::fs::create_dir_all(package.join("Sources/Consumer")).unwrap();
    std::fs::write(package.join("Package.swift"), "// swift-tools-version: 6.0\nimport PackageDescription\nlet package = Package(name: \"EliminationConsumer\", platforms: [.macOS(.v13)], dependencies: [.package(path: \"../sdk/swift\")], targets: [.executableTarget(name: \"Consumer\", dependencies: [.product(name: \"EliminationFixture\", package: \"swift\")], swiftSettings: [.swiftLanguageMode(.v6)])])\n").unwrap();
    std::fs::write(package.join("Sources/Consumer/main.swift"), consumer.main).unwrap();
    package
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

/// Marker presence over the raw linked binary. Symbol-table strings are part
/// of the artifact, so a retained symbol's mangled name counts as retention.
fn marker_map(binary: &Path) -> BTreeMap<String, bool> {
    let raw = std::fs::read(binary).unwrap();
    let mut markers = BTreeMap::new();
    markers.insert(
        MARKER_OWN_OPERATION.to_owned(),
        contains(&raw, MARKER_OWN_OPERATION),
    );
    for marker in MARKER_UNRELATED_OPERATION
        .iter()
        .chain(MARKER_OTHER_OPERATIONS)
    {
        markers.insert((*marker).to_owned(), contains(&raw, marker));
    }
    markers
}

/// Build one consumer against one SDK generation in one configuration and
/// measure the linked executable: byte size, `nm` symbol count and raw-bytes
/// marker presence.
fn build_and_measure(
    generation_root: &Path,
    consumer: &Consumer,
    configuration: &str,
    extra_linker_args: &[&str],
) -> (u64, u64, BTreeMap<String, bool>) {
    let tag = format!("{}-{}", consumer.directory, configuration);
    let package = write_consumer_package(generation_root, consumer);
    let scratch = generation_root.join(format!("build-{tag}"));
    let mut binding = swift_command("build");
    let command = binding
        .arg("-c")
        .arg(configuration)
        .arg("--package-path")
        .arg(&package)
        .arg("--scratch-path")
        .arg(&scratch);
    for argument in extra_linker_args {
        command.arg(argument);
    }
    checked(command, generation_root, &format!("{tag}-build"));
    let binary = scratch.join(format!("{configuration}/Consumer"));
    assert!(binary.is_file(), "consumer executable linked at {binary:?}");
    let bytes = binary.metadata().unwrap().len();
    let symbols = nm_symbol_count(&binary);
    (bytes, symbols, marker_map(&binary))
}

fn nm_symbol_count(binary: &Path) -> u64 {
    let output = Command::new("nm")
        .arg(binary)
        .output()
        .expect("nm is available on Darwin host toolchains");
    String::from_utf8_lossy(&output.stdout).lines().count() as u64
}

fn swift_version() -> String {
    let output = Command::new(swift())
        .arg("--version")
        .output()
        .expect("required Swift toolchain is unavailable");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn write_report(report: &Value) {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("swift-elimination-report.json"),
        serde_json::to_vec_pretty(report).unwrap(),
    )
    .unwrap();
}

#[ignore = "requires the Swift compiler on the test host"]
#[test]
fn swift_consumers_eliminate_unrelated_operations() {
    let _gate = NATIVE_GATE.lock().unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-swift-elimination");
    std::fs::create_dir_all(&root).unwrap();

    let base_root = root.join(BASE_LABEL);
    std::fs::create_dir_all(base_root.join("sdk")).unwrap();
    suspect_codegen::write_files(&generate(elimination_document()), &base_root.join("sdk"))
        .unwrap();
    assert!(base_root.join("sdk/swift/Package.swift").is_file());
    let extended_root = root.join(EXTENDED_LABEL);
    std::fs::create_dir_all(extended_root.join("sdk")).unwrap();
    suspect_codegen::write_files(
        &generate(document_with_unrelated_operation()),
        &extended_root.join("sdk"),
    )
    .unwrap();
    assert!(extended_root.join("sdk/swift/Package.swift").is_file());

    let mut measurements = BTreeMap::new();
    for consumer in [ONE_OPERATION_CONSUMER, CODEC_ONLY_CONSUMER] {
        for configuration in ["debug", "release"] {
            for (label, generation_root) in
                [(BASE_LABEL, &base_root), (EXTENDED_LABEL, &extended_root)]
            {
                let (bytes, symbols, markers) =
                    build_and_measure(generation_root, &consumer, configuration, &[]);
                measurements.insert(
                    format!("{}/{}/{label}", consumer.label, configuration),
                    json!({
                        "binary_bytes": bytes,
                        "nm_symbols": symbols,
                        "markers": markers,
                    }),
                );
                println!(
                    "{}/{configuration}/{label}: {bytes} B, {symbols} nm symbols, unrelated markers {}",
                    consumer.label,
                    MARKER_UNRELATED_OPERATION
                        .iter()
                        .map(|marker| format!("{marker}={}", markers[*marker]))
                        .collect::<Vec<_>>()
                        .join(" ")
                );
            }
        }
    }

    // Explicit dead-strip probe: recorded evidence that linker dead
    // stripping is not the differentiator — a release codec-only consumer
    // built with `-Xlinker -dead_strip` still retains every operation.
    let (dead_strip_bytes, dead_strip_symbols, dead_strip_markers) = build_and_measure(
        &base_root,
        &CODEC_ONLY_CONSUMER,
        "release",
        &["-Xlinker", "-dead_strip"],
    );
    println!(
        "release codec-only with -Xlinker -dead_strip: {dead_strip_bytes} B, {dead_strip_symbols} nm symbols, listWidgets={}",
        dead_strip_markers["listWidgets"]
    );

    // --- Pinned measured outcomes (recorded defeats) ---------------------------
    let measure = |key: &str| &measurements[key];
    for consumer in [ONE_OPERATION_CONSUMER, CODEC_ONLY_CONSUMER] {
        for configuration in ["debug", "release"] {
            let base = measure(&format!("{}/{configuration}/{BASE_LABEL}", consumer.label));
            let extended = measure(&format!(
                "{}/{configuration}/{EXTENDED_LABEL}",
                consumer.label
            ));

            // The consumer's own referenced operation is linked.
            assert!(
                base["markers"][MARKER_OWN_OPERATION].as_bool().unwrap(),
                "{}/{configuration}: the referenced operation must be linked",
                consumer.label
            );

            // Recorded defeat: every other operation, the pagination walker
            // and the OAuth grants are retained even though no consumer
            // references them.
            for marker in MARKER_OTHER_OPERATIONS {
                assert!(
                    base["markers"][*marker].as_bool().unwrap(),
                    "{}/{configuration}: measured retention of {marker} regressed — the linked artifact no longer retains unrelated operations; update docs/SDK-ELIMINATION-AUDIT.md with the improved measurement",
                    consumer.label
                );
            }

            // Recorded defeat: the unrelated operation is retained and grows
            // every artifact by the measured amount; the plan's additivity
            // property does not hold for Swift at link granularity.
            for marker in MARKER_UNRELATED_OPERATION {
                assert!(
                    extended["markers"][*marker].as_bool().unwrap(),
                    "{}/{configuration}: measured retention of {marker} regressed — the linked artifact no longer retains the unrelated operation; update docs/SDK-ELIMINATION-AUDIT.md with the improved measurement",
                    consumer.label
                );
            }
            let delta =
                extended["binary_bytes"].as_u64().unwrap() - base["binary_bytes"].as_u64().unwrap();
            assert!(
                (MEASURED_GROWTH_FLOOR_BYTES..=MEASURED_GROWTH_CEILING_BYTES).contains(&delta),
                "{}/{configuration}: the unrelated operation moved the artifact by {delta} B; update docs/SDK-ELIMINATION-AUDIT.md with the new measurement",
                consumer.label
            );
        }
    }

    write_report(&json!({
        "gate": "swift-http consumer binaries (swift build debug+release, nm symbol counts, raw-bytes markers)",
        "toolchain": swift_version(),
        "fixture": ENTRY,
        "recorded_defeat": "the Swift linker retains every selected operation in every consumer artifact (debug, release/WMO, and explicit -Xlinker -dead_strip); the unrelated operation costs the measured 64-128 KB in every artifact",
        "dead_strip_probe": {
            "command": "swift build -c release -Xlinker -dead_strip (codec-only consumer, base generation)",
            "binary_bytes": dead_strip_bytes,
            "nm_symbols": dead_strip_symbols,
            "markers": dead_strip_markers,
        },
        "measurements": measurements,
    }));
}
