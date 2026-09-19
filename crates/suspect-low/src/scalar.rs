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
            _ => json_number_kind(raw).unwrap_or(ValueKind::Str),
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

/// RFC 8259 number grammar. Classification depends on spelling, not the
/// representable range of a Rust integer or floating-point type.
fn json_number_kind(raw: &[u8]) -> Option<ValueKind> {
    let mut rest = raw.strip_prefix(b"-").unwrap_or(raw);
    match rest.first()? {
        b'0' => rest = &rest[1..],
        b'1'..=b'9' => {
            let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
            rest = &rest[digits..];
        }
        _ => return None,
    }
    let mut floating = false;
    if let Some(fraction) = rest.strip_prefix(b".") {
        floating = true;
        let digits = fraction.iter().take_while(|b| b.is_ascii_digit()).count();
        if digits == 0 {
            return None;
        }
        rest = &fraction[digits..];
    }
    if matches!(rest.first(), Some(b'e' | b'E')) {
        floating = true;
        let exponent = &rest[1..];
        let exponent = match exponent.first() {
            Some(b'+' | b'-') => &exponent[1..],
            _ => exponent,
        };
        let digits = exponent.iter().take_while(|b| b.is_ascii_digit()).count();
        if digits == 0 {
            return None;
        }
        rest = &exponent[digits..];
    }
    rest.is_empty().then_some(if floating {
        ValueKind::Float
    } else {
        ValueKind::Int
    })
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

/// Tests an already classified decimal scalar for mathematical integrality.
/// Exponent magnitude is bounded by the token length, so the check neither
/// rounds through a float nor expands arbitrarily large powers of ten.
pub(crate) fn decimal_is_integer(raw: &[u8]) -> bool {
    let body = match raw.first() {
        Some(b'+' | b'-') => &raw[1..],
        _ => raw,
    };
    let (coefficient, exponent) = body
        .iter()
        .position(|b| matches!(b, b'e' | b'E'))
        .map_or((body, b"0".as_slice()), |index| {
            (&body[..index], &body[index + 1..])
        });
    // YAML's nonfinite spellings are Float-kind scalars, but not integers.
    if !coefficient.iter().all(|b| b.is_ascii_digit() || *b == b'.')
        || !coefficient.iter().any(u8::is_ascii_digit)
    {
        return false;
    }
    if coefficient.iter().all(|b| matches!(b, b'0' | b'.')) {
        return true;
    }
    let fraction = coefficient
        .iter()
        .position(|b| *b == b'.')
        .map_or(0, |index| coefficient.len() - index - 1);
    let trailing_zeroes = coefficient
        .iter()
        .rev()
        .filter(|b| **b != b'.')
        .take_while(|b| **b == b'0')
        .count();
    let digits = match exponent.first() {
        Some(b'+' | b'-') => &exponent[1..],
        _ => exponent,
    };
    let bound = raw.len().saturating_add(1);
    let magnitude = digits.iter().fold(0usize, |value, digit| {
        value
            .saturating_mul(10)
            .saturating_add(usize::from(digit - b'0'))
            .min(bound)
    });
    if exponent.first() == Some(&b'-') {
        trailing_zeroes >= fraction.saturating_add(magnitude)
    } else {
        magnitude.saturating_add(trailing_zeroes) >= fraction
    }
}

/// Parses an inferred integer scalar.
#[must_use]
pub fn parse_int(raw: &[u8], format: Format) -> Option<i64> {
    if format == Format::Json {
        return std::str::from_utf8(raw).ok()?.parse().ok();
    }
    let (neg, magnitude) = integer_magnitude(raw, format)?;
    // The magnitude of i64::MIN is one larger than i64::MAX. Keep the sign
    // separate until both extremes can be represented, then check the range.
    let magnitude = i128::from(magnitude);
    i64::try_from(if neg { -magnitude } else { magnitude }).ok()
}

/// Parses an inferred integer scalar without first narrowing through i64.
pub(crate) fn parse_uint(raw: &[u8], format: Format) -> Option<u64> {
    let (neg, magnitude) = integer_magnitude(raw, format)?;
    (!neg || magnitude == 0).then_some(magnitude)
}

fn integer_magnitude(raw: &[u8], format: Format) -> Option<(bool, u64)> {
    let s = std::str::from_utf8(raw).ok()?;
    let (neg, body) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let magnitude = if format == Format::Json {
        body.parse::<u64>().ok()?
    } else if let Some(hex) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok()?
    } else if let Some(oct) = body.strip_prefix("0o").or_else(|| body.strip_prefix("0O")) {
        u64::from_str_radix(oct, 8).ok()?
    } else {
        body.parse::<u64>().ok()?
    };
    Some((neg, magnitude))
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
