use serde_json::Value;

use super::{
    AdditionalParts, AdditionalScalars, FormPlan, HeaderPlan, ParameterLocation, ParameterPlan,
    ParameterSerialization, PartMultiplicity, PartPlan, PartRepresentation, PercentEncoding,
    Representation, ScalarType, SourceLocation, Style, WireShape,
};

/// A located value/transport refusal after a declaration has been planned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireError {
    source: SourceLocation,
    code: &'static str,
    message: String,
}
impl WireError {
    pub(super) fn new(
        source: &SourceLocation,
        code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            source: source.clone(),
            code,
            message: message.into(),
        }
    }
    #[must_use]
    pub fn source(&self) -> &SourceLocation {
        &self.source
    }
    #[must_use]
    pub fn code(&self) -> &'static str {
        self.code
    }
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}
impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for WireError {}

/// The wire value of a single parameter. Query/cookie serializations include
/// names; header serializations do not. No leading `?` or `&` is included.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerializedParameter {
    pub(super) value: String,
}
impl SerializedParameter {
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

impl ParameterPlan {
    /// Interpret the planned serialization after the bound native codec has
    /// validated the value. This enforces wire shape/escaping, not all schema
    /// assertions. It is also the public normative witness for native adapters.
    pub fn serialize(&self, value: &Value) -> Result<SerializedParameter, WireError> {
        if let Some(media) = &self.content_media
            && let Representation::Form { form } = &media.representation
        {
            return serialize_form(form, value).map(|value| SerializedParameter { value });
        }
        serialize(
            &self.source.terminal,
            &self.name,
            self.location,
            &self.serialization,
            value,
        )
    }
}

impl HeaderPlan {
    /// Serialize one typed header value without URI encoding or automatic
    /// quoting. Repeated HTTP fields and header-specific parsing remain native
    /// transport responsibilities; the name is not part of this value.
    pub fn serialize(&self, value: &Value) -> Result<SerializedParameter, WireError> {
        serialize(
            &self.source.terminal,
            &self.name,
            ParameterLocation::Header,
            &self.serialization,
            value,
        )
    }
}

fn serialize(
    at: &SourceLocation,
    name: &str,
    location: ParameterLocation,
    serialization: &ParameterSerialization,
    value: &Value,
) -> Result<SerializedParameter, WireError> {
    let value = match serialization {
        ParameterSerialization::Content {
            media_type,
            percent_encoding,
        } => {
            let text = if media_type.is_json() {
                value.to_string()
            } else {
                scalar_text(value, None).ok_or_else(|| {
                    WireError::new(
                        at,
                        "http-wire-value",
                        "text content requires a non-null scalar",
                    )
                })?
            };
            let text = encode_value(at, &text, *percent_encoding, location, None)?;
            if matches!(
                location,
                ParameterLocation::Query | ParameterLocation::Cookie
            ) {
                format!(
                    "{}={text}",
                    percent_encode(name, PercentEncoding::UriComponent)
                )
            } else {
                text
            }
        }
        ParameterSerialization::Style {
            style,
            explode,
            shape,
            percent_encoding,
        } => {
            let name = percent_encode(
                name,
                if location == ParameterLocation::Header || *style == Style::Cookie {
                    PercentEncoding::None
                } else {
                    PercentEncoding::UriComponent
                },
            );
            let (scalar, items, properties) = values(at, value, shape)?;
            let encode =
                |s: &str| encode_value(at, s, *percent_encoding, location, Some((*style, shape)));
            let scalar = scalar.as_deref().map(encode).transpose()?;
            let items: Vec<_> = items.iter().map(|s| encode(s)).collect::<Result<_, _>>()?;
            let properties: Vec<_> = properties
                .iter()
                .map(|(k, v)| Ok((encode(k)?, encode(v)?)))
                .collect::<Result<_, WireError>>()?;
            let flatten = |delimiter: &str| {
                properties
                    .iter()
                    .flat_map(|(k, v)| [k.as_str(), v.as_str()])
                    .collect::<Vec<_>>()
                    .join(delimiter)
            };
            let pairs = |delimiter: &str| {
                properties
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join(delimiter)
            };
            let named = |n: &str, v: &str, matrix: bool| {
                if matrix && v.is_empty() {
                    n.to_owned()
                } else {
                    format!("{n}={v}")
                }
            };
            match style {
                Style::Simple => scalar.unwrap_or_else(|| {
                    if matches!(shape, WireShape::Array { .. }) {
                        items.join(",")
                    } else if *explode {
                        pairs(",")
                    } else {
                        flatten(",")
                    }
                }),
                Style::Label => format!(
                    ".{}",
                    scalar.unwrap_or_else(|| if matches!(shape, WireShape::Array { .. }) {
                        items.join(if *explode { "." } else { "," })
                    } else if *explode {
                        pairs(".")
                    } else {
                        flatten(",")
                    })
                ),
                Style::Matrix => {
                    if let Some(scalar) = scalar {
                        format!(";{}", named(&name, &scalar, true))
                    } else if matches!(shape, WireShape::Array { .. }) {
                        if *explode {
                            items
                                .iter()
                                .map(|item| format!(";{}", named(&name, item, true)))
                                .collect()
                        } else {
                            format!(";{name}={}", items.join(","))
                        }
                    } else if *explode {
                        properties
                            .iter()
                            .map(|(key, value)| format!(";{}", named(key, value, true)))
                            .collect()
                    } else {
                        format!(";{name}={}", flatten(","))
                    }
                }
                Style::Form | Style::Cookie => {
                    let delimiter = if *style == Style::Cookie { "; " } else { "&" };
                    if let Some(scalar) = scalar {
                        format!("{name}={scalar}")
                    } else if matches!(shape, WireShape::Array { .. }) {
                        if *explode {
                            items
                                .iter()
                                .map(|item| format!("{name}={item}"))
                                .collect::<Vec<_>>()
                                .join(delimiter)
                        } else {
                            format!("{name}={}", items.join(","))
                        }
                    } else if *explode {
                        pairs(delimiter)
                    } else {
                        format!("{name}={}", flatten(","))
                    }
                }
                Style::SpaceDelimited | Style::PipeDelimited => {
                    let delimiter = if *style == Style::SpaceDelimited {
                        "%20"
                    } else {
                        "%7C"
                    };
                    format!(
                        "{name}={}",
                        if matches!(shape, WireShape::Array { .. }) {
                            items.join(delimiter)
                        } else {
                            flatten(delimiter)
                        }
                    )
                }
                Style::DeepObject => properties
                    .iter()
                    .map(|(key, value)| format!("{name}%5B{key}%5D={value}"))
                    .collect::<Vec<_>>()
                    .join("&"),
            }
        }
    };
    Ok(SerializedParameter { value })
}

type Values = (Option<String>, Vec<String>, Vec<(String, String)>);

fn values(at: &SourceLocation, value: &Value, shape: &WireShape) -> Result<Values, WireError> {
    let error = || {
        WireError::new(
            at,
            "http-wire-value",
            "value must match the planned non-null scalar/flat wire shape; validate schema with the bound codec before serialization",
        )
    };
    match shape {
        WireShape::Scalar { scalar } => Ok((
            Some(scalar_text(value, Some(*scalar)).ok_or_else(error)?),
            Vec::new(),
            Vec::new(),
        )),
        WireShape::Array { items } => {
            let array = value.as_array().ok_or_else(error)?;
            if array.is_empty() {
                return Err(WireError::new(
                    at,
                    "http-empty-composite",
                    "empty composites have no value expansion; omit an optional parameter rather than guessing a required empty representation",
                ));
            }
            let values = array
                .iter()
                .map(|v| scalar_text(v, Some(*items)).ok_or_else(error))
                .collect::<Result<_, _>>()?;
            Ok((None, values, Vec::new()))
        }
        WireShape::FlatObject {
            properties,
            additional,
        } => {
            let object = value.as_object().ok_or_else(error)?;
            if object.is_empty() {
                return Err(WireError::new(
                    at,
                    "http-empty-composite",
                    "empty composites have no value expansion; omit an optional parameter rather than guessing a required empty representation",
                ));
            }
            let mut values = Vec::new();
            for (key, value) in object {
                let scalar = if let Some(scalar) = properties.get(key) {
                    Some(*scalar)
                } else {
                    match additional {
                        AdditionalScalars::Forbidden => return Err(error()),
                        AdditionalScalars::Typed(scalar) => Some(*scalar),
                        AdditionalScalars::AnyScalar => None,
                    }
                };
                values.push((key.clone(), scalar_text(value, scalar).ok_or_else(error)?));
            }
            // OpenAPI/RFC6570 do not impose object order. The reference plan's
            // stable policy is Unicode scalar/UTF-8 lexicographic key order.
            values.sort_by(|(a, _), (b, _)| a.cmp(b));
            Ok((None, Vec::new(), values))
        }
    }
}

fn scalar_text(value: &Value, scalar: Option<ScalarType>) -> Option<String> {
    match value {
        Value::String(s) if scalar.is_none_or(|v| v == ScalarType::String) => Some(s.clone()),
        Value::Bool(b) if scalar.is_none_or(|v| v == ScalarType::Boolean) => Some(b.to_string()),
        Value::Number(n)
            if scalar.is_none_or(|v| matches!(v, ScalarType::Integer | ScalarType::Number)) =>
        {
            let text = n.to_string();
            if scalar == Some(ScalarType::Integer) && !integral(&text) {
                None
            } else {
                Some(text)
            }
        }
        _ => None,
    }
}

// The input is already a JSON number. Decide integrality using its written
// coefficient/exponent without f64 rounding or exponent-sized allocation.
fn integral(text: &str) -> bool {
    let (mantissa, exponent) = text.split_once(['e', 'E']).unwrap_or((text, "0"));
    let digits: Vec<_> = mantissa.bytes().filter(u8::is_ascii_digit).collect();
    if digits.iter().all(|b| *b == b'0') {
        return true;
    }
    let fraction = mantissa.split_once('.').map_or(0, |(_, f)| f.len());
    let trailing_zeroes = digits.iter().rev().take_while(|b| **b == b'0').count();
    match exponent.parse::<i128>() {
        Ok(exponent) => exponent >= fraction as i128 - trailing_zeroes as i128,
        Err(_) => !exponent.starts_with('-'),
    }
}

fn encode_value(
    at: &SourceLocation,
    value: &str,
    encoding: PercentEncoding,
    location: ParameterLocation,
    style: Option<(Style, &WireShape)>,
) -> Result<String, WireError> {
    if encoding == PercentEncoding::None
        && value
            .chars()
            .any(|c| c.is_control() && !(location == ParameterLocation::Header && c == '\t'))
    {
        return Err(WireError::new(
            at,
            "http-wire-control",
            "header/cookie/part values must not inject control characters",
        ));
    }
    if location == ParameterLocation::Cookie
        && encoding == PercentEncoding::None
        && value
            .bytes()
            .any(|b| matches!(b, b' ' | b'\t' | b'"' | b',' | b';' | b'\\') || b > 0x7e)
    {
        return Err(WireError::new(
            at,
            "http-cookie-escaping",
            "cookie data requiring escaping must be supplied in an already escaped form",
        ));
    }
    if let Some((style, _)) = style {
        let ambiguous = match style {
            Style::SpaceDelimited => value.contains(' '),
            Style::PipeDelimited => value.contains('|'),
            Style::DeepObject => value.contains(['[', ']']),
            _ => false,
        };
        if ambiguous {
            return Err(WireError::new(
                at,
                "http-delimiter-escaping",
                "data contains a percent-encoded style delimiter; the API must define additional escaping before OpenAPI serialization",
            ));
        }
    }
    if encoding == PercentEncoding::ReservedExpansion {
        let uri_hazard = match location {
            ParameterLocation::Path => value.contains(['#', '[', ']', '/', '?']),
            ParameterLocation::Query | ParameterLocation::Querystring => {
                value.contains(['#', '[', ']', '&', '=', '+'])
            }
            ParameterLocation::Cookie => value.contains([';', ',']),
            ParameterLocation::Header => false,
        };
        let delimiter_hazard = style.is_some_and(|(style, shape)| {
            !matches!(shape, WireShape::Scalar { .. })
                && match style {
                    Style::Simple | Style::Form | Style::Cookie => value.contains(','),
                    Style::Label => value.contains(['.', ',']),
                    Style::Matrix => value.contains([';', ',']),
                    _ => false,
                }
        });
        if uri_hazard || delimiter_hazard {
            return Err(WireError::new(
                at,
                "http-reserved-value-escaping",
                "allowReserved callers must pre-escape illegal URI/form characters and active data delimiters; the planner never silently emits an ambiguous query",
            ));
        }
    }
    Ok(percent_encode(value, encoding))
}

fn percent_encode(value: &str, encoding: PercentEncoding) -> String {
    if encoding == PercentEncoding::None {
        return value.to_owned();
    }
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::new();
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        let unreserved = byte.is_ascii_alphanumeric() || b"-._~".contains(&byte);
        let reserved = b":/?#[]@!$&'()*+,;=".contains(&byte);
        if encoding == PercentEncoding::ReservedExpansion
            && byte == b'%'
            && bytes.get(index + 1).is_some_and(u8::is_ascii_hexdigit)
            && bytes.get(index + 2).is_some_and(u8::is_ascii_hexdigit)
        {
            out.push('%');
            out.push(char::from(bytes[index + 1]));
            out.push(char::from(bytes[index + 2]));
            index += 3;
            continue;
        }
        let pass = if encoding == PercentEncoding::FormUrlEncoded {
            byte.is_ascii_alphanumeric() || b"*-._".contains(&byte)
        } else {
            unreserved || encoding == PercentEncoding::ReservedExpansion && reserved
        };
        if pass {
            out.push(char::from(byte));
        } else if encoding == PercentEncoding::FormUrlEncoded && byte == b' ' {
            out.push('+');
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 15)]));
        }
        index += 1;
    }
    out
}

fn serialize_form(form: &FormPlan, value: &Value) -> Result<String, WireError> {
    let source = &form.rules.schema.source.terminal;
    let object = value.as_object().ok_or_else(|| {
        WireError::new(source, "http-form-value", "form content requires an object")
    })?;
    for name in &form.rules.required {
        if !object.contains_key(&name.value) {
            return Err(WireError::new(
                &name.source,
                "http-form-required",
                format!("missing required form field {:?}", name.value),
            ));
        }
    }
    let count = object.len() as u64;
    if form
        .rules
        .min_properties
        .as_ref()
        .is_some_and(|n| count < n.value)
        || form
            .rules
            .max_properties
            .as_ref()
            .is_some_and(|n| count > n.value)
    {
        return Err(WireError::new(
            source,
            "http-form-cardinality",
            "form property count is outside its declared bounds",
        ));
    }
    // Field order is an explicit deterministic implementation choice; OpenAPI
    // leaves it unspecified. Array item order is the source value's order.
    let mut fields: Vec<_> = object.iter().collect();
    fields.sort_by_key(|(name, _)| *name);
    let mut output = Vec::new();
    for (name, value) in fields {
        let part = match form.fields.iter().find(|p| p.name.as_deref() == Some(name)) {
            Some(part) => part,
            None => match &form.additional {
                AdditionalParts::Allowed(part) => part,
                AdditionalParts::Forbidden => {
                    return Err(WireError::new(
                        source,
                        "http-form-extra",
                        format!("undeclared form field {name:?}"),
                    ));
                }
            },
        };
        if part.multiplicity == PartMultiplicity::RepeatedArrayItems {
            let values = value.as_array().ok_or_else(|| {
                WireError::new(
                    &part.source.terminal,
                    "http-form-array",
                    "repeated form field requires an array",
                )
            })?;
            let count = values.len() as u64;
            if part.min_items.as_ref().is_some_and(|n| count < n.value)
                || part.max_items.as_ref().is_some_and(|n| count > n.value)
                || part.required && values.is_empty()
            {
                return Err(WireError::new(
                    &part.source.terminal,
                    "http-form-cardinality",
                    "repeated field cannot satisfy its required/cardinality declaration",
                ));
            }
            for value in values {
                output.push(form_field(part, name, value)?);
            }
        } else {
            output.push(form_field(part, name, value)?);
        }
    }
    Ok(output.join("&"))
}

fn form_field(part: &PartPlan, name: &str, value: &Value) -> Result<String, WireError> {
    let source = &part.source.terminal;
    let (text, encoding) = match &part.representation {
        PartRepresentation::Json { outer_encoding, .. } => (value.to_string(), *outer_encoding),
        PartRepresentation::Text {
            scalar,
            outer_encoding,
            ..
        } => (
            scalar_text(value, Some(*scalar)).ok_or_else(|| {
                WireError::new(
                    source,
                    "http-form-scalar",
                    "form field must match its scalar codec type",
                )
            })?,
            *outer_encoding,
        ),
        PartRepresentation::Style { serialization, .. } => {
            return serialize(source, name, ParameterLocation::Query, serialization, value)
                .map(|v| v.value);
        }
        PartRepresentation::Binary { .. } => {
            return Err(WireError::new(
                source,
                "http-form-binary",
                "raw bytes have no form-urlencoded text representation",
            ));
        }
    };
    Ok(format!(
        "{}={}",
        percent_encode(name, PercentEncoding::FormUrlEncoded),
        percent_encode(&text, encoding)
    ))
}
