use super::{
    Headers, JsonLimits, SdkError, Source, representation_error, request_codec_error,
    resource_error,
};
use crate::{JsonInteger, JsonNonNullValue as Value, JsonNumber, JsonValue, Nullable};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterLocation {
    Path,
    Query,
    Querystring,
    Header,
    Cookie,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Simple,
    Label,
    Matrix,
    Form,
    SpaceDelimited,
    PipeDelimited,
    DeepObject,
    Cookie,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PercentEncoding {
    UriComponent,
    ReservedExpansion,
    None,
    FormUrlEncoded,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarType {
    String,
    Boolean,
    Integer,
    Number,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdditionalScalars {
    Forbidden,
    Typed(ScalarType),
    AnyScalar,
}
#[derive(Debug, Clone, Copy)]
pub enum Shape {
    Scalar(ScalarType),
    Array(ScalarType),
    Object {
        properties: &'static [(&'static str, ScalarType)],
        additional: AdditionalScalars,
    },
}
#[derive(Debug, Clone, Copy)]
pub enum Serialization {
    Form {
        spec: &'static super::AggregateSpec,
    },
    Style {
        style: Style,
        explode: bool,
        shape: Shape,
        encoding: PercentEncoding,
    },
    Content {
        json: bool,
        scalar: ScalarType,
        encoding: PercentEncoding,
    },
}
#[derive(Debug, Clone, Copy)]
pub struct Parameter {
    pub name: &'static str,
    pub source: Source,
    pub location: ParameterLocation,
    pub required: bool,
    pub serialization: Serialization,
}
#[derive(Debug, Clone)]
pub struct ParameterValue {
    pub parameter: Parameter,
    pub value: JsonValue,
}

pub(super) fn empty_composite(value: &JsonValue) -> bool {
    matches!(value,Nullable::Value(Value::Array(v)) if v.is_empty())
        || matches!(value,Nullable::Value(Value::Object(v)) if v.is_empty())
}

fn scalar<'a>(value: &'a JsonValue, ty: Option<ScalarType>) -> Option<&'a str> {
    match value {
        Nullable::Value(Value::String(s)) if ty.is_none_or(|t| t == ScalarType::String) => Some(s),
        Nullable::Value(Value::Bool(b)) if ty.is_none_or(|t| t == ScalarType::Boolean) => {
            Some(if *b { "true" } else { "false" })
        }
        Nullable::Value(Value::Number(n))
            if ty.is_none_or(|t| matches!(t, ScalarType::Number | ScalarType::Integer)) =>
        {
            if ty == Some(ScalarType::Integer) && n.as_str().parse::<JsonInteger>().is_err() {
                None
            } else {
                Some(n.as_str())
            }
        }
        _ => None,
    }
}
pub fn scalar_text(
    operation: Source,
    source: Source,
    value: &JsonValue,
    ty: Option<ScalarType>,
) -> Result<String, SdkError> {
    scalar(value, ty).map(str::to_owned).ok_or_else(|| {
        representation_error(
            operation,
            source,
            "value requires a non-null scalar representation",
        )
    })
}

/// Check scalar text length before making a byte copy.
pub fn scalar_bytes(
    operation: Source,
    source: Source,
    value: &JsonValue,
    ty: ScalarType,
    limit: usize,
) -> Result<Vec<u8>, SdkError> {
    let text = scalar(value, Some(ty)).ok_or_else(|| {
        representation_error(
            operation,
            source,
            "value requires its declared scalar representation",
        )
    })?;
    super::checked_bytes(operation, source, text.as_bytes(), limit)
}
pub fn scalar_value(
    operation: Source,
    source: Source,
    text: &str,
    ty: ScalarType,
) -> Result<JsonValue, SdkError> {
    let invalid = || {
        representation_error(
            operation,
            source,
            "text does not represent its declared scalar type",
        )
    };
    let value = match ty {
        ScalarType::String => Value::String(text.into()),
        ScalarType::Boolean => Value::Bool(match text {
            "true" => true,
            "false" => false,
            _ => return Err(invalid()),
        }),
        ScalarType::Integer | ScalarType::Number => {
            if ty == ScalarType::Integer {
                text.parse::<JsonInteger>().map_err(|_| invalid())?;
            }
            Value::Number(text.parse::<JsonNumber>().map_err(|_| invalid())?)
        }
    };
    Ok(Nullable::Value(value))
}

/// Bounded append; escaping and repeated names never allocate the final output
/// before proving its ceiling. The bound is independent of JSON codec budgets.
pub(super) struct Writer {
    pub value: String,
    limit: usize,
    operation: Source,
    source: Source,
}
impl Writer {
    pub fn new(operation: Source, source: Source, limit: usize) -> Self {
        Self {
            value: String::new(),
            limit,
            operation,
            source,
        }
    }
    pub fn push(&mut self, value: &str) -> Result<(), SdkError> {
        if value.len() > self.limit.saturating_sub(self.value.len()) {
            return Err(resource_error(
                self.operation,
                self.source,
                "serialized wire value exceeds its byte ceiling",
            ));
        }
        self.value.push_str(value);
        Ok(())
    }
    fn separated<'a>(
        &mut self,
        values: impl IntoIterator<Item = &'a str>,
        delimiter: &str,
    ) -> Result<(), SdkError> {
        let mut first = true;
        for value in values {
            if !first {
                self.push(delimiter)?;
            }
            first = false;
            self.push(value)?;
        }
        Ok(())
    }
}

pub(super) fn encode(
    operation: Source,
    source: Source,
    value: &str,
    encoding: PercentEncoding,
    limit: usize,
) -> Result<String, SdkError> {
    if encoding == PercentEncoding::None {
        if value.len() > limit {
            return Err(resource_error(
                operation,
                source,
                "wire value exceeds its byte ceiling",
            ));
        }
        return Ok(value.into());
    }
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = Writer::new(operation, source, limit);
    let bytes = value.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        if encoding == PercentEncoding::ReservedExpansion
            && byte == b'%'
            && bytes.get(at + 1).is_some_and(u8::is_ascii_hexdigit)
            && bytes.get(at + 2).is_some_and(u8::is_ascii_hexdigit)
        {
            out.push(&value[at..at + 3])?;
            at += 3;
            continue;
        }
        let pass = if encoding == PercentEncoding::FormUrlEncoded {
            byte.is_ascii_alphanumeric() || b"*-._".contains(&byte)
        } else {
            byte.is_ascii_alphanumeric()
                || b"-._~".contains(&byte)
                || encoding == PercentEncoding::ReservedExpansion
                    && b":/?#[]@!$&'()*+,;=".contains(&byte)
        };
        if pass {
            out.push(&char::from(byte).to_string())?;
        } else if encoding == PercentEncoding::FormUrlEncoded && byte == b' ' {
            out.push("+")?;
        } else {
            let text = [
                b'%',
                HEX[usize::from(byte >> 4)],
                HEX[usize::from(byte & 15)],
            ];
            out.push(std::str::from_utf8(&text).expect("ASCII escape"))?;
        }
        at += 1;
    }
    Ok(out.value)
}

fn encode_value(
    operation: Source,
    p: &Parameter,
    value: &str,
    encoding: PercentEncoding,
    style: Option<(Style, Shape)>,
    limit: usize,
) -> Result<String, SdkError> {
    let bad = |message| representation_error(operation, p.source, message);
    if encoding == PercentEncoding::None
        && value
            .chars()
            .any(|c| c.is_control() && !(p.location == ParameterLocation::Header && c == '\t'))
    {
        return Err(bad(
            "header, cookie or part value contains a control character",
        ));
    }
    if p.location == ParameterLocation::Cookie
        && encoding == PercentEncoding::None
        && value
            .bytes()
            .any(|b| matches!(b, b' ' | b'\t' | b'"' | b',' | b';' | b'\\') || b > 126)
    {
        return Err(bad("cookie value requires caller-supplied escaping"));
    }
    if style.is_some_and(|(s, _)| match s {
        Style::SpaceDelimited => value.contains(' '),
        Style::PipeDelimited => value.contains('|'),
        Style::DeepObject => value.contains(['[', ']']),
        _ => false,
    }) {
        return Err(bad("data contains an ambiguous active style delimiter"));
    }
    if encoding == PercentEncoding::None
        && p.location == ParameterLocation::Query
        && style.is_some_and(|(style, shape)| {
            value.contains('&')
                || !matches!(shape, Shape::Scalar(_))
                    && matches!(style, Style::Form | Style::DeepObject)
                    && value.contains([',', '='])
        })
    {
        return Err(bad(
            "unencoded multipart style data contains an active form delimiter",
        ));
    }
    if matches!(
        encoding,
        PercentEncoding::None | PercentEncoding::ReservedExpansion
    ) && style.is_some_and(|(style, _)| match style {
        Style::SpaceDelimited => value
            .as_bytes()
            .windows(3)
            .any(|v| v.eq_ignore_ascii_case(b"%20")),
        Style::PipeDelimited => value
            .as_bytes()
            .windows(3)
            .any(|v| v.eq_ignore_ascii_case(b"%7C")),
        _ => false,
    }) {
        return Err(bad(
            "data already contains the encoded active style delimiter",
        ));
    }
    // Header composite delimiters cannot be percent-escaped or silently quoted.
    if p.location == ParameterLocation::Header
        && style.is_some_and(|(_, shape)| !matches!(shape, Shape::Scalar(_)))
        && value.contains([',', '='])
    {
        return Err(bad("header composite contains an ambiguous delimiter"));
    }
    if encoding == PercentEncoding::ReservedExpansion {
        let hazard = match p.location {
            ParameterLocation::Path => value.contains(['#', '[', ']', '/', '?']),
            ParameterLocation::Query | ParameterLocation::Querystring => {
                value.contains(['#', '[', ']', '&', '=', '+'])
            }
            ParameterLocation::Cookie => value.contains([';', ',']),
            ParameterLocation::Header => false,
        };
        let delimiter = style.is_some_and(|(s, shape)| {
            !matches!(shape, Shape::Scalar(_))
                && match s {
                    Style::Simple | Style::Form | Style::Cookie => value.contains(','),
                    Style::Label => value.contains(['.', ',']),
                    Style::Matrix => value.contains([';', ',']),
                    _ => false,
                }
        });
        if hazard || delimiter {
            return Err(bad(
                "allowReserved data requires caller pre-escaping of query hazards and active delimiters",
            ));
        }
    }
    encode(operation, p.source, value, encoding, limit)
}

/// Serialize a value already validated by its exact source-bound codec.
pub fn serialize_parameter(
    operation: Source,
    p: &Parameter,
    value: &JsonValue,
    limit: usize,
    json: JsonLimits,
) -> Result<String, SdkError> {
    let mut out = Writer::new(operation, p.source, limit);
    match p.serialization {
        Serialization::Form { spec } => {
            return super::parts::query_form(operation, spec, value, limit, limit, json);
        }
        Serialization::Content {
            json: is_json,
            encoding,
            ..
        } => {
            let text = if is_json {
                let mut json = json;
                json.max_output_bytes = json.max_output_bytes.min(limit);
                crate::stringify_json(value, json)
                    .map_err(|e| request_codec_error(operation, p.source, e.into()))?
            } else {
                scalar_text(operation, p.source, value, None)?
            };
            if matches!(
                p.location,
                ParameterLocation::Query | ParameterLocation::Cookie
            ) {
                out.push(&encode(
                    operation,
                    p.source,
                    p.name,
                    PercentEncoding::UriComponent,
                    limit,
                )?)?;
                out.push("=")?;
            }
            out.push(&encode_value(operation, p, &text, encoding, None, limit)?)?;
        }
        Serialization::Style {
            style,
            explode,
            shape,
            encoding,
        } => {
            let name = encode(
                operation,
                p.source,
                p.name,
                if p.location == ParameterLocation::Header || style == Style::Cookie {
                    PercentEncoding::None
                } else {
                    PercentEncoding::UriComponent
                },
                limit,
            )?;
            let bad = || {
                representation_error(
                    operation,
                    p.source,
                    "value must have its declared nonempty flat wire shape",
                )
            };
            let mut scalar_value = None;
            let mut items = Vec::new();
            let mut properties = Vec::new();
            let encoded = |value: &str| {
                encode_value(operation, p, value, encoding, Some((style, shape)), limit)
            };
            let mut total = 0usize;
            let mut account = |s: String| {
                total = total.saturating_add(s.len());
                if total > limit {
                    Err(resource_error(
                        operation,
                        p.source,
                        "composite wire values exceed the byte ceiling",
                    ))
                } else {
                    Ok(s)
                }
            };
            match shape {
                Shape::Scalar(ty) => {
                    scalar_value = Some(encoded(scalar(value, Some(ty)).ok_or_else(bad)?)?)
                }
                Shape::Array(ty) => {
                    let Nullable::Value(Value::Array(values)) = value else {
                        return Err(bad());
                    };
                    if values.is_empty() {
                        return Err(bad());
                    }
                    for v in values {
                        items.push(account(encoded(scalar(v, Some(ty)).ok_or_else(bad)?)?)?);
                    }
                }
                Shape::Object {
                    properties: declared,
                    additional,
                } => {
                    let Nullable::Value(Value::Object(values)) = value else {
                        return Err(bad());
                    };
                    if values.is_empty() {
                        return Err(bad());
                    }
                    for (k, v) in values {
                        let ty = declared
                            .iter()
                            .find(|(key, _)| key == k)
                            .map(|(_, ty)| *ty)
                            .map(Some)
                            .unwrap_or_else(|| match additional {
                                AdditionalScalars::Typed(t) => Some(t),
                                _ => None,
                            });
                        if ty.is_none() && additional == AdditionalScalars::Forbidden {
                            return Err(bad());
                        }
                        properties.push((
                            account(encoded(k)?)?,
                            account(encoded(scalar(v, ty).ok_or_else(bad)?)?)?,
                        ));
                    }
                }
            }
            let flat = |out: &mut Writer, delimiter: &str| {
                out.separated(
                    properties
                        .iter()
                        .flat_map(|(k, v)| [k.as_str(), v.as_str()]),
                    delimiter,
                )
            };
            let pairs = |out: &mut Writer, delimiter: &str| -> Result<(), SdkError> {
                for (i, (k, v)) in properties.iter().enumerate() {
                    if i != 0 {
                        out.push(delimiter)?;
                    }
                    out.push(k)?;
                    out.push("=")?;
                    out.push(v)?;
                }
                Ok(())
            };
            match style {
                Style::Simple | Style::Label => {
                    if style == Style::Label {
                        out.push(".")?;
                    }
                    if let Some(v) = &scalar_value {
                        out.push(v)?;
                    } else if matches!(shape, Shape::Array(_)) {
                        out.separated(
                            items.iter().map(String::as_str),
                            if style == Style::Label && explode {
                                "."
                            } else {
                                ","
                            },
                        )?;
                    } else if explode {
                        pairs(&mut out, if style == Style::Label { "." } else { "," })?;
                    } else {
                        flat(&mut out, ",")?;
                    }
                }
                Style::Matrix => {
                    let mut named = |name: &str, value: &str| -> Result<(), SdkError> {
                        out.push(";")?;
                        out.push(name)?;
                        if !value.is_empty() {
                            out.push("=")?;
                            out.push(value)?;
                        }
                        Ok(())
                    };
                    if let Some(v) = &scalar_value {
                        named(&name, v)?;
                    } else if explode {
                        if matches!(shape, Shape::Array(_)) {
                            for v in &items {
                                named(&name, v)?;
                            }
                        } else {
                            for (k, v) in &properties {
                                named(k, v)?;
                            }
                        }
                    } else {
                        out.push(";")?;
                        out.push(&name)?;
                        out.push("=")?;
                        if matches!(shape, Shape::Array(_)) {
                            out.separated(items.iter().map(String::as_str), ",")?;
                        } else {
                            flat(&mut out, ",")?;
                        }
                    }
                }
                Style::Form | Style::Cookie => {
                    let separator = if style == Style::Cookie { "; " } else { "&" };
                    if let Some(v) = &scalar_value {
                        out.push(&name)?;
                        out.push("=")?;
                        out.push(v)?;
                    } else if explode {
                        if matches!(shape, Shape::Array(_)) {
                            for (i, v) in items.iter().enumerate() {
                                if i != 0 {
                                    out.push(separator)?;
                                }
                                out.push(&name)?;
                                out.push("=")?;
                                out.push(v)?;
                            }
                        } else {
                            pairs(&mut out, separator)?;
                        }
                    } else {
                        out.push(&name)?;
                        out.push("=")?;
                        if matches!(shape, Shape::Array(_)) {
                            out.separated(items.iter().map(String::as_str), ",")?;
                        } else {
                            flat(&mut out, ",")?;
                        }
                    }
                }
                Style::SpaceDelimited | Style::PipeDelimited => {
                    out.push(&name)?;
                    out.push("=")?;
                    let delimiter = if style == Style::SpaceDelimited {
                        "%20"
                    } else {
                        "%7C"
                    };
                    if matches!(shape, Shape::Array(_)) {
                        out.separated(items.iter().map(String::as_str), delimiter)?;
                    } else {
                        flat(&mut out, delimiter)?;
                    }
                }
                Style::DeepObject => {
                    for (i, (k, v)) in properties.iter().enumerate() {
                        if i != 0 {
                            out.push("&")?;
                        }
                        out.push(&name)?;
                        out.push("%5B")?;
                        out.push(k)?;
                        out.push("%5D=")?;
                        out.push(v)?;
                    }
                }
            }
        }
    }
    Ok(out.value)
}

/// Parse declared header serialization, then let the generated bound codec
/// enforce schema assertions. Repeated scalar fields (notably Set-Cookie) are
/// explicitly rejected; array/object list fields are combined with commas.
pub fn decode_header(
    operation: Source,
    p: &Parameter,
    headers: &Headers,
    json: JsonLimits,
) -> Result<Option<JsonValue>, SdkError> {
    let values = headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case(p.name))
        .map(|(_, bytes)| {
            std::str::from_utf8(bytes)
                .map(trim_ows)
                .map_err(|_| representation_error(operation, p.source, "typed header is not UTF-8"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if values.is_empty() {
        return if p.required {
            Err(representation_error(
                operation,
                p.source,
                "required response or part header is absent",
            ))
        } else {
            Ok(None)
        };
    }
    let repeated = matches!(
        p.serialization,
        Serialization::Style {
            shape: Shape::Array(_) | Shape::Object { .. },
            ..
        }
    ) && !p.name.eq_ignore_ascii_case("set-cookie");
    if values.len() > 1 && !repeated {
        return Err(representation_error(
            operation,
            p.source,
            "repeated scalar header has no unambiguous typed representation",
        ));
    }
    let text = values.join(",");
    decode_serialized(operation, p, &text, json).map(Some)
}

pub(super) fn decode_serialized(
    operation: Source,
    p: &Parameter,
    text: &str,
    json: JsonLimits,
) -> Result<JsonValue, SdkError> {
    let bad = || representation_error(operation, p.source, "invalid serialized composite value");
    match p.serialization {
        Serialization::Form { .. } => Err(representation_error(
            operation,
            p.source,
            "a querystring form is not a header representation",
        )),
        Serialization::Content { json: true, .. } => crate::parse_json(text, json)
            .map_err(|e| request_codec_error(operation, p.source, e.into())),
        Serialization::Content {
            json: false,
            scalar,
            ..
        } => scalar_value(operation, p.source, text, scalar),
        Serialization::Style {
            shape: Shape::Scalar(ty),
            ..
        } => scalar_value(operation, p.source, text, ty),
        Serialization::Style {
            shape: Shape::Array(ty),
            ..
        } => {
            if text.is_empty() {
                return Err(bad());
            }
            Ok(Nullable::Value(Value::Array(
                text.split(',')
                    .map(|v| scalar_value(operation, p.source, trim_ows(v), ty))
                    .collect::<Result<_, _>>()?,
            )))
        }
        Serialization::Style {
            shape:
                Shape::Object {
                    properties,
                    additional,
                },
            explode,
            ..
        } => {
            let mut object = BTreeMap::new();
            let parts = text.split(',').map(trim_ows).collect::<Vec<_>>();
            let mut add = |name: &str, value: &str| -> Result<(), SdkError> {
                let ty = properties
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, ty)| *ty)
                    .or(match additional {
                        AdditionalScalars::Typed(t) => Some(t),
                        AdditionalScalars::AnyScalar => Some(ScalarType::String),
                        _ => None,
                    })
                    .ok_or_else(bad)?;
                if object
                    .insert(
                        name.to_owned(),
                        scalar_value(operation, p.source, value, ty)?,
                    )
                    .is_some()
                {
                    return Err(bad());
                }
                Ok(())
            };
            if explode {
                for part in parts {
                    let (key, value) = part.split_once('=').ok_or_else(bad)?;
                    if value.contains('=') {
                        return Err(bad());
                    }
                    add(key, value)?;
                }
            } else {
                if parts.len() % 2 != 0 {
                    return Err(bad());
                }
                for pair in parts.chunks_exact(2) {
                    add(pair[0], pair[1])?;
                }
            }
            if object.is_empty() {
                return Err(bad());
            }
            Ok(Nullable::Value(Value::Object(object)))
        }
    }
}

fn trim_ows(value: &str) -> &str {
    value.trim_matches([' ', '\t'])
}

pub(super) fn percent_decode(
    operation: Source,
    source: Source,
    value: &str,
    form: bool,
) -> Result<String, SdkError> {
    let mut bytes = Vec::with_capacity(value.len());
    let raw = value.as_bytes();
    let mut at = 0;
    while at < raw.len() {
        match raw[at] {
            b'%' if at + 2 < raw.len() => {
                let a = char::from(raw[at + 1]).to_digit(16);
                let b = char::from(raw[at + 2]).to_digit(16);
                let (Some(a), Some(b)) = (a, b) else {
                    return Err(representation_error(
                        operation,
                        source,
                        "invalid percent escape",
                    ));
                };
                bytes.push(u8::try_from(a * 16 + b).expect("hex octet"));
                at += 3;
            }
            b'%' => {
                return Err(representation_error(
                    operation,
                    source,
                    "truncated percent escape",
                ));
            }
            b'+' if form => {
                bytes.push(b' ');
                at += 1;
            }
            b => {
                bytes.push(b);
                at += 1;
            }
        }
    }
    String::from_utf8(bytes)
        .map_err(|_| representation_error(operation, source, "percent-encoded text is not UTF-8"))
}
