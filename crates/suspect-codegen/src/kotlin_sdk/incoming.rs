//! Emitted-only incoming receipt helpers for the Kotlin coroutine client.
//!
//! The compiled `http_protocol::IncomingPlan` becomes one generated
//! `Incoming.kt`: per receipt, a decoder for what the provider sends (declared
//! required headers are presence-checked and the body decodes through the
//! package's existing model codec machinery), a constructor for the declared
//! exact 2xx reply, and the frozen receipt route plus the frozen compiled
//! receipt descriptors. HTTP semantics are inverted for these receipts: the
//! declared method is the verb the provider sends and the declared responses
//! are what the handler returns. The v1 helpers decode declared JSON media
//! only. Static runtime files are never modified, and plans without any
//! incoming declaration emit no file and no bytes at all.

use super::{
    HttpDiagnostic, Plan,
    emit::{header, kdoc, quote, source},
    models,
};
use crate::http_protocol as wire;
use crate::http_protocol::{IncomingKind, Representation, ResponseStatus};
use std::collections::BTreeSet;
use suspect_ir::contract::{Contract, SchemaId, SourceId};

/// How `decode*Webhook` interprets the received body. The v1 helpers decode
/// declared JSON media only: through the schema-bound model codec, or as the
/// parsed schema-free JSON value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IncomingPayload {
    /// No declared request body: the decoder validates headers and returns.
    None,
    /// JSON through the operation's own model codec.
    Json { schema: SchemaId },
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
}

/// How `construct*Response` encodes the declared reply body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IncomingBody {
    /// JSON through the operation's own model codec.
    Json { schema: SchemaId },
    /// Schema-free JSON: the constructed value is encoded as bounded JSON.
    SchemaFreeJson,
}

impl IncomingBody {
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
pub struct IncomingReply {
    pub status: u16,
    pub body: Option<IncomingBody>,
    /// Declared reply header wire names, in declaration order.
    pub headers: Vec<String>,
    pub required_headers: Vec<String>,
    /// Bound after model planning: the constructed reply's native type.
    pub reply_type: String,
    pub reply_codec: String,
}

/// One incoming operation's emission-ready receipt helpers with reserved,
/// collision-free public Kotlin names. Phase one (`reserve`) allocates the
/// names before model planning; phase two (`bind`) fills the codec bindings.
#[derive(Debug, Clone)]
pub struct IncomingEntry {
    /// Webhook key, or `parentOperationId.callbackName`.
    pub name: String,
    pub kind: &'static str,
    /// The verb the provider sends.
    pub method: String,
    pub route: String,
    pub expression: bool,
    pub document: String,
    pub pointer: String,
    pub source: SourceId,
    pub descriptor_key: String,
    pub route_const: String,
    pub decode: String,
    pub construct: String,
    pub payload: IncomingPayload,
    pub required_body: bool,
    pub required_headers: Vec<String>,
    pub response: Option<IncomingReply>,
    pub explanations: Vec<String>,
    /// Bound after model planning: the decoded payload's native type.
    pub payload_type: String,
    pub payload_codec: String,
}

/// Phase one, before model planning: compile the per-receipt guards and
/// reserve every public name the emitted `Incoming.kt` will declare, so model
/// symbols can never take them. Receipts the v1 helpers cannot decode produce
/// source-linked `sdk-incoming-*` errors instead of silent skips.
pub(super) fn reserve(
    contract: &Contract,
    plan: &wire::IncomingPlan,
    type_names: &mut BTreeSet<String>,
) -> Result<Vec<IncomingEntry>, Vec<HttpDiagnostic>> {
    let mut errors = Vec::new();
    let mut result = Vec::new();
    for operation in plan.operations() {
        let source = operation.source().use_site().source().clone();
        let refuse = |errors: &mut Vec<HttpDiagnostic>, code: &'static str, message: &str| {
            errors.push(super::diagnostic(contract, source.clone(), code, message));
        };
        let stem = models::type_name(operation.name());
        let descriptor_key = models::allocate(
            &operation
                .name()
                .chars()
                .map(|character| {
                    if character.is_ascii_alphanumeric() {
                        character
                    } else {
                        '_'
                    }
                })
                .collect::<String>(),
            type_names,
        );
        let route_const = models::allocate(&format!("{stem}WebhookRoute"), type_names);
        let decode = models::allocate(&format!("decode{stem}Webhook"), type_names);
        let construct = models::allocate(&format!("construct{stem}Response"), type_names);
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
                    Some(Representation::Json { codec: Some(codec) }) => IncomingPayload::Json {
                        schema: codec.schema().id().clone(),
                    },
                    Some(Representation::Json { codec: None }) => IncomingPayload::SchemaFreeJson,
                    Some(_) => {
                        refuse(
                            &mut errors,
                            "sdk-incoming-receipt-unrepresentable",
                            "the declared incoming body representation has no v1 receipt decode; the Kotlin receipt helpers decode declared JSON payloads only, and text, binary, form, multipart and stream receipts are refused",
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
                    Some(Representation::Json { codec: Some(codec) }) => Some(IncomingBody::Json {
                        schema: codec.schema().id().clone(),
                    }),
                    Some(Representation::Json { codec: None }) => {
                        Some(IncomingBody::SchemaFreeJson)
                    }
                    Some(_) => {
                        refuse(
                            &mut errors,
                            "sdk-incoming-response-unrepresentable",
                            "the declared reply representation has no v1 constructor; the Kotlin receipt helpers encode declared JSON replies and body-less replies only",
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
                Some(IncomingReply {
                    status,
                    body,
                    headers,
                    required_headers,
                    reply_type: String::new(),
                    reply_codec: String::new(),
                })
            }
        };
        result.push(IncomingEntry {
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
            source: source.clone(),
            descriptor_key,
            route_const,
            decode,
            construct,
            payload,
            required_body: operation
                .request()
                .body()
                .as_ref()
                .is_some_and(|body| body.required()),
            required_headers,
            response,
            explanations,
            payload_type: String::new(),
            payload_codec: String::new(),
        });
    }
    if errors.is_empty() {
        Ok(result)
    } else {
        Err(errors)
    }
}

/// Phase two, after model planning: bind each entry to its payload and reply
/// codecs. Entries whose schema never reached the native model plan are
/// dropped instead of emitting guesses.
pub(super) fn bind(entries: &mut Vec<IncomingEntry>, plan: &models::ModelPlan) {
    entries.retain_mut(|entry| {
        match &entry.payload {
            IncomingPayload::Json { schema } => {
                let Some(symbol) = plan.symbol(schema) else {
                    return false;
                };
                entry.payload_type = symbol.kotlin_type.clone();
                entry.payload_codec = symbol.codec_name.clone();
            }
            IncomingPayload::SchemaFreeJson => entry.payload_type = "JsonValue".into(),
            IncomingPayload::None => entry.payload_type = "Unit".into(),
        }
        if let Some(reply) = &mut entry.response {
            match &reply.body {
                Some(IncomingBody::Json { schema }) => {
                    let Some(symbol) = plan.symbol(schema) else {
                        return false;
                    };
                    reply.reply_type = symbol.kotlin_type.clone();
                    reply.reply_codec = symbol.codec_name.clone();
                }
                Some(IncomingBody::SchemaFreeJson) => reply.reply_type = "JsonValue".into(),
                None => reply.reply_type = "Unit".into(),
            }
        }
        true
    });
}

/// The KDoc summary paragraphs shared by one receipt's declarations.
fn summary(entry: &IncomingEntry, lead: &str) -> String {
    let mut text = format!("{} Source: {}.", kdoc(lead), source(&entry.source));
    if !entry.explanations.is_empty() {
        text.push(' ');
        text.push_str(&kdoc(&entry.explanations.join(" ")));
    }
    text
}

/// The plain `document#pointer` source label carried by branded failures.
fn source_label(entry: &IncomingEntry) -> String {
    format!("{}#{}", entry.document, entry.pointer)
}

/// One receipt's frozen route constant.
fn route_constant(entry: &IncomingEntry) -> String {
    format!(
        "/** {summary} */\npublic val {route_const}: IncomingRoute = IncomingRoute(\n    method = {method},\n    path = {route},\n    expression = {expression},\n)\n\n",
        summary = summary(
            entry,
            &format!(
                "The declared receipt route of the {} {:?} receipt. `method` is the verb the provider sends; `path` is the webhook key or the callback expression exactly as declared. Register a handler at this route in your framework; runtime expression substitution is a framework concern.",
                entry.kind, entry.name,
            )
        ),
        route_const = entry.route_const,
        method = quote(&entry.method),
        route = quote(&entry.route),
        expression = entry.expression,
    )
}

/// One receipt's decoder.
fn decoder(entry: &IncomingEntry) -> String {
    let mut code = format!(
        "/** {summary} */\npublic fun {decode}(headers: Map<String, String>, body: ByteArray): {payload} {{\n",
        summary = summary(
            entry,
            &format!(
                "Decodes one received {} {:?} request. Declared required headers are presence-checked (a missing one throws IncomingRequestException) and the declared body decodes through the operation's own compiled codec machinery.",
                entry.kind, entry.name,
            )
        ),
        decode = entry.decode,
        payload = entry.payload_type,
    );
    for header in &entry.required_headers {
        code.push_str(&format!(
            "    incomingRequireHeader(headers, {})\n",
            quote(header)
        ));
    }
    if entry.required_body {
        code.push_str(
            "    if (body.isEmpty()) throw IncomingRequestException(\"the declared required receipt body is absent\")\n",
        );
    }
    match &entry.payload {
        IncomingPayload::None => {}
        IncomingPayload::Json { .. } => code.push_str(&format!(
            "    return incomingValidated({}) {{ Codecs.{}.decode(body) }}\n",
            quote(&source_label(entry)),
            entry.payload_codec,
        )),
        IncomingPayload::SchemaFreeJson => code.push_str(&format!(
            "    return incomingJson({}) {{ Json.parse(body) }}\n",
            quote(&source_label(entry)),
        )),
    }
    code.push_str("}\n\n");
    code
}

/// One receipt's reply constructor, when an exact 2xx response is declared.
fn constructor(entry: &IncomingEntry, reply: &IncomingReply) -> String {
    let has_body = reply.body.is_some();
    let has_headers = !reply.headers.is_empty();
    let mut parameters = String::new();
    if has_body {
        parameters.push_str("result: ");
        parameters.push_str(&reply.reply_type);
    }
    if has_headers {
        if has_body {
            parameters.push_str(", ");
        }
        parameters.push_str("typedHeaders: Map<String, String> = emptyMap()");
    }
    let mut code = format!(
        "/** {summary} */\npublic fun {construct}({parameters}): Triple<Int, Map<String, String>, ByteArray> {{\n",
        summary = summary(
            entry,
            &format!(
                "Constructs the declared {} reply of the {} {:?} receipt: the returned triple carries the declared status, the applied declared reply headers and the encoded body.",
                reply.status, entry.kind, entry.name,
            )
        ),
        construct = entry.construct,
        parameters = parameters,
    );
    if has_headers {
        let optional = reply
            .headers
            .iter()
            .filter(|header| !reply.required_headers.contains(header))
            .cloned()
            .collect::<Vec<_>>();
        code.push_str("    val headers = linkedMapOf<String, String>()\n");
        if !optional.is_empty() {
            code.push_str(&format!(
                "    for (name in listOf({names})) {{\n        incomingHeaderValue(typedHeaders, name)?.let {{ headers[name] = it }}\n    }}\n",
                names = optional
                    .iter()
                    .map(|h| quote(h))
                    .collect::<Vec<_>>()
                    .join(", "),
            ));
        }
        if !reply.required_headers.is_empty() {
            code.push_str(&format!(
                "    for (name in listOf({names})) {{\n        val found = incomingHeaderValue(typedHeaders, name)\n        if (found == null) throw IncomingRequestException(\"the declared required reply header $name is absent\")\n        headers[name] = found\n    }}\n",
                names = reply
                    .required_headers
                    .iter()
                    .map(|h| quote(h))
                    .collect::<Vec<_>>()
                    .join(", "),
            ));
        }
    }
    let headers_expression = if has_headers {
        "headers"
    } else {
        "emptyMap()"
    };
    let body = match &reply.body {
        None => "ByteArray(0)".into(),
        Some(IncomingBody::Json { .. }) => format!(
            "Codecs.{}.encode(result).toByteArray(StandardCharsets.UTF_8)",
            reply.reply_codec,
        ),
        Some(IncomingBody::SchemaFreeJson) => format!(
            "incomingJsonBytes({}) {{ Json.stringify(result) }}",
            quote(&source_label(entry)),
        ),
    };
    code.push_str(&format!(
        "    return Triple({status}, {headers}, {body})\n}}\n\n",
        status = reply.status,
        headers = headers_expression,
        body = body,
    ));
    code
}

/// The frozen compiled receipt descriptor map: generated data, never parsed
/// from OpenAPI.
fn descriptors(entries: &[IncomingEntry]) -> String {
    let mut code = String::from(
        "/** Frozen compiled receipt descriptors for this package: one entry per declared\n * webhook/callback receipt. `payload` names the decode representation and\n * `reply` the constructed reply representation (`json`, `schema-free-json` or\n * `none`); the codec members name the generated model codec exports.\n */\npublic object IncomingDescriptors {\n    public val receipts: Map<String, IncomingDescriptor> = mapOf(\n",
    );
    for entry in entries {
        let (reply, reply_status, reply_codec, reply_headers) = match &entry.response {
            None => (
                quote("none"),
                "null".to_owned(),
                "null".to_owned(),
                "emptyList()".to_owned(),
            ),
            Some(response) => (
                quote(response.body.as_ref().map_or_else(|| "none", |body| body.label())),
                response.status.to_string(),
                match &response.body {
                    Some(IncomingBody::Json { .. }) => quote(&response.reply_codec),
                    _ => "null".to_owned(),
                },
                if response.headers.is_empty() {
                    "emptyList()".to_owned()
                } else {
                    format!(
                        "listOf({})",
                        response
                            .headers
                            .iter()
                            .map(|header| quote(header))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                },
            ),
        };
        let payload_codec = match &entry.payload {
            IncomingPayload::Json { .. } => quote(&entry.payload_codec),
            _ => "null".to_owned(),
        };
        #[allow(clippy::format_in_format_args)]
        code.push_str(&format!(
            "        {key} to IncomingDescriptor(\n            kind = {kind},\n            method = {method},\n            path = {route},\n            expression = {expression},\n            source = {source_value},\n            requiredHeaders = {headers},\n            payload = {payload},\n            payloadCodec = {payload_codec},\n            reply = {reply},\n            replyStatus = {reply_status},\n            replyCodec = {reply_codec},\n            replyHeaders = {reply_headers},\n        ),\n",
            key = quote(&entry.descriptor_key),
            kind = quote(entry.kind),
            method = quote(&entry.method),
            route = quote(&entry.route),
            expression = entry.expression,
            source_value = format!(
                "IncomingSource({}, {})",
                quote(&entry.document),
                quote(&entry.pointer),
            ),
            headers = if entry.required_headers.is_empty() {
                "emptyList()".to_owned()
            } else {
                format!(
                    "listOf({})",
                    entry
                        .required_headers
                        .iter()
                        .map(|header| quote(header))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            },
            payload = quote(IncomingPayload::label(&entry.payload)),
            payload_codec = payload_codec,
            reply = reply,
            reply_status = reply_status,
            reply_codec = reply_codec,
            reply_headers = reply_headers,
        ));
    }
    code.push_str("    )\n}\n\n");
    code
}

/// The generated `Incoming.kt` module: the shared receipt types and helpers
/// plus, per receipt, the frozen route, decoder and reply constructor, and the
/// frozen descriptor map.
pub(super) fn module(plan: &Plan) -> String {
    let mut out = header(plan);
    out.push_str("import java.nio.charset.StandardCharsets\n\n");
    out.push_str(
        "/** Generated incoming receipt helpers for this package's declared webhooks and\n * callbacks. HTTP semantics are inverted for these receipts: the declared\n * method is the verb the provider sends and the declared responses are what\n * the handler returns. Decoders presence-check the declared required headers\n * and decode the declared body through the same compiled model codecs the\n * client uses; constructors build the declared 2xx reply. The compiled receipt\n * descriptors are frozen generated data — the runtime never parses OpenAPI.\n * Route expressions (`{$request...}`) are carried verbatim; their substitution\n * is a runtime/framework concern.\n */\n\n",
    );
    out.push_str(
        "/** Typed failure for a received receipt that violates its declared headers or\n * body, or for a declared reply that cannot be constructed. The optional\n * cause carries the underlying codec failure. */\npublic class IncomingRequestException(\n    message: String,\n    cause: Throwable? = null,\n) : RuntimeException(message, cause)\n\n",
    );
    out.push_str(
        "/** The declared receipt route of one registered handler. `method` is the verb\n * the provider sends; `path` is the webhook key or the callback expression\n * exactly as declared, carried verbatim. */\npublic data class IncomingRoute(\n    public val method: String,\n    public val path: String,\n    public val expression: Boolean,\n)\n\n",
    );
    out.push_str(
        "/** Original document URI and schema pointer of one receipt declaration. */\npublic data class IncomingSource(\n    /** Canonical document URI. */\n    public val document: String,\n    /** Escaped RFC 6901 source pointer. */\n    public val pointer: String,\n)\n\n",
    );
    out.push_str(
        "/** One compiled receipt's frozen descriptor: exactly what the source declared\n * plus the compiled codec bindings. `payload` names the decode representation\n * (`json`, `schema-free-json` or `none`) and `reply` the constructed reply\n * representation (`json`, `schema-free-json` or `none`); the codec members\n * name the generated model codec exports. */\npublic data class IncomingDescriptor(\n    public val kind: String,\n    public val method: String,\n    public val path: String,\n    public val expression: Boolean,\n    public val source: IncomingSource,\n    public val requiredHeaders: List<String>,\n    public val payload: String,\n    public val payloadCodec: String?,\n    public val reply: String,\n    public val replyStatus: Int?,\n    public val replyCodec: String?,\n    public val replyHeaders: List<String>,\n)\n\n",
    );
    for entry in plan.incoming_entries() {
        out.push_str(&route_constant(entry));
        out.push_str(&decoder(entry));
        if let Some(reply) = &entry.response {
            out.push_str(&constructor(entry, reply));
        }
    }
    out.push_str(&descriptors(plan.incoming_entries()));
    out.push_str(
        "/** Reads one declared receipt header case-insensitively; absent stays null. */\ninternal fun incomingHeaderValue(headers: Map<String, String>, name: String): String? {\n    for ((key, value) in headers) {\n        if (key.equals(name, ignoreCase = true)) return value\n    }\n    return null\n}\n\n",
    );
    out.push_str(
        "/** Throws the typed failure when a required declared receipt header is absent. */\ninternal fun incomingRequireHeader(headers: Map<String, String>, name: String) {\n    if (incomingHeaderValue(headers, name) == null) {\n        throw IncomingRequestException(\"required receipt header $name is absent\")\n    }\n}\n\n",
    );
    out.push_str(
        "/** Decodes one received body through its compiled codec; a source violation\n * is a branded incoming failure carrying the codec failure as its cause. */\ninternal inline fun <T> incomingValidated(source: String, decode: () -> T): T = try {\n    decode()\n} catch (error: Exception) {\n    throw IncomingRequestException(\"the received payload does not satisfy its declared schema ($source)\", error)\n}\n\n",
    );
    out.push_str(
        "/** Parses one schema-free JSON payload; an unparseable payload is a branded\n * incoming failure. */\ninternal inline fun incomingJson(source: String, parse: () -> JsonValue): JsonValue = try {\n    parse()\n} catch (error: Exception) {\n    throw IncomingRequestException(\"the received payload is not valid JSON ($source)\", error)\n}\n\n",
    );
    out.push_str(
        "/** Encodes one schema-free JSON reply; an unrepresentable reply value is a\n * branded incoming failure. */\ninternal inline fun incomingJsonBytes(source: String, encode: () -> String): ByteArray = try {\n    encode().toByteArray(StandardCharsets.UTF_8)\n} catch (error: Exception) {\n    throw IncomingRequestException(\"the reply value is not representable JSON ($source)\", error)\n}\n",
    );
    out
}
