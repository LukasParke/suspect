//! Native names and types over the admitted shared protocol descriptors.
//! No HTTP semantics are recovered from emitted Swift or a second admission plan.
use std::collections::BTreeSet;

use crate::http_protocol::*;
use suspect_ir::contract::Contract;

use super::{
    HttpDiagnostic, SwiftConfig, allocate, diagnostic, exported, member, models::ModelPlan,
};

pub(super) fn capabilities(config: &SwiftConfig) -> Capabilities {
    use Capability::*;
    let mut result = Capabilities::for_adapter(
        "swift-http-protocol-v1",
        [
            UnnamedOperations,
            AdditionalMethods,
            CustomMethods,
            HttpServers,
            RelativeServers,
            DocumentRelativeServers,
            MultipleServers,
            ServerVariables,
            AnonymousSecurity,
            SecurityAlternatives,
            ConjunctiveSecurity,
            HttpBasic,
            ApiKeys,
            OAuth2,
            OpenIdConnect,
            SecurityRoles,
            ParameterStyles,
            HeaderParameters,
            CookieParameters,
            ReservedParameters,
            ContentParameters,
            QuerystringParameters,
            QuerystringForm,
            RangeResponses,
            DefaultResponses,
            MultipleMediaTypes,
            MediaRanges,
            MediaTypeParameters,
            StructuredJsonMedia,
            SchemaFreeJson,
            TextBodies,
            BinaryBodies,
            UndeclaredResponseBody,
            ResponseHeaders,
            ResponseLinks,
            FormBodies,
            MultipartBodies,
            PartEncodings,
            PositionalMultipart,
            ServerSentEvents,
            JsonLines,
            OpenApi30,
            OpenApi32,
            SchemaResources,
            DynamicSchemaReferences,
        ],
    )
    .with_limits(ByteLimits::new(
        config.max_request_bytes.max(config.max_response_bytes) as u64,
        config.max_part_bytes as u64,
        config.max_stream_item_bytes as u64,
    ));
    for profile in &config.compatibility_profiles {
        result = result.with_profile(*profile);
    }
    result
}

// These names belong to the transport projection, not the model compiler. The
// existing typed-name table permits collision allocation without changing any
// schema layout, source binding, validation instruction or codec implementation.
pub(super) const RUNTIME_NAMES: &[&str] = &[
    "Attribution",
    "HTTPNoContent",
    "HTTPMediaType",
    "HTTPServer",
    "HTTPServerVariable",
    "HTTPProvenance",
    "HTTPResourceContext",
    "HTTPURLBase",
    "HTTPLocated",
    "HTTPLink",
    "HTTPLinkTarget",
    "HTTPCredentialContext",
    "HTTPOAuthFlow",
    "HTTPPermission",
    "HTTPBasicCredential",
    "HTTPAuthorizationProvider",
    "HTTPAuthValue",
    "HTTPRequirement",
    "HTTPSecurity",
    "HTTPParameter",
    "HTTPStyle",
    "HTTPWireShape",
    "HTTPScalar",
    "HTTPPercentEncoding",
    "HTTPSerialization",
    "HTTPWire",
    "HTTPWireBuffer",
    "HTTPWireFailure",
    "HTTPPart",
    "HTTPPartRule",
    "HTTPParts",
    "HTTPRawPart",
    "HTTPObjectRules",
    "HTTPBody",
    "HTTPResponseRule",
    "HTTPMetadata",
    "HTTPStreamResponse",
    "HTTPByteStream",
    "HTTPEventStream",
    "HTTPStreamLease",
    "HTTPByteState",
    "HTTPEventReader",
    "HTTPFramer",
    "HTTPURLSessionStreamTransfer",
    "HTTPStreamFraming",
    "AsyncSequence",
    "AsyncIteratorProtocol",
    "UUID",
    "NSCondition",
    "DispatchQueue",
    "DispatchTime",
    "HTTPByteOnce",
    "HTTPPositionalRules",
    "HTTPExactMethodTransport",
    "HTTPExactConnection",
    "HTTPExactResponseReader",
    "HTTPExactRequestHead",
    "Network",
    "Security",
    "NWConnection",
    "NWEndpoint",
    "NWParameters",
    "NWProtocolTLS",
    "NWProtocolTCP",
    "NWError",
    "DispatchWorkItem",
];

pub(super) fn reserve_runtime_names(models: &mut ModelPlan) {
    let mut used: BTreeSet<String> = models.names.values().cloned().collect();
    used.extend(RUNTIME_NAMES.iter().map(|v| (*v).to_owned()));
    for name in models.names.values_mut() {
        if RUNTIME_NAMES.contains(&name.as_str()) {
            *name = allocate(name, &mut used);
        }
    }
}

pub(super) fn admit(contract: &Contract, plan: &ProtocolPlan) -> Result<(), Vec<HttpDiagnostic>> {
    let mut findings = Vec::new();
    for op in plan.operations() {
        for media in op
            .body()
            .into_iter()
            .flat_map(|b| b.media())
            .chain(op.responses().iter().flat_map(|r| r.media()))
        {
            if let Representation::Multipart { multipart } = media.representation() {
                let (parts, additional) = match multipart {
                    MultipartPlan::Named {
                        parts, additional, ..
                    } => (parts.as_slice(), additional),
                    MultipartPlan::Positional { prefix, items, .. } => (prefix.as_slice(), items),
                };
                let form_data = matches!(media.media_type().range(), MediaRange::Concrete { type_name, subtype } if type_name == "multipart" && subtype == "form-data");
                for part in parts.iter().chain(match additional {
                    AdditionalParts::Allowed(p) => Some(p.as_ref()),
                    _ => None,
                }) {
                    if matches!(part.representation(), PartRepresentation::Style { .. })
                        && (!form_data || super::protocol_positional::style_expands(part))
                    {
                        let source = part
                            .encoding_source()
                            .map(|p| p.use_site().source())
                            .unwrap_or_else(|| part.source().use_site().source());
                        findings.push(diagnostic(contract, source.clone(), "swift-multipart-style-grouping-unsupported", "MIME style values require a form-data context and a single-part value expansion; multi-part style grouping is not inferred from positional or named schemas"));
                    }
                }
            }
        }
        if let Some(body) = op.body() {
            for media in body.media() {
                if matches!(media.representation(), Representation::Stream { .. }) {
                    findings.push(diagnostic(contract, media.source().use_site().source().clone(),
                        "swift-stream-request-unsupported", "Swift sequential media currently supports responses; streaming request producers require a separate upload lifetime contract"));
                }
            }
        }
        for header in op.responses().iter().flat_map(|r| r.headers()) {
            if header.name().eq_ignore_ascii_case("set-cookie") {
                findings.push(diagnostic(contract, header.source().use_site().source().clone(),
                    "swift-set-cookie-header-unsupported", "URLSession does not expose uncombined repeated Set-Cookie fields; a source-typed Set-Cookie decoder cannot recover their boundaries"));
            }
        }
        for media in op.responses().iter().flat_map(|r| r.media()) {
            if let Representation::Form { form } = media.representation() {
                for part in form.fields().iter().chain(match form.additional() {
                    AdditionalParts::Allowed(p) => Some(p.as_ref()),
                    _ => None,
                }) {
                    if matches!(
                        part.representation(),
                        PartRepresentation::Style {
                            serialization: ParameterSerialization::Style {
                                shape: WireShape::Array { .. } | WireShape::FlatObject { .. },
                                ..
                            },
                            ..
                        }
                    ) {
                        findings.push(diagnostic(contract, part.source().use_site().source().clone(), "swift-form-response-style-unsupported", "composite RFC6570 form response decoding needs an unambiguous field-grouping descriptor; use the supported content-based field encoding"));
                    }
                }
            }
        }
    }
    if findings.is_empty() {
        Ok(())
    } else {
        Err(findings)
    }
}

#[derive(Debug, Clone)]
pub(super) struct PlannedCredential {
    pub property: String,
    pub requirement: CredentialRequirement,
}
impl PlannedCredential {
    pub fn ty(&self) -> &'static str {
        match self.requirement.credential() {
            CredentialHook::Basic => "HTTPBasicCredential",
            CredentialHook::OAuth2 { .. } | CredentialHook::OpenIdConnect { .. } => {
                "HTTPAuthorizationProvider"
            }
            _ => "String",
        }
    }
}

/// One typed response or part header property.
#[derive(Debug, Clone)]
pub struct PlannedHeader {
    /// Shared wire decisions for the header.
    pub wire: HeaderPlan,
    /// Swift property name, unique within its headers type.
    pub field_name: String,
    /// Swift type of the header codec schema.
    pub type_name: String,
}

/// One form field or multipart part.
#[derive(Debug, Clone)]
pub struct PlannedPart {
    /// Shared wire decisions for the part.
    pub wire: PartPlan,
    /// Swift property name, unique within its aggregate.
    pub field_name: String,
    /// One part's native payload, without optional/repeated or metadata wrappers.
    pub value_type: String,
    /// One part input, including multipart filename/media/typed headers.
    pub item_type: String,
    /// Property type: `item_type`, as an array for repeated array items.
    pub type_name: String,
    /// Generated part headers type, when headers are declared.
    pub header_type: Option<String>,
    /// Part header properties in declaration order.
    pub headers: Vec<PlannedHeader>,
}

/// Generated URL-encoded form or named multipart body type.
#[derive(Debug, Clone)]
pub struct PlannedParts {
    /// Generated struct name.
    pub type_name: String,
    /// Multipart rather than URL-encoded form.
    pub multipart: bool,
    /// Object rules the runtime enforces over fields.
    pub rules: ObjectRules,
    /// Declared fields in source order.
    pub fields: Vec<PlannedPart>,
    /// Additional-property part; `None` when additional fields are forbidden.
    pub additional: Option<Box<PlannedPart>>,
}

/// Whole-query form encoder for a `content`-typed form query parameter.
#[derive(Debug, Clone)]
pub struct PlannedQueryForm {
    /// Shared form encoding decisions.
    pub wire: FormPlan,
    /// Generated encoder name.
    pub encoder_name: String,
}

/// Ordered, finite MIME positions. The aggregate is never a JSON codec input.
#[derive(Debug, Clone)]
pub struct PlannedPositionalParts {
    /// Generated struct name.
    pub type_name: String,
    /// Array schema the parts are drawn from.
    pub schema: SchemaUse,
    /// Positional prefix parts (`part1`, `part2`, ...).
    pub prefix: Vec<PlannedPart>,
    /// Part for the remaining items; `None` when extra items are forbidden.
    pub items: Option<Box<PlannedPart>>,
    /// Source `minItems`, when declared.
    pub min_items: Option<Located<u64>>,
    /// Source `maxItems`, when declared.
    pub max_items: Option<Located<u64>>,
    /// Whether the media type is exactly `multipart/form-data`.
    pub form_data: bool,
}

/// One request or response media type and its native value.
#[derive(Debug, Clone)]
pub struct PlannedMedia {
    /// Shared wire decisions for the media.
    pub wire: MediaPlan,
    /// Enum case name derived from the representation, unique within its body
    /// or response.
    pub case_name: String,
    /// Swift type of the media value.
    pub type_name: String,
    /// Named form or multipart aggregate, when the representation is one.
    pub parts: Option<PlannedParts>,
    /// Positional multipart aggregate, when the representation is one.
    pub positional: Option<PlannedPositionalParts>,
}

/// Operation request body as the input's `body` field.
#[derive(Debug, Clone)]
pub struct PlannedBody {
    /// Shared wire decisions for the body.
    pub wire: BodyPlan,
    /// Type accepted by the input's body field (before OptionalField).
    pub type_name: String,
    /// Planned media in protocol order.
    pub media: Vec<PlannedMedia>,
    /// Whether `type_name` is a generated enum over media, used for several
    /// media or any media range.
    pub is_enum: bool,
}

/// One declared response status as a result or API error case.
#[derive(Debug, Clone)]
pub struct PlannedResponse {
    /// Shared wire decisions for the response.
    pub wire: ResponsePlan,
    /// Enum case name (`status<key>` or `defaultResponse`).
    pub case_name: String,
    /// Body payload or discriminated media/no-content union.
    pub type_name: String,
    /// Complete response, `APIResponse<T>` or a wrapper with typed headers.
    pub response_type: String,
    /// Generated response headers type, when headers are declared.
    pub header_type: Option<String>,
    /// Response header properties in declaration order.
    pub headers: Vec<PlannedHeader>,
    /// Planned media; empty when the body is always empty.
    pub media: Vec<PlannedMedia>,
    /// Whether `type_name` is a generated enum over media and no-content.
    pub is_enum: bool,
    /// Whether HTTP forbids a body: `HEAD`, 1xx, 204, 205 or 304.
    pub always_empty: bool,
    /// Whether the body may be absent: `always_empty`, `2XX`, `3XX` or `default`.
    pub may_be_empty: bool,
}
impl PlannedResponse {
    /// Whether this status can yield a success: 2xx, `2XX` or `default`.
    pub fn may_succeed(&self) -> bool {
        matches!(
            self.wire.status(),
            ResponseStatus::Exact(200..=299) | ResponseStatus::Range(2) | ResponseStatus::Default
        )
    }
    /// Whether this status can yield an error: anything but 2xx or `2XX`.
    pub fn may_fail(&self) -> bool {
        !matches!(
            self.wire.status(),
            ResponseStatus::Exact(200..=299) | ResponseStatus::Range(2)
        )
    }
}

pub(super) fn headers(wire: &[HeaderPlan], models: &ModelPlan) -> Vec<PlannedHeader> {
    let mut used = BTreeSet::new();
    wire.iter()
        .map(|h| PlannedHeader {
            wire: h.clone(),
            field_name: allocate(&member(h.name()), &mut used),
            type_name: models.ty(h.codec().schema().id()),
        })
        .collect()
}

fn part(
    p: &PartPlan,
    stem: &str,
    multipart: bool,
    models: &ModelPlan,
    names: &mut BTreeSet<String>,
    members: &mut BTreeSet<String>,
) -> PlannedPart {
    let value_type = match p.representation() {
        PartRepresentation::Json { codec, .. }
        | PartRepresentation::Text { codec, .. }
        | PartRepresentation::Style { codec, .. } => models.ty(codec.schema().id()),
        PartRepresentation::Binary { .. } => "Data".into(),
    };
    let header_type = (!p.headers().is_empty()).then(|| allocate(&format!("{stem}Headers"), names));
    let item_type = if !multipart {
        value_type.clone()
    } else if header_type.is_some() {
        allocate(&format!("{stem}Part"), names)
    } else {
        format!("HTTPPart<{value_type}>")
    };
    let type_name = if p.multiplicity() == PartMultiplicity::RepeatedArrayItems {
        format!("[{item_type}]")
    } else {
        item_type.clone()
    };
    PlannedPart {
        wire: p.clone(),
        field_name: allocate(&member(p.name().unwrap_or("additionalProperties")), members),
        value_type,
        item_type,
        type_name,
        header_type,
        headers: headers(p.headers(), models),
    }
}

fn parts(
    rules: &ObjectRules,
    fields: &[PartPlan],
    additional: &AdditionalParts,
    multipart: bool,
    stem: &str,
    models: &ModelPlan,
    names: &mut BTreeSet<String>,
) -> PlannedParts {
    let type_name = allocate(&format!("{stem}Body"), names);
    let mut members = BTreeSet::from(["additionalProperties".into()]);
    let fields = fields
        .iter()
        .map(|p| {
            part(
                p,
                &format!("{stem}{}", exported(p.name().unwrap_or("Part"))),
                multipart,
                models,
                names,
                &mut members,
            )
        })
        .collect();
    let additional = match additional {
        AdditionalParts::Forbidden => None,
        AdditionalParts::Allowed(p) => Some(Box::new(part(
            p,
            &format!("{stem}Additional"),
            multipart,
            models,
            names,
            &mut members,
        ))),
    };
    PlannedParts {
        type_name,
        multipart,
        rules: rules.clone(),
        fields,
        additional,
    }
}

pub(super) fn query_form(
    parameter: &ParameterPlan,
    stem: &str,
    names: &mut BTreeSet<String>,
) -> Option<PlannedQueryForm> {
    let Representation::Form { form } = parameter.content_media()?.representation() else {
        return None;
    };
    Some(PlannedQueryForm {
        wire: form.clone(),
        encoder_name: allocate(
            &format!("{stem}{}QueryForm", exported(parameter.name())),
            names,
        ),
    })
}

fn positional(
    wire: &MultipartPlan,
    media: &MediaType,
    stem: &str,
    models: &ModelPlan,
    names: &mut BTreeSet<String>,
) -> PlannedPositionalParts {
    let MultipartPlan::Positional {
        schema,
        prefix,
        items,
        min_items,
        max_items,
    } = wire
    else {
        unreachable!("typed positional plan")
    };
    let stem = format!("{stem}Multipart");
    let type_name = allocate(&format!("{stem}Body"), names);
    let mut members = BTreeSet::from(["items".into(), "encode".into(), "decode".into()]);
    let prefix = prefix
        .iter()
        .enumerate()
        .map(|(index, p)| {
            let mut native = part(
                p,
                &format!("{stem}Part{}", index + 1),
                true,
                models,
                names,
                &mut BTreeSet::new(),
            );
            native.field_name = allocate(&format!("part{}", index + 1), &mut members);
            native
        })
        .collect();
    let items = match items {
        AdditionalParts::Forbidden => None,
        AdditionalParts::Allowed(p) => Some(Box::new(part(
            p,
            &format!("{stem}Item"),
            true,
            models,
            names,
            &mut BTreeSet::new(),
        ))),
    };
    PlannedPositionalParts {
        type_name,
        schema: schema.clone(),
        prefix,
        items,
        min_items: min_items.clone(),
        max_items: max_items.clone(),
        form_data: matches!(media.range(), MediaRange::Concrete { type_name, subtype } if type_name == "multipart" && subtype == "form-data"),
    }
}

pub(super) fn media(
    wire: &[MediaPlan],
    stem: &str,
    models: &ModelPlan,
    names: &mut BTreeSet<String>,
) -> Vec<PlannedMedia> {
    let mut cases = BTreeSet::from(["none".into()]);
    wire.iter()
        .map(|m| {
            let mut ordered = None;
            let (label, type_name, parts) = match m.representation() {
                Representation::Json { codec } => (
                    "json",
                    codec
                        .as_ref()
                        .map(|c| models.ty(c.schema().id()))
                        .unwrap_or_else(|| "JsonValue".into()),
                    None,
                ),
                Representation::Text { codec, .. } => (
                    "text",
                    codec
                        .as_ref()
                        .map(|c| models.ty(c.schema().id()))
                        .unwrap_or_else(|| "String".into()),
                    None,
                ),
                Representation::Binary { .. } => ("bytes", "Data".into(), None),
                Representation::Form { form } => {
                    let part_stem = if stem.ends_with("Form") {
                        stem.to_owned()
                    } else {
                        format!("{stem}Form")
                    };
                    let p = parts(
                        form.rules(),
                        form.fields(),
                        form.additional(),
                        false,
                        &part_stem,
                        models,
                        names,
                    );
                    ("form", p.type_name.clone(), Some(p))
                }
                Representation::Multipart {
                    multipart:
                        MultipartPlan::Named {
                            rules,
                            parts: fields,
                            additional,
                        },
                } => {
                    let p = parts(
                        rules,
                        fields,
                        additional,
                        true,
                        &format!("{stem}Multipart"),
                        models,
                        names,
                    );
                    ("multipart", p.type_name.clone(), Some(p))
                }
                Representation::Multipart { multipart } => {
                    let native = positional(multipart, m.media_type(), stem, models, names);
                    let ty = native.type_name.clone();
                    ordered = Some(native);
                    ("multipart", ty, None)
                }
                Representation::Stream { stream } => match stream.item_codec() {
                    Some(codec) => (
                        if stream.framing() == StreamFraming::ServerSentEvents {
                            "events"
                        } else {
                            "jsonLines"
                        },
                        format!("HTTPEventStream<{}>", models.ty(codec.schema().id())),
                        None,
                    ),
                    // A schemaless stream surfaces untyped whole-body JSON values
                    // because the native stream runtime has no untyped codec.
                    None => ("schemalessStream", "JsonValue".into(), None),
                },
            };
            PlannedMedia {
                wire: m.clone(),
                case_name: allocate(label, &mut cases),
                type_name,
                parts,
                positional: ordered,
            }
        })
        .collect()
}

pub(super) fn body(
    wire: &BodyPlan,
    stem: &str,
    models: &ModelPlan,
    names: &mut BTreeSet<String>,
) -> PlannedBody {
    let media = media(wire.media(), stem, models, names);
    let is_enum = media.len() != 1
        || media
            .iter()
            .any(|m| !matches!(m.wire.media_type().range(), MediaRange::Concrete { .. }));
    let type_name = if is_enum {
        allocate(&format!("{stem}Body"), names)
    } else {
        media[0].type_name.clone()
    };
    PlannedBody {
        wire: wire.clone(),
        type_name,
        media,
        is_enum,
    }
}

pub(super) fn response(
    wire: &ResponsePlan,
    method: &str,
    stem: &str,
    models: &ModelPlan,
    names: &mut BTreeSet<String>,
) -> PlannedResponse {
    let case_name = match wire.status() {
        ResponseStatus::Default => "defaultResponse".into(),
        _ => format!("status{}", wire.status_key()),
    };
    let stem = format!(
        "{stem}{}",
        if wire.status() == ResponseStatus::Default {
            "Default".into()
        } else {
            format!("Status{}", wire.status_key())
        }
    );
    let always_empty = method == "HEAD"
        || matches!(
            wire.status(),
            ResponseStatus::Exact(100..=199 | 204 | 205 | 304) | ResponseStatus::Range(1)
        );
    let may_be_empty = always_empty
        || matches!(
            wire.status(),
            ResponseStatus::Default | ResponseStatus::Range(2 | 3)
        );
    let media = if always_empty {
        Vec::new()
    } else {
        media(wire.media(), &stem, models, names)
    };
    let is_enum = !always_empty && (media.len() > 1 || may_be_empty);
    let type_name = if is_enum {
        allocate(&format!("{stem}Body"), names)
    } else if always_empty {
        "HTTPNoContent".into()
    } else {
        media
            .first()
            .map(|m| m.type_name.clone())
            .unwrap_or_else(|| "Data".into())
    };
    let header_type =
        (!wire.headers().is_empty()).then(|| allocate(&format!("{stem}Headers"), names));
    let response_type = if header_type.is_some() {
        allocate(&format!("{stem}Response"), names)
    } else {
        format!("APIResponse<{type_name}>")
    };
    PlannedResponse {
        wire: wire.clone(),
        case_name,
        type_name,
        response_type,
        header_type,
        headers: headers(wire.headers(), models),
        media,
        is_enum,
        always_empty,
        may_be_empty,
    }
}
