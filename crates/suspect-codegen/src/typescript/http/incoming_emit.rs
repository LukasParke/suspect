//! Generated-only incoming receipt helpers for the TypeScript HTTP adapter.
//!
//! The compiled `http_protocol::IncomingPlan` becomes one generated
//! `typescript/incoming.ts` module: per receipt, a typed payload, a decoder for
//! what the provider sends (declared required headers are presence-checked and
//! the body decodes through the operation's own codec machinery), a constructor
//! for the declared exact 2xx reply, and the frozen receipt route. The decoded
//! side binds the response-view model (we read what we receive) and the
//! constructed side binds the request-view model (we write what we return), so
//! directional views keep their exact semantics. Static runtime files are never
//! modified — the branded `IncomingRequestError` follows the generated
//! `PaginationError` pattern because the runtime's `SdkFailureKind` union is
//! closed — and plans without any incoming declaration emit no file and no
//! bytes at all.

use std::collections::{BTreeMap, BTreeSet};

use super::src;
use crate::http_contract::HttpDiagnostic;
use crate::http_protocol as protocol;
use crate::http_protocol::{
    IncomingKind, IncomingPlan, Representation, ResponseStatus, ScalarType,
};
use crate::rust_models::pascal;
use suspect_ir::contract::{Contract, SchemaId};

/// How `decode` interprets the received body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Payload {
    /// No declared request body: the decoder validates headers and returns undefined.
    None,
    /// JSON through the operation's own model codec.
    Json { codec: String },
    /// Schema-free JSON: the decoded value is the parsed JSON value.
    SchemaFreeJson,
    /// Declared text: scalar wire conversion, optionally through a codec.
    Text {
        scalar: &'static str,
        codec: Option<String>,
    },
    /// Opaque declared bytes, passed through unchanged.
    Binary,
}

impl Payload {
    /// The descriptor label for the decode representation.
    fn label(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Json { .. } => "json",
            Self::SchemaFreeJson => "schema-free-json",
            Self::Text { .. } => "text",
            Self::Binary => "binary",
        }
    }
}

/// How `construct` encodes the declared reply body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ConstructedBody {
    Json { codec: String },
    SchemaFreeJson,
    Binary,
}

impl ConstructedBody {
    /// The descriptor label for the reply representation.
    fn label(&self) -> &'static str {
        match self {
            Self::Json { .. } => "json",
            Self::SchemaFreeJson => "schema-free-json",
            Self::Binary => "binary",
        }
    }
}

/// One declared reply the construct helper builds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ConstructedResponse {
    pub status: u16,
    pub body: Option<ConstructedBody>,
    /// Declared reply header wire names, in declaration order.
    pub headers: Vec<String>,
    pub required_headers: Vec<String>,
}

/// One incoming operation's emission-ready receipt helpers with reserved,
/// collision-free public TypeScript names.
pub(super) struct IncomingEmission {
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
    pub payload_type: String,
    pub decode: String,
    pub construct: Option<String>,
    pub payload: Payload,
    pub required_body: bool,
    pub required_headers: Vec<String>,
    pub response: Option<ConstructedResponse>,
    pub explanations: Vec<String>,
}

fn allocate(base: &str, public: &mut BTreeSet<String>) -> String {
    let mut name = base.to_owned();
    let mut suffix = 2;
    while !public.insert(name.clone()) {
        name = format!("{base}_{suffix}");
        suffix += 1;
    }
    name
}

fn q(value: &str) -> String {
    serde_json::to_string(value).unwrap()
}

fn scalar_name(scalar: ScalarType) -> &'static str {
    match scalar {
        ScalarType::String => "string",
        ScalarType::Boolean => "boolean",
        ScalarType::Integer => "integer",
        ScalarType::Number => "number",
    }
}

/// Compiles the incoming plan against the native symbol bindings and the
/// public symbol allocation. Receipts the v1 helpers cannot decode produce
/// source-linked `sdk-incoming-*` errors instead of silent skips.
pub(super) fn prepare(
    contract: &Contract,
    plan: &IncomingPlan,
    request_symbols: &BTreeMap<SchemaId, String>,
    response_symbols: &BTreeMap<SchemaId, String>,
    public: &mut BTreeSet<String>,
    errors: &mut Vec<HttpDiagnostic>,
) -> Vec<IncomingEmission> {
    let mut result = Vec::new();
    for operation in plan.operations() {
        let source = operation.source().use_site().source().clone();
        let refuse = |errors: &mut Vec<HttpDiagnostic>, code: &'static str, message: &str| {
            errors.push(super::diag(contract, source.clone(), code, message));
        };
        let stem = pascal(operation.name());
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
            public,
        );
        let payload_type = allocate(&format!("{stem}Payload"), public);
        let route_const = allocate(&format!("{stem}WebhookRoute"), public);
        let decode = allocate(&format!("decode{stem}Webhook"), public);
        let construct = allocate(&format!("construct{stem}Response"), public);
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
                    Some(Representation::Json { codec: Some(codec) }) => {
                        match response_symbols.get(codec.schema().id()) {
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
                            .and_then(|codec| response_symbols.get(codec.schema().id()).cloned()),
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
                let body = match response
                    .media()
                    .first()
                    .map(protocol::MediaPlan::representation)
                {
                    None => None,
                    Some(Representation::Json { codec: Some(codec) }) => {
                        match request_symbols.get(codec.schema().id()) {
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
        result.push(IncomingEmission {
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
            source: src(&source),
            descriptor_key,
            route_const,
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

/// The TypeScript type of one decoded payload.
fn payload_annotation(payload: &Payload) -> String {
    match payload {
        Payload::None => "undefined".into(),
        Payload::Json { codec }
        | Payload::Text {
            codec: Some(codec), ..
        } => {
            format!("Models.{codec}")
        }
        Payload::SchemaFreeJson => "JsonValue".into(),
        Payload::Text { codec: None, .. } => "string".into(),
        Payload::Binary => "Uint8Array".into(),
    }
}

/// The declared body lines of one decoder.
fn decode_body(entry: &IncomingEmission) -> String {
    let mut code = String::new();
    if entry.required_body {
        code.push_str("    if (body.length === 0) throw new IncomingRequestError('the declared required receipt body is absent');\n");
    }
    match &entry.payload {
        Payload::None => code.push_str("    return undefined;\n"),
        Payload::Json { codec } => code.push_str(&format!(
            "    return validated((json) => Codecs.{codec}Codec.decode(json), bodyText(body), {});\n",
            q(&entry.source)
        )),
        Payload::SchemaFreeJson => {
            code.push_str("    return parsedJson(bodyText(body));\n");
        }
        Payload::Text { scalar, codec } => match codec {
            Some(codec) => code.push_str(&format!(
                "    const wire = scalarWire(bodyText(body), {scalar});\n    return validated((json) => Codecs.{codec}Codec.decode(json), stringifyJson(wire), {});\n",
                q(&entry.source)
            )),
            None => code.push_str("    return bodyText(body);\n"),
        },
        Payload::Binary => {
            code.push_str("    if (!isBytes(body)) throw new IncomingRequestError('the declared binary receipt payload requires raw Uint8Array bytes');\n    return body;\n");
        }
    }
    code
}

/// One receipt's payload type and route constant.
fn type_and_route(entry: &IncomingEmission) -> String {
    let mut code = String::new();
    code.push_str(&crate::typescript::declaration_comment(
        &format!(
            "The decoded payload of the {} {:?} receipt. The provider sends this body; the decoder validates it against the declared schema.",
            entry.kind, entry.name
        ),
        &entry.source,
        &entry.explanations.join(" "),
    ));
    code.push_str(&format!(
        "export type {} = {};\n",
        entry.payload_type,
        payload_annotation(&entry.payload)
    ));
    code.push_str(&crate::typescript::declaration_comment(
        &format!(
            "The declared receipt route of the {} {:?} receipt. `method` is the verb the provider sends; the route string is carried verbatim from the source declaration.",
            entry.kind, entry.name
        ),
        &entry.source,
        "Register a handler at this route in your framework; runtime expression substitution is a framework concern.",
    ));
    code.push_str(&format!(
        "export const {}: IncomingRoute = /* @__PURE__ */ Object.freeze({{ method: {}, route: {}, expression: {} }} as const);\n",
        entry.route_const,
        q(&entry.method),
        q(&entry.route),
        entry.expression,
    ));
    code
}

/// One receipt's decoder.
fn decoder(entry: &IncomingEmission) -> String {
    let mut code = crate::typescript::declaration_comment(
        &format!(
            "Decodes one received {} {:?} request. Declared required headers are presence-checked (a missing one throws the branded IncomingRequestError) and the declared body decodes through the operation's own compiled codec machinery.",
            entry.kind, entry.name
        ),
        &entry.source,
        &entry.explanations.join(" "),
    );
    code.push_str(&format!(
        "export function {}(headers: Readonly<Record<string, string>>, body: Uint8Array | string): {} {{\n",
        entry.decode, entry.payload_type,
    ));
    for header in &entry.required_headers {
        code.push_str(&format!("    requireHeader(headers, {});\n", q(header)));
    }
    code.push_str(&decode_body(entry));
    code.push_str("}\n");
    code
}

/// One receipt's reply constructor, when an exact 2xx response is declared.
fn constructor(entry: &IncomingEmission, response: &ConstructedResponse) -> String {
    let Some(construct) = &entry.construct else {
        return String::new();
    };
    let has_body = response.body.is_some();
    let has_headers = !response.headers.is_empty();
    let result_type = match &response.body {
        Some(ConstructedBody::Json { codec }) => format!("Models.{codec}"),
        Some(ConstructedBody::SchemaFreeJson) => "JsonValue".into(),
        Some(ConstructedBody::Binary) => "Uint8Array".into(),
        None => "void".into(),
    };
    let mut parameters = String::new();
    if has_body {
        parameters.push_str(&format!("result: {result_type}"));
        if has_headers {
            parameters.push_str(", ");
        }
    }
    if has_headers {
        parameters.push_str("typedHeaders?: Record<string, string>");
    }
    let mut code = crate::typescript::declaration_comment(
        &format!(
            "Constructs the declared {} reply of the {} {:?} receipt: the returned record carries the declared status, the encoded body and the applied declared headers.",
            response.status, entry.kind, entry.name
        ),
        &entry.source,
        &entry.explanations.join(" "),
    );
    code.push_str(&format!(
        "export function {construct}({parameters}): ConstructedResponse {{\n"
    ));
    if has_body {
        if matches!(response.body, Some(ConstructedBody::Binary)) {
            code.push_str("    if (!isBytes(result)) throw new IncomingRequestError('the declared binary reply requires raw Uint8Array bytes');\n");
        }
        if matches!(response.body, Some(ConstructedBody::SchemaFreeJson)) {
            code.push_str("    let reply: string;\n    try { reply = stringifyJson(result); }\n    catch (error) { throw new IncomingRequestError('the reply value is not representable JSON', { cause: error }); }\n");
        }
    }
    if has_headers {
        let optional = response
            .headers
            .iter()
            .filter(|header| !response.required_headers.contains(header))
            .cloned()
            .collect::<Vec<_>>();
        code.push_str("    const headers: Record<string, string> = {};\n");
        if !optional.is_empty() {
            code.push_str(&format!(
                "    for (const name of [{}]) {{\n        const value = replyHeader(typedHeaders, name);\n        if (value !== undefined) headers[name] = value;\n    }}\n",
                optional.iter().map(|h| q(h)).collect::<Vec<_>>().join(", ")
            ));
        }
        if !response.required_headers.is_empty() {
            code.push_str(&format!(
                "    for (const name of [{}]) {{\n        const value = replyHeader(typedHeaders, name);\n        if (value === undefined) throw new IncomingRequestError(`the declared required reply header ${{name}} is absent`);\n        headers[name] = value;\n    }}\n",
                response.required_headers.iter().map(|h| q(h)).collect::<Vec<_>>().join(", ")
            ));
        }
    }
    let body = match &response.body {
        Some(ConstructedBody::Json { codec }) => {
            format!("Codecs.{codec}Codec.encode(result)")
        }
        Some(ConstructedBody::SchemaFreeJson) => "reply".into(),
        Some(ConstructedBody::Binary) => "result".into(),
        None => "''".into(),
    };
    let headers_expression: String = if has_headers {
        "headers".into()
    } else {
        "{}".into()
    };
    code.push_str(&format!(
        "    return {{ status: {}, headers: {headers_expression}, body: {body} }};\n}}\n",
        response.status,
    ));
    code
}

/// The frozen compiled receipt descriptor map: generated data, never parsed
/// from OpenAPI, and sheddable when the consumer references no receipt helper.
fn descriptor_map(entries: &[IncomingEmission]) -> String {
    let mut code = String::from(
        "/** Frozen compiled receipt descriptors for this package: one entry per declared webhook/callback receipt. `payload` names the decode representation and `reply` the constructed reply representation ('json' | 'schema-free-json' | 'text' | 'binary' | 'none'); codecs name the generated model-codecs exports. Every initializer is annotated pure, so a bundler may shed the whole map (and every receipt's compiled descriptors with it) when the consumer references no receipt helper.\n */\n",
    );
    code.push_str("export const incomingDescriptors = /* @__PURE__ */ Object.freeze({");
    for entry in entries {
        let (reply, reply_status, reply_codec, reply_headers) = match &entry.response {
            None => (
                "none".to_owned(),
                "null".to_owned(),
                "null".to_owned(),
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
                    Some(ConstructedBody::Json { codec }) => q(codec),
                    _ => "null".to_owned(),
                },
                format!(
                    "[{}] as const",
                    response
                        .headers
                        .iter()
                        .map(|header| q(header))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ),
        };
        let payload_codec = match &entry.payload {
            Payload::Json { codec }
            | Payload::Text {
                codec: Some(codec), ..
            } => q(codec),
            _ => "null".to_owned(),
        };
        code.push_str(&format!(
            "\n  {}: /* @__PURE__ */ Object.freeze({{ kind: {}, method: {}, route: {}, expression: {}, source: /* @__PURE__ */ Object.freeze({{ document: {}, pointer: {} }} as const), requiredHeaders: /* @__PURE__ */ Object.freeze([{}] as const), payload: {}, payloadCodec: {}, reply: {}, replyStatus: {}, replyCodec: {}, replyHeaders: /* @__PURE__ */ Object.freeze({}) }} as const),",
            entry.descriptor_key,
            q(entry.kind),
            q(&entry.method),
            q(&entry.route),
            entry.expression,
            q(&entry.document),
            q(&entry.pointer),
            entry
                .required_headers
                .iter()
                .map(|header| q(header))
                .collect::<Vec<_>>()
                .join(", "),
            q(Payload::label(&entry.payload)),
            payload_codec,
            q(&reply),
            reply_status,
            reply_codec,
            reply_headers,
        ));
    }
    code.push_str("\n} as const);\n");
    code
}

/// The generated `typescript/incoming.ts` module: shared receipt library plus
/// the per-receipt compiled helpers and the frozen descriptor map.
pub(super) fn emit(entries: &[IncomingEmission]) -> String {
    let mut code = String::from(LIBRARY);
    for entry in entries {
        code.push_str(&type_and_route(entry));
        code.push_str(&decoder(entry));
        if let Some(response) = &entry.response {
            code.push_str(&constructor(entry, response));
        }
    }
    code.push_str(&descriptor_map(entries));
    code
}

/// The static library half of the generated module.
const LIBRARY: &str = r#"// Generated incoming receipt helpers for this package's declared webhooks and
// callbacks. HTTP semantics are inverted for these receipts: the declared
// method is the verb the provider sends and the declared responses are what
// the handler returns. Decoders presence-check the declared required headers
// and decode the declared body through the same compiled model codecs the
// client uses; constructors build the declared 2xx reply. The compiled receipt
// descriptors below are frozen generated data — the runtime never parses
// OpenAPI. Route expressions ({$request...}) are carried verbatim; their
// substitution is a runtime/framework concern.
import type * as Models from './models.js';
import * as Codecs from './model-codecs.js';
import { parseJson, stringifyJson, type JsonValue } from './json.js';
import { decodeUtf8, isBytes } from './http/common.js';
import { scalarWire } from './http/wire.js';

/** The handler return record for one constructed declared reply. */
export interface ConstructedResponse {
    readonly status: number;
    readonly headers: Record<string, string>;
    readonly body: string | Uint8Array;
}
/** The declared receipt route. `method` is the verb the provider sends. */
export interface IncomingRoute {
    readonly method: string;
    readonly route: string;
    /** True when the declared route carries RFC 6570 runtime expressions; v1 carries it verbatim and substitution is a runtime/framework concern. */
    readonly expression: boolean;
}
/**
 * Branded failure for a received receipt that violates its declared headers or
 * body, or a reply that cannot be constructed. The static runtime's
 * SdkFailureKind union is closed, so this generated class carries the
 * 'incoming-request' brand the same way the generated pagination walker
 * carries PaginationError; use isIncomingRequestError to test caught values.
 */
export class IncomingRequestError extends Error {
    readonly suspectIncomingRequest = true as const;
    readonly incomingKind = 'incoming-request' as const;
    constructor(message: string, options?: { cause?: unknown }) {
        super(message, options);
        this.name = 'IncomingRequestError';
    }
}
/** Tests whether a caught value is the branded incoming-request failure. */
export function isIncomingRequestError(error: unknown): error is IncomingRequestError {
    return error instanceof IncomingRequestError
        || (typeof error === 'object' && error !== null && (error as { readonly suspectIncomingRequest?: unknown }).suspectIncomingRequest === true);
}
/** Reads one declared receipt header case-insensitively. */
function headerValue(headers: Readonly<Record<string, string>>, name: string): string | undefined {
    for (const [key, value] of Object.entries(headers)) if (key.toLowerCase() === name.toLowerCase()) return value;
    return undefined;
}
/** Throws the branded failure when a required declared receipt header is absent. */
function requireHeader(headers: Readonly<Record<string, string>>, name: string): void {
    if (headerValue(headers, name) === undefined) throw new IncomingRequestError(`required receipt header ${name} is absent`);
}
/** Decodes the received body text; invalid UTF-8 is a branded failure. */
function bodyText(body: Uint8Array | string): string {
    if (typeof body === 'string') return body;
    try { return decodeUtf8(body); }
    catch (error) { throw new IncomingRequestError('the received payload is not valid UTF-8', { cause: error }); }
}
/** Decodes one body text through its compiled model codec. */
function validated<T>(decode: (text: string) => T, text: string, source: string): T {
    let decoded: T;
    try { decoded = decode(text); }
    catch (error) { throw new IncomingRequestError(`the received payload does not satisfy its declared schema (${source})`, { cause: error }); }
    return decoded;
}
/** Parses one schema-free JSON payload. */
function parsedJson(text: string): JsonValue {
    try { return parseJson(text, { maxLength: text.length }); }
    catch (error) { throw new IncomingRequestError('the received payload is not valid JSON', { cause: error }); }
}
/** Reads one declared reply header from the caller-provided values. */
function replyHeader(typedHeaders: Record<string, string> | undefined, name: string): string | undefined {
    if (typedHeaders === undefined) return undefined;
    return headerValue(typedHeaders, name);
}
"#;
