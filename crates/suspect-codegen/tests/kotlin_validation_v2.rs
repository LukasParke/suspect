#![cfg(all(feature = "kotlin-sdk", feature = "http-protocol"))]
//! Source-driven scoped execution; never consumes an earlier target witness.
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
};
use suspect_codegen::kotlin_sdk::{self, validation};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_ref::WorkspaceBuilder;
use suspect_schema::{Config, OwnedProgram, ProgramInstruction};
use suspect_source::Uri;
#[path = "kotlin_support/mod.rs"]
#[allow(dead_code)]
mod support;

fn root() -> PathBuf {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-kotlin-applicators");
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix("gate-")
        .tempdir_in(base)
        .unwrap()
        .keep()
        .canonicalize()
        .unwrap()
}
fn load(root: &Path, name: &str, schema: Value) -> (Arc<Contract>, SchemaId) {
    let path = root.join(format!("{name}.json"));
    std::fs::write(&path,json!({"openapi":"3.1.2","info":{"title":"Kotlin scoped validation","version":"1"},"paths":{},"components":{"schemas":{"Root":schema}}}).to_string()).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(root).build().unwrap());
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap());
    let id = SchemaId::new(contract.entry().clone(), Default::default())
        .child("components")
        .child("schemas")
        .child("Root");
    (contract, id)
}
fn config() -> Config {
    Config {
        max_depth: 128,
        ..Default::default()
    }
}
fn source_vectors(root: &Path) -> Vec<Value> {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../suspect-schema/tests/fixtures/owned-applicators-v2.json"
    ))
    .unwrap();
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 32);
    fixture["cases"].as_array().unwrap().iter().map(|case|{
        let (contract,id)=load(root,case["id"].as_str().unwrap(),serde_json::from_str(case["schemaJson"].as_str().unwrap()).unwrap());
        let mut config=config();
        if let Some(cap)=case["limits"]["maxNumberBytes"].as_u64(){config.max_number_bytes=cap as usize;}
        if let Some(cap)=case["limits"]["maxEvaluationSteps"].as_u64(){config.max_evaluation_steps=cap as usize;}
        let program=validation::plan_validation_v2(contract,&[id],config).unwrap();
        assert_eq!(program.version,OwnedProgram::V2_VERSION);
        program.check().unwrap();
        let path=match case["id"].as_str().unwrap(){
            "contains-zero-does-not-mark-unmatched"|"contains-exact-integrality"=>"/0",
            "contains-failure-after-exceeded-maximum"=>"/1",
            "pattern-overlap-rejects"|"named-and-pattern-both-apply"=>"/x",
            "property-names-checks-key-not-value"=>"/long",
            "property-names-does-not-annotate-values"=>"/ok",
            "failed-anyof-branch-does-not-leak"|"allof-cousins-have-independent-scopes"|"not-discards-annotations"|"required-is-not-an-evaluation"=>"/a",
            "nested-members-do-not-mark-parent"=>"/inner",
            "prefix-and-contains-leave-unmatched-item"=>"/2",
            _=>"",
        };
        json!({"id":case["id"],"program":program,"instanceJson":case["instanceJson"],"expected":case["expected"],"source":case.get("source"),"instancePath":path})
    }).collect()
}

fn additional_vectors(root: &Path) -> Vec<Value> {
    let costs = [
        ("if", json!({"if":true,"then":true}), json!(null), 6, "/if"),
        (
            "dependentRequired",
            json!({"dependentRequired":{"a":["b"]}}),
            json!({"a":null,"b":0}),
            4,
            "/dependentRequired",
        ),
        (
            "dependentSchemas",
            json!({"dependentSchemas":{"a":true}}),
            json!({"a":null}),
            5,
            "/dependentSchemas/a",
        ),
        (
            "contains",
            json!({"contains":true}),
            json!([null]),
            6,
            "/contains",
        ),
        (
            "patternProperties",
            json!({"patternProperties":{"":true}}),
            json!({"a":null}),
            10,
            "/patternProperties",
        ),
        (
            "additionalPropertiesWithPatterns",
            json!({"patternProperties":{"":true},"additionalProperties":false}),
            json!({"a":null}),
            16,
            "/patternProperties",
        ),
        (
            "propertyNames",
            json!({"propertyNames":true}),
            json!({"a":null}),
            5,
            "/propertyNames",
        ),
        (
            "unevaluatedProperties",
            json!({"unevaluatedProperties":{}}),
            json!({"a":null}),
            5,
            "/unevaluatedProperties",
        ),
        (
            "unevaluatedItems",
            json!({"unevaluatedItems":{}}),
            json!([null]),
            5,
            "/unevaluatedItems",
        ),
        (
            "duplicate-merge-candidates",
            json!({"allOf":[{"properties":{"a":true}},{"properties":{"a":true}}],"unevaluatedProperties":false}),
            json!({"a":1}),
            21,
            "/unevaluatedProperties",
        ),
    ];
    let mut result = Vec::new();
    for (name, schema, value, steps, _) in costs {
        for maximum in [steps - 1, steps] {
            let (contract, id) = load(root, &format!("cost-{name}-{maximum}"), schema.clone());
            let config = Config {
                max_evaluation_steps: maximum,
                ..config()
            };
            let program = validation::plan_validation_v2(contract, &[id], config).unwrap();
            assert_eq!(program.version, OwnedProgram::V2_VERSION, "cost-{name}");
            result.push(json!({"id":format!("cost-{name}-{maximum}"),"program":program,"instanceJson":value.to_string(),"expected":if maximum==steps{"Valid"}else{"EvaluationFailure"},"source":null}));
        }
    }
    let extra = [
        (
            "property-name-identity",
            r##"{"propertyNames":{"$ref":"#/components/schemas/Root"}}"##,
            r#"{"a":1}"#,
            "Valid",
            None,
            "",
            config(),
            None,
        ),
        (
            "property-name-recursion",
            r##"{"propertyNames":{"$ref":"#/components/schemas/Root/propertyNames"}}"##,
            r#"{"a":1}"#,
            "EvaluationFailure",
            Some("/propertyNames"),
            "/a",
            config(),
            None,
        ),
        (
            "property-name-pointer",
            r#"{"propertyNames":{"maxLength":0}}"#,
            r#"{"a/b~":null}"#,
            "Invalid",
            Some("/propertyNames/maxLength"),
            "/a~1b~0",
            config(),
            None,
        ),
        (
            "property-name-equality",
            r#"{"propertyNames":{"enum":["a"]}}"#,
            r#"{"a":null}"#,
            "EvaluationFailure",
            Some("/propertyNames/enum"),
            "/a",
            Config {
                max_equality_steps: 0,
                ..config()
            },
            None,
        ),
        (
            "unicode-scalar-order",
            r#"{"propertyNames":{"enum":["😀"]}}"#,
            r#"{"😀":null,"\ue000":null}"#,
            "EvaluationFailure",
            Some("/propertyNames/enum"),
            "/\u{e000}",
            Config {
                max_evaluation_steps: 5,
                ..config()
            },
            None,
        ),
        (
            "implicit-minimum-no-numeric-operand",
            r#"{"contains":true}"#,
            r#"[12345]"#,
            "Valid",
            None,
            "",
            Config {
                max_number_bytes: 0,
                ..config()
            },
            None,
        ),
        (
            "symbolic-contains-count",
            r#"{"contains":true,"minContains":-0.0,"maxContains":1e999999999999999999999}"#,
            r#"[9007199254740993]"#,
            "Valid",
            None,
            "",
            config(),
            None,
        ),
        (
            "failed-pattern-exclusion",
            r#"{"patternProperties":{"^x":false},"additionalProperties":{"type":"integer"}}"#,
            r#"{"x":12345}"#,
            "Invalid",
            Some("/patternProperties/^x"),
            "/x",
            Config {
                max_number_bytes: 3,
                ..config()
            },
            None,
        ),
        (
            "report-cap-cannot-hide-failure",
            r#"{"properties":{"a":false,"b":{"contains":{"type":"integer"}}}}"#,
            r#"{"a":null,"b":[12345]}"#,
            "EvaluationFailure",
            Some("/properties/b/contains/type"),
            "/b/0",
            Config {
                max_errors: 1,
                max_number_bytes: 3,
                ..config()
            },
            None,
        ),
        (
            "zero-report-cap-is-unlimited",
            r#"{"propertyNames":false}"#,
            r#"{"a":null,"b":null,"c":null}"#,
            "Invalid",
            Some("/propertyNames"),
            "/a",
            Config {
                max_errors: 0,
                ..config()
            },
            Some(3),
        ),
        (
            "nonproductive-ref-is-not-anyof-success",
            r##"{"anyOf":[true,{"$ref":"#/components/schemas/Root"}],"unevaluatedProperties":{}}"##,
            r#"{}"#,
            "EvaluationFailure",
            Some(""),
            "",
            config(),
            None,
        ),
        (
            "zero-work-does-no-entry",
            r#"{"propertyNames":true}"#,
            r#"{}"#,
            "EvaluationFailure",
            Some(""),
            "",
            Config {
                max_evaluation_steps: 0,
                ..config()
            },
            None,
        ),
    ];
    for (name, schema, value, expected, source, path, config, findings) in extra {
        let (contract, id) = load(root, name, serde_json::from_str(schema).unwrap());
        let program = validation::plan_validation_v2(contract, &[id], config).unwrap();
        assert_eq!(program.version, OwnedProgram::V2_VERSION, "{name}");
        result.push(json!({"id":name,"program":program,"instanceJson":value,"expected":expected,"source":source.map(|s|format!("/components/schemas/Root{s}")),"instancePath":path,"findingsCount":findings}));
    }
    result
}

fn invalid_programs(root: &Path) -> Vec<Value> {
    let (contract, id) = load(
        root,
        "native-guard",
        json!({"if":true,"then":true,"contains":true,"minContains":0,"patternProperties":{"^x":true},"additionalProperties":false,"unevaluatedProperties":false}),
    );
    let program = validation::plan_validation_v2(contract, &[id], config()).unwrap();
    let root_index = program.roots[0].target;
    (0..8)
        .map(|case| {
            let mut value = serde_json::to_value(&program).unwrap();
            match case {
                0 => value["version"] = json!("unknown-scoped-version"),
                1 => value["profile"] = json!(OwnedProgram::V1_PROFILE),
                2 => {
                    value["version"] = json!(OwnedProgram::V1_VERSION);
                    value["profile"] = json!(OwnedProgram::V1_PROFILE);
                }
                3 => {
                    let check = value["nodes"][root_index]["checks"]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .find(|c| c["op"] == "contains")
                        .unwrap();
                    check["minimum"] = json!("1e-400");
                }
                4 => {
                    let check = value["nodes"][root_index]["checks"]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .find(|c| c["op"] == "if")
                        .unwrap();
                    check["thenTarget"] = json!(4294967296_u64 + root_index as u64);
                }
                5 => {
                    let check = value["nodes"][root_index]["checks"]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .find(|c| c["op"] == "additionalPropertiesWithPatterns")
                        .unwrap();
                    check["op"] = json!("additionalProperties");
                }
                6 => value["nodes"][root_index]["checks"]
                    .as_array_mut()
                    .unwrap()
                    .reverse(),
                _ => {
                    let check = value["nodes"][root_index]["checks"]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .find(|c| c["op"] == "patternProperties")
                        .unwrap();
                    check["patterns"][0][1]["start"] = json!(99999);
                }
            }
            value
        })
        .collect()
}

#[test]
fn v1_program_and_runtime_bytes_remain_frozen() {
    use sha2::{Digest, Sha256};
    let root = root();
    let (contract, id) = load(
        &root,
        "base",
        json!({"type":"object","properties":{"x":{"type":"integer","minimum":0}},"additionalProperties":false}),
    );
    let v1 =
        validation::plan_validation(contract.clone(), std::slice::from_ref(&id), config()).unwrap();
    let v2 = validation::plan_validation_v2(contract, std::slice::from_ref(&id), config()).unwrap();
    assert_eq!(v1, v2);
    assert_eq!(v1.version, OwnedProgram::V1_VERSION);
    assert_eq!(
        validation::emit_validation(&v1, "example.base").unwrap(),
        validation::emit_validation(&v2, "example.base").unwrap()
    );
    let hashes: Value =
        serde_json::from_str(include_str!("kotlin_support/v1-runtime-hashes.json")).unwrap();
    for (name, bytes) in [
        (
            "Json.kt",
            include_bytes!("../src/kotlin_sdk/Json.kt").as_slice(),
        ),
        (
            "Validation.kt",
            include_bytes!("../src/kotlin_sdk/Validation.kt").as_slice(),
        ),
    ] {
        assert_eq!(
            format!("{:x}", Sha256::digest(bytes)),
            hashes[name].as_str().unwrap(),
            "frozen v1 runtime asset {name}"
        );
    }
}

#[test]
fn all_32_independent_sources_compile_through_v2() {
    let root = root();
    let cases = source_vectors(&root);
    assert_eq!(cases.len(), 32);
    let ops = cases
        .iter()
        .flat_map(|c| c["program"]["nodes"].as_array().unwrap())
        .flat_map(|n| n["checks"].as_array().unwrap())
        .map(|c| c["op"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    for op in [
        "if",
        "dependentRequired",
        "dependentSchemas",
        "contains",
        "patternProperties",
        "additionalPropertiesWithPatterns",
        "propertyNames",
        "unevaluatedProperties",
        "unevaluatedItems",
    ] {
        assert!(ops.contains(op), "missing source witness {op}");
    }
}

#[test]
fn control_sources_really_need_scoped_execution() {
    let root = root();
    assert_eq!(additional_vectors(&root).len(), 32);
    assert_eq!(invalid_programs(&root).len(), 8);
}

#[test]
fn malformed_scoped_programs_decline_before_native_artifacts() {
    let root = root();
    let (contract, id) = load(
        &root,
        "guard",
        json!({"if":true,"then":true,"patternProperties":{"^x":true},"additionalProperties":false,"contains":true,"minContains":0,"unevaluatedProperties":false}),
    );
    let original =
        validation::plan_validation_v2(contract, std::slice::from_ref(&id), config()).unwrap();
    let root_index = original.roots[0].target;
    for case in 0..7 {
        let mut changed = original.clone();
        match case {
            0 => {
                changed.version = OwnedProgram::V1_VERSION;
                changed.profile = OwnedProgram::V1_PROFILE;
            }
            1 => changed.profile = OwnedProgram::V1_PROFILE,
            2 => changed.version = "unknown-native-validation-v9",
            3 => changed.nodes[root_index].checks.reverse(),
            4 => {
                let c = changed.nodes[root_index]
                    .checks
                    .iter_mut()
                    .find(|c| matches!(c.instruction, ProgramInstruction::Contains { .. }))
                    .unwrap();
                if let ProgramInstruction::Contains { minimum, .. } = &mut c.instruction {
                    *minimum = Some("1e-400".into());
                }
            }
            5 => {
                let c = changed.nodes[root_index]
                    .checks
                    .iter_mut()
                    .find(|c| matches!(c.instruction, ProgramInstruction::If { .. }))
                    .unwrap();
                if let ProgramInstruction::If { then_target, .. } = &mut c.instruction {
                    *then_target = Some(root_index);
                }
            }
            _ => {
                let c = changed.nodes[root_index]
                    .checks
                    .iter_mut()
                    .find(|c| {
                        matches!(
                            c.instruction,
                            ProgramInstruction::AdditionalPropertiesWithPatterns { .. }
                        )
                    })
                    .unwrap();
                if let ProgramInstruction::AdditionalPropertiesWithPatterns { declared, target } =
                    &c.instruction
                {
                    c.instruction = ProgramInstruction::AdditionalProperties {
                        declared: declared.clone(),
                        target: *target,
                    };
                }
            }
        }
        assert!(
            validation::emit_validation(&changed, "example.scoped").is_err(),
            "mutated program {case} was emitted"
        );
    }
}

#[test]
#[ignore = "JDK 21/25 source-driven scoped execution, ownership and exact budget gates"]
fn native_scoped_32_vectors() {
    let root = root();
    let cases = source_vectors(&root);
    let (contract, id) = load(&root, "program", json!({"contains":true}));
    let program = validation::plan_validation_v2(contract, &[id], config()).unwrap();
    for file in validation::emit_validation(&program, "example.scoped").unwrap() {
        let p = root.join(file.path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, file.content).unwrap();
    }
    let package = root.join("kotlin");
    let mut pom = include_str!("../src/kotlin_sdk/pom.xml").to_owned();
    for (key, value) in [
        ("GROUP", "test.suspect.kotlin"),
        ("ARTIFACT", "scoped-vectors"),
        ("VERSION", "0.3.0"),
        ("PACKAGE", "example.scoped"),
        ("KOTLIN", kotlin_sdk::KOTLIN_VERSION),
        ("COROUTINES", kotlin_sdk::COROUTINES_VERSION),
        ("DOKKA", kotlin_sdk::DOKKA_VERSION),
    ] {
        pom = pom.replace(&format!("__{key}__"), value);
    }
    std::fs::write(package.join("pom.xml"), pom).unwrap();
    std::fs::create_dir_all(package.join("docs")).unwrap();
    std::fs::write(package.join("docs/module.md"),"# Module scoped-vectors\n\nSource-driven scoped validation.\n\n# Package example.scoped\n\nExact checked validation APIs.\n").unwrap();
    std::fs::create_dir_all(package.join("src/test/kotlin/example/scoped")).unwrap();
    std::fs::create_dir_all(package.join("src/test/resources/example/scoped")).unwrap();
    std::fs::write(
        package.join("src/test/kotlin/example/scoped/GeneratedExamples.kt"),
        include_str!("../src/kotlin_sdk/native_validation_v2.kt"),
    )
    .unwrap();
    std::fs::write(
        package.join("src/test/resources/example/scoped/vectors.json"),
        json!({"cases":cases,"controls":additional_vectors(&root),"invalidPrograms":invalid_programs(&root)}).to_string(),
    )
    .unwrap();
    for (version, home) in support::java_homes() {
        let output = support::maven(&home)
            .arg("install")
            .current_dir(&package)
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        std::fs::write(root.join(format!("vectors-{version}.log")), &text).unwrap();
        assert!(output.status.success(), "{}\n{text}", root.display());
        assert!(text.contains("KOTLIN_SCOPED_32_PASSED"));
    }
    println!("Kotlin scoped vectors: {}", root.display());
}

fn sdk_plan(root: &Path) -> kotlin_sdk::Plan {
    let path = root.join("sdk.json");
    std::fs::write(&path, include_str!("kotlin_support/scoped-sdk.json")).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(root).build().unwrap());
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap());
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    kotlin_sdk::plan_sdk_v2(
        contract,
        &selected,
        kotlin_sdk::SdkConfig {
            group_id: "test.suspect.kotlin".into(),
            artifact_id: "scoped-sdk".into(),
            version: "0.3.0".into(),
            package_name: "example.scoped.sdk".into(),
            credential_env: None,
            sdk_defaults: None,
            attribution: None,
        },
    )
    .unwrap()
}

#[test]
fn scoped_native_carriers_and_examples_preserve_real_source_constraints() {
    let root = root();
    let plan = sdk_plan(&root);
    assert_eq!(plan.program().version, OwnedProgram::V2_VERSION);
    let pattern = plan
        .models()
        .symbols()
        .iter()
        .find(|s| s.name == "PatternRecord")
        .unwrap();
    assert!(matches!(
        pattern.shape,
        kotlin_sdk::models::Shape::Object {
            additional: kotlin_sdk::models::Additional::Scoped,
            ..
        }
    ));
    for name in ["ConditionalCarrier", "ScopedComposition", "ScopedArray"] {
        let symbol = plan
            .models()
            .symbols()
            .iter()
            .find(|s| s.name == name)
            .unwrap();
        assert!(
            matches!(symbol.shape, kotlin_sdk::models::Shape::CheckedJson),
            "{name}: {:?}",
            symbol.shape
        );
        assert_eq!(symbol.constructor.as_deref(), Some(name));
    }
    assert_eq!(plan.examples().operations().len(), 6);
    for operation in plan.examples().operations() {
        assert!(
            operation.entries.iter().any(|e| e.role
                == suspect_codegen::examples::ExampleRole::RequestBody
                && e.origin == suspect_codegen::examples::ExampleOrigin::Declared),
            "{} lost its source request example",
            operation.operation_id
        );
    }
    assert!(
        plan.examples().diagnostics().is_empty(),
        "{:?}",
        plan.examples().diagnostics()
    );
    assert!(!plan.render().unwrap().is_empty());
}

#[test]
#[ignore = "source-scoped SDK operations, installed consumers, negative types, examples and Dokka on JDK21/25"]
fn native_scoped_sdk_operations() {
    let root = root();
    let plan = sdk_plan(&root);
    for file in plan.render().unwrap() {
        let path = root.join(file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, file.content).unwrap();
    }
    let consumer = root.join("consumer");
    std::fs::create_dir_all(consumer.join("src/main/kotlin")).unwrap();
    std::fs::write(
        consumer.join("src/main/kotlin/NativeScoped.kt"),
        include_str!("../src/kotlin_sdk/native_scoped_sdk.kt"),
    )
    .unwrap();
    let readme = std::fs::read_to_string(root.join("kotlin/README.md")).unwrap();
    let snippet = readme
        .split("```kotlin\n")
        .skip(1)
        .map(|p| p.split("\n```").next().unwrap())
        .find(|p| p.contains("public object Quickstart"))
        .expect("scoped native quickstart");
    std::fs::write(
        consumer.join("src/main/kotlin/Quickstart.kt"),
        format!("package consumer\n{snippet}"),
    )
    .unwrap();
    let guide = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/SDK-KOTLIN.md"),
    )
    .unwrap();
    let blocks = guide
        .split("```kotlin\n")
        .skip(1)
        .map(|p| p.split("\n```").next().unwrap())
        .filter(|p| p.contains("import example.scoped.sdk.*"))
        .collect::<Vec<_>>();
    assert!(!blocks.is_empty());
    let body = blocks
        .iter()
        .flat_map(|b| b.lines())
        .filter(|l| !l.starts_with("import "))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(
        consumer.join("src/main/kotlin/ScopedGuide.kt"),
        format!("package consumer\nimport example.scoped.sdk.*\n{body}\n"),
    )
    .unwrap();
    std::fs::write(consumer.join("pom.xml"),format!(r#"<project xmlns="http://maven.apache.org/POM/4.0.0"><modelVersion>4.0.0</modelVersion><groupId>test.suspect</groupId><artifactId>scoped-consumer</artifactId><version>0.3.0</version>
<properties><project.build.sourceEncoding>UTF-8</project.build.sourceEncoding><kotlin.compiler.daemon>false</kotlin.compiler.daemon></properties>
<dependencies><dependency><groupId>test.suspect.kotlin</groupId><artifactId>scoped-sdk</artifactId><version>0.3.0</version></dependency></dependencies>
<build><sourceDirectory>src/main/kotlin</sourceDirectory><plugins><plugin><groupId>org.jetbrains.kotlin</groupId><artifactId>kotlin-maven-plugin</artifactId><version>{}</version><configuration><jvmTarget>21</jvmTarget><args><arg>-Werror</arg></args></configuration><executions><execution><id>compile</id><phase>compile</phase><goals><goal>compile</goal></goals></execution></executions></plugin>
<plugin><groupId>org.codehaus.mojo</groupId><artifactId>exec-maven-plugin</artifactId><version>3.6.3</version><configuration><executable>${{java.home}}/bin/java</executable><arguments><argument>-Xmx512m</argument><argument>-cp</argument><classpath/><argument>consumer.NativeScopedKt</argument></arguments></configuration></plugin></plugins></build></project>"#,kotlin_sdk::KOTLIN_VERSION)).unwrap();
    for (version, home) in support::java_homes() {
        for (label, dir, args) in [
            ("sdk", root.join("kotlin"), vec!["install"]),
            ("consumer", consumer.clone(), vec!["compile", "exec:exec"]),
        ] {
            let output = support::maven(&home)
                .args(args)
                .current_dir(dir)
                .output()
                .unwrap();
            let log = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            std::fs::write(root.join(format!("{label}-{version}.log")), &log).unwrap();
            assert!(output.status.success(), "{}\n{log}", root.display());
        }
        support::native_docs(&plan, &root, &version);
        for (i, code) in [
            "val bad = PatternRecord()",
            "val bad = PatternRecord(label = \"x\", additionalProperties = mapOf(\"n_x\" to 1))",
            "val bad = PatternRecord(label = \"x\", kind = PatternRecordKind.RECORD)",
            "val bad = ConditionalCarrier(value = \"raw\")",
            "val bad = RoundtripCarrierInput(body = JsonString(\"raw\"))",
            "val bad = DependenciesRecord(credit = null)",
        ]
        .iter()
        .enumerate()
        {
            let path = consumer.join("src/main/kotlin/Negative.kt");
            std::fs::write(
                &path,
                format!("package consumer\nimport example.scoped.sdk.*\n{code}\n"),
            )
            .unwrap();
            let output = support::maven(&home)
                .arg("compile")
                .current_dir(&consumer)
                .output()
                .unwrap();
            let log = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            std::fs::write(root.join(format!("negative-{version}-{i}.log")), &log).unwrap();
            assert!(
                !output.status.success()
                    && log.contains("Negative.kt")
                    && !log.contains("Unresolved reference"),
                "uncontrolled negative {code}\n{log}"
            );
            std::fs::remove_file(path).unwrap();
        }
    }
    println!("Kotlin scoped SDK: {}", root.display());
}
