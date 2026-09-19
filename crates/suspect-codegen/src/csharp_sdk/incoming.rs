//! Emitted-only incoming receipt helpers for the C# HTTP adapter.
//!
//! The compiled `http_protocol::IncomingPlan` becomes one generated
//! `src/Incoming.g.cs` file: per receipt, a typed decoder for what the provider
//! sends (declared required headers are presence-checked and the body decodes
//! through the package's existing model codec machinery), a constructor for the
//! declared exact 2xx reply, and the frozen receipt route plus the frozen
//! compiled receipt descriptors. HTTP semantics are inverted for these
//! receipts: the declared method is the verb the provider sends and the
//! declared responses are what the handler returns. The v1 helpers decode
//! declared JSON payloads only. Static runtime files are never modified, and
//! plans without any incoming declaration emit no file and no bytes at all.

use super::{
    HttpDiagnostic, SdkPlan,
    emit::{quote, xml},
    models,
};
use crate::http_protocol as p;
use crate::http_protocol::{IncomingKind, IncomingPlan, Representation, ResponseStatus};
use std::collections::BTreeSet;
use suspect_ir::contract::{Contract, SchemaId};

/// How `Decode*Webhook` interprets the received body. The v1 helpers decode
/// declared JSON payloads only: through the schema-bound model codec, or as the
/// schema-free JSON element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Payload {
    /// No declared request body: the decoder validates headers and returns.
    None,
    /// JSON through the operation's own model codec.
    Json { schema: SchemaId },
    /// Schema-free JSON: the decoded value is the parsed JSON element.
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
}

/// How `Construct*Response` encodes the declared reply body.
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

/// One incoming operation's emission-ready receipt helpers with reserved,
/// collision-free public C# names.
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
    pub payload: Payload,
    pub required_body: bool,
    pub required_headers: Vec<String>,
    pub response: Option<Reply>,
    pub explanations: Vec<String>,
}

/// Compiles the incoming plan against the native model bindings and the
/// emitted symbol allocation. Receipts the v1 helpers cannot express produce
/// source-linked `sdk-incoming-*` errors instead of silent skips.
pub(super) fn prepare(
    contract: &Contract,
    plan: &IncomingPlan,
    models: &models::ModelPlan,
) -> Result<Vec<Receipt>, Vec<HttpDiagnostic>> {
    let mut errors = Vec::new();
    let mut used: BTreeSet<String> = models
        .names
        .values()
        .cloned()
        .chain(super::reserved_types().into_iter().map(str::to_owned))
        .chain([
            "Incoming".to_owned(),
            "IncomingRequestException".to_owned(),
            "IncomingRoute".to_owned(),
            "IncomingSource".to_owned(),
            "IncomingDescriptor".to_owned(),
        ])
        .collect();
    let mut result = Vec::new();
    for operation in plan.operations() {
        let source = operation.source().use_site().source().clone();
        let refuse = |errors: &mut Vec<HttpDiagnostic>, code: &'static str, message: &str| {
            errors.push(super::diagnostic(contract, source.clone(), code, message));
        };
        let stem = super::exported(operation.name());
        let descriptor_key = super::allocate(
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
            &mut used,
        );
        let route_const = super::allocate(&format!("{stem}WebhookRoute"), &mut used);
        let decode = super::allocate(&format!("Decode{stem}Webhook"), &mut used);
        let construct = super::allocate(&format!("Construct{stem}Response"), &mut used);
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
                        let schema = codec.schema().id().clone();
                        if !models.names.contains_key(&models::key(&schema)) {
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
                            "the declared incoming body representation has no v1 receipt decode; the C# receipt helpers decode declared JSON payloads only",
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
                        let schema = codec.schema().id().clone();
                        if !models.names.contains_key(&models::key(&schema)) {
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
                            "the declared reply representation has no v1 constructor; the C# receipt helpers encode declared JSON replies only",
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
            location: super::emit::source(&source),
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
        });
    }
    if errors.is_empty() {
        Ok(result)
    } else {
        Err(errors)
    }
}

/// The admitted receipts; planning already refused every unrepresentable one.
pub(super) fn compiled(plan: &SdkPlan) -> Vec<Receipt> {
    prepare(plan.contract(), plan.incoming(), plan.models())
        .expect("receipts admitted at plan time")
}

/// Whether any receipt lowers into an emitted `src/Incoming.g.cs`.
pub(super) fn emits(plan: &SdkPlan) -> bool {
    !plan.incoming().is_empty() && !compiled(plan).is_empty()
}

/// The generated `src/Incoming.g.cs`: the shared receipt types plus one frozen
/// route, decoder and reply constructor per receipt and the frozen descriptor
/// map.
pub(super) fn module(plan: &SdkPlan) -> String {
    let receipts = compiled(plan);
    let mut out = super::emit::header(plan);
    out.push_str(LIBRARY);
    for receipt in &receipts {
        out.push_str(&route_constant(receipt));
        out.push_str(&decoder(plan, receipt));
        if let Some(reply) = &receipt.response {
            out.push_str(&constructor(plan, receipt, reply));
        }
    }
    out.push_str(&descriptors(plan, &receipts));
    out.push_str(HELPERS);
    out.push_str("}\n");
    out
}

/// The summary paragraphs shared by every generated helper. The lead text is
/// XML-escaped whole, so debug-quoted names stay representable.
fn summary(receipt: &Receipt, lead: &str) -> String {
    let mut text = format!("{} Source: {}.", xml(lead), xml(&receipt.location));
    if !receipt.explanations.is_empty() {
        text.push(' ');
        text.push_str(&xml(&receipt.explanations.join(" ")));
    }
    text
}

/// One receipt's frozen route constant.
fn route_constant(receipt: &Receipt) -> String {
    format!(
        "    /// <summary>{summary}</summary>\n    public static readonly IncomingRoute {route_const} = new({method}, {route}, {expression});\n\n",
        summary = summary(
            receipt,
            &format!(
                "The declared receipt route of the {} {:?} receipt. `Method` is the verb the provider sends; `Path` is the webhook key or the callback expression exactly as declared. Register a handler at this route in your framework; runtime expression substitution is a framework concern.",
                receipt.kind, receipt.name,
            )
        ),
        route_const = receipt.route_const,
        method = quote(&receipt.method),
        route = quote(&receipt.route),
        expression = receipt.expression,
    )
}

/// The native C# type of one decoded payload.
fn payload_type(plan: &SdkPlan, payload: &Payload) -> String {
    match payload {
        Payload::None => "void".into(),
        Payload::SchemaFreeJson => "global::System.Text.Json.JsonElement".into(),
        Payload::Json { schema } => plan.models.native_type(schema),
    }
}

/// The native C# type of one constructed reply value.
fn reply_type(plan: &SdkPlan, body: &ConstructedBody) -> String {
    match body {
        ConstructedBody::SchemaFreeJson => "global::System.Text.Json.JsonElement".into(),
        ConstructedBody::Json { schema } => plan.models.native_type(schema),
    }
}

/// One receipt's decoder.
fn decoder(plan: &SdkPlan, receipt: &Receipt) -> String {
    let mut code = format!(
        "    /// <summary>{summary}</summary>\n    public static {ty} {decode}(IReadOnlyDictionary<string, string> headers, ReadOnlyMemory<byte> body)\n    {{\n",
        summary = summary(
            receipt,
            &format!(
                "Decodes one received {} {:?} request. Declared required headers are presence-checked (a missing one throws IncomingRequestException) and the declared body decodes through the operation's own compiled codec machinery.",
                receipt.kind, receipt.name,
            )
        ),
        ty = payload_type(plan, &receipt.payload),
        decode = receipt.decode,
    );
    for header in &receipt.required_headers {
        code.push_str(&format!(
            "        RequireHeader(headers, {});\n",
            quote(header)
        ));
    }
    if receipt.required_body {
        code.push_str(
            "        if (body.Length == 0) throw new IncomingRequestException(\"the declared required receipt body is absent\");\n",
        );
    }
    match &receipt.payload {
        Payload::None => code.push_str("        return;\n"),
        Payload::Json { schema } => code.push_str(&format!(
            "        try\n        {{\n            return Codecs.Decode{codec}(body.Span);\n        }}\n        catch (CodecException error)\n        {{\n            throw new IncomingRequestException(\"the received payload does not satisfy its declared schema ({location})\", error);\n        }}\n",
            codec = plan.models.codec_name(schema),
            location = xml(&receipt.location),
        )),
        Payload::SchemaFreeJson => code.push_str(&format!(
            "        try\n        {{\n            return JsonRuntime.Parse(body.Span);\n        }}\n        catch (CodecException error)\n        {{\n            throw new IncomingRequestException(\"the received payload is not valid JSON ({location})\", error);\n        }}\n",
            location = xml(&receipt.location),
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
            reply_type(plan, reply.body.as_ref().expect("declared body"))
        ));
        if has_headers {
            parameters.push_str(", ");
        }
    }
    if has_headers {
        parameters.push_str("IReadOnlyDictionary<string, string>? typedHeaders = null");
    }
    let mut code = format!(
        "    /// <summary>{summary}</summary>\n    public static (int Status, Dictionary<string, string> Headers, byte[] Body) {construct}({parameters})\n    {{\n        var headers = new Dictionary<string, string>(StringComparer.Ordinal);\n",
        summary = summary(
            receipt,
            &format!(
                "Constructs the declared {} reply of the {} {:?} receipt: the returned tuple carries the declared status, the applied declared reply headers and the encoded body.",
                reply.status, receipt.kind, receipt.name,
            )
        ),
        construct = receipt.construct,
        parameters = parameters,
    );
    if has_headers {
        let optional = reply
            .headers
            .iter()
            .filter(|header| !reply.required_headers.contains(header))
            .cloned()
            .collect::<Vec<_>>();
        if !optional.is_empty() {
            code.push_str(&format!(
                "        foreach (var name in new[] {{ {} }})\n        {{\n            var value = HeaderValue(typedHeaders, name);\n            if (value is not null) headers[name] = value;\n        }}\n",
                optional.iter().map(|h| quote(h)).collect::<Vec<_>>().join(", ")
            ));
        }
        if !reply.required_headers.is_empty() {
            code.push_str(&format!(
                "        foreach (var name in new[] {{ {} }})\n        {{\n            if (HeaderValue(typedHeaders, name) is not {{ }} found) throw new IncomingRequestException($\"the declared required reply header {{name}} is absent\");\n            headers[name] = found;\n        }}\n",
                reply
                    .required_headers
                    .iter()
                    .map(|h| quote(h))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    match &reply.body {
        None => code.push_str(&format!(
            "        return ({status}, headers, []);\n    }}\n\n",
            status = reply.status,
        )),
        Some(ConstructedBody::Json { schema }) => code.push_str(&format!(
            "        return ({status}, headers, Codecs.Encode{codec}(result));\n    }}\n\n",
            status = reply.status,
            codec = plan.models.codec_name(schema),
        )),
        Some(ConstructedBody::SchemaFreeJson) => code.push_str(&format!(
            "        byte[] body;\n        try\n        {{\n            body = ProtocolRuntime.EncodeJson(result);\n        }}\n        catch (Exception error) when (error is not AuthException)\n        {{\n            throw new IncomingRequestException(\"the reply value is not representable JSON\", error);\n        }}\n        return ({status}, headers, body);\n    }}\n\n",
            status = reply.status,
        )),
    }
    code
}

/// The declared header list of one descriptor entry.
fn headers_array(headers: &[String]) -> String {
    if headers.is_empty() {
        "global::System.Array.Empty<string>()".into()
    } else {
        format!(
            "new[] {{ {} }}",
            headers
                .iter()
                .map(|h| quote(h))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

/// The frozen compiled receipt descriptor map: generated data, never parsed
/// from OpenAPI.
fn descriptors(plan: &SdkPlan, receipts: &[Receipt]) -> String {
    let mut code = String::from(
        "    /// <summary>Frozen compiled receipt descriptors for this package: one entry per declared webhook/callback receipt. `Payload` names the decode representation and `Reply` the constructed reply representation ('json' | 'schema-free-json' | 'none'); the codec members name the generated model codec exports.</summary>\n    public static readonly IReadOnlyDictionary<string, IncomingDescriptor> Receipts = new global::System.Collections.ObjectModel.ReadOnlyDictionary<string, IncomingDescriptor>(new Dictionary<string, IncomingDescriptor>(StringComparer.Ordinal)\n    {\n",
    );
    for receipt in receipts {
        let (reply, reply_status, reply_codec, reply_headers) = match &receipt.response {
            None => (
                "none".to_owned(),
                "null".to_owned(),
                "null".to_owned(),
                headers_array(&[]),
            ),
            Some(response) => (
                response
                    .body
                    .as_ref()
                    .map_or_else(|| "none".to_owned(), |body| body.label().to_owned()),
                response.status.to_string(),
                match &response.body {
                    Some(ConstructedBody::Json { schema }) => quote(plan.models.codec_name(schema)),
                    _ => "null".to_owned(),
                },
                headers_array(&response.headers),
            ),
        };
        let payload_codec = match &receipt.payload {
            Payload::Json { schema } => quote(plan.models.codec_name(schema)),
            _ => "null".to_owned(),
        };
        code.push_str(&format!(
            "        [{}] = new IncomingDescriptor({}, {}, {}, {}, new IncomingSource({}, {}), {}, {}, {}, {}, {}, {}, {}),\n",
            quote(&receipt.descriptor_key),
            quote(receipt.kind),
            quote(&receipt.method),
            quote(&receipt.route),
            receipt.expression,
            quote(&receipt.document),
            quote(&receipt.pointer),
            headers_array(&receipt.required_headers),
            quote(Payload::label(&receipt.payload)),
            payload_codec,
            quote(&reply),
            reply_status,
            reply_codec,
            reply_headers,
        ));
    }
    code.push_str("    });\n\n");
    code
}

/// The shared case-insensitive receipt header readers, emitted once at the end
/// of the generated class.
const HELPERS: &str = r#"    /// <summary>Reads one declared receipt header case-insensitively; absent stays null.</summary>
    internal static string? HeaderValue(IReadOnlyDictionary<string, string>? headers, string name)
    {
        if (headers is null) return null;
        foreach (var pair in headers)
        {
            if (StringComparer.OrdinalIgnoreCase.Equals(pair.Key, name)) return pair.Value;
        }
        return null;
    }

    /// <summary>Throws the typed failure when a required declared receipt header is absent.</summary>
    internal static void RequireHeader(IReadOnlyDictionary<string, string> headers, string name)
    {
        if (HeaderValue(headers, name) is null) throw new IncomingRequestException($"required receipt header {name} is absent");
    }
"#;

/// The static library half of the generated file: the generated-data comment,
/// the shared receipt types and the opening of the generated class.
const LIBRARY: &str = r#"// Generated incoming receipt helpers for this package's declared webhooks and
// callbacks. HTTP semantics are inverted for these receipts: the declared
// method is the verb the provider sends and the declared responses are what
// the handler returns. Decoders presence-check the declared required headers
// and decode the declared body through the same compiled model codecs the
// client uses; constructors build the declared 2xx reply. The compiled receipt
// descriptors are frozen generated data - the runtime never parses OpenAPI.
// Route expressions ({$request...}) are carried verbatim; their substitution
// is a runtime/framework concern.

/// <summary>The declared receipt route of one registered handler. `Method` is the verb the provider sends; `Path` is the webhook key or the callback expression exactly as declared, carried verbatim.</summary>
public sealed record IncomingRoute(string Method, string Path, bool Expression);

/// <summary>Typed failure for a received receipt that violates its declared headers or body, or for a declared reply that cannot be constructed.</summary>
public sealed class IncomingRequestException : Exception
{
    /// <summary>Construct the typed failure; the optional cause carries the underlying codec failure.</summary>
    public IncomingRequestException(string message, Exception? cause = null) : base(message, cause) { }
}

/// <summary>Original document URI and schema pointer of one receipt declaration.</summary>
public sealed record IncomingSource(string Document, string Pointer);

/// <summary>One compiled receipt's frozen descriptor: exactly what the source declared plus the compiled codec bindings. `Payload` names the decode representation ('json' | 'schema-free-json' | 'none') and `Reply` the constructed reply representation ('json' | 'schema-free-json' | 'none'); the codec members name the generated model codec exports.</summary>
public sealed record IncomingDescriptor(string Kind, string Method, string Path, bool Expression, IncomingSource Source, IReadOnlyList<string> RequiredHeaders, string Payload, string? PayloadCodec, string Reply, int? ReplyStatus, string? ReplyCodec, IReadOnlyList<string> ReplyHeaders);

/// <summary>Generated receipt helpers for this package's declared webhooks and callbacks: one frozen route, one decoder and one reply constructor per receipt, plus the frozen compiled descriptor map.</summary>
public static class Incoming
{
"#;
