# Exact JSON for generated Rust SDKs

The generated Rust JSON runtime parses and writes the existing `JsonValue` domain without an external runtime dependency. It preserves every valid JSON number token exactly, including negative zero, exponent spelling, and integers beyond native ranges. Parsing never routes numbers through floating point.

The public runtime API is:

```rust
pub fn parse_json(text: &str, limits: JsonLimits) -> Result<JsonValue, JsonError>;
pub fn parse_json_bytes(bytes: &[u8], limits: JsonLimits) -> Result<JsonValue, JsonError>;
pub fn stringify_json(value: &JsonValue, limits: JsonLimits) -> Result<String, JsonError>;
```

`JsonLimits` bounds input bytes, output bytes, nesting depth, and explicit scanning/traversal work. Defaults are finite. Input size is checked before parsing or allocating decoded containers, depth is checked before allocating a nested container, and output capacity is capped before every append. Work is charged incrementally while scanning, decoding, traversing, and writing; numeric validation, temporary digits, and owned token storage are admitted separately from the initial token scan. This counter is not a complete CPU or memory accounting mechanism: standard UTF-8 admission, allocator overhead, and `BTreeMap` key comparisons are bounded indirectly by the byte/depth limits rather than charged exactly. The recursive implementation rejects requested depth limits above its fixed ceiling of 256, so a caller cannot turn a configured limit into unbounded native stack use. Model codecs apply the documented JSON, validation and conversion budgets at their respective boundaries.

`JsonErrorKind` distinguishes syntax, invalid UTF-8, duplicate keys, and resource limits. Errors include a byte offset when the failure belongs to input. Messages are fixed and bounded; errors never retain or echo the input body or property value.

Objects use the existing `BTreeMap<String, JsonValue>`. Duplicate decoded names are rejected, including names that differ only in escape spelling. Output is deterministic in map-key order. String output uses direct Unicode scalars plus JSON escapes for quotes, backslashes, and controls. Object order and whitespace from the source are not retained because they are not part of the `JsonValue` representation.

Rust `&str` can never contain malformed UTF-8 or a lone surrogate scalar. `parse_json_bytes` reports malformed UTF-8 explicitly. For JSON escape sequences, the parser accepts a valid high/low UTF-16 surrogate pair and decodes it to one Unicode scalar; it rejects lone high surrogates, lone low surrogates, and invalid pairs. Because Rust strings cannot represent lone surrogates, the writer cannot be asked to emit one.

This layer establishes exact JSON representation only. It does not validate an OpenAPI schema, select a union branch, apply defaults, coerce values, or convert models. Generated model codecs must parse, validate the selected checked portable program, and then convert. Encoding must convert a model, validate the resulting `JsonValue`, and then write it.

The source template lives at `crates/suspect-codegen/src/rust_codecs/json_runtime.rs`. `ModelPlan::render` emits it as `rust/src/json.rs` and re-exports the public API from the generated crate root. It refers only to generated support types and Rust's standard library. The separate `rust_codecs::plan_codecs` layer now adds source-aware model conversion and checked validation for admitted model closures; see [Rust model codecs](SDK-RUST-MODEL-PLAN.md#validated-model-codecs). The JSON representation helpers themselves remain schema-neutral.
