use std::collections::BTreeMap;

use serde::Serialize;
use suspect_ir::contract::SourceId;

use super::planner::Planner;
use super::{
    Capability, MediaPlan, Method, OperationPlan, ResponseBodyDisposition, ResponsePlan,
    ResponseStatus,
};

/// RFC9110 media ranges: suffixes such as `+json` are concrete subtypes, not
/// invented suffix-wildcard syntax (`application/*+json` is declined).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum MediaRange {
    Any,
    Type { type_name: String },
    Concrete { type_name: String, subtype: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MediaType {
    pub(super) declared: String,
    pub(super) range: MediaRange,
    pub(super) parameters: BTreeMap<String, String>,
}
impl MediaType {
    #[must_use]
    pub fn declared(&self) -> &str {
        &self.declared
    }
    #[must_use]
    pub fn range(&self) -> &MediaRange {
        &self.range
    }
    #[must_use]
    pub fn parameters(&self) -> &BTreeMap<String, String> {
        &self.parameters
    }
    #[must_use]
    pub fn is_json(&self) -> bool {
        matches!(&self.range, MediaRange::Concrete { type_name, subtype } if (type_name == "application" && subtype == "json") || subtype.ends_with("+json"))
    }
    #[must_use]
    pub fn is_text(&self) -> bool {
        matches!(&self.range, MediaRange::Concrete { type_name, .. } | MediaRange::Type { type_name } if type_name == "text")
    }
    pub(super) fn essence(&self, name: &str) -> bool {
        match &self.range {
            MediaRange::Concrete { type_name, subtype } => name
                .split_once('/')
                .is_some_and(|(t, s)| t == type_name && s == subtype),
            _ => false,
        }
    }
    pub(super) fn specificity(&self) -> (u8, usize) {
        (
            match self.range {
                MediaRange::Concrete { .. } => 2,
                MediaRange::Type { .. } => 1,
                MediaRange::Any => 0,
            },
            self.parameters.len(),
        )
    }
    pub(super) fn matches(&self, actual: &Self) -> bool {
        let MediaRange::Concrete { type_name, subtype } = &actual.range else {
            return false;
        };
        let essence = match &self.range {
            MediaRange::Any => true,
            MediaRange::Type { type_name: t } => t == type_name,
            MediaRange::Concrete {
                type_name: t,
                subtype: s,
            } => t == type_name && s == subtype,
        };
        essence
            && self.parameters.iter().all(|(key, value)| {
                actual
                    .parameters
                    .get(key)
                    .is_some_and(|v| parameter_eq(key, value, v))
            })
    }
}

/// Matching never guesses a Content-Type, falls through to a less specific
/// status because of a media mismatch, or classifies `default` as an error.
#[derive(Debug, Clone, Copy)]
pub struct ResponseMatch<'a> {
    response: &'a ResponsePlan,
    media: Option<&'a MediaPlan>,
    status: u16,
    disposition: ResponseBodyDisposition,
}
impl<'a> ResponseMatch<'a> {
    #[must_use]
    pub fn response(&self) -> &'a ResponsePlan {
        self.response
    }
    #[must_use]
    pub fn media(&self) -> Option<&'a MediaPlan> {
        self.media
    }
    #[must_use]
    pub fn status(&self) -> u16 {
        self.status
    }
    #[must_use]
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
    #[must_use]
    pub fn body_disposition(&self) -> ResponseBodyDisposition {
        self.disposition
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseMatchError {
    InvalidStatus(u16),
    UndeclaredStatus(u16),
    MissingContentType,
    InvalidContentType(String),
    UndeclaredMediaType(String),
    UnsupportedCharset(String),
}
impl std::fmt::Display for ResponseMatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ResponseMatchError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaMatchError {
    InvalidContentType(String),
    UndeclaredMediaType(String),
    UnsupportedCharset(String),
}
impl std::fmt::Display for MediaMatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for MediaMatchError {}
impl From<MediaMatchError> for ResponseMatchError {
    fn from(error: MediaMatchError) -> Self {
        match error {
            MediaMatchError::InvalidContentType(value) => Self::InvalidContentType(value),
            MediaMatchError::UndeclaredMediaType(value) => Self::UndeclaredMediaType(value),
            MediaMatchError::UnsupportedCharset(value) => Self::UnsupportedCharset(value),
        }
    }
}

impl super::BodyPlan {
    /// Select the declaration applicable to a caller's concrete Content-Type.
    /// A wildcard declaration cannot bypass a more specific request schema.
    /// The value must then be encoded using this returned representation.
    pub fn match_media(&self, content_type: &str) -> Result<&MediaPlan, MediaMatchError> {
        select_media(&self.media, content_type)
    }
}

impl OperationPlan {
    /// Exact status > class range > default. Media precedence is concrete over
    /// type range over `*/*`, then parameter count. Equally specific overlapping
    /// declarations are rejected at planning time, independent of map order.
    pub fn match_response(
        &self,
        status: u16,
        content_type: Option<&str>,
    ) -> Result<ResponseMatch<'_>, ResponseMatchError> {
        if !(100..600).contains(&status) {
            return Err(ResponseMatchError::InvalidStatus(status));
        }
        let response = self
            .responses
            .iter()
            .filter_map(|r| {
                let precedence = match r.status {
                    ResponseStatus::Exact(s) if s == status => 3,
                    ResponseStatus::Range(c) if u16::from(c) == status / 100 => 2,
                    ResponseStatus::Default => 1,
                    _ => return None,
                };
                Some((precedence, r))
            })
            .max_by_key(|(rank, _)| *rank)
            .map(|(_, r)| r)
            .ok_or(ResponseMatchError::UndeclaredStatus(status))?;
        let forbidden = self.method == Method::Head
            || (100..200).contains(&status)
            || matches!(status, 204 | 205 | 304);
        if forbidden || response.media.is_empty() {
            return Ok(ResponseMatch {
                response,
                media: None,
                status,
                disposition: if forbidden {
                    ResponseBodyDisposition::ForbiddenByHttp
                } else {
                    ResponseBodyDisposition::UndeclaredBoundedBytes
                },
            });
        }
        let content_type = content_type.ok_or(ResponseMatchError::MissingContentType)?;
        let media = select_media(&response.media, content_type)?;
        Ok(ResponseMatch {
            response,
            media: Some(media),
            status,
            disposition: ResponseBodyDisposition::Declared,
        })
    }
}

fn select_media<'a>(
    media: &'a [MediaPlan],
    content_type: &str,
) -> Result<&'a MediaPlan, MediaMatchError> {
    let actual = parse(content_type, false)
        .map_err(|_| MediaMatchError::InvalidContentType(content_type.to_owned()))?;
    let media = media
        .iter()
        .filter(|m| m.media_type.matches(&actual))
        .max_by_key(|m| m.media_type.specificity())
        .ok_or_else(|| MediaMatchError::UndeclaredMediaType(content_type.to_owned()))?;
    if matches!(
        &media.representation,
        super::Representation::Text { .. }
            | super::Representation::Stream { .. }
            | super::Representation::Form { .. }
    ) && actual
        .parameters
        .get("charset")
        .is_some_and(|v| !v.eq_ignore_ascii_case("utf-8"))
    {
        return Err(MediaMatchError::UnsupportedCharset(
            actual.parameters["charset"].clone(),
        ));
    }
    Ok(media)
}

pub(super) fn planned(p: &mut Planner<'_>, source: &SourceId, name: &str) -> Option<MediaType> {
    match parse(name, true) {
        Ok(media) => {
            if !matches!(media.range, MediaRange::Concrete { .. }) {
                p.require(Capability::MediaRanges, source);
            }
            if !media.parameters.is_empty() {
                p.require(Capability::MediaTypeParameters, source);
            }
            if media.is_json() && !media.essence("application/json") {
                p.require(Capability::StructuredJsonMedia, source);
            }
            Some(media)
        }
        Err(reason) => {
            p.error(source, "http-media-type-invalid", reason);
            None
        }
    }
}

pub(super) fn validate_map(p: &mut Planner<'_>, entries: &[MediaPlan]) {
    for (i, left) in entries.iter().enumerate() {
        for right in &entries[i + 1..] {
            if left.media_type.specificity() == right.media_type.specificity()
                && overlap(&left.media_type, &right.media_type)
            {
                p.error(&right.source.use_site.source, "http-media-type-ambiguous", format!("equally specific media declarations {:?} and {:?} can match the same Content-Type", left.media_type.declared, right.media_type.declared));
            }
        }
    }
}

fn overlap(a: &MediaType, b: &MediaType) -> bool {
    if a.range != b.range {
        return false;
    }
    a.parameters.iter().all(|(k, v)| {
        b.parameters
            .get(k)
            .is_none_or(|other| parameter_eq(k, v, other))
    })
}
fn parameter_eq(key: &str, a: &str, b: &str) -> bool {
    if key == "charset" {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

pub(super) fn token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// Parse without silently discarding malformed parameters, duplicates, trailing
/// separators, quoted commas, or invalid wildcard combinations.
pub(super) fn parse(value: &str, ranges: bool) -> Result<MediaType, &'static str> {
    if value.is_empty() || value.bytes().any(|b| b < 0x20 && b != b'\t' || b == 0x7f) {
        return Err("media type contains invalid control characters");
    }
    let pieces = split_quoted(value, ';')?;
    let (type_name, subtype) = pieces[0]
        .trim()
        .split_once('/')
        .ok_or("media type requires type/subtype")?;
    if !token(type_name) || !token(subtype) {
        return Err("invalid media type token");
    }
    let type_name = type_name.to_ascii_lowercase();
    let subtype = subtype.to_ascii_lowercase();
    let range = match (type_name.as_str(), subtype.as_str()) {
        ("*", "*") if ranges => MediaRange::Any,
        (_, "*") if ranges && !type_name.contains('*') => MediaRange::Type { type_name },
        _ if type_name.contains('*') || subtype.contains('*') => {
            return Err("only */* and type/* are valid wildcard media ranges");
        }
        _ => MediaRange::Concrete { type_name, subtype },
    };
    let mut parameters = BTreeMap::new();
    for piece in &pieces[1..] {
        let (key, raw) = piece
            .trim()
            .split_once('=')
            .ok_or("media parameters require name=value")?;
        if !token(key) {
            return Err("invalid media parameter name");
        }
        let decoded = if raw.starts_with('"') {
            if !raw.ends_with('"') || raw.len() < 2 {
                return Err("unterminated quoted media parameter");
            }
            let mut out = String::new();
            let mut escape = false;
            for c in raw[1..raw.len() - 1].chars() {
                if escape {
                    out.push(c);
                    escape = false;
                } else if c == '\\' {
                    escape = true;
                } else if c == '"' {
                    return Err("unescaped quote in media parameter");
                } else {
                    out.push(c);
                }
            }
            if escape {
                return Err("unterminated quoted pair in media parameter");
            }
            out
        } else {
            if !token(raw) {
                return Err("invalid media parameter value");
            }
            raw.to_owned()
        };
        if parameters
            .insert(key.to_ascii_lowercase(), decoded)
            .is_some()
        {
            return Err("duplicate case-insensitive media parameter");
        }
    }
    Ok(MediaType {
        declared: value.to_owned(),
        range,
        parameters,
    })
}

pub(super) fn split_quoted(value: &str, delimiter: char) -> Result<Vec<&str>, &'static str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut escape = false;
    for (i, c) in value.char_indices() {
        if escape {
            escape = false;
        } else if quoted && c == '\\' {
            escape = true;
        } else if c == '"' {
            quoted = !quoted;
        } else if !quoted && c == delimiter {
            result.push(&value[start..i]);
            start = i + c.len_utf8();
        }
    }
    if quoted || escape {
        return Err("unterminated quoted string");
    }
    result.push(&value[start..]);
    Ok(result)
}
