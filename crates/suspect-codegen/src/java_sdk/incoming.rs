//! Emitted-only incoming receipt helpers for the Java HTTP SDK.
//!
//! The compiled `http_protocol::IncomingPlan` becomes one generated
//! `Incoming.java`: per receipt, a typed decoder for what the provider sends
//! (declared required headers are presence-checked case-insensitively and the
//! declared JSON body decodes through the package's existing model codec
//! machinery), a constructor for the declared exact 2xx reply, the frozen
//! receipt route and the frozen compiled receipt descriptors. HTTP semantics
//! are inverted for these receipts: the declared method is the verb the
//! provider sends and the declared responses are what the handler returns.
//! The v1 helpers decode declared JSON payloads only. Static runtime files
//! are never modified, and plans without any incoming declaration emit no
//! file and no bytes at all.

use std::collections::BTreeSet;

use super::{
    SdkPlan,
    models::{JavaModelPlan, allocate, bounded, is_java_identifier, javadoc, q},
};
use crate::http_contract::HttpDiagnostic;
use crate::http_protocol::{IncomingKind, IncomingPlan, Representation, ResponseStatus};
use crate::rust_models::{pascal, snake};
use suspect_ir::contract::{Contract, SchemaId};

/// Top-level type name `Incoming.java` owns in the package. Reserved against
/// model symbols only while the file is emitted, so receipt-less allocation
/// behavior is unchanged; every other emitted type is nested inside `Incoming`.
pub(crate) const TOP_LEVEL_NAMES: &[&str] = &["Incoming"];

/// How `decode*Webhook` interprets the received body. The v1 helpers decode
/// declared JSON payloads only: through the schema-bound model codec, or as
/// the schema-free JSON value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Payload {
    /// No declared request body: the decoder validates headers and returns.
    None,
    /// JSON through the operation's own model codec.
    Json { schema: SchemaId },
    /// Schema-free JSON: the decoded value is the parsed JSON value.
    SchemaFreeJson,
}

impl Payload {
    /// The descriptor label of the decode representation.
    fn label(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Json { .. } => "json",
            Self::SchemaFreeJson => "schema-free-json",
        }
    }

    /// The emitted return type of one decoder.
    fn native_type(&self, models: &JavaModelPlan) -> String {
        match self {
            Self::None => "void".into(),
            Self::Json { schema } => models.native_type(schema),
            Self::SchemaFreeJson => "JsonRuntime.JsonValue".into(),
        }
    }

    /// The codec holder one decoder reads through, when it has one.
    fn codec_holder(&self, models: &JavaModelPlan) -> Option<String> {
        match self {
            Self::Json { schema } => Some(models.codec(schema).holder.clone()),
            _ => None,
        }
    }
}

/// How `construct*Response` encodes the declared reply body. The v1 helpers
/// encode declared JSON replies only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ConstructedBody {
    /// JSON through the operation's own model codec.
    Json { schema: SchemaId },
    /// Schema-free JSON: the constructed value is encoded as bounded JSON.
    SchemaFreeJson,
}

impl ConstructedBody {
    /// The descriptor label of the reply representation.
    fn label(&self) -> &'static str {
        match self {
            Self::Json { .. } => "json",
            Self::SchemaFreeJson => "schema-free-json",
        }
    }

    /// The emitted native type of one constructed reply value.
    fn native_type(&self, models: &JavaModelPlan) -> String {
        match self {
            Self::Json { schema } => models.native_type(schema),
            Self::SchemaFreeJson => "JsonRuntime.JsonValue".into(),
        }
    }

    /// The codec holder one constructor encodes through, when it has one.
    fn codec_holder(&self, models: &JavaModelPlan) -> Option<String> {
        match self {
            Self::Json { schema } => Some(models.codec(schema).holder.clone()),
            _ => None,
        }
    }
}

/// One declared reply the construct helper builds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Reply {
    pub status: u16,
    pub body: Option<ConstructedBody>,
    /// Declared reply header wire names, in declaration order.
    pub headers: Vec<String>,
    pub required_headers: Vec<String>,
}

/// One incoming operation's emission-ready receipt helpers with allocated,
/// collision-free public Java names.
#[derive(Debug, Clone)]
pub(super) struct Receipt {
    /// Webhook key, or `parentOperationId.callbackName`.
    pub name: String,
    pub kind: &'static str,
    /// The verb the provider sends.
    pub method: String,
    pub route: String,
    pub expression: bool,
    pub document: String,
    pub pointer: String,
    pub location: String,
    pub descriptor_key: String,
    pub route_const: String,
    pub decode: String,
    pub construct: String,
    pub response_type: String,
    pub payload: Payload,
    pub required_body: bool,
    pub required_headers: Vec<String>,
    pub response: Option<Reply>,
    pub explanations: Vec<String>,
}

/// The receipt's Pascal stem, bounded to a portable Java identifier.
fn receipt_stem(name: &str) -> String {
    let stem = bounded(&pascal(name));
    if is_java_identifier(&stem) {
        stem
    } else {
        "Receipt".into()
    }
}

/// Compiles the incoming plan against the native model bindings and the
/// emitted member allocation. Receipts the v1 helpers cannot express produce
/// source-linked `sdk-incoming-*` errors instead of silent skips.
pub(crate) fn prepare(
    contract: &Contract,
    plan: &IncomingPlan,
    models: &JavaModelPlan,
) -> Result<Vec<Receipt>, Vec<HttpDiagnostic>> {
    let mut members: BTreeSet<String> = ["DESCRIPTORS", "headerValue", "requireHeader"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    let mut types: BTreeSet<String> = [
        "IncomingDescriptor",
        "IncomingRequestException",
        "IncomingRoute",
        "IncomingSource",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let mut errors = Vec::new();
    let mut result = Vec::new();
    for operation in plan.operations() {
        let source = operation.source().use_site().source().clone();
        let refuse = |errors: &mut Vec<HttpDiagnostic>, code: &'static str, message: &str| {
            errors.push(super::diagnostic(contract, source.clone(), code, message));
        };
        let stem = receipt_stem(operation.name());
        let descriptor_key = allocate(
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
            &mut members,
        );
        let route_const = allocate(
            &format!(
                "{}_WEBHOOK_ROUTE",
                snake(operation.name()).to_ascii_uppercase()
            ),
            &mut members,
        );
        let decode = allocate(&format!("decode{stem}Webhook"), &mut members);
        let construct = allocate(&format!("construct{stem}Response"), &mut members);
        let response_type = allocate(&format!("{stem}Response"), &mut types);
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
                        let schema = codec.schema().id().clone();
                        if models.symbol(&schema).is_none() {
                            refuse(
                                &mut errors,
                                "sdk-incoming-codec-binding",
                                "the declared incoming payload schema has no compiled model codec",
                            );
                            continue;
                        }
                        Payload::Json { schema }
                    }
                    Some(Representation::Json { codec: None }) => Payload::SchemaFreeJson,
                    Some(_) => {
                        refuse(
                            &mut errors,
                            "sdk-incoming-receipt-unrepresentable",
                            "the declared incoming body representation has no v1 receipt decode; the Java receipt helpers decode declared JSON payloads only",
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
                        let schema = codec.schema().id().clone();
                        if models.symbol(&schema).is_none() {
                            refuse(
                                &mut errors,
                                "sdk-incoming-codec-binding",
                                "the declared reply schema has no compiled model codec",
                            );
                            continue;
                        }
                        Some(ConstructedBody::Json { schema })
                    }
                    Some(Representation::Json { codec: None }) => {
                        Some(ConstructedBody::SchemaFreeJson)
                    }
                    Some(_) => {
                        refuse(
                            &mut errors,
                            "sdk-incoming-response-unrepresentable",
                            "the declared reply representation has no v1 constructor; the Java receipt helpers encode declared JSON replies only",
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
                Some(Reply {
                    status,
                    body,
                    headers,
                    required_headers,
                })
            }
        };
        result.push(Receipt {
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
            location: format!(
                "{}#{}",
                source.document(),
                source.pointer()
            ),
            descriptor_key,
            route_const,
            decode,
            construct,
            response_type,
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

/// The admitted receipts; planning already refused every unrepresentable one.
pub(crate) fn compiled(plan: &SdkPlan) -> Vec<Receipt> {
    prepare(plan.contract(), plan.incoming(), plan.models())
        .expect("receipts admitted at plan time")
}

/// The complete `Incoming.java` source body: the shared receipt types, one
/// frozen route, decoder and reply constructor per receipt, the shared
/// case-insensitive header readers and the frozen descriptor map. `None`
/// when the plan declares no receipts at all.
pub(crate) fn source(plan: &SdkPlan) -> Option<String> {
    if plan.incoming().is_empty() {
        return None;
    }
    let receipts = compiled(plan);
    if receipts.is_empty() {
        return None;
    }
    let mut out = String::from(PREAMBLE);
    for receipt in &receipts {
        out.push_str(&route_constant(receipt));
        out.push_str(&decoder(plan, receipt));
        if let Some(reply) = &receipt.response {
            out.push_str(&constructor(plan, receipt, reply));
        }
    }
    out.push_str(HELPERS);
    out.push_str(&descriptors(plan, &receipts));
    out.push_str("}\n");
    Some(out)
}

/// The escaped lead prose shared by every generated helper, with the source
/// location and any v1 explanations appended.
fn summary(receipt: &Receipt, lead: &str) -> String {
    let mut text = format!("{lead} Source: {}.", receipt.location);
    if !receipt.explanations.is_empty() {
        text.push(' ');
        text.push_str(&receipt.explanations.join(" "));
    }
    javadoc(&text)
}

/// One receipt's frozen route constant: the registration hint whose
/// `method` is the verb the provider sends.
fn route_constant(receipt: &Receipt) -> String {
    format!(
        "    /** {summary} Register a handler at this route in your framework; runtime expression substitution is a runtime/framework concern. */\n    public static final IncomingRoute {route_const} = new IncomingRoute({method}, {route}, {expression});\n\n",
        summary = summary(
            receipt,
            &format!(
                "The declared receipt route of the {} {:?} receipt. {{@code method}} is the verb the provider sends; the route string is carried verbatim from the source declaration.",
                receipt.kind, receipt.name,
            )
        ),
        route_const = receipt.route_const,
        method = q(&receipt.method),
        route = q(&receipt.route),
        expression = receipt.expression,
    )
}

/// One receipt's decoder: case-insensitive declared-header presence checks
/// followed by the declared JSON body decode through its model codec.
fn decoder(plan: &SdkPlan, receipt: &Receipt) -> String {
    let returns = match receipt.payload {
        Payload::None => "",
        _ => "\n     * @return the decoded payload",
    };
    let mut code = format!(
        "    /** {summary} Declared required headers are presence-checked case-insensitively (a missing one throws the typed IncomingRequestException) and the declared JSON body decodes through the operation's own compiled codec machinery.\n     * @param headers the received header names and values\n     * @param body the received body bytes{returns}\n     */\n    public static {ty} {decode}(java.util.Map<String,String> headers, byte[] body) {{\n",
        summary = summary(
            receipt,
            &format!(
                "Decodes one received {} {:?} request.",
                receipt.kind, receipt.name,
            )
        ),
        ty = receipt.payload.native_type(plan.models()),
        decode = receipt.decode,
        returns = returns,
    );
    for header in &receipt.required_headers {
        code.push_str(&format!(
            "        requireHeader(headers, {});\n",
            q(header)
        ));
    }
    if receipt.required_body {
        code.push_str("        if (body.length == 0) throw new IncomingRequestException(\"the declared required receipt body is absent\");\n");
    }
    match &receipt.payload {
        Payload::None => code.push_str("        return;\n"),
        Payload::Json { .. } => code.push_str(&format!(
            "        try {{\n            return {}.CODEC.decode(body);\n        }} catch (CodecException error) {{\n            throw new IncomingRequestException({}, error);\n        }}\n",
            receipt
                .payload
                .codec_holder(plan.models())
                .expect("json payload carries a codec"),
            q(&format!(
                "the received payload does not satisfy its declared schema ({})",
                receipt.location
            )),
        )),
        Payload::SchemaFreeJson => code.push_str(&format!(
            "        try {{\n            return JsonRuntime.parse(body);\n        }} catch (JsonRuntime.JsonError error) {{\n            throw new IncomingRequestException({}, error);\n        }}\n",
            q(&format!("the received payload is not valid JSON ({})", receipt.location)),
        )),
    }
    code.push_str("    }\n\n");
    code
}

/// One receipt's reply constructor, when an exact 2xx response is declared.
fn constructor(plan: &SdkPlan, receipt: &Receipt, reply: &Reply) -> String {
    let has_body = reply.body.is_some();
    let has_headers = !reply.headers.is_empty();
    let mut parameters = String::new();
    if has_body {
        parameters.push_str(&format!(
            "{} result",
            reply
                .body
                .as_ref()
                .expect("declared reply body")
                .native_type(plan.models())
        ));
        if has_headers {
            parameters.push_str(", ");
        }
    }
    if has_headers {
        parameters.push_str("java.util.Map<String,String> typedHeaders");
    }
    let mut code = format!(
        "    /** The handler return record of the constructed {status} reply of the {kind} {name:?} receipt: the declared status, the applied declared reply headers and the encoded body bytes. */\n    public record {response_type}(int status, java.util.Map<String,String> headers, byte[] body) {{}}\n\n",
        status = reply.status,
        kind = receipt.kind,
        name = receipt.name,
        response_type = receipt.response_type,
    );
    let mut params_doc = String::new();
    if has_body {
        params_doc.push_str("\n     * @param result the declared reply body value");
    }
    if has_headers {
        params_doc.push_str("\n     * @param typedHeaders the caller-supplied reply header values");
    }
    code.push_str(&format!(
        "    /** {summary} The returned record carries the declared status, the encoded body and the applied declared headers.{params_doc}\n     * @return the constructed reply\n     */\n    public static {response_type} {construct}({parameters}) {{\n        java.util.Map<String,String> headers = new java.util.LinkedHashMap<>();\n",
        summary = summary(
            receipt,
            &format!(
                "Constructs the declared {} reply of the {} {:?} receipt.",
                reply.status, receipt.kind, receipt.name,
            )
        ),
        params_doc = params_doc,
        response_type = receipt.response_type,
        construct = receipt.construct,
        parameters = parameters,
    ));
    if has_headers {
        let optional = reply
            .headers
            .iter()
            .filter(|header| !reply.required_headers.contains(header))
            .cloned()
            .collect::<Vec<_>>();
        if !optional.is_empty() {
            code.push_str(&format!(
                "        for (String name : java.util.List.of({names})) {{\n            String value = headerValue(typedHeaders, name);\n            if (value != null) headers.put(name, value);\n        }}\n",
                names = optional
                    .iter()
                    .map(|header| q(header))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !reply.required_headers.is_empty() {
            code.push_str(&format!(
                "        for (String name : java.util.List.of({names})) {{\n            String value = headerValue(typedHeaders, name);\n            if (value == null) throw new IncomingRequestException(\"the declared required reply header \" + name + \" is absent\");\n            headers.put(name, value);\n        }}\n",
                names = reply
                    .required_headers
                    .iter()
                    .map(|header| q(header))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    let body = match &reply.body {
        Some(ConstructedBody::Json { .. }) => format!(
            "JsonRuntime.bytes({}.CODEC.encodeValue(result))",
            reply
                .body
                .as_ref()
                .and_then(|body| body.codec_holder(plan.models()))
                .expect("json reply carries a codec")
        ),
        Some(ConstructedBody::SchemaFreeJson) => "JsonRuntime.bytes(result)".into(),
        None => "new byte[0]".into(),
    };
    code.push_str(&format!(
        "        return new {response_type}({status}, java.util.Collections.unmodifiableMap(headers), {body});\n    }}\n\n",
        response_type = receipt.response_type,
        status = reply.status,
        body = body,
    ));
    code
}

/// The shared case-insensitive receipt header readers, emitted once before
/// the frozen descriptor map.
const HELPERS: &str = r#"    /** Reads one declared receipt header case-insensitively; absent stays null. */
    private static String headerValue(java.util.Map<String,String> headers, String name) {
        if (headers == null) return null;
        String wanted = name.toLowerCase(java.util.Locale.ROOT);
        for (java.util.Map.Entry<String,String> entry : headers.entrySet()) {
            if (entry.getKey() != null && entry.getKey().toLowerCase(java.util.Locale.ROOT).equals(wanted)) return entry.getValue();
        }
        return null;
    }

    /** Throws the typed failure when a required declared receipt header is absent. */
    private static void requireHeader(java.util.Map<String,String> headers, String name) {
        if (headerValue(headers, name) == null) throw new IncomingRequestException("required receipt header " + name + " is absent");
    }

"#;

/// The declared header list of one descriptor entry.
fn header_list(headers: &[String]) -> String {
    if headers.is_empty() {
        return "java.util.List.of()".into();
    }
    format!(
        "java.util.List.of({})",
        headers
            .iter()
            .map(|header| q(header))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// The frozen compiled receipt descriptor map: generated data, never parsed
/// from OpenAPI.
fn descriptors(plan: &SdkPlan, receipts: &[Receipt]) -> String {
    let mut code = String::from(
        "    /** Frozen compiled receipt descriptors for this package: one entry per declared webhook/callback receipt. {@code payload} names the decode representation ('json' | 'schema-free-json' | 'none') and {@code reply} the constructed reply representation ('json' | 'schema-free-json' | 'none'); the codec members name the generated model codec holders. */\n    public static final java.util.Map<String,IncomingDescriptor> DESCRIPTORS;\n    static {\n        java.util.Map<String,IncomingDescriptor> descriptors = new java.util.LinkedHashMap<>();\n",
    );
    for receipt in receipts {
        let (reply, reply_status, reply_codec, reply_headers) = match &receipt.response {
            None => (
                "none",
                "null".to_owned(),
                "null".to_owned(),
                header_list(&[]),
            ),
            Some(response) => (
                response
                    .body
                    .as_ref()
                    .map_or("none", |body| body.label()),
                response.status.to_string(),
                match &response.body {
                    Some(ConstructedBody::Json { .. }) => q(
                        &response
                            .body
                            .as_ref()
                            .and_then(|body| body.codec_holder(plan.models()))
                            .expect("json reply carries a codec"),
                    ),
                    _ => "null".to_owned(),
                },
                header_list(&response.headers),
            ),
        };
        let payload_codec = match &receipt.payload {
            Payload::Json { .. } => q(
                &receipt
                    .payload
                    .codec_holder(plan.models())
                    .expect("json payload carries a codec"),
            ),
            _ => "null".to_owned(),
        };
        code.push_str(&format!(
            "        descriptors.put({key}, new IncomingDescriptor({kind}, {method}, {route}, {expression}, new IncomingSource({document}, {pointer}), {required_headers}, {payload}, {payload_codec}, {reply}, {reply_status}, {reply_codec}, {reply_headers}));\n",
            key = q(&receipt.descriptor_key),
            kind = q(receipt.kind),
            method = q(&receipt.method),
            route = q(&receipt.route),
            expression = receipt.expression,
            document = q(&receipt.document),
            pointer = q(&receipt.pointer),
            required_headers = header_list(&receipt.required_headers),
            payload = q(receipt.payload.label()),
            payload_codec = payload_codec,
            reply = q(reply),
            reply_status = reply_status,
            reply_codec = reply_codec,
            reply_headers = reply_headers,
        ));
    }
    code.push_str("        DESCRIPTORS = java.util.Collections.unmodifiableMap(descriptors);\n    }\n\n");
    code
}

/// The static library half of the generated class: the class documentation,
/// the shared receipt types and the opening of the generated class.
const PREAMBLE: &str = r#"/**
 * Generated incoming receipt helpers for this package's declared webhooks and
 * callbacks.
 *
 * <p>HTTP semantics are inverted for these receipts: the declared method is
 * the verb the provider sends and the declared responses are what the handler
 * returns. Every decoder presence-checks the declared required headers
 * case-insensitively and decodes the declared JSON body through the same
 * compiled model codecs the client uses; every constructor builds the
 * declared exact 2xx reply, encoding the body through the response codec and
 * applying the declared reply headers with required-presence checks. The
 * compiled receipt descriptors in {@code DESCRIPTORS} are frozen generated
 * data: the runtime never parses OpenAPI. Route expression strings are
 * carried verbatim; their substitution is a runtime/framework concern.
 */
public final class Incoming {
    private Incoming() {}

    /** The declared receipt route of one registered handler. {@code method} is the verb the provider sends; the route string is carried verbatim from the source declaration. */
    public record IncomingRoute(String method, String route, boolean expression) {}

    /** Original document URI and schema pointer of one receipt declaration. */
    public record IncomingSource(String document, String pointer) {}

    /**
     * Typed failure for a received receipt that violates its declared headers
     * or body, or for a declared reply that cannot be constructed. The cause
     * carries the underlying codec failure when one exists.
     */
    public static final class IncomingRequestException extends RuntimeException {
        private static final long serialVersionUID = 1L;
        public IncomingRequestException(String message) { super(message); }
        public IncomingRequestException(String message, Throwable cause) { super(message, cause); }
    }

    /** One compiled receipt's frozen descriptor: exactly what the source declared plus the compiled codec bindings. {@code payload} names the decode representation ('json' | 'schema-free-json' | 'none') and {@code reply} the constructed reply representation ('json' | 'schema-free-json' | 'none'); the codec members name the generated model codec holders. */
    public record IncomingDescriptor(String kind, String method, String route, boolean expression, IncomingSource source, java.util.List<String> requiredHeaders, String payload, String payloadCodec, String reply, Integer replyStatus, String replyCodec, java.util.List<String> replyHeaders) {}

"#;
