//! Golden typed-stream semantics bind through the public, admitted protocol
//! plan; unadmitted documents compile the documented conservative plan.
use std::sync::Arc;

use serde_json::{Value, json};
use suspect_codegen::{http_protocol, rust_http};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

fn contract_with_document(document: Value) -> Arc<Contract> {
    let entry = Uri::parse("https://source.stream.test/openapi.json").unwrap();
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

fn protocol_plan(contract: &Arc<Contract>) -> http_protocol::ProtocolPlan {
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    http_protocol::plan(contract, &selected, rust_http::native_capabilities_v3())
}

fn stream_plan(
    contract: &Arc<Contract>,
) -> (
    http_protocol::ProtocolPlan,
    http_protocol::StreamSemanticsPlan,
) {
    let protocol = protocol_plan(contract);
    let plan = http_protocol::plan_stream_semantics(contract, &protocol);
    (protocol, plan)
}

fn document_with_paths(paths: Value) -> Value {
    json!({
        "openapi": "3.2.0",
        "info": {"title": "Streams", "version": "1"},
        "paths": paths
    })
}

#[test]
fn a_discriminated_sse_item_schema_compiles_one_event_per_declared_kind() {
    let contract = contract_with_document(document_with_paths(json!({
        "/chat": {"post": {
            "operationId": "streamChat",
            "responses": {"200": {"description": "stream", "content": {"text/event-stream": {
                "itemSchema": {
                    "type": "object",
                    "properties": {
                        "event": {"type": "string", "enum": ["message", "done"]},
                        "data": {"type": "string"}
                    },
                    "required": ["event", "data"]
                }
            }}}}
        }}
    })));
    let (protocol, plan) = stream_plan(&contract);
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    assert_eq!(plan.streams.len(), 1);
    let stream = &plan.streams[0];
    assert_eq!(stream.operation, "streamChat");
    assert_eq!(stream.media_type, "text/event-stream");
    assert_eq!(
        stream.framing,
        http_protocol::StreamFraming::ServerSentEvents
    );
    assert_eq!(stream.events.len(), 2);
    assert_eq!(stream.events[0].event_name, "message");
    assert_eq!(stream.events[1].event_name, "done");
    let item_codec = stream.item_codec.as_ref().unwrap().schema().id().clone();
    for event in &stream.events {
        let http_protocol::EventPayload::Json { codec } = &event.payload else {
            panic!("declared kinds decode typed JSON items");
        };
        assert_eq!(*codec.schema().id(), item_codec);
        assert!(!event.fatal);
    }
    // No sentinel evidence: the description never declares one.
    assert!(!stream.sentinel.enabled);
    assert_eq!(stream.sentinel.token, "");
    assert!(stream.sentinel.evidence.is_none());
    assert_eq!(
        stream.sentinel.stage,
        http_protocol::SentinelStage::BeforeJsonDecode
    );
    assert_eq!(
        stream.terminal.on_sentinel,
        http_protocol::TerminalAction::Complete
    );
    assert_eq!(
        stream.terminal.on_eof,
        http_protocol::TerminalAction::Complete
    );
    assert!(!stream.terminal.keep_final_usage);
    assert_eq!(
        stream
            .item_metadata
            .as_ref()
            .unwrap()
            .event_field
            .as_deref(),
        Some("event")
    );
    assert!(
        stream
            .explanations
            .iter()
            .any(|note| note.contains("discriminator property"))
    );
}

#[test]
fn a_declared_sentinel_enables_before_json_decode_completion() {
    let contract = contract_with_document(document_with_paths(json!({
        "/transcribe": {"post": {
            "operationId": "streamTranscription",
            "description": "Audio chunks arrive as JSON objects. [DONE] terminates the stream.",
            "responses": {"200": {"description": "stream", "content": {"text/event-stream": {
                "itemSchema": {"type": "object", "properties": {"data": {"type": "string"}}}
            }}}}
        }}
    })));
    let (protocol, plan) = stream_plan(&contract);
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    assert_eq!(plan.streams.len(), 1);
    let stream = &plan.streams[0];
    assert!(stream.sentinel.enabled);
    assert_eq!(stream.sentinel.token, "[DONE]");
    assert_eq!(
        stream.sentinel.stage,
        http_protocol::SentinelStage::BeforeJsonDecode
    );
    assert!(matches!(
        stream.sentinel.evidence,
        Some(http_protocol::SentinelEvidence::Description { .. })
    ));
    assert!(stream.terminal.keep_final_usage);
    assert_eq!(
        stream.terminal.on_sentinel,
        http_protocol::TerminalAction::Complete
    );
    // Without discrimination evidence, one default event decodes the envelope.
    assert_eq!(stream.events.len(), 1);
    assert_eq!(
        stream.events[0].event_name,
        http_protocol::StreamEventPlan::DEFAULT_EVENT_NAME
    );
    assert!(matches!(
        stream.events[0].payload,
        http_protocol::EventPayload::Json { .. }
    ));
    assert!(
        stream
            .explanations
            .iter()
            .any(|note| note.contains("sentinel"))
    );
}

#[test]
fn an_undeclared_item_schema_compiles_the_conservative_text_plan() {
    let contract = contract_with_document(document_with_paths(json!({
        "/logs": {"get": {
            "operationId": "streamLogs",
            "responses": {"200": {"description": "stream", "content": {"text/event-stream": {}}}}
        }}
    })));
    let (protocol, plan) = stream_plan(&contract);
    // The protocol planner refuses stream media without OAS 3.2 itemSchema, so
    // no admitted operation exists and the conservative plan applies.
    assert!(!protocol.is_admitted());
    assert_eq!(plan.streams.len(), 1);
    let stream = &plan.streams[0];
    assert_eq!(stream.operation, "streamLogs");
    assert!(stream.item_codec.is_none());
    assert_eq!(stream.events.len(), 1);
    assert_eq!(stream.events[0].event_name, "default");
    assert_eq!(stream.events[0].payload, http_protocol::EventPayload::Text);
    assert!(stream.item_metadata.is_none());
    assert!(!stream.sentinel.enabled);
    assert!(
        stream
            .explanations
            .iter()
            .any(|note| note.contains("not admitted"))
    );
}

#[test]
fn json_lines_streams_compile_per_line_json_payloads_without_sentinels() {
    let contract = contract_with_document(document_with_paths(json!({
        "/rows": {"get": {
            "operationId": "streamRows",
            "responses": {"200": {"description": "lines", "content": {"application/x-ndjson": {
                "itemSchema": {"type": "object", "properties": {"value": {"type": "integer"}}}
            }}}}
        }}
    })));
    let (protocol, plan) = stream_plan(&contract);
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    assert_eq!(plan.streams.len(), 1);
    let stream = &plan.streams[0];
    assert_eq!(stream.operation, "streamRows");
    assert_eq!(stream.media_type, "application/x-ndjson");
    assert_eq!(stream.framing, http_protocol::StreamFraming::JsonLines);
    assert_eq!(stream.events.len(), 1);
    assert!(matches!(
        stream.events[0].payload,
        http_protocol::EventPayload::Json { .. }
    ));
    assert!(!stream.sentinel.enabled);
    assert!(!stream.terminal.keep_final_usage);
    // JSON-lines framing facts, and no SSE envelope metadata.
    assert!(stream.frame_rules.line_delimited);
    assert!(stream.frame_rules.eof_completes);
    assert!(!stream.frame_rules.comments_ignored);
    assert!(!stream.frame_rules.multiline_data_joined);
    assert!(stream.item_metadata.is_none());
}

#[test]
fn a_non_stream_operation_compiles_no_stream_entry() {
    let contract = contract_with_document(document_with_paths(json!({
        "/widgets": {"get": {
            "operationId": "getWidget",
            "responses": {"200": {"description": "ok", "content": {"application/json": {
                "schema": {"type": "object", "properties": {"name": {"type": "string"}}}
            }}}}
        }}
    })));
    let (protocol, plan) = stream_plan(&contract);
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    assert!(plan.streams.is_empty());
}

#[test]
fn two_stream_operations_compile_independently() {
    let contract = contract_with_document(document_with_paths(json!({
        "/events": {"get": {
            "operationId": "streamEvents",
            "responses": {"200": {"description": "stream", "content": {"text/event-stream": {
                "itemSchema": {"type": "object", "properties": {"data": {"type": "string"}}}
            }}}}
        }},
        "/metrics": {"get": {
            "operationId": "streamMetrics",
            "responses": {"200": {"description": "lines", "content": {"application/x-ndjson": {
                "itemSchema": {"type": "object", "properties": {"value": {"type": "integer"}}}
            }}}}
        }}
    })));
    let (protocol, plan) = stream_plan(&contract);
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    assert_eq!(plan.streams.len(), 2);
    assert_eq!(plan.streams[0].operation, "streamEvents");
    assert_eq!(
        plan.streams[0].framing,
        http_protocol::StreamFraming::ServerSentEvents
    );
    assert_eq!(plan.streams[1].operation, "streamMetrics");
    assert_eq!(
        plan.streams[1].framing,
        http_protocol::StreamFraming::JsonLines
    );
    // Declaring nothing on either operation keeps both policies independent
    // and conservative.
    assert!(!plan.streams[0].sentinel.enabled);
    assert!(!plan.streams[1].sentinel.enabled);
    assert_ne!(plan.streams[0], plan.streams[1]);
}

#[test]
fn the_plan_serializes_to_stable_kebab_case_json() {
    let contract = contract_with_document(document_with_paths(json!({
        "/chat": {"post": {
            "operationId": "streamChat",
            "responses": {"200": {"description": "stream", "content": {"text/event-stream": {
                "itemSchema": {
                    "type": "object",
                    "properties": {
                        "event": {"type": "string", "enum": ["message", "done"]},
                        "data": {"type": "string"}
                    }
                }
            }}}}
        }},
        "/transcribe": {"post": {
            "operationId": "streamTranscription",
            "description": "Audio chunks arrive as JSON objects. [DONE] terminates the stream.",
            "responses": {"200": {"description": "stream", "content": {"text/event-stream": {
                "itemSchema": {"type": "object", "properties": {"data": {"type": "string"}}}
            }}}}
        }}
    })));
    let (_, plan) = stream_plan(&contract);
    let value = serde_json::to_value(&plan).unwrap();
    let reparsed: Value = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(&reparsed).unwrap(), value);
    assert_eq!(value["streams"].as_array().unwrap().len(), 2);
    let chat = &value["streams"][0];
    assert_eq!(chat["framing"], "server-sent-events");
    assert_eq!(chat["events"][0]["payload"]["kind"], "json");
    assert_eq!(chat["unknown_events"]["representation"], "typed-unknown");
    assert_eq!(
        chat["unknown_events"]["invalid_payload"],
        "decoding-failure"
    );
    let transcription = &value["streams"][1];
    assert_eq!(transcription["sentinel"]["token"], "[DONE]");
    assert_eq!(transcription["sentinel"]["stage"], "before-json-decode");
    assert_eq!(transcription["sentinel"]["evidence"]["kind"], "description");
    assert_eq!(transcription["terminal"]["on_sentinel"], "complete");
    assert_eq!(transcription["terminal"]["on_eof"], "complete");
    assert_eq!(transcription["terminal"]["keep_final_usage"], true);
}

#[test]
fn oneof_variants_compile_one_event_per_declared_kind_with_variant_codecs() {
    let contract = contract_with_document(document_with_paths(json!({
        "/chat": {"post": {
            "operationId": "streamChat",
            "responses": {"200": {"description": "stream", "content": {"text/event-stream": {
                "itemSchema": {"oneOf": [
                    {
                        "type": "object",
                        "properties": {
                            "event": {"const": "message"},
                            "data": {"type": "string"}
                        },
                        "required": ["event", "data"]
                    },
                    {
                        "type": "object",
                        "properties": {"event": {"const": "done"}},
                        "required": ["event"]
                    }
                ]}
            }}}}
        }}
    })));
    let (protocol, plan) = stream_plan(&contract);
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    assert_eq!(plan.streams.len(), 1);
    let stream = &plan.streams[0];
    assert_eq!(stream.events.len(), 2);
    assert_eq!(stream.events[0].event_name, "message");
    assert_eq!(stream.events[1].event_name, "done");
    let base = "/paths/~1chat/post/responses/200/content/text~1event-stream/itemSchema/oneOf";
    let http_protocol::EventPayload::Json { codec } = &stream.events[0].payload else {
        panic!("variant payload");
    };
    assert_eq!(codec.schema().id().pointer(), format!("{base}/0"));
    let http_protocol::EventPayload::Json { codec } = &stream.events[1].payload else {
        panic!("variant payload");
    };
    assert_eq!(codec.schema().id().pointer(), format!("{base}/1"));
}

#[test]
fn ambiguous_discrimination_compiles_a_single_default_event_with_explanation() {
    let contract = contract_with_document(document_with_paths(json!({
        "/chat": {"post": {
            "operationId": "streamChat",
            "responses": {"200": {"description": "stream", "content": {"text/event-stream": {
                "itemSchema": {
                    "type": "object",
                    "properties": {
                        "event": {"type": "string", "enum": ["message", "done"]},
                        "type": {"type": "string", "enum": ["delta", "final"]},
                        "data": {"type": "string"}
                    }
                }
            }}}}
        }}
    })));
    let (protocol, plan) = stream_plan(&contract);
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    assert_eq!(plan.streams.len(), 1);
    let stream = &plan.streams[0];
    assert_eq!(stream.events.len(), 1);
    assert_eq!(stream.events[0].event_name, "default");
    assert!(
        stream
            .explanations
            .iter()
            .any(|note| note.contains("ambiguous"))
    );
}

#[test]
fn an_annotation_declaring_a_sentinel_enables_it_with_recorded_evidence() {
    let contract = contract_with_document(document_with_paths(json!({
        "/notify": {"post": {
            "operationId": "streamNotifications",
            "x-sse-sentinel": "[END]",
            "responses": {"200": {"description": "stream", "content": {"text/event-stream": {
                "itemSchema": {"type": "object", "properties": {"data": {"type": "string"}}}
            }}}}
        }}
    })));
    let (protocol, plan) = stream_plan(&contract);
    // Extensions stay annotations; they still carry sentinel evidence here.
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    assert_eq!(plan.streams.len(), 1);
    let stream = &plan.streams[0];
    assert!(stream.sentinel.enabled);
    assert_eq!(stream.sentinel.token, "[END]");
    assert!(matches!(
        stream.sentinel.evidence,
        Some(http_protocol::SentinelEvidence::Annotation { .. })
    ));
    assert!(stream.terminal.keep_final_usage);
}

#[test]
fn fatal_event_marking_follows_description_and_annotation_evidence() {
    let contract = contract_with_document(document_with_paths(json!({
        "/chat": {"post": {
            "operationId": "streamChat",
            "description": "A fatal error event ends the stream; message events are ordinary.",
            "x-fatal-events": ["aborted"],
            "responses": {"200": {"description": "stream", "content": {"text/event-stream": {
                "itemSchema": {
                    "type": "object",
                    "properties": {
                        "event": {"type": "string", "enum": ["message", "error", "aborted"]},
                        "data": {"type": "string"}
                    }
                }
            }}}}
        }}
    })));
    let (protocol, plan) = stream_plan(&contract);
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    let events = &plan.streams[0].events;
    let fatal_of = |name: &str| {
        events
            .iter()
            .find(|event| event.event_name == name)
            .unwrap()
            .fatal
    };
    // "A fatal error event" names `error` next to fatal wording; the
    // annotation names `aborted`; `message` stays an ordinary event.
    assert!(fatal_of("error"));
    assert!(fatal_of("aborted"));
    assert!(!fatal_of("message"));
}
