//! M2 per-language elimination gate for the Java HTTP SDK (`java-http`):
//! real `mvn install` SDK builds, a `javac`-compiled single-operation
//! consumer, and `javap` constant-pool analysis, mirroring the TypeScript
//! bundler gate's property — a single-operation consumer's compiled unit must
//! not retain unrelated operations, and adding an unrelated operation to the
//! source must not change the consumer artifact.
//!
//! Two gates exist by design and both run here:
//! - **Consumer unit gate:** the one-operation consumer (it only calls
//!   `Client.getGadget`) is compiled against the SDK jar and inspected with
//!   `javap -v -p`. Its constant pool must reference exactly one operation's
//!   methods and none of the unrelated operation's types. The artifact-level
//!   composition shape is then documented from the jar itself: the emitted
//!   package is one artifact whose `Client` carries every operation, so the
//!   unrelated operation's classes remain in the jar (the plan's M0
//!   root-composition caveat, measured instead of assumed).
//! - **Dependency gate:** the emitted `pom.xml` declares zero dependencies
//!   beyond the JDK, which is the hard elimination guarantee — no transitive
//!   retention is possible at all.
//!
//! Additivity: the consumer's `.class` bytes must be identical whether the
//! SDK source contains five or six operations.
//!
//! Retained artifacts live under `target/sdk-java-elimination/` and a
//! machine-readable record of the last run is written to
//! `target/tmp/java-elimination-report.json`.

use std::{
    fs,
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

const GROUP_ARTIFACT: &str = "test.elimination:elimination-sdk";
const JAVA_PACKAGE: &str = "test.elimination.generated";
const VERSION: &str = "0.0.0";

/// Methods of the four operations the one-operation consumer never calls.
const ABSENT_METHOD_MARKERS: &[&str] =
    &["listWidgets", "streamChat", "createBanner", "listLicenses"];
/// The unrelated operation's type and wire-literal markers.
const UNRELATED_TYPE_MARKER: &str = "ListGizmos";
const MARKER_OWN_METHOD: &str = "getGadget";

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

use std::collections::BTreeMap;

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
            backend: Backend::JavaHttp,
            package_name: GROUP_ARTIFACT.into(),
            package_version: VERSION.into(),
            import_name: Some(JAVA_PACKAGE.into()),
        },
        &elimination_options(),
    )
    .unwrap()
}

fn java_home() -> PathBuf {
    std::env::var_os("SUSPECT_JAVA_HOME")
        .or_else(|| std::env::var_os("JAVA_HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            "/Users/luke/.local/share/mise/installs/java/temurin-21.0.12+101.0.LTS".into()
        })
}

fn maven() -> PathBuf {
    std::env::var_os("SUSPECT_MAVEN_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            "/Users/luke/.local/share/mise/installs/maven/3.9.16/apache-maven-3.9.16/bin/mvn".into()
        })
}

fn maven_repository() -> PathBuf {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/sdk-java-maven-cache/java/repository");
    fs::create_dir_all(&repository).unwrap();
    repository
}

fn checked(command: &mut Command, retained: &Path, label: &str) -> String {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("{label}: required native tool is unavailable: {error}"));
    fs::create_dir_all(retained.join("logs")).unwrap();
    let text = format!(
        "$ {command:?}\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    fs::write(retained.join(format!("logs/{label}.log")), &text).unwrap();
    assert!(
        output.status.success(),
        "{label} failed; artifacts retained at {}\n{text}",
        retained.display()
    );
    text
}

/// Build the emitted Maven package and return the installed jar path.
fn install_package(root: &Path) -> PathBuf {
    let pom = root.join("java/pom.xml");
    assert!(pom.is_file(), "emitted pom at {}", pom.display());
    let mut command = Command::new(maven());
    command
        .args(["-B", "-q", "install"])
        .arg(format!(
            "-Dmaven.repo.local={}",
            maven_repository().display()
        ))
        .env("JAVA_HOME", java_home())
        .current_dir(root.join("java"));
    checked(&mut command, root, "maven-install");
    let jar = root.join(format!("java/target/elimination-sdk-{VERSION}.jar"));
    assert!(jar.is_file(), "installed jar at {}", jar.display());
    jar
}

/// The single-operation consumer: it constructs the client and references
/// exactly one operation. It is compiled, never executed.
const ONE_OPERATION_CONSUMER: &str = r#"import test.elimination.generated.Client;
import test.elimination.generated.HttpRuntime;

public final class OneOperation {
    public static void main(String[] args) {
        Client client = new Client(HttpRuntime.Options.builder().build());
        Client.GetGadgetStatus200 status =
            client.getGadget(Client.GetGadgetInput.builder("g").build());
        System.out.println(status == null);
    }
}
"#;

/// Compile the consumer against the installed jar; returns its `.class` bytes.
fn compile_consumer(jar: &Path, root: &Path) -> Vec<u8> {
    let consumer = root.join("consumer");
    fs::create_dir_all(&consumer).unwrap();
    fs::write(consumer.join("OneOperation.java"), ONE_OPERATION_CONSUMER).unwrap();
    let classes = consumer.join("classes");
    fs::create_dir_all(&classes).unwrap();
    let mut command = Command::new(java_home().join("bin/javac"));
    command
        .arg("-cp")
        .arg(jar)
        .arg("-d")
        .arg(&classes)
        .arg(consumer.join("OneOperation.java"))
        .env("JAVA_HOME", java_home());
    checked(&mut command, root, "javac-consumer");
    fs::read(classes.join("OneOperation.class")).unwrap()
}

/// `javap -v -p` constant-pool text of a class inside the consumer build.
fn javap_constant_pool(class: &Path, root: &Path) -> String {
    let mut command = Command::new(java_home().join("bin/javap"));
    command
        .args(["-v", "-p"])
        .arg(class)
        .env("JAVA_HOME", java_home());
    checked(&mut command, root, "javap-consumer")
}

/// `jar tf` entries of the SDK artifact.
fn jar_entries(jar: &Path, root: &Path) -> Vec<String> {
    let mut command = Command::new(java_home().join("bin/jar"));
    command.arg("tf").arg(jar).env("JAVA_HOME", java_home());
    let text = checked(&mut command, root, "jar-list");
    text.lines()
        .filter(|line| line.ends_with(".class"))
        .map(str::to_owned)
        .collect()
}

fn write_report(report: &Value) {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("java-elimination-report.json"),
        serde_json::to_vec_pretty(report).unwrap(),
    )
    .unwrap();
}

#[ignore = "requires JDK and Maven on the test host"]
#[test]
fn java_consumers_eliminate_unrelated_operations() {
    let _gate = NATIVE_GATE.lock().unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-java-elimination");
    fs::create_dir_all(&root).unwrap();

    let mut measurements = BTreeMap::new();
    let mut consumer_classes = BTreeMap::new();
    let mut jar_class_lists = BTreeMap::new();
    for (label, document) in [
        (BASE_LABEL, elimination_document()),
        (EXTENDED_LABEL, document_with_unrelated_operation()),
    ] {
        let generation = root.join(label);
        fs::create_dir_all(&generation).unwrap();
        suspect_codegen::write_files(&generate(document), &generation).unwrap();

        // Dependency gate: the emitted pom declares zero dependencies beyond
        // the JDK, so no transitive retention is possible at all.
        let pom = fs::read_to_string(generation.join("java/pom.xml")).unwrap();
        assert!(
            !pom.contains("<dependencies>"),
            "java-http pom must declare zero runtime dependencies; found {pom}"
        );
        measurements.insert(
            format!("{label}/pom-dependencies"),
            json!("zero (no <dependencies> element; JDK only)"),
        );

        // Build the SDK artifact and compile the one-operation consumer.
        let jar = install_package(&generation);
        let class_bytes = compile_consumer(&jar, &generation);
        let class_path = generation.join("consumer/classes/OneOperation.class");
        let pool = javap_constant_pool(&class_path, &generation);

        // Consumer unit gate: exactly one operation is referenced.
        for method in ABSENT_METHOD_MARKERS {
            assert!(
                !pool.contains(method),
                "{label}: the single-operation consumer's compiled unit references unrelated operation method {method}"
            );
        }
        assert!(
            !pool.contains(UNRELATED_TYPE_MARKER),
            "{label}: the single-operation consumer's compiled unit references the unrelated operation type {UNRELATED_TYPE_MARKER}"
        );
        assert!(
            pool.contains(MARKER_OWN_METHOD),
            "{label}: the consumer's compiled unit must reference {MARKER_OWN_METHOD}"
        );

        // Artifact composition shape (documented, not fixed): the unrelated
        // operation's classes stay in the emitted jar.
        let entries = jar_entries(&jar, &generation);
        let has_unrelated = entries
            .iter()
            .any(|entry| entry.contains(UNRELATED_TYPE_MARKER));
        if label == EXTENDED_LABEL {
            assert!(
                has_unrelated,
                "documented composition retention regressed: the unrelated operation's classes no longer ship in the jar"
            );
        } else {
            assert!(!has_unrelated);
        }

        measurements.insert(format!("{label}/jar_classes"), json!(entries.len()));
        jar_class_lists.insert(label, entries);
        consumer_classes.insert(label, class_bytes);
    }

    // Additivity: the consumer's compiled unit is byte-identical whether the
    // SDK source contains five or six operations.
    let base_class = consumer_classes.get(BASE_LABEL).unwrap();
    let extended_class = consumer_classes.get(EXTENDED_LABEL).unwrap();
    assert_eq!(
        base_class, extended_class,
        "the one-operation consumer's compiled unit must be byte-identical across generations"
    );
    measurements.insert("consumer_class_bytes".to_owned(), json!(base_class.len()));
    measurements.insert(
        "additivity".to_owned(),
        json!(format!(
            "OneOperation.class byte-identical ({} B) across base and extended generations",
            base_class.len()
        )),
    );
    println!(
        "java one-operation consumer: {} B compiled unit, byte-identical across generations; unrelated method refs absent; unrelated op classes in extended jar: {}",
        base_class.len(),
        jar_class_lists[EXTENDED_LABEL]
            .iter()
            .filter(|entry| entry.contains(UNRELATED_TYPE_MARKER))
            .count()
    );

    let toolchain_version = checked(
        Command::new(java_home().join("bin/java"))
            .arg("-version")
            .env("JAVA_HOME", java_home()),
        &root,
        "java-version",
    );
    write_report(&json!({
        "gate": "java-http consumer unit (javap constant pool) + pom dependency gate",
        "toolchain": toolchain_version.trim(),
        "fixture": ENTRY,
        "recorded_composition_shape": "the emitted package is one artifact: Client carries every operation and the unrelated operation's classes ship in the jar; the consumer's own compiled unit references only its operation (method-level elimination measured, artifact-level composition documented)",
        "measurements": measurements,
        "extended_jar_unrelated_classes": jar_class_lists[EXTENDED_LABEL]
            .iter()
            .filter(|entry| entry.contains(UNRELATED_TYPE_MARKER))
            .collect::<Vec<_>>(),
    }));
}
