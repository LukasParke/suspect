//! Emitted-only incoming receipt helpers for the Kotlin backend: generation-
//! time emission shape, the no-receipt controls, and native compilation plus a
//! behavioral probe of the emitted package when a JDK/Maven toolchain is
//! available. Static runtime files are never modified; the receipt helpers
//! live entirely in the emitted `Incoming.kt`, and plans without any incoming
//! declaration emit nothing at all.

#![cfg(feature = "kotlin-sdk")]

use serde_json::{Value, json};
use std::{process::Command, sync::Arc};
use suspect_codegen::{
    OutFile,
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

/// One webhook with a required header and required JSON body plus a declared
/// reply header, one schema-free JSON webhook, one webhook with a JSON reply,
/// and one operation-attached callback receipt with a runtime expression
/// route. The same shape the shared planner and the TypeScript/Python/C#
/// backends pin.
fn incoming_document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {"title": "Incoming", "version": "1"},
        "servers": [{"url": "https://api.incoming.test/v1"}],
        "components": {
            "schemas": {
                "IssueEvent": {
                    "type": "object",
                    "properties": {"id": {"type": "string"}, "title": {"type": "string"}},
                    "required": ["id", "title"],
                    "additionalProperties": false
                },
                "Event": {
                    "type": "object",
                    "properties": {"kind": {"type": "string"}},
                    "required": ["kind"],
                    "additionalProperties": false
                },
                "Pong": {
                    "type": "object",
                    "properties": {"pong": {"type": "boolean"}},
                    "required": ["pong"],
                    "additionalProperties": false
                }
            }
        },
        "paths": {
            "/widgets": {"get": {"operationId": "listWidgets", "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"type": "array", "items": {"type": "string"}}}}}}}},
            "/subscribe": {"post": {
                "operationId": "subscribe",
                "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"type": "object", "properties": {"status": {"type": "string"}}, "required": ["status"], "additionalProperties": false}}}}},
                "callbacks": {
                    "onEvent": {
                        "{$request.body#/callbackUrl}": {
                            "post": {
                                "operationId": "eventReceived",
                                "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Event"}}}},
                                "responses": {"200": {"description": "ok"}}
                            }
                        }
                    }
                }
            }}
        },
        "webhooks": {
            "newIssue": {"post": {
                "operationId": "onNewIssue",
                "parameters": [{"name": "x-signature", "in": "header", "required": true, "schema": {"type": "string"}}],
                "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/IssueEvent"}}}},
                "responses": {"204": {"description": "Accepted", "headers": {"x-received": {"description": "when", "schema": {"type": "string"}}}}}
            }},
            "ping": {"post": {
                "operationId": "onPing",
                "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Pong"}}}}}
            }},
            "note": {"post": {
                "operationId": "onNote",
                "requestBody": {"content": {"application/json": {}}},
                "responses": {"200": {"description": "ok", "content": {"application/json": {}}}}
            }}
        }
    })
}

/// The control document: no webhooks and no callback. Nothing new may be
/// emitted for it.
fn control_document() -> Value {
    let mut document = incoming_document();
    document
        .as_object_mut()
        .unwrap()
        .remove("webhooks")
        .expect("webhooks key");
    document["paths"]["/subscribe"].as_object_mut().unwrap()["post"]
        .as_object_mut()
        .unwrap()
        .remove("callbacks")
        .expect("callbacks key");
    document
}

/// The control document with an empty webhooks map: still receipt-less.
fn emptied_document() -> Value {
    let mut document = control_document();
    document["webhooks"] = json!({});
    document
}

const ENTRY: &str = "https://source.incoming.test/openapi.json";

fn contract_with_document(document: Value, entry: &str) -> Arc<Contract> {
    let entry = Uri::parse(entry).unwrap();
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

fn generate(document: Value) -> Vec<OutFile> {
    let contract = contract_with_document(document, ENTRY);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    generate_with_options(
        contract,
        &selected,
        &TargetConfig {
            backend: Backend::KotlinHttp,
            package_name: "test.suspect:incoming-kotlin".into(),
            package_version: "0.1.0".into(),
            import_name: None,
        },
        &GenerationOptions::default(),
    )
    .unwrap()
}

fn file<'a>(files: &'a [OutFile], suffix: &str) -> &'a str {
    files
        .iter()
        .find(|file| file.path.ends_with(suffix))
        .unwrap_or_else(|| panic!("no generated file ending in {suffix}"))
        .content
        .as_str()
}

#[ignore = "requires JDK and Maven on the test host"]
#[test]
fn incoming_helpers_emit_exactly_when_receipts_are_declared() {
    let configured = generate(incoming_document());
    let control = generate(control_document());
    let emptied = generate(emptied_document());

    // Exactly one new file: the incoming receipt helpers.
    let incoming = file(&configured, "incoming_kotlin/Incoming.kt");
    assert!(
        !control
            .iter()
            .any(|file| file.path.ends_with("incoming_kotlin/Incoming.kt")),
        "receipt-less output must not carry the incoming helpers"
    );
    assert!(
        !emptied
            .iter()
            .any(|file| file.path.ends_with("incoming_kotlin/Incoming.kt")),
        "an emptied webhooks map is still receipt-less"
    );
    assert_eq!(
        configured.len(),
        control.len() + 1,
        "the declared receipts add exactly one file"
    );
    assert!(
        !control.iter().any(|file| file.path.contains("Incoming")),
        "a receipt-less contract must emit no incoming artifacts"
    );
    assert_eq!(
        control.iter().map(|file| &file.path).collect::<Vec<_>>(),
        emptied.iter().map(|file| &file.path).collect::<Vec<_>>(),
        "receipt-less documents must agree on their file lists"
    );
    for file in &control {
        assert!(
            !file.content.contains("decodeNewIssueWebhook")
                && !file.content.contains("IncomingDescriptor"),
            "{} leaked incoming helpers",
            file.path
        );
    }

    // The shared receipt types: the frozen route, the typed failure, the
    // source locator and the compiled descriptor record.
    for expected in [
        "public class IncomingRequestException(",
        "public data class IncomingRoute(",
        "public data class IncomingSource(",
        "public data class IncomingDescriptor(",
        "public object IncomingDescriptors {",
        "public val receipts: Map<String, IncomingDescriptor> = mapOf(",
    ] {
        assert!(
            incoming.contains(expected),
            "Incoming.kt lacks {expected}\n--- emitted: ---\n{incoming}"
        );
    }

    // The newIssue webhook receipt: frozen route, header presence check,
    // required-body refusal and codec decode; the declared 204 reply pins the
    // status and applies the declared reply header.
    for expected in [
        "public val NewIssueWebhookRoute: IncomingRoute = IncomingRoute(",
        "public fun decodeNewIssueWebhook(headers: Map<String, String>, body: ByteArray):",
        "incomingRequireHeader(headers, \"x-signature\")",
        "if (body.isEmpty()) throw IncomingRequestException(\"the declared required receipt body is absent\")",
        "public fun constructNewIssueResponse(typedHeaders: Map<String, String> = emptyMap()): Triple<Int, Map<String, String>, ByteArray>",
        "return Triple(204, headers, ByteArray(0))",
    ] {
        assert!(
            incoming.contains(expected),
            "Incoming.kt lacks {expected}\n--- emitted: ---\n{incoming}"
        );
    }

    // The ping receipt decodes nothing and its declared 200 reply encodes
    // through the response codec; the note receipt decodes schema-free JSON.
    for expected in [
        "public fun decodePingWebhook(headers: Map<String, String>, body: ByteArray): Unit",
        "public fun constructPingResponse(result:",
        "public fun decodeNoteWebhook(headers: Map<String, String>, body: ByteArray): JsonValue",
        "Json.parse(body)",
        "Json.stringify(result)",
    ] {
        assert!(
            incoming.contains(expected),
            "Incoming.kt lacks {expected}\n--- emitted: ---\n{incoming}"
        );
    }

    // The callback receipt carries its runtime expression verbatim and decodes
    // through its own declared schema.
    for expected in [
        "public val SubscribeOnEventWebhookRoute: IncomingRoute = IncomingRoute(",
        r#"path = "{\$request.body#/callbackUrl}","#,
        "expression = true,",
        "public fun decodeSubscribeOnEventWebhook(",
    ] {
        assert!(
            incoming.contains(expected),
            "Incoming.kt lacks {expected}\n--- emitted: ---\n{incoming}"
        );
    }

    // The frozen compiled descriptors: one entry per declared receipt.
    for expected in [
        "\"newIssue\" to IncomingDescriptor(",
        "kind = \"webhook\",",
        "method = \"POST\",",
        "path = \"newIssue\",",
        "requiredHeaders = listOf(\"x-signature\"),",
        "payload = \"json\",",
        "payloadCodec = \"onNewIssueBody\",",
        "reply = \"none\",",
        "replyStatus = 204,",
        "replyHeaders = listOf(\"x-received\"),",
        "\"ping\" to IncomingDescriptor(",
        "payload = \"none\",",
        "reply = \"json\",",
        "replyCodec = \"onPingResponse200\",",
        "replyStatus = 200,",
        "\"note\" to IncomingDescriptor(",
        "payload = \"schema-free-json\",",
        "reply = \"schema-free-json\",",
        "\"subscribe_onEvent\" to IncomingDescriptor(",
        "kind = \"callback\",",
        r#"path = "{\$request.body#/callbackUrl}","#,
        "expression = true,",
        "payloadCodec = \"eventReceivedBody\",",
    ] {
        assert!(
            incoming.contains(expected),
            "Incoming.kt descriptors lack {expected}\n--- emitted: ---\n{incoming}"
        );
    }
}

#[ignore = "requires JDK and Maven on the test host"]
#[test]
fn plan_carries_the_compiled_incoming_receipts_only_when_declared() {
    let contract = contract_with_document(incoming_document(), ENTRY);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = suspect_codegen::kotlin_sdk::plan_sdk(
        contract,
        &selected,
        suspect_codegen::kotlin_sdk::SdkConfig {
            group_id: "test.suspect".into(),
            artifact_id: "incoming-kotlin".into(),
            version: "0.1.0".into(),
            package_name: "test.suspect.incoming_kotlin".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let incoming = plan.incoming();
    assert_eq!(incoming.operations().len(), 4);
    assert_eq!(plan.incoming_entries().len(), 4);
    let new_issue = plan
        .incoming_entries()
        .iter()
        .find(|entry| entry.name == "newIssue")
        .expect("the newIssue webhook entry");
    assert_eq!(new_issue.kind, "webhook");
    assert_eq!(new_issue.method, "POST");
    assert_eq!(new_issue.route, "newIssue");
    assert!(!new_issue.expression);
    assert_eq!(new_issue.required_headers, vec!["x-signature".to_owned()]);
    assert!(new_issue.required_body);
    assert_eq!(new_issue.payload_type, "IssueEvent");
    assert_eq!(new_issue.payload_codec, "onNewIssueBody");
    let reply = new_issue.response.as_ref().expect("the declared 204 reply");
    assert_eq!(reply.status, 204);
    assert!(reply.body.is_none());
    assert_eq!(reply.headers, vec!["x-received".to_owned()]);
    let callback = plan
        .incoming_entries()
        .iter()
        .find(|entry| entry.name == "subscribe.onEvent")
        .expect("the callback entry");
    assert_eq!(callback.kind, "callback");
    assert_eq!(callback.route, "{$request.body#/callbackUrl}");
    assert!(callback.expression);

    let control_contract = contract_with_document(control_document(), ENTRY);
    let control_selected = control_contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let control = suspect_codegen::kotlin_sdk::plan_sdk(
        control_contract,
        &control_selected,
        suspect_codegen::kotlin_sdk::SdkConfig::default(),
    )
    .unwrap();
    assert!(control.incoming().is_empty());
    assert!(control.incoming_entries().is_empty());
    assert!(control.incoming().codec_roots().is_empty());
}

fn java_home() -> Option<std::path::PathBuf> {
    if let Some(home) =
        std::env::var_os("SUSPECT_KOTLIN_JAVA_HOME").or_else(|| std::env::var_os("JAVA_HOME"))
    {
        return Some(std::path::PathBuf::from(home));
    }
    // The mise-managed Temurin install used by this workspace.
    let home = std::path::PathBuf::from(std::env::var_os("HOME")?)
        .join(".local/share/mise/installs/java/temurin-21");
    let executable = home.join("bin/java");
    Command::new(&executable)
        .arg("-version")
        .output()
        .ok()
        .and_then(|output| output.status.success().then_some(home))
}

fn maven() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("SUSPECT_KOTLIN_MAVEN") {
        return Some(std::path::PathBuf::from(path));
    }
    if Command::new("mvn")
        .arg("--version")
        .output()
        .is_ok_and(|probe| probe.status.success())
    {
        return Some(std::path::PathBuf::from("mvn"));
    }
    // mise-managed Maven installs carry a nested distribution directory.
    let home = std::path::PathBuf::from(std::env::var_os("HOME")?)
        .join(".local/share/mise/installs/maven");
    let mut candidates = match std::fs::read_dir(&home) {
        Ok(installs) => installs
            .filter_map(|entry| entry.ok())
            .flat_map(|entry| {
                std::fs::read_dir(entry.path())
                    .into_iter()
                    .flatten()
                    .filter_map(|distribution| distribution.ok())
                    .map(|distribution| distribution.path().join("bin/mvn"))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };
    candidates.sort();
    candidates.into_iter().find(|path| path.is_file())
}

/// Native compile of the emitted package, when a JDK and Maven are available.
#[ignore = "requires JDK and Maven on the test host"]
#[test]
fn generated_incoming_module_compiles_with_the_package() {
    let Some(java_home) = java_home() else {
        eprintln!("kotlin_incoming: no JDK 21 found; degrading to static assertions");
        return;
    };
    let Some(maven) = maven() else {
        eprintln!("kotlin_incoming: no Maven found; degrading to static assertions");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate(incoming_document()), root.path()).unwrap();
    let output = Command::new(&maven)
        .args(["-q", "-B", "-DskipTests", "test-compile"])
        .env("JAVA_HOME", &java_home)
        .current_dir(root.path().join("kotlin"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "maven test-compile failed\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!("kotlin_incoming: maven test-compile succeeded");
}

const PROBE: &str = r#"// Fake webhook deliveries through the generated receipt helpers.
package test.suspect.incoming_kotlin

private fun expect(condition: Boolean, message: String) {
    check(condition) { "failed: $message" }
}

fun main() {
    // Route constants are registration hints: the declared method is what the
    // provider sends and the route is carried verbatim.
    expect(
        NewIssueWebhookRoute == IncomingRoute(method = "POST", path = "newIssue", expression = false),
        "the newIssue route differs",
    )
    expect(
        SubscribeOnEventWebhookRoute == IncomingRoute(
            method = "POST",
            path = "{\$request.body#/callbackUrl}",
            expression = true,
        ),
        "the callback route differs",
    )

    // A valid fake webhook POST decodes into the declared model, regardless of
    // header-name casing.
    val payload = decodeNewIssueWebhook(
        mapOf("X-Signature" to "abc"),
        """{"id":"i1","title":"t"}""".toByteArray(),
    )
    expect(payload.id == "i1", "the decoded payload id differs: ${payload.id}")
    expect(payload.title == "t", "the decoded payload title differs: ${payload.title}")

    // A missing required header is the typed incoming failure.
    val missing = try {
        decodeNewIssueWebhook(emptyMap(), """{"id":"i1","title":"t"}""".toByteArray())
        null
    } catch (thrown: IncomingRequestException) {
        thrown
    }
    expect(missing != null, "a missing required header must fail")
    expect(
        missing?.message?.contains("x-signature") == true,
        "the failure names the missing header",
    )

    // An invalid payload is a typed failure carrying the codec failure as its
    // cause.
    val invalid = try {
        decodeNewIssueWebhook(mapOf("x-signature" to "abc"), """{"id":"i1"}""".toByteArray())
        null
    } catch (thrown: IncomingRequestException) {
        thrown
    }
    expect(invalid != null, "an invalid payload must fail")
    expect(invalid?.cause != null, "the codec failure is carried as the cause")

    // A required declared body refuses an empty delivery.
    val empty = try {
        decodeNewIssueWebhook(mapOf("x-signature" to "abc"), ByteArray(0))
        null
    } catch (thrown: IncomingRequestException) {
        thrown
    }
    expect(empty != null, "an empty required body must fail")

    // A schema-free JSON body decodes as the parsed JSON value.
    val note = decodeNoteWebhook(emptyMap(), """{"seen":true}""".toByteArray())
    expect(note is JsonObject && note.values["seen"] == JsonBoolean(true), "the schema-free payload differs")

    // A schema-free JSON reply encodes the constructed value.
    val noteReply = constructNoteResponse(JsonObject(mapOf("seen" to JsonBoolean(true))))
    expect(noteReply.first == 200, "the note reply status differs: ${noteReply.first}")
    expect(
        String(noteReply.third, Charsets.UTF_8) == """{"seen":true}""",
        "the note reply body differs: ${String(noteReply.third, Charsets.UTF_8)}",
    )

    // Body-less receipts validate their headers and return.
    decodePingWebhook(emptyMap(), ByteArray(0))

    // The declared 204 reply constructs with the pinned status and the
    // declared reply header applied from the caller-provided values.
    val accepted = constructNewIssueResponse(mapOf("X-Received" to "now"))
    expect(accepted.first == 204, "the reply status differs: ${accepted.first}")
    expect(accepted.second["x-received"] == "now", "the reply header differs: ${accepted.second}")
    expect(accepted.third.isEmpty(), "a body-less reply must carry no bytes")
    // An undeclared reply header never appears.
    expect(constructNewIssueResponse(emptyMap()).second.isEmpty(), "an absent header must not appear")

    // The declared 200 reply encodes its body through the response codec.
    val reply = constructPingResponse(Pong(pong = true))
    expect(reply.first == 200, "the ping reply status differs: ${reply.first}")
    expect(String(reply.third, Charsets.UTF_8) == """{"pong":true}""", "the ping reply body differs: ${String(reply.third, Charsets.UTF_8)}")
    expect(reply.second.isEmpty(), "an undeclared reply header must not appear")

    // The callback reply constructs its declared 200 with no body.
    val callback = constructSubscribeOnEventResponse()
    expect(callback.first == 200 && callback.third.isEmpty() && callback.second.isEmpty(), "the callback reply differs")

    // The frozen compiled descriptors carry the receipt facts.
    val descriptors = IncomingDescriptors.receipts
    expect(descriptors["newIssue"]?.payload == "json", "the newIssue payload descriptor differs")
    expect(descriptors["newIssue"]?.replyStatus == 204, "the newIssue reply descriptor differs")
    expect(
        descriptors["newIssue"]?.requiredHeaders == listOf("x-signature"),
        "the newIssue header descriptor differs",
    )
    expect(
        descriptors["subscribe_onEvent"]?.path == "{\$request.body#/callbackUrl}" &&
            descriptors["subscribe_onEvent"]?.expression == true,
        "the callback descriptor differs",
    )
    expect(descriptors["ping"]?.replyStatus == 200, "the ping reply descriptor differs")
    expect(descriptors["note"]?.payload == "schema-free-json", "the note payload descriptor differs")

    println("incoming behavior verified")
}
"#;

/// Native behavioral verification of the emitted receipt helpers: decode a
/// valid fake webhook POST, refuse a missing required header, refuse an
/// invalid payload, and construct the declared replies.
#[ignore = "requires JDK and Maven on the test host"]
#[test]
fn receipts_drive_the_decoder_and_constructor() {
    let Some(java_home) = java_home() else {
        eprintln!("kotlin_incoming: no JDK 21 found; degrading to static assertions");
        return;
    };
    let Some(maven) = maven() else {
        eprintln!("kotlin_incoming: no Maven found; degrading to static assertions");
        return;
    };
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate(incoming_document()), root.path()).unwrap();
    let probe = root
        .path()
        .join("kotlin/src/test/kotlin/test/suspect/incoming_kotlin");
    std::fs::create_dir_all(&probe).unwrap();
    std::fs::write(probe.join("IncomingBehavior.kt"), PROBE).unwrap();
    let output = Command::new(&maven)
        .args(["-q", "-B", "-DskipTests", "test-compile"])
        .env("JAVA_HOME", &java_home)
        .current_dir(root.path().join("kotlin"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "maven test-compile failed for the probe\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let run = Command::new(&maven)
        .args([
            "-q",
            "-B",
            "org.codehaus.mojo:exec-maven-plugin:3.6.3:java",
            "-Dexec.mainClass=test.suspect.incoming_kotlin.IncomingBehaviorKt",
            "-Dexec.classpathScope=test",
        ])
        .env("JAVA_HOME", &java_home)
        .current_dir(root.path().join("kotlin"))
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "incoming probe failed\n{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    eprintln!("kotlin_incoming: behavioral probe succeeded");
}
