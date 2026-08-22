use suspect_syntax::{Format, ScalarStyle};

/// The semantic kind of a value, after YAML 1.2 core-schema inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    /// Empty, `~`, or `null` (any case); also JSON `null`.
    Null,
    /// Boolean literal; YAML accepts `true`/`True`/`TRUE` spellings.
    Bool,
    /// Integer literal (decimal, `0o` octal, or `0x` hexadecimal).
    Int,
    /// Floating-point literal, including `.inf`/`.nan` forms and exponents.
    Float,
    /// Any other scalar: quoted text, block scalars, or plain strings.
    Str,
    /// Mapping / object.
    Object,
    /// Sequence / array.
    Array,
}

/// Infers the semantic type of a scalar from its raw bytes and quoting style.
///
/// YAML 1.2 core schema: `null`/`~`/empty → Null; `true|false` (any case
/// variant of the three spellings) → Bool; decimal/octal/hex integers;
/// floats incl. `.inf`/`.nan`. Quoted and block scalars are always Str.
/// JSON literals follow JSON.
#[must_use]
pub fn infer_scalar(raw: &[u8], style: ScalarStyle, format: Format) -> ValueKind {
    match style {
        ScalarStyle::SingleQuoted | ScalarStyle::DoubleQuoted | ScalarStyle::Block => {
            ValueKind::Str
        }
        ScalarStyle::Plain => infer_plain(raw, format),
    }
}

fn infer_plain(raw: &[u8], format: Format) -> ValueKind {
    if format == Format::Json {
        return match raw {
            b"true" | b"false" => ValueKind::Bool,
            b"null" => ValueKind::Null,
            _ => {
                if raw.first() == Some(&b'"') {
                    ValueKind::Str
                } else if raw
                    .iter()
                    .all(|b| b.is_ascii_digit() || matches!(b, b'-' | b'+'))
                    && !raw.is_empty()
                {
                    // integer-shaped (sign + digits only)
                    if raw.iter().filter(|&&b| b == b'-' || b == b'+').count() > 1
                        || raw.first() == Some(&b'+')
                        || raw == b"-"
                    {
                        ValueKind::Str
                    } else if raw.iter().any(|&b| b.is_ascii_digit()) {
                        ValueKind::Int
                    } else {
                        ValueKind::Str
                    }
                } else if looks_like_float_json(raw) {
                    ValueKind::Float
                } else {
                    ValueKind::Str
                }
            }
        };
    }
    // YAML 1.2 core schema
    match raw {
        b"" | b"~" | b"null" | b"Null" | b"NULL" => return ValueKind::Null,
        b"true" | b"True" | b"TRUE" | b"false" | b"False" | b"FALSE" => return ValueKind::Bool,
        b".inf" | b".Inf" | b".INF" | b"+.inf" | b"+.Inf" | b"+.INF" | b"-.inf" | b"-.Inf"
        | b"-.INF" => return ValueKind::Float,
        b".nan" | b".NaN" | b".NAN" => return ValueKind::Float,
        _ => {}
    }
    if is_yaml_int(raw) {
        ValueKind::Int
    } else if is_yaml_float(raw) {
        ValueKind::Float
    } else {
        ValueKind::Str
    }
}

fn looks_like_float_json(raw: &[u8]) -> bool {
    // JSON number with fraction or exponent
    let s = match std::str::from_utf8(raw) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let body = s.strip_prefix('-').unwrap_or(s);
    if body.is_empty() {
        return false;
    }
    let (int_part, rest) = match body.split_once('.') {
        Some((i, r)) => (i, Some(r)),
        None => match body.split_once(['e', 'E']) {
            Some((i, r)) => (i, Some(r)),
            None => (body, None),
        },
    };
    if int_part.is_empty() || !int_part.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    match rest {
        None => false,
        Some(frac) => {
            let (frac_digits, exp) = match frac.split_once(['e', 'E']) {
                Some((f, e)) => (f, Some(e)),
                None => (frac, None),
            };
            let frac_ok = frac_digits.is_empty() || frac_digits.bytes().all(|b| b.is_ascii_digit());
            let exp_ok = match exp {
                None => true,
                Some(e) => {
                    let e = e
                        .strip_prefix('+')
                        .unwrap_or(e.strip_prefix('-').unwrap_or(e));
                    !e.is_empty() && e.bytes().all(|b| b.is_ascii_digit())
                }
            };
            frac_ok && exp_ok
        }
    }
}

fn is_yaml_int(raw: &[u8]) -> bool {
    let s = match std::str::from_utf8(raw) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let (sign, body) = match s.as_bytes().first() {
        Some(b'-') => (-1i8, &s[1..]),
        Some(b'+') => (1, &s[1..]),
        _ => (0, s),
    };
    let _ = sign;
    if body.is_empty() {
        return false;
    }
    if let Some(hex) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        return !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit());
    }
    if let Some(oct) = body.strip_prefix("0o").or_else(|| body.strip_prefix("0O")) {
        return !oct.is_empty() && oct.bytes().all(|b| (b'0'..=b'7').contains(&b));
    }
    body.bytes().all(|b| b.is_ascii_digit())
}

fn is_yaml_float(raw: &[u8]) -> bool {
    let Ok(s) = std::str::from_utf8(raw) else {
        return false;
    };
    let body = s.strip_prefix(['-', '+']).unwrap_or(s);
    if body.is_empty() {
        return false;
    }
    // mantissa[ (e|E) [+|-] digits ]
    let (mantissa, exp) = match body.split_once(['e', 'E']) {
        Some((m, e)) => (m, Some(e)),
        None => (body, None),
    };
    if let Some(e) = exp {
        let e = e.strip_prefix(['+', '-']).unwrap_or(e);
        if e.is_empty() || !e.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
    }
    let (int_part, frac_part) = match mantissa.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (mantissa, None),
    };
    let int_ok = int_part.bytes().all(|b| b.is_ascii_digit());
    let frac_ok = frac_part.is_none_or(|f| f.bytes().all(|b| b.is_ascii_digit()));
    let has_digit = !int_part.is_empty() || frac_part.is_some_and(|f| !f.is_empty());
    let dot_or_exp = frac_part.is_some() || exp.is_some();
    int_ok && frac_ok && has_digit && dot_or_exp
}

/// Parses an inferred integer scalar.
#[must_use]
pub fn parse_int(raw: &[u8], format: Format) -> Option<i64> {
    if format == Format::Json {
        return std::str::from_utf8(raw).ok()?.parse().ok();
    }
    let Ok(s) = std::str::from_utf8(raw) else {
        return None;
    };
    let (neg, body) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let magnitude = if let Some(hex) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16).ok()?
    } else if let Some(oct) = body.strip_prefix("0o").or_else(|| body.strip_prefix("0O")) {
        i64::from_str_radix(oct, 8).ok()?
    } else {
        body.parse::<i64>().ok()?
    };
    Some(if neg { -magnitude } else { magnitude })
}

pub fn parse_float(raw: &[u8]) -> Option<f64> {
    let s = std::str::from_utf8(raw).ok()?;
    match s {
        ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" => return Some(f64::INFINITY),
        "-.inf" | "-.Inf" | "-.INF" => return Some(f64::NEG_INFINITY),
        ".nan" | ".NaN" | ".NAN" => return Some(f64::NAN),
        _ => {}
    }
    s.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn y(raw: &[u8]) -> ValueKind {
        infer_scalar(raw, ScalarStyle::Plain, Format::Yaml)
    }

    #[test]
    fn yaml_core_schema() {
        assert_eq!(y(b""), ValueKind::Null);
        assert_eq!(y(b"~"), ValueKind::Null);
        assert_eq!(y(b"null"), ValueKind::Null);
        assert_eq!(y(b"TRUE"), ValueKind::Bool);
        assert_eq!(y(b"False"), ValueKind::Bool);
        assert_eq!(y(b"42"), ValueKind::Int);
        assert_eq!(y(b"-7"), ValueKind::Int);
        assert_eq!(y(b"0x1f"), ValueKind::Int);
        assert_eq!(y(b"0o17"), ValueKind::Int);
        assert_eq!(y(b"3.14"), ValueKind::Float);
        assert_eq!(y(b"-1e10"), ValueKind::Float);
        assert_eq!(y(b".inf"), ValueKind::Float);
        assert_eq!(y(b".NaN"), ValueKind::Float);
        assert_eq!(y(b"hello"), ValueKind::Str);
        assert_eq!(y(b"3.1.0"), ValueKind::Str); // version strings stay strings
        assert_eq!(y(b"1abc"), ValueKind::Str);
        assert_eq!(y(b"yes"), ValueKind::Str); // 1.2 core: yes is a string
        assert_eq!(y(b"on"), ValueKind::Str);
    }

    #[test]
    fn quoted_is_always_string() {
        assert_eq!(
            infer_scalar(b"true", ScalarStyle::SingleQuoted, Format::Yaml),
            ValueKind::Str
        );
        assert_eq!(
            infer_scalar(b"42", ScalarStyle::DoubleQuoted, Format::Yaml),
            ValueKind::Str
        );
        assert_eq!(
            infer_scalar(b"line", ScalarStyle::Block, Format::Yaml),
            ValueKind::Str
        );
    }

    #[test]
    fn json_literals() {
        assert_eq!(
            infer_scalar(b"true", ScalarStyle::Plain, Format::Json),
            ValueKind::Bool
        );
        assert_eq!(
            infer_scalar(b"null", ScalarStyle::Plain, Format::Json),
            ValueKind::Null
        );
        assert_eq!(
            infer_scalar(b"12", ScalarStyle::Plain, Format::Json),
            ValueKind::Int
        );
        assert_eq!(
            infer_scalar(b"1.5", ScalarStyle::Plain, Format::Json),
            ValueKind::Float
        );
        assert_eq!(
            infer_scalar(b"1e9", ScalarStyle::Plain, Format::Json),
            ValueKind::Float
        );
        assert_eq!(
            infer_scalar(b"3.1", ScalarStyle::Plain, Format::Json),
            ValueKind::Float
        );
    }

    #[test]
    fn parse_int_forms() {
        assert_eq!(parse_int(b"0x1f", Format::Yaml), Some(31));
        assert_eq!(parse_int(b"0o17", Format::Yaml), Some(15));
        assert_eq!(parse_int(b"-42", Format::Yaml), Some(-42));
        assert_eq!(parse_int(b"42", Format::Json), Some(42));
        assert_eq!(parse_int(b"nope", Format::Yaml), None);
    }
}
