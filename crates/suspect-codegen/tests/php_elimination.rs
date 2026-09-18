//! M2 per-language elimination gate for the PHP HTTP SDK (`php-http`): real
//! Composer autoload measurements with the repo-pinned PHP 8.3 + Composer
//! toolchains, mirroring the TypeScript bundler gate's property — a
//! single-operation consumer artifact must not retain unrelated operations.
//!
//! PHP is interpreted: no linker exists, so elimination is decided at
//! emission (which classes exist) and at load time (which files Composer's
//! classmap autoload actually loads). Three gates run here:
//! - **Classmap gate:** `composer dump-autoload` must map exactly the
//!   emitted `src/` classes; the classmap is the package's whole load graph
//!   (Composer never loads a class that is not referenced).
//! - **Codec consumer gate (functional):** requires `vendor/autoload.php`
//!   and decodes a `Gadget` through `EliminationSdk\Gadget::fromJson`. Its
//!   `get_included_files()` must exclude every client/OAuth/stream/transport
//!   file, and `Client` must remain unloaded — the measured elimination at
//!   autoload granularity.
//! - **Client consumer gate:** touching `EliminationSdk\Client` loads the one
//!   client class with every operation's method (composition-shape finding).
//!
//! Recorded finding (asserted here per the typescript_elimination.rs
//! convention): all schemas' codec data lives in one `Codecs.php` table, so
//! the codec consumer's loaded set carries the unrelated schema's markers
//! when it is added (file-granular retention, the PHP analogue of the
//! TypeScript descriptor-map defeats).
//!
//! Retained artifacts live under `target/sdk-php-elimination/` and a
//! machine-readable record of the last run is written to
//! `target/tmp/php-elimination-report.json`.

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

const PACKAGE_NAME: &str = "elimination/fixture-sdk";
const NAMESPACE_NAME: &str = "EliminationSdk";

/// Client/OAuth/stream/transport files a codec-only consumer must not load.
const ABSENT_CLIENT_FILES: &[&str] = &[
    "Client.php",
    "OAuth.php",
    "Stream.php",
    "Transport.php",
    "Http.php",
    "Parts.php",
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
            backend: Backend::PhpHttp,
            package_name: PACKAGE_NAME.into(),
            package_version: "0.0.0".into(),
            import_name: Some(NAMESPACE_NAME.into()),
        },
        &elimination_options(),
    )
    .unwrap()
}

fn tools() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-php-tools")
}

fn php() -> PathBuf {
    std::env::var_os("SUSPECT_PHP_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| tools().join("php-8.3.32/php"))
}

fn composer() -> PathBuf {
    std::env::var_os("SUSPECT_COMPOSER_PHAR")
        .map(PathBuf::from)
        .unwrap_or_else(|| tools().join("composer-2.10.3.phar"))
}

fn checked(command: &mut Command, retained: &Path, label: &str) -> String {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("{label}: required PHP toolchain is unavailable: {error}"));
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

/// Generate the Composer autoload (classmap) for one emitted package.
fn dump_autoload(generation: &Path) {
    let home = generation.join("composer-home");
    std::fs::create_dir_all(&home).unwrap();
    let mut command = Command::new(php());
    command
        .arg(composer())
        .args(["dump-autoload", "--no-interaction", "--quiet"])
        .env("COMPOSER_HOME", &home)
        .current_dir(generation.join("php"));
    checked(&mut command, generation, "composer-dump-autoload");
    assert!(
        generation.join("php/vendor/autoload.php").is_file(),
        "composer autoload generated in {}",
        generation.display()
    );
}

/// FQCNs mapped by the generated Composer classmap.
fn classmap_classes(generation: &Path) -> Vec<String> {
    let text =
        std::fs::read_to_string(generation.join("php/vendor/composer/autoload_classmap.php"))
            .unwrap();
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let rest = trimmed.strip_prefix('\'')?;
            let end = rest.find('\'')?;
            let class = &rest[..end];
            trimmed
                .contains(" => ")
                .then(|| class.replace("\\\\", "\\"))
        })
        .collect()
}

/// Run one PHP consumer program and return the package-local included files.
fn included_files(generation: &Path, program: &str, label: &str) -> Vec<PathBuf> {
    let script = format!(
        "require 'vendor/autoload.php';\n{program}\nforeach (get_included_files() as $file) {{ if (str_contains($file, 'php/src')) echo $file, PHP_EOL; }}"
    );
    let output = checked(
        Command::new(php())
            .arg("-r")
            .arg(&script)
            .current_dir(generation.join("php")),
        generation,
        label,
    );
    output
        .lines()
        .filter(|line| line.ends_with(".php"))
        .map(PathBuf::from)
        .collect()
}

fn marker_content(files: &[PathBuf], marker: &str) -> bool {
    files
        .iter()
        .any(|file| std::fs::read_to_string(file).is_ok_and(|text| text.contains(marker)))
}

fn write_report(report: &Value) {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("php-elimination-report.json"),
        serde_json::to_vec_pretty(report).unwrap(),
    )
    .unwrap();
}

#[test]
fn php_consumers_eliminate_unrelated_operations() {
    let _gate = NATIVE_GATE.lock().unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-php-elimination");
    std::fs::create_dir_all(&root).unwrap();

    let mut generations = BTreeMap::new();
    for (label, document) in [
        (BASE_LABEL, elimination_document()),
        (EXTENDED_LABEL, document_with_unrelated_operation()),
    ] {
        let generation = root.join(label);
        std::fs::create_dir_all(&generation).unwrap();
        suspect_codegen::write_files(&generate(document), &generation).unwrap();
        assert!(generation.join("php/src/Client.php").is_file());
        dump_autoload(&generation);
        generations.insert(label, generation);
    }

    let mut measurements = BTreeMap::new();

    // --- Classmap gate -------------------------------------------------------
    for (label, generation) in &generations {
        let classes = classmap_classes(generation);
        assert!(
            classes
                .iter()
                .any(|class| class == &format!("{NAMESPACE_NAME}\\Client")),
            "the composer classmap must map the package's one client class"
        );
        let has_unrelated = classes.iter().any(|class| class.contains("UnrelatedGizmo"));
        if *label == EXTENDED_LABEL {
            assert!(
                has_unrelated,
                "documented classmap composition regressed: the unrelated model class no longer ships in the classmap"
            );
        } else {
            assert!(!has_unrelated);
        }
        measurements.insert(
            format!("classmap/{label}"),
            json!({
                "classes": classes.len(),
                "unrelated_model_class_mapped": has_unrelated,
            }),
        );
        println!("php classmap/{label}: {} classes mapped", classes.len());
    }

    // --- Codec consumer (functional, autoload-level) --------------------------
    let codec_program = r#"$gadget = EliminationSdk\Gadget::fromJson('{"kind":"standard","label":"x"}');
if ($gadget->label !== 'x') { throw new RuntimeException('wrong decode'); }
echo get_class($gadget), PHP_EOL;
if (class_exists('EliminationSdk\\Client', false)) { throw new RuntimeException('Client autoloaded by a codec-only consumer'); }"#;
    for (label, generation) in &generations {
        let files = included_files(generation, codec_program, &format!("codec-only-{label}"));
        let names: Vec<&str> = files
            .iter()
            .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
            .collect();
        // Measured elimination: no client/OAuth/stream/transport file loads.
        for file in ABSENT_CLIENT_FILES {
            assert!(
                !names.contains(file),
                "codec-only consumer must not load {file}"
            );
        }
        let bytes = files
            .iter()
            .filter_map(|file| std::fs::metadata(file).ok())
            .map(|metadata| metadata.len())
            .sum::<u64>();
        measurements.insert(
            format!("codec-only/{label}"),
            json!({
                "included_files": names,
                "included_bytes": bytes,
                "unrelated_markers_loaded": {
                    "listGizmos": marker_content(&files, "listGizmos"),
                    "zeta-quantum": marker_content(&files, "zeta-quantum"),
                },
            }),
        );
        println!(
            "php codec-only/{label}: {} files, {bytes} B loaded",
            files.len()
        );
    }
    // Recorded defeat: Codecs.php groups every schema's codec data, so the
    // extended codec consumer's loaded set carries the unrelated schema's
    // markers (the PHP analogue of the TypeScript descriptor-map defeats).
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
        "documented file-granular retention regressed: the unrelated schema no longer rides in Codecs.php; update docs/SDK-ELIMINATION-AUDIT.md"
    );

    // --- Client consumer (composition shape) ---------------------------------
    let client_program = "if (!class_exists('EliminationSdk\\Client')) { throw new RuntimeException('Client missing'); }";
    for (label, generation) in &generations {
        let files = included_files(generation, client_program, &format!("client-{label}"));
        let names: Vec<&str> = files
            .iter()
            .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
            .collect();
        assert!(
            names.contains(&"Client.php"),
            "client-touching consumer loads Client.php (composition shape)"
        );
        let bytes = files
            .iter()
            .filter_map(|file| std::fs::metadata(file).ok())
            .map(|metadata| metadata.len())
            .sum::<u64>();
        measurements.insert(
            format!("client/{label}"),
            json!({
                "included_files": names,
                "included_bytes": bytes,
                "unrelated_markers_loaded": {
                    "listGizmos": marker_content(&files, "listGizmos"),
                    "zeta-quantum": marker_content(&files, "zeta-quantum"),
                },
            }),
        );
        println!(
            "php client/{label}: {} files, {bytes} B loaded",
            files.len()
        );
    }
    let client_markers = &measurements["client/extd"]["unrelated_markers_loaded"];
    assert!(
        client_markers["listGizmos"].as_bool().unwrap(),
        "documented composition retention regressed: the unrelated operation's methods no longer load with the client class"
    );

    write_report(&json!({
        "gate": "php-http classmap + autoload reachability (get_included_files over PHP 8.3)",
        "toolchain": format!("{} + composer phar {}", php().display(), composer().display()),
        "fixture": ENTRY,
        "recorded_defeats": [
            "the package's operations live in one Client.php class, so any client-touching consumer loads every operation",
            "Codecs.php groups every schema's codec data, so the codec consumer's loaded set grows when an unrelated schema is added",
        ],
        "measurements": measurements,
    }));
}
