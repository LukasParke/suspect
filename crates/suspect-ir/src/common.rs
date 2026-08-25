//! Pure helpers shared by the workspace and fast IR construction paths.
//!
//! Everything here is input-shape agnostic: the CST-based walk in
//! the CST walk (`lib`) and the [`FastValue`]-based walk in `fast` both
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
/// Local `#/components/schemas/{name}` references resolve to the bare,
/// percent-decoded component name; anything else stays unresolved.
pub(crate) fn local_schema_ref(reference: &str) -> Option<String> {
    reference
        .strip_prefix("#/components/schemas/")
        .map(percent_decode)
        .filter(|n| !n.is_empty())
}

/// Decodes `~1`/`~0` JSON-pointer escapes plus `%XX` sequences.
pub(crate) fn percent_decode(text: &str) -> String {
    let unescaped = text.replace("~1", "/").replace("~0", "~");
    let bytes = unescaped.as_bytes();
    let mut out = String::with_capacity(unescaped.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &unescaped[i + 1..i + 3];
            if let Ok(value) = u8::from_str_radix(hex, 16) {
                out.push(value as char);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
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
                    && let Some(name) = v
                        .as_str()
                        .and_then(|r| r.strip_prefix("#/components/schemas/"))
                {
                    out.push(name.to_owned());
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

/// Materializes one plain-scalar token into JSON using the YAML 1.2 core
/// schema — the exact rules `suspect-low` applies (`infer_scalar`,
/// `parse_int`, `parse_float`, mirrored here because that crate does not
/// export the parse helpers).
///
/// Quoted scalars are always strings. Non-finite floats become `null`, and
/// integer literals outside `i64` degrade to `0`, matching the overlay
/// round-trip (`Value::Int(as_i64().unwrap_or(0))` through a JSON string).
pub(crate) fn scalar_json(raw: &str, quoted: bool) -> serde_json::Value {
    if quoted {
        return serde_json::Value::String(raw.to_owned());
    }
    match raw {
        "" | "~" | "null" | "Null" | "NULL" => serde_json::Value::Null,
        "true" | "True" | "TRUE" => serde_json::Value::Bool(true),
        "false" | "False" | "FALSE" => serde_json::Value::Bool(false),
        _ => {
            if is_yaml_int(raw.as_bytes()) {
                serde_json::Value::Number(serde_json::Number::from(
                    parse_yaml_int(raw).unwrap_or(0),
                ))
            } else if is_yaml_float(raw.as_bytes()) {
                match parse_yaml_float(raw) {
                    Some(f) => serde_json::Number::from_f64(f)
                        .map_or(serde_json::Value::Null, serde_json::Value::Number),
                    None => serde_json::Value::Null,
                }
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

/// Mirrors `suspect_low::parse_int` for YAML spellings.
fn parse_yaml_int(s: &str) -> Option<i64> {
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

/// Mirrors `suspect_low::parse_float`.
fn parse_yaml_float(s: &str) -> Option<f64> {
    match s {
        ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" => return Some(f64::INFINITY),
        "-.inf" | "-.Inf" | "-.INF" => return Some(f64::NEG_INFINITY),
        ".nan" | ".NaN" | ".NAN" => return Some(f64::NAN),
        _ => {}
    }
    s.parse().ok()
}
