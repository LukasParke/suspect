//! Emitted-only incoming receipt helpers for the Swift package.
//!
//! The compiled `http_protocol::IncomingPlan` becomes one generated
//! `Incoming.swift` file: per receipt, a decoded payload type, a decoder for
//! what the provider sends (declared required headers are presence-checked and
//! the body decodes through the package's existing codec machinery), a
//! constructor for the declared exact 2xx reply, the frozen receipt route and
//! the frozen compiled receipt descriptors. HTTP semantics are inverted for
//! these receipts: the declared method is the verb the provider sends and the
//! declared responses are what the handler returns. Static runtime files and
//! the shared planner stay untouched, and plans without any incoming
//! declaration emit no file and no bytes at all, so receipt-less output stays
//! byte-identical.
//!
//! The Swift v1 receipt helpers decode JSON media only (through the compiled
//! model codecs or the package's own JSON value parser); every other declared
//! representation is a source-linked refusal, mirroring the shared planner's
//! no-silent-skip discipline. Route expressions (`{$request...}`) are carried
//! verbatim; their substitution is a runtime/framework concern.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use super::SdkPlan;
use super::models::ModelPlan;
use crate::http_contract::HttpDiagnostic;
use crate::http_protocol as wire;
use crate::http_protocol::{IncomingKind, Representation, ResponseStatus};
use suspect_ir::contract::Contract;

/// How `decode` interprets the received body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Payload {
    /// No declared request body: the decoder validates headers and returns
    /// void.
    None,
    /// JSON through the package's existing model codec.
    Json { ty: String, codec: String },
    /// Schema-free JSON: the decoded value is the parsed JSON value.
    SchemaFreeJson,
}

impl Payload {
    /// The descriptor label for the decode representation.
    fn label(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Json { .. } => "json",
            Self::SchemaFreeJson => "schema-free-json",
        }
    }

    /// The Swift annotation of the decoded payload.
    fn annotation(&self) -> String {
        match self {
            Self::None => "Void".into(),
            Self::Json { ty, .. } => ty.clone(),
            Self::SchemaFreeJson => "JsonValue".into(),
        }
    }
}

/// How `construct` encodes the declared reply body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstructedBody {
    Json { ty: String, codec: String },
    SchemaFreeJson,
}

impl ConstructedBody {
    /// The descriptor label for the reply representation.
    fn label(&self) -> &'static str {
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

/// One incoming operation's emission-ready receipt helpers with reserved,
/// collision-free public Swift names.
#[derive(Debug, Clone)]
pub struct IncomingReceipt {
    /// Webhook key, or `parentOperationId.callbackName`.
    pub name: String,
    pub kind: &'static str,
    /// The verb the provider sends.
    pub method: String,
    pub route: String,
    pub expression: bool,
    pub document: String,
    pub pointer: String,
    /// The rendered `{document}#{pointer}` source identity.
    pub source: String,
    /// The rendered `SourceLocation(...)` initializer of the declaration.
    pub source_constant: String,
    /// The `IncomingDescriptors` member name of the compiled descriptor.
    pub descriptor: String,
    /// The frozen receipt-route constant type.
    pub route_type: String,
    /// The decoded payload type.
    pub payload_type: String,
    /// The allocated decoder function.
    pub decode: String,
    /// The allocated reply constructor function.
    pub construct: Option<String>,
    pub payload: Payload,
    pub required_body: bool,
    pub required_headers: Vec<String>,
    pub response: Option<ConstructedResponse>,
    pub explanations: Vec<String>,
}

fn q(value: &str) -> String {
    super::protocol_metadata::q(value)
}

fn doc(out: &mut String, text: &str, indent: &str) {
    for line in text.lines() {
        let _ = writeln!(out, "{indent}/// {}", super::emit::prose(line));
    }
}

/// Compiles the incoming plan against the native model bindings and the
/// reserved type names. Receipts the v1 helpers cannot decode produce
/// source-linked `sdk-incoming-*` errors instead of silent skips, mirroring
/// the shared planner's refusal discipline.
pub(super) fn prepare(
    contract: &Contract,
    incoming: &wire::IncomingPlan,
    models: &ModelPlan,
    type_names: &mut BTreeSet<String>,
    errors: &mut Vec<HttpDiagnostic>,
) -> Vec<IncomingReceipt> {
    let mut result = Vec::new();
    let mut descriptors = BTreeSet::new();
    for operation in incoming.operations() {
        let source = operation.source().use_site().source().clone();
        let refuse = |errors: &mut Vec<HttpDiagnostic>, code: &'static str, message: &str| {
            errors.push(super::diagnostic(contract, source.clone(), code, message));
        };
        let stem = super::exported(operation.name());
        let payload_type = super::allocate(&format!("{stem}Payload"), type_names);
        let route_type = super::allocate(&format!("{stem}WebhookRoute"), type_names);
        let decode = super::allocate(&format!("decode{stem}Webhook"), type_names);
        let construct = super::allocate(&format!("construct{stem}Response"), type_names);
        let descriptor_key: String = operation
            .name()
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character
                } else {
                    '_'
                }
            })
            .collect();
        let descriptor = super::allocate(&super::member(&descriptor_key), &mut descriptors);
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
                    Some(Representation::Json { codec: Some(codec) }) => {
                        let id = codec.schema().id();
                        let Some(codec_name) = models.codecs.get(id) else {
                            refuse(
                                errors,
                                "sdk-incoming-codec-binding",
                                "the declared incoming payload schema has no compiled model codec",
                            );
                            continue;
                        };
                        Payload::Json {
                            ty: models.ty(id),
                            codec: codec_name.clone(),
                        }
                    }
                    Some(Representation::Json { codec: None }) => Payload::SchemaFreeJson,
                    Some(_) => {
                        refuse(
                            errors,
                            "sdk-incoming-receipt-unrepresentable",
                            "the declared incoming body representation has no v1 receipt decode; the Swift helpers decode declared JSON media only",
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
                    .map(wire::MediaPlan::representation)
                {
                    None => None,
                    Some(Representation::Json { codec: Some(codec) }) => {
                        let id = codec.schema().id();
                        let Some(codec_name) = models.codecs.get(id) else {
                            refuse(
                                errors,
                                "sdk-incoming-codec-binding",
                                "the declared reply schema has no compiled model codec",
                            );
                            continue;
                        };
                        Some(ConstructedBody::Json {
                            ty: models.ty(id),
                            codec: codec_name.clone(),
                        })
                    }
                    Some(Representation::Json { codec: None }) => {
                        Some(ConstructedBody::SchemaFreeJson)
                    }
                    Some(_) => {
                        refuse(
                            errors,
                            "sdk-incoming-response-unrepresentable",
                            "the declared reply representation has no v1 constructor; the Swift helpers encode declared JSON media and body-less replies only",
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
                        "declared reply headers are applied from the optional headers argument with required-presence checks only; v1 performs no schema re-validation on constructed header values".into(),
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
            source_constant: super::protocol_metadata::source(&source),
            descriptor,
            route_type,
            payload_type,
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
        code.push_str("    if body.isEmpty {\n        throw IncomingRequestError(\"the declared required receipt body is absent\")\n    }\n");
    }
    match &receipt.payload {
        Payload::None => code.push_str("    return ()\n"),
        Payload::Json { codec, .. } => {
            let _ = writeln!(
                code,
                "    do {{\n        return try Codecs.{codec}.decode(body)\n    }} catch {{\n        throw IncomingRequestError(\"the received payload does not satisfy its declared schema ({})\", cause: error)\n    }}",
                receipt.source
            );
        }
        Payload::SchemaFreeJson => code.push_str(
            "    do {\n        return try JsonValue.parse(body)\n    } catch {\n        throw IncomingRequestError(\"the received payload is not valid JSON\", cause: error)\n    }\n",
        ),
    }
    code
}

/// One receipt's payload type and frozen route constant.
fn type_and_route(receipt: &IncomingReceipt, out: &mut String) {
    out.push('\n');
    doc(
        out,
        &format!(
            "The decoded payload of the {} {:?} receipt. The provider sends this body; the decoder validates it against the declared schema.",
            receipt.kind, receipt.name
        ),
        "",
    );
    doc(out, &format!("Source: {}.", receipt.source), "");
    doc(out, &receipt.explanations.join(" "), "");
    let _ = writeln!(
        out,
        "public typealias {} = {};\n",
        receipt.payload_type,
        receipt.payload.annotation()
    );
    doc(
        out,
        &format!(
            "The declared receipt route of the {} {:?} receipt. `method` is the verb the provider sends; the route string is carried verbatim from the source declaration. Register a handler at this route in your framework; runtime expression substitution is a framework concern.",
            receipt.kind, receipt.name
        ),
        "",
    );
    doc(out, &format!("Source: {}.", receipt.source), "");
    let _ = writeln!(
        out,
        "public struct {}: Sendable, Equatable {{\n    /// The verb the provider sends.\n    public static let method = {}\n    /// The declared route, carried verbatim from the source declaration.\n    public static let path = {}\n    /// True when the declared route carries RFC 6570 runtime expressions;\n    /// generated code never substitutes one.\n    public static let expression = {}\n}}\n",
        receipt.route_type,
        q(&receipt.method),
        q(&receipt.route),
        receipt.expression,
    );
}

/// One receipt's decoder.
fn decoder(receipt: &IncomingReceipt) -> String {
    let mut code = String::new();
    doc(
        &mut code,
        &format!(
            "Decodes one received {} {:?} request. Declared required headers are presence-checked (a missing one throws ``IncomingRequestError``) and the declared body decodes through the package's own compiled codec machinery.",
            receipt.kind, receipt.name
        ),
        "",
    );
    doc(&mut code, &format!("Source: {}.", receipt.source), "");
    doc(&mut code, &receipt.explanations.join(" "), "");
    let _ = writeln!(
        code,
        "public func {}(headers: [String: String], body: Data) throws -> {} {{",
        receipt.decode, receipt.payload_type
    );
    for header in &receipt.required_headers {
        let _ = writeln!(
            code,
            "    if incomingHeaderValue(headers, {}) == nil {{\n        throw IncomingRequestError(\"required receipt header {} is absent\")\n    }}",
            q(header),
            header.replace('\\', "\\\\").replace('"', "\\\"")
        );
    }
    code.push_str(&decode_body(receipt));
    code.push_str("}\n");
    code
}

/// One receipt's reply constructor, when an exact 2xx response is declared.
fn constructor(receipt: &IncomingReceipt, response: &ConstructedResponse) -> String {
    let Some(construct) = receipt.construct.as_ref() else {
        return String::new();
    };
    let has_body = response.body.is_some();
    let has_headers = !response.headers.is_empty();
    let result_type = match &response.body {
        Some(ConstructedBody::Json { ty, .. }) => ty.clone(),
        Some(ConstructedBody::SchemaFreeJson) => "JsonValue".into(),
        None => String::new(),
    };
    let mut parameters = String::new();
    if has_body {
        parameters.push_str(&format!("result: {result_type}"));
        if has_headers {
            parameters.push_str(", ");
        }
    }
    if has_headers {
        parameters.push_str("headers: [String: String]? = nil");
    }
    let mut code = String::new();
    doc(
        &mut code,
        &format!(
            "Constructs the declared {} reply of the {} {:?} receipt: the returned tuple carries the declared status, the applied declared headers and the encoded body.",
            response.status, receipt.kind, receipt.name
        ),
        "",
    );
    doc(&mut code, &format!("Source: {}.", receipt.source), "");
    doc(&mut code, &receipt.explanations.join(" "), "");
    let _ = writeln!(
        code,
        "public func {construct}({parameters}) throws -> (status: Int, headers: [String: String], body: Data) {{"
    );
    if has_headers {
        let optional: Vec<_> = response
            .headers
            .iter()
            .filter(|header| !response.required_headers.contains(header))
            .cloned()
            .collect();
        code.push_str("    var reply: [String: String] = [:]\n");
        if !optional.is_empty() {
            let _ = writeln!(
                code,
                "    for name in [{}] {{\n        if let value = incomingHeaderValue(headers ?? [:], name) {{\n            reply[name] = value\n        }}\n    }}",
                optional.iter().map(|h| q(h)).collect::<Vec<_>>().join(", ")
            );
        }
        if !response.required_headers.is_empty() {
            let _ = writeln!(
                code,
                "    for name in [{}] {{\n        guard let value = incomingHeaderValue(headers ?? [:], name) else {{\n            throw IncomingRequestError(\"the declared required reply header \\(name) is absent\")\n        }}\n        reply[name] = value\n    }}",
                response
                    .required_headers
                    .iter()
                    .map(|h| q(h))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
    let body = match &response.body {
        Some(ConstructedBody::Json { codec, .. }) => {
            let _ = writeln!(
                code,
                "    let body: Data\n    do {{\n        body = try Codecs.{codec}.encode(result)\n    }} catch {{\n        throw IncomingRequestError(\"the declared reply value does not satisfy its declared schema ({})\", cause: error)\n    }}",
                receipt.source
            );
            "body".to_owned()
        }
        Some(ConstructedBody::SchemaFreeJson) => {
            code.push_str("    let body: Data\n    do {\n        body = try result.encoded()\n    } catch {\n        throw IncomingRequestError(\"the reply value is not representable JSON\", cause: error)\n    }\n");
            "body".to_owned()
        }
        None => "Data()".to_owned(),
    };
    let headers = if has_headers { "reply" } else { "[:]" };
    let _ = writeln!(
        code,
        "    return (status: {}, headers: {headers}, body: {body})\n}}\n",
        response.status,
    );
    code
}

/// The frozen compiled receipt descriptor constants: generated data, never
/// parsed from OpenAPI, sheddable with the receipt helpers themselves.
fn descriptors(receipts: &[IncomingReceipt]) -> String {
    let mut code = String::new();
    doc(
        &mut code,
        "Frozen compiled receipt descriptors for this package: one member per declared webhook/callback receipt. `payload` names the decode representation and `reply` the constructed reply representation (\"json\", \"schema-free-json\" or \"none\"); codecs name the generated `Codecs` members. Generated data, never parsed from OpenAPI.",
        "",
    );
    code.push_str("public enum IncomingDescriptors {\n");
    doc(&mut code, "One compiled receipt descriptor.", "    ");
    code.push_str(
        "    public struct Descriptor: Sendable, Equatable {\n        /// \"webhook\" for a document-level webhooks entry, \"callback\" for a\n        /// receipt attached to an operation.\n        public let kind: String\n        /// The verb the provider sends.\n        public let method: String\n        /// The declared route, carried verbatim from the source declaration.\n        public let route: String\n        /// True when the route carries RFC 6570 runtime expressions.\n        public let expression: Bool\n        /// The receipt declaration's document and JSON pointer.\n        public let source: SourceLocation\n        /// The declared receipt headers the decoder requires present.\n        public let requiredHeaders: [String]\n        /// The decode representation of the received body.\n        public let payload: String\n        /// The `Codecs` member of the payload codec, when one is compiled.\n        public let payloadCodec: String?\n        /// The reply representation of the constructed reply.\n        public let reply: String\n        /// The pinned reply status, when an exact 2xx reply is declared.\n        public let replyStatus: Int?\n        /// The `Codecs` member of the reply codec, when one is compiled.\n        public let replyCodec: String?\n        /// The declared reply header wire names, in declaration order.\n        public let replyHeaders: [String]\n    }\n\n",
    );
    for receipt in receipts {
        let (reply, reply_status, reply_codec, reply_headers) = match &receipt.response {
            None => (
                "none".to_owned(),
                "nil".to_owned(),
                "nil".to_owned(),
                "[]".to_owned(),
            ),
            Some(response) => (
                response
                    .body
                    .as_ref()
                    .map_or_else(|| "none", |body| body.label())
                    .to_owned(),
                response.status.to_string(),
                match &response.body {
                    Some(ConstructedBody::Json { codec, .. }) => format!("\"{codec}\""),
                    _ => "nil".to_owned(),
                },
                format!(
                    "[{}]",
                    response
                        .headers
                        .iter()
                        .map(|header| q(header))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ),
        };
        let payload_codec = match &receipt.payload {
            Payload::Json { codec, .. } => format!("\"{codec}\""),
            _ => "nil".to_owned(),
        };
        doc(
            &mut code,
            &format!(
                "The compiled descriptor of the {} {:?} receipt.",
                receipt.kind, receipt.name
            ),
            "    ",
        );
        let _ = writeln!(
            code,
            "    public static let {} = Descriptor(\n        kind: {}, method: {}, route: {}, expression: {},\n        source: {},\n        requiredHeaders: [{}], payload: {}, payloadCodec: {},\n        reply: {}, replyStatus: {}, replyCodec: {}, replyHeaders: {})",
            receipt.descriptor,
            q(receipt.kind),
            q(&receipt.method),
            q(&receipt.route),
            receipt.expression,
            receipt.source_constant,
            receipt
                .required_headers
                .iter()
                .map(|header| q(header))
                .collect::<Vec<_>>()
                .join(", "),
            q(Payload::label(&receipt.payload)),
            payload_codec,
            q(&reply),
            reply_status,
            reply_codec,
            reply_headers,
        );
    }
    code.push_str("}\n");
    code
}

/// Render the generated `Incoming.swift` module: the branded failure, the
/// shared header lookup, the per-receipt compiled helpers and the frozen
/// descriptor constants. `None` keeps the package free of any incoming byte.
pub(super) fn emit(plan: &SdkPlan) -> Option<String> {
    let receipts = plan.incoming.as_slice();
    if receipts.is_empty() {
        return None;
    }
    let mut out = String::from("import Foundation\n\n");
    out.push_str(
        "/// Generated incoming receipt helpers for this package's declared webhooks\n/// and callbacks. HTTP semantics are inverted for these receipts: the declared\n/// method is the verb the provider sends and the declared responses are what\n/// the handler returns. Decoders presence-check the declared required headers\n/// and decode the declared body through the same compiled model codecs the\n/// client uses; constructors build the declared 2xx reply. The compiled receipt\n/// descriptors below are frozen generated data — the runtime never parses\n/// OpenAPI. Route expressions ({$request...}) are carried verbatim; their\n/// substitution is a runtime/framework concern.\n\n",
    );
    out.push_str(
        "/// Branded failure for a received receipt that violates its declared headers\n/// or body, or a declared reply that cannot be constructed. The cause carries\n/// the underlying codec failure; descriptions stay bounded and never carry\n/// received payload text.\npublic struct IncomingRequestError: Error, Sendable, CustomStringConvertible {\n    public let message: String\n    public let cause: (any Error)?\n    public var description: String { message }\n\n    init(_ message: String, cause: (any Error)? = nil) {\n        self.message = message\n        self.cause = cause\n    }\n}\n\n/// Reads one declared receipt header case-insensitively.\nfunc incomingHeaderValue(_ headers: [String: String], _ name: String) -> String? {\n    for (key, value) in headers where key.caseInsensitiveCompare(name) == .orderedSame {\n        return value\n    }\n    return nil\n}\n",
    );
    for receipt in receipts {
        out.push_str(&format!(
            "\n// ---- {} {} — {} ----\n",
            super::emit::prose(receipt.method.as_str()),
            super::emit::prose(&receipt.route),
            super::emit::prose(receipt.kind),
        ));
        type_and_route(receipt, &mut out);
        out.push_str(&decoder(receipt));
        if let Some(response) = &receipt.response {
            out.push_str(&constructor(receipt, response));
        }
    }
    out.push_str(&descriptors(receipts));
    Some(out)
}
