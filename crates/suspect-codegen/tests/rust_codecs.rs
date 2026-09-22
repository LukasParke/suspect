//! Consumer behavior through Contract -> complete native Rust codec package.
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::rust_codecs::{CodecConfig, CodecPlan, plan_codecs};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;
fn contract(path: &Path) -> Arc<Contract> {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap())
}
fn fixture(schemas: Value, f: impl FnOnce(Arc<Contract>)) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.json");
    std::fs::write(&path,serde_json::to_vec(&json!({"openapi":"3.1.0","info":{"title":"Codec contract","version":"1"},"paths":{},"components":{"schemas":schemas}})).unwrap()).unwrap();
    f(contract(&path));
}
fn native(plan: &CodecPlan, source: &str) {
    // Cargo's lock does not span Rustdoc's child compilers; same-named fixture
    // crates must not replace each other's cached rlibs during doctests.
    static NATIVE_CACHE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _cache = NATIVE_CACHE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let dir = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&plan.render(), dir.path()).unwrap();
    let consumer = dir.path().join("consumer");
    std::fs::create_dir_all(consumer.join("src")).unwrap();
    std::fs::write(consumer.join("Cargo.toml"),"[package]\nname=\"codec-consumer\"\nversion=\"0.0.0\"\nedition=\"2024\"\n[workspace]\n[dependencies]\ngenerated-models={path=\"../rust\"}\n").unwrap();
    std::fs::write(
        consumer.join("src/lib.rs"),
        format!("#![cfg(test)]\n{source}"),
    )
    .unwrap();
    let generated = dir.path().join("rust");
    for (mode, package, extra) in [
        ("test", consumer.as_path(), None),
        ("test", generated.as_path(), Some("--doc")),
        ("doc", generated.as_path(), Some("--no-deps")),
    ] {
        let mut command = Command::new("cargo");
        command
            .args([mode, "--offline", "--quiet", "--manifest-path"])
            .arg(package.join("Cargo.toml"))
            .arg("--target-dir")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/native-rust-codecs"))
            .env("RUSTFLAGS", "-D warnings")
            .env("RUSTDOCFLAGS", "-D warnings");
        if let Some(extra) = extra {
            command.arg(extra);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
#[test]
#[ignore = "requires native Cargo"]
fn exact_native_models_validate_mutations_unions_presence_and_recursion() {
    fixture(
        json!({
            "Node":{"type":"object","required":["name","nullable"],"properties":{"name":{"type":"string","minLength":1},"nullable":{"type":["string","null"]},"optional":{"type":"string"},"maybe":{"type":["string","null"]},"next":{"$ref":"#/components/schemas/Node"}},"additionalProperties":{"type":"integer"}},
            "Integer":{"type":"integer"},"Small":{"type":"integer","minimum":0,"maximum":255},"Number":{"type":"number"},"Literal":{"enum":[1]},"Anything":true,"Nothing":false,
            "Inclusive":{"anyOf":[{"type":"integer"},{"type":"number"}]},"Exclusive":{"oneOf":[{"type":"integer"},{"type":"number"}]},
            "Word":{"anyOf":[{"type":"string","minLength":3},{"type":"string"}]}
        }),
        |contract| {
            let plan = plan_codecs(
                contract.clone(),
                contract.schema_roots(),
                CodecConfig::default(),
            )
            .unwrap();
            native(
                &plan,
                r##"
use generated_models::{codecs::*,models::*,Nullable,Presence};
#[test] fn round_trip_and_mutation(){
 let mut node=NodeCodec::decode(r#"{"name":"root","nullable":null,"next":{"name":"child","nullable":"yes"},"extra":9007199254740993}"#).unwrap();
 assert_eq!(node.name,"root");assert!(matches!(node.maybe,Presence::Absent));assert!(node.optional.is_none());assert_eq!(node.next.as_ref().unwrap().name,"child");
 node.maybe=Presence::Null;node.optional=Some("present".into());
 assert!(node.insert_extra("name".into(),"1".parse().unwrap()).is_err());
 let wire=NodeCodec::encode(&node).unwrap();let again=NodeCodec::decode(&wire).unwrap();assert!(matches!(again.maybe,Presence::Null));assert_eq!(again.optional.as_deref(),Some("present"));assert_eq!(again.extra_fields().next().unwrap().1.as_str(),"9007199254740993");
 node.name.clear();assert!(matches!(NodeCodec::encode(&node),Err(CodecError::Invalid(_))));
 assert!(matches!(NodeCodec::decode(r#"{"name":"x","nullable":null,"optional":null}"#),Err(CodecError::Invalid(_))));
 assert!(matches!(NodeCodec::decode(r#"{"name":"x","nullable":null,"extra":true}"#),Err(CodecError::Invalid(_))));
}
#[test] fn numbers_and_unions(){
 assert_eq!(IntegerCodec::encode(&IntegerCodec::decode("1e400").unwrap()).unwrap(),"1e400");
 assert_eq!(SmallCodec::decode("100e-2").unwrap(),1u8);assert!(SmallCodec::decode("256").is_err());
 assert_eq!(NumberCodec::encode(&NumberCodec::decode("-0.00e+50").unwrap()).unwrap(),"-0.00e+50");
 assert_eq!(LiteralCodec::encode(&LiteralCodec::decode("1.0").unwrap()).unwrap(),"1");
 assert!(matches!(InclusiveCodec::decode("1").unwrap(),Inclusive::Variant1(_)));
 assert!(matches!(ExclusiveCodec::decode("1"),Err(CodecError::Invalid(_))));
 assert!(matches!(WordCodec::encode(&Word::Variant1("x".into())),Err(CodecError::Invalid(_))));
 assert_eq!(WordCodec::encode(&Word::Variant2("x".into())).unwrap(),"\"x\"");
 assert!(matches!(AnythingCodec::decode("null").unwrap(),Nullable::Null));assert!(matches!(NothingCodec::decode("null"),Err(CodecError::Invalid(_))));
}
"##,
            );
        },
    );
}
#[test]
fn unsupported_intersections_fail_before_artifacts_and_keep_source() {
    fixture(
        json!({"Base":{"type":"object","properties":{"value":{"type":"string"}}},"Bad":{"allOf":[{"$ref":"#/components/schemas/Base"},{"required":["undeclared"]}]}}),
        |contract| {
            let errors = plan_codecs(
                contract.clone(),
                contract.schema_roots(),
                CodecConfig::default(),
            )
            .unwrap_err();
            assert!(
                errors
                    .iter()
                    .any(|d| d.source.pointer() == "/components/schemas/Bad/allOf")
            );
        },
    );
}
#[test]
#[ignore = "requires native Cargo"]
fn finite_conversion_and_evaluation_failures_are_not_schema_mismatches() {
    fixture(json!({"Text":{"type":"string"}}), |contract| {
        let plan = plan_codecs(
            contract.clone(),
            contract.schema_roots(),
            CodecConfig {
                max_conversion_steps: 2,
                ..CodecConfig::default()
            },
        )
        .unwrap();
        native(
            &plan,
            "#[test] fn bounded(){use generated_models::codecs::*;assert!(matches!(TextCodec::encode(&\"long\".into()),Err(CodecError::Conversion(_))));}",
        );
        let mut config = CodecConfig::default();
        config.schema.max_evaluation_steps = 0;
        let plan = plan_codecs(contract.clone(), contract.schema_roots(), config).unwrap();
        native(
            &plan,
            "#[test] fn bounded(){use generated_models::codecs::*;assert!(matches!(TextCodec::decode(\"\\\"x\\\"\"),Err(CodecError::EvaluationFailure(_))));}",
        );
    });
}
fn corpus() -> Arc<Contract> {
    let path = std::env::var_os("SUSPECT_OPENROUTER_YAML")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../openrouter-web/projects/docs/openapi/openapi.yaml")
        });
    contract(&path)
}
fn select(contract: &Contract, pointers: &[&str]) -> Vec<SchemaId> {
    pointers
        .iter()
        .map(|p| {
            p.trim_start_matches('/').split('/').fold(
                SchemaId::new(contract.entry().clone(), Default::default()),
                |parent, token| parent.child(&token.replace("~1", "/").replace("~0", "~")),
            )
        })
        .collect()
}
#[test]
#[ignore = "requires native Cargo and tracked OpenRouter corpus"]
fn tracked_caller_image_integer_and_typed_extras_are_native_codecs() {
    let contract = corpus();
    let roots = select(
        &contract,
        &[
            "/components/schemas/ORAnthropicNullableCaller",
            "/components/schemas/AnthropicImageBlockParam",
            "/components/schemas/ChatChoice/properties/index",
            "/components/schemas/ImageGenerationServerToolConfig",
        ],
    );
    let plan = plan_codecs(contract, &roots, CodecConfig::default()).unwrap();
    let index = plan
        .models()
        .symbols()
        .iter()
        .find(|s| s.source() == &roots[2])
        .unwrap()
        .name();
    native(&plan,&r##"
use generated_models::{models::*,codecs::*,Nullable};
#[test] fn corpus(){
 assert!(matches!(OrAnthropicNullableCallerCodec::decode("null").unwrap(),Nullable::Null));
 let direct=OrAnthropicNullableCallerCodec::decode(r#"{"type":"direct"}"#).unwrap();assert!(matches!(direct,Nullable::Value(OrAnthropicNullableCallerValue::Direct(_))));
 let mut caller=AnthropicCodeExecution20260120Caller::new("tool".into());caller.tool_id="changed".into();let value=Nullable::Value(OrAnthropicNullableCallerValue::CodeExecution20260120(caller));let wire=OrAnthropicNullableCallerCodec::encode(&value).unwrap();assert!(wire.contains("changed"));
 let mut image=AnthropicImageBlockParamCodec::decode(r#"{"type":"image","source":{"type":"url","url":"https://example.test/a.png"}}"#).unwrap();image.source=AnthropicImageBlockParamSource::Base64(AnthropicBase64ImageSource::new("YWJj".into(),AnthropicImageMimeType::ImagePng));let wire=AnthropicImageBlockParamCodec::encode(&image).unwrap();let image=AnthropicImageBlockParamCodec::decode(&wire).unwrap();assert!(matches!(image.source,AnthropicImageBlockParamSource::Base64(_)));
 assert_eq!($INDEXCodec::encode(&$INDEXCodec::decode("9007199254740993").unwrap()).unwrap(),"9007199254740993");
 let mut config=ImageGenerationServerToolConfigCodec::decode(r#"{"model":"image-model","quality":"high","weight":1e-400,"sizes":[1,null]}"#).unwrap();assert_eq!(config.model.as_deref(),Some("image-model"));assert_eq!(config.extra_fields().count(),3);
 config.model=Some("changed".into());let wire=ImageGenerationServerToolConfigCodec::encode(&config).unwrap();assert_eq!(ImageGenerationServerToolConfigCodec::decode(&wire).unwrap().model.as_deref(),Some("changed"));assert!(matches!(ImageGenerationServerToolConfigCodec::decode(r#"{"model":42}"#),Err(CodecError::Invalid(_))));assert!(matches!(ImageGenerationServerToolConfigCodec::decode(r#"{"unknown":true}"#),Err(CodecError::Invalid(_))));
}
"##.replace("$INDEX",index));
}
#[test]
#[ignore = "requires native Cargo and tracked OpenRouter corpus"]
fn tracked_chat_result_transparent_intersections_preserve_native_fields() {
    let contract = corpus();
    let roots = select(&contract, &["/components/schemas/ChatResult"]);
    let plan = plan_codecs(contract, &roots, CodecConfig::default()).unwrap();
    native(
        &plan,
        r##"
use generated_models::codecs::*;
#[test] fn result(){let mut result=ChatResultCodec::decode(r#"{"id":"test","object":"chat.completion","created":9007199254740993,"model":"test","choices":[{"finish_reason":"stop","index":0,"message":{"role":"assistant","content":"hello"}}],"system_fingerprint":null}"#).unwrap();assert_eq!(result.created.as_str(),"9007199254740993");result.id="mutated".into();let wire=ChatResultCodec::encode(&result).unwrap();assert_eq!(ChatResultCodec::decode(&wire).unwrap().id,"mutated");}
"##,
    );
}

#[test]
#[ignore = "requires native Cargo and tracked OpenRouter corpus"]
fn tracked_request_intersections_keep_named_fields_and_enforce_presence() {
    let contract = corpus();
    let roots = select(&contract, &["/components/schemas/ChatRequest"]);
    let plan = plan_codecs(contract, &roots, CodecConfig::default()).unwrap();
    native(
        &plan,
        r##"
use generated_models::{codecs::*,Presence};
#[test] fn request(){
 let mut request=ChatRequestCodec::decode(r#"{"messages":[{"role":"user","content":"hello"}],"plugins":[{"id":"web","user_location":{"type":"approximate","country":"US"}}]}"#).unwrap();
 assert_eq!(request.messages.len(),1);request.max_tokens=Presence::Value("9007199254740993".parse().unwrap());
 let wire=ChatRequestCodec::encode(&request).unwrap();let round=ChatRequestCodec::decode(&wire).unwrap();match round.max_tokens {Presence::Value(n)=>assert_eq!(n.as_str(),"9007199254740993"),_=>panic!("lost exact optional integer")};
 request.messages.clear();assert!(matches!(ChatRequestCodec::encode(&request),Err(CodecError::Invalid(_))));
 assert!(matches!(ChatRequestCodec::decode(r#"{"messages":[{"role":"user","content":"hello"}],"plugins":[{"id":"web","user_location":{"country":"US"}}]}"#),Err(CodecError::Invalid(_))));
}
"##,
    );
}

#[test]
#[ignore = "requires native Cargo"]
fn schema_literals_do_not_consume_transport_input_limits() {
    fixture(
        json!({"Choice": {"enum": ["oversized", "x"]}}),
        |contract| {
            let mut config = CodecConfig::default();
            config.json_limits.max_input_bytes = 3;
            let plan = plan_codecs(contract.clone(), contract.schema_roots(), config).unwrap();
            native(
                &plan,
                r#"
#[test]
fn source_literals_have_independent_admission() {
    use generated_models::{codecs::ChoiceCodec, models::Choice};
    let value = ChoiceCodec::decode("\"x\"").unwrap();
    assert_eq!(ChoiceCodec::encode(&value).unwrap(), "\"x\"");
    assert_eq!(ChoiceCodec::encode(&Choice::Oversized).unwrap(), "\"oversized\"");
}
"#,
            );
        },
    );
}

#[test]
#[ignore = "requires native Cargo"]
fn codec_documentation_supports_models_named_result() {
    fixture(json!({"Result": {"type": "string"}}), |contract| {
        let plan = plan_codecs(
            contract.clone(),
            contract.schema_roots(),
            CodecConfig::default(),
        )
        .unwrap();
        native(
            &plan,
            r#"
#[test]
fn result_is_an_ordinary_model_name() {
    let value = generated_models::codecs::ResultCodec::decode("\"native\"").unwrap();
    assert_eq!(value, "native");
}
"#,
        );
    });
}

#[test]
#[ignore = "requires native Cargo"]
fn nested_codec_evaluation_failures_keep_the_responsible_source_and_instance() {
    fixture(
        json!({"M": {"type": "object", "properties": {
            "a": {"anyOf": [{"type": "string"}, {"type": "boolean"}]},
            "b": {"enum": ["x"]}
        }}}),
        |contract| {
            let mut config = CodecConfig::default();
            config.schema.max_equality_steps = 1;
            let plan = plan_codecs(contract.clone(), contract.schema_roots(), config).unwrap();
            native(
                &plan,
                r##"
#[test]
fn a_prior_union_trial_cannot_steal_the_failure_location() {
    use generated_models::codecs::{CodecError, MCodec};
    let failure = match MCodec::decode(r#"{"a":"s","b":"x"}"#) {
        Err(CodecError::EvaluationFailure(failure)) => failure,
        other => panic!("expected exhausted equality budget, got {other:?}"),
    };
    assert!(failure.pointer.starts_with("/components/schemas/M/properties/b"), "{failure:?}");
    assert_eq!(failure.instance_path, "/b");
}
"##,
            );
        },
    );
}

#[test]
#[ignore = "requires native Cargo"]
fn folded_overlays_are_not_standalone_unless_independently_referenced() {
    fixture(
        json!({
            "Base": {"type": "object", "properties": {"x": {"type": "string"}}},
            "Strong": {"allOf": [
                {"$ref": "#/components/schemas/Base"},
                {"type": "object", "required": ["x"]}
            ]},
            "Separate": {"$ref": "#/components/schemas/Strong/allOf/1"}
        }),
        |contract| {
            let strong = select(&contract, &["/components/schemas/Strong"]);
            let plan = plan_codecs(contract.clone(), &strong, CodecConfig::default()).unwrap();
            native(
                &plan,
                r#"
#[test]
fn strengthening_changes_native_presence() {
    use generated_models::codecs::{CodecError, StrongCodec};
    let value = StrongCodec::decode("{\"x\":\"ok\"}").unwrap();
    assert_eq!(value.x, "ok");
    assert!(matches!(StrongCodec::decode("{}"), Err(CodecError::Invalid(_))));
}
"#,
            );
            let both = select(
                &contract,
                &["/components/schemas/Strong", "/components/schemas/Separate"],
            );
            let findings = plan_codecs(contract, &both, CodecConfig::default()).unwrap_err();
            assert!(findings.iter().any(|finding| {
                finding
                    .source
                    .pointer()
                    .starts_with("/components/schemas/Strong/allOf/1")
            }));
        },
    );
}

#[test]
#[ignore = "requires native Cargo"]
fn nested_encode_branch_failures_keep_array_and_escaped_extra_paths() {
    fixture(
        json!({"Batch":{"type":"object","required":["items"],"properties":{
            "items":{"type":"array","items":{"type":"object","additionalProperties":{
                "anyOf":[{"type":"string","minLength":2},{"type":"string"}]
            }}}
        }}}),
        |contract| {
            let plan = plan_codecs(
                contract.clone(),
                contract.schema_roots(),
                CodecConfig::default(),
            )
            .unwrap();
            let branch = plan
                .models()
                .symbols()
                .iter()
                .find(|symbol| {
                    symbol.source().pointer()
                        == "/components/schemas/Batch/properties/items/items/additionalProperties"
                })
                .unwrap()
                .name();
            native(&plan,&r##"
#[test]
fn nested_branch_validation_reports_the_original_wire_path() {
    use generated_models::{codecs::{BatchCodec,CodecError},models::$BRANCH};
    let mut batch=BatchCodec::decode(r#"{"items":[{"a/b~c":"long"}]}"#).unwrap();
    batch.items[0].insert("a/b~c".into(),$BRANCH::Variant1("x".into()));
    let findings=match BatchCodec::encode(&batch) {
        Err(CodecError::Invalid(findings))=>findings,
        other=>panic!("expected selected-branch failure, got {other:?}"),
    };
    assert!(findings.iter().any(|finding| finding.pointer == "/components/schemas/Batch/properties/items/items/additionalProperties/anyOf/0/minLength" && finding.instance_path == "/items/0/a~1b~0c"),"{findings:?}");
    batch.items[0].insert("a/b~c".into(),$BRANCH::Variant2("x".into()));
    let wire=BatchCodec::encode(&batch).unwrap();
    let decoded=BatchCodec::decode(&wire).unwrap();
    assert!(matches!(decoded.items[0].get("a/b~c"),Some($BRANCH::Variant2(value)) if value == "x"));
}
"##.replace("$BRANCH",branch));
        },
    );
}
