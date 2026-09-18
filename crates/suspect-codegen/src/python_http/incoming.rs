//! Emitted-only incoming receipt helpers for the Python HTTP adapter.
//!
//! The compiled `http_protocol::IncomingPlan` becomes one generated
//! `_incoming.py` module: per receipt, a decoder for what the provider sends
//! (declared required headers are presence-checked and the body decodes through
//! the package's existing model codec machinery) plus a constructor for the
//! declared exact 2xx reply and the frozen receipt route. Decoding binds the
//! same codecs the client uses for response bodies; construction encodes
//! through the request-side codecs. Static runtime files are untouched, and
//! plans without any incoming declaration emit no module and no bytes at all.
//!
//! HTTP semantics are inverted for these receipts: the declared method is the
//! verb the provider sends, and the declared responses are what the handler
//! returns. Route expressions (`{$request...}`) are carried verbatim; their
//! substitution is a runtime/framework concern.

use std::collections::{BTreeMap, BTreeSet};

use crate::http_contract::HttpDiagnostic;
use crate::http_protocol as p;
use crate::http_protocol::{
    IncomingKind, IncomingPlan, Representation, ResponseStatus, ScalarType,
};
use crate::python_http::{allocate, python_name};
use suspect_ir::contract::{Contract, SchemaId};

/// How `decode_*_webhook` interprets the received body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Payload {
    None,
    Json {
        codec: String,
    },
    SchemaFreeJson,
    Text {
        scalar: &'static str,
        codec: Option<String>,
    },
    Binary,
}

impl Payload {
    fn label(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Json { .. } => "json",
            Self::SchemaFreeJson => "schema-free-json",
            Self::Text { .. } => "text",
            Self::Binary => "binary",
        }
    }

    /// The Python annotation of the decoded payload.
    fn annotation(&self) -> String {
        match self {
            Self::None => "None".into(),
            Self::Json { codec }
            | Self::Text {
                codec: Some(codec), ..
            } => {
                format!("models.{codec}")
            }
            Self::SchemaFreeJson => "J.JsonValue".into(),
            Self::Text { codec: None, .. } => "str".into(),
            Self::Binary => "bytes".into(),
        }
    }
}

/// How `construct_*_response` encodes the declared reply body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ConstructedBody {
    Json { codec: String },
    SchemaFreeJson,
    Binary,
}

impl ConstructedBody {
    fn label(&self) -> &'static str {
        match self {
            Self::Json { .. } => "json",
            Self::SchemaFreeJson => "schema-free-json",
            Self::Binary => "binary",
        }
    }

    /// The Python annotation of the constructed reply value.
    fn annotation(&self) -> String {
        match self {
            Self::Json { codec } => format!("models.{codec}"),
            Self::SchemaFreeJson => "J.JsonValue".into(),
            Self::Binary => "bytes".into(),
        }
    }
}

/// One declared reply the construct helper builds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ConstructedResponse {
    pub status: u16,
    pub body: Option<ConstructedBody>,
    pub headers: Vec<String>,
    pub required_headers: Vec<String>,
}

/// One incoming operation's emission-ready receipt helpers with allocated,
/// collision-free Python names.
#[derive(Debug, Clone)]
pub(super) struct IncomingReceipt {
    /// Webhook key, or `parentOperationId.callbackName`.
    pub name: String,
    pub kind: &'static str,
    /// The verb the provider sends.
    pub method: String,
    pub route: String,
    pub expression: bool,
    pub document: String,
    pub pointer: String,
    pub source: String,
    pub descriptor_key: String,
    pub route_const: String,
    pub source_const: String,
    pub decode: String,
    pub construct: Option<String>,
    pub payload: Payload,
    pub required_body: bool,
    pub required_headers: Vec<String>,
    pub response: Option<ConstructedResponse>,
    pub explanations: Vec<String>,
}

fn q(text: &str) -> String {
    super::native_examples::quote(text)
}

fn scalar_name(scalar: ScalarType) -> &'static str {
    match scalar {
        ScalarType::String => "string",
        ScalarType::Boolean => "boolean",
        ScalarType::Integer => "integer",
        ScalarType::Number => "number",
    }
}

/// Compiles the incoming plan against the native symbol bindings. Receipts the
/// v1 helpers cannot decode produce source-linked `sdk-incoming-*` errors
/// instead of silent skips.
pub(super) fn prepare(
    contract: &Contract,
    plan: &IncomingPlan,
    symbols: &BTreeMap<SchemaId, String>,
    errors: &mut Vec<HttpDiagnostic>,
) -> Vec<IncomingReceipt> {
    let mut used = BTreeSet::from([
        "decode".to_owned(),
        "construct".to_owned(),
        "header_value".to_owned(),
        "require_header".to_owned(),
        "body_text".to_owned(),
        "INCOMING_DESCRIPTORS".to_owned(),
        "Source".to_owned(),
        "SdkError".to_owned(),
    ]);
    let mut result = Vec::new();
    for operation in plan.operations() {
        let source = operation.source().use_site().source().clone();
        let refuse = |errors: &mut Vec<HttpDiagnostic>, code: &'static str, message: &str| {
            errors.push(super::diagnostic(contract, source.clone(), code, message));
        };
        let snake = python_name(operation.name());
        let descriptor_key = allocate(&snake, &mut used);
        let route_const = allocate(
            &format!("{}_WEBHOOK_ROUTE", snake.to_ascii_uppercase()),
            &mut used,
        );
        let source_const = allocate(&format!("_{}", snake.to_ascii_uppercase()), &mut used);
        let source_const = format!("{source_const}_SOURCE");
        let decode = allocate(&format!("decode_{snake}_webhook"), &mut used);
        let construct = allocate(&format!("construct_{snake}_response"), &mut used);
        let required_headers = operation
            .request()
            .parameters()
            .iter()
            .filter(|parameter| {
                parameter.required() && parameter.location() == p::ParameterLocation::Header
            })
            .map(|parameter| parameter.name().to_owned())
            .collect::<Vec<_>>();
        let mut explanations = operation.explanations().clone();
        let payload = match operation.request().body() {
            None => Payload::None,
            Some(body) => {
                if body.media().len() > 1 {
                    explanations.push(
                        "the declared body offers several media; the v1 receipt helper decodes the first declared representation rather than branching on Content-Type".into(),
                    );
                }
                match body.media().first().map(p::MediaPlan::representation) {
                    None => Payload::None,
                    Some(Representation::Json { codec: Some(codec) }) => {
                        match symbols.get(codec.schema().id()) {
                            Some(name) => Payload::Json {
                                codec: name.clone(),
                            },
                            None => {
                                refuse(
                                    errors,
                                    "sdk-incoming-codec-binding",
                                    "the declared incoming payload schema has no compiled model codec",
                                );
                                continue;
                            }
                        }
                    }
                    Some(Representation::Json { codec: None }) => Payload::SchemaFreeJson,
                    Some(Representation::Text { codec, scalar, .. }) => Payload::Text {
                        scalar: scalar_name(*scalar),
                        codec: codec
                            .as_ref()
                            .and_then(|codec| symbols.get(codec.schema().id()).cloned()),
                    },
                    Some(Representation::Binary { .. }) => Payload::Binary,
                    Some(_) => {
                        refuse(
                            errors,
                            "sdk-incoming-receipt-unrepresentable",
                            "the declared incoming body representation has no v1 receipt decode; form, multipart and stream receipts are refused",
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
            .find(|response| matches!(response.status(), ResponseStatus::Exact(200..=299)));
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
                        matches!(candidate.status(), ResponseStatus::Exact(200..=299))
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
                let body = match response.media().first().map(p::MediaPlan::representation) {
                    None => None,
                    Some(Representation::Json { codec: Some(codec) }) => {
                        match symbols.get(codec.schema().id()) {
                            Some(name) => Some(ConstructedBody::Json {
                                codec: name.clone(),
                            }),
                            None => {
                                refuse(
                                    errors,
                                    "sdk-incoming-codec-binding",
                                    "the declared reply schema has no compiled model codec",
                                );
                                continue;
                            }
                        }
                    }
                    Some(Representation::Json { codec: None }) => {
                        Some(ConstructedBody::SchemaFreeJson)
                    }
                    Some(Representation::Binary { .. }) => Some(ConstructedBody::Binary),
                    Some(_) => {
                        refuse(
                            errors,
                            "sdk-incoming-response-unrepresentable",
                            "the declared reply representation has no v1 constructor; only JSON, binary and body-less replies are emitted",
                        );
                        continue;
                    }
                };
                let ResponseStatus::Exact(status) = response.status() else {
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
                        "declared reply headers are applied from the optional typed_headers argument with required-presence checks only; v1 performs no schema re-validation on constructed header values".into(),
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
                IncomingKind::Webhook => "webhook",
                IncomingKind::Callback => "callback",
            },
            method: operation.method().as_str().to_owned(),
            route: operation.route().route().clone(),
            expression: operation.route().expression(),
            document: source.document().to_string(),
            pointer: source.pointer().to_owned(),
            source: format!("{}#{}", source.document(), source.pointer()),
            descriptor_key,
            route_const,
            source_const,
            decode,
            construct: Some(construct),
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
    result
}

/// The declared body statements of one decoder.
fn decode_body(receipt: &IncomingReceipt) -> String {
    let mut code = String::new();
    if receipt.required_body {
        code.push_str(&format!(
            "    if len(body) == 0:\n        raise SdkError('incoming-request', {source}, code='incoming-request')\n",
            source = receipt.source_const,
        ));
    }
    match &receipt.payload {
        Payload::None => code.push_str("    return None\n"),
        Payload::Json { codec } => code.push_str(&format!(
            "    return codecs.{codec}Codec.decode(body_text(body, {source}))\n",
            source = receipt.source_const,
        )),
        Payload::SchemaFreeJson => code.push_str(&format!(
            "    return J.parse_json(body_text(body, {source}))\n",
            source = receipt.source_const,
        )),
        Payload::Text { scalar, codec } => match codec {
            Some(codec) => code.push_str(&format!(
                "    wire = parse_scalar(body_text(body, {source}), {scalar_kind}, {source})\n    return codecs.{codec}Codec.decode_value(wire)\n",
                source = receipt.source_const,
                scalar_kind = q(scalar),
            )),
            None => code.push_str(&format!(
                "    return parse_scalar(body_text(body, {source}), {scalar_kind}, {source})\n",
                source = receipt.source_const,
                scalar_kind = q(scalar),
            )),
        },
        Payload::Binary => code.push_str(&format!(
            "    if not isinstance(body, bytes):\n        raise SdkError('incoming-request', {source}, code='incoming-request')\n    return body\n",
            source = receipt.source_const,
        )),
    }
    code
}

/// One receipt's decoder.
fn decoder(receipt: &IncomingReceipt) -> String {
    let mut doc = format!(
        "Decode one received {} {:?} request. Declared required headers are presence-checked (a missing one raises the branded SdkError('incoming-request')) and the declared body decodes through the package's own compiled codec machinery.",
        receipt.kind, receipt.name
    );
    if !receipt.explanations.is_empty() {
        doc.push(' ');
        doc.push_str(&receipt.explanations.join(" "));
    }
    doc.push_str(&format!("\nSource: {}", receipt.source));
    let mut code = format!(
        "def {decode}(headers: Mapping[str, str], body: bytes | str) -> {annotation}:\n    {doc}\n",
        decode = receipt.decode,
        annotation = receipt.payload.annotation(),
        doc = q(&doc),
    );
    for header in &receipt.required_headers {
        code.push_str(&format!(
            "    require_header(headers, {header}, {source})\n",
            header = q(header),
            source = receipt.source_const,
        ));
    }
    code.push_str(&decode_body(receipt));
    code
}

/// One receipt's reply constructor, when an exact 2xx response is declared.
fn constructor(receipt: &IncomingReceipt, response: &ConstructedResponse) -> String {
    let Some(construct) = &receipt.construct else {
        return String::new();
    };
    let has_body = response.body.is_some();
    let has_headers = !response.headers.is_empty();
    let mut parameters = String::new();
    if has_body {
        parameters.push_str("result: ");
        parameters.push_str(
            &response
                .body
                .as_ref()
                .map(ConstructedBody::annotation)
                .unwrap_or_default(),
        );
        if has_headers {
            parameters.push_str(", ");
        }
    }
    if has_headers {
        parameters.push_str("typed_headers: Mapping[str, str] | None = None");
    }
    let mut doc = format!(
        "Construct the declared {} reply of the {} {:?} receipt: the returned tuple carries the declared status, the encoded body and the applied declared headers.",
        response.status, receipt.kind, receipt.name
    );
    if !receipt.explanations.is_empty() {
        doc.push(' ');
        doc.push_str(&receipt.explanations.join(" "));
    }
    doc.push_str(&format!("\nSource: {}", receipt.source));
    let mut code = format!(
        "def {construct}({parameters}) -> tuple[int, dict[str, str], bytes]:\n    {doc}\n",
        construct = construct,
        doc = q(&doc),
    );
    if has_body {
        if matches!(response.body, Some(ConstructedBody::Binary)) {
            code.push_str(&format!(
                "    if not isinstance(result, bytes):\n        raise SdkError('incoming-request', {source}, code='incoming-request')\n",
                source = receipt.source_const,
            ));
        }
        if matches!(response.body, Some(ConstructedBody::SchemaFreeJson)) {
            code.push_str(&format!(
                "    try:\n        reply = J.stringify_json(result)\n    except (TypeError, ValueError) as error:\n        raise SdkError('incoming-request', {source}, cause=error, code='incoming-request') from None\n",
                source = receipt.source_const,
            ));
        }
    }
    if has_headers {
        let optional = response
            .headers
            .iter()
            .filter(|header| !response.required_headers.contains(header))
            .cloned()
            .collect::<Vec<_>>();
        code.push_str("    headers: dict[str, str] = {}\n");
        if !optional.is_empty() {
            code.push_str(&format!(
                "    for name in [{names}]:\n        value = header_value(typed_headers, name)\n        if value is not None:\n            headers[name] = value\n",
                names = optional.iter().map(|h| q(h)).collect::<Vec<_>>().join(", "),
            ));
        }
        if !response.required_headers.is_empty() {
            code.push_str(&format!(
                "    for name in [{names}]:\n        value = header_value(typed_headers, name)\n        if value is None:\n            raise SdkError('incoming-request', {source}, code='incoming-request')\n        headers[name] = value\n",
                names = response
                    .required_headers
                    .iter()
                    .map(|h| q(h))
                    .collect::<Vec<_>>()
                    .join(", "),
                source = receipt.source_const,
            ));
        }
    }
    let body = match &response.body {
        Some(ConstructedBody::Json { codec }) => {
            format!("codecs.{codec}Codec.encode(result).encode('utf-8')")
        }
        Some(ConstructedBody::SchemaFreeJson) => "reply.encode('utf-8')".into(),
        Some(ConstructedBody::Binary) => "result".into(),
        None => "b''".into(),
    };
    let headers_expression = if has_headers { "headers" } else { "{}" };
    code.push_str(&format!(
        "    return ({status}, {headers_expression}, {body})\n",
        status = response.status,
    ));
    code
}

/// The Python boolean literal.
fn py_bool(value: bool) -> &'static str {
    if value { "True" } else { "False" }
}

/// A single-quoted Python literal for generated data values.
fn sq(text: &str) -> String {
    let mut out = String::from("'");
    for character in text.chars() {
        match character {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(character),
        }
    }
    out.push('\'');
    out
}

/// A Python tuple literal; the trailing comma keeps one-element tuples tuples.
fn py_tuple(items: &[String]) -> String {
    let mut out = String::from("(");
    for item in items {
        out.push_str(&sq(item));
        out.push(',');
    }
    out.push(')');
    out
}

/// The frozen compiled receipt descriptor map: generated data, never parsed
/// from OpenAPI.
fn descriptor_map(receipts: &[IncomingReceipt]) -> String {
    let mut code = String::from(
        "# Frozen compiled receipt descriptors for this package: one entry per declared\n# webhook/callback receipt. `payload` names the decode representation and\n# `reply` the constructed reply representation ('json' | 'schema-free-json' |\n# 'text' | 'binary' | 'none'); codecs name the generated model codec exports.\n",
    );
    code.push_str("INCOMING_DESCRIPTORS: Mapping[str, Mapping[str, object]] = {\n");
    for receipt in receipts {
        let (reply, reply_status, reply_codec, reply_headers) = match &receipt.response {
            None => (
                "none".to_owned(),
                "None".to_owned(),
                "None".to_owned(),
                py_tuple(&[]),
            ),
            Some(response) => (
                response
                    .body
                    .as_ref()
                    .map_or_else(|| "none", |body| body.label())
                    .to_owned(),
                response.status.to_string(),
                match &response.body {
                    Some(ConstructedBody::Json { codec }) => q(codec),
                    _ => "None".to_owned(),
                },
                py_tuple(&response.headers),
            ),
        };
        let payload_codec = match &receipt.payload {
            Payload::Json { codec }
            | Payload::Text {
                codec: Some(codec), ..
            } => sq(codec),
            _ => "None".to_owned(),
        };
        code.push_str(&format!(
            "    {key}: {{'kind': {kind}, 'method': {method}, 'route': {route}, 'expression': {expression}, 'source': Source(document={document}, pointer={pointer}), 'required_headers': {headers}, 'payload': {payload}, 'payload_codec': {payload_codec}, 'reply': {reply}, 'reply_status': {reply_status}, 'reply_codec': {reply_codec}, 'reply_headers': {reply_headers}}},\n",
            key = sq(&receipt.descriptor_key),
            kind = sq(receipt.kind),
            method = sq(&receipt.method),
            route = sq(&receipt.route),
            expression = py_bool(receipt.expression),
            document = sq(&receipt.document),
            pointer = sq(&receipt.pointer),
            headers = py_tuple(&receipt.required_headers),
            payload = sq(receipt.payload.label()),
            payload_codec = payload_codec,
            reply = sq(&reply),
            reply_status = reply_status,
            reply_codec = reply_codec,
            reply_headers = reply_headers,
        ));
    }
    code.push_str("}\n");
    code
}

/// The generated `python/src/<pkg>/_incoming.py` module: shared receipt
/// library plus the per-receipt compiled helpers and the frozen descriptor map.
pub(super) fn emit(receipts: &[IncomingReceipt]) -> String {
    let mut code = String::from(LIBRARY);
    for receipt in receipts {
        code.push_str(&format!(
            "{route_const} = {{'method': {method}, 'route': {route}, 'expression': {expression}}}\n\n{source_const} = Source(document={document}, pointer={pointer})\n\n",
            route_const = receipt.route_const,
            method = sq(&receipt.method),
            route = sq(&receipt.route),
            expression = py_bool(receipt.expression),
            source_const = receipt.source_const,
            document = sq(&receipt.document),
            pointer = sq(&receipt.pointer),
        ));
        code.push_str(&decoder(receipt));
        if let Some(response) = &receipt.response {
            code.push('\n');
            code.push_str(&constructor(receipt, response));
        }
        code.push('\n');
    }
    code.push_str(&descriptor_map(receipts));
    code
}

/// The static library half of the generated module.
const LIBRARY: &str = r#""""Generated incoming receipt helpers for this package's declared webhooks and callbacks.

HTTP semantics are inverted for these receipts: the declared method is the verb
the provider sends and the declared responses are what the handler returns.
Decoders presence-check the declared required headers and decode the declared
body through the same compiled model codecs the client uses; constructors build
the declared 2xx reply. The compiled receipt descriptors below are frozen
generated data - the runtime never parses OpenAPI. Route expressions
({$request...}) are carried verbatim; their substitution is a runtime/framework
concern.
"""
from __future__ import annotations
from typing import Any, Mapping

from . import json_runtime as J, model_codecs as codecs, models
from ._types import Source, SdkError
from ._wire import parse_scalar


def header_value(headers: Mapping[str, str], name: str) -> str | None:
    """Read one declared receipt header case-insensitively."""
    for key, value in headers.items():
        if key.lower() == name.lower():
            return value
    return None


def require_header(headers: Mapping[str, str], name: str, source: Source) -> None:
    """Raise the branded failure when a required declared receipt header is absent."""
    if header_value(headers, name) is None:
        raise SdkError('incoming-request', source, code='incoming-request')


def body_text(body: bytes | str, source: Source) -> str:
    """Decode the received body text; invalid UTF-8 is a branded failure."""
    if isinstance(body, str):
        return body
    try:
        return body.decode('utf-8')
    except UnicodeDecodeError as error:
        raise SdkError('incoming-request', source, cause=error, code='incoming-request') from None


"#;
