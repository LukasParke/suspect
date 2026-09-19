//! Versioned compatibility profiles close the published-SDK admission gap for
//! real-world dialect patterns. Every profile is opt-in: without it, each
//! refusal and emitted byte stays exactly as before (the pinned fixtures and
//! no-policy tests are the byte-identity gate).

#![cfg(feature = "http-protocol")]

use std::{process::Command, sync::Arc};

use serde_json::{Value, json};
use suspect_codegen::{
    backend::{Backend, GenerationOptions, TargetConfig, generate_with_options},
    http_protocol,
    python_codecs::{self, CodecConfig},
    rust_http,
};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn contract_with_document(document: Value) -> Arc<Contract> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(&path, document.to_string()).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap())
}

fn selected(contract: &Arc<Contract>) -> Vec<SourceId> {
    contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect()
}

fn rust_plan(
    document: &Value,
    profiles: Vec<http_protocol::CompatibilityProfile>,
) -> Result<rust_http::HttpPlan, Vec<rust_http::HttpDiagnostic>> {
    let contract = contract_with_document(document.clone());
    let selected = selected(&contract);
    rust_http::plan_http(
        contract,
        &selected,
        rust_http::HttpConfig {
            compatibility_profiles: profiles,
            ..rust_http::HttpConfig::default()
        },
    )
}

fn refusal_codes(
    document: &Value,
    profiles: Vec<http_protocol::CompatibilityProfile>,
) -> Vec<String> {
    match rust_plan(document, profiles) {
        Ok(_) => panic!("the document was unexpectedly admitted"),
        Err(errors) => {
            assert!(!errors.is_empty());
            errors
                .into_iter()
                .map(|error| error.code.to_owned())
                .collect()
        }
    }
}

fn all_profiles() -> Vec<http_protocol::CompatibilityProfile> {
    vec![
        http_protocol::CompatibilityProfile::Oas30NullableIn31V1,
        http_protocol::CompatibilityProfile::ColonPathParametersV1,
        http_protocol::CompatibilityProfile::SchemalessStreamEventsV1,
    ]
}

fn tool_available(name: &str) -> bool {
    Command::new(name).arg("--version").output().is_ok()
}

// ---------------------------------------------------------------------------
// Profile 2: colon-path-parameters-v1
// ---------------------------------------------------------------------------

fn colon_paths_document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {"title": "Colon paths", "version": "1"},
        "servers": [{"url": "https://api.colon.test/v1"}],
        "components": {"securitySchemes": {"apiKey": {"type": "http", "scheme": "bearer"}}},
        "security": [{"apiKey": []}],
        "paths": {
            "/keys/:hash": {"delete": {
                "operationId": "deleteKeysHash",
                "parameters": [{"name": "hash", "in": "path", "required": true, "schema": {"type": "string"}}],
                "responses": {"204": {"description": "deleted"}}
            }},
            "/workspaces/:id/members/:member": {"delete": {
                "operationId": "removeMember",
                "parameters": [
                    {"name": "id", "in": "path", "required": true, "schema": {"type": "string"}},
                    {"name": "member", "in": "path", "required": true, "schema": {"type": "string"}}
                ],
                "responses": {"204": {"description": "removed"}}
            }},
            "/ghosts/:ghost": {"get": {
                "operationId": "getGhost",
                "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string"}}],
                "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"type": "object"}}}}}
            }}
        }
    })
}

#[test]
fn without_the_colon_profile_the_template_refuses() {
    let document = colon_paths_document();
    let codes = refusal_codes(&document, vec![]);
    assert!(
        codes.iter().any(|code| code == "http-path-parameters"),
        "{codes:?}"
    );
}

fn declared_colon_paths_document() -> Value {
    let mut document = colon_paths_document();
    document["paths"]
        .as_object_mut()
        .unwrap()
        .remove("/ghosts/:ghost");
    document
}

#[test]
fn with_the_colon_profile_declared_segments_normalize() {
    let document = declared_colon_paths_document();
    let plan = rust_plan(
        &document,
        vec![http_protocol::CompatibilityProfile::ColonPathParametersV1],
    )
    .unwrap();
    let path = plan
        .operations()
        .iter()
        .find(|operation| operation.operation_id == "removeMember")
        .unwrap()
        .wire()
        .path();
    // The template normalizes to the OAS brace expression before matching.
    assert_eq!(path, "/workspaces/{id}/members/{member}");
    // The declared path parameters stay bound to their normalized expressions.
    let parameters: Vec<_> = plan
        .operations()
        .iter()
        .find(|operation| operation.operation_id == "removeMember")
        .unwrap()
        .wire()
        .parameters()
        .iter()
        .filter(|parameter| parameter.location() == http_protocol::ParameterLocation::Path)
        .map(|parameter| parameter.name().to_owned())
        .collect();
    assert_eq!(parameters, vec!["id", "member"]);
}

#[test]
fn an_undeclared_colon_segment_still_refuses_with_the_profile() {
    let document = colon_paths_document();
    let codes = refusal_codes(&document, all_profiles());
    // The ghost colon segment has no declared parameter: never guessed.
    assert!(
        codes.iter().any(|code| code == "http-path-parameters"),
        "{codes:?}"
    );
    let plan = rust_plan(
        &json!({
            "openapi": "3.1.0",
            "info": {"title": "Ghost", "version": "1"},
            "servers": [{"url": "https://api.colon.test/v1"}],
            "paths": {
                "/ghosts/:ghost": {"get": {
                    "operationId": "getGhost",
                    "parameters": [{"name": "id", "in": "path", "required": true, "schema": {"type": "string"}}],
                    "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {"type": "object"}}}}}
                }}
            }
        }),
        vec![http_protocol::CompatibilityProfile::ColonPathParametersV1],
    )
    .unwrap_err();
    assert!(
        plan.iter()
            .any(|error| error.code == "http-path-parameters")
    );
}

// ---------------------------------------------------------------------------
// Profile 1: oas30-nullable-in-3.1-v1
// ---------------------------------------------------------------------------

fn nullable_document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {"title": "Nullable", "version": "1"},
        "paths": {},
        "components": {"schemas": {
            "Workspace": {"type": "object", "required": ["name", "description"],
                "properties": {
                    "name": {"type": "string"},
                    "description": {"type": "string", "nullable": true}
                }},
            "Tags": {"type": "object", "required": ["values"],
                "properties": {
                    "values": {"type": ["array", "null"], "items": {"type": "string"}, "nullable": false}
                }},
            "Untyped": {"description": "no type", "nullable": true}
        }}
    })
}

/// The emitted codec-plan field types for one model field, as the emitted
/// `model_codecs` consume them (`kind: nullable` nests the inner type).
fn emitted_field_types(policy: suspect_codegen::schema_view::DialectPolicy) -> Value {
    let contract = contract_with_document(nullable_document());
    let config = CodecConfig {
        dialect: policy,
        ..CodecConfig::default()
    };
    let plan =
        python_codecs::plan_codecs(contract.clone(), contract.schema_roots(), config).unwrap();
    let files = plan.render();
    let codec_plan: Value = serde_json::from_str(
        &files
            .iter()
            .find(|file| file.path == "python/codec-plan.json")
            .unwrap()
            .content,
    )
    .unwrap();
    codec_plan["models"]
        .as_object()
        .unwrap()
        .iter()
        .filter_map(|(name, model)| Some((name.clone(), model.get("fields")?.clone())))
        .collect::<serde_json::Map<String, Value>>()
        .into()
}

#[test]
fn without_the_nullable_profile_the_31_annotation_stays_semantics_free() {
    let models = emitted_field_types(suspect_codegen::schema_view::DialectPolicy::default());
    // `description` is a plain str: a null wire value has no represented state.
    let description = &models["Workspace"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["name"] == "description")
        .unwrap()["type"];
    assert_eq!(description["kind"], "primitive", "{description}");
}

#[test]
fn with_the_nullable_profile_the_type_gains_null() {
    let models = emitted_field_types(suspect_codegen::schema_view::DialectPolicy {
        oas30_nullable_in_31: true,
        ..suspect_codegen::schema_view::DialectPolicy::default()
    });
    // {type: "string", nullable: true} behaves as {type: ["string", "null"]}.
    let description = &models["Workspace"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["name"] == "description")
        .unwrap()["type"];
    assert_eq!(description["kind"], "nullable", "{description}");
    // nullable: false on a type array removes "null": the null value refuses.
    let values = &models["Tags"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["name"] == "values")
        .unwrap()["type"];
    assert_ne!(values["kind"], "nullable", "{values}");
}

#[test]
fn the_nullable_profile_leaves_oas30_documents_unchanged() {
    let document = json!({
        "openapi": "3.0.3",
        "info": {"title": "Legacy", "version": "1"},
        "paths": {},
        "components": {"schemas": {
            "Item": {"type": "object", "required": ["note"],
                "properties": {"note": {"type": "string", "nullable": true}}}
        }}
    });
    let contract = contract_with_document(document);
    let shape = |policy: suspect_codegen::schema_view::DialectPolicy| {
        emitted_field_types_for(&contract, policy).to_string()
    };
    assert_eq!(
        shape(suspect_codegen::schema_view::DialectPolicy::default()),
        shape(suspect_codegen::schema_view::DialectPolicy {
            oas30_nullable_in_31: true,
            ..suspect_codegen::schema_view::DialectPolicy::default()
        }),
        "OAS 3.0 nullable is already dialect semantics"
    );
}

fn emitted_field_types_for(
    contract: &Arc<Contract>,
    policy: suspect_codegen::schema_view::DialectPolicy,
) -> Value {
    let config = CodecConfig {
        dialect: policy,
        ..CodecConfig::default()
    };
    let plan =
        python_codecs::plan_codecs(contract.clone(), contract.schema_roots(), config).unwrap();
    let files = plan.render();
    let codec_plan: Value = serde_json::from_str(
        &files
            .iter()
            .find(|file| file.path == "python/codec-plan.json")
            .unwrap()
            .content,
    )
    .unwrap();
    codec_plan
}

#[test]
fn the_nullable_profile_decodes_null_in_the_emitted_python_codecs() {
    if !tool_available(
        &std::env::var_os("SUSPECT_PYTHON_BIN")
            .unwrap_or_else(|| "python3".into())
            .to_string_lossy(),
    ) {
        eprintln!("python3 is not on PATH; skipping the native codec check");
        return;
    }
    let contract = contract_with_document(nullable_document());
    for (policy, accepts_null) in [
        (
            suspect_codegen::schema_view::DialectPolicy::default(),
            false,
        ),
        (
            suspect_codegen::schema_view::DialectPolicy {
                oas30_nullable_in_31: true,
                ..suspect_codegen::schema_view::DialectPolicy::default()
            },
            true,
        ),
    ] {
        let config = CodecConfig {
            dialect: policy,
            ..CodecConfig::default()
        };
        let plan =
            python_codecs::plan_codecs(contract.clone(), contract.schema_roots(), config).unwrap();
        let root = tempfile::tempdir().unwrap().keep();
        suspect_codegen::write_files(&plan.render(), &root).unwrap();
        let python = root.join("python");
        std::fs::write(
            python.join("consumer.py"),
            r#"from model_codecs import WorkspaceCodec, TagsCodec
from codec_runtime import CodecError
try:
    value = WorkspaceCodec.decode('{"name": "eng", "description": null}')
except CodecError:
    print("REJECTED")
    raise SystemExit(0)
assert value.description is None
try:
    TagsCodec.decode('{"values": null}')
except CodecError:
    pass
else:
    print("NULLABLE FALSE")
    raise SystemExit(1)
print("OK")
"#,
        )
        .unwrap();
        let output = Command::new(
            std::env::var_os("SUSPECT_PYTHON_BIN").unwrap_or_else(|| "python3".into()),
        )
        .arg("consumer.py")
        .current_dir(&python)
        .output()
        .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        if accepts_null {
            assert!(
                stdout.contains("OK"),
                "the nullable profile must decode the null wire value: {}{}",
                stdout,
                String::from_utf8_lossy(&output.stderr)
            );
        } else {
            assert!(
                stdout.contains("REJECTED"),
                "the strict profile decoded a null into a non-null carrier: {}{}",
                stdout,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

// ---------------------------------------------------------------------------
// Profile 3: schemaless-stream-events-v1
// ---------------------------------------------------------------------------

fn stream_document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {"title": "Schemaless streams", "version": "1"},
        "servers": [{"url": "https://api.streams.test/v1"}],
        "paths": {
            "/chat": {"post": {
                "operationId": "streamChat",
                "requestBody": {"content": {"application/json": {"schema": {
                    "type": "object", "properties": {"prompt": {"type": "string"}}
                }}}},
                "responses": {"200": {"description": "chat", "content": {"text/event-stream": {
                    "schema": {"$ref": "#/components/schemas/Envelope"},
                    "x-speakeasy-sse-sentinel": "[DONE]"
                }}}}
            }},
            "/logs": {"get": {
                "operationId": "streamLogs",
                "responses": {"200": {"description": "events", "content": {"text/event-stream": {
                    "x-speakeasy-sse-sentinel": "[DONE]"
                }}}}
            }},
            "/rows": {"get": {
                "operationId": "streamRows",
                "responses": {"200": {"description": "rows", "content": {"application/x-ndjson": {}}}}
            }},
            "/echo": {"post": {
                "operationId": "streamEcho",
                "requestBody": {"content": {"text/event-stream": {}}},
                "responses": {"204": {"description": "done"}}
            }}
        },
        "components": {"schemas": {
            "Envelope": {"type": "object", "required": ["data"],
                "properties": {"data": {"$ref": "#/components/schemas/Chunk"}}},
            "Chunk": {"type": "object", "properties": {"text": {"type": "string"}}}
        }}
    })
}

#[test]
fn without_the_stream_profile_the_schemaless_sse_refuses() {
    let codes = refusal_codes(&stream_document(), vec![]);
    assert!(
        codes
            .iter()
            .any(|code| code == "http-stream-item-schema-required"),
        "{codes:?}"
    );
}

fn schemaless_sse_document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {"title": "Schemaless SSE", "version": "1"},
        "servers": [{"url": "https://api.streams.test/v1"}],
        "paths": {
            "/chat": {"post": {
                "operationId": "streamChat",
                "responses": {"200": {"description": "chat", "content": {"text/event-stream": {
                    "schema": {"$ref": "#/components/schemas/Envelope"},
                    "x-speakeasy-sse-sentinel": "[DONE]"
                }}}}
            }},
            "/logs": {"get": {
                "operationId": "streamLogs",
                "responses": {"200": {"description": "events", "content": {"text/event-stream": {
                    "x-speakeasy-sse-sentinel": "[DONE]"
                }}}}
            }}
        },
        "components": {"schemas": {
            "Envelope": {"type": "object", "required": ["data"],
                "properties": {"data": {"$ref": "#/components/schemas/Chunk"}}},
            "Chunk": {"type": "object", "properties": {"text": {"type": "string"}}}
        }}
    })
}

#[test]
fn with_the_stream_profile_schemaless_sse_admits_untyped_frames() {
    let contract = contract_with_document(schemaless_sse_document());
    let selected = selected(&contract);
    let capabilities = all_profiles().iter().fold(
        rust_http::native_capabilities_v3(),
        |capabilities, profile| capabilities.with_profile(*profile),
    );
    let protocol = http_protocol::plan(&contract, &selected, capabilities)
        .into_result()
        .unwrap();
    for id in ["streamLogs", "streamChat"] {
        let operation = protocol
            .operations()
            .iter()
            .find(|operation| {
                operation
                    .operation_id()
                    .is_some_and(|located| located.value() == id)
            })
            .unwrap_or_else(|| panic!("{id} admitted"));
        let matched = operation
            .match_response(200, Some("text/event-stream"))
            .unwrap();
        let http_protocol::Representation::Stream { stream } =
            matched.media().unwrap().representation()
        else {
            panic!("{id} admits as a stream");
        };
        assert!(
            stream.item_codec().is_none(),
            "{id}: no item codec is compiled"
        );
    }
}

#[test]
fn json_lines_and_request_streams_keep_refusing_with_the_stream_profile() {
    let codes = refusal_codes(&stream_document(), all_profiles());
    assert!(
        codes
            .iter()
            .any(|code| code == "http-stream-item-schema-required"),
        "JSON-lines and request streams keep their ordinary rules: {codes:?}"
    );
}

#[test]
fn a_declared_item_schema_keeps_strict_admission_with_the_stream_profile() {
    let contract = contract_with_document(json!({
        "openapi": "3.2.0",
        "info": {"title": "Declared items", "version": "1"},
        "paths": {"/chat": {"post": {
            "operationId": "streamChat",
            "responses": {"200": {"description": "chat", "content": {"text/event-stream": {
                "itemSchema": {
                    "type": "object",
                    "properties": {
                        "event": {"type": "string", "enum": ["message", "done"]},
                        "data": {"type": "string"}
                    },
                    "required": ["event", "data"]
                }
            }}}}
        }}}
    }));
    let selected = selected(&contract);
    let capabilities = all_profiles().iter().fold(
        rust_http::native_capabilities_v3(),
        |capabilities, profile| capabilities.with_profile(*profile),
    );
    let protocol = http_protocol::plan(&contract, &selected, capabilities)
        .into_result()
        .unwrap();
    let matched = protocol.operations()[0]
        .match_response(200, Some("text/event-stream"))
        .unwrap();
    let http_protocol::Representation::Stream { stream } =
        matched.media().unwrap().representation()
    else {
        panic!("declared item schema");
    };
    assert!(stream.item_codec().is_some());
}

#[test]
fn the_schemaless_semantics_plan_aligns_with_the_conservative_text_plan() {
    let contract = contract_with_document(json!({
        "openapi": "3.1.0",
        "info": {"title": "Aligned", "version": "1"},
        "paths": {"/logs": {"get": {
            "operationId": "streamLogs",
            "responses": {"200": {"description": "events", "content": {"text/event-stream": {}}}}
        }}}
    }));
    let selected = selected(&contract);
    let capabilities = all_profiles().iter().fold(
        rust_http::native_capabilities_v3(),
        |capabilities, profile| capabilities.with_profile(*profile),
    );
    let protocol = http_protocol::plan(&contract, &selected, capabilities)
        .into_result()
        .unwrap();
    assert!(protocol.is_admitted());
    let semantics = http_protocol::plan_stream_semantics(&contract, &protocol);
    assert_eq!(semantics.streams.len(), 1);
    let compiled = &semantics.streams[0];
    assert!(compiled.item_codec.is_none());
    assert_eq!(compiled.events.len(), 1);
    assert_eq!(
        compiled.events[0].event_name,
        http_protocol::StreamEventPlan::DEFAULT_EVENT_NAME
    );
    assert_eq!(
        compiled.events[0].payload,
        http_protocol::EventPayload::Text
    );
    assert!(compiled.item_metadata.is_none());
    assert!(
        compiled
            .explanations
            .iter()
            .any(|note| note.contains("schemaless"))
    );
}

// ---------------------------------------------------------------------------
// End-to-end emitted SDK spot checks through the generation seam
// ---------------------------------------------------------------------------

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::TypescriptHttp,
        package_name: "@compat/profiles".into(),
        package_version: "1.0.0".into(),
        import_name: None,
    }
}

fn generated(document: Value, options: &GenerationOptions) -> Vec<suspect_codegen::OutFile> {
    let contract = contract_with_document(document);
    let selected = selected(&contract);
    generate_with_options(contract, &selected, &target(), options).unwrap()
}

fn generation_options() -> GenerationOptions {
    GenerationOptions {
        compatibility_profiles: all_profiles().into_iter().collect(),
        ..GenerationOptions::default()
    }
}

#[test]
fn the_emitted_typescript_sdk_normalizes_colon_paths_and_surfaces_null() {
    let document = json!({
        "openapi": "3.1.0",
        "info": {"title": "Profiles", "version": "1"},
        "servers": [{"url": "https://api.profiles.test/v1"}],
        "components": {"securitySchemes": {"apiKey": {"type": "http", "scheme": "bearer"}}},
        "security": [{"apiKey": []}],
        "paths": {
            "/keys/:hash": {"delete": {
                "operationId": "deleteKeysHash",
                "parameters": [{"name": "hash", "in": "path", "required": true, "schema": {"type": "string"}}],
                "responses": {"204": {"description": "deleted"}}
            }},
            "/items": {"get": {
                "operationId": "getItem",
                "responses": {"200": {"description": "ok", "content": {"application/json": {"schema": {
                    "type": "object", "required": ["name", "note"],
                    "properties": {
                        "name": {"type": "string"},
                        "note": {"type": "string", "nullable": true}
                    }
                }}}}}
            }}
        }
    });
    let files = generated(document, &generation_options());
    let operations = files
        .iter()
        .find(|file| file.path == "typescript/operations.ts")
        .unwrap();
    // The colon segment renders as the OAS brace expression with its parameter.
    assert!(
        operations.content.contains("\"/keys/{hash}\""),
        "the normalized template must render"
    );
    let models = files
        .iter()
        .find(|file| file.path == "typescript/models.ts")
        .unwrap();
    assert!(
        models.content.contains("string | null"),
        "the nullable field surfaces a null type"
    );
}

#[test]
fn the_emitted_typescript_sdk_streams_schemaless_sse_as_untyped_frames() {
    if !tool_available("node") {
        eprintln!("node is not on PATH; skipping the emitted schemaless-stream check");
        return;
    }
    let document = json!({
        "openapi": "3.1.0",
        "info": {"title": "Schemaless", "version": "1"},
        "servers": [{"url": "https://api.streams.test/v1"}],
        "paths": {
            "/logs": {"get": {
                "operationId": "streamLogs",
                "responses": {"200": {"description": "events", "content": {"text/event-stream": {
                    "x-speakeasy-sse-sentinel": "[DONE]"
                }}}}
            }}
        }
    });
    let files = generated(document, &generation_options());
    let operations = files
        .iter()
        .find(|file| file.path == "typescript/operations.ts")
        .unwrap();
    // The item type is the untyped parsed envelope value, and the descriptor's
    // stream item codec is null so the runtime JSON-parses each framed envelope.
    let at = operations.content.find("streamLogs").unwrap_or(0);
    let window = &operations.content[at..(at + 1500).min(operations.content.len())];
    assert!(
        operations
            .content
            .contains("AsyncIterable<Models.JsonValue>"),
        "{window}"
    );
}

#[test]
fn the_profiles_are_opt_in_for_generation() {
    let document = declared_colon_paths_document();
    assert!(rust_plan(&document, vec![]).is_err());
    assert!(rust_plan(&document, all_profiles()).is_ok());
}
