//! Independent schema vectors -> real v2 programs -> native Rust validation/codecs.
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::rust_validation::emit;
use suspect_ir::contract::{Contract, SchemaId};
use suspect_ref::WorkspaceBuilder;
use suspect_schema::{Config, OwnedCompiler, OwnedProgram, ProgramInstruction};
use suspect_source::Uri;

static NATIVE_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn load(schema: &Value) -> (Arc<Contract>, SchemaId) {
    let directory = tempfile::tempdir().unwrap();
    let entry = directory.path().join("api.json");
    std::fs::write(&entry,json!({"openapi":"3.1.2","info":{"title":"Native v2","version":"1"},"paths":{},"components":{"schemas":{"Root":schema}}}).to_string()).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(directory.path())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&entry).unwrap()).unwrap());
    let root = SchemaId::new(contract.entry().clone(), Default::default())
        .child("components")
        .child("schemas")
        .child("Root");
    (contract, root)
}
fn config(value: &Value) -> Config {
    let mut c = Config::default();
    for (key, slot) in [
        ("maxNumberBytes", &mut c.max_number_bytes),
        ("maxEvaluationSteps", &mut c.max_evaluation_steps),
        ("maxEqualitySteps", &mut c.max_equality_steps),
        ("maxErrors", &mut c.max_errors),
        ("maxDepth", &mut c.max_depth),
    ] {
        if let Some(value) = value[key].as_u64() {
            *slot = usize::try_from(value).unwrap();
        }
    }
    c
}
fn program(schema: &Value, config: Config) -> OwnedProgram {
    let (contract, root) = load(schema);
    let compiled = OwnedCompiler::new(config)
        .compile_v2(contract.clone(), &[root])
        .unwrap();
    let p = compiled.program();
    p.check().unwrap();
    for node in &p.nodes {
        assert!(
            contract
                .schemas()
                .any(|s| s.id().document().as_str() == node.source.document
                    && s.id().pointer() == node.source.pointer),
            "invented schema identity"
        );
    }
    p
}
fn checked(command: &mut Command, root: &Path) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "retained native v2 attempt: {}\n{command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn cargo(mode: &str, manifest: &Path, target: &Path) -> Command {
    let mut command = Command::new("cargo");
    command
        .args([mode, "--offline", "--quiet", "--manifest-path"])
        .arg(manifest)
        .arg("--target-dir")
        .arg(target)
        .env_remove("RUST_MIN_STACK")
        .env("RUSTFLAGS", "-D warnings")
        .env("RUSTDOCFLAGS", "-D warnings");
    if let Some(toolchain) = std::env::var_os("SUSPECT_NATIVE_RUST_TOOLCHAIN") {
        command.env("RUSTUP_TOOLCHAIN", toolchain);
    }
    command
}
fn target() -> PathBuf {
    std::env::var_os("SUSPECT_RUST_V2_TARGET")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join(
                if std::env::var_os("SUSPECT_NATIVE_RUST_TOOLCHAIN").is_some() {
                    "../../target/native-rust-validation-v2-msrv"
                } else {
                    "../../target/native-rust-validation-v2"
                },
            )
        })
}
fn native(programs: &[OwnedProgram], assertions: &str, private: &str) {
    let _gate = NATIVE_GATE.lock().unwrap_or_else(|p| p.into_inner());
    let root = tempfile::Builder::new()
        .prefix("rust-validation-v2-")
        .tempdir()
        .unwrap()
        .keep();
    let package = root.join("package");
    std::fs::create_dir_all(package.join("src")).unwrap();
    std::fs::write(
        package.join("src/support.rs"),
        include_str!("../src/rust_models/runtime.rs"),
    )
    .unwrap();
    std::fs::write(
        package.join("src/json.rs"),
        include_str!("../src/rust_codecs/json_runtime.rs"),
    )
    .unwrap();
    let mut lib = String::from(
        "#![forbid(unsafe_code)]\nmod support;\npub mod json;\npub use support::{ExtraFieldError,JsonInteger,JsonNonNullValue,JsonNumber,JsonValue,Never,Nullable,NumberError,Presence};\n",
    );
    for (index, p) in programs.iter().enumerate() {
        for file in emit(p).unwrap() {
            let relative = file.path.strip_prefix("rust/src/").unwrap();
            let path = if relative == "validation.rs" {
                format!("v{index}.rs")
            } else {
                format!("v{index}/{}", relative.strip_prefix("validation/").unwrap())
            };
            let path = package.join("src").join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, file.content).unwrap();
        }
        lib.push_str(&format!("pub mod v{index};\n"));
    }
    lib.push_str(private);
    std::fs::write(package.join("src/lib.rs"), lib).unwrap();
    std::fs::write(package.join("Cargo.toml"),"[package]\nname=\"native-v2-validation\"\nversion=\"0.0.0\"\nedition=\"2024\"\nrust-version=\"1.88\"\npublish=false\n[workspace]\n").unwrap();
    let target = target();
    let manifest = package.join("Cargo.toml");
    checked(&mut cargo("test", &manifest, &target), &root);
    checked(cargo("doc", &manifest, &target).arg("--no-deps"), &root);
    checked(
        cargo("package", &manifest, &target).args(["--allow-dirty", "--no-verify"]),
        &root,
    );
    let consumer = root.join("consumer");
    std::fs::create_dir_all(consumer.join("src")).unwrap();
    std::fs::create_dir_all(consumer.join("vendor")).unwrap();
    let archive = target.join("package/native-v2-validation-0.0.0.crate");
    checked(
        Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(consumer.join("vendor")),
        &root,
    );
    std::fs::write(consumer.join("Cargo.toml"),"[package]\nname=\"validation-v2-consumer\"\nversion=\"0.0.0\"\nedition=\"2024\"\n[workspace]\n[dependencies]\nsdk={package=\"native-v2-validation\",path=\"vendor/native-v2-validation-0.0.0\"}\n").unwrap();
    std::fs::write(
        consumer.join("src/main.rs"),
        format!("fn main(){{ {assertions} }}"),
    )
    .unwrap();
    use sha2::{Digest, Sha256};
    let digest = format!("{:x}", Sha256::digest(std::fs::read(archive).unwrap()));
    checked(
        &mut cargo(
            "run",
            &consumer.join("Cargo.toml"),
            &target.join("installed").join(digest),
        ),
        &root,
    );
    // Attempts are retained, including successful installed packages, for handoff.
    eprintln!("native-v2 retained {}", root.display());
}
fn assertion(
    module: usize,
    p: &OwnedProgram,
    input: &str,
    expected: &str,
    source: Option<&str>,
    path: Option<&str>,
    label: &str,
) -> String {
    let node = p.roots[0].target;
    let document = &p.roots[0].source.document;
    format!(
        "{{let value=sdk::json::parse_json({input:?},Default::default()).unwrap();let (kind,findings)=match sdk::v{module}::validate({node},&value){{sdk::v{module}::ValidationOutcome::Valid=>(\"Valid\",vec![]),sdk::v{module}::ValidationOutcome::Invalid(f)=>(\"Invalid\",f),sdk::v{module}::ValidationOutcome::EvaluationFailure(f)=>(\"EvaluationFailure\",vec![f])}};assert_eq!(kind,{expected:?},\"{label}: {{findings:?}}\");for finding in &findings{{assert_eq!(finding.document,{document:?});}}{}{} }}\n",
        source
            .map(|s| format!(
                "assert!(findings.iter().any(|f|f.pointer=={s:?}),\"{label}: {{findings:?}}\");"
            ))
            .unwrap_or_default(),
        path.map(|p| format!(
            "assert!(findings.iter().any(|f|f.instance_path=={p:?}),\"{label}: {{findings:?}}\");"
        ))
        .unwrap_or_default()
    )
}

#[test]
fn v2_program_admission_is_source_linked_and_rejects_malformed_variants() {
    let original = program(
        &json!({"if":true,"then":false,"contains":true,"minContains":0,"patternProperties":{"^x":true},"additionalProperties":false,"unevaluatedProperties":false}),
        Config::default(),
    );
    assert_eq!(original.version, OwnedProgram::V2_VERSION);
    assert!(emit(&original).is_ok());
    let first = original
        .nodes
        .iter()
        .flat_map(|n| &n.checks)
        .find(|c| c.instruction.requires_v2())
        .unwrap()
        .source
        .clone();
    let mut wrong = original.clone();
    wrong.version = OwnedProgram::V1_VERSION;
    wrong.profile = OwnedProgram::V1_PROFILE;
    assert_eq!(emit(&wrong).unwrap_err().source, Some(first));
    for mutation in ["target", "count", "pattern", "tail"] {
        let mut wrong = original.clone();
        let root = wrong.roots[0].target;
        let source = match mutation {
            "target" => {
                let c = wrong.nodes[root]
                    .checks
                    .iter_mut()
                    .find(|c| matches!(c.instruction, ProgramInstruction::If { .. }))
                    .unwrap();
                if let ProgramInstruction::If { then_target, .. } = &mut c.instruction {
                    *then_target = Some(root);
                }
                c.source.clone()
            }
            "count" => {
                let c = wrong.nodes[root]
                    .checks
                    .iter_mut()
                    .find(|c| matches!(c.instruction, ProgramInstruction::Contains { .. }))
                    .unwrap();
                if let ProgramInstruction::Contains { minimum, .. } = &mut c.instruction {
                    *minimum = Some("1e-400".into());
                }
                let mut source = c.source.clone();
                source.pointer = "/components/schemas/Root/minContains".into();
                source
            }
            "pattern" => {
                let c = wrong.nodes[root]
                    .checks
                    .iter_mut()
                    .find(|c| matches!(c.instruction, ProgramInstruction::PatternProperties { .. }))
                    .unwrap();
                if let ProgramInstruction::PatternProperties { patterns } = &mut c.instruction {
                    patterns[0].1.start = usize::MAX;
                }
                let mut source = c.source.clone();
                source.pointer.push_str("/^x");
                source
            }
            _ => {
                wrong.nodes[root].checks.reverse();
                wrong.nodes[root].checks[1].source.clone()
            }
        };
        assert_eq!(emit(&wrong).unwrap_err().source, Some(source), "{mutation}");
    }
}

#[test]
fn unused_v2_features_keep_v1_emission_bytes() {
    let (contract, root) = load(
        &json!({"type":"object","properties":{"name":{"type":"string","pattern":"^x+$"}},"additionalProperties":false}),
    );
    let compiler = OwnedCompiler::new(Config::default());
    let v1 = compiler
        .compile(contract.clone(), std::slice::from_ref(&root))
        .unwrap()
        .program();
    let v2 = compiler
        .compile_v2(contract, std::slice::from_ref(&root))
        .unwrap()
        .program();
    assert_eq!(v1, v2);
    assert_eq!(v2.version, OwnedProgram::V1_VERSION);
    assert_eq!(emit(&v1).unwrap(), emit(&v2).unwrap());
    assert_eq!(
        emit(&v1)
            .unwrap()
            .iter()
            .find(|f| f.path == "rust/src/validation/runtime.rs")
            .unwrap()
            .content,
        include_str!("../src/rust_validation/runtime.rs")
    );
}

#[test]
#[ignore = "requires native Cargo/current or 1.88 and tar"]
fn independent_v2_programs_run_through_installed_native_validation() {
    let shared: Value = serde_json::from_str(include_str!(
        "../../suspect-schema/tests/fixtures/owned-applicators-v2.json"
    ))
    .unwrap();
    let own: Value =
        serde_json::from_str(include_str!("fixtures/rust-validation-v2.json")).unwrap();
    let mut programs = Vec::new();
    let mut assertions = String::new();
    for case in shared["cases"].as_array().unwrap() {
        let p = program(
            &serde_json::from_str(case["schemaJson"].as_str().unwrap()).unwrap(),
            config(&case["limits"]),
        );
        assertions.push_str(&assertion(
            programs.len(),
            &p,
            case["instanceJson"].as_str().unwrap(),
            case["expected"].as_str().unwrap(),
            case["source"].as_str(),
            None,
            case["id"].as_str().unwrap(),
        ));
        programs.push(p);
    }
    for case in own["cases"].as_array().unwrap() {
        let p = program(&case["schema"], Config::default());
        for (key, expected) in [("valid", "Valid"), ("invalid", "Invalid")] {
            for input in case[key].as_array().unwrap() {
                assertions.push_str(&assertion(
                    programs.len(),
                    &p,
                    input.as_str().unwrap(),
                    expected,
                    None,
                    None,
                    case["id"].as_str().unwrap(),
                ));
            }
        }
        programs.push(p);
    }
    for case in own["outcomes"].as_array().unwrap() {
        let schema = case["schemaJson"]
            .as_str()
            .map(|s| serde_json::from_str(s).unwrap())
            .unwrap_or_else(|| case["schema"].clone());
        let p = program(&schema, config(&case["limits"]));
        assertions.push_str(&assertion(
            programs.len(),
            &p,
            case["input"].as_str().unwrap(),
            case["expected"].as_str().unwrap(),
            case["source"].as_str(),
            case["path"].as_str(),
            case["id"].as_str().unwrap(),
        ));
        programs.push(p);
    }
    native(&programs, &assertions, "");
}

#[test]
#[ignore = "requires native Cargo/current or 1.88 and tar"]
fn official_applicators_run_against_native_v2_with_unmodified_schema_roots() {
    let suites = [
        (
            "if-then-else",
            include_str!(
                "../../suspect-schema/tests/fixtures/applicator-conformance/if-then-else.json"
            ),
        ),
        (
            "dependentRequired",
            include_str!(
                "../../suspect-schema/tests/fixtures/applicator-conformance/dependentRequired.json"
            ),
        ),
        (
            "dependentSchemas",
            include_str!(
                "../../suspect-schema/tests/fixtures/applicator-conformance/dependentSchemas.json"
            ),
        ),
        (
            "patternProperties",
            include_str!(
                "../../suspect-schema/tests/fixtures/applicator-conformance/patternProperties.json"
            ),
        ),
        (
            "propertyNames",
            include_str!(
                "../../suspect-schema/tests/fixtures/applicator-conformance/propertyNames.json"
            ),
        ),
        (
            "additionalProperties",
            include_str!(
                "../../suspect-schema/tests/fixtures/applicator-conformance/additionalProperties.json"
            ),
        ),
        (
            "contains",
            include_str!("../../suspect-schema/tests/conformance/draft2020-12/contains.json"),
        ),
        (
            "minContains",
            include_str!("../../suspect-schema/tests/conformance/draft2020-12/minContains.json"),
        ),
        (
            "maxContains",
            include_str!("../../suspect-schema/tests/conformance/draft2020-12/maxContains.json"),
        ),
        (
            "unevaluatedProperties",
            include_str!(
                "../../suspect-schema/tests/conformance/draft2020-12/unevaluatedProperties.json"
            ),
        ),
        (
            "unevaluatedItems",
            include_str!(
                "../../suspect-schema/tests/conformance/draft2020-12/unevaluatedItems.json"
            ),
        ),
    ];
    let mut programs = Vec::new();
    let mut assertions = String::new();
    let mut executed = 0;
    let mut deferred = 0;
    for (suite, text) in suites {
        let groups: Vec<Value> = serde_json::from_str(text).unwrap();
        for (group_index, group) in groups.iter().enumerate() {
            let directory = tempfile::tempdir().unwrap();
            let schema_path = directory.path().join("schema.json");
            let entry = directory.path().join("api.json");
            std::fs::write(&schema_path, group["schema"].to_string()).unwrap();
            std::fs::write(&entry,json!({"openapi":"3.1.2","info":{"title":"Official native v2","version":"1"},"components":{"schemas":{"Entry":{"$ref":"schema.json"}}}}).to_string()).unwrap();
            let workspace = Arc::new(
                WorkspaceBuilder::new()
                    .root(directory.path())
                    .build()
                    .unwrap(),
            );
            let contract = Arc::new(
                Contract::from_workspace(&workspace, &Uri::from_path(&entry).unwrap()).unwrap(),
            );
            let root = SchemaId::new(Uri::from_path(&schema_path).unwrap(), Default::default());
            let compiled = OwnedCompiler::new(Config::default())
                .compile_v2(contract, std::slice::from_ref(&root));
            let description = group["description"].as_str().unwrap();
            if matches!(
                description,
                "unevaluatedProperties with $dynamicRef"
                    | "unevaluatedItems with $dynamicRef"
                    | "patternProperties with Unicode property escape"
            ) {
                let errors = compiled
                    .err()
                    .expect("explicit unimplemented dynamic/NFA feature");
                assert!(
                    errors
                        .iter()
                        .any(|e| e.kind == suspect_schema::OwnedCompileErrorKind::Unsupported),
                    "{errors:?}"
                );
                deferred += group["tests"].as_array().unwrap().len();
                continue;
            }
            let p = compiled
                .unwrap_or_else(|e| panic!("{suite}/{description}: {e:?}"))
                .program();
            p.check().unwrap();
            assert_eq!(p.roots[0].source.pointer, "");
            for (case_index, case) in group["tests"].as_array().unwrap().iter().enumerate() {
                executed += 1;
                let expected = if case["valid"].as_bool().unwrap() {
                    "Valid"
                } else {
                    "Invalid"
                };
                assertions.push_str(&assertion(
                    programs.len(),
                    &p,
                    &case["data"].to_string(),
                    expected,
                    None,
                    None,
                    &format!("{suite}-{group_index}-{case_index}"),
                ));
            }
            programs.push(p);
        }
    }
    assert_eq!((executed, deferred), (395, 6));
    native(&programs, &assertions, "");
}

#[test]
#[ignore = "requires native Cargo/current or 1.88 and tar"]
fn v2_recursive_depth_and_codec_sessions_have_finite_isolated_scopes() {
    let p = program(
        &json!({"type":"object","properties":{"next":{"$ref":"#/components/schemas/Root"}},"unevaluatedProperties":false}),
        Config::default(),
    );
    let root = p.roots[0].target;
    let mut assertions = format!(
        r#"
        std::thread::Builder::new().stack_size(2*1024*1024).spawn(||{{
            let mut value=sdk::Nullable::Value(sdk::JsonNonNullValue::Object(std::collections::BTreeMap::new()));
            for _ in 0..200 {{value=sdk::Nullable::Value(sdk::JsonNonNullValue::Object(std::collections::BTreeMap::from([("next".into(),value)])));}}
            assert!(matches!(sdk::v0::validate({root},&value),sdk::v0::ValidationOutcome::Valid));
            for _ in 0..100 {{value=sdk::Nullable::Value(sdk::JsonNonNullValue::Object(std::collections::BTreeMap::from([("next".into(),value)])));}}
            match sdk::v0::validate({root},&value){{sdk::v0::ValidationOutcome::EvaluationFailure(f)=>assert!(f.message.contains("depth")),other=>panic!("expected explicit depth failure: {{other:?}}")}}
        }}).unwrap().join().unwrap();
    "#
    );
    let mut session = program(
        &json!({"properties":{"a":true},"if":true,"then":true,"else":{"unevaluatedProperties":false},"unevaluatedProperties":false}),
        Config {
            max_evaluation_steps: 100,
            max_equality_steps: 1,
            ..Default::default()
        },
    );
    // Root the actual indexed unselected else schema as a second public
    // entry: there is no fabricated schema, mutable annotation context or cast.
    let target = session
        .nodes
        .iter()
        .position(|n| n.source.pointer == "/components/schemas/Root/else")
        .unwrap();
    session.roots.push(suspect_schema::ProgramRoot {
        source: session.nodes[target].source.clone(),
        target,
    });
    session.check().unwrap();
    let a = session.roots[0].target;
    let private = format!(
        r#"
        #[test] fn session_preserves_budgets_but_never_annotation_sets(){{
            let value=json::parse_json("{{\"a\":1}}",Default::default()).unwrap();
            let mut state=v1::ValidationSession::new();
            assert!(matches!(state.validate_at({a},&value,"/first"),v1::ValidationOutcome::Valid));
            assert!(matches!(state.validate_at({target},&value,"/second"),v1::ValidationOutcome::Invalid(_)));
            let first=json::parse_json("1",Default::default()).unwrap();
            let second=json::parse_json("1.0",Default::default()).unwrap();
            assert!(state.equal_at({a},&first,&second,"/number").unwrap());
            assert!(state.equal_at({a},&first,&second,"/number").is_err());
        }}
    "#
    );
    let mut programs = vec![p, session];
    for count in [510, 511] {
        let definitions = (0..count)
            .map(|index| {
                (
                    format!("N{index}"),
                    if index + 1 == count {
                        json!(true)
                    } else {
                        json!({"$ref":format!("#/components/schemas/Root/$defs/N{}",index+1)})
                    },
                )
            })
            .collect::<serde_json::Map<_, _>>();
        let p = program(
            &json!({"if":true,"then":{"$ref":"#/components/schemas/Root/$defs/N0"},"$defs":definitions}),
            Config::default(),
        );
        let check = assertion(
            programs.len(),
            &p,
            "null",
            if count == 510 {
                "Valid"
            } else {
                "EvaluationFailure"
            },
            None,
            Some(""),
            "exact-native-depth-ceiling",
        );
        let check = if count == 510 {
            assertion(
                programs.len(),
                &p,
                "null",
                "Valid",
                None,
                None,
                "exact-native-depth-ceiling",
            )
        } else {
            check
        };
        assertions.push_str(&format!("std::thread::Builder::new().stack_size(2*1024*1024).spawn(||{{{check}}}).unwrap().join().unwrap();\n"));
        programs.push(p);
    }
    native(&programs, &assertions, &private);
}

fn model_contract() -> (Arc<Contract>, Vec<SchemaId>) {
    let schemas = json!({
        "Mode":{"type":"string","enum":["cash","credit"]},
        "Checkout":{"type":"object","required":["amount","mode"],"properties":{
            "amount":{"type":"integer","minimum":0,"maximum":1000},"mode":{"$ref":"#/components/schemas/Mode"},
            "card":{"type":"string","minLength":1},"billing":{"type":"string","minLength":1}
        },"if":{"properties":{"mode":{"const":"credit"}},"required":["mode"]},"then":{"required":["card"]},
          "dependentRequired":{"card":["billing"]},"patternProperties":{"^x-[a-z]+$":{"type":"string"}},"additionalProperties":false,"unevaluatedProperties":false},
        "PatternMap":{"type":"object","patternProperties":{"^n:":{"type":"integer"},"^s:":{"type":"string"}},"additionalProperties":{"type":"boolean"}},
        "Tuple":{"type":"array","prefixItems":[{"type":"integer"},{"type":"string"}],"contains":{"type":"boolean"},"unevaluatedItems":false},
        "Conditional":{"if":{"type":"string"},"then":{"minLength":2},"else":{"type":"integer"}},
        "Maybe":{"if":{"const":null},"then":true,"else":{"type":"string"}},
        "Node":{"type":"object","required":["value"],"properties":{"value":{"type":"integer"},"next":{"$ref":"#/components/schemas/Node"}},"unevaluatedProperties":false},
        "Open":{"type":"object","required":["base"],"properties":{"base":{"type":"integer"}}},
        "ClosedRef":{"$ref":"#/components/schemas/Open","properties":{"extra":{"type":"integer"}},"unevaluatedProperties":false}
    });
    let directory = tempfile::tempdir().unwrap();
    let entry = directory.path().join("models.json");
    std::fs::write(&entry,json!({"openapi":"3.1.2","info":{"title":"Native v2 models","version":"1"},"components":{"schemas":schemas}}).to_string()).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(directory.path())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&entry).unwrap()).unwrap());
    let roots = contract.schema_roots().to_vec();
    (contract, roots)
}

#[test]
fn v2_model_carriers_retain_constraints_and_base_codecs_keep_their_bytes() {
    use suspect_codegen::{rust_codecs, rust_models};
    let (contract, roots) = model_contract();
    let models = rust_models::plan_models_v2(&contract, &roots);
    assert!(!models.has_errors(), "{:?}", models.diagnostics());
    assert!(!models.release_ready());
    assert!(
        models
            .diagnostics()
            .iter()
            .any(|d| d.code == "v2-codec-obligation"
                && d.source.pointer().ends_with("/patternProperties"))
    );
    assert!(
        rust_models::plan_models(&contract, &roots).has_errors(),
        "v1 admission remains frozen"
    );
    let codecs = rust_codecs::plan_codecs_v2(contract, &roots, Default::default()).unwrap();
    assert_eq!(codecs.validation_version(), OwnedProgram::V2_VERSION);
    assert_eq!(codecs.validation_profile(), OwnedProgram::V2_PROFILE);
    let (base, root) = load(
        &json!({"type":"object","required":["name"],"properties":{"name":{"type":"string","minLength":1},"optional":{"type":["string","null"]},"numbers":{"type":"array","items":{"type":"integer"}}},"additionalProperties":false}),
    );
    let v1 = rust_codecs::plan_codecs(
        base.clone(),
        std::slice::from_ref(&root),
        Default::default(),
    )
    .unwrap();
    let v2 = rust_codecs::plan_codecs_v2(base, &[root], Default::default()).unwrap();
    assert_eq!(v2.validation_version(), OwnedProgram::V1_VERSION);
    assert_eq!(
        v1.render(),
        v2.render(),
        "unused features must not rewrite a v1 package"
    );
    let (invalid, root) = load(&json!({"type":"object","patternProperties":{"[":true}}));
    assert!(
        rust_codecs::plan_codecs_v2(invalid, &[root], Default::default())
            .unwrap_err()
            .iter()
            .any(|d| d.code == "codec-schema-compilation"
                && d.source.pointer().ends_with("/patternProperties/["))
    );
}

#[test]
#[ignore = "requires native Cargo/current or 1.88 and tar"]
fn installed_v2_codecs_validate_mutable_native_carriers_and_negative_consumers() {
    let (contract, roots) = model_contract();
    let plan =
        suspect_codegen::rust_codecs::plan_codecs_v2(contract, &roots, Default::default()).unwrap();
    assert_eq!(plan.validation_version(), OwnedProgram::V2_VERSION);
    native_codecs(&plan, CODEC_CONSUMER);
}

fn native_codecs(plan: &suspect_codegen::rust_codecs::CodecPlan, source: &str) {
    let _gate = NATIVE_GATE.lock().unwrap_or_else(|p| p.into_inner());
    let directory = tempfile::Builder::new()
        .prefix("rust-codecs-v2-")
        .tempdir()
        .unwrap()
        .keep();
    suspect_codegen::write_files(&plan.render(), &directory).unwrap();
    let target = target();
    let manifest = directory.join("rust/Cargo.toml");
    checked(&mut cargo("test", &manifest, &target), &directory);
    checked(cargo("test", &manifest, &target).arg("--doc"), &directory);
    checked(
        cargo("doc", &manifest, &target).arg("--no-deps"),
        &directory,
    );
    checked(
        cargo("package", &manifest, &target).args(["--allow-dirty", "--no-verify"]),
        &directory,
    );
    let consumer = directory.join("consumer");
    std::fs::create_dir_all(consumer.join("src")).unwrap();
    std::fs::create_dir_all(consumer.join("vendor")).unwrap();
    let archive = target.join("package/generated-models-0.0.0.crate");
    checked(
        Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(consumer.join("vendor")),
        &directory,
    );
    std::fs::write(consumer.join("Cargo.toml"),"[package]\nname=\"codecs-v2-consumer\"\nversion=\"0.0.0\"\nedition=\"2024\"\n[workspace]\n[dependencies]\nsdk={package=\"generated-models\",path=\"vendor/generated-models-0.0.0\"}\n").unwrap();
    std::fs::write(consumer.join("src/lib.rs"), source).unwrap();
    use sha2::{Digest, Sha256};
    let digest = format!("{:x}", Sha256::digest(std::fs::read(archive).unwrap()));
    checked(
        &mut cargo(
            "test",
            &consumer.join("Cargo.toml"),
            &target.join("codec-installed").join(digest),
        ),
        &directory,
    );
    eprintln!("native-v2-codecs retained {}", directory.display());
}

#[test]
#[ignore = "requires native Cargo/current or 1.88 and tar"]
fn installed_v2_codec_resource_failures_are_never_invalid_or_representation_success() {
    let (contract, roots) = model_contract();
    let mut config = suspect_codegen::rust_codecs::CodecConfig::default();
    config.schema.max_evaluation_steps = 0;
    let plan = suspect_codegen::rust_codecs::plan_codecs_v2(contract, &roots, config).unwrap();
    native_codecs(
        &plan,
        r##"
        #[test] fn zero_evaluation_budget_survives_both_codec_directions(){
            use sdk::{models::{Checkout,Mode},codecs::{CheckoutCodec,CodecError}};
            assert!(matches!(CheckoutCodec::decode(r#"{"amount":1,"mode":"cash"}"#),Err(CodecError::EvaluationFailure(_))));
            assert!(matches!(CheckoutCodec::encode(&Checkout::new(1,Mode::Cash)),Err(CodecError::EvaluationFailure(_))));
        }
    "##,
    );
}

fn scoped_http_plan() -> suspect_codegen::rust_http::HttpPlan {
    let directory = tempfile::tempdir().unwrap();
    let entry = directory.path().join("api.json");
    let schema = json!({"type":"object","required":["active"],"properties":{"active":{"type":"boolean"},"quantity":{"type":"integer"}},
        "if":{"properties":{"active":{"const":true}}},"then":{"required":["quantity"]},"unevaluatedProperties":false});
    let media = json!({"schema":schema,"examples":{"good":{"value":{"active":true,"quantity":1}},"bad":{"value":{"active":true}},"extra":{"value":{"active":false,"extra":1}}}});
    std::fs::write(&entry,json!({"openapi":"3.1.2","info":{"title":"Scoped HTTP samples","version":"1"},"servers":[{"url":"https://example.test"}],
        "paths":{"/items":{"post":{"operationId":"putItem","requestBody":{"required":true,"content":{"application/json":media.clone()}},
        "responses":{"200":{"description":"ok","content":{"application/json":media}}}}}}}).to_string()).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(directory.path())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&entry).unwrap()).unwrap());
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    assert!(
        suspect_codegen::rust_http::plan_http(contract.clone(), &selected, Default::default())
            .is_err(),
        "legacy admission is explicit"
    );
    let plan =
        suspect_codegen::rust_http::plan_http_v2(contract, &selected, Default::default()).unwrap();
    assert_eq!(plan.codecs().validation_version(), OwnedProgram::V2_VERSION);
    let examples = plan.examples();
    assert_eq!(examples.operations()[0].entries.len(), 2);
    assert!(
        examples.operations()[0]
            .entries
            .iter()
            .all(|entry| entry.value == json!({"active":true,"quantity":1})
                && entry.declared_source.is_some())
    );
    assert!(
        examples
            .diagnostics()
            .iter()
            .any(|finding| finding.source.pointer().contains("/examples/bad"))
    );
    assert!(
        examples
            .diagnostics()
            .iter()
            .any(|finding| finding.source.pointer().contains("/examples/extra"))
    );
    plan
}

#[test]
fn scoped_http_examples_bind_the_v2_program_and_preserve_invalid_source_findings() {
    scoped_http_plan();
}

#[test]
#[ignore = "requires native Cargo/current or 1.88 and the pinned http dependency"]
fn scoped_http_native_constructor_examples_compile_and_execute() {
    let _gate = NATIVE_GATE.lock().unwrap_or_else(|p| p.into_inner());
    let plan = scoped_http_plan();
    let directory = tempfile::Builder::new()
        .prefix("rust-http-v2-examples-")
        .tempdir()
        .unwrap()
        .keep();
    suspect_codegen::write_files(&plan.render(), &directory).unwrap();
    let manifest = directory.join("rust/Cargo.toml");
    let target = target();
    checked(
        cargo("run", &manifest, &target).args(["--example", "validated", "--features", "http"]),
        &directory,
    );
    checked(
        cargo("test", &manifest, &target).args(["--doc", "--features", "reqwest-rustls"]),
        &directory,
    );
    eprintln!("native-v2-http examples retained {}", directory.display());
}

const CODEC_CONSUMER: &str = r##"
/// Conditional dependencies retain an explicit native mode type.
/// ```compile_fail
/// sdk::models::Checkout::new(1, "credit".to_owned());
/// ```
/// Heterogeneous prefix arrays preserve exact JSON values, not a uniform tail type.
/// ```compile_fail
/// let _: sdk::models::Tuple = vec![1i32, 2i32];
/// ```
/// Patterned dynamic values cannot be an unchecked primitive map.
/// ```compile_fail
/// let _:sdk::models::PatternMap=std::collections::BTreeMap::from([("n:x".into(),1i32)]);
/// ```
/// Exact recursive integers cannot be silently populated through f64.
/// ```compile_fail
/// sdk::models::Node::new(9007199254740993.0f64);
/// ```
pub struct NativeTyping;

#[cfg(test)]
mod tests {
    use sdk::{JsonNonNullValue as J,Nullable as N,codecs::{CodecError,CheckoutCodec,PatternMapCodec,TupleCodec,ConditionalCodec,MaybeCodec,NodeCodec,ClosedRefCodec}};
    fn json(text:&str)->sdk::JsonValue{sdk::parse_json(text,Default::default()).unwrap()}
    #[test]fn conditional_requirements_patterns_and_mutable_fields_use_full_root_validation(){
        let mut value=CheckoutCodec::decode(r#"{"amount":1,"mode":"credit","card":"card","billing":"address","x-id":"trace"}"#).unwrap();
        assert_eq!(value.amount,1);assert_eq!(value.card.as_deref(),Some("card"));
        assert!(value.extra_fields().any(|(name,_)|name=="x-id"));
        assert!(value.insert_extra("mode".into(),json("\"cash\"")).is_err(),"declared fields cannot be shadowed");
        value.amount=1001;assert!(matches!(CheckoutCodec::encode(&value),Err(CodecError::Invalid(_))));value.amount=1;
        value.billing=None;assert!(matches!(CheckoutCodec::encode_value(&value),Err(CodecError::Invalid(_))));value.billing=Some("address".into());
        value.insert_extra("x-id".into(),json("false")).unwrap();assert!(matches!(CheckoutCodec::encode(&value),Err(CodecError::Invalid(_))));
        assert!(matches!(CheckoutCodec::decode(r#"{"amount":1,"mode":"credit"}"#),Err(CodecError::Invalid(_))));
        assert!(CheckoutCodec::decode(r#"{"amount":1,"mode":"cash"}"#).is_ok());
    }
    #[test]fn patterned_extras_are_independent_of_unmatched_additional_properties(){
        let mut values=PatternMapCodec::decode(r#"{"n:count":9007199254740993,"s:text":"snow 雪","__proto__":false}"#).unwrap();
        assert!(matches!(&values["n:count"],N::Value(J::Number(n)) if n.as_str()=="9007199254740993"));
        assert!(matches!(values["__proto__"],N::Value(J::Bool(false))));
        values.insert("n:count".into(),json("\"not an integer\""));assert!(matches!(PatternMapCodec::encode_value(&values),Err(CodecError::Invalid(_))));
        assert!(matches!(PatternMapCodec::decode(r#"{"unmatched":"not boolean"}"#),Err(CodecError::Invalid(_))));
    }
    #[test]fn prefix_contains_and_unevaluated_items_keep_heterogeneous_values(){
        let mut values=TupleCodec::decode(r#"[9007199254740993,"text",true]"#).unwrap();
        assert!(matches!(&values[0],N::Value(J::Number(n)) if n.as_str()=="9007199254740993"));
        values.push(json("3"));assert!(matches!(TupleCodec::encode(&values),Err(CodecError::Invalid(_))));
        assert!(matches!(TupleCodec::decode(r#"[1,"text","unmarked"]"#),Err(CodecError::Invalid(_))));
    }
    #[test]fn untyped_conditionals_do_not_invent_a_common_scalar_or_drop_null(){
        assert!(ConditionalCodec::decode("9007199254740993").is_ok());assert!(ConditionalCodec::decode("\"ok\"").is_ok());
        assert!(matches!(ConditionalCodec::decode("null"),Err(CodecError::Invalid(_))));
        assert!(matches!(ConditionalCodec::encode(&J::String("x".into())),Err(CodecError::Invalid(_))));
        assert!(matches!(MaybeCodec::decode("null").unwrap(),N::Null));
        assert!(matches!(MaybeCodec::decode("\"ok\"").unwrap(),N::Value(J::String(v)) if v=="ok"));
        assert!(matches!(MaybeCodec::encode(&json("7")),Err(CodecError::Invalid(_))));
    }
    #[test]fn recursive_values_and_ref_sibling_assertions_are_source_bound(){
        let mut value=NodeCodec::decode(r#"{"value":1,"next":{"value":2}}"#).unwrap();assert_eq!(value.next.as_ref().unwrap().value.as_str(),"2");
        value.next.as_mut().unwrap().insert_extra("a/b~".into(),json("true")).unwrap();
        let Err(CodecError::Invalid(findings))=NodeCodec::encode(&value)else{panic!("unevaluated recursive property accepted")};assert!(findings.iter().any(|f|f.instance_path=="/next/a~1b~0"));
        let mut value=ClosedRefCodec::decode(r#"{"base":1,"extra":2}"#).unwrap();assert_eq!(value.base.as_str(),"1");
        value.insert_extra("extra".into(),json("\"wrong\"")).unwrap();assert!(matches!(ClosedRefCodec::encode(&value),Err(CodecError::Invalid(_))));
        assert!(matches!(ClosedRefCodec::decode(r#"{"base":1,"other":2}"#),Err(CodecError::Invalid(_))));
    }
}
"##;
