//! Rust leaf-function implementations — the conformance *reference* for the
//! TS mirrors in `rules-runtime/src/functions.ts`. Both run
//! `conformance/cases.json`; identical outputs are a CI gate.
//!
//! These are the same semantics suspect-lint's spectral functions expose;
//! they live here so the rule runtime's contract does not depend on the
//! lint engine's internals.

/// Word-splitting shared by every casing check: camelCase boundaries,
/// separators (`-`, `_`, `.`, spaces), runs of capitals.
fn words(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0usize;
    let mut prev_lower_or_digit = false;
    let mut prev_upper = false;
    for i in 0..bytes.len() {
        let c = bytes[i];
        let is_upper = c.is_ascii_uppercase();
        let is_lower_or_digit = c.is_ascii_lowercase() || c.is_ascii_digit();
        let boundary = (is_upper && prev_lower_or_digit)
            || (is_upper && prev_upper && i + 1 < bytes.len() && bytes[i + 1].is_ascii_lowercase());
        let is_sep = !(c.is_ascii_alphanumeric());
        if boundary {
            out.push(&s[start..i]);
            start = i;
        } else if is_sep {
            if i > start {
                out.push(&s[start..i]);
            }
            start = i + 1;
        }
        prev_lower_or_digit = is_lower_or_digit;
        prev_upper = is_upper;
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

/// Casing styles shared with the TS mirrors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Casing {
    /// `operationId`
    Camel,
    /// `PetStore`
    Pascal,
    /// `pet-store`
    Kebab,
    /// `pet_store`
    Snake,
    /// `PET_STORE`
    Macro,
    /// `PET-STORE`
    Cobol,
    /// `pet.store`
    Dot,
}

impl Casing {
    /// Parses a style name as the TS SDK spells it.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "camel" => Some(Self::Camel),
            "pascal" => Some(Self::Pascal),
            "kebab" => Some(Self::Kebab),
            "snake" => Some(Self::Snake),
            "macro" => Some(Self::Macro),
            "cobol" => Some(Self::Cobol),
            "dot" => Some(Self::Dot),
            _ => None,
        }
    }
}

/// `casing(s, style)`; mirrors `functions.ts::casing`.
#[must_use]
pub fn casing(s: &str, style: Casing) -> bool {
    let ws = words(s);
    if ws.is_empty() {
        return true;
    }
    let joined: String = ws.concat();
    match style {
        Casing::Camel => {
            joined == s
                && s.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                && s.chars().all(|c| c.is_ascii_alphanumeric())
        }
        Casing::Pascal => {
            joined == s
                && s.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                && s.chars().all(|c| c.is_ascii_alphanumeric())
        }
        Casing::Kebab => {
            joined == s.replace('-', "")
                && s.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        }
        Casing::Snake => {
            joined == s.replace('_', "")
                && s.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        }
        Casing::Macro => {
            joined == s.replace('_', "")
                && s.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        }
        Casing::Cobol => {
            joined == s.replace('-', "")
                && s.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-')
        }
        Casing::Dot => {
            joined == s.replace('.', "")
                && s.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.')
        }
    }
}

/// `defined(v)`: present and not null; mirrors `functions.ts::defined`.
#[must_use]
pub fn defined(v: &serde_json::Value) -> bool {
    !v.is_null()
}

/// `truthy(v)`: JS truthiness; mirrors `functions.ts::truthy`.
#[must_use]
pub fn truthy(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(b) => *b,
        serde_json::Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        serde_json::Value::String(s) => !s.is_empty(),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => true,
    }
}

/// `lengthBetween(v, min, max)`; mirrors `functions.ts::lengthBetween`.
#[must_use]
pub fn length_between(v: &serde_json::Value, min: usize, max: usize) -> bool {
    match v {
        serde_json::Value::String(s) => s.chars().count() >= min && s.chars().count() <= max,
        serde_json::Value::Array(a) => a.len() >= min && a.len() <= max,
        _ => false,
    }
}

/// `matches(v, pattern)`; mirrors `functions.ts::matches`. Invalid patterns
/// return `false` on both sides.
#[must_use]
pub fn matches(v: &serde_json::Value, pattern: &str) -> bool {
    let Some(s) = v.as_str() else { return false };
    let Ok(re) = regex_lite(pattern) else {
        return false;
    };
    re.is_match(s)
}

// Minimal regex engine is overkill: suspect-jsonpath already depends on the
// `regex` crate through suspect-lint; declare it here to keep this crate's
// deps honest.
fn regex_lite(pattern: &str) -> Result<regex::Regex, regex::Error> {
    regex::Regex::new(pattern)
}

/// `isDateTime(v)`: ISO 8601 date / RFC 3339 shape; mirrors
/// `functions.ts::isDateTime`.
#[must_use]
pub fn is_date_time(v: &serde_json::Value) -> bool {
    let Some(s) = v.as_str() else { return false };
    let bytes = s.as_bytes();
    if bytes.len() < 10 {
        return false;
    }
    let date = &s[..10];
    let Some((y, m, d)) = parse_ymd(date) else {
        return false;
    };
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return false;
    }
    if bytes.len() == 10 {
        return true;
    }
    let sep = bytes[10];
    if sep != b'T' && sep != b't' {
        return false;
    }
    let time = &s[11..];
    let time = time.strip_suffix(['Z', 'z']).unwrap_or(time);
    let time = split_offset(time);
    // HH:MM:SS with optional fraction
    let parts: Vec<&str> = time.split('.').collect();
    let hms: Vec<&str> = parts[0].split(':').collect();
    if hms.len() != 3 {
        return false;
    }
    let valid_time = hms
        .iter()
        .all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_digit()));
    let _ = (y, d, m);
    valid_time
}

fn parse_ymd(s: &str) -> Option<(i32, u32, u32)> {
    if s.len() != 10 || s.as_bytes()[4] != b'-' || s.as_bytes()[7] != b'-' {
        return None;
    }
    let y: i32 = s[..4].parse().ok()?;
    let m: u32 = s[5..7].parse().ok()?;
    let d: u32 = s[8..10].parse().ok()?;
    Some((y, m, d))
}

fn split_offset(t: &str) -> &str {
    if t.len() >= 6 {
        let bytes = t.as_bytes();
        let maybe = &t[t.len() - 6..];
        if (bytes[t.len() - 6] == b'+' || bytes[t.len() - 6] == b'-')
            && maybe[3..].starts_with(':')
            && maybe[3..].len() == 3
        {
            return &t[..t.len() - 6];
        }
    }
    t
}

/// `enumValues(v)`; mirrors `functions.ts::enumValues` (absent → `None`).
#[must_use]
pub fn enum_values(v: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    v.get("enum").and_then(|e| e.as_array())
}
