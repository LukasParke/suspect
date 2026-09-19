//! Emitted-only incoming receipt helpers for the generated PHP package.
//!
//! The compiled `http_protocol::IncomingPlan` becomes one generated
//! `src/Incoming.php` class: per receipt, a static decoder for what the
//! provider sends (declared required headers are presence-checked
//! case-insensitively and the body decodes through the package's existing
//! codec machinery), a static constructor for the declared exact 2xx reply,
//! the frozen receipt route constants and the compiled descriptors as frozen
//! class constants. HTTP semantics are inverted for these receipts: the
//! declared method is the verb the provider sends and the declared responses
//! are what the handler returns. Static runtime files are never modified, and
//! plans without any incoming declaration emit no file and no bytes at all.
//! Only JSON receipts are representable in the v1 PHP helpers; every other
//! representation is refused with the shared `sdk-incoming-*` plan errors
//! instead of being skipped.

use std::collections::BTreeSet;

use crate::http_contract::HttpDiagnostic;
use crate::http_protocol as protocol;
use crate::http_protocol::{IncomingKind, IncomingPlan, Representation, ResponseStatus};
use crate::php_sdk::models;
use suspect_ir::contract::Contract;

use super::SdkPlan;

/// Class names `src/Incoming.php` owns in the package namespace. Reserved
/// against native model symbols only while receipts are declared, so
/// receipt-less plans reserve nothing.
pub(super) const CLASS_NAMES: &[&str] = &["Incoming", "IncomingRequestException"];

/// How `decode*Webhook` interprets the received body. Only JSON is
/// representable in the PHP v1 receipt helpers; every other representation is
/// refused at plan time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Payload {
    /// No declared request body: the decoder validates headers and returns null.
    None,
    /// JSON through the package's own compiled model codec: `codec` names the
    /// compiled model and `decode` its string-decode method.
    Json { codec: String, decode: String },
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

    /// The compiled model codec of the payload, when it has one.
    fn codec(&self) -> Option<&str> {
        match self {
            Self::Json { codec, .. } => Some(codec),
            Self::None | Self::SchemaFreeJson => None,
        }
    }
}

/// How `construct*Response` encodes the declared reply body. Only JSON is
/// representable in the PHP v1 receipt helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ConstructedBody {
    /// JSON through the package's own compiled model codec: `codec` names the
    /// compiled model and `encode` its string-encode method.
    Json { codec: String, encode: String },
    /// Schema-free JSON: the reply value is encoded as the JSON value.
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

    /// The compiled model codec of the reply, when it has one.
    fn codec(&self) -> Option<&str> {
        match self {
            Self::Json { codec, .. } => Some(codec),
            Self::SchemaFreeJson => None,
        }
    }
}

/// One declared reply the construct helper builds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ConstructedResponse {
    /// The declared exact 2xx status the handler returns.
    pub status: u16,
    pub body: Option<ConstructedBody>,
    /// Declared reply header wire names, in declaration order.
    pub headers: Vec<String>,
    pub required_headers: Vec<String>,
}

/// One incoming operation's emission-ready receipt helpers with allocated,
/// collision-free PHP member names.
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
    /// The `document#pointer` locator embedded in branded failure messages.
    pub source: String,
    /// The compiled descriptor table key; the declared receipt name, which the
    /// shared planner guarantees unique.
    pub descriptor_key: String,
    pub route_const: String,
    pub payload_type: String,
    pub payload_doc: String,
    pub decode: String,
    pub construct: Option<String>,
    pub payload: Payload,
    pub required_body: bool,
    pub required_headers: Vec<String>,
    pub response: Option<ConstructedResponse>,
    pub explanations: Vec<String>,
}

/// The UPPER_SNAKE form of one receipt name, for the route constant.
fn snake_upper(value: &str) -> String {
    let mut out = String::new();
    let mut previous_case = false;
    for character in value.chars() {
        if character.is_ascii_uppercase() {
            if previous_case {
                out.push('_');
            }
            out.push(character);
            previous_case = false;
        } else if character.is_ascii_alphanumeric() {
            out.push(character.to_ascii_uppercase());
            previous_case = character.is_ascii_lowercase();
        } else {
            out.push('_');
            previous_case = false;
        }
    }
    out
}

/// Compiles the incoming plan against the compiled model codecs, allocating
/// every emitted member name. Receipts the v1 helpers cannot express produce
/// source-linked `sdk-incoming-*` errors instead of silent skips.
pub(super) fn prepare(
    contract: &Contract,
    plan: &IncomingPlan,
    models: &models::ModelPlan,
    errors: &mut Vec<HttpDiagnostic>,
) -> Vec<Receipt> {
    let mut used = BTreeSet::new();
    let mut result = Vec::new();
    for operation in plan.operations() {
        let source = operation.source().use_site().source().clone();
        let refuse = |errors: &mut Vec<HttpDiagnostic>, code: &'static str, message: &str| {
            errors.push(crate::php_sdk::diagnostic(contract, source.clone(), code, message));
        };
        let stem = models::pascal(operation.name());
        let descriptor_key = operation.name().clone();
        let route_const = models::allocate(
            &format!("{}_WEBHOOK_ROUTE", snake_upper(operation.name())),
            &mut used,
        );
        let decode = models::allocate(&format!("decode{stem}Webhook"), &mut used);
        let construct = models::allocate(&format!("construct{stem}Response"), &mut used);
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
        let payload_schema = operation.request().body().as_ref().and_then(|body| {
            body.media()
                .first()
                .and_then(|media| match media.representation() {
                    Representation::Json { codec: Some(codec) } => {
                        Some(codec.schema().id().clone())
                    }
                    _ => None,
                })
        });
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
                        let node = models.nodes.get(codec.schema().id());
                        match node {
                            Some(node) => Payload::Json {
                                codec: node.name.clone(),
                                decode: node.codecs.decode.clone(),
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
                    Some(_) => {
                        refuse(
                            errors,
                            "sdk-incoming-receipt-unrepresentable",
                            "the declared incoming body representation has no PHP receipt decode; only JSON receipts are emitted and form, multipart, stream, text and binary receipts are refused",
                        );
                        continue;
                    }
                }
            }
        };
        let payload_type = match &payload_schema {
            Some(id) => models.type_name(id, false),
            None => match &payload {
                Payload::SchemaFreeJson => "JsonValue".to_owned(),
                _ => "null".to_owned(),
            },
        };
        let payload_doc = match &payload_schema {
            Some(id) => models.type_name(id, true),
            None => match &payload {
                Payload::SchemaFreeJson => "JsonValue".to_owned(),
                _ => "null".to_owned(),
            },
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
                        let node = models.nodes.get(codec.schema().id());
                        match node {
                            Some(node) => Some(ConstructedBody::Json {
                                codec: node.name.clone(),
                                encode: node.codecs.encode.clone(),
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
                    Some(_) => {
                        refuse(
                            errors,
                            "sdk-incoming-response-unrepresentable",
                            "the declared reply representation has no PHP constructor; only JSON and body-less replies are emitted",
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
            source: format!("{}#{}", source.document(), source.pointer()),
            descriptor_key,
            route_const,
            payload_type,
            payload_doc,
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

/// Whether the compiled plan emits `src/Incoming.php` at all.
pub(super) fn emittable(plan: &IncomingPlan) -> bool {
    !plan.is_empty()
}

/// The declared body statements of one decoder.
fn decode_body(receipt: &Receipt) -> String {
    let mut code = String::new();
    if receipt.required_body {
        code.push_str(
            "        if ($body === '') { throw new IncomingRequestException('the declared required receipt body is absent'); }\n",
        );
    }
    match &receipt.payload {
        Payload::None => code.push_str("        return null;\n"),
        Payload::SchemaFreeJson => code.push_str(
            "        try { return JsonValue::parse($body, new JsonLimits()); }\n        catch (\\Throwable $error) { throw new IncomingRequestException('the received payload is not valid JSON', 0, $error); }\n",
        ),
        Payload::Json { decode, .. } => code.push_str(&format!(
            "        try {{ return Codecs::{decode}($body); }}\n        catch (\\Throwable $error) {{ throw new IncomingRequestException('the received payload does not satisfy its declared schema ({source})', 0, $error); }}\n",
            decode = decode,
            source = super::emit::php(&receipt.source),
        )),
    }
    code
}

/// One receipt's route constant.
fn route_constant(receipt: &Receipt) -> String {
    let mut code = String::new();
    super::emit::doc_tags(
        &mut code,
        &format!(
            "The declared receipt route of the {} \"{}\" receipt. `method` is the verb the provider sends; the route string is carried verbatim from the source declaration and its runtime expressions are never substituted at generation time. Register a handler at this route in your framework.",
            receipt.kind, receipt.name
        ),
        &[],
    );
    code.push_str(&format!(
        "    public const {name} = ['method' => {method}, 'route' => {route}, 'expression' => {expression}];\n\n",
        name = receipt.route_const,
        method = super::emit::php(&receipt.method),
        route = super::emit::php(&receipt.route),
        expression = receipt.expression,
    ));
    code
}

/// One receipt's decoder.
fn decoder(receipt: &Receipt) -> String {
    let mut summary = format!(
        "Decodes one received {} \"{}\" request. Declared required headers are presence-checked case-insensitively (a missing one throws the typed IncomingRequestException) and the declared body decodes through the package's own compiled codec machinery.",
        receipt.kind, receipt.name
    );
    if !receipt.explanations.is_empty() {
        summary.push(' ');
        summary.push_str(&receipt.explanations.join(" "));
    }
    summary.push_str(&format!("\nSource: {}", receipt.source));
    let mut code = String::new();
    super::emit::doc_tags(
        &mut code,
        &summary,
        &[
            "@param array<string, string> $headers The received request headers.".to_owned(),
            "@param string $body The received request body.".to_owned(),
            format!("@return {}", receipt.payload_doc),
        ],
    );
    code.push_str(&format!(
        "    public static function {decode}(array $headers, string $body): {payload}\n    {{\n",
        decode = receipt.decode,
        payload = receipt.payload_type,
    ));
    for header in &receipt.required_headers {
        code.push_str(&format!(
            "        self::requireHeader($headers, {});\n",
            super::emit::php(header)
        ));
    }
    code.push_str(&decode_body(receipt));
    code.push_str("    }\n\n");
    code
}

/// One receipt's reply constructor, when an exact 2xx response is declared.
fn constructor(receipt: &Receipt, response: &ConstructedResponse) -> String {
    let Some(construct) = &receipt.construct else {
        return String::new();
    };
    let has_body = response.body.is_some();
    let has_headers = !response.headers.is_empty();
    let mut summary = format!(
        "Constructs the declared {} reply of the {} \"{}\" receipt: the returned array carries the declared status, the encoded body and the applied declared headers.",
        response.status, receipt.kind, receipt.name
    );
    if !receipt.explanations.is_empty() {
        summary.push(' ');
        summary.push_str(&receipt.explanations.join(" "));
    }
    summary.push_str(&format!("\nSource: {}", receipt.source));
    let mut tags = Vec::new();
    if has_body {
        tags.push("@param mixed $result The reply value the handler returns.".to_owned());
    }
    if has_headers {
        tags.push("@param array<string, string>|null $typedHeaders Declared reply header values, applied by wire name.".to_owned());
    }
    tags.push("@return array{int, array<string, string>, string}".to_owned());
    let mut code = String::new();
    super::emit::doc_tags(&mut code, &summary, &tags);
    let result_parameter = if has_body { "mixed $result" } else { "" };
    let separator = if has_body && has_headers { ", " } else { "" };
    let headers_parameter = if has_headers {
        "?array $typedHeaders = null"
    } else {
        ""
    };
    code.push_str(&format!(
        "    public static function {construct}({result_parameter}{separator}{headers_parameter}): array\n    {{\n",
        construct = construct,
        result_parameter = result_parameter,
        separator = separator,
        headers_parameter = headers_parameter,
    ));
    if has_headers {
        let optional = response
            .headers
            .iter()
            .filter(|header| !response.required_headers.contains(header))
            .cloned()
            .collect::<Vec<_>>();
        code.push_str("        $headers = [];\n");
        if !optional.is_empty() {
            code.push_str(&format!(
                "        foreach ([{names}] as $name) {{\n            $value = self::headerValue($typedHeaders, $name);\n            if ($value !== null) {{ $headers[$name] = $value; }}\n        }}\n",
                names = optional
                    .iter()
                    .map(|header| super::emit::php(header))
                    .collect::<Vec<_>>()
                    .join(", "),
            ));
        }
        if !response.required_headers.is_empty() {
            code.push_str(&format!(
                "        foreach ([{names}] as $name) {{\n            $value = self::headerValue($typedHeaders, $name);\n            if ($value === null) {{ throw new IncomingRequestException(\"the declared required reply header {{$name}} is absent\"); }}\n            $headers[$name] = $value;\n        }}\n",
                names = response
                    .required_headers
                    .iter()
                    .map(|header| super::emit::php(header))
                    .collect::<Vec<_>>()
                    .join(", "),
            ));
        }
    }
    if has_body {
        match &response.body {
            Some(ConstructedBody::Json { encode, .. }) => code.push_str(&format!(
                "        $reply = Codecs::{encode}($result);\n",
                encode = encode,
            )),
            Some(ConstructedBody::SchemaFreeJson) => code.push_str(
                "        if (!$result instanceof JsonValue) { throw new IncomingRequestException('the declared schema-free JSON reply requires a JsonValue'); }\n        try { $reply = $result->toJson(new JsonLimits()); }\n        catch (\\Throwable $error) { throw new IncomingRequestException('the reply value is not representable JSON', 0, $error); }\n",
            ),
            None => unreachable!("a constructed reply body exists"),
        }
    }
    let headers_expression = if has_headers { "$headers" } else { "[]" };
    let body_expression = if has_body { "$reply" } else { "''" };
    code.push_str(&format!(
        "        return [{status}, {headers}, {body}];\n    }}\n\n",
        status = response.status,
        headers = headers_expression,
        body = body_expression,
    ));
    code
}

/// The frozen compiled receipt descriptor constant: generated data, never
/// parsed from OpenAPI.
fn descriptor_map(receipts: &[Receipt]) -> String {
    let mut code = String::from(
        "    /** Frozen compiled receipt descriptors for this package: one entry per declared\n     * webhook/callback receipt, keyed by the declared receipt name. `payload` names the\n     * decode representation and `reply` the constructed reply representation\n     * (\"json\" | \"schema-free-json\" | \"none\"); codecs name the generated model codec\n     * methods. Every value is generated data - the runtime never parses OpenAPI.\n     */\n",
    );
    code.push_str("    private const RECEIPTS = [\n");
    for receipt in receipts {
        let (reply, reply_status, reply_codec, reply_headers) = match &receipt.response {
            None => ("none".to_owned(), "null".to_owned(), "null".to_owned(), "[]".to_owned()),
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
                    .map_or_else(|| "null".to_owned(), super::emit::php),
                format!(
                    "[{}]",
                    response
                        .headers
                        .iter()
                        .map(|header| super::emit::php(header))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ),
        };
        let payload_codec = receipt
            .payload
            .codec()
            .map_or_else(|| "null".to_owned(), super::emit::php);
        code.push_str(&format!(
            "        {key} => ['kind' => {kind}, 'method' => {method}, 'route' => {route}, 'expression' => {expression}, 'source' => ['document' => {document}, 'pointer' => {pointer}], 'required_headers' => [{required_headers}], 'payload' => {payload}, 'payload_codec' => {payload_codec}, 'reply' => {reply}, 'reply_status' => {reply_status}, 'reply_codec' => {reply_codec}, 'reply_headers' => {reply_headers}],\n",
            key = super::emit::php(&receipt.descriptor_key),
            kind = super::emit::php(receipt.kind),
            method = super::emit::php(&receipt.method),
            route = super::emit::php(&receipt.route),
            expression = receipt.expression,
            document = super::emit::php(&receipt.document),
            pointer = super::emit::php(&receipt.pointer),
            required_headers = receipt
                .required_headers
                .iter()
                .map(|header| super::emit::php(header))
                .collect::<Vec<_>>()
                .join(", "),
            payload = super::emit::php(receipt.payload.label()),
            payload_codec = payload_codec,
            reply = super::emit::php(&reply),
            reply_status = reply_status,
            reply_codec = reply_codec,
            reply_headers = reply_headers,
        ));
    }
    code.push_str("    ];\n\n");
    code
}

/// The generated `src/Incoming.php` file: the branded failure, the shared
/// header helpers, and one route constant, descriptor, decode and construct
/// per compiled receipt. Called only when at least one receipt compiles.
pub(super) fn source(plan: &SdkPlan, receipts: &[Receipt]) -> String {
    let mut out = String::new();
    out.push_str("<?php\ndeclare(strict_types=1);\n\n");
    out.push_str(&format!("namespace {};\n\n", plan.config().namespace));
    out.push_str(HEADER);
    out.push_str(EXCEPTION);
    out.push_str("final class Incoming\n{\n\n");
    for receipt in receipts {
        out.push_str(&route_constant(receipt));
    }
    out.push_str(&descriptor_map(receipts));
    out.push_str(HELPERS);
    for receipt in receipts {
        out.push_str(&decoder(receipt));
        if let Some(response) = &receipt.response {
            out.push_str(&constructor(receipt, response));
        }
    }
    out.push_str("}\n");
    out
}

/// The generated file header.
const HEADER: &str = r#"/**
 * Generated incoming receipt helpers for this package's declared webhooks and
 * callbacks. HTTP semantics are inverted for these receipts: the declared
 * method is the verb the provider sends and the declared responses are what
 * the handler returns. Decoders presence-check the declared required headers
 * and decode the declared body through the same compiled model codecs the
 * client uses; constructors build the declared exact 2xx reply. The compiled
 * receipt descriptors below are frozen generated data - the runtime never
 * parses OpenAPI. Route expressions ({$request...}) are carried verbatim;
 * their substitution is a runtime/framework concern.
 */

"#;

/// The branded receipt failure. The runtime's SdkError kind union is closed,
/// so this generated exception carries the receipt decode/construct failures
/// the same way the generated pagination walker carries its own failure.
const EXCEPTION: &str = r#"/** Typed failure for a received receipt that violates its declared headers or body, or a reply that cannot be constructed. Messages never carry received payload text. */
final class IncomingRequestException extends \RuntimeException
{
}

"#;

/// The shared receipt helpers: case-insensitive header reads with
/// required-presence checks.
const HELPERS: &str = r#"    /** Reads one declared receipt header case-insensitively.
     * @param array<string, string>|null $headers
     */
    private static function headerValue(?array $headers, string $name): ?string
    {
        if ($headers === null) { return null; }
        foreach ($headers as $key => $value) {
            if (strtolower((string) $key) === strtolower($name) && is_string($value)) { return $value; }
        }
        return null;
    }

    /** Throws the typed failure when a required declared receipt header is absent.
     * @param array<string, string> $headers
     */
    private static function requireHeader(array $headers, string $name): void
    {
        if (self::headerValue($headers, $name) === null) {
            throw new IncomingRequestException("required receipt header {$name} is absent");
        }
    }

"#;
