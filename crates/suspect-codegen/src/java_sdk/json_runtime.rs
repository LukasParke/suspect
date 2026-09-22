//! Java sources embedded as static strings so the crate has no build-time
//! Java dependency and the exact bytes are reviewable in-tree.

/// Exact JSON value domain: recursive `JsonValue` with opaque `JsonNumber`
/// carrying a symbolic `BigInteger` unscaled-value/exponent pair, parsed by a
/// native recursive-descent parser with bounded input depth. No float
/// conversion anywhere; equality is exact numeric equality.
#[must_use]
pub fn json_runtime_source() -> &'static str {
    include_str!("JsonRuntime.java")
}
