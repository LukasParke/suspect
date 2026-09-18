//! Emitted-only incoming receipt helpers for the Java HTTP backend: one
//! generated `Incoming.java` with per-receipt decoders, reply constructors,
//! frozen route constants and the frozen compiled descriptor map, a javac
//! compile gate over the whole package, and native behavior for decode and
//! construct against the generated codecs. Static runtime files are never
//! modified, and plans without a webhook or callback emit no new file at all.
#![cfg(all(feature = "java-sdk", feature = "http-protocol"))]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use serde_json::{Value, json};
use suspect_codegen::{
    OutFile,
    backend::{self, Backend, GenerationOptions, TargetConfig},
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

/// Two webhooks (a signed delivery with a required body and a 204 reply, and a
/// ping whose declared 200 reply carries a JSON body) and one callback with a
/// runtime-expression route: every emitted receipt shape in one package.
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
                    "required": ["id", "title"]
                },
                "Event": {
                    "type": "object",
                    "properties": {"kind": {"type": "string"}},
                    "required": ["kind"]
                },
                "Pong": {
                    "type": "object",
                    "properties": {"pong": {"type": "boolean"}},
                    "required": ["pong"]
                }
            }
        },
        "paths": {
            "/widgets": {"get": {"operationId": "listWidgets", "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"type": "array", "items": {"type": "string"}}}}}}}},
            "/subscribe": {"post": {
                "operationId": "subscribe",
                "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"type": "object", "properties": {"status": {"type": "string"}}, "required": ["status"]}}}}},
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
            "newIssue": {
                "post": {
                    "operationId": "onNewIssue",
                    "parameters": [{"name": "x-signature", "in": "header", "required": true, "schema": {"type": "string"}}],
                    "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/IssueEvent"}}}},
                    "responses": {"204": {"description": "Accepted"}}
                }
            },
            "ping": {
                "post": {
                    "operationId": "onPing",
                    "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Pong"}}}}}
                }
            }
        }
    })
}

/// The same document without any incoming declaration: the no-receipt control.
fn control_document() -> Value {
    let mut document = incoming_document();
    document
        .as_object_mut()
        .unwrap()
        .remove("webhooks")
        .expect("webhooks key");
    document["paths"]["/subscribe"] = json!({"post": {
        "operationId": "subscribe",
        "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"type": "object", "properties": {"status": {"type": "string"}}, "required": ["status"]}}}}}}
    });
    document
}

/// The incoming document with an emptied `webhooks` map: identical planning
/// outcome to removing the key entirely.
fn emptied_document() -> Value {
    let mut document = incoming_document();
    document["webhooks"] = json!({});
    document["paths"]["/subscribe"] = control_document()["paths"]["/subscribe"].clone();
    document
}

fn contract_with_document(document: Value) -> Arc<Contract> {
    let entry = Uri::parse("https://source.incoming.test/java-incoming.json").unwrap();
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

fn generate_document(document: Value) -> Vec<OutFile> {
    let contract = contract_with_document(document);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    backend::generate_with_options(
        contract,
        &selected,
        &TargetConfig {
            backend: Backend::JavaHttp,
            package_name: "test.suspect:incoming-java".into(),
            package_version: "1.0.0".into(),
            import_name: None,
        },
        &GenerationOptions::default(),
    )
    .unwrap()
}

fn generate() -> Vec<OutFile> {
    generate_document(incoming_document())
}

fn incoming_file(files: &[OutFile]) -> &OutFile {
    files
        .iter()
        .find(|file| file.path == "java/src/main/java/test/suspect/Incoming.java")
        .expect("generated Incoming.java")
}

fn paths(files: &[OutFile]) -> Vec<&str> {
    files.iter().map(|file| file.path.as_str()).collect()
}

#[test]
fn incoming_helpers_emit_exactly_when_receipts_are_declared() {
    let files = generate();
    let incoming = incoming_file(&files).content.clone();
    for expected in [
        // The generated class with its shared receipt types.
        "public final class Incoming {",
        "public record IncomingRoute(String method, String route, boolean expression) {}",
        "public static final class IncomingRequestException extends RuntimeException {",
        "public record IncomingSource(String document, String pointer) {}",
        "public record IncomingDescriptor(",
        // Per-receipt frozen route constants: the declared method is what the
        // provider sends; the callback route carries its expression verbatim.
        "public static final IncomingRoute NEW_ISSUE_WEBHOOK_ROUTE = new IncomingRoute(\"POST\", \"newIssue\", false);",
        "public static final IncomingRoute PING_WEBHOOK_ROUTE = new IncomingRoute(\"POST\", \"ping\", false);",
        "public static final IncomingRoute SUBSCRIBE_ON_EVENT_WEBHOOK_ROUTE = new IncomingRoute(\"POST\", \"{$request.body#/callbackUrl}\", true);",
        // Decoders: case-insensitive required-header presence and the model
        // codec decode.
        "public static IssueEvent decodeNewIssueWebhook(java.util.Map<String,String> headers, byte[] body) {",
        "requireHeader(headers, \"x-signature\");",
        "if (body.length == 0) throw new IncomingRequestException(\"the declared required receipt body is absent\");",
        "WebhooksNewIssuePostRequestBody.CODEC.decode(body);",
        "the received payload does not satisfy its declared schema (",
        // The declared 204 reply: pinned status, no body, no result parameter.
        "public record NewIssueResponse(int status, java.util.Map<String,String> headers, byte[] body) {}",
        "public static NewIssueResponse constructNewIssueResponse() {",
        "return new NewIssueResponse(204, java.util.Collections.unmodifiableMap(headers), new byte[0]);",
        // The 200 reply encodes through the response codec.
        "public static PingResponse constructPingResponse(Pong result) {",
        "return new PingResponse(200, java.util.Collections.unmodifiableMap(headers), JsonRuntime.bytes(",
        "WebhooksPingPostResponsesValue200.CODEC.encodeValue(result))",
        // The callback receipt decodes through its own declared schema.
        "public static Event decodeSubscribeOnEventWebhook(java.util.Map<String,String> headers, byte[] body) {",
        // Shared helpers and the frozen compiled descriptors.
        "private static String headerValue(java.util.Map<String,String> headers, String name) {",
        "private static void requireHeader(java.util.Map<String,String> headers, String name) {",
        "public static final java.util.Map<String,IncomingDescriptor> DESCRIPTORS;",
        "descriptors.put(\"newIssue\", new IncomingDescriptor(\"webhook\", \"POST\", \"newIssue\", false,",
        "descriptors.put(\"subscribe_onEvent\", new IncomingDescriptor(\"callback\", \"POST\", \"{$request.body#/callbackUrl}\", true,",
        "\"json\", \"WebhooksNewIssuePostRequestBody\", \"none\", 204, null, java.util.List.of()",
        "\"none\", null, \"json\", 200, \"WebhooksPingPostResponsesValue200\"",
    ] {
        assert!(
            incoming.contains(expected),
            "Incoming.java is missing:\n{expected}\n--- emitted: ---\n{incoming}"
        );
    }

    // Without declared receipts nothing new is emitted: no Incoming class at
    // all, with or without an empty webhooks map.
    let control = generate_document(control_document());
    let emptied = generate_document(emptied_document());
    assert!(
        !control
            .iter()
            .any(|file| file.path.ends_with("Incoming.java")),
        "a receipt-less contract must emit no Incoming.java"
    );
    assert!(
        !emptied
            .iter()
            .any(|file| file.path.ends_with("Incoming.java")),
        "an emptied webhooks map must emit no Incoming.java"
    );
    assert!(
        !paths(&control).iter().any(|path| path.contains("Incoming")),
        "a receipt-less contract must emit no incoming artifacts"
    );
    assert_eq!(paths(&control), paths(&emptied));

    // The static runtime is untouched by incoming emission: every shared
    // runtime file is byte-identical between the two documents.
    let with = generate();
    for name in [
        "JsonRuntime.java",
        "Presence.java",
        "Never.java",
        "ModelCodec.java",
        "CodecException.java",
        "Validation.java",
        "ValidationResources.java",
        "HttpRuntime.java",
        "SdkException.java",
        "Bytes.java",
        "NoContent.java",
        "ResponseBody.java",
        "Protocol.java",
        "HttpWire.java",
        "WireValue.java",
        "WireCodec.java",
        "EventStream.java",
        "RequestOptions.java",
        "ExactHttp.java",
    ] {
        let path = format!("java/src/main/java/test/suspect/{name}");
        let control_file = control
            .iter()
            .find(|file| file.path == path)
            .unwrap_or_else(|| panic!("{path} missing"));
        let with_file = with
            .iter()
            .find(|candidate| candidate.path == path)
            .unwrap_or_else(|| panic!("{path} missing"));
        assert_eq!(with_file.content, control_file.content, "{name} changed");
    }
}

/// A JDK 21+ toolchain, mirroring the other Java acceptance tests.
fn java_home() -> PathBuf {
    std::env::var_os("SUSPECT_JAVA_HOME")
        .or_else(|| std::env::var_os("JAVA_HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            "/Users/luke/.local/share/mise/installs/java/temurin-21.0.12+101.0.LTS".into()
        })
}

/// Compile the whole emitted package, including the generated Incoming class.
fn compiled_package(root: &Path) -> PathBuf {
    let home = java_home();
    assert!(
        home.join("bin/javac").is_file(),
        "required JDK: {}",
        home.display()
    );
    let mut sources = fs::read_dir(root.join("java/src/main/java/test/suspect"))
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("java"))
        .collect::<Vec<_>>();
    sources.sort();
    let classes = root.join("classes");
    fs::create_dir_all(&classes).unwrap();
    let list = root.join("sources.txt");
    fs::write(
        &list,
        sources
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();
    let output = Command::new(home.join("bin/javac"))
        .args(["--release", "21", "-Xlint:all", "-Werror", "-d"])
        .arg(&classes)
        .arg(format!("@{}", list.display()))
        .current_dir(root)
        .output()
        .unwrap();
    fs::write(root.join("javac.stdout.log"), &output.stdout).unwrap();
    fs::write(root.join("javac.stderr.log"), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for entry in fs::read_dir(root.join("java/src/main/resources/test/suspect"))
        .unwrap()
        .flatten()
    {
        fs::copy(
            entry.path(),
            classes.join("test/suspect").join(entry.file_name()),
        )
        .unwrap();
    }
    classes
}

#[test]
fn generated_incoming_compiles_strictly_with_the_package() {
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate(), root.path()).unwrap();
    compiled_package(root.path());
}

#[test]
fn fake_webhook_deliveries_drive_the_decoder_and_constructor_in_java() {
    let root = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&generate(), root.path()).unwrap();
    let classes = compiled_package(root.path());
    fs::write(root.path().join("IncomingProbe.java"), PROBE).unwrap();
    let home = java_home();
    let output = Command::new(home.join("bin/javac"))
        .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
        .arg(&classes)
        .arg("-d")
        .arg(root.path().join("probe"))
        .arg(root.path().join("IncomingProbe.java"))
        .current_dir(root.path())
        .output()
        .unwrap();
    fs::write(root.path().join("probe-javac.stderr.log"), &output.stderr).unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let mut classpath = classes.into_os_string();
    classpath.push(":");
    classpath.push(root.path().join("probe").into_os_string());
    let runtime = Command::new(home.join("bin/java"))
        .args(["-ea", "-cp"])
        .arg(&classpath)
        .arg("IncomingProbe")
        .current_dir(root.path())
        .output()
        .unwrap();
    fs::write(root.path().join("probe.stdout.log"), &runtime.stdout).unwrap();
    fs::write(root.path().join("probe.stderr.log"), &runtime.stderr).unwrap();
    assert!(
        runtime.status.success(),
        "{}{}",
        String::from_utf8_lossy(&runtime.stdout),
        String::from_utf8_lossy(&runtime.stderr)
    );
}

const PROBE: &str = r#"
import java.nio.charset.StandardCharsets;
import java.util.*;

import test.suspect.*;
import static test.suspect.JsonRuntime.*;

/** Independent incoming-receipt acceptance over the generated codecs. */
public class IncomingProbe {
    static void check(boolean value, String message) { if (!value) throw new AssertionError(message); }

    static byte[] bytes(String text) { return text.getBytes(StandardCharsets.UTF_8); }

    public static void main(String[] args) {
        // Route constants are the registration hints; the declared method is
        // what the provider sends, and the callback expression stays verbatim.
        check(Incoming.NEW_ISSUE_WEBHOOK_ROUTE.method().equals("POST"), "method");
        check(Incoming.NEW_ISSUE_WEBHOOK_ROUTE.route().equals("newIssue"), "route");
        check(!Incoming.NEW_ISSUE_WEBHOOK_ROUTE.expression(), "no expression");
        check(Incoming.PING_WEBHOOK_ROUTE.route().equals("ping"), "ping route");
        check(Incoming.SUBSCRIBE_ON_EVENT_WEBHOOK_ROUTE.expression(), "callback expression");
        check(Incoming.SUBSCRIBE_ON_EVENT_WEBHOOK_ROUTE.route().equals("{$request.body#/callbackUrl}"),
            "callback route: " + Incoming.SUBSCRIBE_ON_EVENT_WEBHOOK_ROUTE.route());

        // A valid fake webhook POST decodes into the declared model, regardless
        // of header-name casing.
        IssueEvent payload = Incoming.decodeNewIssueWebhook(
            Map.of("X-Signature", "abc"), bytes("{\"id\":\"i1\",\"title\":\"t\"}"));
        check(payload.id().equals("i1"), "decoded id: " + payload.id());
        check(payload.title().equals("t"), "decoded title: " + payload.title());

        // A missing required header is the typed incoming-request failure.
        try {
            Incoming.decodeNewIssueWebhook(new HashMap<>(), bytes("{\"id\":\"i1\",\"title\":\"t\"}"));
            throw new AssertionError("expected a missing-header failure");
        } catch (Incoming.IncomingRequestException error) {
            check(error.getMessage().contains("x-signature"), "missing header message: " + error.getMessage());
        }

        // An invalid payload is a typed failure too, without leaking the body.
        try {
            Incoming.decodeNewIssueWebhook(Map.of("x-signature", "abc"), bytes("{\"id\":\"i1\"}"));
            throw new AssertionError("expected a schema failure");
        } catch (Incoming.IncomingRequestException error) {
            check(!error.getMessage().contains("i1"), "the failure carries no payload text: " + error.getMessage());
        }

        // Malformed JSON is a typed failure.
        try {
            Incoming.decodeNewIssueWebhook(Map.of("x-signature", "abc"), bytes("{"));
            throw new AssertionError("expected a JSON failure");
        } catch (Incoming.IncomingRequestException error) {
            check(error.getMessage().contains("not valid JSON") || error.getMessage().contains("declared schema"),
                "malformed payload message: " + error.getMessage());
        }

        // A required declared body refuses an empty delivery.
        try {
            Incoming.decodeNewIssueWebhook(Map.of("x-signature", "abc"), new byte[0]);
            throw new AssertionError("expected an absent-body failure");
        } catch (Incoming.IncomingRequestException error) {
            check(error.getMessage().contains("absent"), "absent body message: " + error.getMessage());
        }

        // The callback receipt decodes through its own declared schema.
        Incoming.decodeSubscribeOnEventWebhook(new HashMap<>(), bytes("{\"kind\":\"open\"}"));

        // The declared 204 reply constructs with the pinned status, no headers
        // and no body.
        Incoming.NewIssueResponse accepted = Incoming.constructNewIssueResponse();
        check(accepted.status() == 204, "204 status: " + accepted.status());
        check(accepted.headers().isEmpty(), "no headers: " + accepted.headers());
        check(accepted.body().length == 0, "no body");

        // The declared 200 reply encodes its body through the response codec.
        Pong pong = WebhooksPingPostResponsesValue200.CODEC.decode(bytes("{\"pong\":true}"));
        Incoming.PingResponse reply = Incoming.constructPingResponse(pong);
        check(reply.status() == 200, "200 status: " + reply.status());
        check(reply.headers().isEmpty(), "no declared reply headers");
        check(new String(reply.body(), StandardCharsets.UTF_8).equals(WebhooksPingPostResponsesValue200.CODEC.encode(pong)),
            "the reply body is the response codec encoding");

        // The frozen compiled descriptors: generated data, never parsed source.
        Incoming.IncomingDescriptor newIssue = Incoming.DESCRIPTORS.get("newIssue");
        check(newIssue != null, "the newIssue descriptor exists");
        check(newIssue.kind().equals("webhook") && newIssue.method().equals("POST") && newIssue.route().equals("newIssue"),
            "newIssue descriptor identity");
        check(newIssue.requiredHeaders().equals(List.of("x-signature")), "required headers");
        check(newIssue.payload().equals("json"), "payload representation");
        check(newIssue.reply().equals("none") && newIssue.replyStatus() == 204, "reply");
        Incoming.IncomingDescriptor callback = Incoming.DESCRIPTORS.get("subscribe_onEvent");
        check(callback != null && callback.kind().equals("callback") && callback.expression(),
            "callback descriptor identity");
        check(callback.source().pointer().contains("/callbacks/"), "callback descriptor source");

        System.out.println("incoming behavior verified");
    }
}
"#;
