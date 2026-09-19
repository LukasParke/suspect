//! Emitted-only incoming receipt helpers for the native Dart HTTP adapter.
//!
//! The compiled `http_protocol::IncomingPlan` becomes one generated
//! `lib/src/incoming.dart` part: per receipt, a decoder for what the provider
//! sends (declared required headers are presence-checked and the body decodes
//! through the package's existing codec classes), a constructor for the
//! declared exact 2xx reply and the frozen receipt route. HTTP semantics are
//! inverted for these receipts: the declared method is the verb the provider
//! sends and the declared responses are what the handler returns. Route
//! expressions (`{$request...}`) are carried verbatim; their substitution is a
//! runtime/framework concern.
//!
//! Static runtime files are never modified: the typed
//! `IncomingException` extends the package's sealed `SdkException` exactly the
//! way the generated `PaginationException` does, which is possible because
//! every emitted part shares the generated library. Receipts the v1 helpers
//! cannot decode produce source-linked `sdk-incoming-*` plan errors instead of
//! silent skips, and plans without any incoming declaration emit no file and
//! no bytes at all.

use super::{
    HttpDiagnostic, Plan, diag,
    emit::{doc, quote},
    models::{ModelPlan, allocate, exported, member},
};
use crate::http_protocol::{
    self as p, IncomingKind, IncomingPlan, Representation, ResponseStatus, ScalarType,
};
use std::collections::BTreeSet;
use std::fmt::Write;
use suspect_ir::contract::Contract;

/// How `decode*Webhook` interprets the received body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Payload {
    /// No declared request body: the decoder validates headers and returns void.
    None,
    /// JSON through the package's own compiled model codec.
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
    /// The declared type of the decoded payload.
    fn annotation(&self, native_type: impl Fn() -> String) -> String {
        match self {
            Self::None => "void".into(),
            Self::Json { .. } | Self::Text { codec: Some(_), .. } => native_type(),
            Self::SchemaFreeJson => "JsonValue".into(),
            Self::Text { codec: None, .. } => "String".into(),
            Self::Binary => "Uint8List".into(),
        }
    }
}

/// How `construct*Response` encodes the declared reply body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstructedBody {
    /// JSON through the package's own compiled model codec.
    Json { codec: String },
    /// Schema-free JSON: the reply value is encoded as the JSON value.
    SchemaFreeJson,
    /// Opaque declared bytes, passed through unchanged.
    Binary,
}

/// One declared reply the construct helper builds: the first declared exact
/// 2xx response.
#[derive(Debug, Clone)]
pub struct ConstructedResponse {
    pub status: u16,
    pub body: Option<ConstructedBody>,
    /// The result parameter's exact native type.
    pub result_type: String,
    /// Declared reply header wire names, in declaration order.
    pub headers: Vec<String>,
    pub required_headers: Vec<String>,
}

/// One incoming operation's emission-ready receipt helpers with allocated,
/// collision-free Dart names.
#[derive(Debug, Clone)]
pub struct Receipt {
    /// Webhook key, or `parentOperationId.callbackName`.
    pub name: String,
    pub kind: &'static str,
    /// The verb the provider sends.
    pub method: String,
    pub route: String,
    pub expression: bool,
    /// Source location for documentation.
    pub location: String,
    /// The decoded-payload type alias.
    pub payload_type: String,
    /// The declared payload type.
    pub payload_annotation: String,
    /// The route constant's allocated name.
    pub route_const: String,
    pub decode: String,
    pub construct: Option<String>,
    pub payload: Payload,
    pub required_body: bool,
    pub required_headers: Vec<String>,
    pub response: Option<ConstructedResponse>,
    pub explanations: Vec<String>,
}

/// The prepared incoming emission: the allocated typed exception extending the
/// package's `SdkException`, plus one receipt per compiled incoming operation.
#[derive(Debug, Clone)]
pub struct Prepared {
    /// The allocated exception class name, usually `IncomingException`.
    pub exception_type: String,
    pub receipts: Vec<Receipt>,
}

fn scalar_name(scalar: ScalarType) -> &'static str {
    match scalar {
        ScalarType::String => "string",
        ScalarType::Boolean => "boolean",
        ScalarType::Integer => "integer",
        ScalarType::Number => "number",
    }
}

/// Compiles the incoming plan against the compiled model codecs, allocating
/// every emitted Dart name into `names`. Receipts the v1 helpers cannot
/// decode produce source-linked `sdk-incoming-*` errors instead of silent
/// skips; a plan with errors carries no receipts at all.
pub(super) fn prepare(
    contract: &Contract,
    incoming: &IncomingPlan,
    models: &ModelPlan,
    names: &mut BTreeSet<String>,
) -> Result<Prepared, Vec<HttpDiagnostic>> {
    let mut errors = Vec::new();
    let exception_type = allocate("IncomingException", names);
    let mut receipts = Vec::new();
    for operation in incoming.operations() {
        let source = operation.source().use_site().source().clone();
        let refuse = |errors: &mut Vec<HttpDiagnostic>, code: &'static str, message: &str| {
            errors.push(diag(contract, source.clone(), code, message));
        };
        let stem = exported(operation.name());
        let payload_type = allocate(&format!("{stem}Payload"), names);
        let route_const =
            allocate(&format!("{}WebhookRoute", member(operation.name())), names);
        let decode = allocate(&format!("decode{stem}Webhook"), names);
        let construct = allocate(&format!("construct{stem}Response"), names);
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
                match body
                    .media()
                    .first()
                    .map(p::MediaPlan::representation)
                {
                    None => Payload::None,
                    Some(Representation::Json { codec: Some(codec) }) => {
                        let Some(model) = models.model(codec.schema().id()) else {
                            refuse(
                                &mut errors,
                                "sdk-incoming-codec-binding",
                                "the declared incoming payload schema has no compiled model codec",
                            );
                            continue;
                        };
                        Payload::Json {
                            codec: model.codec_name.clone(),
                        }
                    }
                    Some(Representation::Json { codec: None }) => Payload::SchemaFreeJson,
                    Some(Representation::Text { codec, scalar, .. }) => Payload::Text {
                        scalar: scalar_name(*scalar),
                        codec: codec.as_ref().and_then(|codec| {
                            models
                                .model(codec.schema().id())
                                .map(|model| model.codec_name.clone())
                        }),
                    },
                    Some(Representation::Binary { .. }) => Payload::Binary,
                    Some(_) => {
                        refuse(
                            &mut errors,
                            "sdk-incoming-receipt-unrepresentable",
                            "the declared incoming body representation has no v1 receipt decode; form, multipart and stream receipts are refused",
                        );
                        continue;
                    }
                }
            }
        };
        let payload_annotation = payload.annotation(|| {
            operation
                .request()
                .body()
                .as_ref()
                .and_then(|body| body.media().first())
                .and_then(|media| match media.representation() {
                    Representation::Json { codec: Some(codec) }
                    | Representation::Text {
                        codec: Some(codec), ..
                    } => models.native_type(codec.schema().id()),
                    _ => None,
                })
                .expect("codec-bound payload annotation")
        });
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
                    .map(p::MediaPlan::representation)
                {
                    None => None,
                    Some(Representation::Json { codec: Some(codec) }) => {
                        let Some(model) = models.model(codec.schema().id()) else {
                            refuse(
                                &mut errors,
                                "sdk-incoming-codec-binding",
                                "the declared reply schema has no compiled model codec",
                            );
                            continue;
                        };
                        Some(ConstructedBody::Json {
                            codec: model.codec_name.clone(),
                        })
                    }
                    Some(Representation::Json { codec: None }) => {
                        Some(ConstructedBody::SchemaFreeJson)
                    }
                    Some(Representation::Binary { .. }) => Some(ConstructedBody::Binary),
                    Some(_) => {
                        refuse(
                            &mut errors,
                            "sdk-incoming-response-unrepresentable",
                            "the declared reply representation has no v1 constructor; only JSON, binary and body-less replies are emitted",
                        );
                        continue;
                    }
                };
                let result_type = response
                    .media()
                    .first()
                    .and_then(|media| match media.representation() {
                        Representation::Json { codec: Some(codec) }
                        | Representation::Text {
                            codec: Some(codec), ..
                        } => models.native_type(codec.schema().id()),
                        Representation::Binary { .. } => Some("Uint8List".into()),
                        _ => None,
                    })
                    .unwrap_or_default();
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
                    result_type,
                    headers,
                    required_headers,
                })
            }
        };
        receipts.push(Receipt {
            name: operation.name().clone(),
            kind: match operation.kind() {
                IncomingKind::Webhook => "webhook",
                IncomingKind::Callback => "callback",
            },
            method: operation.method().as_str().to_owned(),
            route: operation.route().route().clone(),
            expression: operation.route().expression(),
            location: format!(
                "{}#{}",
                source.document(),
                source.pointer()
            ),
            payload_type,
            payload_annotation,
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
    if errors.is_empty() {
        Ok(Prepared {
            exception_type,
            receipts,
        })
    } else {
        Err(errors)
    }
}

/// The shared helper pieces the compiled receipts actually use. Every private
/// helper is emitted only when a receipt references it, so the emitted part
/// never carries dead members.
fn shared_helpers(prepared: &Prepared) -> String {
    let exceptions = &prepared.exception_type;
    let require_header = prepared
        .receipts
        .iter()
        .any(|receipt| !receipt.required_headers.is_empty());
    let reply_headers = prepared.receipts.iter().any(|receipt| {
        receipt
            .response
            .as_ref()
            .is_some_and(|response| !response.headers.is_empty())
    });
    let decoded = prepared.receipts.iter().any(|receipt| {
        matches!(
            receipt.payload,
            Payload::Json { .. } | Payload::SchemaFreeJson | Payload::Text { codec: Some(_), .. }
        )
    });
    let text = prepared
        .receipts
        .iter()
        .any(|receipt| matches!(receipt.payload, Payload::Text { .. }));
    let reply_json = prepared.receipts.iter().any(|receipt| {
        receipt.response.as_ref().is_some_and(|response| {
            matches!(response.body, Some(ConstructedBody::SchemaFreeJson))
        })
    });
    let mut out = String::new();
    if require_header || reply_headers {
        out.push_str("/// Reads one declared receipt header case-insensitively.\nString? _incomingHeaderValue(Map<String, String> headers, String name) {\n  for (final entry in headers.entries) {\n    if (entry.key.toLowerCase() == name.toLowerCase()) {\n      return entry.value;\n    }\n  }\n  return null;\n}\n\n");
    }
    if require_header {
        out.push_str(&format!(
            "/// Throws the typed failure when a required declared receipt header is absent.\nvoid _incomingRequireHeader(Map<String, String> headers, String name) {{\n  if (_incomingHeaderValue(headers, name) == null) {{\n    throw {exceptions}('required receipt header $name is absent');\n  }}\n}}\n\n"
        ));
    }
    if decoded {
        out.push_str(&format!(
            "/// Decodes one received payload through the package's codec machinery,\n/// wrapping decode failures in the typed incoming-receipt exception.\nT _incomingDecoded<T>(T Function() decode) {{\n  try {{\n    return decode();\n  }} on CodecException catch (error) {{\n    throw {exceptions}('the received payload does not satisfy its declared schema', codecFailure: error);\n  }} on JsonException {{\n    throw const {exceptions}('the received payload is not readable JSON');\n  }}\n}}\n\n"
        ));
    }
    if text {
        out.push_str(&format!(
            "/// Decodes the received body text; invalid UTF-8 is a typed failure.\nString _incomingText(Uint8List body) {{\n  try {{\n    return utf8.decode(body, allowMalformed: false);\n  }} on FormatException {{\n    throw const {exceptions}('the received payload is not valid UTF-8');\n  }}\n}}\n\n"
        ));
    }
    if reply_headers {
        out.push_str("/// Reads one declared reply header from the caller-provided values.\nString? _incomingReplyHeader(Map<String, String>? typedHeaders, String name) {\n  if (typedHeaders == null) {\n    return null;\n  }\n  return _incomingHeaderValue(typedHeaders, name);\n}\n\n");
    }
    if reply_json {
        out.push_str(&format!(
            "/// Encodes one schema-free reply value into bounded reply bytes.\nUint8List _incomingReplyJsonBytes(JsonValue value) {{\n  try {{\n    return Uint8List.fromList(utf8.encode(writeJson(value, limits: _encodeLimits)));\n  }} on JsonException {{\n    throw const {exceptions}('the reply value is not representable JSON');\n  }}\n}}\n\n"
        ));
    }
    out
}

/// One receipt's payload type alias and frozen route constant.
fn type_and_route(receipt: &Receipt) -> String {
    let mut code = String::new();
    let mut summary = match receipt.payload {
        Payload::None => format!(
            "The decoded payload of the {} {:?} receipt: the receipt declares no request body, so the decoder validates the declared headers and returns no value.",
            receipt.kind, receipt.name
        ),
        _ => format!(
            "The decoded payload of the {} {:?} receipt: the provider sends this body and the decoder validates it against the declared schema.",
            receipt.kind, receipt.name
        ),
    };
    if !receipt.explanations.is_empty() {
        summary.push(' ');
        summary.push_str(&receipt.explanations.join(" "));
    }
    summary.push_str(&format!(" Source: {}.", receipt.location));
    doc(&mut code, &summary, "");
    writeln!(
        code,
        "typedef {} = {};",
        receipt.payload_type, receipt.payload_annotation
    )
    .unwrap();
    code.push('\n');
    doc(
        &mut code,
        &format!(
            "The declared receipt route of the {} {:?} receipt: `method` is the verb the provider sends and the route string is carried verbatim from the source declaration. Register a handler at this route in your framework; runtime expression substitution is a framework concern. Source: {}.",
            receipt.kind, receipt.name, receipt.location
        ),
        "",
    );
    writeln!(
        code,
        "const ({{String method, String route, bool expression}}) {} = (method: {}, route: {}, expression: {});",
        receipt.route_const,
        quote(&receipt.method),
        quote(&receipt.route),
        receipt.expression,
    )
    .unwrap();
    code
}

/// The declared body statements of one decoder.
fn decode_body(receipt: &Receipt, exceptions: &str) -> String {
    let mut code = String::new();
    if receipt.required_body {
        writeln!(
            code,
            "  if (body.isEmpty) {{\n    throw const {exceptions}('the declared required receipt body is absent');\n  }}"
        )
        .unwrap();
    }
    match &receipt.payload {
        Payload::None => code.push_str(
            "  // The receipt declares no request body; only the declared headers are checked.\n",
        ),
        Payload::Json { codec } => writeln!(
            code,
            "  return _incomingDecoded(() => {codec}.decodeBytes(body));"
        )
        .unwrap(),
        Payload::SchemaFreeJson => writeln!(
            code,
            "  return _incomingDecoded(() => parseJsonBytes(body, limits: _decodeLimits));"
        )
        .unwrap(),
        Payload::Text { scalar, codec } => match codec {
            Some(codec) => writeln!(
                code,
                "  final text = _incomingText(body);\n  return _incomingDecoded(() => {codec}.fromJson(_textJson(text, _Scalar.{scalar})));"
            )
            .unwrap(),
            None => writeln!(code, "  return _incomingText(body);").unwrap(),
        },
        Payload::Binary => code.push_str("  return body;\n"),
    }
    code
}

/// One receipt's decoder.
fn decoder(receipt: &Receipt, exceptions: &str) -> String {
    let mut code = String::new();
    let mut summary = format!(
        "Decodes one received {} {:?} request: declared required headers are presence-checked (a missing one throws the typed {}) and the declared body decodes through the package's own compiled codec machinery.",
        receipt.kind, receipt.name, exceptions
    );
    if !receipt.explanations.is_empty() {
        summary.push(' ');
        summary.push_str(&receipt.explanations.join(" "));
    }
    summary.push_str(&format!(" Source: {}.", receipt.location));
    doc(&mut code, &summary, "");
    writeln!(
        code,
        "{} {}(Map<String, String> headers, Uint8List body) {{",
        receipt.payload_type, receipt.decode
    )
    .unwrap();
    for header in &receipt.required_headers {
        writeln!(
            code,
            "  _incomingRequireHeader(headers, {});",
            quote(header)
        )
        .unwrap();
    }
    code.push_str(&decode_body(receipt, exceptions));
    code.push_str("}\n");
    code
}

/// One receipt's reply constructor, when an exact 2xx response is declared.
fn constructor(receipt: &Receipt, response: &ConstructedResponse, exceptions: &str) -> String {
    let Some(construct) = &receipt.construct else {
        return String::new();
    };
    let has_body = response.body.is_some();
    let has_headers = !response.headers.is_empty();
    let mut code = String::new();
    let mut summary = format!(
        "Constructs the declared {} reply of the {} {:?} receipt: the returned record carries the declared status, the encoded body and the applied declared headers.",
        response.status, receipt.kind, receipt.name
    );
    if !receipt.explanations.is_empty() {
        summary.push(' ');
        summary.push_str(&receipt.explanations.join(" "));
    }
    summary.push_str(&format!(" Source: {}.", receipt.location));
    doc(&mut code, &summary, "");
    let mut parameters = String::new();
    if has_body {
        parameters.push_str(&format!("{} result", response.result_type));
        if has_headers {
            parameters.push_str(", ");
        }
    }
    if has_headers {
        parameters.push_str("{Map<String, String>? typedHeaders}");
    }
    writeln!(
        code,
        "({{int status, Map<String, String> headers, Uint8List body}}) {construct}({parameters}) {{"
    )
    .unwrap();
    if has_headers {
        let optional = response
            .headers
            .iter()
            .filter(|header| !response.required_headers.contains(header))
            .collect::<Vec<_>>();
        code.push_str("  final headers = <String, String>{};\n");
        if !optional.is_empty() {
            code.push_str(&format!(
                "  for (final name in const [{}]) {{\n    final value = _incomingReplyHeader(typedHeaders, name);\n    if (value != null) {{\n      headers[name] = value;\n    }}\n  }}\n",
                optional
                    .iter()
                    .map(|header| quote(header))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !response.required_headers.is_empty() {
            code.push_str(&format!(
                "  for (final name in const [{}]) {{\n    final value = _incomingReplyHeader(typedHeaders, name);\n    if (value == null) {{\n      throw {exceptions}('the declared required reply header $name is absent');\n    }}\n    headers[name] = value;\n  }}\n",
                response
                    .required_headers
                    .iter()
                    .map(|header| quote(header))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    let body = match &response.body {
        Some(ConstructedBody::Json { codec }) => format!("{codec}.encodeBytes(result)"),
        Some(ConstructedBody::SchemaFreeJson) => "_incomingReplyJsonBytes(result)".into(),
        Some(ConstructedBody::Binary) => "result".into(),
        None => "Uint8List(0)".into(),
    };
    let headers: String = if has_headers {
        "headers".into()
    } else {
        "const <String, String>{}".into()
    };
    writeln!(
        code,
        "  return (status: {}, headers: {}, body: {});",
        response.status, headers, body
    )
    .unwrap();
    code.push_str("}\n");
    code
}

/// The generated `lib/src/incoming.dart` part: the typed receipt exception,
/// the shared helpers the compiled receipts use, and one payload alias, route
/// constant, decoder and reply constructor per receipt.
pub(super) fn emission(plan: &Plan) -> Option<String> {
    let prepared = &plan.incoming_receipts;
    if prepared.receipts.is_empty() {
        return None;
    }
    let exceptions = &prepared.exception_type;
    let mut part = format!(
        "/// A received receipt request violated its declared headers or body, or a\n/// declared reply could not be constructed. It extends the package's\n/// SdkException hierarchy, so receipt decoding composes with the same error\n/// handling as every client call, and messages stay free of received payload\n/// text.\nfinal class {exceptions} extends SdkException {{\n  const {exceptions}(this.message, {{this.codecFailure}});\n\n  final String message;\n\n  /// The compiled codec failure when the received payload failed its declared\n  /// schema; null for header, body-presence and reply-construction failures.\n  final CodecException? codecFailure;\n\n  @override\n  String toString() => '{exceptions}: $message';\n}}\n\n"
    );
    part.push_str(&shared_helpers(prepared));
    for receipt in &prepared.receipts {
        part.push_str(&type_and_route(receipt));
        part.push('\n');
        part.push_str(&decoder(receipt, exceptions));
        part.push('\n');
        if let Some(response) = &receipt.response {
            part.push_str(&constructor(receipt, response, exceptions));
            part.push('\n');
        }
    }
    Some(part)
}
