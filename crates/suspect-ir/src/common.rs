//! Pure helpers shared by the workspace and fast IR construction paths.
//!
//! Everything here is input-shape agnostic: the CST-based walk in
//! the CST walk (`lib`) and the `FastValue`-based walk in `fast` both
//! route through these functions so naming, decoding, and scalar semantics
//! cannot drift between the two pipelines.

/// HTTP method of an operation (re-exported type alias target).
use crate::Method;

/// Lowercase spec key for a method.
pub(crate) fn method_key(method: Method) -> &'static str {
    match method {
        Method::Get => "get",
        Method::Put => "put",
        Method::Post => "post",
        Method::Delete => "delete",
        Method::Options => "options",
        Method::Head => "head",
        Method::Patch => "patch",
        Method::Trace => "trace",
    }
}

/// Resolves a `$ref` value to a local component name.
///
/// URI-fragment decoding precedes JSON Pointer token decoding. Only a pointer
/// to an immediate component schema resolves to a name; malformed fragments
/// and references to nested schema locations stay unresolved.
pub(crate) fn local_schema_ref(reference: &str) -> Option<String> {
    let fragment = percent_decode(reference.strip_prefix('#')?)?;
    let token = fragment.strip_prefix("/components/schemas/")?;
    if token.is_empty() || token.contains('/') {
        return None;
    }
    let mut name = String::with_capacity(token.len());
    let mut chars = token.chars();
    while let Some(ch) = chars.next() {
        name.push(match ch {
            '~' => match chars.next()? {
                '0' => '~',
                '1' => '/',
                _ => return None,
            },
            ch => ch,
        });
    }
    Some(name)
}

/// Decodes URI-fragment `%XX` bytes without corrupting UTF-8 or replacing
/// invalid sequences. JSON Pointer escapes are handled by the caller.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let high = char::from(*bytes.get(i + 1)?).to_digit(16)? as u8;
            let low = char::from(*bytes.get(i + 2)?).to_digit(16)? as u8;
            out.push(high * 16 + low);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Collects local `#/components/schemas/{name}` references from JSON.
pub(crate) fn collect_local_refs(json: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    walk_refs(json, &mut out);
    out.sort();
    out.dedup();
    out
}

fn walk_refs(json: &serde_json::Value, out: &mut Vec<String>) {
    match json {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                if k == "$ref"
                    && let Some(name) = v.as_str().and_then(local_schema_ref)
                {
                    out.push(name);
                } else {
                    walk_refs(v, out);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                walk_refs(item, out);
            }
        }
        _ => {}
    }
}

/// Materializes scalar tokens using YAML 1.2 core-schema inference.
///
/// Quoted scalars stay strings. Finite numbers retain their exact value
/// through `serde_json`'s arbitrary-precision representation, without an
/// intermediate `i64` or `f64`. YAML-only number spellings are normalized to
/// JSON; non-JSON spellings that are not finite core-schema numbers retain
/// their text for validation rather than silently becoming zero or null.
pub(crate) fn scalar_json(raw: &str, quoted: bool) -> serde_json::Value {
    if quoted {
        return serde_json::Value::String(raw.to_owned());
    }
    match raw {
        "" | "~" | "null" | "Null" | "NULL" => serde_json::Value::Null,
        "true" | "True" | "TRUE" => serde_json::Value::Bool(true),
        "false" | "False" | "FALSE" => serde_json::Value::Bool(false),
        _ => {
            if let Ok(number) = raw.parse::<serde_json::Number>() {
                serde_json::Value::Number(number)
            } else if is_yaml_int(raw.as_bytes()) {
                serde_json::Value::Number(parse_yaml_int(raw))
            } else if is_yaml_float(raw.as_bytes()) {
                serde_json::Value::Number(parse_yaml_float(raw))
            } else {
                serde_json::Value::String(raw.to_owned())
            }
        }
    }
}

/// Mirrors `suspect_low::is_yaml_int`.
fn is_yaml_int(raw: &[u8]) -> bool {
    let Ok(s) = std::str::from_utf8(raw) else {
        return false;
    };
    let body = s.strip_prefix(['-', '+']).unwrap_or(s);
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

/// Mirrors `suspect_low::is_yaml_float`.
fn is_yaml_float(raw: &[u8]) -> bool {
    let Ok(s) = std::str::from_utf8(raw) else {
        return false;
    };
    let body = s.strip_prefix(['-', '+']).unwrap_or(s);
    if body.is_empty() {
        return false;
    }
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

/// Normalizes a validated YAML integer without a bounded integer conversion.
fn parse_yaml_int(s: &str) -> serde_json::Number {
    let (neg, body) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let magnitude = if let Some(hex) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        radix_to_decimal(hex, 16)
    } else if let Some(oct) = body.strip_prefix("0o").or_else(|| body.strip_prefix("0O")) {
        radix_to_decimal(oct, 8)
    } else {
        decimal_digits(body).to_owned()
    };
    let normalized = if neg {
        format!("-{magnitude}")
    } else {
        magnitude
    };
    normalized
        .parse()
        .expect("validated YAML integer normalizes to a JSON number")
}

/// Converts hexadecimal/octal digits to little-endian decimal digits.
fn radix_to_decimal(digits: &str, radix: u8) -> String {
    let mut decimal = vec![0u8];
    for digit in digits.chars() {
        let mut carry = digit
            .to_digit(u32::from(radix))
            .expect("validated YAML radix digit") as u8;
        for place in &mut decimal {
            let value = *place * radix + carry;
            *place = value % 10;
            carry = value / 10;
        }
        while carry != 0 {
            decimal.push(carry % 10);
            carry /= 10;
        }
    }
    decimal
        .into_iter()
        .rev()
        .map(|digit| char::from(b'0' + digit))
        .collect()
}

/// Removes YAML's optional leading zeroes, retaining a zero for an empty part.
fn decimal_digits(digits: &str) -> &str {
    let trimmed = digits.trim_start_matches('0');
    if trimmed.is_empty() { "0" } else { trimmed }
}

/// Normalizes a validated finite YAML float by editing its spelling only.
fn parse_yaml_float(s: &str) -> serde_json::Number {
    let body = s.strip_prefix(['-', '+']).unwrap_or(s);
    let (mantissa, exponent) = match body.find(['e', 'E']) {
        Some(index) => (&body[..index], &body[index..]),
        None => (body, ""),
    };
    let (integer, fraction) = match mantissa.split_once('.') {
        Some((integer, fraction)) => (integer, Some(fraction)),
        None => (mantissa, None),
    };
    let mut normalized = String::with_capacity(s.len() + 2);
    if s.starts_with('-') {
        normalized.push('-');
    }
    normalized.push_str(decimal_digits(integer));
    if let Some(fraction) = fraction {
        normalized.push('.');
        normalized.push_str(if fraction.is_empty() { "0" } else { fraction });
    }
    normalized.push_str(exponent);
    normalized
        .parse()
        .expect("validated finite YAML float normalizes to a JSON number")
}
