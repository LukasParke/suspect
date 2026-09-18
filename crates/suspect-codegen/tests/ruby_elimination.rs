//! M2 per-language elimination gate for the Ruby HTTP SDK (`ruby-http`):
//! real Ruby 3.3 `$LOADED_FEATURES` require-graph measurements, mirroring the
//! TypeScript bundler gate's property — a single-operation consumer artifact
//! must not retain unrelated operations.
//!
//! Ruby is interpreted: no linker exists, so elimination is decided at
//! emission (which files exist) and at load time (which files a consumer's
//! `require` chain reaches). Two consumer shapes are measured against both
//! the canonical fixture and the same fixture plus one unrelated operation:
//! - **Full-gem consumer:** `require "elimination_fixture"` plus
//!   `Client.new`. The gem entry requires a fixed module list ending in
//!   `client.rb`, so this consumer measures the artifact's actual load
//!   granularity.
//! - **Codec-only consumer:** requires only `json`, `models` and `codecs`,
//!   then decodes a `Gadget` through `Codecs::Gadget` (functional round
//!   trip). Its loaded set must exclude every operation/OAuth/stream/HTTP
//!   module — the measured elimination at load granularity.
//!
//! Recorded findings (asserted here per the typescript_elimination.rs
//! convention that a divergence is recorded rather than a marker weakened):
//! - The emitted gem has no per-operation require targets: all operations
//!   live in one `client.rb`, so any client-touching consumer loads every
//!   operation's code (composition-shape finding).
//! - All schemas' shape/codec data lives in one `models.rb`/`codecs.rb`
//!   table pair, so the codec-only consumer's loaded set grows by design
//!   when the unrelated schema is added (file-granular retention, the Ruby
//!   analogue of the TypeScript descriptor-map defeats).
//!
//! Retained artifacts live under `target/sdk-ruby-elimination/` and a
//! machine-readable record of the last run is written to
//! `target/tmp/ruby-elimination-report.json`.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
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

const GEM_NAME: &str = "elimination-fixture";
const REQUIRE_NAME: &str = "elimination_fixture";

/// Client/OAuth/stream modules a codec-only consumer must not load.
const ABSENT_CLIENT_MODULES: &[&str] = &[
    "client.rb",
    "oauth.rb",
    "streams.rb",
    "http.rb",
    "payload.rb",
    "wire.rb",
];

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
            backend: Backend::RubyHttp,
            package_name: GEM_NAME.into(),
            package_version: "0.0.0".into(),
            import_name: Some("EliminationFixture".into()),
        },
        &elimination_options(),
    )
    .unwrap()
}

fn ruby_home() -> PathBuf {
    std::env::var_os("SUSPECT_RUBY_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new("/Users/luke").join(".local/share/mise/installs/ruby/3.3.12"))
}

fn ruby_command() -> Command {
    let mut command = Command::new(ruby_home().join("bin/ruby"));
    command
        .env_clear()
        .env("PATH", ruby_home().join("bin"))
        .env("HOME", std::env::var_os("HOME").unwrap_or_default());
    command
}

fn checked(command: &mut Command, retained: &Path, label: &str) -> String {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("{label}: required Ruby toolchain is unavailable: {error}"));
    std::fs::create_dir_all(retained.join("logs")).unwrap();
    let text = format!(
        "$ {command:?}\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::write(retained.join(format!("logs/{label}.log")), &text).unwrap();
    assert!(
        output.status.success(),
        "{label} failed; artifacts retained at {}\n{text}",
        retained.display()
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// The measurement harness: requires the requested modules, performs the
/// consumer action, then prints the gem-local loaded-feature list.
fn loaded_features_script(requires: &[&str], action: &str) -> String {
    let mut script = String::new();
    for requirement in requires {
        script.push_str(&format!("require \"{requirement}\"\n"));
    }
    script.push_str(action);
    script.push_str("\n$LOADED_FEATURES.each do |feature|\n");
    script.push_str("  puts feature if feature.include?(\"");
    script.push_str(REQUIRE_NAME);
    script.push_str("\")\nend\n");
    script
}

/// Run one consumer shape against one generation; returns the gem-local
/// loaded-feature paths.
fn run_consumer(generation: &Path, requires: &[&str], action: &str, label: &str) -> Vec<PathBuf> {
    let lib = generation.join("ruby/lib");
    let script = loaded_features_script(requires, action);
    let output = checked(
        ruby_command().arg("-I").arg(&lib).arg("-e").arg(&script),
        generation,
        label,
    );
    output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(PathBuf::from)
        .collect()
}

fn marker_content(features: &[PathBuf], marker: &str) -> bool {
    features
        .iter()
        .any(|feature| std::fs::read_to_string(feature).is_ok_and(|text| text.contains(marker)))
}

fn loaded_bytes(features: &[PathBuf]) -> u64 {
    features
        .iter()
        .filter_map(|feature| std::fs::metadata(feature).ok())
        .map(|metadata| metadata.len())
        .sum()
}

fn file_name(feature: &Path) -> &str {
    feature
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
}

fn write_report(report: &Value) {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("ruby-elimination-report.json"),
        serde_json::to_vec_pretty(report).unwrap(),
    )
    .unwrap();
}

#[test]
fn ruby_consumers_eliminate_unrelated_operations() {
    let _gate = NATIVE_GATE.lock().unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-ruby-elimination");
    std::fs::create_dir_all(&root).unwrap();

    let mut generations = BTreeMap::new();
    for (label, document) in [
        (BASE_LABEL, elimination_document()),
        (EXTENDED_LABEL, document_with_unrelated_operation()),
    ] {
        let generation = root.join(label);
        std::fs::create_dir_all(&generation).unwrap();
        suspect_codegen::write_files(&generate(document), &generation).unwrap();
        assert!(
            generation
                .join("ruby/lib")
                .join(REQUIRE_NAME)
                .join("client.rb")
                .is_file()
        );
        generations.insert(label, generation);
    }

    // Gemspec dependency gate: the documented gem dependency set only (all
    // Ruby standard-library gems; nothing transitive beyond them).
    let gemspec = std::fs::read_to_string(
        generations[BASE_LABEL]
            .join("ruby")
            .join(format!("{GEM_NAME}.gemspec")),
    )
    .unwrap();
    let declared: Vec<&str> = gemspec
        .lines()
        .filter_map(|line| line.trim().strip_prefix("s.add_dependency "))
        .filter_map(|line| line.split('\'').nth(1))
        .collect();
    assert_eq!(
        declared,
        vec![
            "net-http",
            "uri",
            "openssl",
            "timeout",
            "base64",
            "securerandom"
        ],
        "ruby-http gemspec must declare exactly the documented dependency set; found {declared:?}"
    );

    let mut measurements = BTreeMap::new();

    // --- Full-gem consumer -------------------------------------------------
    for (label, generation) in &generations {
        let features = run_consumer(
            generation,
            &[REQUIRE_NAME],
            "EliminationFixture::Client.new",
            &format!("full-gem-{label}"),
        );
        let names: Vec<&str> = features.iter().map(|feature| file_name(feature)).collect();
        // Composition shape: the client module carries every operation, so
        // the full-gem consumer loads it (and its fixed require list).
        assert!(
            names.contains(&"client.rb"),
            "full-gem consumer must load client.rb (composition shape)"
        );
        let bytes = loaded_bytes(&features);
        measurements.insert(
            format!("full-gem/{label}"),
            json!({
                "loaded_features": names,
                "loaded_bytes": bytes,
                "unrelated_markers_loaded": {
                    "listGizmos": marker_content(&features, "listGizmos"),
                    "zeta-quantum": marker_content(&features, "zeta-quantum"),
                },
            }),
        );
        println!(
            "ruby full-gem/{label}: {} modules, {bytes} B loaded",
            features.len()
        );
    }
    // Recorded defeat: the unrelated operation rides in client.rb/models.rb,
    // so the extended full-gem consumer loads its markers.
    let extended_markers = &measurements["full-gem/extd"]["unrelated_markers_loaded"];
    assert!(
        extended_markers["listGizmos"].as_bool().unwrap()
            && extended_markers["zeta-quantum"].as_bool().unwrap(),
        "documented composition retention regressed: the unrelated operation no longer loads with the full gem; update docs/SDK-ELIMINATION-AUDIT.md"
    );

    // --- Codec-only consumer ------------------------------------------------
    for (label, generation) in &generations {
        let features = run_consumer(
            generation,
            &[
                "elimination_fixture/policy",
                "elimination_fixture/json",
                "elimination_fixture/program_guard",
                "elimination_fixture/resource_guard",
                "elimination_fixture/validation",
                "elimination_fixture/validation_v2",
                "elimination_fixture/validation_v3",
                "elimination_fixture/program",
                "elimination_fixture/codecs",
                "elimination_fixture/models",
            ],
            "gadget = EliminationFixture::Codecs::Gadget.decode_json('{\"kind\":\"standard\",\"label\":\"x\"}')\nraise 'wrong decode' unless gadget.label == 'x'\nputs gadget.class",
            &format!("codec-only-{label}"),
        );
        let names: Vec<&str> = features.iter().map(|feature| file_name(feature)).collect();
        // Measured elimination: no client/OAuth/stream/HTTP module loads.
        for module in ABSENT_CLIENT_MODULES {
            assert!(
                !names.contains(module),
                "codec-only consumer must not load {module}"
            );
        }
        let bytes = loaded_bytes(&features);
        measurements.insert(
            format!("codec-only/{label}"),
            json!({
                "loaded_features": names,
                "loaded_bytes": bytes,
                "unrelated_markers_loaded": {
                    "listGizmos": marker_content(&features, "listGizmos"),
                    "zeta-quantum": marker_content(&features, "zeta-quantum"),
                },
            }),
        );
        println!(
            "ruby codec-only/{label}: {} modules, {bytes} B loaded",
            features.len()
        );
    }
    // Recorded defeat: models.rb groups every schema's shape data, so the
    // extended codec-only consumer's loaded set carries the unrelated
    // schema's markers.
    let codec_base = &measurements["codec-only/base"];
    let codec_extended = &measurements["codec-only/extd"];
    assert!(
        !codec_base["unrelated_markers_loaded"]["zeta-quantum"]
            .as_bool()
            .unwrap(),
        "base codec-only consumer must not load any unrelated-operation marker"
    );
    assert!(
        codec_extended["unrelated_markers_loaded"]["zeta-quantum"]
            .as_bool()
            .unwrap(),
        "documented file-granular retention regressed: the unrelated schema no longer rides in models.rb; update docs/SDK-ELIMINATION-AUDIT.md"
    );
    // The codec consumer works against the models/codecs table pair, which
    // grows by the unrelated schema's shape+codec data (measured, recorded).
    let codec_delta = codec_extended["loaded_bytes"].as_u64().unwrap()
        - codec_base["loaded_bytes"].as_u64().unwrap();
    assert!(
        codec_delta > 0,
        "codec-only loaded bytes must reflect the models.rb shape-table growth (recorded defeat)"
    );

    write_report(&json!({
        "gate": "ruby-http require-graph reachability ($LOADED_FEATURES over Ruby 3.3)",
        "toolchain": format!("ruby {}", ruby_version()),
        "fixture": ENTRY,
        "recorded_defeats": [
            "no per-operation require targets exist: all operations live in one client.rb, so client-touching consumers load every operation",
            "models.rb/codecs.rb group every schema's shape+codec data, so the codec-only consumer's loaded set grows when an unrelated schema is added",
        ],
        "measurements": measurements,
    }));
}

fn ruby_version() -> String {
    let output = Command::new(ruby_home().join("bin/ruby"))
        .arg("-e")
        .arg("puts RUBY_VERSION")
        .output()
        .expect("required Ruby toolchain is unavailable");
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}
