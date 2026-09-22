//! Canonical Contract → Rust models/docs → native package consumers.

use serde_json::{Value, json};
use std::{path::Path, process::Command, sync::Arc};
use suspect_codegen::rust_models::{ModelPlan, plan_models};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn contract(path: &Path) -> Contract {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap()
}
fn with_spec(schemas: Value, test: impl FnOnce(&Contract)) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.json");
    std::fs::write(&path,serde_json::to_vec(&json!({"openapi":"3.1.0","info":{"title":"Rust models","version":"1"},"paths":{},"components":{"schemas":schemas}})).unwrap()).unwrap();
    test(&contract(&path));
}
fn roots(contract: &Contract, names: &[&str]) -> Vec<SchemaId> {
    contract
        .schema_roots()
        .iter()
        .filter(|id| {
            names
                .iter()
                .any(|name| id.pointer() == format!("/components/schemas/{name}"))
        })
        .cloned()
        .collect()
}
fn native(plan: &ModelPlan, consumer: &str) {
    static NATIVE_CACHE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _cache = NATIVE_CACHE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    assert!(!plan.has_errors(), "{:?}", plan.diagnostics());
    assert!(!plan.release_ready());
    let dir = tempfile::tempdir().unwrap();
    let files = plan.render().unwrap();
    suspect_codegen::write_files(&files, dir.path()).unwrap();
    let package = dir.path().join("rust");
    // The consumer is a library so its compile_fail examples are real doctests.
    let consumer_package = dir.path().join("consumer");
    std::fs::create_dir_all(consumer_package.join("src")).unwrap();
    std::fs::write(consumer_package.join("Cargo.toml"),"[package]\nname=\"model-consumer\"\nversion=\"0.0.0\"\nedition=\"2024\"\n[dependencies]\ngenerated-models={path=\"../rust\"}\n[workspace]\n").unwrap();
    std::fs::write(
        consumer_package.join("src/lib.rs"),
        format!("#![allow(unused_imports)]\n{consumer}"),
    )
    .unwrap();
    let target = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/native-rust-models");
    for (mode, use_consumer, doc_tests) in [
        ("test", true, false),
        ("test", false, true),
        ("doc", false, false),
    ] {
        let mut command = Command::new("cargo");
        let manifest = if use_consumer {
            consumer_package.join("Cargo.toml")
        } else {
            package.join("Cargo.toml")
        };
        command
            .args([mode, "--offline", "--quiet", "--manifest-path"])
            .arg(manifest)
            .arg("--target-dir")
            .arg(&target)
            .env("RUSTFLAGS", "-D warnings");
        if doc_tests {
            command.arg("--doc");
        }
        if mode == "doc" {
            command
                .arg("--no-deps")
                .env("RUSTDOCFLAGS", "-D rustdoc::broken_intra_doc_links");
        }
        let output = command.output().expect("native Rust tests require Cargo");
        assert!(
            output.status.success(),
            "cargo {mode}:\n{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
#[ignore = "requires native Cargo"]
fn presence_null_boolean_schemas_recursive_layout_and_extra_fields_remain_distinct() {
    with_spec(
        json!({
            "Anything":true,"Nothing":false,
            "Node":{"type":"object","required":["name","nullable"],"properties":{
                "name":{"type":"string"},"nullable":{"type":["string","null"]},
                "optional":{"type":"string"},"optional_nullable":{"type":["string","null"]},
                "next":{"$ref":"#/components/schemas/Node"},"additional_properties":{"type":"string"},
                "01ai":{"type":"string"},"type":{"type":"string"}
            }}
        }),
        |contract| {
            native(
                &plan_models(contract, contract.schema_roots()),
                r#"
use generated_models::{models::*,Nullable,Presence,JsonNonNullValue};
#[test] fn fields() {
 let mut node=Node::new("name".into(),Nullable::Null);
 assert!(matches!(node.optional,None));assert!(matches!(node.optional_nullable,Presence::Absent));
 node.optional_nullable=Presence::Null;node.optional=Some("present".into());
 node.next=Some(Box::new(Node::new("child".into(),Nullable::Value("value".into()))));
 node._01ai=Some("wire".into());node.type_=Some("kind".into());
 assert!(node.insert_extra("name".into(),Nullable::Value(JsonNonNullValue::String("shadow".into()))).is_err());
 node.insert_extra("new_wire_key".into(),Nullable::Null).unwrap();assert_eq!(node.extra_fields().count(),1);
 let _: Anything=Nullable::Null;
}
/// ```compile_fail
/// use generated_models::{models::Node,Nullable};
/// let node=Node::new("name".into()); // required nullable field cannot be omitted
/// ```
/// ```compile_fail
/// use generated_models::{models::Node,Nullable};
/// let mut node=Node::new("name".into(),Nullable::Null);
/// node.optional=Nullable::Null; // optional non-null is not nullable
/// ```
/// ```compile_fail
/// let impossible: generated_models::models::Nothing = ();
/// ```
/// ```compile_fail
/// use generated_models::{models::Node,Nullable};
/// let mut node=Node::new("name".into(),Nullable::Null);
/// node._extra_fields.insert("name".into(),Nullable::Null);
/// ```
pub struct NegativeConsumers;
"#,
            )
        },
    );
}

#[test]
#[ignore = "requires native Cargo"]
fn exact_numeric_helpers_accept_integral_exponents_without_float_rounding() {
    with_spec(
        json!({"Count":{"type":"integer"},"Ratio":{"type":"number"},"Small":{"type":"integer","minimum":0,"maximum":255},"Wide":{"type":"integer","minimum":0,"maximum":18446744073709551615_u64}}),
        |contract| {
            native(
                &plan_models(contract, contract.schema_roots()),
                r#"
use generated_models::{models::*,JsonInteger,JsonNumber,JsonValue,JsonNonNullValue,Nullable};
#[test] fn exact_numbers() {
 let huge:Count="1e400".parse().unwrap();assert_eq!(huge.as_str(),"1e400");assert_eq!(huge.to_i128(),None);
 assert!("1e-400".parse::<JsonInteger>().is_err());assert_eq!("100e-2".parse::<JsonInteger>().unwrap().to_i128(),Some(1));
 assert_eq!("0e-99999999999999999999999999999".parse::<JsonInteger>().unwrap().to_i128(),Some(0));
 let adjacent:Count="9007199254740993".parse().unwrap();assert_eq!(adjacent.to_i128(),Some(9007199254740993));
 let tiny:Ratio="1e-400".parse().unwrap();assert_eq!(tiny.as_str(),"1e-400");
 let _:Small=255u8;let _:Wide=u64::MAX;
 assert!("NaN".parse::<JsonNumber>().is_err());
}
#[test] fn json_grammar_and_checked_conversions() {
 for token in ["0","-0","-0.0","1","0.1","1.0","1E+2","1e-400","1e99999999999999999999999999999999999999999999999999999999999"] {
   let number:JsonNumber=token.parse().unwrap();assert_eq!(number.as_str(),token);
 }
 for token in [""," ","1 "," 1","+1","01","-01",".1","1.","1e","1e+","--1","1_0","NaN","Infinity","١","1e 2","\0","0x1"] {
   assert!(token.parse::<JsonNumber>().is_err(),"accepted {token:?}");
 }
 for (token,expected) in [("-0",0),("-0.0",0),("-0e999999999999999999999999999999999999999999999999",0),("0e-999999999999999999999999999999999999999999999999",0),("1.2300e2",123),("-0.00100e3",-1),("100e-2",1)] {
   let number:JsonInteger=token.parse().unwrap();assert_eq!(number.to_i128(),Some(expected));assert_eq!(number.as_str(),token);
 }
 for token in ["1.1","1e-400","123.450e1","1e-999999999999999999999999999999999999999999999999"] {assert!(token.parse::<JsonInteger>().is_err());}
 assert!("1e999999999999999999999999999999999999999999999999".parse::<JsonInteger>().unwrap().to_u128().is_none());
 assert_eq!("170141183460469231731687303715884105727".parse::<JsonInteger>().unwrap().to_i128(),Some(i128::MAX));
 assert_eq!("-170141183460469231731687303715884105728".parse::<JsonInteger>().unwrap().to_i128(),Some(i128::MIN));
 assert_eq!("170141183460469231731687303715884105728".parse::<JsonInteger>().unwrap().to_i128(),None);
 assert_eq!("-170141183460469231731687303715884105729".parse::<JsonInteger>().unwrap().to_i128(),None);
 assert_eq!("340282366920938463463374607431768211455".parse::<JsonInteger>().unwrap().to_u128(),Some(u128::MAX));
 assert_eq!("340282366920938463463374607431768211456".parse::<JsonInteger>().unwrap().to_u128(),None);
 assert_eq!("-1".parse::<JsonInteger>().unwrap().to_u128(),None);
 assert_eq!("-0.000e-9000".parse::<JsonInteger>().unwrap().to_u128(),Some(0));
 let value:JsonValue=Nullable::Value(JsonNonNullValue::Object(std::collections::BTreeMap::from([("exact".into(),Nullable::Value(JsonNonNullValue::Number("1e-400".parse().unwrap())))])));
 let Nullable::Value(JsonNonNullValue::Object(fields))=value else {panic!("object")};
 let Some(Nullable::Value(JsonNonNullValue::Number(number)))=fields.get("exact") else {panic!("number")};assert_eq!(number.as_str(),"1e-400");
}
/// ```compile_fail
/// let value: generated_models::models::Count = 9007199254740993.0f64;
/// ```
/// ```compile_fail
/// let value=generated_models::JsonNumber("NaN".into());
/// ```
pub struct NoNarrowing;
"#,
            )
        },
    );
}

#[test]
fn alias_cycles_block_emission_including_collection_aliases() {
    for schemas in [
        json!({"A":{"$ref":"#/components/schemas/B"},"B":{"$ref":"#/components/schemas/A"}}),
        json!({"A":{"type":"array","items":{"$ref":"#/components/schemas/A"}}}),
    ] {
        with_spec(schemas, |contract| {
            let plan = plan_models(contract, contract.schema_roots());
            assert!(plan.has_errors());
            assert!(plan.render().is_err());
            assert!(
                plan.diagnostics()
                    .iter()
                    .any(|d| d.code == "recursive-type-alias")
            );
        });
    }
}

#[test]
#[ignore = "requires native Cargo"]
fn mutually_recursive_models_box_layout_edges_but_not_collections() {
    with_spec(
        json!({
            "A":{"type":"object","required":["b"],"properties":{"b":{"$ref":"#/components/schemas/B"}},"additionalProperties":false},
            "B":{"type":"object","required":["list"],"properties":{"list":{"type":"array","items":{"$ref":"#/components/schemas/A"}},"a":{"$ref":"#/components/schemas/A"}},"additionalProperties":false},
            "Null":{"type":"null"},"EmptyEnum":{"enum":[]},
            "Maybe":{"type":["null","string"],"enum":[null,"a"]}
        }),
        |contract| {
            native(
                &plan_models(contract, contract.schema_roots()),
                r#"
use generated_models::{models::*,Nullable};
#[test] fn native_shapes() {
 let mut b=B::new(vec![A::new(B::new(vec![]))]);
 b.a=Some(Box::new(A::new(B::new(vec![]))));
 let _:Null=Nullable::Null;let _:Maybe=Nullable::Value(MaybeValue::A);
}
/// ```compile_fail
/// let value:generated_models::models::EmptyEnum=();
/// ```
pub struct EmptyHasNoValues;
"#,
            )
        },
    );
}

#[test]
#[ignore = "requires native Cargo"]
fn names_wire_keys_and_untrusted_documentation_compile_without_reinterpretation() {
    with_spec(
        json!({
            "Names":{"type":"object","description":"    panic!(\"do not execute OpenAPI prose\");\n```rust\ncompile_error!(\"prose\");\n``` [not_a_rust_link] <script>","properties":{
                "a b":{"type":"string"},"a-b":{"type":"string"},"\"\\\n":{"type":"string"},"é":{"type":"string"},"_extra_fields":{"type":"string"}
            }},
            "A B":{"type":"string"},"A-B":{"type":"integer"},"String":{"type":"string"},"Self":{"type":"string"},
            "Tags":{"enum":["A B","A-B","quote\"slash\\\n","é"]}
        }),
        |contract| {
            let plan = plan_models(contract, contract.schema_roots());
            native(
                &plan,
                r#"
use generated_models::{models::Names,Nullable};
#[test] fn exact_extra_key_rules() {
 let mut model=Names::new();model.a_b=Some("first".into());model.a_b_2=Some("second".into());
 for key in ["a b","a-b","\"\\\n","é","_extra_fields"] {assert!(model.insert_extra(key.into(),Nullable::Null).is_err(),"accepted declared {key:?}");}
 model.insert_extra("unknown".into(),Nullable::Null).unwrap();assert_eq!(model.extra_fields().count(),1);
}
"#,
            );
        },
    );
}

#[test]
fn unsupported_shapes_are_errors_and_never_dynamic_fallbacks() {
    with_spec(
        json!({
            "Untyped":{"properties":{"name":{"type":"string"}}},
            "Intersection":{"allOf":[{"type":"string"},{"minLength":1}]},
            "NumericIntersection":{"const":1,"enum":[1.0]}
        }),
        |contract| {
            for name in ["Untyped", "Intersection", "NumericIntersection"] {
                let plan = plan_models(contract, &roots(contract, &[name]));
                assert!(plan.has_errors(), "{name}");
                assert!(plan.render().is_err());
                assert!(plan.diagnostics().iter().any(|diagnostic| {
                    diagnostic
                        .source
                        .pointer()
                        .starts_with(&format!("/components/schemas/{name}"))
                }));
            }
        },
    );
}

#[test]
#[ignore = "requires native Cargo and the pinned tracked OpenRouter checkout"]
fn tracked_openrouter_nullable_caller_and_image_sources_preserve_every_payload() {
    let path = std::env::var_os("SUSPECT_OPENROUTER_YAML")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../openrouter-web/projects/docs/openapi/openapi.yaml")
        });
    let contract = contract(&path);
    let selected = roots(
        &contract,
        &["ORAnthropicNullableCaller", "AnthropicImageBlockParam"],
    );
    assert_eq!(selected.len(), 2);
    native(
        &plan_models(&contract, &selected),
        r#"
use generated_models::{models::*,Nullable};
#[test] fn branches() {
 let null:OrAnthropicNullableCaller=Nullable::Null;
 let direct:OrAnthropicNullableCaller=Nullable::Value(OrAnthropicNullableCallerValue::Direct(AnthropicDirectCaller::new()));
 let code=OrAnthropicNullableCallerValue::CodeExecution20260120(AnthropicCodeExecution20260120Caller::new("tool".into()));
 match code {OrAnthropicNullableCallerValue::CodeExecution20260120(value)=>assert_eq!(value.tool_id,"tool"),_=>panic!("wrong branch")};
 let source=AnthropicImageBlockParamSource::Url(AnthropicUrlImageSource::new("https://example.test/image".into()));
 let image=AnthropicImageBlockParam::new(source);assert_eq!(image.type_.wire_json(),"\"image\"");
 let base64=AnthropicBase64ImageSource::new("abc".into(),AnthropicImageMimeType::ImagePng);
 let _:AnthropicImageBlockParamSource=AnthropicImageBlockParamSource::Base64(base64);
 assert!(matches!(null,Nullable::Null));assert!(matches!(direct,Nullable::Value(_)));
}
/// ```compile_fail
/// use generated_models::models::*;
/// let bad=AnthropicBase64ImageSource::new("url".into());
/// ```
pub struct MissingImageFields;
"#,
    );
}
