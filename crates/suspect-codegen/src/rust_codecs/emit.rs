use super::CodecConfig;
use crate::{
    OutFile,
    rust_models::{Decl, Key, ModelPlan, RepresentationRole, Type, prose},
};
use std::collections::BTreeMap;
use suspect_ir::contract::SchemaId;

type Indices = BTreeMap<(String, String), usize>;
fn root(indices: &Indices, source: &SchemaId) -> usize {
    indices[&(source.document().to_string(), source.pointer().into())]
}
fn literal_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "Nullable::Null".into(),
        serde_json::Value::Bool(value) => {
            format!("Nullable::Value(JsonNonNullValue::Bool({value}))")
        }
        serde_json::Value::String(value) => {
            format!("Nullable::Value(JsonNonNullValue::String({value:?}.into()))")
        }
        serde_json::Value::Number(value) => format!(
            "Nullable::Value(JsonNonNullValue::Number({:?}.parse().expect(\"compiler-checked source number\")))",
            value.to_string()
        ),
        _ => unreachable!("compound literals are rejected during model planning"),
    }
}

pub(super) fn package(
    models: &ModelPlan,
    indices: &Indices,
    config: &CodecConfig,
    crate_name: &str,
) -> Vec<OutFile> {
    let mut code = String::from(
        "//! Validated native codecs; all schema trials share one per-call evaluation session.\n#![allow(dead_code, unused_imports)]\n",
    );
    code.push_str(include_str!("model_runtime.rs"));
    let j = config.json_limits;
    code.push_str(&format!("\nconst JSON_LIMITS: crate::JsonLimits = crate::JsonLimits {{ max_input_bytes: {}, max_output_bytes: {}, max_depth: {}, max_work: {} }};\nconst CONVERSION_DEPTH: usize = {};\nconst CONVERSION_STEPS: usize = {};\n",j.max_input_bytes,j.max_output_bytes,j.max_depth,j.max_work,config.max_conversion_depth,config.max_conversion_steps));
    let emitter = Emitter {
        indices,
        functions: models
            .declarations
            .keys()
            .enumerate()
            .map(|(i, k)| (k.clone(), i))
            .collect(),
    };
    let mut model_code = String::from(
        "//! Neutral native models. Use crate::codecs to validate decode and encode.\n",
    );
    let mut docs = String::from(
        "# Rust model codecs\n\nStandalone Rust 2024 package (Rust 1.88+), no dependencies. Neutral models and exact JSON codecs; no HTTP transport or request/response projection.\n\nEvery public symbol has a `codecs::<Symbol>Codec` with `decode(&str)`, `decode_bytes(&[u8])`, `decode_value(JsonValue)`, `encode(&Model)` and `encode_value(&Model)`. Decode validates before conversion; encode checks the selected enum branch and complete schema after conversion. Inclusive unions choose the first validating source branch; exclusive unions require exactly one. Evaluation failure is never treated as a branch mismatch.\n\n`Nullable` and `Presence` distinguish null from absence. Unknown properties retain their declared additional-property type independently of named properties. Exact numbers and arbitrary integers retain tokens; native bounded integer fields retain mathematical values (not spelling). Literal enum variants encode the canonical source literal. No defaults or coercions are inferred.\n\n`CodecError` distinguishes JSON errors, invalid source values, evaluation failures and conversion failures. Schema budgets are shared across the root check, all branch trials, and literal equality. JSON bytes/work/nesting and conversion work/depth have separate finite policies; they are not complete allocator/CPU accounting.\n\nRun `cargo test`, `cargo test --doc`, and `cargo doc --no-deps`.\n\n",
    );
    docs.push_str("## Optional serde-json interoperability\n\nWith the package feature `serde-json` enabled, every `{Symbol}Codec` gains `serialize(model, serializer)` and `deserialize(deserializer)` functions suitable for `#[serde(with = \"SymbolCodec\")]` on caller fields. Serialization runs the full validated encode and emits exact JSON verbatim; deserialization captures raw JSON through `serde_json::value::RawValue` before invoking the real codec. The adapters are supported with serde_json serializers/deserializers only; other formats may interpret Serde's private raw-value representation differently. The feature enables `raw_value` without enabling `arbitrary_precision` in the caller's graph; both caller precision settings preserve these codec values.\n\nCodec input, evaluation and conversion limits apply after serde_json captures the value. The caller must bound the outer JSON input/reader and configure its deserializer; that external capture can allocate/scan before codec limits apply. Use the codec's `decode`/`decode_bytes` methods directly for SDK-controlled initial parsing. With the feature off the package has no active Serde dependency.\n\n");
    for symbol in models.symbols() {
        let name = symbol.name();
        let key = (symbol.source().clone(), symbol.role());
        let index = emitter.functions[&key];
        let node = root(indices, symbol.source());
        let document = symbol.source().document().to_string();
        let pointer = symbol.source().pointer();
        let description = prose(&symbol.description);
        model_code.push_str(&format!(
            "/// {description}\n/// Source: {}#{}\n{}\n\n",
            prose(&document),
            prose(pointer),
            symbol.code
        ));
        code.push_str(&format!("\n/// Validated codec for [`crate::models::{name}`].\n/// {description}\n/// Source: {}#{}\n///\n/// ```no_run\n/// use {crate_name}::{{codecs::{name}Codec, models::{name}}};\n/// fn round_trip(value: &{name}) -> std::result::Result<{name}, {crate_name}::codecs::CodecError> {{\n///     {name}Codec::decode(&{name}Codec::encode(value)?)\n/// }}\n/// ```\npub struct {name}Codec;\nimpl {name}Codec {{\n    /// Decode exact JSON and validate its source schema.\n    pub fn decode(input: &str) -> Result<crate::models::{name}, CodecError> {{ Self::decode_value(crate::parse_json(input, JSON_LIMITS)?) }}\n    /// Decode UTF-8 JSON bytes, rejecting malformed UTF-8 and duplicate keys.\n    pub fn decode_bytes(input: &[u8]) -> Result<crate::models::{name}, CodecError> {{ Self::decode_value(crate::parse_json_bytes(input, JSON_LIMITS)?) }}\n    /// Validate and consume an exact JSON value.\n    pub fn decode_value(value: JsonValue) -> Result<crate::models::{name}, CodecError> {{ let mut cx=Context::new({document:?},{pointer:?}); cx.check({node},&value)?; decode_{index}(value,&mut cx,0) }}\n    /// Validate a possibly mutated native model and write exact JSON.\n    pub fn encode(value: &crate::models::{name}) -> Result<String, CodecError> {{ Ok(crate::stringify_json(&Self::encode_value(value)?, JSON_LIMITS)?) }}\n    /// Convert a possibly mutated model and validate its complete source schema.\n    pub fn encode_value(value: &crate::models::{name}) -> Result<JsonValue, CodecError> {{ let mut cx=Context::new({document:?},{pointer:?}); let wire=encode_{index}(value,&mut cx,0)?; cx.check({node},&wire)?; Ok(wire) }}\n}}\n",prose(&document),prose(pointer)));
        let decl = &models.declarations[&key];
        if let Decl::Literals(literals) = decl {
            for (literal_index, literal) in literals.iter().enumerate() {
                code.push_str(&format!("static LITERAL_{index}_{literal_index}: std::sync::LazyLock<JsonValue> = std::sync::LazyLock::new(|| {});\n",literal_value(&literal.value)));
            }
        }
        let decode = emitter.decode_decl(decl, name, index, node);
        let encode = emitter.encode_decl(decl, name, index);
        let nonnull = if symbol.role() == RepresentationRole::NonNullValue {
            "if matches!(v, Nullable::Null) { return Err(cx.error(\"expected non-null representation\")); }"
        } else {
            ""
        };
        code.push_str(&format!("fn decode_{index}(v: JsonValue, cx: &mut Context, d: usize) -> Result<crate::models::{name},CodecError> {{ cx.source({document:?},{pointer:?},|cx| {{ let _ = &v; cx.step(d)?; {nonnull} {decode} }}) }}\nfn encode_{index}(v: &crate::models::{name}, cx: &mut Context, d: usize) -> Result<JsonValue,CodecError> {{ cx.source({document:?},{pointer:?},|cx| {{ let _ = v; cx.step(d)?; {encode} }}) }}\n"));
        code.push_str(&format!("#[cfg(feature = \"serde-json\")]\nimpl {name}Codec {{\n    /// Serialize through the validated codec: encode (schema-checked, budgeted) and\n    /// emit the exact produced JSON verbatim via `serde_json::value::RawValue`. Number\n    /// tokens are never reinterpreted. JSON only; not a general Serde-format adapter.\n    pub fn serialize<S: serde::Serializer>(model: &crate::models::{name}, serializer: S) -> Result<S::Ok, S::Error> {{\n        let text = Self::encode(model).map_err(serde::ser::Error::custom)?;\n        let raw = serde_json::value::RawValue::from_string(text).map_err(serde::ser::Error::custom)?;\n        serde::Serialize::serialize(&*raw, serializer)\n    }}\n    /// Deserialize through the validated codec from a serde_json-based\n    /// `Deserializer`. The raw JSON text (including number tokens) is captured first,\n    /// then decoded by the real codec; values never round-trip through binary floats.\n    /// Other Serde formats are rejected by design.\n    pub fn deserialize<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<crate::models::{name}, D::Error> {{\n        let raw: std::boxed::Box<serde_json::value::RawValue> = serde::Deserialize::deserialize(deserializer)?;\n        Self::decode(raw.get()).map_err(serde::de::Error::custom)\n    }}\n}}\n"));
        docs.push_str(&format!("## `{name}` / `{name}Codec`\n\n{description}\n\nSource: `{}#{}`. Role: `{:?}`. [Model](src/models.rs), [codec](src/codecs.rs).\n\n```rust,ignore\n{}\n```\n\nOriginal schema:\n\n```json\n{}\n```\n\n",prose(&document),prose(pointer),symbol.role(),symbol.code,symbol.source_json));
    }
    vec![
        OutFile { path:"rust/src/codecs.rs".into(),content:code },
        OutFile { path:"rust/src/models.rs".into(),content:model_code },
        OutFile { path:"rust/README.md".into(),content:docs },
        OutFile { path:"rust/src/lib.rs".into(),content:"//! Exact neutral OpenAPI models and validated native codecs. No HTTP transport.\n#![forbid(unsafe_code)]\npub mod models;\npub mod codecs;\npub mod validation;\nmod support;\nmod json;\npub use support::{ExtraFieldError, JsonInteger, JsonNonNullValue, JsonNumber, JsonValue, Never, Nullable, NumberError, Presence};\npub use json::{JsonError, JsonErrorKind, JsonLimits, parse_json, parse_json_bytes, stringify_json};\n".into() },
        OutFile { path:"rust/Cargo.toml".into(),content:super::cargo_manifest("generated-models","0.0.0",false) },
    ]
}
struct Emitter<'a> {
    indices: &'a Indices,
    functions: BTreeMap<Key, usize>,
}
impl Emitter<'_> {
    fn decode(&self, ty: &Type, value: &str) -> String {
        let body = match ty {
            Type::Primitive(t) => format!("<{t} as Scalar>::from_json(v,cx)"),
            Type::Named(k) => format!("decode_{}(v,cx,d+1)", self.functions[k]),
            Type::Nullable(t) => format!(
                "match v {{ Nullable::Null=>Ok(Nullable::Null),v=>Ok(Nullable::Value({}?)) }}",
                self.decode(t, "v")
            ),
            Type::Boxed(t) => format!("Ok(Box::new({}?))", self.decode(t, "v")),
            Type::Vec(t) => format!(
                "match v {{ Nullable::Value(JsonNonNullValue::Array(items))=>Ok(items.into_iter().enumerate().map(|(i,v)| cx.at_index(i,|cx| {})).collect::<Result<_,CodecError>>()?),_=>Err(cx.error(\"expected array\")) }}",
                self.decode(t, "v")
            ),
            Type::Map(t) => format!(
                "match v {{ Nullable::Value(JsonNonNullValue::Object(items))=>Ok(items.into_iter().map(|(k,v)| {{let value=cx.at(&k,|cx| {})?; Ok((k,value))}}).collect::<Result<_,CodecError>>()?),_=>Err(cx.error(\"expected object\")) }}",
                self.decode(t, "v")
            ),
            Type::Optional(_) | Type::Presence(_) => unreachable!("presence only occurs on fields"),
        };
        format!(
            "{{ let v={value}; let d=d+1; (|| -> Result<_,CodecError> {{ cx.step(d)?; {body} }})() }}"
        )
    }
    fn encode(&self, ty: &Type, value: &str) -> String {
        let body = match ty {
            Type::Primitive(_) => "Scalar::to_json(v,cx,d)".into(),
            Type::Named(k) => format!("encode_{}(v,cx,d+1)", self.functions[k]),
            Type::Nullable(t) => format!(
                "match v {{ Nullable::Null=>Ok(Nullable::Null),Nullable::Value(v)=>{} }}",
                self.encode(t, "v")
            ),
            Type::Boxed(t) => self.encode(t, "v.as_ref()"),
            Type::Vec(t) => format!(
                "Ok(Nullable::Value(JsonNonNullValue::Array(v.iter().enumerate().map(|(i,v)| cx.at_index(i,|cx| {})).collect::<Result<_,CodecError>>()?)))",
                self.encode(t, "v")
            ),
            Type::Map(t) => format!(
                "Ok(Nullable::Value(JsonNonNullValue::Object(v.iter().map(|(k,v)| {{cx.spend(k.len())?; Ok((k.clone(),cx.at(k,|cx| {})?))}}).collect::<Result<_,CodecError>>()?)))",
                self.encode(t, "v")
            ),
            Type::Optional(_) | Type::Presence(_) => unreachable!("presence only occurs on fields"),
        };
        format!(
            "{{ let v={value}; let d=d+1; (|| -> Result<_,CodecError> {{ cx.step(d)?; {body} }})() }}"
        )
    }
    fn decode_decl(&self, decl: &Decl, name: &str, index: usize, node: usize) -> String {
        match decl {
            Decl::Alias(t) => self.decode(t, "v"),
            Decl::Literals(values) => {
                let mut s = String::new();
                for (literal_index, lit) in values.iter().enumerate() {
                    s.push_str(&format!(
                        "if cx.literal({node},&v,&LITERAL_{index}_{literal_index})? {{ return Ok(crate::models::{name}::{}); }}",
                        lit.name
                    ));
                }
                s.push_str("Err(cx.error(\"no matching literal representation\"))");
                s
            }
            Decl::Enum(variants) => {
                let mut s = String::new();
                for variant in variants {
                    s.push_str(&format!(
                        "if cx.member({},&v)? {{ return Ok(crate::models::{name}::{}({}?)); }}",
                        root(self.indices, &variant.source),
                        variant.name,
                        self.decode(&variant.ty, "v")
                    ));
                }
                s.push_str("Err(cx.error(\"no matching union representation\"))");
                s
            }
            Decl::Struct { fields, extras } => {
                let mutable = if fields.is_empty() { "" } else { "mut " };
                let mut s = format!(
                    "let Nullable::Value(JsonNonNullValue::Object({mutable}fields))=v else {{return Err(cx.error(\"expected object\"));}};"
                );
                if fields.is_empty() && extras.is_none() {
                    s.push_str("drop(fields);");
                }
                for (i, f) in fields.iter().enumerate() {
                    let value=match &f.ty {
                        Type::Optional(t)=>format!("match fields.remove({:?}) {{ None=>None,Some(v)=>Some({}?) }}",f.wire,self.decode(t,"v")),
                        Type::Presence(t)=>format!("match fields.remove({:?}) {{ None=>Presence::Absent,Some(Nullable::Null)=>Presence::Null,Some(v)=>Presence::Value({}?) }}",f.wire,self.decode(t,"v")),
                        t=>format!("{}?",self.decode(t,&format!("fields.remove({:?}).ok_or_else(||cx.error(\"missing required field\"))?",f.wire))),
                    };
                    s.push_str(&format!(
                        "let field_{i}=cx.at({:?},|cx| {{ Ok({value}) }})?;",
                        f.wire
                    ));
                }
                if let Some((_, t)) = extras {
                    s.push_str(&format!("let extras=fields.into_iter().map(|(k,v)|{{let value=cx.at(&k,|cx| {})?;Ok((k,value))}}).collect::<Result<_,CodecError>>()?;",self.decode(t,"v")));
                }
                s.push_str(&format!("Ok(crate::models::{name} {{"));
                for (i, f) in fields.iter().enumerate() {
                    s.push_str(&format!("{}:field_{i},", f.name));
                }
                match extras {
                    Some((n, _)) => s.push_str(&format!("{n}:extras,")),
                    None => s.push_str("_construction:(),"),
                }
                s.push_str("})");
                s
            }
        }
    }
    fn encode_decl(&self, decl: &Decl, name: &str, index: usize) -> String {
        match decl {
            Decl::Alias(t) => self.encode(t, "v"),
            Decl::Literals(values) => {
                if values.is_empty() {
                    return "match *v {}".into();
                }
                let mut s = String::from("match v {");
                for (literal_index, lit) in values.iter().enumerate() {
                    s.push_str(&format!(
                        "crate::models::{name}::{}=>copy_json(&LITERAL_{index}_{literal_index},cx,d),",
                        lit.name
                    ));
                }
                s.push('}');
                s
            }
            Decl::Enum(variants) => {
                if variants.is_empty() {
                    return "match *v {}".into();
                }
                let mut s = String::from("match v {");
                for variant in variants {
                    s.push_str(&format!("crate::models::{name}::{}(v)=>{{let wire={}?;cx.check({},&wire)?;Ok(wire)}},",variant.name,self.encode(&variant.ty,"v"),root(self.indices,&variant.source)));
                }
                s.push('}');
                s
            }
            Decl::Struct { fields, extras } => {
                let mutable = if fields.is_empty() && extras.is_none() {
                    ""
                } else {
                    "mut "
                };
                let mut s = format!("let {mutable}fields=std::collections::BTreeMap::new();");
                for f in fields {
                    let insert = |t: &Type, v: &str| {
                        format!(
                            "cx.spend({})?;fields.insert({:?}.into(),cx.at({:?},|cx| {})?);",
                            f.wire.len(),
                            f.wire,
                            f.wire,
                            self.encode(t, v)
                        )
                    };
                    s.push_str(&match &f.ty {
                        Type::Optional(t)=>format!("if let Some(value)=&v.{} {{{}}}",f.name,insert(t,"value")),
                        Type::Presence(t)=>format!("match &v.{} {{Presence::Absent=>{{}},Presence::Null=>{{cx.spend({})?;fields.insert({:?}.into(),Nullable::Null);}},Presence::Value(value)=>{{{}}}}}",f.name,f.wire.len(),f.wire,insert(t,"value")),
                        t=>insert(t,&format!("&v.{}",f.name)),
                    });
                }
                if let Some((n, t)) = extras {
                    s.push_str(&format!("for (key,value) in &v.{n} {{cx.spend(key.len())?;fields.insert(key.clone(),cx.at(key,|cx| {})?);}}",self.encode(t,"value")));
                }
                s.push_str("Ok(Nullable::Value(JsonNonNullValue::Object(fields)))");
                s
            }
        }
    }
}
