use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use suspect_ir::contract::{Operation, SourceId};

use super::planner::Planner;
use super::*;

#[derive(Clone)]
struct Metadata {
    origin: Provenance,
    name: String,
    location: ParameterLocation,
    required: bool,
    deprecated: bool,
    description: Option<Located<String>>,
    ignored: bool,
    examples: ExampleMetadata,
}

pub(super) fn plan(p: &mut Planner<'_>, op: &Operation<'_>) -> Vec<ParameterPlan> {
    let mut declarations = BTreeMap::new();
    // Validate both declaration levels before consulting the override-filtered
    // view. Duplicate/malformed overridden declarations still have sources.
    for collection in op.parameter_collections() {
        let Some(values) = p.contract.source(&collection).and_then(Value::as_array) else {
            p.error(
                &collection,
                "http-parameters-invalid",
                "parameters must be an array",
            );
            continue;
        };
        let mut identities = BTreeSet::new();
        for (index, _) in values.iter().enumerate() {
            let source = collection.child(&index.to_string());
            if let Some(metadata) = metadata(p, &source, None) {
                if !identities.insert((metadata.name.clone(), metadata.location)) {
                    p.error(&source, "http-parameter-duplicate", "duplicate (name, in) at one declaration level; operation-level overrides are only between levels");
                }
                declarations.insert(source, metadata);
            }
        }
    }
    let mut result = Vec::new();
    let mut wire_identities = BTreeSet::new();
    let mut querystring_source = None;
    let mut query_source = None;
    for parameter in op.parameters() {
        let Some(meta) = declarations.get(parameter.source()).cloned() else {
            continue;
        };
        if meta.ignored {
            continue;
        }
        match meta.location {
            ParameterLocation::Querystring => {
                p.require(Capability::QuerystringParameters, parameter.source());
                if querystring_source
                    .replace(parameter.source().clone())
                    .is_some()
                {
                    p.error(parameter.source(), "http-querystring-duplicate", "there may be only one effective querystring parameter, regardless of its name");
                }
            }
            ParameterLocation::Query => {
                query_source = Some(parameter.source().clone());
            }
            _ => {}
        }
        let identity = (
            meta.location,
            if meta.location == ParameterLocation::Header {
                meta.name.to_ascii_lowercase()
            } else {
                meta.name.clone()
            },
        );
        if !wire_identities.insert(identity) {
            p.error(parameter.source(), "http-parameter-wire-duplicate", "effective parameters target the same wire field (header names are case-insensitive)");
        }
        if meta.location == ParameterLocation::Header {
            p.require(Capability::HeaderParameters, parameter.source());
        }
        if meta.location == ParameterLocation::Cookie {
            p.require(Capability::CookieParameters, parameter.source());
        }
        let Some((codec, serialization, content_media)) =
            serialization(p, &meta.origin.terminal.source, meta.location)
        else {
            continue;
        };
        let baseline = matches!(
            (&serialization, meta.location),
            (
                ParameterSerialization::Style {
                    style: Style::Simple,
                    explode: false,
                    shape: WireShape::Scalar {
                        scalar: ScalarType::String,
                    },
                    percent_encoding: PercentEncoding::UriComponent,
                },
                ParameterLocation::Path,
            ) | (
                ParameterSerialization::Style {
                    style: Style::Form,
                    shape: WireShape::Scalar { .. } | WireShape::Array { .. },
                    percent_encoding: PercentEncoding::UriComponent,
                    ..
                },
                ParameterLocation::Query,
            )
        );
        if !baseline && meta.location != ParameterLocation::Querystring {
            p.require(Capability::ParameterStyles, &meta.origin.terminal.source);
        }
        result.push(ParameterPlan {
            source: meta.origin,
            name: meta.name,
            location: meta.location,
            required: meta.required,
            deprecated: meta.deprecated,
            description: meta.description,
            codec,
            serialization,
            content_media,
            examples: meta.examples,
        });
    }
    if let (Some(querystring), Some(_)) = (querystring_source, query_source) {
        p.error(&querystring, "http-querystring-query-conflict", "ordinary query and whole-querystring parameters cannot coexist in the effective operation, including across path/operation declarations");
    }
    result
}

fn metadata(p: &mut Planner<'_>, source: &SourceId, header_name: Option<&str>) -> Option<Metadata> {
    let origin = p.resolve(source, false)?;
    p.validate_metadata_shape(&origin);
    let at = &origin.terminal.source;
    let raw = p.object(
        at,
        if header_name.is_some() {
            "Header"
        } else {
            "Parameter"
        },
    )?;
    p.known(
        at,
        raw,
        &[
            "name",
            "in",
            "description",
            "required",
            "deprecated",
            "allowEmptyValue",
            "style",
            "explode",
            "allowReserved",
            "schema",
            "content",
            "example",
            "examples",
        ],
    );
    let name = if let Some(name) = header_name {
        for key in ["name", "in", "allowEmptyValue"] {
            if raw.contains_key(key) {
                p.error(
                    &at.child(key),
                    "http-header-field-forbidden",
                    format!("Header Object must not declare {key}"),
                );
            }
        }
        name.to_owned()
    } else {
        p.string(at, "name", true)?.value
    };
    let location = if header_name.is_some() {
        ParameterLocation::Header
    } else {
        let location = p.string(at, "in", true)?;
        match location.value.as_str() {
            "path" => ParameterLocation::Path,
            "query" => ParameterLocation::Query,
            "header" => ParameterLocation::Header,
            "cookie" => ParameterLocation::Cookie,
            "querystring" if p.is_32(at) => ParameterLocation::Querystring,
            _ => {
                p.error(
                    &location.source.source,
                    "http-parameter-location",
                    "unknown parameter location for this OpenAPI version",
                );
                return None;
            }
        }
    };
    if name.is_empty()
        || matches!(
            location,
            ParameterLocation::Header | ParameterLocation::Cookie
        ) && !super::media::token(&name)
    {
        p.error(
            at,
            "http-parameter-name",
            "wire parameter name must be nonempty and a valid token for header/cookie locations",
        );
    }
    let description = p.text(&origin, "description", false);
    let required = p.boolean(at, "required", false);
    let deprecated = p.boolean(at, "deprecated", false);
    p.boolean(at, "explode", false);
    p.boolean(at, "allowReserved", false);
    let allow_empty = p.boolean(at, "allowEmptyValue", false);
    if location == ParameterLocation::Querystring {
        if !raw.contains_key("content") {
            p.error(
                at,
                "http-querystring-content-required",
                "querystring parameters require content",
            );
        }
        for field in ["schema", "style", "explode", "allowReserved"] {
            if raw.contains_key(field) {
                p.error(
                    &at.child(field),
                    "http-querystring-field-forbidden",
                    "schema-style fields are forbidden for querystring parameters",
                );
            }
        }
    }
    if raw.contains_key("allowEmptyValue") && location != ParameterLocation::Query {
        p.error(
            &at.child("allowEmptyValue"),
            "http-parameter-field-location",
            "allowEmptyValue applies only to query parameters",
        );
    }
    if allow_empty {
        p.unsupported(&at.child("allowEmptyValue"), "http-empty-value-policy", "allowEmptyValue interactions with schema and omission are implementation-defined and need a versioned wire policy");
    }
    if location == ParameterLocation::Path && (!required || !raw.contains_key("required")) {
        p.error(
            &if raw.contains_key("required") {
                at.child("required")
            } else {
                at.clone()
            },
            "http-path-parameter-required",
            "path parameters must explicitly declare required: true",
        );
    }
    if let Some(style) = p.string(at, "style", false) {
        parse_style(p, &style.source.source, &style.value, location);
    }
    let ignored = header_name.is_none()
        && location == ParameterLocation::Header
        && ["Accept", "Content-Type", "Authorization"]
            .iter()
            .any(|h| name.eq_ignore_ascii_case(h));
    if ignored {
        p.warn(
            at,
            "http-parameter-ignored",
            "OpenAPI ignores Accept, Content-Type and Authorization header Parameter Objects",
        );
    }
    if !ignored {
        if raw.contains_key("schema") == raw.contains_key("content") {
            p.error(
                at,
                "http-parameter-schema-content",
                "exactly one of schema and content is required",
            );
        }
        if let Some(schema) = raw.get("schema")
            && !schema.is_object()
            && !(schema.is_boolean() && !p.is_30(at))
        {
            p.error(
                &at.child("schema"),
                "http-schema-invalid",
                "parameter/header schema must be a Schema Object allowed by its dialect",
            );
        }
        if let Some(content) = raw.get("content") {
            if !content.is_object() || content.as_object().is_some_and(|m| m.len() != 1) {
                p.error(
                    &at.child("content"),
                    "http-parameter-content",
                    "parameter/header content must contain exactly one media entry",
                );
            }
            for key in ["style", "explode", "allowReserved"] {
                if raw.contains_key(key) {
                    p.error(
                        &at.child(key),
                        "http-content-style-conflict",
                        "schema-style serialization fields cannot be combined with content",
                    );
                }
            }
        }
    }
    if header_name.is_none()
        && location == ParameterLocation::Header
        && name.eq_ignore_ascii_case("Cookie")
    {
        p.unsupported(
            at,
            "http-cookie-header-undefined",
            "OpenAPI leaves Cookie header Parameter Object semantics undefined; use in: cookie",
        );
    }
    let examples = super::bodies::validate_examples(p, at);
    Some(Metadata {
        origin,
        name,
        location,
        required,
        deprecated,
        description,
        ignored,
        examples,
    })
}

pub(super) fn header(p: &mut Planner<'_>, source: &SourceId, name: &str) -> Option<HeaderPlan> {
    let meta = metadata(p, source, Some(name))?;
    // Set-Cookie cannot use comma-folded list handling; the descriptor only
    // admits a single opaque string field per declared Header Object today.
    let (codec, serialization, content_media) =
        serialization(p, &meta.origin.terminal.source, ParameterLocation::Header)?;
    if name.eq_ignore_ascii_case("Set-Cookie")
        && !matches!(
            &serialization,
            ParameterSerialization::Style {
                shape: WireShape::Scalar {
                    scalar: ScalarType::String
                },
                ..
            }
        )
    {
        p.unsupported(source, "http-set-cookie-repetition", "typed Set-Cookie lists need a repeated-field descriptor, not comma-folded header arrays");
    }
    Some(HeaderPlan {
        source: meta.origin,
        name: name.to_owned(),
        required: meta.required,
        deprecated: meta.deprecated,
        description: meta.description,
        codec,
        serialization,
        content_media,
        examples: meta.examples,
    })
}

fn serialization(
    p: &mut Planner<'_>,
    at: &SourceId,
    location: ParameterLocation,
) -> Option<(CodecRef, ParameterSerialization, Option<MediaPlan>)> {
    let raw = p.contract.source(at)?.as_object()?;
    if let Some(content) = raw.get("content") {
        p.require(Capability::ContentParameters, &at.child("content"));
        let map = content.as_object()?;
        if map.len() != 1 {
            return None;
        }
        if location == ParameterLocation::Querystring {
            let mut media = super::bodies::content(p, &at.child("content"), true, true);
            let media = media.pop()?;
            let (codec, percent_encoding) = match &media.representation {
                Representation::Json { codec: Some(codec) } => {
                    (codec.clone(), PercentEncoding::UriComponent)
                }
                Representation::Text {
                    codec: Some(codec),
                    scalar: ScalarType::String,
                    ..
                } => (codec.clone(), PercentEncoding::UriComponent),
                Representation::Form { form } => {
                    p.require(Capability::QuerystringForm, &media.source.use_site.source);
                    // This complete parameter is an actual JSON object input;
                    // supported form fields contain no byte placeholders.
                    (
                        CodecRef {
                            schema: form.rules.schema.clone(),
                            input: CodecInput::Json,
                        },
                        PercentEncoding::None,
                    )
                }
                Representation::Json { codec: None } | Representation::Text { codec: None, .. } => {
                    p.unsupported(&media.source.terminal.source, "http-querystring-schema-required", "typed querystring inputs require an indexed schema; no schema is synthesized");
                    return None;
                }
                _ => {
                    p.unsupported(&media.source.terminal.source, "http-querystring-content-media", "querystring content supports JSON, UTF-8 text strings, or form-urlencoded; file/multipart/stream inputs have no complete-query mapping");
                    return None;
                }
            };
            let serialization = ParameterSerialization::Content {
                media_type: media.media_type.clone(),
                percent_encoding,
            };
            return Some((codec, serialization, Some(media)));
        }
        let (name, _) = map.iter().next()?;
        let source = at.child("content").child(name);
        let media_type = super::media::planned(p, &source, name)?;
        let origin = super::bodies::media_origin(p, &source)?;
        let terminal = &origin.terminal.source;
        let media = p.object(terminal, "Parameter Media Type")?;
        p.known(
            terminal,
            media,
            &[
                "schema",
                "example",
                "examples",
                "encoding",
                "itemSchema",
                "prefixEncoding",
                "itemEncoding",
            ],
        );
        let examples = super::bodies::validate_examples(p, terminal);
        for field in ["encoding", "itemSchema", "prefixEncoding", "itemEncoding"] {
            if media.contains_key(field) {
                p.unsupported(&terminal.child(field), "http-parameter-content-encoding", "parameter content supports complete JSON or text/plain, not form/stream encoding");
            }
        }
        let input = if media_type.is_json() {
            CodecInput::Json
        } else if media_type.essence("text/plain") {
            if media_type
                .parameters
                .get("charset")
                .is_some_and(|value| !value.eq_ignore_ascii_case("utf-8"))
            {
                p.unsupported(
                    &source,
                    "http-text-charset-unsupported",
                    "text content parameters require UTF-8; no transcoding is inferred",
                );
                return None;
            }
            CodecInput::TextScalar
        } else {
            p.unsupported(
                &source,
                "http-parameter-content-media",
                "content parameters require concrete JSON/+json or text/plain media",
            );
            return None;
        };
        if media_type.is_json() && location == ParameterLocation::Cookie {
            p.unsupported(&source, "http-cookie-json-content", "JSON cookie content requires an explicit escaping policy; use text/plain with a caller-escaped cookie value");
            return None;
        }
        let codec = p.codec(&terminal.child("schema"), input)?;
        let representation = if input == CodecInput::TextScalar {
            let scalar = super::shapes::scalar(p, &codec.schema)?;
            Representation::Text {
                codec: Some(codec.clone()),
                scalar,
                encoding: TextEncoding::Utf8,
            }
        } else {
            Representation::Json {
                codec: Some(codec.clone()),
            }
        };
        let percent_encoding =
            if location == ParameterLocation::Header || location == ParameterLocation::Cookie {
                PercentEncoding::None
            } else {
                PercentEncoding::UriComponent
            };
        let content_media = MediaPlan {
            source: origin,
            media_type: media_type.clone(),
            representation,
            examples,
        };
        Some((
            codec,
            ParameterSerialization::Content {
                media_type,
                percent_encoding,
            },
            Some(content_media),
        ))
    } else {
        let codec = p.codec(&at.child("schema"), CodecInput::Json)?;
        let shape = super::shapes::shape(p, &codec.schema)?;
        let serialization = style(p, at, location, shape, None)?;
        Some((codec, serialization, None))
    }
}

pub(super) fn style(
    p: &mut Planner<'_>,
    at: &SourceId,
    location: ParameterLocation,
    shape: WireShape,
    encoding_override: Option<PercentEncoding>,
) -> Option<ParameterSerialization> {
    let raw = p.contract.source(at)?.as_object()?;
    let style = if let Some(value) = p.string(at, "style", false) {
        parse_style(p, &value.source.source, &value.value, location)?
    } else if matches!(
        location,
        ParameterLocation::Query | ParameterLocation::Cookie
    ) {
        Style::Form
    } else {
        Style::Simple
    };
    let explode = p.boolean(at, "explode", matches!(style, Style::Form | Style::Cookie));
    let reserved = p.boolean(at, "allowReserved", false);
    let unsupported = match style {
        Style::DeepObject => {
            !matches!(shape, WireShape::FlatObject { .. }) || !p.is_32(at) && !explode
        }
        Style::SpaceDelimited | Style::PipeDelimited => {
            matches!(shape, WireShape::Scalar { .. }) || explode
        }
        Style::Form if location == ParameterLocation::Cookie => {
            explode && !matches!(shape, WireShape::Scalar { .. })
        }
        _ => false,
    };
    if unsupported {
        let source = if raw.contains_key("explode") {
            at.child("explode")
        } else if raw.contains_key("style") {
            at.child("style")
        } else {
            at.clone()
        };
        p.unsupported(&source, "http-parameter-combination-undefined", "style/explode/type combination is undefined by this OAS version (including exploded form cookies); no nested or delimiter convention is inferred");
        return None;
    }
    let percent_encoding = if let Some(encoding) = encoding_override {
        encoding
    } else if location == ParameterLocation::Header || style == Style::Cookie {
        PercentEncoding::None
    } else if reserved && (location == ParameterLocation::Query || p.is_32(at)) {
        p.require(Capability::ReservedParameters, &at.child("allowReserved"));
        PercentEncoding::ReservedExpansion
    } else {
        if reserved {
            p.warn(
                &at.child("allowReserved"),
                "http-allow-reserved-ignored",
                "allowReserved applies only to query parameters before OAS 3.2",
            );
        }
        PercentEncoding::UriComponent
    };
    Some(ParameterSerialization::Style {
        style,
        explode,
        shape,
        percent_encoding,
    })
}

fn parse_style(
    p: &mut Planner<'_>,
    source: &SourceId,
    name: &str,
    location: ParameterLocation,
) -> Option<Style> {
    let style = match name {
        "simple" => Style::Simple,
        "label" => Style::Label,
        "matrix" => Style::Matrix,
        "form" => Style::Form,
        "spaceDelimited" => Style::SpaceDelimited,
        "pipeDelimited" => Style::PipeDelimited,
        "deepObject" => Style::DeepObject,
        "cookie" if p.is_32(source) => Style::Cookie,
        _ => {
            p.error(
                source,
                "http-parameter-style",
                "unknown parameter style for this OpenAPI version",
            );
            return None;
        }
    };
    let valid = match location {
        ParameterLocation::Path => matches!(style, Style::Simple | Style::Label | Style::Matrix),
        ParameterLocation::Query => matches!(
            style,
            Style::Form | Style::SpaceDelimited | Style::PipeDelimited | Style::DeepObject
        ),
        ParameterLocation::Header => style == Style::Simple,
        ParameterLocation::Cookie => matches!(style, Style::Form | Style::Cookie),
        ParameterLocation::Querystring => false,
    };
    if !valid {
        p.error(
            source,
            "http-parameter-style-location",
            "style is not defined for this parameter location",
        );
        return None;
    }
    Some(style)
}
