//! Compiled incoming receipt emission inputs for the Rust HTTP adapter.
//!
//! The shared `http_protocol::IncomingPlan` is lowered here into one
//! emission-ready receipt per declared webhook and callback. Rendering happens
//! in `rust_http/emit/incoming.rs`; this module owns the source admission
//! decisions: receipts the v1 Rust helpers cannot express (anything beyond JSON
//! bodies) refuse with source-linked `sdk-incoming-*` diagnostics instead of
//! being skipped.

use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::http_protocol as wire;
use suspect_ir::contract::{Contract, SchemaId};

/// How `decode_<x>_webhook` interprets the received body. The Rust helpers are
/// JSON-only: bodies without a compiled model codec stay schema-free JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IncomingPayload {
    /// No declared request body: the decoder validates headers and returns `()`.
    None,
    /// JSON through the operation's own model codec.
    Json {
        /// The compiled model codec's model name.
        model: String,
        /// The payload schema source, for decoding-failure branding.
        schema: SchemaId,
    },
    /// Schema-free JSON: the decoded value is the parsed JSON value.
    SchemaFreeJson,
}

impl IncomingPayload {
    /// The descriptor label of the decode representation.
    pub fn label(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Json { .. } => "json",
            Self::SchemaFreeJson => "schema-free-json",
        }
    }

    /// The model codec of the decoded payload, when it has one.
    pub fn codec(&self) -> Option<&str> {
        match self {
            Self::Json { model, .. } => Some(model),
            _ => None,
        }
    }
}

/// How `construct_<x>_response` encodes the declared reply body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstructedBody {
    /// JSON through the response model codec.
    Json {
        /// The compiled model codec's model name.
        model: String,
        /// The reply schema source, for encoding-failure branding.
        schema: SchemaId,
    },
    /// Schema-free JSON: the reply value is written as JSON verbatim.
    SchemaFreeJson,
}

impl ConstructedBody {
    /// The descriptor label of the reply representation.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Json { .. } => "json",
            Self::SchemaFreeJson => "schema-free-json",
        }
    }
}

/// One declared reply the construct helper builds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstructedResponse {
    pub status: u16,
    pub body: Option<ConstructedBody>,
    /// Declared reply header wire names, in declaration order.
    pub headers: Vec<String>,
    pub required_headers: Vec<String>,
}

/// One incoming operation's emission-ready receipt helpers with allocated,
/// collision-free public Rust names.
#[derive(Debug, Clone)]
pub struct IncomingReceipt {
    /// Webhook key, or `parentOperationId.callbackName`.
    pub name: String,
    /// `"webhook"` or `"callback"`.
    pub kind: &'static str,
    /// The verb the provider sends.
    pub method: String,
    pub route: String,
    pub expression: bool,
    /// The receipt declaration's use-site source.
    pub source: SourceId,
    /// Allocated `decode_<x>_webhook` function name.
    pub decode: String,
    /// Allocated `construct_<x>_response` function name, present exactly when
    /// an exact 2xx reply is declared.
    pub construct: Option<String>,
    /// Allocated route-constant name.
    pub route_const: String,
    /// Allocated descriptor-constant name.
    pub descriptor_const: String,
    /// Allocated source-constant name (the receipt declaration's use site).
    pub source_const: String,
    pub payload: IncomingPayload,
    pub required_body: bool,
    pub required_headers: Vec<String>,
    pub response: Option<ConstructedResponse>,
    pub explanations: Vec<String>,
}

impl IncomingReceipt {
    /// The Rust type of one decoded payload.
    pub fn payload_type(&self) -> String {
        match &self.payload {
            IncomingPayload::None => "()".into(),
            IncomingPayload::Json { model, .. } => format!("crate::models::{model}"),
            IncomingPayload::SchemaFreeJson => "crate::JsonValue".into(),
        }
    }

    /// Whether the decoder decodes a body through a model codec.
    pub fn decodes_codec(&self) -> bool {
        matches!(self.payload, IncomingPayload::Json { .. })
    }
}

/// Compiles the incoming plan against the compiled codec symbols. Receipts the
/// v1 Rust helpers cannot express refuse with source-linked `sdk-incoming-*`
/// diagnostics instead of silent skips.
pub(super) fn prepare(
    contract: &Contract,
    incoming: &wire::IncomingPlan,
    symbols: &BTreeMap<SchemaId, String>,
) -> Result<Vec<IncomingReceipt>, Vec<HttpDiagnostic>> {
    let mut errors = Vec::new();
    // Module-local names every receipt helper can reach.
    let mut used = BTreeSet::from([
        "IncomingRoute".to_owned(),
        "IncomingDescriptor".to_owned(),
        "RECEIPTS".to_owned(),
        "JSON_LIMITS".to_owned(),
        "header_value".to_owned(),
        "receipt_failure".to_owned(),
        "decode_failure".to_owned(),
        "construct_failure".to_owned(),
        "payload_json_failure".to_owned(),
        "reply_json_failure".to_owned(),
        "Source".to_owned(),
        "SdkError".to_owned(),
        "SdkErrorKind".to_owned(),
        "CodecError".to_owned(),
        "JsonError".to_owned(),
    ]);
    let mut result = Vec::new();
    for operation in incoming.operations() {
        let source = operation.source().use_site().source().clone();
        let refuse = |errors: &mut Vec<HttpDiagnostic>, code: &'static str, message: String| {
            errors.push(diagnostic(contract, source.clone(), code, message));
        };
        let stem = rust_models::snake(operation.name());
        let upper = stem.to_ascii_uppercase();
        let decode = allocate(&format!("decode_{stem}_webhook"), &mut used);
        let construct = allocate(&format!("construct_{stem}_response"), &mut used);
        let route_const = allocate(&format!("{upper}_ROUTE"), &mut used);
        let descriptor_const = allocate(&format!("{upper}_DESCRIPTOR"), &mut used);
        let source_const = allocate(&format!("{upper}_SOURCE"), &mut used);
        let required_headers = operation
            .request()
            .parameters()
            .iter()
            .filter(|parameter| {
                parameter.required() && parameter.location() == wire::ParameterLocation::Header
            })
            .map(|parameter| parameter.name().to_owned())
            .collect::<Vec<_>>();
        let mut explanations = operation.explanations().clone();
        let payload = match operation.request().body() {
            None => IncomingPayload::None,
            Some(body) => {
                if body.media().len() > 1 {
                    explanations.push(
                        "the declared body offers several media; the v1 receipt helper decodes the first declared representation rather than branching on Content-Type".into(),
                    );
                }
                match body.media().first().map(wire::MediaPlan::representation) {
                    None => IncomingPayload::None,
                    Some(wire::Representation::Json { codec: Some(codec) }) => {
                        match symbols.get(codec.schema().id()) {
                            Some(model) => IncomingPayload::Json {
                                model: model.clone(),
                                schema: codec.schema().id().clone(),
                            },
                            None => {
                                refuse(
                                    &mut errors,
                                    "sdk-incoming-codec-binding",
                                    "the declared incoming payload schema has no compiled model codec".into(),
                                );
                                continue;
                            }
                        }
                    }
                    Some(wire::Representation::Json { codec: None }) => {
                        IncomingPayload::SchemaFreeJson
                    }
                    Some(_) => {
                        refuse(
                            &mut errors,
                            "sdk-incoming-receipt-unrepresentable",
                            "the declared incoming body representation has no v1 Rust receipt decode; only JSON receipts are emitted and form, multipart, stream, text and binary receipts are refused".into(),
                        );
                        continue;
                    }
                }
            }
        };
        // The constructed reply: the first declared exact 2xx response. Range
        // and default statuses cannot pin one concrete status, so they explain
        // instead of inventing one.
        let chosen = operation
            .responses()
            .iter()
            .find(|response| matches!(response.status(), wire::ResponseStatus::Exact(200..=299)));
        let response = match chosen {
            None => {
                explanations.push(
                    "no exact 2xx response is declared for this receipt; no construct helper is emitted".into(),
                );
                None
            }
            Some(response) => {
                if operation
                    .responses()
                    .iter()
                    .filter(|candidate| {
                        matches!(candidate.status(), wire::ResponseStatus::Exact(200..=299))
                    })
                    .count()
                    > 1
                {
                    explanations.push(
                        "several exact 2xx responses are declared; the construct helper builds the first declared one".into(),
                    );
                }
                if response.media().len() > 1 {
                    explanations.push(
                        "the declared reply offers several media; the v1 constructor encodes the first declared representation".into(),
                    );
                }
                let body = match response
                    .media()
                    .first()
                    .map(wire::MediaPlan::representation)
                {
                    None => None,
                    Some(wire::Representation::Json { codec: Some(codec) }) => {
                        match symbols.get(codec.schema().id()) {
                            Some(model) => Some(ConstructedBody::Json {
                                model: model.clone(),
                                schema: codec.schema().id().clone(),
                            }),
                            None => {
                                refuse(
                                    &mut errors,
                                    "sdk-incoming-codec-binding",
                                    "the declared reply schema has no compiled model codec".into(),
                                );
                                continue;
                            }
                        }
                    }
                    Some(wire::Representation::Json { codec: None }) => {
                        Some(ConstructedBody::SchemaFreeJson)
                    }
                    Some(_) => {
                        refuse(
                                &mut errors,
                                "sdk-incoming-response-unrepresentable",
                                "the declared reply representation has no v1 Rust constructor; only JSON and body-less replies are emitted".into(),
                            );
                        continue;
                    }
                };
                let wire::ResponseStatus::Exact(status) = response.status() else {
                    unreachable!("chosen response is an exact 2xx status");
                };
                let headers = response
                    .headers()
                    .iter()
                    .map(|header| header.name().to_owned())
                    .collect::<Vec<_>>();
                let required_headers = response
                    .headers()
                    .iter()
                    .filter(|header| header.required())
                    .map(|header| header.name().to_owned())
                    .collect::<Vec<_>>();
                if !headers.is_empty() {
                    explanations.push(
                        "declared reply headers are applied from the typed_headers argument with required-presence checks only; v1 performs no schema re-validation on constructed header values".into(),
                    );
                }
                Some(ConstructedResponse {
                    status,
                    body,
                    headers,
                    required_headers,
                })
            }
        };
        result.push(IncomingReceipt {
            name: operation.name().clone(),
            kind: match operation.kind() {
                wire::IncomingKind::Webhook => "webhook",
                wire::IncomingKind::Callback => "callback",
            },
            method: operation.method().as_str().to_owned(),
            route: operation.route().route().clone(),
            expression: operation.route().expression(),
            source,
            decode,
            construct: Some(construct),
            route_const,
            descriptor_const,
            source_const,
            payload,
            required_body: operation
                .request()
                .body()
                .as_ref()
                .is_some_and(|body| body.required()),
            required_headers,
            response,
            explanations,
        });
    }
    if errors.is_empty() {
        Ok(result)
    } else {
        Err(errors)
    }
}
