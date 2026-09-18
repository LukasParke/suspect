//! Emitted-only incoming receipt helpers for the Go HTTP adapter.
//!
//! The compiled `http_protocol::IncomingPlan` becomes one generated
//! `go/incoming.go` file: per receipt, a typed decoder for what the provider
//! sends (declared required headers are presence-checked and the body decodes
//! through the package's existing codec machinery), a constructor for the
//! declared exact 2xx reply, the frozen receipt route and the compiled
//! descriptors as generated package-level data. HTTP semantics are inverted
//! for these receipts: the declared method is the verb the provider sends and
//! the declared responses are what the handler returns. Static runtime files
//! are never modified, and plans without any incoming declaration emit no file
//! and no bytes at all. Only JSON receipts are representable in the v1 Go
//! helpers; every other representation is refused with the shared
//! `sdk-incoming-*` plan errors instead of being skipped.

use std::collections::BTreeSet;

use super::emit::q;
use super::*;
use crate::http_contract::HttpDiagnostic;
use crate::http_protocol as protocol;
use crate::http_protocol::{IncomingKind, IncomingPlan, ResponseStatus};
use suspect_ir::contract::{Contract, SchemaId};

/// Package-level names the emitted `go/incoming.go` owns. Reserved beside the
/// model namespace only when receipts are declared, so receipt-less plans
/// reserve nothing.
pub(super) const PACKAGE_NAMES: &[&str] =
    &["IncomingRoute", "IncomingDescriptor", "IncomingDescriptors"];

/// How `Decode*Webhook` interprets the received body. Only JSON is
/// representable in the Go v1 receipt helpers; every other representation is
/// refused at plan time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Payload {
    /// No declared request body: the decoder validates headers and returns nil.
    None,
    /// JSON through the package's own compiled model codec.
    Json { codec: String },
    /// Schema-free JSON: the decoded value is the parsed JSON value.
    SchemaFreeJson,
}

impl Payload {
    /// The descriptor label for the decode representation.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Json { .. } => "json",
            Self::SchemaFreeJson => "schema-free-json",
        }
    }

    /// The compiled model codec of the payload, empty when it has none.
    #[must_use]
    pub fn codec(&self) -> Option<&str> {
        match self {
            Self::Json { codec } => Some(codec),
            Self::None | Self::SchemaFreeJson => None,
        }
    }
}

/// How `Construct*Response` encodes the declared reply body. Only JSON is
/// representable in the Go v1 receipt helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstructedBody {
    /// JSON through the package's own compiled model codec.
    Json { codec: String },
    /// Schema-free JSON: the reply value is encoded as the JSON value.
    SchemaFreeJson,
}

impl ConstructedBody {
    /// The descriptor label for the reply representation.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Json { .. } => "json",
            Self::SchemaFreeJson => "schema-free-json",
        }
    }

    /// The compiled model codec of the reply, empty when it has none.
    #[must_use]
    pub fn codec(&self) -> Option<&str> {
        match self {
            Self::Json { codec } => Some(codec),
            Self::SchemaFreeJson => None,
        }
    }
}

/// One declared reply the construct helper builds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstructedResponse {
    /// The declared exact 2xx status the handler returns.
    pub status: u16,
    pub body: Option<ConstructedBody>,
    /// Declared reply header wire names, in declaration order.
    pub headers: Vec<String>,
    pub required_headers: Vec<String>,
}

/// One incoming operation's emission-ready receipt helpers with allocated,
/// collision-free Go names.
#[derive(Debug, Clone)]
pub struct IncomingReceipt {
    /// Webhook key, or `parentOperationId.callbackName`.
    pub name: String,
    pub kind: &'static str,
    /// The verb the provider sends.
    pub method: String,
    pub route: String,
    pub expression: bool,
    /// The receipt declaration's use site.
    pub source: SourceId,
    /// The compiled descriptor table key; the declared receipt name, which the
    /// shared planner guarantees unique.
    pub descriptor_key: String,
    pub route_const: String,
    pub payload_type: String,
    pub source_const: String,
    pub decode: String,
    pub construct: Option<String>,
    pub payload: Payload,
    pub required_body: bool,
    pub required_headers: Vec<String>,
    pub response: Option<ConstructedResponse>,
    pub explanations: Vec<String>,
}

/// Compiles the incoming plan against the compiled model codecs, allocating
/// every emitted Go name into `names`. Receipts the v1 helpers cannot express
/// produce source-linked `sdk-incoming-*` errors instead of silent skips.
pub(super) fn prepare(
    contract: &Contract,
    plan: &IncomingPlan,
    symbols: &BTreeMap<SchemaId, String>,
    names: &mut BTreeSet<String>,
    errors: &mut Vec<HttpDiagnostic>,
) -> Vec<IncomingReceipt> {
    let mut result = Vec::new();
    for operation in plan.operations() {
        let source = operation.source().use_site().source().clone();
        let refuse = |errors: &mut Vec<HttpDiagnostic>, code: &'static str, message: &str| {
            errors.push(super::diagnostic(contract, source.clone(), code, message));
        };
        let stem = exported(operation.name());
        let descriptor_key = operation.name().clone();
        let payload_type = allocate(&format!("{stem}Payload"), names);
        let route_const = allocate(&format!("{stem}WebhookRoute"), names);
        let source_const = allocate(&format!("incoming{stem}Source"), names);
        let decode = allocate(&format!("Decode{stem}Webhook"), names);
        let construct = allocate(&format!("Construct{stem}Response"), names);
        let required_headers = operation
            .request()
            .parameters()
            .iter()
            .filter(|parameter| {
                parameter.required() && parameter.location() == protocol::ParameterLocation::Header
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
                match body
                    .media()
                    .first()
                    .map(protocol::MediaPlan::representation)
                {
                    None => Payload::None,
                    Some(protocol::Representation::Json { codec: Some(codec) }) => {
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
                    Some(protocol::Representation::Json { codec: None }) => Payload::SchemaFreeJson,
                    Some(_) => {
                        refuse(
                            errors,
                            "sdk-incoming-receipt-unrepresentable",
                            "the declared incoming body representation has no Go receipt decode; only JSON receipts are emitted and form, multipart, stream, text and binary receipts are refused",
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
                let body = match response
                    .media()
                    .first()
                    .map(protocol::MediaPlan::representation)
                {
                    None => None,
                    Some(protocol::Representation::Json { codec: Some(codec) }) => {
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
                    Some(protocol::Representation::Json { codec: None }) => {
                        Some(ConstructedBody::SchemaFreeJson)
                    }
                    Some(_) => {
                        refuse(
                            errors,
                            "sdk-incoming-response-unrepresentable",
                            "the declared reply representation has no Go constructor; only JSON and body-less replies are emitted",
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
                        "declared reply headers are applied from the optional typedHeaders argument with required-presence checks only; v1 performs no schema re-validation on constructed header values".into(),
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
            source,
            descriptor_key,
            route_const,
            payload_type,
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

fn prose(text: &str) -> String {
    text.replace(['\n', '\r'], " ").replace("*/", "* /")
}

/// The documentation comment of one emitted declaration: the summary lines,
/// then the value-reducing v1 explanations as a second paragraph.
fn comment(summary: &str, receipt: &IncomingReceipt) -> String {
    let mut code = String::new();
    for line in summary.split('\n') {
        code.push_str("// ");
        code.push_str(line.trim_end());
        code.push('\n');
    }
    if !receipt.explanations.is_empty() {
        code.push_str("//\n// ");
        code.push_str(&prose(&receipt.explanations.join(" ")));
        code.push('\n');
    }
    code
}

/// One receipt's payload type alias, route constant and source locator.
fn type_and_route(receipt: &IncomingReceipt) -> String {
    let payload_summary = match &receipt.payload {
        Payload::None => format!(
            "{} is the decoded payload of the {} {:?} receipt. The receipt\ndeclares no request body; the decoder validates the declared headers and\nreturns nil.",
            receipt.payload_type, receipt.kind, receipt.name
        ),
        Payload::Json { .. } | Payload::SchemaFreeJson => format!(
            "{} is the decoded payload of the {} {:?} receipt. The provider\nsends this body; the decoder validates it against the declared schema.",
            receipt.payload_type, receipt.kind, receipt.name
        ),
    };
    let mut code = comment(&payload_summary, receipt);
    code.push_str(&match &receipt.payload {
        Payload::None => format!("type {name} = struct{{}}\n\n", name = receipt.payload_type),
        Payload::Json { codec } => format!(
            "type {name} = {codec}\n\n",
            name = receipt.payload_type,
            codec = codec
        ),
        Payload::SchemaFreeJson => format!("type {name} = Value\n\n", name = receipt.payload_type),
    });
    code.push_str(&comment(
        &format!(
            "{} is the declared receipt route of the {} {:?} receipt.\nMethod is the verb the provider sends; register a handler for the route in\nyour framework. The route string is carried verbatim from the source\ndeclaration and its runtime expressions are never substituted at generation\ntime.",
            receipt.route_const, receipt.kind, receipt.name
        ),
        receipt,
    ));
    code.push_str(&format!(
        "var {route_const} = IncomingRoute{{Method: {method}, Path: {route}, Expression: {expression}}}\n\n",
        route_const = receipt.route_const,
        method = q(&receipt.method),
        route = q(&receipt.route),
        expression = receipt.expression,
    ));
    code.push_str(&comment(
        &format!(
            "{} locates the {} {:?} declaration.",
            receipt.source_const, receipt.kind, receipt.name
        ),
        receipt,
    ));
    code.push_str(&format!(
        "var {source_const} = HTTPSource{{Document: {document}, Pointer: {pointer}}}\n\n",
        source_const = receipt.source_const,
        document = q(receipt.source.document().as_str()),
        pointer = q(receipt.source.pointer()),
    ));
    code
}

/// The declared body statements of one decoder.
fn decode_body(receipt: &IncomingReceipt, limit: usize) -> String {
    let source = &receipt.source_const;
    let mut code = String::new();
    if receipt.required_body {
        code.push_str(&format!(
            "\tif len(body) == 0 {{\n\t\treturn nil, httpIncomingError({source}, errors.New(\"the declared required receipt body is absent\"))\n\t}}\n",
        ));
    }
    match &receipt.payload {
        Payload::None => code.push_str("\treturn nil, nil\n"),
        Payload::Json { codec } => code.push_str(&format!(
            "\tif err := httpIncomingBodyText(body, {source}); err != nil {{\n\t\treturn nil, err\n\t}}\n\tvalue, err := httpJSONParse(body, {limit})\n\tif err != nil {{\n\t\treturn nil, httpIncomingError({source}, err)\n\t}}\n\tmodel, err := Codecs.{codec}.DecodeValue(value)\n\tif err != nil {{\n\t\treturn nil, httpIncomingError({source}, err)\n\t}}\n\treturn &model, nil\n",
            source = source,
            limit = limit,
            codec = codec,
        )),
        Payload::SchemaFreeJson => code.push_str(&format!(
            "\tif err := httpIncomingBodyText(body, {source}); err != nil {{\n\t\treturn nil, err\n\t}}\n\tvalue, err := httpJSONParse(body, {limit})\n\tif err != nil {{\n\t\treturn nil, httpIncomingError({source}, err)\n\t}}\n\treturn &value, nil\n",
            source = source,
            limit = limit,
        )),
    }
    code
}

/// One receipt's decoder.
fn decoder(receipt: &IncomingReceipt, limit: usize) -> String {
    let mut code = comment(
        &format!(
            "{} decodes one received {} {:?} request. Declared required\nheaders are presence-checked (a missing one fails with the\n\"incoming-request\" SDK error kind) and the declared body decodes through\nthe package's own compiled codec machinery.",
            receipt.decode, receipt.kind, receipt.name
        ),
        receipt,
    );
    code.push_str(&format!(
        "func {decode}(headers map[string]string, body []byte) (*{payload_type}, error) {{\n",
        decode = receipt.decode,
        payload_type = receipt.payload_type,
    ));
    if !receipt.required_headers.is_empty() {
        code.push_str(&format!(
            "\tif err := httpIncomingRequireHeaders(headers, []string{{{names}}}, {source}); err != nil {{\n\t\treturn nil, err\n\t}}\n",
            names = receipt
                .required_headers
                .iter()
                .map(|header| q(header))
                .collect::<Vec<_>>()
                .join(", "),
            source = receipt.source_const,
        ));
    }
    code.push_str(&decode_body(receipt, limit));
    code.push_str("}\n\n");
    code
}

/// One receipt's reply constructor, when an exact 2xx response is declared.
fn constructor(receipt: &IncomingReceipt, response: &ConstructedResponse, limit: usize) -> String {
    let Some(construct) = &receipt.construct else {
        return String::new();
    };
    let has_body = response.body.is_some();
    let has_headers = !response.headers.is_empty();
    let result_type = match &response.body {
        Some(ConstructedBody::Json { codec }) => codec.clone(),
        Some(ConstructedBody::SchemaFreeJson) => "Value".into(),
        None => String::new(),
    };
    let mut parameters = String::new();
    if has_body {
        parameters.push_str(&format!("result {result_type}"));
        if has_headers {
            parameters.push_str(", ");
        }
    }
    if has_headers {
        parameters.push_str("typedHeaders ...map[string]string");
    }
    let mut code = comment(
        &format!(
            "{construct} constructs the declared {} reply of the {} {:?}\nreceipt: the returned record carries the declared status, the encoded body\nand the applied declared headers.",
            response.status, receipt.kind, receipt.name
        ),
        receipt,
    );
    code.push_str(&format!(
        "func {construct}({parameters}) (int, map[string]string, []byte, error) {{\n",
    ));
    if has_headers {
        code.push_str(
            "\tvar typed map[string]string\n\tif len(typedHeaders) > 0 {\n\t\ttyped = typedHeaders[0]\n\t}\n",
        );
        let optional = response
            .headers
            .iter()
            .filter(|header| !response.required_headers.contains(header))
            .collect::<Vec<_>>();
        code.push_str("\treplyHeaders := map[string]string{}\n");
        if !optional.is_empty() {
            code.push_str(&format!(
                "\tfor _, name := range []string{{{names}}} {{\n\t\tif value, ok := httpIncomingHeaderValue(typed, name); ok {{\n\t\t\treplyHeaders[name] = value\n\t\t}}\n\t}}\n",
                names = optional
                    .iter()
                    .map(|header| q(header))
                    .collect::<Vec<_>>()
                    .join(", "),
            ));
        }
        if !response.required_headers.is_empty() {
            code.push_str(&format!(
                "\tfor _, name := range []string{{{names}}} {{\n\t\tvalue, ok := httpIncomingHeaderValue(typed, name)\n\t\tif !ok {{\n\t\t\treturn 0, nil, nil, httpIncomingError({source}, errors.New(\"the declared required reply header \"+name+\" is absent\"))\n\t\t}}\n\t\treplyHeaders[name] = value\n\t}}\n",
                names = response
                    .required_headers
                    .iter()
                    .map(|header| q(header))
                    .collect::<Vec<_>>()
                    .join(", "),
                source = receipt.source_const,
            ));
        }
    }
    if has_body {
        match &response.body {
            Some(ConstructedBody::Json { codec }) => code.push_str(&format!(
                "\tdata, err := Codecs.{codec}.Encode(result)\n\tif err != nil {{\n\t\treturn 0, nil, nil, httpIncomingError({source}, err)\n\t}}\n",
                codec = codec,
                source = receipt.source_const,
            )),
            Some(ConstructedBody::SchemaFreeJson) => code.push_str(&format!(
                "\tdata, err := httpJSONEncode(result, {limit})\n\tif err != nil {{\n\t\treturn 0, nil, nil, httpIncomingError({source}, err)\n\t}}\n",
                limit = limit,
                source = receipt.source_const,
            )),
            None => unreachable!("a constructed reply body exists"),
        }
    }
    let headers_expression = if has_headers {
        "replyHeaders".to_owned()
    } else {
        "map[string]string{}".to_owned()
    };
    let body_expression = if has_body { "data" } else { "nil" };
    code.push_str(&format!(
        "\treturn {status}, {headers}, {body}, nil\n}}\n\n",
        status = response.status,
        headers = headers_expression,
        body = body_expression,
    ));
    code
}

/// The compiled receipt descriptor table: generated data, never parsed from
/// OpenAPI.
fn descriptor_map(receipts: &[IncomingReceipt]) -> String {
    let mut code = String::from(
        "// IncomingDescriptors is the compiled receipt descriptor table: one entry\n// per declared webhook/callback receipt, keyed by the declared receipt name.\n",
    );
    code.push_str("var IncomingDescriptors = map[string]IncomingDescriptor{\n");
    for receipt in receipts {
        let (reply, reply_status, reply_codec, reply_headers) = match &receipt.response {
            None => (
                "none".to_owned(),
                "0".to_owned(),
                String::new(),
                "nil".to_owned(),
            ),
            Some(response) => (
                response
                    .body
                    .as_ref()
                    .map_or_else(|| "none", |body| body.label())
                    .to_owned(),
                response.status.to_string(),
                response
                    .body
                    .as_ref()
                    .and_then(ConstructedBody::codec)
                    .unwrap_or_default()
                    .to_owned(),
                if response.headers.is_empty() {
                    "nil".to_owned()
                } else {
                    format!(
                        "[]string{{{}}}",
                        response
                            .headers
                            .iter()
                            .map(|header| q(header))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                },
            ),
        };
        let payload_codec = receipt.payload.codec().unwrap_or_default();
        code.push_str(&format!(
            "\t{key}: {{\n\t\tKind: {kind},\n\t\tMethod: {method},\n\t\tRoute: {route},\n\t\tExpression: {expression},\n\t\tSource: HTTPSource{{Document: {document}, Pointer: {pointer}}},\n\t\tRequiredHeaders: {required_headers},\n\t\tPayload: {payload},\n\t\tPayloadCodec: {payload_codec},\n\t\tReply: {reply},\n\t\tReplyStatus: {reply_status},\n\t\tReplyCodec: {reply_codec},\n\t\tReplyHeaders: {reply_headers},\n\t}},\n",
            key = q(&receipt.descriptor_key),
            kind = q(receipt.kind),
            method = q(&receipt.method),
            route = q(&receipt.route),
            expression = receipt.expression,
            document = q(receipt.source.document().as_str()),
            pointer = q(receipt.source.pointer()),
            required_headers = if receipt.required_headers.is_empty() {
                "nil".to_owned()
            } else {
                format!(
                    "[]string{{{}}}",
                    receipt
                        .required_headers
                        .iter()
                        .map(|header| q(header))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            },
            payload = q(receipt.payload.label()),
            payload_codec = q(payload_codec),
            reply = q(&reply),
            reply_status = reply_status,
            reply_codec = q(&reply_codec),
            reply_headers = reply_headers,
        ));
    }
    code.push_str("}\n");
    code
}

/// Render `go/incoming.go`: the shared receipt library plus one typed decode,
/// construct and route per compiled receipt, and the frozen descriptor table.
/// Called only when at least one receipt compiles.
pub(super) fn emit(plan: &HttpPlan, receipts: &[IncomingReceipt]) -> String {
    let mut code = String::from(LIBRARY);
    for receipt in receipts {
        code.push_str(&type_and_route(receipt));
        code.push_str(&decoder(receipt, plan.config.max_request_bytes));
        if let Some(response) = &receipt.response {
            code.push_str(&constructor(
                receipt,
                response,
                plan.config.max_response_bytes,
            ));
        }
    }
    code.push_str(&descriptor_map(receipts));
    code
}

/// The static library half of the generated file: the shared route/descriptor
/// types, the branded failure constructor and the header/body helpers. Their
/// bodies reference every import the file carries, so the import block is
/// fixed regardless of which receipt kinds compile.
const LIBRARY: &str = r#"// Code generated by suspect. DO NOT EDIT.
//
// Generated incoming receipt helpers for this package's declared webhooks and
// callbacks. HTTP semantics are inverted for these receipts: the declared
// method is the verb the provider sends and the declared responses are what
// the handler returns. Decoders presence-check the declared required headers
// and decode the declared body through the same compiled model codecs the
// client uses; constructors build the declared exact 2xx reply. The compiled
// receipt descriptors below are generated data - the runtime never parses
// OpenAPI. Route expressions ({$request...}) are carried verbatim; their
// substitution is a runtime/framework concern.
package sdk

import (
	"errors"
	"strings"
	"unicode/utf8"
)

// IncomingRoute is the declared receipt route of one webhook or callback
// receipt. Register a handler for it in your framework: Method is the verb the
// provider sends, and runtime expression substitution (see Expression) is a
// runtime/framework concern, never a generation-time rewrite.
type IncomingRoute struct {
	// Method is the verb the provider sends.
	Method string
	// Path is the declared route string, carried verbatim from the source
	// declaration.
	Path string
	// Expression is true when the declared route carries RFC 6570 runtime
	// expressions.
	Expression bool
}

// IncomingDescriptor is the compiled descriptor of one declared webhook or
// callback receipt. Payload names the decode representation and Reply the
// constructed reply representation ("json", "schema-free-json" or "none");
// codecs name the generated model codec exports. Every field is generated
// data - the runtime never parses OpenAPI.
type IncomingDescriptor struct {
	// Kind is "webhook" for a document-level webhooks entry and "callback"
	// for an operation-attached callbacks entry.
	Kind string
	// Method is the verb the provider sends.
	Method string
	// Route is the declared route, carried verbatim from the source
	// declaration.
	Route string
	// Expression is true when the route carries RFC 6570 runtime expressions.
	Expression bool
	// Source locates the receipt declaration in the source documents.
	Source HTTPSource
	// RequiredHeaders are the received headers whose presence the decoder
	// checks.
	RequiredHeaders []string
	// Payload names the decode representation: "json" through a compiled
	// model codec, "schema-free-json" as the parsed JSON value, or "none"
	// when no request body is declared.
	Payload string
	// PayloadCodec names the generated model codec of the payload; empty when
	// the representation has none.
	PayloadCodec string
	// Reply names the constructed reply representation: "json",
	// "schema-free-json" or "none" for a body-less reply.
	Reply string
	// ReplyStatus is the declared exact reply status; 0 when no exact 2xx
	// response is declared, in which case no constructor is emitted.
	ReplyStatus int
	// ReplyCodec names the generated model codec of the reply; empty when the
	// representation has none.
	ReplyCodec string
	// ReplyHeaders are the declared reply header wire names, in declaration
	// order; the constructor applies them with required-presence checks.
	ReplyHeaders []string
}

// httpIncomingError brands one receipt decode or construct failure.
func httpIncomingError(at HTTPSource, cause error) *SDKError {
	return &SDKError{Kind: "incoming-request", Operation: at, Source: at, Cause: cause}
}

// httpIncomingHeaderValue reads one declared receipt header case-insensitively.
func httpIncomingHeaderValue(headers map[string]string, name string) (string, bool) {
	for key, value := range headers {
		if strings.EqualFold(key, name) {
			return value, true
		}
	}
	return "", false
}

// httpIncomingRequireHeaders brands the failure when a required declared
// receipt header is absent.
func httpIncomingRequireHeaders(headers map[string]string, required []string, at HTTPSource) error {
	for _, name := range required {
		if _, ok := httpIncomingHeaderValue(headers, name); !ok {
			return httpIncomingError(at, errors.New("required receipt header "+name+" is absent"))
		}
	}
	return nil
}

// httpIncomingBodyText brands the failure when the received JSON payload is
// not valid UTF-8.
func httpIncomingBodyText(body []byte, at HTTPSource) error {
	if !utf8.Valid(body) {
		return httpIncomingError(at, errors.New("the received payload is not valid UTF-8"))
	}
	return nil
}

"#;
