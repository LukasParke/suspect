# Rust SDK: optional serde-json interoperability profile

Status: M2 Rust native-DX addition. Everything here is behind the generated package
feature `serde-json`; without it the generated package has no serde dependency and
compiles exactly as before.

## What is generated

The canonical model/codec and HTTP packages share a structured Cargo manifest
emitter. The model/codec base includes:

```toml
[features]
default = []
serde-json = ["dep:serde", "dep:serde_json"]

[dependencies]
serde = { version = "=1.0.229", optional = true }
serde_json = { version = "=1.0.151", optional = true, features = ["raw_value"] }
```

Versions are the reviewed workspace pins (`Cargo.lock`: serde 1.0.229, serde_json
1.0.151). `raw_value` preserves original JSON tokens. The package leaves the
caller's `arbitrary_precision` choice alone; native consumers exercise both
settings under Cargo feature unification.

HTTP packages (`rust_http::emit_http`) add `http`/`reqwest-rustls` features and the
pinned `url`/`reqwest` dependencies onto this same base manifest; `serde-json` stays
independent of HTTP. Enabling the adapter does not enable a transport; transports
do not enable these codec adapters (their own dependency graphs can use Serde).

## Adapter API

Every `codecs::<Symbol>Codec` gains, under `#[cfg(feature = "serde-json")]`:

```rust,ignore
impl SymbolCodec {
    pub fn serialize<S: serde::Serializer>(
        model: &crate::models::Symbol,
        serializer: S,
    ) -> Result<S::Ok, S::Error>;

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<crate::models::Symbol, D::Error>;
}
```

Caller field adapter, using the model's actual allocated codec symbol:

```rust,ignore
#[derive(serde::Serialize, serde::Deserialize)]
struct Wrapper {
    #[serde(with = "AreaCodec")]
    area: Area,
}
```

Semantics:

- `serialize` calls the real `SymbolCodec::encode` on the (possibly mutated) model
  first — full schema check, enum-branch check and codec budgets apply — then exposes
  the exact produced JSON to serde_json via `serde_json::value::RawValue`. Number
  tokens are emitted verbatim; nothing re-derives them from floats.
- `deserialize` captures the raw JSON text with `Box<serde_json::value::RawValue>`
  **before** decoding, so the real codec (not a `serde_json::Value -> f64` path)
  validates and converts with original number tokens intact.
- Errors from encode/decode become `serde::ser::Error::custom` /
  `serde::de::Error::custom`. Mutated invalid models are rejected at serialization
  time; ambiguous exclusive (`oneOf`) values are rejected at deserialization time.
- `Nullable`/`Presence`, additional properties, recursion, tagged unions and alias
  semantics are exactly the codec semantics; the adapters add nothing.

## JSON-only by design

`RawValue` deserialization is defined for serde_json-based `Deserializer`s
(`serde_json::from_str`, `from_slice`, or `from_reader` over original JSON).
Re-deserializing an already parsed `serde_json::Value` cannot recover tokens or
precision its original parsing discarded. Other Serde formats may interpret the
private raw-value wrapper differently and are outside this adapter's support.

The outer serde_json parser captures the complete value before the codec checks
its byte/evaluation/conversion limits. Bound the caller's input/reader and apply
the caller deserializer's policy as well. For SDK-controlled parsing from the
start, use `SymbolCodec::decode` or `decode_bytes` directly. Neither entry point
changes the schema, supplies defaults or strips fields.

## Verification

`crates/suspect-codegen/tests/rust_serde.rs` (native, `#[ignore = "requires native
Cargo"]`): checks the feature-off active dependency tree, builds and documents
the generated package, then uses a renamed Cargo dependency from a real caller.
Caller precision features are tested both off/on. Cases cover exact
big/tiny/exponent values, absence/null, mutable invalid models, oneOf ambiguity,
recursive values, malformed JSON/UTF-8, duplicate keys and finite depth. A model
literally named `Result` stays usable through its own module-qualified symbol.

Codec budgets are enforced inside the adapters (they call the ordinary codec paths);
no emitted Rust is parsed by the generator.
