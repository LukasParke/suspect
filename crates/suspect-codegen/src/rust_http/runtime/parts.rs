use super::parameters::{Writer, encode, percent_decode};
use super::{
    Headers, Limits, Media, Operation, Parameter, ParameterLocation, ParsedMedia, PercentEncoding,
    ScalarType, SdkError, Serialization, Shape, Source, representation_error, request_codec_error,
    resource_error, validation_error,
};
use crate::{JsonNonNullValue as Value, JsonValue, Nullable};
use std::collections::{BTreeMap, BTreeSet};

/// Native finite part. Byte parts use `Vec<u8>` directly; filenames are metadata,
/// never paths opened by the runtime. Required declared headers use a generated
/// header struct supplied with `Part::with_headers`.
#[derive(Debug, Clone)]
pub struct Part<T, H = ()> {
    pub data: T,
    pub filename: Option<String>,
    pub content_type: Option<String>,
    pub headers: H,
}

/// A whole-query form is an actual JSON object codec input. The shared plan
/// supplies field strategies, with no second URI-encoding pass or name prefix.
pub(super) fn query_form(
    operation: Source,
    spec: &AggregateSpec,
    value: &JsonValue,
    limit: usize,
    part_limit: usize,
    json: super::JsonLimits,
) -> Result<String, SdkError> {
    let Nullable::Value(Value::Object(object)) = value else {
        return Err(representation_error(
            operation,
            spec.source,
            "querystring form requires an object",
        ));
    };
    check_cardinality(operation, spec.source, object.len(), spec.min, spec.max)?;
    for name in spec.required {
        if !object.contains_key(*name) {
            return Err(validation_error(
                operation,
                spec.source,
                "required querystring form property is absent",
            ));
        }
    }
    let mut out = Writer::new(operation, spec.source, limit);
    for (name, value) in object {
        let part = spec
            .parts
            .iter()
            .find(|p| p.name == Some(name.as_str()))
            .or(spec.additional)
            .ok_or_else(|| {
                validation_error(
                    operation,
                    spec.source,
                    "additional querystring form property is forbidden",
                )
            })?;
        let values = if part.repeated {
            let Nullable::Value(Value::Array(values)) = value else {
                return Err(representation_error(
                    operation,
                    part.source,
                    "repeated querystring form field requires an array",
                ));
            };
            check_cardinality(
                operation,
                part.source,
                values.len(),
                part.min_items,
                part.max_items,
            )?;
            if values.is_empty() {
                return Err(representation_error(
                    operation,
                    part.source,
                    "empty querystring form field has no representation",
                ));
            }
            values.as_slice()
        } else {
            std::slice::from_ref(value)
        };
        for value in values {
            if !out.value.is_empty() {
                out.push("&")?;
            }
            match part.kind {
                PartKind::Style(serialization) => {
                    let parameter = Parameter {
                        name: part.name.unwrap_or(""),
                        source: part.source,
                        location: ParameterLocation::Query,
                        required: part.required,
                        serialization,
                    };
                    out.push(&super::serialize_parameter(
                        operation,
                        &parameter,
                        value,
                        limit.min(part.max_bytes).min(part_limit),
                        json,
                    )?)?;
                }
                PartKind::Json | PartKind::Text(_) => {
                    let text = if let PartKind::Text(ty) = part.kind {
                        super::scalar_text(operation, part.source, value, Some(ty))?
                    } else {
                        let mut json = json;
                        json.max_output_bytes = json
                            .max_output_bytes
                            .min(limit.min(part.max_bytes).min(part_limit));
                        crate::stringify_json(value, json)
                            .map_err(|e| request_codec_error(operation, part.source, e.into()))?
                    };
                    if text.len() > part.max_bytes.min(part_limit) {
                        return Err(resource_error(
                            operation,
                            part.source,
                            "querystring form part byte ceiling exceeded",
                        ));
                    }
                    out.push(&encode(
                        operation,
                        part.source,
                        name,
                        PercentEncoding::FormUrlEncoded,
                        limit,
                    )?)?;
                    out.push("=")?;
                    out.push(&encode(
                        operation,
                        part.source,
                        &text,
                        PercentEncoding::FormUrlEncoded,
                        limit,
                    )?)?;
                }
                PartKind::Binary => {
                    return Err(representation_error(
                        operation,
                        part.source,
                        "raw bytes have no form querystring representation",
                    ));
                }
            }
        }
    }
    Ok(out.value)
}
impl<T> Part<T> {
    #[must_use]
    pub fn new(data: T) -> Self {
        Self {
            data,
            filename: None,
            content_type: None,
            headers: (),
        }
    }
}
impl<T, H> Part<T, H> {
    #[must_use]
    pub fn with_headers(data: T, headers: H) -> Self {
        Self {
            data,
            headers,
            filename: None,
            content_type: None,
        }
    }
    #[must_use]
    pub fn with_filename(mut self, value: impl Into<String>) -> Self {
        self.filename = Some(value.into());
        self
    }
    #[must_use]
    pub fn with_content_type(mut self, value: impl Into<String>) -> Self {
        self.content_type = Some(value.into());
        self
    }
}
#[derive(Debug, Clone, Copy)]
pub enum PartKind {
    Json,
    Text(ScalarType),
    Binary,
    Style(Serialization),
}
#[derive(Debug, Clone, Copy)]
pub struct PartSpec {
    pub source: Source,
    pub name: Option<&'static str>,
    pub required: bool,
    pub repeated: bool,
    pub min_items: Option<u64>,
    pub max_items: Option<u64>,
    pub media: &'static [Media],
    pub kind: PartKind,
    pub max_bytes: usize,
}
#[derive(Debug, Clone, Copy)]
pub struct AggregateSpec {
    pub source: Source,
    pub multipart: bool,
    pub positional: bool,
    pub parts: &'static [PartSpec],
    pub additional: Option<&'static PartSpec>,
    pub required: &'static [&'static str],
    pub min: Option<u64>,
    pub max: Option<u64>,
}
pub enum PartValue<'a> {
    Json(&'a JsonValue),
    Bytes(&'a [u8]),
}
/// Checked serialized part, also used as the raw input to a bound response codec.
pub struct EncodedPart {
    pub name: Option<String>,
    pub bytes: Vec<u8>,
    pub content_type: Option<String>,
    pub headers: Headers,
    pub filename: Option<String>,
    pub styled: bool,
}
pub struct ParsedParts {
    pub fields: Vec<Vec<EncodedPart>>,
    pub additional: BTreeMap<String, Vec<EncodedPart>>,
    pub items: Vec<EncodedPart>,
}

pub fn check_cardinality(
    operation: Source,
    source: Source,
    count: usize,
    min: Option<u64>,
    max: Option<u64>,
) -> Result<(), SdkError> {
    let count = u64::try_from(count).map_err(|_| {
        resource_error(
            operation,
            source,
            "aggregate count exceeds the portable count range",
        )
    })?;
    if min.is_some_and(|min| count < min) || max.is_some_and(|max| count > max) {
        return Err(validation_error(
            operation,
            source,
            "aggregate cardinality violates its source schema",
        ));
    }
    Ok(())
}

/// Structural validation over actual part counts, never a JSON aggregate with
/// binary placeholders. Required extras, duplicates and repeated item counts
/// are checked independently of the JSON/text item codecs.
pub fn validate_counts(
    op: &Operation,
    spec: &AggregateSpec,
    counts: &[(String, usize)],
) -> Result<(), SdkError> {
    if spec.positional {
        let count = counts
            .iter()
            .try_fold(0usize, |total, (_, n)| total.checked_add(*n))
            .ok_or_else(|| resource_error(op.source, spec.source, "part count overflow"))?;
        check_cardinality(op.source, spec.source, count, spec.min, spec.max)?;
        if spec.additional.is_none() && count > spec.parts.len() {
            return Err(validation_error(
                op.source,
                spec.source,
                "additional positional parts are forbidden",
            ));
        }
        return Ok(());
    }
    let mut names = BTreeSet::new();
    for (name, count) in counts {
        if !names.insert(name.as_str()) {
            return Err(validation_error(
                op.source,
                spec.source,
                "duplicate aggregate property",
            ));
        }
        let part = spec
            .parts
            .iter()
            .find(|p| p.name == Some(name.as_str()))
            .or(spec.additional)
            .ok_or_else(|| {
                validation_error(
                    op.source,
                    spec.source,
                    "additional form or multipart property is forbidden",
                )
            })?;
        if *count == 0 {
            return Err(representation_error(
                op.source,
                part.source,
                "an empty repeated property has no wire representation; omit an optional property",
            ));
        }
        if part.repeated {
            check_cardinality(
                op.source,
                part.source,
                *count,
                part.min_items,
                part.max_items,
            )?;
        } else if *count != 1 {
            return Err(validation_error(
                op.source,
                part.source,
                "a scalar part property occurs more than once",
            ));
        }
    }
    for required in spec.required {
        if !names.contains(required) {
            return Err(validation_error(
                op.source,
                spec.source,
                "a required form or multipart property is absent",
            ));
        }
    }
    check_cardinality(op.source, spec.source, names.len(), spec.min, spec.max)
}

pub fn encode_part(
    op: &Operation,
    spec: &PartSpec,
    name: Option<&str>,
    value: PartValue<'_>,
    filename: Option<&str>,
    content_type: Option<&str>,
    headers: Headers,
    multipart: bool,
    limits: Limits,
) -> Result<EncodedPart, SdkError> {
    let limit = limits.part.min(spec.max_bytes).min(limits.request);
    let bytes = match (spec.kind, value) {
        (PartKind::Binary, PartValue::Bytes(value)) => {
            super::checked_bytes(op.source, spec.source, value, limit)?
        }
        (PartKind::Json, PartValue::Json(value)) => {
            let mut json = op.json_limits;
            json.max_output_bytes = json.max_output_bytes.min(limit);
            crate::stringify_json(value, json)
                .map(String::into_bytes)
                .map_err(|e| request_codec_error(op.source, spec.source, e.into()))?
        }
        (PartKind::Text(ty), PartValue::Json(value)) => {
            let text = super::scalar_text(op.source, spec.source, value, Some(ty))?;
            if text.len() > limit {
                return Err(resource_error(
                    op.source,
                    spec.source,
                    "text part exceeds its byte ceiling",
                ));
            }
            text.into_bytes()
        }
        (PartKind::Style(serialization), PartValue::Json(value)) => {
            let parameter = Parameter {
                name: spec.name.unwrap_or(""),
                source: spec.source,
                location: ParameterLocation::Query,
                required: spec.required,
                serialization,
            };
            // The public serializer's name is static. Additional parts have no
            // explicit Encoding Object, so styled dynamic names never occur.
            super::serialize_parameter(op.source, &parameter, value, limit, op.json_limits)?
                .into_bytes()
        }
        _ => {
            return Err(representation_error(
                op.source,
                spec.source,
                "part value has the wrong native representation",
            ));
        }
    };
    let content_type = if matches!(spec.kind, PartKind::Style(_)) {
        if content_type.is_some() {
            return Err(representation_error(
                op.source,
                spec.source,
                "RFC6570-style parts do not use contentType",
            ));
        }
        None
    } else {
        let value = content_type
            .or_else(|| spec.media.first().map(|m| m.declared))
            .ok_or_else(|| {
                representation_error(
                    op.source,
                    spec.source,
                    "part requires a concrete content type",
                )
            })?;
        super::media::select(spec.media, value).map_err(|_| {
            representation_error(
                op.source,
                spec.source,
                "part content type is invalid or undeclared",
            )
        })?;
        Some(value.to_owned())
    };
    super::check_headers(op.source, spec.source, &headers, limits.header, false)?;
    if headers.iter().any(|(n, _)| {
        n.eq_ignore_ascii_case("content-type")
            || n.eq_ignore_ascii_case("content-transfer-encoding")
    }) {
        return Err(representation_error(
            op.source,
            spec.source,
            "part content type or transfer encoding cannot be overridden by an arbitrary header",
        ));
    }
    if !multipart && (filename.is_some() || !headers.is_empty()) {
        return Err(representation_error(
            op.source,
            spec.source,
            "form-urlencoded values have no filenames or part headers",
        ));
    }
    if let Some(filename) = filename {
        quote(op.source, spec.source, filename, limits.header)?;
    }
    if let Some(name) = name {
        quote(op.source, spec.source, name, limits.header)?;
    }
    Ok(EncodedPart {
        name: name.map(str::to_owned),
        bytes,
        content_type,
        headers,
        filename: filename.map(str::to_owned),
        styled: matches!(spec.kind, PartKind::Style(_)),
    })
}

pub fn prepare_parts(
    op: &Operation,
    spec: &AggregateSpec,
    parts: Vec<EncodedPart>,
    content_type: &str,
    limits: Limits,
) -> Result<(Vec<u8>, String), SdkError> {
    let counts = counts(spec, &parts)?;
    validate_counts(op, spec, &counts)?;
    if !spec.multipart {
        let mut out = Writer::new(op.source, spec.source, limits.request);
        for (index, part) in parts.iter().enumerate() {
            if index != 0 {
                out.push("&")?;
            }
            if part.styled {
                out.push(std::str::from_utf8(&part.bytes).map_err(|_| {
                    representation_error(op.source, spec.source, "styled form is not UTF-8")
                })?)?;
            } else {
                out.push(&encode(
                    op.source,
                    spec.source,
                    part.name.as_deref().ok_or_else(|| {
                        representation_error(op.source, spec.source, "form field has no name")
                    })?,
                    PercentEncoding::FormUrlEncoded,
                    limits.request,
                )?)?;
                out.push("=")?;
                let text = std::str::from_utf8(&part.bytes).map_err(|_| {
                    representation_error(op.source, spec.source, "form value is not UTF-8")
                })?;
                out.push(&encode(
                    op.source,
                    spec.source,
                    text,
                    PercentEncoding::FormUrlEncoded,
                    limits.request,
                )?)?;
            }
        }
        // Check collisions introduced by exploded object encodings as well as
        // declared field identities; parsing uses the same unambiguous mapping.
        parse_form(op, spec, out.value.as_bytes(), limits)?;
        return Ok((out.value.into_bytes(), content_type.into()));
    }
    let media = ParsedMedia::parse(content_type).map_err(|_| {
        representation_error(op.source, spec.source, "invalid multipart content type")
    })?;
    let boundary = if let Some(boundary) = media.parameters.get("boundary") {
        validate_boundary(boundary).map_err(|m| representation_error(op.source, spec.source, m))?;
        boundary.clone()
    } else {
        let mut selected = None;
        for index in 0..1024 {
            let boundary = format!("suspect-boundary-{index}");
            if parts
                .iter()
                .all(|part| !contains(&part.bytes, boundary.as_bytes()))
            {
                selected = Some(boundary);
                break;
            }
        }
        selected.ok_or_else(|| {
            resource_error(
                op.source,
                spec.source,
                "multipart boundary selection budget exhausted",
            )
        })?
    };
    if parts
        .iter()
        .any(|part| contains(&part.bytes, boundary.as_bytes()))
    {
        return Err(representation_error(
            op.source,
            spec.source,
            "multipart boundary occurs in a part body",
        ));
    }
    let mut head = Vec::new();
    let mut total = boundary.len().saturating_add(6);
    for part in &parts {
        if part.bytes.len() > limits.part {
            return Err(resource_error(
                op.source,
                spec.source,
                "part exceeds its byte ceiling",
            ));
        }
        let mut text = Writer::new(op.source, spec.source, limits.header);
        let declared_disposition = part
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-disposition"));
        if part
            .headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("content-disposition"))
            .count()
            > 1
        {
            return Err(representation_error(
                op.source,
                spec.source,
                "duplicate Content-Disposition part header",
            ));
        }
        let parsed_disposition = declared_disposition
            .map(|(_, value)| disposition(op, spec.source, value))
            .transpose()?;
        if spec.positional
            && media.type_name == "multipart"
            && media.subtype == "form-data"
            && parsed_disposition
                .as_ref()
                .is_none_or(|(name, _)| name.is_none())
        {
            return Err(representation_error(
                op.source,
                spec.source,
                "positional form-data parts require a Content-Disposition name",
            ));
        }
        if !spec.positional {
            let name = part.name.as_deref().ok_or_else(|| {
                representation_error(
                    op.source,
                    spec.source,
                    "named multipart part is missing its name",
                )
            })?;
            if let Some((declared, filename)) = &parsed_disposition {
                if declared.as_deref() != Some(name) {
                    return Err(representation_error(
                        op.source,
                        spec.source,
                        "declared disposition does not match the part property name",
                    ));
                }
                if part.filename.is_some() && part.filename != *filename {
                    return Err(representation_error(
                        op.source,
                        spec.source,
                        "filename cannot override a declared Content-Disposition header",
                    ));
                }
            } else {
                text.push("Content-Disposition: form-data; name=")?;
                text.push(&quote(op.source, spec.source, name, limits.header)?)?;
                if let Some(filename) = &part.filename {
                    text.push("; filename=")?;
                    text.push(&quote(op.source, spec.source, filename, limits.header)?)?;
                }
                text.push("\r\n")?;
            }
        } else if part.filename.is_some()
            && parsed_disposition
                .as_ref()
                .is_none_or(|(_, filename)| filename != &part.filename)
        {
            return Err(representation_error(
                op.source,
                spec.source,
                "positional filename needs a declared Content-Disposition header",
            ));
        }
        if let Some(media) = &part.content_type {
            text.push("Content-Type: ")?;
            text.push(media)?;
            text.push("\r\n")?;
        }
        for (name, value) in &part.headers {
            text.push(name)?;
            text.push(": ")?;
            text.push(std::str::from_utf8(value).map_err(|_| {
                representation_error(op.source, spec.source, "typed part header is not UTF-8")
            })?)?;
            text.push("\r\n")?;
        }
        total = total
            .saturating_add(boundary.len())
            .saturating_add(8)
            .saturating_add(text.value.len())
            .saturating_add(part.bytes.len());
        if total > limits.request {
            return Err(resource_error(
                op.source,
                spec.source,
                "multipart body exceeds its byte ceiling before assembly",
            ));
        }
        head.push(text.value);
    }
    let mut bytes = Vec::with_capacity(total);
    for (part, head) in parts.into_iter().zip(head) {
        bytes.extend_from_slice(b"--");
        bytes.extend_from_slice(boundary.as_bytes());
        bytes.extend_from_slice(b"\r\n");
        bytes.extend_from_slice(head.as_bytes());
        bytes.extend_from_slice(b"\r\n");
        bytes.extend_from_slice(&part.bytes);
        bytes.extend_from_slice(b"\r\n");
    }
    bytes.extend_from_slice(b"--");
    bytes.extend_from_slice(boundary.as_bytes());
    bytes.extend_from_slice(b"--\r\n");
    let content_type = if media.parameters.contains_key("boundary") {
        content_type.to_owned()
    } else {
        format!("{content_type}; boundary={boundary}")
    };
    Ok((bytes, content_type))
}

fn counts(spec: &AggregateSpec, parts: &[EncodedPart]) -> Result<Vec<(String, usize)>, SdkError> {
    if spec.positional {
        return Ok(vec![(String::new(), parts.len())]);
    }
    let mut counts = BTreeMap::new();
    for part in parts {
        let name = part.name.as_ref().ok_or_else(|| {
            representation_error(spec.source, spec.source, "named part is missing its name")
        })?;
        *counts.entry(name.clone()).or_insert(0) += 1;
    }
    Ok(counts.into_iter().collect())
}
fn quote(operation: Source, source: Source, value: &str, limit: usize) -> Result<String, SdkError> {
    if value.chars().any(char::is_control) {
        return Err(representation_error(
            operation,
            source,
            "part name or filename contains control characters",
        ));
    }
    let mut text = Writer::new(operation, source, limit);
    text.push("\"")?;
    for c in value.chars() {
        if matches!(c, '"' | '\\') {
            text.push("\\")?;
        }
        text.push(&c.to_string())?;
    }
    text.push("\"")?;
    Ok(text.value)
}
fn validate_boundary(boundary: &str) -> Result<(), &'static str> {
    if boundary.is_empty()
        || boundary.len() > 70
        || boundary.ends_with(' ')
        || !boundary
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"'()+_,-./:=? ".contains(&b))
    {
        Err("invalid MIME multipart boundary")
    } else {
        Ok(())
    }
}
fn contains(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|w| w == needle)
}

/// Parse a bounded complete form/multipart body into finite raw parts. Native
/// generated code then consumes each part through its actual bound item codec.
pub fn parse_parts(
    op: &Operation,
    spec: &AggregateSpec,
    bytes: &[u8],
    content_type: &str,
    limits: Limits,
) -> Result<ParsedParts, SdkError> {
    if bytes.len() > limits.response {
        return Err(resource_error(
            op.source,
            spec.source,
            "aggregate response exceeds its byte ceiling",
        ));
    }
    let parts = if spec.multipart {
        parse_multipart(op, spec, bytes, content_type, limits)?
    } else {
        parse_form(op, spec, bytes, limits)?
    };
    validate_counts(op, spec, &counts(spec, &parts)?)?;
    let mut result = ParsedParts {
        fields: spec.parts.iter().map(|_| Vec::new()).collect(),
        additional: BTreeMap::new(),
        items: Vec::new(),
    };
    for (index, part) in parts.into_iter().enumerate() {
        if spec.positional {
            if index < spec.parts.len() {
                result.fields[index].push(part);
            } else {
                result.items.push(part);
            }
        } else if let Some(index) = spec
            .parts
            .iter()
            .position(|s| s.name == part.name.as_deref())
        {
            result.fields[index].push(part);
        } else {
            result
                .additional
                .entry(part.name.clone().expect("checked named part"))
                .or_default()
                .push(part);
        }
    }
    Ok(result)
}
fn parse_multipart(
    op: &Operation,
    spec: &AggregateSpec,
    bytes: &[u8],
    content_type: &str,
    limits: Limits,
) -> Result<Vec<EncodedPart>, SdkError> {
    let media = ParsedMedia::parse(content_type).map_err(|_| {
        representation_error(op.source, spec.source, "invalid multipart Content-Type")
    })?;
    let boundary = media.parameters.get("boundary").ok_or_else(|| {
        representation_error(
            op.source,
            spec.source,
            "multipart Content-Type has no boundary",
        )
    })?;
    validate_boundary(boundary).map_err(|m| representation_error(op.source, spec.source, m))?;
    let marker = format!("--{boundary}").into_bytes();
    let bad = || representation_error(op.source, spec.source, "malformed multipart framing");
    let next_boundary = |start: usize| -> Option<(usize, bool, usize)> {
        for index in start..bytes.len() {
            if index != 0 && (index < 2 || &bytes[index - 2..index] != b"\r\n") {
                continue;
            }
            if bytes.get(index..index + marker.len()) != Some(marker.as_slice()) {
                continue;
            }
            let mut at = index + marker.len();
            let closing = bytes.get(at..at + 2) == Some(b"--");
            if closing {
                at += 2;
            }
            while matches!(bytes.get(at), Some(b' ' | b'\t')) {
                at += 1;
            }
            if bytes.get(at..at + 2) == Some(b"\r\n") {
                return Some((index, closing, at + 2));
            }
            if closing && at == bytes.len() {
                return Some((index, true, at));
            }
        }
        None
    };
    let (_, mut closing, mut at) = next_boundary(0).ok_or_else(bad)?;
    let mut parts = Vec::new();
    while !closing {
        let (next, end, after) = next_boundary(at).ok_or_else(bad)?;
        if next < 2 || next - 2 < at {
            return Err(bad());
        }
        let part = &bytes[at..next - 2];
        let (header_end, body_start) = if part.starts_with(b"\r\n") {
            (0, 2)
        } else {
            let end = part
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .ok_or_else(bad)?;
            (end, end + 4)
        };
        if header_end > limits.header {
            return Err(resource_error(
                op.source,
                spec.source,
                "part header ceiling exceeded",
            ));
        }
        let mut headers = Vec::new();
        for line in part[..header_end]
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
        {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            let colon = line.iter().position(|b| *b == b':').ok_or_else(bad)?;
            let name = std::str::from_utf8(&line[..colon]).map_err(|_| bad())?;
            if !super::media::is_token(name)
                || name.eq_ignore_ascii_case("content-transfer-encoding")
            {
                return Err(bad());
            }
            let value = &line[colon + 1..];
            let start = value
                .iter()
                .position(|b| !matches!(b, b' ' | b'\t'))
                .unwrap_or(value.len());
            let value = &value[start..];
            let end = value
                .iter()
                .rposition(|b| !matches!(b, b' ' | b'\t'))
                .map_or(0, |at| at + 1);
            headers.push((name.into(), value[..end].to_vec()));
        }
        super::check_headers(op.source, spec.source, &headers, limits.header, true)?;
        let dispositions = headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("content-disposition"))
            .collect::<Vec<_>>();
        if dispositions.len() > 1 {
            return Err(bad());
        }
        let (name, filename) = if let Some((_, value)) = dispositions.first() {
            disposition(op, spec.source, value)?
        } else {
            (None, None)
        };
        if (!spec.positional || media.type_name == "multipart" && media.subtype == "form-data")
            && name.is_none()
        {
            return Err(bad());
        }
        let binding = if spec.positional {
            spec.parts.get(parts.len()).or(spec.additional)
        } else {
            spec.parts
                .iter()
                .find(|p| p.name == name.as_deref())
                .or(spec.additional)
        }
        .ok_or_else(|| validation_error(op.source, spec.source, "unexpected multipart part"))?;
        let data = &part[body_start..];
        if data.len() > limits.part.min(binding.max_bytes) {
            return Err(resource_error(
                op.source,
                binding.source,
                "multipart part byte ceiling exceeded",
            ));
        }
        let content_types = headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .count();
        let content_type = super::media::content_type(&headers);
        if content_types != 0 && content_type.is_none() {
            return Err(bad());
        }
        if !matches!(binding.kind, PartKind::Style(_)) {
            super::media::select(
                binding.media,
                content_type.as_deref().unwrap_or("text/plain"),
            )
            .map_err(|_| {
                representation_error(
                    op.source,
                    binding.source,
                    "part media does not match its declaration",
                )
            })?;
        }
        parts.push(EncodedPart {
            name,
            filename,
            content_type,
            headers,
            bytes: data.to_vec(),
            styled: matches!(binding.kind, PartKind::Style(_)),
        });
        at = after;
        closing = end;
    }
    Ok(parts)
}

fn disposition(
    op: &Operation,
    source: Source,
    bytes: &[u8],
) -> Result<(Option<String>, Option<String>), SdkError> {
    let bad = || representation_error(op.source, source, "invalid Content-Disposition header");
    let text = std::str::from_utf8(bytes).map_err(|_| bad())?;
    let fields = super::media::split_quoted(text, ';').map_err(|_| bad())?;
    if !super::media::is_token(fields[0].trim()) {
        return Err(bad());
    }
    let mut values = BTreeMap::new();
    for field in &fields[1..] {
        let (name, value) = field.trim().split_once('=').ok_or_else(bad)?;
        let value = if value.starts_with('"') {
            super::media::unquote(value).map_err(|_| bad())?
        } else {
            if !super::media::is_token(value) {
                return Err(bad());
            }
            value.into()
        };
        if !super::media::is_token(name)
            || name.eq_ignore_ascii_case("filename*")
            || values.insert(name.to_ascii_lowercase(), value).is_some()
        {
            return Err(bad());
        }
    }
    if values.contains_key("name") && !fields[0].trim().eq_ignore_ascii_case("form-data") {
        return Err(bad());
    }
    Ok((values.remove("name"), values.remove("filename")))
}

fn parse_form(
    op: &Operation,
    spec: &AggregateSpec,
    bytes: &[u8],
    limits: Limits,
) -> Result<Vec<EncodedPart>, SdkError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| representation_error(op.source, spec.source, "form is not UTF-8"))?;
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let mut result = Vec::new();
    let mut styled: BTreeMap<String, (String, &PartSpec)> = BTreeMap::new();
    for fragment in text.split('&') {
        let (name, value) = fragment.split_once('=').ok_or_else(|| {
            representation_error(op.source, spec.source, "form field has no equals delimiter")
        })?;
        let name = percent_decode(op.source, spec.source, name, true)?;
        let mut matches = spec
            .parts
            .iter()
            .filter(|p| form_name_matches(p, &name))
            .collect::<Vec<_>>();
        let binding = if matches.len() == 1 {
            matches.remove(0)
        } else if matches.is_empty() {
            spec.additional.ok_or_else(|| {
                validation_error(
                    op.source,
                    spec.source,
                    "additional form property is forbidden",
                )
            })?
        } else {
            return Err(representation_error(
                op.source,
                spec.source,
                "form field has ambiguous exploded property ownership",
            ));
        };
        if matches!(binding.kind, PartKind::Style(_)) && !binding.repeated {
            let field = binding.name.unwrap_or(&name).to_owned();
            let entry = styled
                .entry(field)
                .or_insert_with(|| (String::new(), binding));
            if entry
                .0
                .len()
                .saturating_add(fragment.len())
                .saturating_add(usize::from(!entry.0.is_empty()))
                > limits.part.min(binding.max_bytes)
            {
                return Err(resource_error(
                    op.source,
                    binding.source,
                    "form field exceeds part byte ceiling",
                ));
            }
            if !entry.0.is_empty() {
                entry.0.push('&');
            }
            entry.0.push_str(fragment);
        } else {
            let bytes = if matches!(binding.kind, PartKind::Style(_)) {
                fragment.as_bytes().to_vec()
            } else {
                percent_decode(op.source, binding.source, value, true)?.into_bytes()
            };
            if bytes.len() > limits.part.min(binding.max_bytes) {
                return Err(resource_error(
                    op.source,
                    binding.source,
                    "form value exceeds part byte ceiling",
                ));
            }
            result.push(EncodedPart {
                name: Some(binding.name.unwrap_or(&name).into()),
                bytes,
                filename: None,
                content_type: None,
                headers: Vec::new(),
                styled: matches!(binding.kind, PartKind::Style(_)),
            });
        }
    }
    for (name, (value, _)) in styled {
        result.push(EncodedPart {
            name: Some(name),
            bytes: value.into_bytes(),
            filename: None,
            content_type: None,
            headers: Vec::new(),
            styled: true,
        });
    }
    Ok(result)
}
fn form_name_matches(p: &PartSpec, name: &str) -> bool {
    match p.kind {
        PartKind::Style(Serialization::Style {
            style: super::Style::Form,
            explode: true,
            shape:
                Shape::Object {
                    properties,
                    additional,
                },
            ..
        }) => {
            properties.iter().any(|(key, _)| *key == name)
                || additional != super::AdditionalScalars::Forbidden
        }
        PartKind::Style(Serialization::Style {
            style: super::Style::DeepObject,
            ..
        }) => p
            .name
            .is_some_and(|field| name.starts_with(&format!("{field}[")) && name.ends_with(']')),
        _ => p.name == Some(name),
    }
}

pub fn decode_part_value(
    op: &Operation,
    spec: &PartSpec,
    part: &EncodedPart,
    multipart: bool,
) -> Result<JsonValue, SdkError> {
    match spec.kind {
        PartKind::Json => crate::parse_json_bytes(&part.bytes, op.json_limits)
            .map_err(|e| request_codec_error(op.source, spec.source, e.into())),
        PartKind::Text(ty) => super::scalar_value(
            op.source,
            spec.source,
            std::str::from_utf8(&part.bytes).map_err(|_| {
                representation_error(op.source, spec.source, "text part is not UTF-8")
            })?,
            ty,
        ),
        PartKind::Style(serialization) => decode_style(
            op,
            spec,
            serialization,
            std::str::from_utf8(&part.bytes).map_err(|_| {
                representation_error(op.source, spec.source, "styled part is not UTF-8")
            })?,
            multipart,
        ),
        PartKind::Binary => Err(representation_error(
            op.source,
            spec.source,
            "binary parts are native bytes and cannot enter a JSON codec",
        )),
    }
}
fn decode_style(
    op: &Operation,
    spec: &PartSpec,
    serialization: Serialization,
    text: &str,
    multipart: bool,
) -> Result<JsonValue, SdkError> {
    let Serialization::Style {
        style,
        explode,
        shape,
        ..
    } = serialization
    else {
        return Err(representation_error(
            op.source,
            spec.source,
            "part style descriptor is invalid",
        ));
    };
    let decode = |text: &str| {
        if multipart {
            Ok(text.into())
        } else {
            percent_decode(op.source, spec.source, text, false)
        }
    };
    let bad = || representation_error(op.source, spec.source, "invalid RFC6570 form part");
    let mut pairs = Vec::new();
    for fragment in text.split('&') {
        pairs.push(fragment.split_once('=').ok_or_else(bad)?);
    }
    let value = |ty: ScalarType, text: &str| {
        super::scalar_value(op.source, spec.source, &decode(text)?, ty)
    };
    match shape {
        Shape::Scalar(ty) => {
            if pairs.len() != 1 {
                return Err(bad());
            }
            value(ty, pairs[0].1)
        }
        Shape::Array(ty) => {
            let values = if explode {
                pairs
                    .iter()
                    .map(|(_, v)| value(ty, v))
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                if pairs.len() != 1 {
                    return Err(bad());
                }
                if matches!(
                    style,
                    super::Style::SpaceDelimited | super::Style::PipeDelimited
                ) {
                    split_encoded_delimiter(
                        pairs[0].1,
                        if style == super::Style::SpaceDelimited {
                            b"%20"
                        } else {
                            b"%7C"
                        },
                    )
                    .into_iter()
                    .map(|v| value(ty, v))
                    .collect::<Result<Vec<_>, _>>()?
                } else {
                    pairs[0]
                        .1
                        .split(',')
                        .map(|v| value(ty, v))
                        .collect::<Result<Vec<_>, _>>()?
                }
            };
            Ok(Nullable::Value(Value::Array(values)))
        }
        Shape::Object {
            properties,
            additional,
        } => {
            let mut object = BTreeMap::new();
            let mut add = |key: String, text: String| -> Result<(), SdkError> {
                let ty = properties
                    .iter()
                    .find(|(name, _)| *name == key)
                    .map(|(_, ty)| *ty)
                    .or(match additional {
                        super::AdditionalScalars::Forbidden => None,
                        super::AdditionalScalars::Typed(ty) => Some(ty),
                        super::AdditionalScalars::AnyScalar => Some(ScalarType::String),
                    })
                    .ok_or_else(bad)?;
                if object
                    .insert(key, super::scalar_value(op.source, spec.source, &text, ty)?)
                    .is_some()
                {
                    return Err(bad());
                }
                Ok(())
            };
            if explode || style == super::Style::DeepObject {
                for (key, v) in pairs {
                    let key = if style == super::Style::DeepObject && multipart {
                        let name = encode(
                            op.source,
                            spec.source,
                            spec.name.unwrap_or(""),
                            PercentEncoding::UriComponent,
                            op.limits.request,
                        )?;
                        let (prefix, tail) = key.split_at_checked(name.len()).ok_or_else(bad)?;
                        if percent_decode(op.source, spec.source, prefix, false)?
                            != spec.name.unwrap_or("")
                            || !tail
                                .as_bytes()
                                .get(..3)
                                .is_some_and(|s| s.eq_ignore_ascii_case(b"%5B"))
                            || !tail
                                .as_bytes()
                                .get(tail.len().saturating_sub(3)..)
                                .is_some_and(|s| s.eq_ignore_ascii_case(b"%5D"))
                            || tail.len() < 6
                        {
                            return Err(bad());
                        }
                        tail[3..tail.len() - 3].to_owned()
                    } else if style == super::Style::DeepObject {
                        let key = decode(key)?;
                        key.strip_prefix(&format!("{}[", spec.name.unwrap_or("")))
                            .and_then(|v| v.strip_suffix(']'))
                            .ok_or_else(bad)?
                            .to_owned()
                    } else {
                        decode(key)?
                    };
                    add(key, decode(v)?)?;
                }
            } else {
                if pairs.len() != 1 {
                    return Err(bad());
                }
                let parts = if matches!(
                    style,
                    super::Style::SpaceDelimited | super::Style::PipeDelimited
                ) {
                    split_encoded_delimiter(
                        pairs[0].1,
                        if style == super::Style::SpaceDelimited {
                            b"%20"
                        } else {
                            b"%7C"
                        },
                    )
                } else {
                    pairs[0].1.split(',').collect::<Vec<_>>()
                };
                if parts.len() % 2 != 0 {
                    return Err(bad());
                }
                for pair in parts.chunks_exact(2) {
                    add(decode(pair[0])?, decode(pair[1])?)?;
                }
            }
            Ok(Nullable::Value(Value::Object(object)))
        }
    }
}

fn split_encoded_delimiter<'a>(text: &'a str, delimiter: &[u8; 3]) -> Vec<&'a str> {
    let mut values = Vec::new();
    let mut start = 0;
    for (index, window) in text.as_bytes().windows(3).enumerate() {
        if index >= start && window.eq_ignore_ascii_case(delimiter) {
            values.push(&text[start..index]);
            start = index + 3;
        }
    }
    values.push(&text[start..]);
    values
}
