use super::{Headers, Operation, Selection, Source};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaRange {
    Any,
    Type(&'static str),
    Concrete(&'static str, &'static str),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Json,
    Text,
    Binary,
    Form,
    Multipart,
    Stream,
}
#[derive(Debug, Clone, Copy)]
pub struct Media {
    pub source: Source,
    pub declared: &'static str,
    pub range: MediaRange,
    pub parameters: &'static [(&'static str, &'static str)],
    pub kind: MediaKind,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Exact(u16),
    Range(u8),
    Default,
}
#[derive(Debug, Clone, Copy)]
pub struct ResponseSpec {
    pub source: Source,
    pub status: Status,
    pub media: &'static [Media],
    pub links: &'static [super::Link],
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedMedia {
    pub type_name: String,
    pub subtype: String,
    pub parameters: BTreeMap<String, String>,
}
impl ParsedMedia {
    /// Parse a concrete RFC9110 media type, retaining distinct parameter values.
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        if value.is_empty() || value.bytes().any(|b| (b < 32 && b != b'\t') || b == 127) {
            return Err("invalid media control character");
        }
        let parts = split_quoted(value, ';')?;
        let (ty, subtype) = parts[0]
            .trim()
            .split_once('/')
            .ok_or("media type needs type/subtype")?;
        if !is_token(ty) || !is_token(subtype) || ty.contains('*') || subtype.contains('*') {
            return Err("invalid concrete media token");
        }
        let mut parameters = BTreeMap::new();
        for part in &parts[1..] {
            let (key, value) = part
                .trim()
                .split_once('=')
                .ok_or("media parameter needs name=value")?;
            if !is_token(key) {
                return Err("invalid media parameter name");
            }
            let value = if value.starts_with('"') {
                unquote(value)?
            } else {
                if !is_token(value) {
                    return Err("invalid media parameter value");
                }
                value.into()
            };
            if parameters.insert(key.to_ascii_lowercase(), value).is_some() {
                return Err("duplicate media parameter");
            }
        }
        Ok(Self {
            type_name: ty.to_ascii_lowercase(),
            subtype: subtype.to_ascii_lowercase(),
            parameters,
        })
    }
}
pub(super) fn is_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}
pub(super) fn unquote(raw: &str) -> Result<String, &'static str> {
    if !raw.starts_with('"') || !raw.ends_with('"') || raw.len() < 2 {
        return Err("unterminated quoted value");
    }
    let mut result = String::new();
    let mut escape = false;
    for c in raw[1..raw.len() - 1].chars() {
        if escape {
            result.push(c);
            escape = false;
        } else if c == '\\' {
            escape = true;
        } else if c == '"' || c.is_control() && c != '\t' {
            return Err("invalid quoted value");
        } else {
            result.push(c);
        }
    }
    if escape {
        return Err("unterminated quoted pair");
    }
    Ok(result)
}
pub(super) fn split_quoted(value: &str, delimiter: char) -> Result<Vec<&str>, &'static str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut escape = false;
    for (at, c) in value.char_indices() {
        if escape {
            escape = false;
        } else if quoted && c == '\\' {
            escape = true;
        } else if c == '"' {
            quoted = !quoted;
        } else if !quoted && c == delimiter {
            result.push(&value[start..at]);
            start = at + c.len_utf8();
        }
    }
    if quoted || escape {
        return Err("unterminated quoted string");
    }
    result.push(&value[start..]);
    Ok(result)
}
pub(super) fn content_type(headers: &Headers) -> Option<String> {
    let mut values = headers
        .iter()
        .filter(|(n, _)| n.eq_ignore_ascii_case("content-type"));
    let (_, value) = values.next()?;
    if values.next().is_some() {
        return None;
    }
    let value = std::str::from_utf8(value).ok()?;
    ParsedMedia::parse(value).ok()?;
    Some(value.into())
}
pub(super) fn matches(media: &Media, actual: &ParsedMedia) -> bool {
    let matched = match media.range {
        MediaRange::Any => true,
        MediaRange::Type(t) => actual.type_name == t,
        MediaRange::Concrete(t, s) => actual.type_name == t && actual.subtype == s,
    };
    matched
        && media.parameters.iter().all(|(k, v)| {
            actual.parameters.get(*k).is_some_and(|actual| {
                if *k == "charset" {
                    actual.eq_ignore_ascii_case(v)
                } else {
                    actual == v
                }
            })
        })
}
pub(super) fn select(media: &[Media], content_type: &str) -> Result<usize, &'static str> {
    let actual = ParsedMedia::parse(content_type)?;
    let index = media
        .iter()
        .enumerate()
        .filter(|(_, m)| matches(m, &actual))
        .max_by_key(|(_, m)| {
            (
                match m.range {
                    MediaRange::Any => 0,
                    MediaRange::Type(_) => 1,
                    MediaRange::Concrete(_, _) => 2,
                },
                m.parameters.len(),
            )
        })
        .map(|(i, _)| i)
        .ok_or("undeclared media type")?;
    if matches!(
        media[index].kind,
        MediaKind::Text | MediaKind::Form | MediaKind::Stream
    ) && actual
        .parameters
        .get("charset")
        .is_some_and(|v| !v.eq_ignore_ascii_case("utf-8"))
    {
        return Err("unsupported charset");
    }
    Ok(index)
}
pub(super) fn forbidden(method: &str, status: u16) -> bool {
    method == "HEAD" || (100..200).contains(&status) || matches!(status, 204 | 205 | 304)
}
pub(super) fn match_response(
    op: &Operation,
    status: u16,
    content_type: Option<&str>,
) -> Result<Selection, Source> {
    if !(100..600).contains(&status) {
        return Err(op.source);
    }
    let response = op
        .responses
        .iter()
        .enumerate()
        .filter_map(|(i, r)| match r.status {
            Status::Exact(s) if s == status => Some((3, i)),
            Status::Range(c) if u16::from(c) == status / 100 => Some((2, i)),
            Status::Default => Some((1, i)),
            _ => None,
        })
        .max_by_key(|(rank, _)| *rank)
        .map(|(_, i)| i)
        .ok_or(op.source)?;
    let r = &op.responses[response];
    let forbidden = forbidden(op.method, status);
    let media = if forbidden || r.media.is_empty() {
        None
    } else {
        Some(select(r.media, content_type.ok_or(r.source)?).map_err(|_| r.source)?)
    };
    Ok(Selection {
        response,
        media,
        forbidden,
    })
}
