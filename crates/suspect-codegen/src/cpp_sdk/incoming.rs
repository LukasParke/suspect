//! Emitted-only incoming receipt helpers for the native C++ client.
//!
//! The compiled, generation-time `http_protocol::plan_incoming` outcome lowers
//! into one generated, header-only `include/<package>/incoming.hpp`: per
//! receipt, the decoded payload alias, the frozen receipt route constant, a
//! decoder for what the provider sends (declared required headers are
//! presence-checked and the body decodes through the package's own compiled
//! codec machinery) plus a constructor for the declared exact 2xx reply, and
//! the frozen compiled receipt descriptor table. The branded
//! `IncomingRequestError` follows the generated `OAuthError` pattern because
//! the static runtime's error vocabulary is closed. HTTP semantics are
//! inverted for these receipts: the declared method is the verb the provider
//! sends and the declared responses are what the handler returns. Route
//! expressions (`{$request...}`) are carried verbatim; their substitution is a
//! runtime/framework concern.
//!
//! Emission is strictly conditional: plans without any incoming declaration
//! lower to nothing, so no file is emitted and the whole package stays
//! byte-identical. Receipts the v1 helpers cannot represent produce
//! source-linked `sdk-incoming-*` plan errors instead of silent skips, and
//! only JSON receipts are emitted — form, multipart, stream, text and binary
//! receipts are refused.

use std::collections::BTreeSet;

use super::emit::string;
use super::models::{ModelPlan, allocate, pascal, snake};
use super::{HttpDiagnostic, SdkPlan, diagnostic};
use crate::http_protocol::{IncomingKind, IncomingPlan, Representation, ResponseStatus};
use suspect_ir::contract::{Contract, SourceId};

/// How `decode_<name>_webhook` interprets the received body. JSON media only:
/// the v1 receipt helpers decode JSON and schema-free JSON payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Payload {
    /// No declared request body: the decoder checks the declared headers and
    /// returns no payload.
    None,
    /// JSON through the receipt's own compiled model codec.
    Json {
        cpp_type: String,
        codec_name: String,
    },
    /// Schema-free JSON: the decoded value is the parsed JSON value.
    SchemaFreeJson,
}

impl Payload {
    /// The descriptor label for the decode representation.
    pub fn label(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Json { .. } => "json",
            Self::SchemaFreeJson => "schema-free-json",
        }
    }
}

/// How `construct_<name>_response` encodes the declared reply body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstructedBody {
    /// JSON through the receipt's own compiled model codec.
    Json {
        cpp_type: String,
        codec_name: String,
    },
    /// Schema-free JSON: the reply value is encoded as bounded JSON.
    SchemaFreeJson,
}

impl ConstructedBody {
    /// The descriptor label for the reply representation.
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
    /// The declared exact 2xx status.
    pub status: u16,
    pub body: Option<ConstructedBody>,
    /// Declared reply header wire names, in declaration order.
    pub headers: Vec<String>,
    /// The subset that must be supplied by the caller.
    pub required_headers: Vec<String>,
}

/// One incoming operation's emission-ready receipt helpers with allocated,
/// collision-free native names.
#[derive(Debug, Clone)]
pub struct IncomingReceipt {
    /// Webhook key, or `parentOperationId.callbackName`.
    pub name: String,
    pub kind: &'static str,
    /// The verb the provider sends.
    pub method: String,
    pub route: String,
    pub expression: bool,
    pub source: SourceId,
    /// The allocated, collision-free descriptor-table key.
    pub descriptor_key: String,
    pub route_const: String,
    pub payload_alias: String,
    pub decode: String,
    pub construct: Option<String>,
    pub payload: Payload,
    pub required_body: bool,
    pub required_headers: Vec<String>,
    pub response: Option<ConstructedResponse>,
    pub explanations: Vec<String>,
}

/// The compiled incoming emission carried by one plan.
#[derive(Debug, Clone)]
pub struct IncomingEmission {
    /// The shared compiled incoming selection this emission follows.
    pub plan: IncomingPlan,
    pub receipts: Vec<IncomingReceipt>,
}

/// Lower the compiled incoming plan into its emission-ready receipts.
/// Receipts the v1 helpers cannot decode produce source-linked
/// `sdk-incoming-*` errors instead of silent skips.
pub(super) fn lower(
    contract: &Contract,
    plan: &IncomingPlan,
    models: &ModelPlan,
    names: &mut BTreeSet<String>,
) -> Result<IncomingEmission, Vec<HttpDiagnostic>> {
    // The fixed surface names join the allocation only when receipts are
    // emitted, so no-policy packages reserve nothing.
    for fixed in [
        "ConstructedResponse",
        "IncomingRequestError",
        "IncomingRoute",
        "incoming_header_is",
        "incoming_header_value",
        "incoming_reply_header",
        "incoming_descriptors",
        "incoming_descriptor",
    ] {
        names.insert(fixed.to_owned());
    }
    let mut receipts = Vec::new();
    let mut errors: Vec<HttpDiagnostic> = Vec::new();
    for operation in plan.operations() {
        let source = operation.source().use_site().source().clone();
        let refuse = |errors: &mut Vec<HttpDiagnostic>, code: &'static str, message: &str| {
            errors.push(diagnostic(contract, source.clone(), code, message));
        };
        let stem = pascal(operation.name());
        let snake = snake(operation.name());
        let descriptor_key = allocate(
            &operation
                .name()
                .chars()
                .map(|character| if character.is_ascii_alphanumeric() { character } else { '_' })
                .collect::<String>()
                .to_ascii_lowercase(),
            names,
        );
        let payload_alias = allocate(&format!("{stem}Payload"), names);
        let route_const = allocate(&format!("{snake}_webhook_route"), names);
        let decode = allocate(&format!("decode_{snake}_webhook"), names);
        let construct = allocate(&format!("construct_{snake}_response"), names);
        let required_headers = operation
            .request()
            .parameters()
            .iter()
            .filter(|parameter| {
                parameter.required()
                    && parameter.location() == crate::http_protocol::ParameterLocation::Header
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
                match body.media().first().map(crate::http_protocol::MediaPlan::representation)
                {
                    None => Payload::None,
                    Some(Representation::Json { codec: Some(codec) }) => {
                        let Some(symbol) = models.symbol(codec.schema().id()) else {
                            refuse(
                                &mut errors,
                                "sdk-incoming-codec-binding",
                                "the declared incoming payload schema has no compiled model codec",
                            );
                            continue;
                        };
                        Payload::Json {
                            cpp_type: symbol.cpp_type.clone(),
                            codec_name: symbol.codec_name.clone(),
                        }
                    }
                    Some(Representation::Json { codec: None }) => Payload::SchemaFreeJson,
                    Some(_) => {
                        refuse(
                            &mut errors,
                            "sdk-incoming-receipt-unrepresentable",
                            "the declared incoming body representation has no C++ receipt decode; only JSON receipts are emitted and form, multipart, stream, text and binary receipts are refused",
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
                let body = match response.media().first().map(crate::http_protocol::MediaPlan::representation)
                {
                    None => None,
                    Some(Representation::Json { codec: Some(codec) }) => {
                        let Some(symbol) = models.symbol(codec.schema().id()) else {
                            refuse(
                                &mut errors,
                                "sdk-incoming-codec-binding",
                                "the declared reply schema has no compiled model codec",
                            );
                            continue;
                        };
                        Some(ConstructedBody::Json {
                            cpp_type: symbol.cpp_type.clone(),
                            codec_name: symbol.codec_name.clone(),
                        })
                    }
                    Some(Representation::Json { codec: None }) => Some(ConstructedBody::SchemaFreeJson),
                    Some(_) => {
                        refuse(
                            &mut errors,
                            "sdk-incoming-response-unrepresentable",
                            "the declared reply representation has no C++ constructor; only JSON and body-less replies are emitted",
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
        receipts.push(IncomingReceipt {
            name: operation.name().clone(),
            kind: match operation.kind() {
                IncomingKind::Webhook => "webhook",
                IncomingKind::Callback => "callback",
            },
            method: operation.method().as_str().to_owned(),
            route: operation.route().route().clone(),
            expression: operation.route().expression(),
            source: source.clone(),
            descriptor_key,
            route_const,
            payload_alias,
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
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(IncomingEmission {
        plan: plan.clone(),
        receipts,
    })
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// A `std::string_view` literal with an exact byte length; three-digit octal
/// escapes cannot absorb adjacent digits, and embedded NUL and hostile source
/// prose are data.
fn sv(value: &str) -> String {
    let mut literal = String::new();
    for byte in value.bytes() {
        match byte {
            b'"' => literal.push_str("\\\""),
            b'\\' => literal.push_str("\\\\"),
            32..=126 => literal.push(byte as char),
            byte => literal.push_str(&format!("\\{byte:03o}")),
        }
    }
    format!("std::string_view(\"{literal}\", {})", value.len())
}

/// The `/// Source:` comment lines of one emitted declaration, with the
/// value-reducing v1 explanations recorded as prose.
fn comments(receipt: &IncomingReceipt) -> String {
    let mut out = format!(
        "/// Source: {}#{}\n",
        super::emit::single_line(&receipt.source.document().to_string()),
        super::emit::single_line(receipt.source.pointer())
    );
    for explanation in &receipt.explanations {
        out.push_str(&format!("/// {}\n", super::emit::prose(explanation)));
    }
    out
}

/// One receipt's payload alias and frozen route constant.
fn type_and_route(receipt: &IncomingReceipt) -> String {
    let kind = receipt.kind;
    let name = super::emit::prose(&receipt.name);
    let mut out = format!(
        "/// The decoded payload of the {kind} {name}: the provider sends this body.\n",
    );
    out.push_str(&format!(
        "using {} = {};\n\n",
        receipt.payload_alias,
        match &receipt.payload {
            Payload::None => "Unit".into(),
            Payload::SchemaFreeJson => "JsonValue".into(),
            Payload::Json { cpp_type, .. } => cpp_type.clone(),
        }
    ));
    out.push_str(&comments(receipt));
    out.push_str(&format!(
        "/// The declared receipt route of the {kind} {name}: `method` is the verb\n/// the provider sends; register a handler for this route in your framework.\n",
    ));
    out.push_str(&format!(
        "inline constexpr IncomingRoute {}{{{}, {}, {}}};\n\n",
        receipt.route_const,
        sv(&receipt.method),
        sv(&receipt.route),
        receipt.expression,
    ));
    out
}

/// One receipt's decoder: declared required header presence checks, the
/// required-body check, then the declared JSON decode.
fn decoder(receipt: &IncomingReceipt) -> String {
    let mut out = comments(receipt);
    out.push_str(&format!(
        "/// Decodes one received {} {:?} request. Declared required headers are\n/// presence-checked and the declared body decodes through the package's own\n/// compiled codec machinery.\n",
        receipt.kind,
        super::emit::prose(&receipt.name),
    ));
    out.push_str(&format!(
        "inline Result<{}, IncomingRequestError> {}(const Headers& headers, const Bytes& body) {{\n",
        receipt.payload_alias, receipt.decode,
    ));
    // Named-but-unused parameters are pinned explicitly: the receipt helper
    // signature is uniform whatever the receipt declares.
    if receipt.required_headers.is_empty() {
        out.push_str("    (void)headers;\n");
    }
    if !receipt.required_body && matches!(receipt.payload, Payload::None) {
        out.push_str("    (void)body;\n");
    }
    for header in &receipt.required_headers {
        out.push_str(&format!(
            "    if (!detail::incoming_header_value(headers, {})) {{\n        IncomingRequestError error;\n        error.kind = IncomingRequestError::Kind::MissingHeader;\n        error.header = {};\n        return Result<{}, IncomingRequestError>::failure(std::move(error));\n    }}\n",
            sv(header),
            string(header),
            receipt.payload_alias,
        ));
    }
    if receipt.required_body {
        out.push_str(&format!(
            "    if (body.empty()) {{\n        IncomingRequestError error;\n        error.kind = IncomingRequestError::Kind::MissingBody;\n        return Result<{alias}, IncomingRequestError>::failure(std::move(error));\n    }}\n",
            alias = receipt.payload_alias,
        ));
    }
    match &receipt.payload {
        Payload::None => out.push_str(&format!(
            "    return Result<{}, IncomingRequestError>::success(Unit{{}});\n",
            receipt.payload_alias,
        )),
        Payload::SchemaFreeJson => out.push_str(&format!(
            "    auto parsed = parse_json(detail::incoming_body_view(body), JsonLimits{{}});\n    if (!parsed) {{\n        IncomingRequestError error;\n        error.kind = IncomingRequestError::Kind::InvalidPayload;\n        return Result<{alias}, IncomingRequestError>::failure(std::move(error));\n    }}\n    return Result<{alias}, IncomingRequestError>::success(std::move(parsed).value());\n",
            alias = receipt.payload_alias,
        )),
        Payload::Json { codec_name, .. } => out.push_str(&format!(
            "    auto decoded = {codec}::decode(detail::incoming_body_view(body));\n    if (!decoded) {{\n        IncomingRequestError error;\n        error.kind = IncomingRequestError::Kind::InvalidPayload;\n        return Result<{alias}, IncomingRequestError>::failure(std::move(error));\n    }}\n    return Result<{alias}, IncomingRequestError>::success(std::move(decoded).value());\n",
            codec = codec_name,
            alias = receipt.payload_alias,
        )),
    }
    out.push_str("}\n");
    out
}

/// One receipt's reply constructor, when an exact 2xx response is declared.
fn constructor(receipt: &IncomingReceipt, response: &ConstructedResponse) -> String {
    let Some(construct) = &receipt.construct else {
        return String::new();
    };
    let has_body = response.body.is_some();
    let has_headers = !response.headers.is_empty();
    let mut out = comments(receipt);
    out.push_str(&format!(
        "/// Constructs the declared {} reply of the {} {:?} receipt: the returned\n/// record carries the declared status, the encoded body and the applied\n/// declared headers.\n",
        response.status,
        receipt.kind,
        super::emit::prose(&receipt.name),
    ));
    out.push_str(&format!(
        "inline Result<ConstructedResponse, IncomingRequestError> {construct}({}) {{\n",
        [
            has_body.then(|| {
                format!(
                    "const {}& result",
                    match &response.body {
                        Some(ConstructedBody::Json { cpp_type, .. }) => cpp_type.clone(),
                        Some(ConstructedBody::SchemaFreeJson) => "JsonValue".into(),
                        None => unreachable!("a body-less reply has no result parameter"),
                    }
                )
            }),
            has_headers.then(|| "const Presence<Headers>& typed_headers = std::nullopt".into()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(", "),
    ));
    out.push_str(&format!(
        "    ConstructedResponse reply;\n    reply.status = {};\n",
        response.status,
    ));
    if has_headers {
        for header in &response.headers {
            let wire = sv(header);
            if response.required_headers.contains(header) {
                out.push_str(&format!(
                    "    if (auto value = detail::incoming_reply_header(typed_headers, {wire})) {{\n        reply.headers.emplace_back(std::string({wire}), std::move(*value));\n    }} else {{\n        IncomingRequestError error;\n        error.kind = IncomingRequestError::Kind::InvalidReply;\n        error.header = {};\n        return Result<ConstructedResponse, IncomingRequestError>::failure(std::move(error));\n    }}\n",
                    string(header),
                ));
            } else {
                out.push_str(&format!(
                    "    if (auto value = detail::incoming_reply_header(typed_headers, {wire})) {{\n        reply.headers.emplace_back(std::string({wire}), std::move(*value));\n    }}\n",
                ));
            }
        }
    }
    match &response.body {
        None => {}
        Some(ConstructedBody::SchemaFreeJson) => out.push_str("    auto encoded = write_json(result, JsonLimits{});\n    if (!encoded) {\n        IncomingRequestError error;\n        error.kind = IncomingRequestError::Kind::InvalidReply;\n        return Result<ConstructedResponse, IncomingRequestError>::failure(std::move(error));\n    }\n    reply.body = std::move(encoded).value();\n"),
        Some(ConstructedBody::Json { codec_name, .. }) => out.push_str(&format!(
            "    auto encoded = {codec}::encode(result);\n    if (!encoded) {{\n        IncomingRequestError error;\n        error.kind = IncomingRequestError::Kind::InvalidReply;\n        return Result<ConstructedResponse, IncomingRequestError>::failure(std::move(error));\n    }}\n    reply.body = std::move(encoded).value();\n",
            codec = codec_name,
        )),
    }
    out.push_str("    return Result<ConstructedResponse, IncomingRequestError>::success(std::move(reply));\n}\n");
    out
}

/// One receipt's descriptor-table arrays: the declared reply/request header
/// name arrays the frozen row points into.
fn descriptor_arrays(receipt: &IncomingReceipt) -> String {
    let key = &receipt.descriptor_key;
    let mut out = String::new();
    if let Some(response) = &receipt.response
        && !response.headers.is_empty()
    {
        out.push_str(&format!(
            "inline const std::string_view {key}_reply_headers[] = {{{}}};\n",
            response
                .headers
                .iter()
                .map(|header| sv(header))
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    if !receipt.required_headers.is_empty() {
        out.push_str(&format!(
            "inline const std::string_view {key}_required_headers[] = {{{}}};\n",
            receipt
                .required_headers
                .iter()
                .map(|header| sv(header))
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }
    out
}

/// One receipt's frozen compiled descriptor row.
fn descriptor_row(receipt: &IncomingReceipt) -> String {
    let key = &receipt.descriptor_key;
    let (reply, reply_status, reply_codec, reply_headers) = match &receipt.response {
        None => ("none".to_owned(), 0, SV_NONE.to_owned(), None),
        Some(response) => (
            response
                .body
                .as_ref()
                .map_or_else(|| "none", |body| body.label())
                .to_owned(),
            response.status,
            match &response.body {
                Some(ConstructedBody::Json { codec_name, .. }) => sv(codec_name),
                _ => SV_NONE.to_owned(),
            },
            (!response.headers.is_empty()).then(|| response.headers.clone()),
        ),
    };
    let payload_codec = match &receipt.payload {
        Payload::Json { codec_name, .. } => sv(codec_name),
        _ => SV_NONE.to_owned(),
    };
    format!(
        "    {{{}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}}},\n",
        sv(&receipt.name),
        sv(receipt.kind),
        sv(&receipt.method),
        sv(&receipt.route),
        receipt.expression,
        sv(&receipt.source.document().to_string()),
        sv(receipt.source.pointer()),
        receipt.required_headers.len(),
        if receipt.required_headers.is_empty() {
            "nullptr".to_owned()
        } else {
            format!("{key}_required_headers")
        },
        sv(Payload::label(&receipt.payload)),
        payload_codec,
        sv(&reply),
        reply_status,
        reply_codec,
        reply_headers.as_ref().map_or(0, Vec::len),
        match &reply_headers {
            Some(headers) if !headers.is_empty() => format!("{key}_reply_headers"),
            _ => "nullptr".to_owned(),
        },
    )
}

/// The `std::string_view` empty-member literal used throughout the table.
const SV_NONE: &str = "std::string_view()";

/// The generated `include/<package>/incoming.hpp`: the shared receipt surface
/// plus the per-receipt compiled helpers and the frozen descriptor table.
pub(super) fn header(plan: &SdkPlan, incoming: &IncomingEmission) -> String {
    let name = &plan.config.name;
    let mut out = String::new();
    out.push_str("#pragma once\n");
    out.push_str(
        "/** @file incoming.hpp Generated incoming receipt helpers for this\n * package's declared webhooks and callbacks.\n *\n * HTTP semantics are inverted for these receipts: the declared method is\n * the verb the provider sends and the declared responses are what the\n * handler returns. Decoders presence-check the declared required headers\n * and decode the declared body through the same compiled model codecs the\n * client uses; constructors build the declared 2xx reply. The compiled\n * receipt descriptors in `detail::incoming_descriptors` are frozen\n * generated data — the runtime never parses OpenAPI. Route expressions\n * ({$request...}) are carried verbatim; their substitution is a\n * runtime/framework concern.\n */\n",
    );
    out.push_str(&format!("#include \"{name}/http.hpp\"\n"));
    out.push_str(&format!("#include \"{name}/models.hpp\"\n"));
    out.push_str("#include <string>\n#include <string_view>\n#include <utility>\n\n");
    out.push_str(&format!("namespace {} {{\n\n", plan.config.namespace));
    out.push_str(SURFACE);
    out.push_str("namespace detail {\n\n");
    out.push_str(DESCRIPTORS_HEAD);
    for receipt in &incoming.receipts {
        out.push_str(&descriptor_arrays(receipt));
    }
    out.push_str("inline const IncomingDescriptor incoming_descriptors[] = {\n");
    for receipt in &incoming.receipts {
        out.push_str(&descriptor_row(receipt));
    }
    out.push_str(DESCRIPTORS_TAIL);
    for receipt in &incoming.receipts {
        out.push_str(&type_and_route(receipt));
        out.push_str(&decoder(receipt));
        if let Some(response) = &receipt.response {
            out.push_str(&constructor(receipt, response));
            out.push('\n');
        }
    }
    out.push_str(&format!("}} // namespace {}\n", plan.config.namespace));
    out
}

/// The shared receipt surface: the route record, the constructed reply record
/// and the branded failure type, exactly as emitted.
const SURFACE: &str = r#"/// The declared receipt route. `method` is the verb the provider sends; the
/// route string is carried verbatim from the source declaration.
struct IncomingRoute {
    std::string_view method;
    std::string_view route;
    /// True when the declared route carries RFC 6570 runtime expressions;
    /// v1 carries it verbatim and substitution is a runtime/framework concern.
    bool expression = false;
};

/// The handler return record for one constructed declared reply.
struct ConstructedResponse {
    /// The declared exact 2xx status.
    int status = 0;
    /// The applied declared reply headers.
    Headers headers;
    /// The encoded reply body; empty for body-less replies.
    std::string body;
};

/// Typed failure for a received receipt that violates its declared headers or
/// body, or a declared reply that cannot be constructed. The static runtime's
/// error vocabulary is closed, so this generated type carries the failure the
/// same way the generated OAuth lifecycle carries OAuthError. Messages and
/// members carry only safe metadata — never received payload bytes.
class IncomingRequestError {
public:
    enum class Kind {
        /// A declared required receipt header is absent.
        MissingHeader,
        /// The declared required receipt body is absent.
        MissingBody,
        /// The received payload is not valid JSON or does not satisfy its
        /// declared schema.
        InvalidPayload,
        /// A declared reply cannot be constructed: a required reply header
        /// is absent or the reply value is not representable.
        InvalidReply,
    };
    Kind kind = Kind::InvalidPayload;
    /// The absent declared header name when one is known; empty otherwise.
    std::string header;

    /// Stable kind name for callers that classify failures textually.
    [[nodiscard]] static std::string_view kind_name(Kind kind) {
        switch (kind) {
            case Kind::MissingHeader: return "missing-header";
            case Kind::MissingBody: return "missing-body";
            case Kind::InvalidPayload: return "invalid-payload";
            case Kind::InvalidReply: return "invalid-reply";
        }
        return "invalid-payload";
    }

    /// Safe message carrying only the classification metadata and declared
    /// names.
    [[nodiscard]] std::string message() const {
        std::string text = "incoming ";
        text += kind_name(kind);
        text += " failure";
        if (!header.empty()) {
            text += " for declared header ";
            text += header;
        }
        return text;
    }
};

"#;

/// The frozen descriptor table head: the record type the rows initialize.
const DESCRIPTORS_HEAD: &str = r#"/// One compiled receipt descriptor: frozen generated data, never parsed from
/// OpenAPI, and sheddable when the consumer references no receipt helper.
/// `payload` names the decode representation and `reply` the constructed
/// reply representation ("json" | "schema-free-json" | "none"); codec members
/// name the generated model codecs and stay empty when unused.
struct IncomingDescriptor {
    std::string_view name;
    std::string_view kind;
    std::string_view method;
    std::string_view route;
    bool expression = false;
    std::string_view document;
    std::string_view pointer;
    std::size_t required_header_count = 0;
    const std::string_view* required_headers = nullptr;
    std::string_view payload;
    std::string_view payload_codec;
    std::string_view reply;
    int reply_status = 0;
    std::string_view reply_codec;
    std::size_t reply_header_count = 0;
    const std::string_view* reply_headers = nullptr;
};

"#;

/// The frozen descriptor table tail: the lookup helper.
const DESCRIPTORS_TAIL: &str = r#"};

/// The compiled descriptor for one receipt name; nullptr when the receipt is
/// not compiled into this package.
inline const IncomingDescriptor* incoming_descriptor(std::string_view name) {
    for (const auto& descriptor : incoming_descriptors) {
        if (descriptor.name == name) return &descriptor;
    }
    return nullptr;
}

/// Whether one received header name matches a declared name
/// case-insensitively.
inline bool incoming_header_is(std::string_view key, std::string_view name) {
    if (key.size() != name.size()) return false;
    for (std::size_t at = 0; at < name.size(); ++at) {
        const auto lower = [](const char character) {
            return character >= 'A' && character <= 'Z'
                ? static_cast<char>(character - 'A' + 'a')
                : character;
        };
        if (lower(key[at]) != lower(name[at])) return false;
    }
    return true;
}

/// Reads one declared receipt header case-insensitively.
inline Presence<std::string> incoming_header_value(const Headers& headers, std::string_view name) {
    for (const auto& field : headers) {
        if (incoming_header_is(field.first, name)) return field.second;
    }
    return std::nullopt;
}

/// Reads one declared reply header from the caller-supplied values; an
/// absent map reads as absent.
inline Presence<std::string> incoming_reply_header(const Presence<Headers>& headers,
    std::string_view name) {
    if (!headers) return std::nullopt;
    return incoming_header_value(*headers, name);
}

/// The received body bytes as a JSON decode view; an empty body decodes as
/// an empty (invalid) document.
inline std::string_view incoming_body_view(const Bytes& body) {
    return body.empty()
        ? std::string_view()
        : std::string_view(reinterpret_cast<const char*>(body.data()), body.size());
}

} // namespace detail

"#;
