//! Native consumers exercise emitted programs against the independent owned evaluator.
use serde_json::{Value, json};
use std::{path::Path, process::Command, sync::Arc};
use suspect_codegen::rust_validation::emit;
use suspect_ir::contract::{Contract, SchemaId};
use suspect_ref::WorkspaceBuilder;
use suspect_schema::{
    Config, OwnedCompiler, OwnedOutcome, OwnedProgram, OwnedSchema, ProgramCheck,
    ProgramInstruction, ProgramNode, ProgramRoot, ProgramSource,
};
use suspect_source::Uri;

fn native_cargo() -> Command {
    let mut command = Command::new("cargo");
    if let Some(toolchain) = std::env::var_os("SUSPECT_NATIVE_RUST_TOOLCHAIN") {
        command.env("RUSTUP_TOOLCHAIN", toolchain);
    }
    command
}

fn compile(schemas: Value, config: Config) -> (OwnedSchema, Vec<SchemaId>) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(&path,json!({"openapi":"3.1.0","info":{"title":"Rust validation","version":"1"},"components":{"schemas":schemas}}).to_string()).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(directory.path())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap());
    let roots = contract.schema_roots().to_vec();
    (
        OwnedCompiler::new(config)
            .compile(contract, &roots)
            .unwrap(),
        roots,
    )
}
#[test]
#[ignore = "requires a native Rust toolchain"]
fn versioned_shared_runtime_vectors_use_independent_expected_results() {
    let vectors: Value =
        serde_json::from_str(include_str!("fixtures/runtime-contract-v1.json")).unwrap();
    let schemas = vectors["cases"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(index, case)| (format!("Case{index:02}"), case["schema"].clone()))
        .collect::<serde_json::Map<_, _>>();
    let (compiled, _) = compile(Value::Object(schemas), Config::default());
    let program = compiled.program();
    let mut assertions = String::new();
    for (index, case) in vectors["cases"].as_array().unwrap().iter().enumerate() {
        let node = program
            .roots
            .iter()
            .find(|root| root.source.pointer == format!("/components/schemas/Case{index:02}"))
            .unwrap()
            .target;
        for expected in ["valid", "invalid"] {
            for input in case[expected].as_array().unwrap() {
                let input = input.as_str().unwrap();
                let valid = expected == "valid";
                assertions.push_str(&format!("{{let value=generated::json::parse_json({input:?},Default::default()).unwrap();let outcome=generated::p0::validate({node},&value);match outcome{{generated::p0::ValidationOutcome::Valid=>assert!({valid},\"unexpected valid: {{}}\",{input:?}),generated::p0::ValidationOutcome::Invalid(_)=>assert!(!{valid},\"unexpected invalid: {{}}\",{input:?}),other=>panic!(\"incomplete evaluation: {{other:?}}\")}}}}\n"));
            }
        }
    }
    native(&[program], &assertions, "");
}

fn assertion(module: usize, compiled: &OwnedSchema, root: &SchemaId, input: &str) -> String {
    let program = compiled.program();
    let node = program
        .roots
        .iter()
        .find(|r| r.source.pointer == root.pointer())
        .unwrap()
        .target;
    let outcome = compiled.validate(root, &serde_json::from_str(input).unwrap());
    let expected = match outcome {
        OwnedOutcome::Valid => "ValidationOutcome::Valid => {}".into(),
        OwnedOutcome::Invalid(findings) => {
            let locations: Vec<_> = findings
                .iter()
                .map(|f| {
                    (
                        f.source.document().to_string(),
                        f.source.pointer().to_owned(),
                        f.instance_path.to_path(),
                    )
                })
                .collect();
            format!(
                "ValidationOutcome::Invalid(findings) => assert_eq!(findings.iter().map(|f|(f.document.as_str(),f.pointer.as_str(),f.instance_path.as_str())).collect::<Vec<_>>(),vec!{locations:?})"
            )
        }
        OwnedOutcome::EvaluationFailure(f) => {
            let location = (
                f.source.document().to_string(),
                f.source.pointer().to_owned(),
                f.instance_path.to_path(),
            );
            format!(
                "ValidationOutcome::EvaluationFailure(f) => assert_eq!((f.document.as_str(),f.pointer.as_str(),f.instance_path.as_str()),{location:?})"
            )
        }
    };
    format!(
        "{{ use generated::p{module}::{{validate,ValidationOutcome}}; let value = generated::json::parse_json({input:?},generated::json::JsonLimits::default()).unwrap(); match validate({node},&value) {{ {expected}, other => panic!(\"unexpected outcome for {{}}: {{other:?}}\",{input:?}) }} }}\n"
    )
}
fn cases(
    module: usize,
    compiled: &OwnedSchema,
    roots: &[SchemaId],
    values: &[(&str, &str)],
) -> String {
    values
        .iter()
        .map(|(name, input)| {
            let root = roots
                .iter()
                .find(|r| r.pointer() == format!("/components/schemas/{name}"))
                .unwrap();
            assertion(module, compiled, root, input)
        })
        .collect()
}
fn native(programs: &[OwnedProgram], assertions: &str, private_checks: &str) {
    let directory = tempfile::tempdir().unwrap();
    let package = directory.path().join("package");
    let consumer = directory.path().join("consumer");
    std::fs::create_dir_all(package.join("src")).unwrap();
    std::fs::create_dir_all(consumer.join("src")).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    std::fs::copy(
        source.join("rust_models/runtime.rs"),
        package.join("src/support.rs"),
    )
    .unwrap();
    std::fs::copy(
        source.join("rust_codecs/json_runtime.rs"),
        package.join("src/json.rs"),
    )
    .unwrap();
    let mut lib = String::from(
        "#![forbid(unsafe_code)]\nmod support;\npub use support::{ExtraFieldError,JsonInteger,JsonNonNullValue,JsonNumber,JsonValue,Never,Nullable,NumberError,Presence};\npub mod json;\n",
    );
    for (index, program) in programs.iter().enumerate() {
        for file in emit(program).unwrap() {
            let relative = file.path.strip_prefix("rust/src/").unwrap().replacen(
                "validation",
                &format!("p{index}"),
                1,
            );
            let path = package.join("src").join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, file.content).unwrap();
        }
        lib.push_str(&format!("pub mod p{index};\n"));
    }
    lib.push_str(private_checks);
    std::fs::write(package.join("src/lib.rs"), lib).unwrap();
    std::fs::write(package.join("Cargo.toml"),"[package]\nname='generated'\nversion='0.0.0'\nedition='2024'\nrust-version='1.88'\n[workspace]\n").unwrap();
    std::fs::write(consumer.join("Cargo.toml"),format!("[package]\nname='native-validation-consumer'\nversion='0.0.0'\nedition='2024'\n[workspace]\n[dependencies]\ngenerated={{path={package:?}}}\n")).unwrap();
    std::fs::write(
        consumer.join("src/main.rs"),
        format!("fn main() {{ {assertions} }}"),
    )
    .unwrap();
    let target = directory.path().join("target");
    for (manifest, args) in [
        (
            consumer.join("Cargo.toml"),
            vec!["run", "--offline", "--quiet"],
        ),
        (
            package.join("Cargo.toml"),
            vec!["test", "--offline", "--quiet"],
        ),
        (
            package.join("Cargo.toml"),
            vec!["doc", "--offline", "--no-deps"],
        ),
    ] {
        let output = native_cargo()
            .args(args)
            .arg("--manifest-path")
            .arg(manifest)
            .arg("--target-dir")
            .arg(&target)
            .env("RUSTFLAGS", "-D warnings")
            .env("RUSTDOCFLAGS", "-D warnings")
            .env_remove("RUST_MIN_STACK")
            .output()
            .unwrap();
        if !output.status.success() {
            let retained = directory.keep();
            panic!(
                "native validation failed; artifacts retained at {}\n{}{}",
                retained.display(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    let output = native_cargo()
        .args(["tree", "--offline", "--manifest-path"])
        .arg(package.join("Cargo.toml"))
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().lines().count(),
        1,
        "generated evaluator must have no dependencies"
    );
}

#[test]
fn mutated_programs_are_rejected_before_artifacts_with_source_locations() {
    let (schema, _) = compile(
        json!({"Root":{"type":"number","minimum":1},"Reference":{"$ref":"#/components/schemas/Root"},"Pattern":{"pattern":"^x+$"}}),
        Config::default(),
    );
    let program = schema.program();
    let mut malformed = program.clone();
    malformed.version = "unknown";
    assert!(emit(&malformed).unwrap_err().source.is_none());
    malformed = program.clone();
    malformed.profile = "unknown";
    assert!(emit(&malformed).unwrap_err().source.is_none());
    malformed = program.clone();
    malformed.roots[0].target = program.nodes.len();
    assert_eq!(
        emit(&malformed).unwrap_err().source,
        Some(malformed.roots[0].source.clone())
    );
    malformed = program.clone();
    let check = malformed
        .nodes
        .iter_mut()
        .flat_map(|n| &mut n.checks)
        .find(|c| matches!(c.instruction, ProgramInstruction::Bound { .. }))
        .unwrap();
    if let ProgramInstruction::Bound { value, .. } = &mut check.instruction {
        *value = "NaN".into();
    }
    let source = check.source.clone();
    assert_eq!(emit(&malformed).unwrap_err().source, Some(source));
    malformed = program.clone();
    let check = malformed
        .nodes
        .iter_mut()
        .flat_map(|n| &mut n.checks)
        .find(|c| matches!(c.instruction, ProgramInstruction::Pattern { .. }))
        .unwrap();
    if let ProgramInstruction::Pattern { program } = &mut check.instruction {
        program.start = program.states.len();
    }
    let source = check.source.clone();
    assert_eq!(emit(&malformed).unwrap_err().source, Some(source));
    malformed = program;
    malformed.limits.max_depth = usize::MAX;
    assert!(emit(&malformed).unwrap_err().source.is_some());
}

#[test]
#[ignore = "requires native Cargo"]
fn native_exact_numbers_applicators_patterns_and_locations_match_owned_schema() {
    let (compiled,roots) = compile(serde_json::from_str(r##"{
      "Integer":{"type":"integer"},
      "Range":{"minimum":9007199254740993,"exclusiveMaximum":9007199254740994},
      "Multiple":{"multipleOf":0.01},"Factors":{"multipleOf":0.000375},
      "HugeDivisor":{"multipleOf":3e999999999999999999999999},
      "Tiny":{"minimum":1e-999999999999999999999999,"maximum":1e-999999999999999999999999},
      "Enum":{"enum":[9007199254740993,1e-400,{"x":1.0,"__proto__":null}]},
      "Const":{"const":{"a/b~":[9007199254740993,1e-400]}},"Unique":{"uniqueItems":true},
      "Count":{"minLength":2,"maxLength":3,"minItems":2,"maxItems":3,"minProperties":2,"maxProperties":3},
      "HugeCount":{"maxItems":1e999999999999999999999999,"minProperties":-0e9},
      "Object":{"type":"object","properties":{"a/b~":{"type":["integer","null"]}},"required":["a/b~"],"additionalProperties":false},
      "Array":{"prefixItems":[{"type":"string"}],"items":{"type":"integer"}},
      "Choice":{"oneOf":[{"type":"integer"},{"type":"number"}]},
      "Any":{"anyOf":[{"required":["a","b"]},{"type":"integer"}]},
      "Not":{"not":{"const":9007199254740993}},
      "Node":{"type":"object","properties":{"value":{"type":"integer"},"next":{"$ref":"#/components/schemas/Node"}}},
      "RefSibling":{"$ref":"#/components/schemas/Integer","minimum":5},
      "All":{"allOf":[{"type":"integer"},{"minimum":5}]},
      "Pattern":{"pattern":"^(?:é|[a-c]){2,3}$"},"Search":{"pattern":"a.*b"}
    }"##).unwrap(),Config::default());
    let values = [
        ("Integer", "70e-1"),
        ("Integer", "1e-400"),
        ("Integer", "1e999999999999999999999999"),
        ("Range", "9007199254740993"),
        ("Range", "9007199254740992"),
        ("Range", "9007199254740994"),
        ("Multiple", "0.29"),
        ("Multiple", "0.29000000000000001"),
        ("Multiple", "-0.29"),
        ("Factors", "0.003"),
        ("Factors", "0.000125"),
        ("Factors", "1e999999999999999999999999"),
        ("HugeDivisor", "6e1000000000000000000000000"),
        ("HugeDivisor", "2e1000000000000000000000000"),
        ("HugeDivisor", "3e999999999999999999999998"),
        ("Tiny", "10e-1000000000000000000000000"),
        ("Tiny", "0"),
        ("Enum", "9007199254740993"),
        ("Enum", "9007199254740992"),
        ("Enum", r#"{"__proto__":null,"x":10e-1}"#),
        ("Const", r#"{"a/b~":[9007199254740993,10e-401]}"#),
        ("Const", r#"{"a/b~":[9007199254740992,1e-400]}"#),
        ("Unique", "[1,1.0]"),
        ("Unique", "[9007199254740992,9007199254740993]"),
        ("Unique", "[0,-0e999999999999999999999999]"),
        ("Count", r#""é𝄞""#),
        ("Count", "[1]"),
        ("Count", "[1,2,3,4]"),
        ("Count", r#"{"a":1,"b":2}"#),
        ("HugeCount", "[]"),
        ("Object", r#"{"a/b~":null}"#),
        ("Object", r#"{"a/b~":1.5}"#),
        ("Object", r#"{"x":0}"#),
        ("Array", r#"["x",1,2.5]"#),
        ("Array", r#"["x",10e-1]"#),
        ("Choice", "1"),
        ("Choice", "1.5"),
        ("Any", "1"),
        ("Any", r#"{"a":0}"#),
        ("Not", "9007199254740993"),
        ("Not", "9007199254740992"),
        ("Node", r#"{"next":{"value":1}}"#),
        ("Node", r#"{"next":{"value":"x"}}"#),
        ("RefSibling", "4"),
        ("RefSibling", "5"),
        ("All", "4"),
        ("All", "5"),
        ("Pattern", r#""éa""#),
        ("Pattern", r#""éa\n""#),
        ("Pattern", r#""dddd""#),
        ("Search", r#""xxa𝄞bzz""#),
        ("Search", r#""a\nb""#),
    ];
    let mut assertions = cases(0, &compiled, &roots, &values);
    assertions.push_str("assert!(matches!(generated::p0::validate(usize::MAX,&generated::Nullable::Null),generated::p0::ValidationOutcome::EvaluationFailure(_)));\n");
    native(&[compiled.program()], &assertions, "");
}

#[test]
#[ignore = "requires native Cargo"]
fn native_logical_failures_cycles_and_shared_sessions_never_pass_exhaustion() {
    let expensive = json!({"allOf":vec![true;40]});
    let mut programs = Vec::new();
    let mut assertions = String::new();
    for (schemas, config, values) in [
        (
            json!({"Not":{"not":expensive},"Any":{"anyOf":[true,expensive]},"One":{"oneOf":[true,expensive]},"Wide":{"additionalProperties":true},"Pattern":{"pattern":"a+b"}}),
            Config {
                max_evaluation_steps: 32,
                max_errors: 1,
                ..Config::default()
            },
            vec![
                ("Not", "7"),
                ("Any", "7"),
                ("One", "7"),
                ("Pattern", r#""aaaaaaaaaaaaaaaaaaaaaaaaaaaaa""#),
            ],
        ),
        (
            json!({"Not":{"not":{"const":[1,2]}},"Any":{"anyOf":[true,{"const":[1,2]}]},"Enum":{"enum":[null,1]}}),
            Config {
                max_equality_steps: 1,
                ..Config::default()
            },
            vec![("Not", "[1,2]"), ("Any", "[1,2]"), ("Enum", "1")],
        ),
        (
            json!({"Integer":{"type":"integer"},"Number":{"type":["integer","number"]},"Enum":{"enum":[1000,"x"]}}),
            Config {
                max_number_bytes: 1,
                ..Config::default()
            },
            vec![
                ("Integer", "1000"),
                ("Number", "1000"),
                ("Enum", "1000"),
                ("Enum", r#""x""#),
            ],
        ),
        (
            json!({"Any":true}),
            Config {
                max_evaluation_steps: 0,
                ..Config::default()
            },
            vec![("Any", "null")],
        ),
        (
            json!({"Cycle":{"$ref":"#/components/schemas/Cycle"},"Node":{"type":"object","properties":{"next":{"$ref":"#/components/schemas/Node"}}}}),
            Config {
                max_depth: 8,
                ..Config::default()
            },
            vec![
                ("Cycle", "null"),
                ("Node", r#"{"next":{"next":{"next":{"next":{"next":{}}}}}}"#),
            ],
        ),
        (
            json!({"Root":{"const":1},"Other":true}),
            Config {
                max_evaluation_steps: 4,
                max_equality_steps: 2,
                ..Config::default()
            },
            vec![("Root", "1.0")],
        ),
    ] {
        let (compiled, roots) = compile(schemas, config);
        assertions.push_str(&cases(programs.len(), &compiled, &roots, &values));
        programs.push(compiled.program());
    }
    // Native coefficient division has an additional bounded work allowance.
    // Exceeding it is unknown validity, even when another alternative passed.
    let (numeric, _) = compile(
        json!({
            "Not": {"not": {"multipleOf": 97}},
            "Any": {"anyOf": [true, {"multipleOf": 97}]},
            "One": {"oneOf": [true, {"multipleOf": 97}]}
        }),
        Config {
            max_evaluation_steps: 32,
            ..Config::default()
        },
    );
    let numeric_program = numeric.program();
    for root in &numeric_program.roots {
        assertions.push_str(&format!("{{ let value = generated::json::parse_json(\"1234567890123456789012345678901234\",generated::json::JsonLimits::default()).unwrap(); match generated::p6::validate({},&value) {{ generated::p6::ValidationOutcome::EvaluationFailure(f) => {{ assert_eq!(f.document,{:?}); assert!(f.pointer.ends_with(\"/multipleOf\")); assert_eq!(f.instance_path,\"\"); }}, other => panic!(\"numeric work exhaustion was suppressed: {{other:?}}\") }} }}\n", root.target, root.source.document));
    }
    programs.push(numeric_program);
    let selected = programs[5]
        .roots
        .iter()
        .find(|root| root.source.pointer == "/components/schemas/Root")
        .unwrap();
    let root = selected.target;
    let document = &selected.source.document;
    let pointer = &selected.source.pointer;
    let other = programs[5]
        .roots
        .iter()
        .find(|root| root.source.pointer == "/components/schemas/Other")
        .unwrap()
        .target;
    let private = format!(
        r#"
#[test]
fn codec_session_cannot_reset_limits_between_trials() {{
    use p5::{{ValidationOutcome,ValidationSession}};
    let a = json::parse_json("1",json::JsonLimits::default()).unwrap();
    let b = json::parse_json("1.0",json::JsonLimits::default()).unwrap();
    let mut session = ValidationSession::new();
    assert!(matches!(session.validate_at({root},&a,""),ValidationOutcome::Valid));
    assert!(session.equal_at({root},&a,&b,"").unwrap());
    assert!(session.equal_at({root},&a,&b,"").is_err());
    assert!(matches!(session.validate_at({root},&a,""),ValidationOutcome::EvaluationFailure(_)));
}}
#[test]
fn explicit_sources_and_paths_survive_unrelated_trials() {{
    use p5::{{ValidationOutcome,ValidationSession}};
    let value = Nullable::Value(JsonNonNullValue::Number("1".repeat(4097).parse().unwrap()));
    let mut session = ValidationSession::new();
    assert!(matches!(session.validate_at({other},&value,"/unrelated"),ValidationOutcome::Valid));
    let failure = session.equal_at({root},&value,&value,"/a~1b~0/0").unwrap_err();
    assert_eq!(failure.document,{document:?});
    assert_eq!(failure.pointer,{pointer:?});
    assert_eq!(failure.instance_path,"/a~1b~0/0");
    let failure = session.equal_at(usize::MAX,&value,&value,"/missing").unwrap_err();
    assert_eq!(failure.document,"");
    assert_eq!(failure.instance_path,"/missing");
    let mut session = ValidationSession::new();
    match session.validate_at({root},&Nullable::Null,"/a~1b~0/0") {{
        ValidationOutcome::Invalid(findings) => {{
            assert_eq!(findings[0].document,{document:?});
            assert_eq!(findings[0].pointer,concat!({pointer:?},"/const"));
            assert_eq!(findings[0].instance_path,"/a~1b~0/0");
        }}
        other => panic!("unexpected located validation result: {{other:?}}"),
    }}
}}
"#
    );
    native(&programs, &assertions, &private);
}

#[test]
#[ignore = "requires native Cargo"]
fn native_finite_reference_chain_honors_the_admitted_depth_ceiling() {
    let (compiled, _) = compile(json!({"Root": true}), Config::default());
    let mut programs = Vec::new();
    let mut assertions = String::from("std::thread::spawn(|| {\n");
    for kind in [
        "ref",
        "allOf",
        "anyOf",
        "oneOf",
        "not",
        "properties",
        "additionalProperties",
        "items",
        "prefixItems",
    ] {
        for count in [512, 513] {
            let mut program = compiled.program();
            let document = program.roots[0].source.document.clone();
            let pointer = |index: usize| {
                if kind == "ref" || index.is_multiple_of(2) {
                    format!("/components/schemas/N{index}")
                } else {
                    let suffix = match kind {
                        "allOf" | "anyOf" | "oneOf" | "prefixItems" => format!("{kind}/0"),
                        "properties" => "properties/next".into(),
                        _ => kind.into(),
                    };
                    format!("/components/schemas/N{}/{suffix}", index - 1)
                }
            };
            program.nodes = (0..count)
                .map(|index| {
                    let source = ProgramSource {
                        document: document.clone(),
                        pointer: pointer(index),
                    };
                    let target = index + 1;
                    let (keyword, instruction) = if target == count {
                        (None, ProgramInstruction::Always { value: true })
                    } else if kind == "ref" || !index.is_multiple_of(2) {
                        (Some("$ref"), ProgramInstruction::Ref { target })
                    } else {
                        (
                            Some(kind),
                            match kind {
                                "allOf" => ProgramInstruction::AllOf {
                                    targets: vec![target],
                                },
                                "anyOf" => ProgramInstruction::AnyOf {
                                    targets: vec![target],
                                },
                                "oneOf" => ProgramInstruction::OneOf {
                                    targets: vec![target],
                                },
                                "not" => ProgramInstruction::Not { target },
                                "properties" => ProgramInstruction::Properties {
                                    properties: vec![suspect_schema::ProgramProperty {
                                        name: "next".into(),
                                        target,
                                    }],
                                },
                                "additionalProperties" => {
                                    ProgramInstruction::AdditionalProperties {
                                        declared: vec![],
                                        target,
                                    }
                                }
                                "items" => ProgramInstruction::Items { target, start: 0 },
                                "prefixItems" => ProgramInstruction::PrefixItems {
                                    targets: vec![target],
                                },
                                _ => unreachable!(),
                            },
                        )
                    };
                    let at = ProgramSource {
                        document: document.clone(),
                        pointer: keyword.map_or_else(
                            || source.pointer.clone(),
                            |key| format!("{}/{key}", source.pointer),
                        ),
                    };
                    ProgramNode {
                        source,
                        checks: vec![ProgramCheck {
                            source: at,
                            instruction,
                        }],
                    }
                })
                .collect();
            program.roots = vec![ProgramRoot {
                source: program.nodes[0].source.clone(),
                target: 0,
            }];
            program.check().unwrap();
            let module = programs.len();
            let mut instance_path = String::new();
            assertions.push_str("{\n");
            match kind {
                "properties" | "additionalProperties" => {
                    assertions.push_str("let mut value = generated::Nullable::Null;\n");
                    assertions.push_str("for _ in 0..256 { value = generated::Nullable::Value(generated::JsonNonNullValue::Object(std::collections::BTreeMap::from([(\"next\".into(), value)]))); }\n");
                    instance_path = "/next".repeat(256);
                }
                "items" | "prefixItems" => {
                    assertions.push_str("let mut value = generated::Nullable::Null;\n");
                    assertions.push_str("for _ in 0..256 { value = generated::Nullable::Value(generated::JsonNonNullValue::Array(vec![value])); }\n");
                    instance_path = "/0".repeat(256);
                }
                _ => assertions.push_str("let value = generated::Nullable::Null;\n"),
            }
            if count == 512 {
                assertions.push_str(&format!("assert!(matches!(generated::p{module}::validate(0,&value),generated::p{module}::ValidationOutcome::Valid),\"{kind} at admitted depth\");\n"));
            } else {
                let expected = &program.nodes[512].source;
                assertions.push_str(&format!("match generated::p{module}::validate(0,&value) {{ generated::p{module}::ValidationOutcome::EvaluationFailure(f) => {{ assert_eq!(f.document,{:?}); assert_eq!(f.pointer,{:?}); assert_eq!(f.instance_path,{instance_path:?}); }}, other => panic!(\"{kind} depth did not fail explicitly: {{other:?}}\") }}\n", expected.document, expected.pointer));
            }
            assertions.push_str("}\n");
            programs.push(program);
        }
    }
    assertions.push_str("}).join().expect(\"default-stack validation thread must complete\");");
    native(&programs, &assertions, "");
}

#[test]
#[ignore = "requires native Cargo and OPENROUTER_WEB_ROOT"]
fn tracked_openrouter_nullable_caller_image_and_index_closures_match_owned_schema() {
    let root_dir = std::path::PathBuf::from(
        std::env::var_os("OPENROUTER_WEB_ROOT").expect("set OPENROUTER_WEB_ROOT"),
    );
    let path = root_dir.join("projects/docs/openapi/openapi.yaml");
    let workspace = Arc::new(WorkspaceBuilder::new().root(&root_dir).build().unwrap());
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap());
    let pointers = [
        "/components/schemas/ORAnthropicNullableCaller",
        "/components/schemas/AnthropicImageBlockParam",
        "/components/schemas/ChatChoice/properties/index",
    ];
    let roots: Vec<_> = pointers
        .iter()
        .map(|p| {
            contract
                .schemas()
                .find(|s| s.id().pointer() == *p)
                .unwrap()
                .id()
                .clone()
        })
        .collect();
    let compiled = OwnedCompiler::new(Config::default())
        .compile(contract, &roots)
        .unwrap();
    let mut assertions = String::new();
    for (index, input) in [
        (0, "null"),
        (0, r#"{"type":"direct"}"#),
        (0, r#"{"type":"unknown"}"#),
        (
            1,
            r#"{"type":"image","source":{"type":"url","url":"https://example.com/image.png"}}"#,
        ),
        (
            1,
            r#"{"type":"image","source":{"type":"base64","media_type":"image/png","data":"aGVsbG8="}}"#,
        ),
        (1, r#"{"type":"image"}"#),
        (1, "null"),
        (2, "0"),
        (2, "9007199254740993"),
        (2, "1e-400"),
        (2, r#""0""#),
    ] {
        assertions.push_str(&assertion(0, &compiled, &roots[index], input));
    }
    native(&[compiled.program()], &assertions, "");
}
