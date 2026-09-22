//! Emitted-only incoming receipt helpers for the Ruby gem.
//!
//! The compiled `http_protocol::IncomingPlan` becomes one generated
//! `lib/<require>/incoming.rb` module: per receipt, a decoder for what the
//! provider sends (declared required headers are presence-checked and the body
//! decodes through the package's existing model codec machinery) plus a
//! constructor for the declared exact 2xx reply and the frozen receipt route.
//! The module is registered beside the generated OAuth module — required after
//! it — and the static runtime files and the shared planner stay untouched.
//! Plans without any incoming declaration emit no module and no bytes at all,
//! so receipt-less output stays byte-identical.
//!
//! HTTP semantics are inverted for these receipts: the declared method is the
//! verb the provider sends, and the declared responses are what the handler
//! returns. The v1 Ruby receipt helpers decode and construct JSON media only —
//! form, multipart, stream, text and binary receipts are refused with
//! source-linked diagnostics instead of silent skips. Route expressions
//! (`{$request...}`) are carried verbatim; their substitution is a
//! runtime/framework concern.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use super::models;
use crate::http_contract::HttpDiagnostic;
use crate::http_protocol as wire;
use crate::http_protocol::{IncomingKind, ResponseStatus};
use suspect_ir::contract::{Contract, SchemaId};

/// How `decode_*_webhook` interprets the received body. JSON media only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Payload {
    /// No declared request body: the decoder validates headers and returns nil.
    None,
    /// JSON through the receipt's own compiled model codec.
    Json {
        codec: String,
        /// The RBS annotation of the decoded payload.
        type_name: String,
    },
    /// Schema-free JSON: the decoded value is the parsed JSON value.
    SchemaFreeJson,
}

/// How `construct_*_response` encodes the declared reply body. JSON media only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstructedBody {
    Json {
        codec: String,
        /// The RBS annotation of the constructed reply value.
        type_name: String,
    },
    SchemaFreeJson,
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
/// collision-free Ruby names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingReceipt {
    /// Webhook key, or `parentOperationId.callbackName`.
    pub name: String,
    pub kind: &'static str,
    /// The verb the provider sends.
    pub method: String,
    pub route: String,
    pub expression: bool,
    /// The `document#pointer` source of the receipt declaration.
    pub source: String,
    /// The frozen route constant (`NEW_ISSUE_WEBHOOK_ROUTE`).
    pub route_const: String,
    /// The module function decoding one received request.
    pub decode: String,
    /// The module function constructing the declared reply, when an exact 2xx
    /// response is declared.
    pub construct: Option<String>,
    pub payload: Payload,
    pub required_body: bool,
    pub required_headers: Vec<String>,
    pub response: Option<ConstructedResponse>,
    pub explanations: Vec<String>,
}

/// The compiled incoming emission carried by one Ruby plan: the receipts of
/// every declared webhook and callback, in plan order.
#[derive(Debug, Clone, Default)]
pub struct IncomingEmission {
    pub receipts: Vec<IncomingReceipt>,
}

impl IncomingEmission {
    /// Whether the plan carries no emittable receipt, so no module is emitted.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.receipts.is_empty()
    }
}

/// The module-local helper names the receipt names must never collide with.
fn reserved() -> BTreeSet<String> {
    BTreeSet::from([
        "decode".to_owned(),
        "construct".to_owned(),
        "header_value".to_owned(),
        "SOURCE".to_owned(),
    ])
}

fn q(text: &str) -> String {
    serde_json::to_string(text).unwrap().replace('#', "\\#")
}

fn esc(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('@', "&#64;")
        .replace('{', "&#123;")
        .replace('}', "&#125;")
        .replace(['\r', '\n'], " ")
}

/// Compiles the incoming plan against the compiled model codecs. Receipts the
/// v1 Ruby helpers cannot express surface as source-linked diagnostics
/// instead of silent skips.
pub(super) fn prepare(
    contract: &Contract,
    incoming: &wire::IncomingPlan,
    symbols: &BTreeMap<SchemaId, String>,
    type_names: &BTreeMap<SchemaId, String>,
    errors: &mut Vec<HttpDiagnostic>,
) -> IncomingEmission {
    let mut used = reserved();
    let mut receipts = Vec::new();
    for operation in incoming.operations() {
        let source = operation.source().use_site().source().clone();
        let refuse = |errors: &mut Vec<HttpDiagnostic>, code: &'static str, message: &str| {
            errors.push(super::diagnostic(contract, source.clone(), code, message));
        };
        let snake = crate::rust_models::snake(operation.name());
        let route_const = models::allocate(
            &format!("{}_WEBHOOK_ROUTE", snake.to_ascii_uppercase()),
            &mut used,
        );
        let decode = models::allocate(&format!("decode_{snake}_webhook"), &mut used);
        let construct = models::allocate(&format!("construct_{snake}_response"), &mut used);
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
            None => Payload::None,
            Some(body) => {
                if body.media().len() > 1 {
                    explanations.push(
                        "the declared body offers several media; the v1 receipt helper decodes the first declared representation rather than branching on Content-Type".into(),
                    );
                }
                match body.media().first().map(wire::MediaPlan::representation) {
                    None => Payload::None,
                    Some(wire::Representation::Json { codec: Some(codec) }) => {
                        let Some(name) = symbols.get(codec.schema().id()) else {
                            refuse(
                                errors,
                                "ruby-incoming-codec-binding",
                                "the declared incoming payload schema has no compiled model codec",
                            );
                            continue;
                        };
                        Payload::Json {
                            codec: name.clone(),
                            type_name: format!(
                                "Types::{}",
                                type_names
                                    .get(codec.schema().id())
                                    .expect("compiled codec type name")
                            ),
                        }
                    }
                    Some(wire::Representation::Json { codec: None }) => Payload::SchemaFreeJson,
                    Some(_) => {
                        refuse(
                            errors,
                            "ruby-incoming-representation",
                            "the declared incoming body representation has no v1 receipt decode; the Ruby receipt helpers decode JSON media only",
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
                let body = match response.media().first().map(wire::MediaPlan::representation) {
                    None => None,
                    Some(wire::Representation::Json { codec: Some(codec) }) => {
                        let Some(name) = symbols.get(codec.schema().id()) else {
                            refuse(
                                errors,
                                "ruby-incoming-codec-binding",
                                "the declared reply schema has no compiled model codec",
                            );
                            continue;
                        };
                        Some(ConstructedBody::Json {
                            codec: name.clone(),
                            type_name: format!(
                                "Types::{}",
                                type_names
                                    .get(codec.schema().id())
                                    .expect("compiled codec type name")
                            ),
                        })
                    }
                    Some(wire::Representation::Json { codec: None }) => {
                        Some(ConstructedBody::SchemaFreeJson)
                    }
                    Some(_) => {
                        refuse(
                            errors,
                            "ruby-incoming-representation",
                            "the declared reply representation has no v1 constructor; the Ruby receipt helpers construct JSON media only",
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
                let required_reply_headers = response
                    .headers()
                    .iter()
                    .filter(|header| header.required())
                    .map(|header| header.name().to_owned())
                    .collect::<Vec<_>>();
                if !headers.is_empty() {
                    explanations.push(
                        "declared reply headers are applied from the optional typed_headers keyword with required-presence checks only; v1 performs no schema re-validation on constructed header values".into(),
                    );
                }
                Some(ConstructedResponse {
                    status,
                    body,
                    headers,
                    required_headers: required_reply_headers,
                })
            }
        };
        receipts.push(IncomingReceipt {
            name: operation.name().clone(),
            kind: match operation.kind() {
                IncomingKind::Webhook => "webhook",
                IncomingKind::Callback => "callback",
            },
            method: operation.method().as_str().to_owned(),
            route: operation.route().route().clone(),
            expression: operation.route().expression(),
            source: {
                let document = source.document().to_string();
                format!("{document}#{}", source.pointer())
            },
            route_const,
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
    IncomingEmission { receipts }
}

/// The description, explanation and source comment lines of one emitted
/// member.
fn docs(out: &mut String, description: &str, receipt: &IncomingReceipt) {
    if !description.is_empty() {
        let _ = writeln!(out, "    # {}.", esc(description));
    }
    if !receipt.explanations.is_empty() {
        let joined = receipt.explanations.join(" ");
        let _ = writeln!(out, "    # {}", esc(&joined));
    }
    let _ = writeln!(out, "    # Source: <code>{}</code>.", esc(&receipt.source));
}

/// The declared body statements of one decoder.
fn decode_body(receipt: &IncomingReceipt) -> String {
    let mut code = String::new();
    if receipt.required_body {
        let _ = write!(
            code,
            "      unless body.instance_of?(String) && !body.empty?\n        raise SdkError.new('the declared required receipt body is absent', kind: :'incoming-request', source: SOURCE, operation_id: {name})\n      end\n",
            name = q(&receipt.name),
        );
    }
    match &receipt.payload {
        Payload::None => code.push_str("      nil\n"),
        Payload::Json { codec, .. } => {
            let _ = writeln!(code, "      Codecs::{codec}.decode_json(body)");
        }
        Payload::SchemaFreeJson => code.push_str("      Json.parse(body)\n"),
    }
    code
}

/// One receipt's route constant.
fn route_constant(out: &mut String, receipt: &IncomingReceipt) {
    let name = esc(&receipt.name);
    let _ = writeln!(
        out,
        "    # The declared receipt route of the {kind} \"{name}\": +method+ is the verb\n    # the provider sends and the path is carried verbatim from the source\n    # declaration. Register a handler at this path in your framework; runtime\n    # expression substitution is a framework concern.",
        kind = receipt.kind,
        name = name,
    );
    docs(out, "", receipt);
    let _ = writeln!(
        out,
        "    {} = {{ method: {method}, path: {path}, expression: {expression} }}.freeze\n",
        receipt.route_const,
        method = q(&receipt.method),
        path = q(&receipt.route),
        expression = receipt.expression,
    );
}

/// One receipt's decoder.
fn decoder(out: &mut String, receipt: &IncomingReceipt) {
    let name = esc(&receipt.name);
    let _ = writeln!(
        out,
        "    # Decodes one received {kind} \"{name}\" request: declared required headers\n    # are presence-checked (a missing one raises SdkError kind\n    # :'incoming-request') and the declared body decodes through the\n    # package's own compiled codec machinery.",
        kind = receipt.kind,
        name = name,
    );
    docs(out, "", receipt);
    let _ = writeln!(
        out,
        "    def {decode}(headers, body)",
        decode = receipt.decode,
    );
    for header in &receipt.required_headers {
        let _ = writeln!(
            out,
            "      unless header_value(headers, {header})\n        raise SdkError.new('required receipt header {header} is absent', kind: :'incoming-request', source: SOURCE, operation_id: {name})\n      end",
            header = q(header),
            name = q(&receipt.name),
        );
    }
    out.push_str(&decode_body(receipt));
    out.push_str("    end\n\n");
}

/// One receipt's reply constructor, when an exact 2xx response is declared.
fn constructor(out: &mut String, receipt: &IncomingReceipt, response: &ConstructedResponse) {
    let Some(construct) = &receipt.construct else {
        return;
    };
    let has_body = response.body.is_some();
    let has_headers = !response.headers.is_empty();
    let name = esc(&receipt.name);
    let _ = writeln!(
        out,
        "    # Constructs the declared {status} reply of the {kind} \"{name}\" receipt: the\n    # returned triple carries the declared status, the encoded body and the\n    # applied declared headers.",
        status = response.status,
        kind = receipt.kind,
        name = name,
    );
    docs(out, "", receipt);
    let mut signature = String::from(construct.as_str());
    if has_body {
        signature.push_str("(result");
        if has_headers {
            signature.push_str(", typed_headers: nil");
        }
        signature.push(')');
    } else if has_headers {
        signature.push_str("(typed_headers: nil)");
    }
    let _ = writeln!(out, "    def {signature}");
    if has_headers {
        out.push_str("      headers = {}\n");
        let optional = response
            .headers
            .iter()
            .filter(|header| !response.required_headers.contains(header))
            .cloned()
            .collect::<Vec<_>>();
        for header in &optional {
            let _ = writeln!(
                out,
                "      value = header_value(typed_headers, {header})\n      headers[{header}] = value if value",
                header = q(header),
            );
        }
        for header in &response.required_headers {
            let _ = writeln!(
                out,
                "      value = header_value(typed_headers, {header})\n      unless value\n        raise SdkError.new('the declared required reply header {header} is absent', kind: :'incoming-request', source: SOURCE, operation_id: {name})\n      end\n      headers[{header}] = value",
                header = q(header),
                name = q(&receipt.name),
            );
        }
    }
    let headers_expression = if has_headers {
        "headers".to_owned()
    } else {
        "{}.freeze".to_owned()
    };
    let body = match &response.body {
        Some(ConstructedBody::Json { codec, .. }) => {
            format!("Codecs::{codec}.encode_json(result)")
        }
        Some(ConstructedBody::SchemaFreeJson) => "Json.dump(result)".to_owned(),
        None => "''".to_owned(),
    };
    let _ = writeln!(
        out,
        "      [{status}, {headers_expression}, {body}]\n    end\n",
        status = response.status,
    );
}

/// The generated `lib/<require>/incoming.rb` module: the shared receipt
/// library plus the per-receipt compiled helpers and frozen routes. Called
/// only when the plan carries at least one receipt.
pub(super) fn runtime(emission: &IncomingEmission) -> String {
    let mut out = String::from(
        "# frozen_string_literal: true\nmodule __NAMESPACE__\n  # Generated incoming receipt helpers for this package's declared webhooks\n  # and callbacks. HTTP semantics are inverted for these receipts: the\n  # declared method is the verb the provider sends and the declared responses\n  # are what the handler returns. Decoders presence-check the declared\n  # required headers and decode the declared body through the same compiled\n  # model codecs the client uses; constructors build the declared 2xx reply.\n  # The compiled receipt routes below are frozen generated data — the runtime\n  # never parses OpenAPI. Route expressions ({$request...}) are carried\n  # verbatim; their substitution is a runtime/framework concern.\n  module Incoming\n",
    );
    let _ = writeln!(
        out,
        "    SOURCE = 'suspect-incoming'\n"
    );
    for receipt in &emission.receipts {
        route_constant(&mut out, receipt);
    }
    out.push_str(
        "    module_function\n\n    # Reads one declared receipt header case-insensitively; nil when absent.\n    def header_value(headers, name)\n      return nil if headers.nil?\n      headers.each do |key, value|\n        return value if key.instance_of?(String) && key.casecmp(name).zero?\n      end\n      nil\n    end\n\n",
    );
    for receipt in &emission.receipts {
        decoder(&mut out, receipt);
        if let Some(response) = &receipt.response {
            constructor(&mut out, receipt, response);
        }
    }
    out.push_str("  end\nend\n");
    out
}

/// The RBS annotation of one decoded payload.
fn payload_type(receipt: &IncomingReceipt) -> &str {
    match &receipt.payload {
        Payload::None => "nil",
        Payload::SchemaFreeJson => "json_value",
        Payload::Json { type_name, .. } => type_name,
    }
}

/// The generated module's RBS signatures, appended to the package signature
/// file only when the receipt module is emitted.
pub(super) fn signatures(emission: &IncomingEmission) -> String {
    let mut out = String::from(
        "\n  # Generated incoming receipt helpers (lib/incoming.rb compiled from the\n  # declared webhooks and callbacks).\n  module Incoming\n    SOURCE: String\n",
    );
    for receipt in &emission.receipts {
        let _ = writeln!(
            out,
            "    {}: {{method: String, path: String, expression: bool}}\n",
            receipt.route_const,
        );
    }
    for receipt in &emission.receipts {
        let _ = writeln!(
            out,
            "    def self.{decode}: (Hash[String, String] headers, String body) -> {payload}",
            decode = receipt.decode,
            payload = payload_type(receipt),
        );
        if let (Some(construct), Some(response)) = (&receipt.construct, &receipt.response) {
            let mut signature = String::from("(");
            if let Some(body) = &response.body {
                let type_name = match body {
                    ConstructedBody::Json { type_name, .. } => type_name.clone(),
                    ConstructedBody::SchemaFreeJson => "json_value".to_owned(),
                };
                signature.push_str(&type_name);
                signature.push_str(" result");
                if !response.headers.is_empty() {
                    signature.push_str(", ?typed_headers: Hash[String, String]?");
                }
            } else if !response.headers.is_empty() {
                signature.push_str("?typed_headers: Hash[String, String]?");
            }
            signature.push(')');
            let _ = writeln!(
                out,
                "    def self.{construct}: {signature} -> [Integer, Hash[String, String], String]",
                construct = construct,
            );
        }
    }
    out.push_str("  end\n");
    out
}
