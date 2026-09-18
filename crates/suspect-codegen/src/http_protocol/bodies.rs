use std::collections::BTreeSet;

use serde_json::{Map, Value};
use suspect_ir::contract::SourceId;

use super::planner::Planner;
use super::*;

pub(super) fn plan(p: &mut Planner<'_>, operation: &SourceId) -> Option<BodyPlan> {
    let source = operation.child("requestBody");
    p.contract.source(&source)?;
    let origin = p.resolve(&source, false)?;
    let at = &origin.terminal.source;
    let raw = p.object(at, "Request Body")?;
    p.known(at, raw, &["description", "required", "content"]);
    let description = p.text(&origin, "description", false);
    let required = p.boolean(at, "required", false);
    let media = content(p, &at.child("content"), true, true);
    Some(BodyPlan {
        source: origin,
        required,
        description,
        media,
        limits: p.capabilities.limits,
    })
}

pub(super) fn content(
    p: &mut Planner<'_>,
    source: &SourceId,
    required: bool,
    request: bool,
) -> Vec<MediaPlan> {
    let Some(value) = p.contract.source(source) else {
        if required {
            p.error(
                source,
                "http-content-required",
                "request body content map is required",
            );
        }
        return Vec::new();
    };
    let Some(map) = value.as_object() else {
        p.error(
            source,
            "http-content-invalid",
            "content must be a media-type to Media Type Object map",
        );
        return Vec::new();
    };
    if required && map.is_empty() {
        p.error(
            source,
            "http-content-empty",
            "a declared request body needs at least one media representation",
        );
    }
    if map.len() > 1 {
        p.require(Capability::MultipleMediaTypes, source);
    }
    let mut result = Vec::new();
    for (name, _) in map {
        let at = source.child(name);
        let Some(media_type) = super::media::planned(p, &at, name) else {
            continue;
        };
        let Some(origin) = media_origin(p, &at) else {
            continue;
        };
        let terminal = &origin.terminal.source;
        let Some(raw) = p.object(terminal, "Media Type") else {
            continue;
        };
        p.known(
            terminal,
            raw,
            &[
                "schema",
                "itemSchema",
                "example",
                "examples",
                "encoding",
                "prefixEncoding",
                "itemEncoding",
            ],
        );
        let examples = validate_examples(p, terminal);
        for field in ["itemSchema", "prefixEncoding", "itemEncoding"] {
            if raw.contains_key(field) && !p.is_32(terminal) {
                p.unsupported(
                    &terminal.child(field),
                    "http-version-field",
                    format!("Media Type.{field} requires OAS 3.2"),
                );
            }
        }
        // Validate shape even when the field is ignored for this media type.
        if let Some(encoding) = raw.get("encoding") {
            if let Some(map) = encoding.as_object() {
                for key in map.keys() {
                    validate_encoding(p, &terminal.child("encoding").child(key));
                }
            } else {
                p.error(
                    &terminal.child("encoding"),
                    "http-encoding-map-invalid",
                    "encoding must be a property-name to Encoding Object map",
                );
            }
        }
        if let Some(prefix) = raw.get("prefixEncoding") {
            if let Some(array) = prefix.as_array() {
                for (index, _) in array.iter().enumerate() {
                    validate_encoding(
                        p,
                        &terminal.child("prefixEncoding").child(&index.to_string()),
                    );
                }
            } else {
                p.error(
                    &terminal.child("prefixEncoding"),
                    "http-encoding-map-invalid",
                    "prefixEncoding must be an array of Encoding Objects",
                );
            }
        }
        if raw.contains_key("itemEncoding") {
            validate_encoding(p, &terminal.child("itemEncoding"));
        }
        if raw.contains_key("encoding")
            && (raw.contains_key("prefixEncoding") || raw.contains_key("itemEncoding"))
        {
            p.error(
                &terminal.child("encoding"),
                "http-encoding-exclusive",
                "encoding cannot be combined with prefixEncoding or itemEncoding",
            );
        }
        if let Some(representation) = representation(p, terminal, raw, &media_type, request) {
            result.push(MediaPlan {
                source: origin,
                media_type,
                representation,
                examples,
            });
        }
    }
    super::media::validate_map(p, &result);
    result
}

pub(super) fn media_origin(p: &mut Planner<'_>, source: &SourceId) -> Option<Provenance> {
    if p.contract
        .source(source)
        .is_some_and(|v| v.get("$ref").is_some())
        && !p.is_32(source)
    {
        p.unsupported(
            &source.child("$ref"),
            "http-version-field",
            "Media Type Object references require OAS 3.2",
        );
        return None;
    }
    p.resolve(source, false)
}

fn representation(
    p: &mut Planner<'_>,
    at: &SourceId,
    raw: &Map<String, Value>,
    media: &MediaType,
    request: bool,
) -> Option<Representation> {
    let multipart =
        matches!(&media.range, MediaRange::Concrete { type_name, .. } if type_name == "multipart");
    let form = media.essence("application/x-www-form-urlencoded");
    if !multipart && !form {
        for key in ["encoding", "prefixEncoding", "itemEncoding"] {
            if raw.contains_key(key) {
                p.warn(
                    &at.child(key),
                    "http-encoding-ignored",
                    "encoding metadata applies only to the specified form/multipart media contexts",
                );
            }
        }
    }
    if form || multipart {
        if !request && !p.is_32(at) {
            p.unsupported(
                at,
                "http-form-response-version",
                "form/multipart response encoding requires OAS 3.2 metadata semantics",
            );
            return None;
        }
        if form {
            p.require(Capability::FormBodies, at);
            utf8(p, at, media);
            if raw.contains_key("prefixEncoding") || raw.contains_key("itemEncoding") {
                p.error(
                    at,
                    "http-form-positional-encoding",
                    "positional encoding applies only to multipart",
                );
                return None;
            }
            return named(p, at, raw, false).map(|(rules, fields, additional)| {
                Representation::Form {
                    form: FormPlan {
                        rules,
                        fields,
                        additional,
                    },
                }
            });
        }
        p.require(Capability::MultipartBodies, at);
        if raw.contains_key("prefixEncoding") || raw.contains_key("itemEncoding") {
            p.require(Capability::PositionalMultipart, at);
            return positional(p, at, raw, media)
                .map(|multipart| Representation::Multipart { multipart });
        }
        if !media.essence("multipart/form-data") {
            p.unsupported(at, "http-multipart-encoding-required", "non-form-data multipart requires OAS 3.2 positional encoding; named form-data disposition is not inferred");
            return None;
        }
        return named(p, at, raw, true).map(|(rules, parts, additional)| {
            Representation::Multipart {
                multipart: MultipartPlan::Named {
                    rules,
                    parts,
                    additional,
                },
            }
        });
    }
    let framing = if media.essence("text/event-stream") {
        Some(StreamFraming::ServerSentEvents)
    } else if media.essence("application/jsonl") || media.essence("application/x-ndjson") {
        Some(StreamFraming::JsonLines)
    } else {
        None
    };
    if let Some(framing) = framing {
        utf8(p, at, media);
        p.require(
            if framing == StreamFraming::ServerSentEvents {
                Capability::ServerSentEvents
            } else {
                Capability::JsonLines
            },
            at,
        );
        if p.is_32(at) && raw.contains_key("itemSchema") {
            if raw.contains_key("schema") {
                p.unsupported(&at.child("schema"), "http-stream-aggregate-schema", "this item-stream plan cannot also enforce whole-content schema assertions; aggregate validation needs a separately verified bounded buffering capability");
                return None;
            }
            let item_codec = p.codec(&at.child("itemSchema"), CodecInput::Json)?;
            if framing == StreamFraming::ServerSentEvents {
                validate_sse_item(p, &item_codec.schema);
            }
            return Some(Representation::Stream {
                stream: StreamPlan {
                    source: p.location(at),
                    framing,
                    item_codec: Some(item_codec),
                    max_item_bytes: p.capabilities.limits.stream_item,
                },
            });
        }
        // Schemaless SSE is an explicit compatibility departure: a 3.0/3.1
        // `text/event-stream` response without a standard OAS 3.2 itemSchema
        // streams untyped frames — its complete-content schema (if any) is not
        // an item/data schema and stays uninterpreted. JSON-lines framing,
        // request streams and declared 3.2 item schemas keep their ordinary
        // strict admission.
        if !request
            && framing == StreamFraming::ServerSentEvents
            && p.capabilities
                .profiles
                .contains(&CompatibilityProfile::SchemalessStreamEventsV1)
        {
            p.warn(at, "http-compatibility-profile", "schemaless-stream-events-v1 admits this schemaless text/event-stream response as untyped frames; frame data stays an untyped text payload and no item codec is compiled");
            return Some(Representation::Stream {
                stream: StreamPlan {
                    source: p.location(at),
                    framing,
                    item_codec: None,
                    max_item_bytes: p.capabilities.limits.stream_item,
                },
            });
        }
        p.unsupported(&if raw.contains_key("schema") { at.child("schema") } else { at.clone() }, "http-stream-item-schema-required", "stream item semantics require standard OAS 3.2 itemSchema; a 3.1 complete-content schema is not an item/data schema or a sentinel declaration");
        return None;
    }
    if raw.contains_key("itemSchema") {
        p.unsupported(
            &at.child("itemSchema"),
            "http-stream-media-unsupported",
            "itemSchema requires a supported sequential media framing (SSE or JSON lines)",
        );
        return None;
    }
    if media.essence("application/json-seq") || media.essence("application/geo+json-seq") {
        p.unsupported(
            at,
            "http-stream-framing-unsupported",
            "JSON text sequences require RFC7464 record-separator framing, not JSON-lines framing",
        );
        return None;
    }
    if media.is_json() {
        let codec = if raw.contains_key("schema") {
            Some(p.codec(&at.child("schema"), CodecInput::Json)?)
        } else {
            p.require(Capability::SchemaFreeJson, at);
            None
        };
        return Some(Representation::Json { codec });
    }
    if media.is_text() && matches!(media.range, MediaRange::Concrete { .. }) {
        p.require(Capability::TextBodies, at);
        utf8(p, at, media);
        let codec = if raw.contains_key("schema") {
            Some(p.codec(&at.child("schema"), CodecInput::TextScalar)?)
        } else {
            None
        };
        let scalar = if let Some(codec) = &codec {
            super::shapes::scalar(p, &codec.schema)?
        } else {
            ScalarType::String
        };
        return Some(Representation::Text {
            codec,
            scalar,
            encoding: TextEncoding::Utf8,
        });
    }
    p.require(Capability::BinaryBodies, at);
    let schema = if raw.contains_key("schema") {
        Some(p.schema(&at.child("schema"))?)
    } else {
        None
    };
    let bytes = byte_policy(p, schema.as_ref(), p.capabilities.limits.body)?;
    Some(Representation::Binary { schema, bytes })
}

fn utf8(p: &mut Planner<'_>, at: &SourceId, media: &MediaType) {
    if media
        .parameters
        .get("charset")
        .is_some_and(|v| !v.eq_ignore_ascii_case("utf-8"))
    {
        p.unsupported(
            at,
            "http-text-charset-unsupported",
            "typed text/framing requires UTF-8; no transcoding is inferred",
        );
    }
}

pub(super) fn byte_policy(
    p: &mut Planner<'_>,
    schema: Option<&SchemaUse>,
    limit: u64,
) -> Option<BytePolicy> {
    let Some(schema) = schema else {
        return Some(BytePolicy {
            max_bytes: limit,
            declared_max_bytes: None,
        });
    };
    if p.contract
        .schema(&schema.id)
        .is_some_and(|v| v.raw() == &Value::Bool(true))
    {
        return Some(BytePolicy {
            max_bytes: limit,
            declared_max_bytes: None,
        });
    }
    let (at, raw) = super::shapes::object(p, schema)?;
    let legacy = super::shapes::type_name(raw) == Some("string")
        && raw.get("format").and_then(Value::as_str) == Some("binary");
    if legacy && !p.is_30(&at) {
        if p.capabilities
            .profiles
            .contains(&CompatibilityProfile::LegacyBinaryStringV1)
        {
            p.warn(&at.child("format"), "http-compatibility-profile", "legacy-binary-string-v1 interprets the string/binary marker as a bounded byte input");
        } else {
            p.unsupported(&at.child("format"), "http-binary-legacy-marker", "OAS 3.1+ format: binary does not change JSON Schema's string type; use an unconstrained binary schema or explicitly opt in to legacy-binary-string-v1");
            return None;
        }
    }
    if raw.contains_key("type") && !legacy {
        p.unsupported(&at.child("type"), "http-binary-schema-type", "unencoded bytes are not JSON values; a JSON type constraint cannot be validated using a placeholder null");
        return None;
    }
    for key in raw.keys() {
        if matches!(
            key.as_str(),
            "title"
                | "description"
                | "summary"
                | "deprecated"
                | "readOnly"
                | "writeOnly"
                | "example"
                | "examples"
                | "$comment"
                | "$schema"
                | "contentMediaType"
        ) || key.starts_with("x-")
            || legacy && matches!(key.as_str(), "type" | "format")
            || p.is_32(&at) && key == "maxLength"
            || p.resource_annotation(&at, key)
        {
            continue;
        }
        p.unsupported(&at.child(key), "http-binary-schema-keyword", "this binary assertion/encoding cannot be enforced by the finite byte policy; JSON Schema evaluation on a substitute JSON value is forbidden");
        return None;
    }
    let declared_max_bytes = if p.is_32(&at) {
        p.integer(&at, "maxLength")
    } else {
        None
    };
    Some(BytePolicy {
        max_bytes: declared_max_bytes
            .as_ref()
            .map_or(limit, |v| limit.min(v.value)),
        declared_max_bytes,
    })
}

fn validate_sse_item(p: &mut Planner<'_>, schema: &SchemaUse) {
    // The item is a genuine JSON codec input. Unlike mixed byte aggregates,
    // assertion siblings, unions and boolean schemas must keep their original
    // root; an envelope check is not permission to substitute a ref target.
    let mut pending = vec![schema.id.clone()];
    let mut seen = BTreeSet::new();
    while let Some(at) = pending.pop() {
        if !seen.insert(at.clone()) {
            continue;
        }
        let Some(view) = p.contract.schema(&at) else {
            continue;
        };
        if let Some(raw) = view.raw().as_object() {
            if raw
                .get("type")
                .is_some_and(|value| type_excludes(value, &["object"]))
            {
                p.error(&at.child("type"), "http-sse-envelope", "SSE itemSchema describes a parsed event object, not a data field or JSON payload");
            }
            if let Some(properties) = raw.get("properties").and_then(Value::as_object) {
                for name in ["data", "id", "event", "retry"] {
                    if properties.contains_key(name) {
                        let mut fields = vec![at.child("properties").child(name)];
                        let mut checked = BTreeSet::new();
                        while let Some(field) = fields.pop() {
                            if !checked.insert(field.clone()) {
                                continue;
                            }
                            let Some(schema) = p.contract.schema(&field) else {
                                continue;
                            };
                            let types: &[&str] = if name == "retry" {
                                &["integer", "number"]
                            } else {
                                &["string"]
                            };
                            if schema
                                .raw()
                                .get("type")
                                .is_some_and(|value| type_excludes(value, types))
                            {
                                p.error(&field.child("type"), "http-sse-field-type", "SSE data/event/id are strings and retry is an integer; nested JSON data decoding is not implicit");
                            }
                            fields.extend(
                                schema
                                    .references()
                                    .iter()
                                    .filter(|r| r.keyword == "$ref")
                                    .filter_map(|r| r.target.clone()),
                            );
                        }
                    }
                }
            }
        }
        pending.extend(
            view.references()
                .iter()
                .filter(|r| r.keyword == "$ref")
                .filter_map(|r| r.target.clone()),
        );
    }
}

fn type_excludes(value: &Value, allowed: &[&str]) -> bool {
    match value {
        Value::String(value) => !allowed.contains(&value.as_str()),
        Value::Array(values) => !values
            .iter()
            .filter_map(Value::as_str)
            .any(|value| allowed.contains(&value)),
        _ => false, // Invalid keyword types are the schema compiler's concern.
    }
}

pub(super) fn validate_examples(p: &mut Planner<'_>, at: &SourceId) -> ExampleMetadata {
    super::examples::plan(p, at)
}

fn validate_encoding(p: &mut Planner<'_>, source: &SourceId) {
    let Some(raw) = p.object(source, "Encoding") else {
        return;
    };
    p.known(
        source,
        raw,
        &[
            "contentType",
            "headers",
            "style",
            "explode",
            "allowReserved",
            "encoding",
            "prefixEncoding",
            "itemEncoding",
        ],
    );
    p.string(source, "contentType", false);
    p.string(source, "style", false);
    p.boolean(source, "explode", false);
    p.boolean(source, "allowReserved", false);
    if raw.get("headers").is_some_and(|v| !v.is_object()) {
        p.error(
            &source.child("headers"),
            "http-headers-invalid",
            "encoding headers must be a map of Header Objects/references",
        );
    }
    for key in ["encoding", "prefixEncoding", "itemEncoding"] {
        if raw.contains_key(key) {
            p.unsupported(
                &source.child(key),
                "http-nested-part-encoding",
                "nested multipart/encoding is not represented by this finite flat-part plan",
            );
        }
    }
}

fn named(
    p: &mut Planner<'_>,
    at: &SourceId,
    media: &Map<String, Value>,
    multipart: bool,
) -> Option<(ObjectRules, Vec<PartPlan>, AdditionalParts)> {
    let context = if multipart {
        PartContext::MultipartFormData
    } else {
        PartContext::FormUrlEncoded
    };
    if media.contains_key("itemSchema") {
        p.unsupported(&at.child("itemSchema"), "http-multipart-streaming", "streamed multipart item encoding requires a dedicated native capability; this plan represents finite named/positional parts");
        return None;
    }
    let schema = p.schema(&at.child("schema"))?;
    let (terminal, raw) = super::shapes::object(p, &schema)?;
    if raw.contains_key("type") && super::shapes::type_name(raw) != Some("object") {
        p.error(
            &terminal.child("type"),
            "http-form-object-required",
            "named form/multipart content requires an object schema",
        );
        return None;
    }
    structural_keywords(
        p,
        &terminal,
        raw,
        &[
            "type",
            "properties",
            "required",
            "additionalProperties",
            "minProperties",
            "maxProperties",
        ],
    )?;
    let required = p.strings(&terminal, "required", false).unwrap_or_default();
    let mut unique = BTreeSet::new();
    for field in &required {
        if !unique.insert(field.value.clone()) {
            p.error(
                &field.source.source,
                "http-required-duplicate",
                "required property names must be unique",
            );
        }
    }
    let min_properties = p.integer(&terminal, "minProperties");
    let max_properties = p.integer(&terminal, "maxProperties");
    let empty = Map::new();
    let properties = match raw.get("properties") {
        None => &empty,
        Some(Value::Object(map)) => map,
        Some(_) => {
            p.error(
                &terminal.child("properties"),
                "http-schema-properties",
                "properties must be a map",
            );
            return None;
        }
    };
    let encodings = media.get("encoding").and_then(Value::as_object);
    if let Some(encodings) = encodings {
        if !encodings.is_empty() {
            p.require(Capability::PartEncodings, &at.child("encoding"));
        }
        for key in encodings.keys() {
            if !properties.contains_key(key) {
                if p.is_32(at) {
                    p.warn(
                        &at.child("encoding").child(key),
                        "http-encoding-property-ignored",
                        "OAS 3.2 ignores an encoding entry with no corresponding schema property",
                    );
                } else {
                    p.error(
                        &at.child("encoding").child(key),
                        "http-encoding-property-unknown",
                        "encoding map keys must name declared schema properties",
                    );
                }
            }
        }
    }
    let mut fields = Vec::new();
    for key in properties.keys() {
        let source = terminal.child("properties").child(key);
        let encoding = encodings
            .filter(|map| map.contains_key(key))
            .map(|_| at.child("encoding").child(key));
        if let Some(part) = part(
            p,
            &source,
            Some(key.clone()),
            required.iter().any(|r| &r.value == key),
            encoding.as_ref(),
            context,
            true,
        ) {
            fields.push(part);
        }
    }
    // Unknown extras cannot be closed/stripped or passed through a JSON-null
    // aggregate. A schema gives them an actual codec/byte representation.
    let additional = match raw.get("additionalProperties") {
        Some(Value::Bool(false)) => AdditionalParts::Forbidden,
        Some(Value::Object(_)) => AdditionalParts::Allowed(Box::new(part(
            p,
            &terminal.child("additionalProperties"),
            None,
            false,
            None,
            context,
            true,
        )?)),
        None | Some(Value::Bool(true)) => {
            p.unsupported(&if raw.contains_key("additionalProperties") { terminal.child("additionalProperties") } else { terminal.clone() }, "http-form-untyped-extras", "form/multipart extras need additionalProperties: false or an explicit part schema; no encoding for arbitrary extra values is inferred");
            return None;
        }
        _ => {
            p.error(
                &terminal.child("additionalProperties"),
                "http-schema-additional-properties",
                "additionalProperties must be a schema or boolean",
            );
            return None;
        }
    };
    if matches!(additional, AdditionalParts::Forbidden) {
        for field in &required {
            if !properties.contains_key(&field.value) {
                p.error(
                    &field.source.source,
                    "http-form-required-undeclared",
                    "required field is forbidden by the closed form schema",
                );
            }
        }
    }
    Some((
        ObjectRules {
            schema,
            required,
            min_properties,
            max_properties,
        },
        fields,
        additional,
    ))
}

/// Encoding applicability comes from the enclosing media type, not the
/// property schema, part contentType, or the presence of serialization fields.
#[derive(Clone, Copy)]
enum PartContext {
    FormUrlEncoded,
    MultipartFormData,
    MultipartOther,
}

impl PartContext {
    fn multipart(self) -> bool {
        !matches!(self, Self::FormUrlEncoded)
    }
    fn style_applies(self) -> bool {
        !matches!(self, Self::MultipartOther)
    }
}

fn part(
    p: &mut Planner<'_>,
    source: &SourceId,
    name: Option<String>,
    required: bool,
    encoding: Option<&SourceId>,
    context: PartContext,
    split_arrays: bool,
) -> Option<PartPlan> {
    let schema = p.schema(source)?;
    let (terminal, raw) = super::shapes::object(p, &schema)?;
    let encoding_raw = encoding
        .and_then(|e| p.contract.source(e))
        .and_then(Value::as_object);
    let multipart = context.multipart();
    let style_declared = encoding_raw.is_some_and(|v| {
        ["style", "explode", "allowReserved"]
            .iter()
            .any(|k| v.contains_key(*k))
    });
    let styled = context.style_applies() && style_declared;
    if !context.style_applies()
        && let Some(encoding) = encoding
    {
        for field in ["style", "explode", "allowReserved"] {
            if encoding_raw.is_some_and(|raw| raw.contains_key(field)) {
                p.warn(&encoding.child(field), "http-encoding-style-ignored", format!("{field} applies only to application/x-www-form-urlencoded and multipart/form-data; this part retains its contentType representation"));
            }
        }
    }
    let repeated = split_arrays
        && super::shapes::type_name(raw) == Some("array")
        && (!styled || p.is_32(encoding.unwrap_or(source)));
    let (value_schema, min_items, max_items) = if repeated {
        structural_keywords(
            p,
            &terminal,
            raw,
            &["type", "items", "minItems", "maxItems"],
        )?;
        (
            p.schema(&terminal.child("items"))?,
            p.integer(&terminal, "minItems"),
            p.integer(&terminal, "maxItems"),
        )
    } else {
        (schema.clone(), None, None)
    };
    let (value_at, value_raw) = super::shapes::object(p, &value_schema)?;
    if repeated && super::shapes::type_name(value_raw) == Some("array") && !p.is_32(&value_at) {
        p.unsupported(
            &value_at,
            "http-part-nested-array-version",
            "nested array part values need the explicit OAS 3.2 per-item JSON encoding rules",
        );
        return None;
    }
    let outer_encoding = if multipart {
        PercentEncoding::None
    } else {
        PercentEncoding::FormUrlEncoded
    };
    let mut content_types = Vec::new();
    if let Some(encoding) = encoding
        && let Some(value) = p.string(encoding, "contentType", false)
    {
        match super::media::split_quoted(&value.value, ',') {
            Ok(names) => {
                for name in names {
                    if let Some(media) = super::media::planned(p, &value.source.source, name.trim())
                    {
                        content_types.push(media);
                    }
                }
            }
            Err(reason) => p.error(&value.source.source, "http-part-content-type", reason),
        }
    }
    let headers = if let Some(encoding) = encoding {
        if multipart {
            super::responses::headers(p, &encoding.child("headers"), true)
        } else {
            if encoding_raw.is_some_and(|v| v.contains_key("headers")) {
                p.warn(
                    &encoding.child("headers"),
                    "http-encoding-headers-ignored",
                    "encoding headers apply only to multipart",
                );
            }
            Vec::new()
        }
    } else {
        Vec::new()
    };
    let representation = if styled {
        let encoding = encoding.expect("style is from an Encoding Object");
        if encoding_raw.is_some_and(|v| v.contains_key("contentType")) {
            p.warn(&encoding.child("contentType"), "http-encoding-content-type-ignored", "explicit style/explode/allowReserved selects RFC6570-style serialization; contentType is ignored");
        }
        content_types.clear();
        let shape = super::shapes::shape(p, &value_schema)?;
        let serialization = super::parameters::style(
            p,
            encoding,
            ParameterLocation::Query,
            shape,
            if multipart {
                Some(PercentEncoding::None)
            } else {
                None
            },
        )?;
        PartRepresentation::Style {
            codec: CodecRef {
                schema: value_schema,
                input: CodecInput::Json,
            },
            serialization,
        }
    } else {
        if value_raw.contains_key("contentEncoding") {
            p.unsupported(&value_at.child("contentEncoding"), "http-part-content-encoding", "encoded binary parts require a verified content/transfer-encoding codec; raw byte conversion is not inferred from an annotation");
            return None;
        }
        if content_types.is_empty() {
            let default = match super::shapes::type_name(value_raw) {
                Some("string")
                    if value_raw.get("format").and_then(Value::as_str) == Some("binary")
                        && (p.is_30(&value_at)
                            || p.capabilities
                                .profiles
                                .contains(&CompatibilityProfile::LegacyBinaryStringV1)) =>
                {
                    "application/octet-stream"
                }
                Some("string" | "boolean" | "integer" | "number") => "text/plain",
                Some("object" | "array") => "application/json",
                None if !value_raw.contains_key("type") => "application/octet-stream",
                _ => {
                    p.unsupported(
                        &value_at,
                        "http-part-shape-unknown",
                        "part content type cannot be defaulted for a nullable/ambiguous type",
                    );
                    return None;
                }
            };
            content_types
                .push(super::media::parse(default, true).expect("fixed standard media type"));
        }
        if content_types.iter().all(MediaType::is_json) {
            PartRepresentation::Json {
                codec: CodecRef {
                    schema: value_schema,
                    input: CodecInput::Json,
                },
                outer_encoding,
            }
        } else if content_types
            .iter()
            .all(|m| m.is_text() && matches!(m.range, MediaRange::Concrete { .. }))
        {
            for media in &content_types {
                utf8(p, encoding.unwrap_or(source), media);
            }
            let scalar = super::shapes::scalar(p, &value_schema)?;
            PartRepresentation::Text {
                codec: CodecRef {
                    schema: value_schema,
                    input: CodecInput::TextScalar,
                },
                scalar,
                outer_encoding,
            }
        } else if content_types.iter().all(|m| {
            !m.is_json() && (!m.is_text() || !matches!(m.range, MediaRange::Concrete { .. }))
        }) {
            if !multipart {
                p.unsupported(&value_at, "http-form-binary-encoding", "form-urlencoded is text; raw file bytes need an explicit supported textual content encoding");
                return None;
            }
            PartRepresentation::Binary {
                bytes: byte_policy(p, Some(&value_schema), p.capabilities.limits.part)?,
            }
        } else {
            p.unsupported(encoding.unwrap_or(source), "http-part-mixed-representations", "part contentType choices cross JSON/text/binary representations; a tagged part-value choice is not yet represented");
            return None;
        }
    };
    Some(PartPlan {
        source: schema.source.clone(),
        name,
        schema,
        required,
        multiplicity: if repeated {
            PartMultiplicity::RepeatedArrayItems
        } else {
            PartMultiplicity::One
        },
        min_items,
        max_items,
        encoding_source: encoding.map(|s| p.inline(s)),
        content_types,
        representation,
        headers,
    })
}

fn structural_keywords(
    p: &mut Planner<'_>,
    at: &SourceId,
    raw: &Map<String, Value>,
    allowed: &[&str],
) -> Option<()> {
    for key in raw.keys() {
        if allowed.contains(&key.as_str())
            || matches!(
                key.as_str(),
                "title"
                    | "description"
                    | "summary"
                    | "$schema"
                    | "$comment"
                    | "deprecated"
                    | "example"
                    | "examples"
            )
            || key.starts_with("x-")
            || p.resource_annotation(at, key)
        {
            continue;
        }
        p.unsupported(&at.child(key), "http-form-structural-assertion", "whole form/part-array assertion is not enforceable by the structural rules; the aggregate cannot be sent to a JSON codec with binary placeholders");
        return None;
    }
    Some(())
}

fn positional(
    p: &mut Planner<'_>,
    at: &SourceId,
    media: &Map<String, Value>,
    media_type: &MediaType,
) -> Option<MultipartPlan> {
    let context = if media_type.essence("multipart/form-data") {
        PartContext::MultipartFormData
    } else {
        PartContext::MultipartOther
    };
    if !p.is_32(at) {
        p.unsupported(
            at,
            "http-version-field",
            "positional multipart requires OAS 3.2",
        );
        return None;
    }
    if media.contains_key("itemSchema") {
        p.unsupported(&at.child("itemSchema"), "http-multipart-streaming", "finite positional multipart uses an array schema; streamed multipart itemSchema requires a dedicated capability");
        return None;
    }
    let schema = p.schema(&at.child("schema"))?;
    let (terminal, raw) = super::shapes::object(p, &schema)?;
    if super::shapes::type_name(raw) != Some("array") {
        p.error(
            &terminal,
            "http-multipart-array-required",
            "positional multipart requires an array schema",
        );
        return None;
    }
    structural_keywords(
        p,
        &terminal,
        raw,
        &["type", "prefixItems", "items", "minItems", "maxItems"],
    )?;
    let min_items = p.integer(&terminal, "minItems");
    let max_items = p.integer(&terminal, "maxItems");
    let prefix_encoding = media.get("prefixEncoding").and_then(Value::as_array);
    let mut prefix = Vec::new();
    let schema_prefix = match raw.get("prefixItems") {
        Some(Value::Array(values)) => values.len(),
        None => 0,
        Some(_) => {
            p.error(
                &terminal.child("prefixItems"),
                "http-multipart-prefix-items",
                "prefixItems must be an array of schemas",
            );
            return None;
        }
    };
    let length = schema_prefix.max(prefix_encoding.map_or(0, Vec::len));
    let mut prefix_barrier = false;
    for index in 0..length {
        let item_source = if index < schema_prefix {
            terminal.child("prefixItems").child(&index.to_string())
        } else {
            terminal.child("items")
        };
        if p.contract.source(&item_source) == Some(&Value::Bool(false)) {
            // Later positions are unreachable without supplying this false slot.
            prefix_barrier = true;
            break;
        }
        let encoding = if prefix_encoding.is_some_and(|v| index < v.len()) {
            Some(at.child("prefixEncoding").child(&index.to_string()))
        } else if media.contains_key("itemEncoding") {
            Some(at.child("itemEncoding"))
        } else {
            None
        };
        if let Some(part) = part(
            p,
            &item_source,
            None,
            min_items.as_ref().is_some_and(|v| (index as u64) < v.value),
            encoding.as_ref(),
            context,
            false,
        ) {
            prefix.push(part);
        }
    }
    let items = if prefix_barrier {
        AdditionalParts::Forbidden
    } else {
        match raw.get("items") {
            Some(Value::Bool(false)) => AdditionalParts::Forbidden,
            Some(Value::Object(_)) => {
                let encoding = media
                    .contains_key("itemEncoding")
                    .then(|| at.child("itemEncoding"));
                AdditionalParts::Allowed(Box::new(part(
                    p,
                    &terminal.child("items"),
                    None,
                    false,
                    encoding.as_ref(),
                    context,
                    false,
                )?))
            }
            _ => {
                p.unsupported(
                    &terminal,
                    "http-multipart-untyped-items",
                    "remaining positional parts require items: false or an indexed part schema",
                );
                return None;
            }
        }
    };
    if media_type.essence("multipart/form-data") {
        for part in prefix.iter().chain(match &items {
            AdditionalParts::Allowed(part) => Some(part.as_ref()),
            _ => None,
        }) {
            if !part
                .headers
                .iter()
                .any(|h| h.name.eq_ignore_ascii_case("Content-Disposition") && h.required)
            {
                p.error(&part.source.terminal.source, "http-multipart-disposition-required", "positional form-data parts require a declared required Content-Disposition header including their part name");
            }
        }
    }
    Some(MultipartPlan::Positional {
        schema,
        prefix,
        items,
        min_items,
        max_items,
    })
}
