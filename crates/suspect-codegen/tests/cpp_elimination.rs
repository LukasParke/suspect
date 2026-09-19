//! M2 per-language elimination gate for the C++ HTTP SDK (`cpp-http`): real
//! `cmake` + `clang++` consumer builds, mirroring the TypeScript bundler
//! gate's property — a single-operation consumer artifact must not retain
//! unrelated operations, and adding an unrelated operation to the source must
//! not grow the single-op artifact beyond a negligible delta.
//!
//! The emitted package is a CMake static library whose translation units
//! separate client, models/codecs, HTTP, pagination, OAuth and streaming.
//! Two consumer shapes are measured against both the canonical fixture and
//! the same fixture plus one unrelated operation, in two configurations:
//! - `one-operation`: an executable that only references `get_gadget` through
//!   the generated `Client`.
//! - `codec-only`: an executable that only takes the address of the Gadget
//!   codec pair (`decode_4`/`encode_4`), never touching the client.
//! - `release`: plain CMake Release.
//! - `dead-strip`: Release plus `-ffunction-sections -fdata-sections` and the
//!   Darwin linker `-dead_strip` pass — the mechanism the emitted package's
//!   audit entry claims.
//!
//! Static-library member granularity plus function-section dead stripping can
//! shed unreferenced operation code at link time; whatever the measurement
//! shows — elimination or retention — is asserted exactly and recorded, never
//! weakened (the typescript_elimination.rs convention).
//!
//! Retained artifacts live under `target/sdk-cpp-elimination/` and a
//! machine-readable record of the last run is written to
//! `target/tmp/cpp-elimination-report.json`.

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
/// length-identical across generations.
const BASE_LABEL: &str = "base";
const EXTENDED_LABEL: &str = "extd";

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

/// The canonical elimination fixture (identical to the TypeScript elimination
/// fixture): one limit/offset paginated list, one discriminated SSE stream,
/// one plain JSON read, one OAuth2 client-credentials operation and one
/// device-flow operation, under an API-key document policy.
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
            backend: Backend::CppHttp,
            package_name: "EliminationFixture".into(),
            package_version: "0.0.0".into(),
            import_name: None,
        },
        &elimination_options(),
    )
    .unwrap()
}

/// Retention markers: identifiers (which survive in mangled symbol names) and
/// wire literals that survive only inside retained code or retained data.
const MARKER_OWN_OPERATION: &str = "get_gadget";
/// The unrelated operation's code markers (mangled symbols only survive if
/// the code is linked).
const UNRELATED_CODE_MARKERS: &[&str] = &["list_gizmos", "listGizmos"];
/// The unrelated schema's codec-data markers: these ride in the shared
/// models/program data tables, not in operation code.
const UNRELATED_DATA_MARKERS: &[&str] = &["zeta-quantum", "UnrelatedGizmo"];
const MARKER_OTHER_OPERATIONS: &[&str] = &[
    "list_widgets",
    "stream_chat",
    "create_banner",
    "list_licenses",
    "text/event-stream",
];
/// Markers for code living in its own static-library members (oauth,
/// pagination): the archive linker already sheds these for any consumer that
/// does not reference them, in both configurations.
const MARKER_SEPARATE_MEMBER_MARKERS: &[&str] = &[
    "PaginationError",
    "client_credentials",
    "urn:ietf:params:oauth:grant-type:device_code",
];

fn cmake() -> PathBuf {
    std::env::var_os("SUSPECT_CPP_CMAKE")
        .map(PathBuf::from)
        .unwrap_or_else(|| "cmake".into())
}

fn cxx() -> PathBuf {
    std::env::var_os("SUSPECT_CPP_CXX")
        .map(PathBuf::from)
        .unwrap_or_else(|| "clang++".into())
}

fn checked(command: &mut Command, retained: &Path, label: &str) -> Output {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("{label}: required native tool is unavailable: {error}"));
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

/// A consumer: one executable source plus a CMake project that consumes the
/// emitted SDK as a subdirectory. Never executed — the gate measures its
/// linked binary.
struct Consumer {
    label: &'static str,
    directory: &'static str,
    main: &'static str,
}

const ONE_OPERATION_CONSUMER: Consumer = Consumer {
    label: "one-operation",
    directory: "ops",
    main: r#"#include "EliminationFixture/sdk.hpp"

int main() {
    EliminationFixture::Credentials credentials;
    credentials.api_key = "gate";
    auto connected = EliminationFixture::Client::with_curl(credentials);
    if (!connected) { return 2; }
    auto result = connected.value().get_gadget(EliminationFixture::GetGadgetInput("g"));
    return result ? 0 : 1;
}
"#,
};

const CODEC_ONLY_CONSUMER: Consumer = Consumer {
    label: "codec-only",
    directory: "codec",
    main: r#"#include "EliminationFixture/models.hpp"

int main() {
    return (&EliminationFixture::detail::decode_4 == nullptr || &EliminationFixture::detail::encode_4 == nullptr) ? 1 : 0;
}
"#,
};

fn consumer_cmake_lists() -> &'static str {
    "cmake_minimum_required(VERSION 3.24)\nproject(EliminationConsumer LANGUAGES CXX)\nset(SUSPECT_SDK_BUILD_EXAMPLES OFF CACHE BOOL \"\" FORCE)\nset(SUSPECT_SDK_BUILD_DOCS OFF)\nadd_subdirectory(../sdk/cpp EliminationFixture)\nadd_executable(Consumer main.cpp)\ntarget_compile_features(Consumer PRIVATE cxx_std_20)\nset_target_properties(Consumer PROPERTIES CXX_EXTENSIONS OFF)\ntarget_link_libraries(Consumer PRIVATE EliminationFixture::EliminationFixture)\n"
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

/// Configure and build one consumer against one SDK generation in one
/// configuration, then measure the linked executable.
fn build_and_measure(
    generation_root: &Path,
    consumer: &Consumer,
    configuration: &str,
    extra_flags: &[&str],
) -> (u64, u64, BTreeMap<String, bool>) {
    let tag = format!("{}-{}", consumer.directory, configuration);
    let package = generation_root.join(format!("consumer-{}", consumer.directory));
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("CMakeLists.txt"), consumer_cmake_lists()).unwrap();
    std::fs::write(package.join("main.cpp"), consumer.main).unwrap();
    let build = generation_root.join(format!("build-{tag}"));
    let mut configure = Command::new(cmake());
    configure
        .arg("-S")
        .arg(&package)
        .arg("-B")
        .arg(&build)
        .arg(format!("-DCMAKE_CXX_COMPILER={}", cxx().display()))
        .arg("-DCMAKE_BUILD_TYPE=Release");
    for flag in extra_flags {
        configure.arg(flag);
    }
    checked(&mut configure, generation_root, &format!("{tag}-configure"));
    checked(
        Command::new(cmake())
            .arg("--build")
            .arg(&build)
            .args(["--parallel", "4"]),
        generation_root,
        &format!("{tag}-build"),
    );
    let binary = build.join("Consumer");
    assert!(binary.is_file(), "consumer executable linked at {binary:?}");
    let bytes = binary.metadata().unwrap().len();
    let symbols = nm_symbol_count(&binary);
    let raw = std::fs::read(&binary).unwrap();
    let mut markers = BTreeMap::new();
    markers.insert(
        MARKER_OWN_OPERATION.to_owned(),
        contains(&raw, MARKER_OWN_OPERATION),
    );
    for marker in UNRELATED_CODE_MARKERS
        .iter()
        .chain(UNRELATED_DATA_MARKERS)
        .chain(MARKER_OTHER_OPERATIONS)
        .chain(MARKER_SEPARATE_MEMBER_MARKERS)
    {
        markers.insert((*marker).to_owned(), contains(&raw, marker));
    }
    (bytes, symbols, markers)
}

fn nm_symbol_count(binary: &Path) -> u64 {
    let output = Command::new("nm")
        .arg(binary)
        .output()
        .expect("nm is available on Darwin host toolchains");
    String::from_utf8_lossy(&output.stdout).lines().count() as u64
}

fn cmake_version() -> String {
    let output = Command::new(cmake())
        .arg("--version")
        .output()
        .expect("required cmake is unavailable");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn cxx_version() -> String {
    let output = Command::new(cxx())
        .arg("--version")
        .output()
        .expect("required clang++ is unavailable");
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
        directory.join("cpp-elimination-report.json"),
        serde_json::to_vec_pretty(report).unwrap(),
    )
    .unwrap();
}

#[ignore = "requires cmake and a C++ toolchain on the test host"]
#[test]
fn cpp_consumers_eliminate_unrelated_operations() {
    let _gate = NATIVE_GATE.lock().unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-cpp-elimination");
    std::fs::create_dir_all(&root).unwrap();

    let mut generations = BTreeMap::new();
    for (label, document) in [
        (BASE_LABEL, elimination_document()),
        (EXTENDED_LABEL, document_with_unrelated_operation()),
    ] {
        let generation = root.join(label);
        std::fs::create_dir_all(&generation).unwrap();
        suspect_codegen::write_files(&generate(document), &generation.join("sdk")).unwrap();
        assert!(generation.join("sdk/cpp/CMakeLists.txt").is_file());
        generations.insert(label, generation);
    }

    const PLAIN_FLAGS: &[&str] = &[];
    const DEAD_STRIP_FLAGS: &[&str] = &[
        "-DCMAKE_CXX_FLAGS=-ffunction-sections -fdata-sections",
        "-DCMAKE_EXE_LINKER_FLAGS=-Wl,-dead_strip",
    ];

    let mut measurements = BTreeMap::new();
    for consumer in [ONE_OPERATION_CONSUMER, CODEC_ONLY_CONSUMER] {
        for (configuration, flags) in [("release", PLAIN_FLAGS), ("dead-strip", DEAD_STRIP_FLAGS)] {
            for (label, generation) in &generations {
                let (bytes, symbols, markers) =
                    build_and_measure(generation, &consumer, configuration, flags);
                measurements.insert(
                    format!("{}/{configuration}/{label}", consumer.label),
                    json!({
                        "binary_bytes": bytes,
                        "nm_symbols": symbols,
                        "markers": markers,
                    }),
                );
                println!(
                    "{}/{configuration}/{label}: {bytes} B, {symbols} nm symbols, unrelated markers {}",
                    consumer.label,
                    UNRELATED_CODE_MARKERS
                        .iter()
                        .chain(UNRELATED_DATA_MARKERS)
                        .map(|marker| format!("{marker}={}", markers[*marker]))
                        .collect::<Vec<_>>()
                        .join(" ")
                );
            }
        }
    }

    // Pinned measured outcomes, recorded per configuration.
    for consumer in [ONE_OPERATION_CONSUMER, CODEC_ONLY_CONSUMER] {
        for configuration in ["release", "dead-strip"] {
            let base =
                measurements[&format!("{}/{configuration}/{BASE_LABEL}", consumer.label)].clone();
            let extended = measurements
                [&format!("{}/{configuration}/{EXTENDED_LABEL}", consumer.label)]
                .clone();

            let delta =
                extended["binary_bytes"].as_u64().unwrap() - base["binary_bytes"].as_u64().unwrap();

            if consumer.label == "codec-only" {
                // Measured elimination: the codec-only consumer is
                // byte-identical across generations and carries no unrelated
                // marker in either generation, in both configurations.
                assert_eq!(
                    delta, 0,
                    "{}/{configuration}: the codec-only consumer must be byte-identical across generations",
                    consumer.label
                );
                for marker in UNRELATED_CODE_MARKERS.iter().chain(UNRELATED_DATA_MARKERS) {
                    assert!(!base["markers"][*marker].as_bool().unwrap());
                    assert!(
                        !extended["markers"][*marker].as_bool().unwrap(),
                        "{}/{configuration}: codec-only consumer must not retain unrelated {marker}",
                        consumer.label
                    );
                }
            } else {
                let delta = extended["binary_bytes"].as_u64().unwrap()
                    - base["binary_bytes"].as_u64().unwrap();
                // The referenced operation's code is linked.
                assert!(
                    base["markers"][MARKER_OWN_OPERATION].as_bool().unwrap(),
                    "{configuration}: the referenced operation's symbols must survive in the one-operation consumer"
                );
                if configuration == "dead-strip" {
                    // Measured code elimination: with function sections and
                    // linker dead stripping, every unreferenced operation's code
                    // sheds — including the unrelated operation's own symbols.
                    for marker in MARKER_OTHER_OPERATIONS
                        .iter()
                        .chain(MARKER_SEPARATE_MEMBER_MARKERS)
                        .chain(UNRELATED_CODE_MARKERS)
                    {
                        assert!(
                            !extended["markers"][*marker].as_bool().unwrap(),
                            "{configuration}: measured code elimination regressed — {marker} is retained in the one-operation consumer; update docs/SDK-ELIMINATION-AUDIT.md"
                        );
                    }
                    // Recorded data-granular defeat: the unrelated schema's codec
                    // data rides in the shared models/program data tables.
                    for marker in UNRELATED_DATA_MARKERS {
                        assert!(
                            extended["markers"][*marker].as_bool().unwrap(),
                            "{configuration}: recorded data-granular retention regressed — {marker} no longer ships with the one-operation consumer; update docs/SDK-ELIMINATION-AUDIT.md with the improved measurement"
                        );
                    }
                    assert!(
                        (8_000..=32_000).contains(&delta),
                        "{configuration}: the unrelated schema's codec data moved the artifact by {delta} B; update docs/SDK-ELIMINATION-AUDIT.md with the new measurement"
                    );
                } else {
                    // Recorded retention defeat: without function sections the
                    // whole client member (every operation method) is retained.
                    // Static-library member granularity still sheds the oauth,
                    // stream and pagination members in both configurations.
                    for marker in MARKER_SEPARATE_MEMBER_MARKERS {
                        assert!(
                            !base["markers"][*marker].as_bool().unwrap(),
                            "{configuration}: measured member elimination regressed — {marker} is retained in the one-operation consumer; update docs/SDK-ELIMINATION-AUDIT.md"
                        );
                    }
                    for marker in MARKER_OTHER_OPERATIONS {
                        assert!(
                            base["markers"][*marker].as_bool().unwrap(),
                            "{configuration}: measured retention of {marker} regressed; update docs/SDK-ELIMINATION-AUDIT.md with the improved measurement"
                        );
                    }
                    for marker in UNRELATED_CODE_MARKERS.iter().chain(UNRELATED_DATA_MARKERS) {
                        assert!(
                            extended["markers"][*marker].as_bool().unwrap(),
                            "{configuration}: measured retention of {marker} regressed; update docs/SDK-ELIMINATION-AUDIT.md with the improved measurement"
                        );
                    }
                    assert!(
                        (16_000..=48_000).contains(&delta),
                        "{configuration}: the unrelated operation moved the artifact by {delta} B; update docs/SDK-ELIMINATION-AUDIT.md with the new measurement"
                    );
                }

                measurements.insert(
                    format!("{}/{configuration}/retained", consumer.label),
                    json!({
                        "binary_delta_bytes": delta,
                        "other_operations": MARKER_OTHER_OPERATIONS
                            .iter()
                            .filter(|marker| base["markers"][**marker].as_bool().unwrap())
                            .collect::<Vec<_>>(),
                        "unrelated_code": UNRELATED_CODE_MARKERS
                            .iter()
                            .filter(|marker| extended["markers"][**marker].as_bool().unwrap())
                            .collect::<Vec<_>>(),
                        "unrelated_data": UNRELATED_DATA_MARKERS
                            .iter()
                            .filter(|marker| extended["markers"][**marker].as_bool().unwrap())
                            .collect::<Vec<_>>(),
                    }),
                );
            }
        }
    }

    write_report(&json!({
        "gate": "cpp-http consumer binaries (cmake + clang++ Release, function sections + linker dead stripping)",
        "toolchain": format!("{} / {}", cmake_version(), cxx_version()),
        "fixture": ENTRY,
        "measurements": measurements,
    }));
}
