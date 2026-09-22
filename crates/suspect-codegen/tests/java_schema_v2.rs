#![cfg(feature = "java-sdk")]
//! Source-driven scoped validation gates. No historical target program/report is an input.

use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::java_sdk::{self, MavenConfig, PackageConfig, SdkPlan};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_ref::WorkspaceBuilder;
use suspect_schema::{Config, OwnedCompiler, OwnedProgram};
use suspect_source::Uri;

fn crate_root() -> PathBuf {
    std::env::var_os("SUSPECT_JAVA_CRATE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_MANIFEST_DIR").into())
}
fn attempt() -> PathBuf {
    let base = crate_root().join("../../target/sdk-java-validation-v2/java");
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix("case-")
        .tempdir_in(base)
        .unwrap()
        .keep()
}
fn load(path: &Path) -> Arc<Contract> {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap())
}
fn schema_at(root: &Path, name: &str, schema: Value) -> (Arc<Contract>, SchemaId) {
    let path = root.join(format!("{name}.json"));
    std::fs::write(&path, json!({"openapi":"3.1.2","info":{"title":"Independent scoped Java witness","version":"1"},"paths":{},"components":{"schemas":{"Root":schema}}}).to_string()).unwrap();
    let contract = load(&path);
    let schema = SchemaId::new(contract.entry().clone(), Default::default())
        .child("components")
        .child("schemas")
        .child("Root");
    (contract, schema)
}
fn checked(command: &mut Command, root: &Path) {
    let result = command.output().unwrap();
    let logs = root.join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    let number = std::fs::read_dir(&logs).unwrap().count();
    std::fs::write(
        logs.join(format!("{number:03}.log")),
        format!(
            "{command:?}\nstatus={}\n{}{}",
            result.status,
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        ),
    )
    .unwrap();
    assert!(
        result.status.success(),
        "{}\n{command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
fn java() -> PathBuf {
    std::env::var_os("JAVA_HOME")
        .map(PathBuf::from)
        .expect("select JDK 21 or 25 with JAVA_HOME")
}
fn maven_repository() -> PathBuf {
    crate_root().join("../../target/sdk-java-maven-cache/java/repository")
}
fn install(plan: &SdkPlan, root: &Path) -> PathBuf {
    for file in plan.render().unwrap() {
        let path = root.join(file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, file.content).unwrap();
    }
    let maven = std::env::var_os("SUSPECT_MAVEN_BIN").unwrap_or_else(|| {
        "/Users/luke/.local/share/mise/installs/maven/3.9.16/apache-maven-3.9.16/bin/mvn".into()
    });
    checked(
        Command::new(maven)
            .args(["-B", "-q", "install"])
            .arg(format!(
                "-Dmaven.repo.local={}",
                maven_repository().display()
            ))
            .env("JAVA_HOME", java())
            .current_dir(root.join("java")),
        root,
    );
    let artifact = &plan.maven().artifact_id;
    let version = &plan.package().version;
    let installed = maven_repository()
        .join(plan.maven_group_id().replace('.', "/"))
        .join(artifact)
        .join(version)
        .join(format!("{artifact}-{version}.jar"));
    assert_eq!(
        std::fs::read(&installed).unwrap(),
        std::fs::read(root.join(format!("java/target/{artifact}-{version}.jar"))).unwrap(),
        "installed artifact differs from built SDK"
    );
    installed
}
fn baseline(root: &Path, artifact: &str) -> SdkPlan {
    let file = root.join("baseline.json");
    std::fs::write(&file, json!({"openapi":"3.1.2","info":{"title":"Scoped native runtime witness","version":"1"},"servers":[{"url":"https://example.test"}],"paths":{"/base":{"get":{"operationId":"base","responses":{"200":{"description":"OK","content":{"application/json":{"schema":{"type":"string"}}}}}}}}}).to_string()).unwrap();
    let contract = load(&file);
    let operations = contract
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    java_sdk::plan_sdk_with_maven(
        contract,
        &operations,
        PackageConfig {
            package: "example.schemav2".into(),
            version: "1.0.0".into(),
            api_name: "Client".into(),
        },
        &[],
        MavenConfig {
            artifact_id: artifact.into(),
            ..Default::default()
        },
    )
    .unwrap()
}
fn vector_cases(root: &Path) -> Value {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../suspect-schema/tests/fixtures/owned-applicators-v2.json"
    ))
    .unwrap();
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 32);
    let mut cases = fixture["cases"].as_array().unwrap().clone();
    cases.extend([
        json!({"id":"java-property-key-order","schemaJson":"{\"propertyNames\":{\"maxLength\":0},\"unevaluatedProperties\":true}","instanceJson":"{\"𐀀\":null,\"\":null}","expected":"Invalid","source":"/components/schemas/Root/propertyNames/maxLength","instancePath":"/"}),
        json!({"id":"java-property-key-pointer","schemaJson":"{\"propertyNames\":{\"maxLength\":0}}","instanceJson":"{\"a/b~\":true}","expected":"Invalid","source":"/components/schemas/Root/propertyNames/maxLength","instancePath":"/a~1b~0"}),
        json!({"id":"java-key-equality-failure","schemaJson":"{\"propertyNames\":{\"enum\":[\"a\"]}}","instanceJson":"{\"a\":null}","limits":{"maxEqualitySteps":0},"expected":"EvaluationFailure","source":"/components/schemas/Root/propertyNames/enum","instancePath":"/a"}),
        json!({"id":"java-if-branch-scope","schemaJson":"{\"if\":{\"properties\":{\"a\":true}},\"then\":{\"unevaluatedProperties\":false}}","instanceJson":"{\"a\":1}","expected":"Invalid","source":"/components/schemas/Root/then/unevaluatedProperties","instancePath":"/a"}),
        json!({"id":"java-nfa-shared-work","schemaJson":"{\"patternProperties\":{\"^x+$\":true}}","instanceJson":"{\"xxxxxxxxxxxxxxxx\":true}","limits":{"maxEvaluationSteps":8},"expected":"EvaluationFailure","source":"/components/schemas/Root/patternProperties","instancePath":""}),
        json!({"id":"java-no-error-cap-short-circuit","schemaJson":"{\"allOf\":[{\"unevaluatedProperties\":false},{\"dependentSchemas\":{\"x\":{\"properties\":{\"n\":{\"type\":\"integer\"}}}}}]}","instanceJson":"{\"x\":null,\"n\":12345}","limits":{"maxNumberBytes":3,"maxErrors":1},"expected":"EvaluationFailure","source":"/components/schemas/Root/allOf/1/dependentSchemas/x/properties/n/type","instancePath":"/n"}),
        json!({"id":"java-key-nonproductive-reference","schemaJson":"{\"propertyNames\":{\"$ref\":\"#/components/schemas/Root/propertyNames\"}}","instanceJson":"{\"a\":true}","expected":"EvaluationFailure","source":"/components/schemas/Root/propertyNames","instancePath":"/a"}),
        json!({"id":"java-contains-symbolic-count","schemaJson":"{\"contains\":true,\"minContains\":1e999999999999999999999}","instanceJson":"[1]","expected":"Invalid","source":"/components/schemas/Root/minContains","instancePath":""}),
    ]);
    let executable = cases
        .into_iter()
        .map(|mut case| {
            let id = case["id"].as_str().unwrap();
            let (contract, schema) = schema_at(
                root,
                id,
                serde_json::from_str(case["schemaJson"].as_str().unwrap()).unwrap(),
            );
            let mut config = Config {
                max_depth: 128,
                ..Config::default()
            };
            if let Some(n) = case["limits"]["maxNumberBytes"].as_u64() {
                config.max_number_bytes = n as usize;
            }
            if let Some(n) = case["limits"]["maxEvaluationSteps"].as_u64() {
                config.max_evaluation_steps = n as usize;
            }
            if let Some(n) = case["limits"]["maxEqualitySteps"].as_u64() {
                config.max_equality_steps = n as usize;
            }
            if let Some(n) = case["limits"]["maxErrors"].as_u64() {
                config.max_errors = n as usize;
            }
            let program = OwnedCompiler::new(config)
                .compile_v2(contract, &[schema])
                .unwrap_or_else(|errors| panic!("{id}: {errors:?}"))
                .program();
            program.check().unwrap();
            assert_eq!(program.version, OwnedProgram::V2_VERSION, "{id}");
            case["program"] = serde_json::to_value(program).unwrap();
            case
        })
        .collect::<Vec<_>>();
    let mut malformed = Vec::new();
    for (id, operation) in [
        ("if-then-selected", "if"),
        ("dependent-required-null-is-present", "dependentRequired"),
        ("dependent-schema-annotations", "dependentSchemas"),
        ("contains-marks-all-matches", "contains"),
        (
            "pattern-overlap-accepts-and-excludes-extra",
            "patternProperties",
        ),
        (
            "pattern-overlap-accepts-and-excludes-extra",
            "additionalPropertiesWithPatterns",
        ),
        ("property-names-checks-key-not-value", "propertyNames"),
        ("dependent-schema-annotations", "unevaluatedProperties"),
        (
            "prefix-and-contains-annotations-combine",
            "unevaluatedItems",
        ),
    ] {
        let original = &executable.iter().find(|c| c["id"] == id).unwrap()["program"];
        let mut old = original.clone();
        old["version"] = json!(OwnedProgram::V1_VERSION);
        old["profile"] = json!(OwnedProgram::V1_PROFILE);
        malformed.push(json!({"id":format!("v1-{operation}"),"program":old}));
        let mut invalid = original.clone();
        let root = invalid["roots"][0]["target"].as_u64().unwrap();
        let check = invalid["nodes"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .flat_map(|n| n["checks"].as_array_mut().unwrap())
            .find(|c| c["op"] == operation)
            .unwrap();
        match operation {
            "if" => check["thenTarget"] = json!(root),
            "dependentRequired" => check["dependencies"] = json!([["a", ["b", "b"]]]),
            "dependentSchemas" => check["dependencies"][0]["target"] = json!(root),
            "contains" => check["minimum"] = json!("1e-400"),
            "patternProperties" => check["patterns"][0][1]["start"] = json!(-1),
            "additionalPropertiesWithPatterns" => check["op"] = json!("additionalProperties"),
            _ => check["target"] = json!(0.5),
        }
        malformed.push(json!({"id":format!("operand-{operation}"),"program":invalid}));
    }
    let original = &executable
        .iter()
        .find(|c| c["id"] == "dependent-schema-annotations")
        .unwrap()["program"];
    for change in ["version", "profile", "order", "unknown-op"] {
        let mut invalid = original.clone();
        let root = invalid["roots"][0]["target"].as_u64().unwrap() as usize;
        match change {
            "version" => invalid["version"] = json!("suspect.validation.experimental.v99"),
            "profile" => invalid["profile"] = json!(OwnedProgram::V1_PROFILE),
            "order" => invalid["nodes"][root]["checks"]
                .as_array_mut()
                .unwrap()
                .reverse(),
            _ => invalid["nodes"][root]["checks"][0]["op"] = json!("unknownScopedInstruction"),
        }
        malformed.push(json!({"id":change,"program":invalid}));
    }
    json!({"cases":executable,"malformed":malformed})
}

#[test]
fn source_driven_v2_programs_keep_base_v1_bytes() {
    let root = attempt();
    let (contract, schema) = schema_at(&root, "base-parity", json!({"type":"integer","minimum":0}));
    let compiler = OwnedCompiler::new(Config {
        max_depth: 128,
        ..Default::default()
    });
    let base = compiler
        .compile(contract.clone(), std::slice::from_ref(&schema))
        .unwrap()
        .program();
    let scoped = compiler
        .compile_v2(contract, std::slice::from_ref(&schema))
        .unwrap()
        .program();
    assert_eq!(base.version, OwnedProgram::V1_VERSION);
    assert_eq!(
        serde_json::to_vec(&base).unwrap(),
        serde_json::to_vec(&scoped).unwrap()
    );
    let vectors = vector_cases(&root);
    assert_eq!(vectors["cases"].as_array().unwrap().len(), 40);
}

#[test]
#[ignore = "source-driven scoped v2 executor, JDK21/25 and installed Maven jar"]
fn native_schema_v2_vectors() {
    let root = attempt();
    let plan = baseline(&root, "java-schema-v2-runtime");
    let jar = install(&plan, &root);
    std::fs::write(root.join("vectors.json"), vector_cases(&root).to_string()).unwrap();
    std::fs::write(
        root.join("NativeSchemaV2.java"),
        include_str!("../src/java_sdk/NativeSchemaV2.java"),
    )
    .unwrap();
    checked(
        Command::new(java().join("bin/javac"))
            .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
            .arg(&jar)
            .arg(root.join("NativeSchemaV2.java")),
        &root,
    );
    checked(
        Command::new(java().join("bin/java"))
            .args(["-ea", "-cp"])
            .arg(format!("{}:{}", jar.display(), root.display()))
            .arg("NativeSchemaV2")
            .arg(root.join("vectors.json")),
        &root,
    );
    println!("JAVA_SCHEMA_V2_VECTORS {}", root.display());
}

fn sdk_document() -> Value {
    let record = json!({
        "type":"object","required":["name","kind","x-required"],
        "properties":{
            "name":{"type":"string"},"kind":{"const":"record"},
            "enabled":{"type":["boolean","null"]},"peer":{"type":["string","null"]},"note":{"type":["string","null"]},
            "choice":{"$ref":"#/components/schemas/ScopedChoice"},"sequence":{"$ref":"#/components/schemas/ScopedSequence"},"node":{"$ref":"#/components/schemas/ScopedNode"}
        },
        "if":{"properties":{"enabled":{"const":true}},"required":["enabled"]},
        "then":{"properties":{"note":{"type":"string","minLength":1}},"required":["note"]},
        "dependentRequired":{"enabled":["peer"]},
        "dependentSchemas":{"peer":{"properties":{"peer":{"type":["string","null"]}}}},
        "patternProperties":{"^x-":{"type":"number","minimum":0},"-integer$":{"type":"integer"},"^name$":{"maxLength":64}},
        "additionalProperties":{"type":"boolean"},"propertyNames":{"pattern":"^[A-Za-z][A-Za-z0-9-]*$"},"unevaluatedProperties":false,
        "example":{"name":"declared","kind":"record","x-required":1,"enabled":true,"peer":null,"note":"hello","x-integer":2,"flag":true,"choice":{"a":"x","b":3},"sequence":["first",2]}
    });
    let content =
        json!({"application/json":{"schema":{"$ref":"#/components/schemas/ScopedRecord"}}});
    let sequence =
        json!({"application/json":{"schema":{"$ref":"#/components/schemas/ScopedSequence"}}});
    json!({"openapi":"3.1.2","info":{"title":"Native scoped schema operations","version":"1"},"servers":[{"url":"https://example.test/v2"}],"security":[],
        "paths":{
            "/records":{"post":{"operationId":"echoScoped","requestBody":{"required":true,"content":content},"responses":{"200":{"description":"checked record","content":content},"422":{"description":"typed checked failure","content":content}}}},
            "/read":{"get":{"operationId":"readScoped","responses":{"200":{"description":"checked record","content":content}}}},
            "/sequence":{"post":{"operationId":"sendSequence","requestBody":{"required":true,"content":sequence},"responses":{"200":{"description":"heterogeneous checked sequence","content":sequence}}}}
        },
        "components":{"schemas":{
            "ScopedRecord":record,
            "ScopedChoice":{"anyOf":[{"properties":{"a":{"type":"string"}},"required":["a"]},{"properties":{"b":{"type":"integer"}},"required":["b"]}],"unevaluatedProperties":false,"example":{"a":"x","b":2}},
            "ScopedSequence":{"type":"array","prefixItems":[{"type":"string"}],"items":{"type":"number"},"contains":{"type":"integer","minimum":2},"minContains":1,"maxContains":1,"unevaluatedItems":false,"example":["first",2]},
            "ScopedNode":{"type":"object","required":["value"],"properties":{"value":{"type":"integer"},"next":{"$ref":"#/components/schemas/ScopedNode"}},"unevaluatedProperties":false,"example":{"value":1}}
        }}
    })
}
fn sdk_plan(root: &Path) -> SdkPlan {
    let file = root.join("operations.json");
    std::fs::write(&file, sdk_document().to_string()).unwrap();
    let contract = load(&file);
    let operations = contract
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    java_sdk::plan_sdk_with_maven(
        contract,
        &operations,
        PackageConfig {
            package: "example.schemav2".into(),
            version: "1.0.0".into(),
            api_name: "Client".into(),
        },
        &[],
        MavenConfig {
            artifact_id: "java-schema-v2-operations".into(),
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn scoped_native_fields_extras_carriers_and_compatibility_are_checked() {
    use suspect_codegen::{
        backend::{Backend, TargetConfig},
        compatibility,
    };
    let root = attempt();
    let plan = sdk_plan(&root);
    assert_eq!(plan.program().version, OwnedProgram::V2_VERSION);
    assert!(
        plan.native_examples()
            .iter()
            .all(|e| e.input_expression.is_some()),
        "scoped native examples were lost"
    );
    assert!(
        plan.examples().diagnostics().is_empty(),
        "{:?}",
        plan.examples().diagnostics()
    );
    let native = compatibility::snapshot(
        plan.contract().clone(),
        &[],
        &[TargetConfig {
            backend: Backend::JavaHttp,
            package_name: "example.schemav2:java-schema-v2-operations".into(),
            package_version: "1.0.0".into(),
            import_name: Some("example.schemav2".into()),
        }],
    )
    .unwrap()
    .native
    .remove(0);
    assert_eq!(
        native.status,
        compatibility::PlanStatus::Planned,
        "{:?}",
        native.findings
    );
    let record = native
        .models
        .iter()
        .find(|m| m.name == "example.schemav2.ScopedRecord" && m.role == "model")
        .unwrap()
        .descriptor
        .as_ref()
        .unwrap();
    let field = |name: &str| {
        record["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == name)
            .unwrap()
    };
    assert_eq!(field("name")["type"]["name"], "java.lang.String");
    assert_eq!(
        field("enabled")["type"]["name"],
        "example.schemav2.Presence"
    );
    assert_eq!(field("enabled")["type"]["arguments"][0]["kind"], "nullable");
    assert_eq!(
        field("additionalProperties")["type"]["arguments"][1]["name"],
        "example.schemav2.JsonRuntime.JsonValue"
    );
    assert_eq!(
        field("additionalProperties")["builderSetter"]["name"],
        "putAdditionalProperty"
    );
    assert_eq!(
        record["requiredAdditionalProperties"],
        json!(["x-required"])
    );
    for name in ["ScopedChoice", "ScopedSequence"] {
        let model = native
            .models
            .iter()
            .find(|m| m.name == format!("example.schemav2.{name}") && m.role == "model")
            .unwrap();
        assert_eq!(
            model.descriptor.as_ref().unwrap()["nativeType"]["name"],
            "example.schemav2.JsonRuntime.JsonValue"
        );
    }
}

#[test]
fn resource_and_dynamic_profiles_remain_explicitly_refused() {
    use suspect_codegen::http_protocol::Capability;
    let capabilities = java_sdk::protocol::capabilities(&Default::default());
    assert!(!capabilities.supports(Capability::SchemaResources));
    assert!(!capabilities.supports(Capability::DynamicSchemaReferences));
    let root = attempt();
    let (contract, schema) = schema_at(
        &root,
        "dynamic-refusal",
        json!({"$dynamicRef":"#/components/schemas/Root"}),
    );
    let errors = java_sdk::validation::plan_validation(&contract, &[schema]).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.source.pointer().ends_with("/$dynamicRef") && e.at.end > e.at.start),
        "{errors:?}"
    );
}

#[test]
#[ignore = "actual scoped SDK operations, positive/negative consumers, examples and Javadoc on JDK21/25"]
fn native_schema_v2_sdk_operations() {
    let root = attempt();
    let plan = sdk_plan(&root);
    let jar = install(&plan, &root);
    assert_eq!(plan.program().version, OwnedProgram::V2_VERSION);
    checked(
        Command::new(java().join("bin/java"))
            .args(["-ea", "-cp"])
            .arg(&jar)
            .arg("example.schemav2.SdkExamples"),
        &root,
    );
    std::fs::write(
        root.join("NativeSchemaV2Operations.java"),
        include_str!("../src/java_sdk/NativeSchemaV2Operations.java"),
    )
    .unwrap();
    checked(
        Command::new(java().join("bin/javac"))
            .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
            .arg(&jar)
            .arg(root.join("NativeSchemaV2Operations.java")),
        &root,
    );
    checked(
        Command::new(java().join("bin/java"))
            .args(["-ea", "-cp"])
            .arg(format!("{}:{}", jar.display(), root.display()))
            .arg("NativeSchemaV2Operations"),
        &root,
    );
    let quickstart = root.join("java/examples/GettingStarted.java");
    assert!(quickstart.is_file());
    checked(
        Command::new(java().join("bin/javac"))
            .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
            .arg(&jar)
            .arg("-d")
            .arg(&root)
            .arg(quickstart),
        &root,
    );
    for (index, body) in [
        "ScopedRecord.builder();",
        "ScopedRecord.builder(\"x\").putAdditionalProperty(\"x-required\", 1);",
        "ScopedRecord.builder(\"x\").enabled(\"false\");",
        "ScopedRecord.builder(\"x\").choice(java.util.Map.of(\"a\", \"x\"));",
        "java.util.List<String> value=ScopedSequence.decode(\"[\\\"first\\\",2]\");",
        "new ScopedChoice();",
        "client.echoScoped(EchoScopedInput.builder(JsonNull.INSTANCE).build());",
    ]
    .iter()
    .enumerate()
    {
        let file = root.join(format!("Negative{index}.java"));
        std::fs::write(&file,format!("import example.schemav2.*; import example.schemav2.Client.*; import static example.schemav2.JsonRuntime.*; final class Negative{index} {{ void call(Client client) {{ {body} }} }}")).unwrap();
        let output = Command::new(java().join("bin/javac"))
            .args(["--release", "21", "-cp"])
            .arg(&jar)
            .arg(&file)
            .output()
            .unwrap();
        std::fs::write(
            root.join(format!("negative-{index}.log")),
            format!(
                "status={}\n{}{}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
        )
        .unwrap();
        assert!(
            !output.status.success(),
            "invalid native consumer {index} compiled"
        );
    }
    for name in [
        "ScopedRecord",
        "ScopedChoice",
        "ScopedSequence",
        "ScopedNode",
    ] {
        assert!(
            root.join(format!(
                "java/target/reports/apidocs/example/schemav2/{name}.html"
            ))
            .is_file(),
            "missing Javadoc for {name}"
        );
    }
    println!("JAVA_SCHEMA_V2_OPERATIONS {}", root.display());
}
