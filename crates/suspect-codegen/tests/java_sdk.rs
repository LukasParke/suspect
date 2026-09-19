//! Native jar consumers of the same M2 contract. No generated file is repaired.
#![cfg(feature = "java-sdk")]
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::java_sdk::{PackageConfig, SdkPlan, plan_sdk};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

static NATIVE_GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn crate_root() -> PathBuf {
    std::env::var_os("SUSPECT_JAVA_CRATE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
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
fn contract_from(schemas: serde_json::Value) -> Arc<Contract> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(&path, json!({"openapi":"3.1.0","info":{"title":"Java SDK test","version":"1"},"components":{"schemas":schemas}}).to_string()).unwrap();
    load(&path)
}
fn document(value: serde_json::Value) -> Arc<Contract> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(&path, value.to_string()).unwrap();
    load(&path)
}
fn enveloped(paths: serde_json::Value, schemas: serde_json::Value) -> serde_json::Value {
    json!({"openapi":"3.1.0","info":{"title":"Java native acceptance","version":"1"},"servers":[{"url":"https://example.test/api/v1"}],"security":[{"apiKey":[]}],"paths":paths,"components":{"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}},"schemas":schemas}})
}
fn config() -> PackageConfig {
    PackageConfig {
        package: "com.example.generated".into(),
        version: "0.1.0".into(),
        api_name: "OpenRouter".into(),
    }
}
fn canonical() -> SdkPlan {
    let path = crate_root().join("tests/fixtures/m2/canonical.openapi.yaml");
    let contract = load(&path);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    plan_sdk(contract, &selected, config(), &[]).unwrap()
}
#[test]
fn malformed_and_unsupported_inputs_do_not_emit_a_package() {
    let contract = contract_from(json!({"Thing":{"type":"object"}}));
    for package in ["com..x", "com.class", "../escape"] {
        let errors = plan_sdk(
            contract.clone(),
            &[],
            PackageConfig {
                package: package.into(),
                ..config()
            },
            contract.schema_roots(),
        )
        .unwrap_err();
        assert!(errors.iter().any(|e| e.code == "java-package-invalid"));
    }
    let contract = contract_from(
        json!({"Direction":{"type":"object","properties":{"secret":{"type":"string","writeOnly":true}}}}),
    );
    assert!(
        plan_sdk(contract.clone(), &[], config(), contract.schema_roots())
            .unwrap_err()
            .iter()
            .any(|e| e.code == "java-directional-codec-unsupported")
    );
    let contract = contract_from(json!({"Mixed":{"enum":[true,"x",1,null]}}));
    assert!(plan_sdk(contract.clone(), &[], config(), contract.schema_roots()).is_ok());
}
#[test]
fn canonical_closure_has_source_bound_codecs_and_real_client_artifacts() {
    let plan = canonical();
    assert_eq!(plan.operations().len(), 4);
    let files = plan.render().unwrap();
    assert_eq!(
        files.len(),
        files
            .iter()
            .map(|f| &f.path)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    );
    assert!(
        files
            .iter()
            .any(|f| f.path.ends_with("validation-program.json"))
    );
    assert!(
        files
            .iter()
            .any(|f| f.path.ends_with("OpenRouter.java") && f.content.contains("runtime.call"))
    );
    let widget = plan
        .models()
        .symbols()
        .iter()
        .find(|s| s.name() == "Widget")
        .unwrap();
    assert_eq!(widget.codec().native_type, "Widget");
    assert!(
        plan.models()
            .fields()
            .iter()
            .any(|f| f.model == "Widget" && f.field == "meta" && f.nullable && !f.required)
    );
    assert!(
        plan.models()
            .fields()
            .iter()
            .any(|f| f.model == "Widget" && f.field == "amount" && f.native_type == "JsonNumber")
    );
    assert_eq!(files, plan.render().unwrap());
}
#[test]
fn validation_uses_checked_instructions_and_reports_unsupported_schema_assertions() {
    let contract = contract_from(json!({"Pat":{"type":"string","pattern":"^a+$"}}));
    let program =
        suspect_codegen::java_sdk::validation::plan_validation(&contract, contract.schema_roots())
            .unwrap();
    assert!(program.check().is_ok());
    let contract =
        contract_from(json!({"Scoped":{"type":"object","dependentRequired":{"a":["b"]}}}));
    assert!(
        suspect_codegen::java_sdk::validation::plan_validation(&contract, contract.schema_roots())
            .is_ok()
    );
    let contract =
        contract_from(json!({"Unsupported":{"$dynamicRef":"#/components/schemas/Unsupported"}}));
    assert!(
        suspect_codegen::java_sdk::validation::plan_validation(&contract, contract.schema_roots())
            .is_err()
    );
}

#[test]
fn unsupported_shapes_and_wire_modes_fail_at_original_sources() {
    for schema in [
        json!({"type":["string","number"]}),
        json!({"type":"array","prefixItems":[{"type":"string"}]}),
        json!({"type":"string","anyOf":[{"const":"x"},{"const":"y"}]}),
    ] {
        let contract = contract_from(json!({"Thing":schema,"Text":{"type":"string"}}));
        let errors =
            plan_sdk(contract.clone(), &[], config(), contract.schema_roots()).unwrap_err();
        assert!(
            errors.iter().any(
                |e| e.source.pointer().starts_with("/components/schemas/Thing")
                    && e.at.end > e.at.start
            ),
            "{errors:#?}"
        );
    }
    for path in [
        "/x/{",
        "/x/}",
        "/x/%",
        "/x/%2e%2E",
        "/x/..",
        "/x?query",
        "/x/%FF",
    ] {
        let contract = document(enveloped(
            json!({path:{"get":{"operationId":"getX","responses":{"200":{"description":"OK","content":{"application/json":{"schema":{"type":"string"}}}}}}}}),
            json!({}),
        ));
        let operations = contract
            .operations()
            .map(|o| o.source().clone())
            .collect::<Vec<_>>();
        assert!(
            plan_sdk(contract, &operations, config(), &[])
                .unwrap_err()
                .iter()
                .any(|d| matches!(
                    d.code,
                    "java-path-unsupported" | "http-path-template" | "http-path-parameters"
                )),
            "{path}"
        );
    }
    for change in ["media", "range", "headers", "optional-auth", "querystring"] {
        let mut value = enveloped(
            json!({"/x":{"get":{"operationId":"getX","responses":{"200":{"description":"OK","content":{"application/json":{"schema":{"type":"string"}}}}}}}}),
            json!({}),
        );
        let operation = &mut value["paths"]["/x"]["get"];
        match change {
            "media" => {
                operation["responses"]["200"]["content"] =
                    json!({"text/plain":{"schema":{"type":"string"}}})
            }
            "range" => {
                operation["responses"] = json!({"2XX":{"description":"OK","content":{"application/json":{"schema":{"type":"string"}}}}})
            }
            "headers" => {
                operation["responses"]["200"]["headers"] =
                    json!({"X-Rate":{"schema":{"type":"integer"}}})
            }
            "optional-auth" => operation["security"] = json!([{}, {"apiKey":[]}]),
            _ => {
                operation["parameters"] =
                    json!([{"name":"all","in":"querystring","schema":{"type":"string"}}])
            }
        }
        let contract = document(value);
        let operations = contract
            .operations()
            .map(|o| o.source().clone())
            .collect::<Vec<_>>();
        let result = plan_sdk(contract, &operations, config(), &[]);
        if change == "querystring" {
            assert!(
                result.is_err(),
                "querystring requires OAS 3.2 content, not a schema field"
            );
        } else {
            assert!(
                result.is_ok(),
                "verified protocol {change} was rejected: {result:?}"
            );
        }
    }
}

fn adversarial() -> SdkPlan {
    let schemas = json!({
        "PresenceCase":{"type":"object","required":["required_nullable","required_string","tag"],"properties":{
            "required_nullable":{"type":["string","null"]},"required_string":{"type":"string"},"tag":{"const":"fixed"},
            "optional_nullable":{"type":["string","null"]},"optional_string":{"type":"string"},"amount":{"type":"number"},"raw":true,
            "items":{"type":"array","items":{"type":["string","null"]}}
        }},
        "RequiredJson":{"type":"object","required":["anything"],"properties":{"anything":true}},
        "NullableObject":{"type":["object","null"],"required":["name"],"properties":{"name":{"type":"string"}}},
        "NullableHolder":{"type":"object","required":["value"],"properties":{"value":{"$ref":"#/components/schemas/NullableObject"}}},
        "MixedLiteral":{"enum":["auto",true,1,null,{"a":[1]}]},
        "NumberEnum":{"type":"number","enum":[1,3.5]},
        "Configuration":{"type":"object","required":["name"],"properties":{"name":{"type":"string"}}},
        "MixedUnion":{"oneOf":[{"const":"auto"},{"$ref":"#/components/schemas/Configuration"}]},
        "StringUnion":{"oneOf":[{"type":"string","minLength":3},{"type":"string","maxLength":2}]},
        "Overlap":{"oneOf":[{"type":"integer"},{"type":"number"}]},
        "NullUnion":{"oneOf":[{"type":"null"},{"type":"string"}]},
        "Inclusive":{"anyOf":[{"type":"object","properties":{"a":{"type":"string"}},"required":["a"]},{"type":"object","properties":{"b":{"type":"number"}},"required":["b"]}]},
        "Recursive":{"type":"object","properties":{"name":{"type":"string"},"child":{"$ref":"#/components/schemas/Recursive"}},"required":["name"]},
        "RecursiveUnion":{"oneOf":[{"type":"string"},{"type":"array","items":{"$ref":"#/components/schemas/RecursiveUnion"}}]},
        "Ledger":{"type":"object","additionalProperties":{"type":"number"}},
        "Impossible":false
    });
    let contract = contract_from(schemas);
    plan_sdk(contract.clone(), &[], config(), contract.schema_roots()).unwrap()
}

#[test]
fn native_descriptors_retain_construction_literals_arms_and_codec_bindings() {
    use suspect_codegen::java_sdk::models::JavaDeclaration;
    let plan = adversarial();
    for symbol in plan.models().symbols() {
        let binding = symbol.codec();
        assert_eq!(
            plan.program().nodes[binding.root].source.pointer,
            symbol.source().pointer()
        );
        assert_eq!(binding.holder, symbol.name());
        assert_eq!(
            binding.native_type,
            plan.models().native_type(symbol.source())
        );
    }
    let case = plan
        .models()
        .symbols()
        .iter()
        .find(|s| s.name() == "PresenceCase")
        .unwrap();
    let JavaDeclaration::Object {
        fields,
        constructor,
        ..
    } = case.declaration()
    else {
        panic!("native object missing")
    };
    assert_eq!(
        constructor
            .arguments
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>(),
        ["requiredNullable", "requiredString"]
    );
    assert!(
        fields
            .iter()
            .any(|f| f.name == "tag" && f.fixed == Some(json!("fixed")))
    );
    let union = plan
        .models()
        .symbols()
        .iter()
        .find(|s| s.name() == "MixedUnion")
        .unwrap();
    let JavaDeclaration::Union {
        exclusive,
        variants,
    } = union.declaration()
    else {
        panic!("native union missing")
    };
    assert!(*exclusive);
    assert_eq!(
        variants.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(),
        ["Variant1", "Variant2"]
    );
    assert!(
        plan.render()
            .unwrap()
            .iter()
            .all(|f| f.path.starts_with("java/"))
    );
}
#[test]
fn metadata_loading_limits_are_admitted_before_emission() {
    let mut nested = json!(0);
    for _ in 0..125 {
        nested = json!([nested]);
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("deep.yaml");
    std::fs::write(&path, format!("openapi: 3.1.0\ninfo: {{title: Java, version: '1'}}\ncomponents:\n  schemas:\n    DeepLiteral:\n      const: {nested}\n")).unwrap();
    let contract = load(&path);
    let errors = plan_sdk(contract.clone(), &[], config(), contract.schema_roots()).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.code == "java-validation-resource-limit"),
        "{errors:#?}"
    );
    // Individually bounded operands must also fit the total metadata work policy.
    let number: serde_json::Value =
        serde_json::from_str(&format!("1e{}", "9".repeat(4090))).unwrap();
    let schemas: serde_json::Map<_, _> = (0..40)
        .map(|i| (format!("Literal{i}"), json!({"const":number})))
        .collect();
    let contract = contract_from(serde_json::Value::Object(schemas));
    let errors = plan_sdk(contract.clone(), &[], config(), contract.schema_roots()).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.code == "java-validation-resource-limit"),
        "{errors:#?}"
    );
}

fn checked(command: &mut Command, root: &Path) {
    let output = command
        .output()
        .unwrap_or_else(|e| panic!("{command:?}: {e}"));
    let logs = root.join("java/logs");
    std::fs::create_dir_all(&logs).unwrap();
    let index = std::fs::read_dir(&logs).unwrap().count();
    std::fs::write(
        logs.join(format!("{index:03}.log")),
        format!(
            "{command:?}\nstatus={}\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "fixture retained at {}\n{command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn java_home() -> PathBuf {
    std::env::var_os("SUSPECT_JAVA_HOME")
        .or_else(|| std::env::var_os("JAVA_HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            "/Users/luke/.local/share/mise/installs/java/temurin-21.0.12+101.0.LTS".into()
        })
}
fn native_package(plan: &SdkPlan, consumer: &str) -> PathBuf {
    let base = crate_root().join("../../target/sdk-java-native");
    std::fs::create_dir_all(&base).unwrap();
    let root = tempfile::Builder::new()
        .prefix("sdk-")
        .tempdir_in(base)
        .unwrap()
        .keep();
    for file in plan.render().unwrap() {
        let path = root.join(file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, file.content).unwrap();
    }
    let home = java_home();
    let bin = home.join("bin");
    assert!(
        bin.join("javac").is_file(),
        "required JDK: {}",
        home.display()
    );
    let maven = std::env::var_os("SUSPECT_MAVEN_BIN").unwrap_or_else(|| {
        "/Users/luke/.local/share/mise/installs/maven/3.9.16/apache-maven-3.9.16/bin/mvn".into()
    });
    let repository = crate_root().join("../../target/sdk-java-maven-cache/java/repository");
    checked(Command::new(bin.join("java")).arg("-version"), &root);
    checked(
        Command::new(&maven).arg("-version").env("JAVA_HOME", &home),
        &root,
    );
    checked(
        Command::new(maven)
            .args(["-B", "-q", "install"])
            .arg(format!("-Dmaven.repo.local={}", repository.display()))
            .current_dir(root.join("java"))
            .env("JAVA_HOME", &home),
        &root,
    );
    let name = format!("{}-{}", plan.maven().artifact_id, plan.package().version);
    let jar = root.join(format!("java/target/{name}.jar"));
    assert!(jar.is_file());
    assert!(
        root.join(format!("java/target/{name}-javadoc.jar"))
            .is_file()
    );
    assert!(
        root.join(format!("java/target/{name}-sources.jar"))
            .is_file()
    );
    let installed = repository
        .join(plan.maven_group_id().replace('.', "/"))
        .join(&plan.maven().artifact_id)
        .join(&plan.package().version)
        .join(format!("{name}.jar"));
    assert_eq!(
        std::fs::read(&jar).unwrap(),
        std::fs::read(&installed).unwrap(),
        "installed jar differs from emitted package"
    );
    std::fs::create_dir_all(root.join("java/consumer")).unwrap();
    std::fs::write(
        root.join("java/consumer/openrouter-five-responses.json"),
        include_bytes!("fixtures/openrouter-five-responses.json"),
    )
    .unwrap();
    std::fs::copy(&installed, root.join("java/consumer/sdk.jar")).unwrap();
    native_docs(plan, &root);
    checked(
        Command::new(bin.join("java"))
            .args(["-ea", "-cp"])
            .arg(&jar)
            .arg(format!("{}.SdkExamples", plan.package().package)),
        &root,
    );
    std::fs::write(root.join("java/consumer/Consumer.java"), consumer).unwrap();
    checked(
        Command::new(bin.join("javac"))
            .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
            .arg("sdk.jar:.")
            .arg("Consumer.java")
            .current_dir(root.join("java/consumer")),
        &root,
    );
    checked(
        Command::new(bin.join("java"))
            .args(["-ea", "-cp"])
            .arg("sdk.jar:.")
            .arg("Consumer")
            .current_dir(root.join("java/consumer")),
        &root,
    );
    root
}

fn native_docs(plan: &SdkPlan, root: &Path) {
    let docs = root
        .join("java/target/reports/apidocs")
        .join(plan.package().package.replace('.', "/"));
    assert!(docs.is_dir(), "native Javadoc missing: {}", docs.display());
    for symbol in plan.models().symbols() {
        let text = std::fs::read_to_string(docs.join(format!("{}.html", symbol.name()))).unwrap();
        assert!(
            text.contains("id=\"CODEC\""),
            "missing codec docs: {}",
            symbol.name()
        );
        assert!(
            text.contains(symbol.source().pointer()),
            "missing source docs: {}",
            symbol.name()
        );
    }
    for field in plan.models().fields() {
        let text = std::fs::read_to_string(docs.join(format!("{}.html", field.model))).unwrap();
        assert!(
            text.contains(&format!("id=\"{}()\"", field.field)),
            "missing native member docs: {}.{}",
            field.model,
            field.field
        );
    }
    let client =
        std::fs::read_to_string(docs.join(format!("{}.html", plan.package().api_name))).unwrap();
    for operation in plan.operations() {
        assert!(client.contains(&format!("id=\"{}(", operation.method_name)));
        assert!(client.contains(&format!("id=\"{}(", operation.async_method_name)));
        assert!(
            docs.join(format!(
                "{}.{}.html",
                plan.package().api_name,
                operation.input_type
            ))
            .is_file()
        );
        for response in &operation.responses {
            assert!(
                docs.join(format!(
                    "{}.{}.html",
                    plan.package().api_name,
                    response.variant_name
                ))
                .is_file()
            );
        }
    }
    assert!(
        plan.native_examples()
            .iter()
            .all(|e| e.input_expression.is_some()
                && e.entries.iter().all(|e| e.expression.is_some())),
        "native example coverage unavailable"
    );
    let quickstart = root.join("java/examples/GettingStarted.java");
    if quickstart.is_file() {
        checked(
            Command::new(java_home().join("bin/javac"))
                .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
                .arg(root.join("java/consumer/sdk.jar"))
                .arg("-d")
                .arg(root.join("java/consumer"))
                .arg(quickstart),
            root,
        );
    }
}

fn run_native(root: &Path, class: &str, sources: &[(&str, &str)], args: &[&str]) {
    let consumer = root.join("java/consumer");
    let mut compiler = Command::new(java_home().join("bin/javac"));
    compiler.args([
        "--release",
        "21",
        "-Xlint:all",
        "-Werror",
        "-cp",
        "sdk.jar:.",
    ]);
    for (name, source) in sources {
        std::fs::write(consumer.join(name), source).unwrap();
        compiler.arg(name);
    }
    checked(compiler.current_dir(&consumer), root);
    checked(
        Command::new(java_home().join("bin/java"))
            .args(["-ea", "-cp", "sdk.jar:."])
            .arg(class)
            .args(args)
            .current_dir(consumer),
        root,
    );
}

fn runtime_vectors(root: &Path) {
    let vectors: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/runtime-contract-v1.json")).unwrap();
    let schemas: serde_json::Map<_, _> = vectors["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| (v["name"].as_str().unwrap().to_owned(), v["schema"].clone()))
        .collect();
    let contract = contract_from(serde_json::Value::Object(schemas));
    let program =
        suspect_codegen::java_sdk::validation::plan_validation(&contract, contract.schema_roots())
            .unwrap();
    let data = root.join("java/consumer");
    std::fs::write(
        data.join("vectors-program.json"),
        serde_json::to_vec(&program).unwrap(),
    )
    .unwrap();
    std::fs::write(
        data.join("runtime-contract-v1.json"),
        include_bytes!("fixtures/runtime-contract-v1.json"),
    )
    .unwrap();
    let recursive = contract_from(json!({
        "Cycle":{"$ref":"#/components/schemas/Cycle"},
        "AnyCycle":{"anyOf":[true,{"$ref":"#/components/schemas/Cycle"}]},
        "NotCycle":{"not":{"$ref":"#/components/schemas/Cycle"}},
        "Recursive":{"type":"object","properties":{"next":{"$ref":"#/components/schemas/Recursive"}}},
        "UnicodePattern":{"type":"string","pattern":"^😀[雪]+$"}
    }));
    let program = suspect_codegen::java_sdk::validation::plan_validation(
        &recursive,
        recursive.schema_roots(),
    )
    .unwrap();
    std::fs::write(
        data.join("recursive-program.json"),
        serde_json::to_vec(&program).unwrap(),
    )
    .unwrap();
    run_native(
        root,
        "NativeValidation",
        &[(
            "NativeValidation.java",
            include_str!("../src/java_sdk/NativeValidation.java"),
        )],
        &["."],
    );
}
#[test]
#[ignore = "requires JDK 21+ and Maven; real jar, strict types, Javadoc and independent loopback wire"]
fn maven_jar_consumer_exact_models_types_and_sync_async_wire() {
    let _gate = NATIVE_GATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = native_package(&canonical(), CONSUMER);
    run_native(
        &root,
        "NativeTransport",
        &[
            (
                "NativeSupport.java",
                include_str!("../src/java_sdk/NativeSupport.java"),
            ),
            (
                "NativeTransport.java",
                include_str!("../src/java_sdk/NativeTransport.java"),
            ),
        ],
        &[],
    );
    runtime_vectors(&root);
    for source in [
        "import com.example.generated.*; class Invalid { Object value = WidgetInput.builder(); }",
        "import com.example.generated.*; class Invalid { Object value = WidgetInput.builder(42); }",
        "import com.example.generated.*; class Invalid { void f(Widget w) { w.amount = null; } }",
        "import com.example.generated.*; class Invalid { Object value = new WidgetPayload.Variant1(SecurePayload.builder(\"v\").build()); }",
    ] {
        std::fs::write(root.join("java/consumer/Invalid.java"), source).unwrap();
        let output = Command::new(java_home().join("bin/javac"))
            .args(["--release", "21", "-cp"])
            .arg("sdk.jar")
            .arg("Invalid.java")
            .current_dir(root.join("java/consumer"))
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "negative native typing unexpectedly passed: {source}"
        );
    }
    println!("Java native evidence retained at {}", root.display());
}

#[test]
#[ignore = "required native model/union/literal/mutation/resource checks on JDK 21 and 25"]
fn native_immutable_models_unions_literals_and_shared_budgets() {
    let _gate = NATIVE_GATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = native_package(
        &adversarial(),
        "public final class Consumer { private Consumer() {} public static void main(String[] args) {} }",
    );
    run_native(
        &root,
        "NativeModels",
        &[(
            "NativeModels.java",
            include_str!("../src/java_sdk/NativeModels.java"),
        )],
        &[],
    );
    negative_types(
        &root,
        &[
            "Object value = PresenceCase.builder(\"missing-second-argument\");",
            "Object value = new MixedLiteral(\"auto\");",
            "Object value = new NullUnion.Variant1(\"null\");",
            "Object value = new MixedUnion.Variant2(\"wrong-object-type\");",
            "Object value = PresenceCase.builder(null, \"x\").amount(1.5);",
            "Object value = new Never();",
            "Object value = StringUnion.read(com.example.generated.JsonRuntime.parse(\"\\\"long\\\"\"), null);",
        ],
    );
    println!("Java model evidence retained at {}", root.display());
}

fn negative_types(root: &Path, cases: &[&str]) {
    for (index, body) in cases.iter().enumerate() {
        let consumer = root.join("java/consumer");
        let path = consumer.join("Negative.java");
        std::fs::write(&path, format!("import com.example.generated.*; import com.example.generated.OpenRouter.*; class Negative {{ {body} }}")).unwrap();
        let command = &mut Command::new(java_home().join("bin/javac"));
        command
            .args([
                "--release",
                "21",
                "-Xlint:all",
                "-Werror",
                "-cp",
                "sdk.jar",
                "Negative.java",
            ])
            .current_dir(&consumer);
        let output = command.output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        std::fs::write(
            root.join(format!("java/logs/negative-{index}.log")),
            format!("{body}\n{stderr}"),
        )
        .unwrap();
        assert!(
            !output.status.success(),
            "negative native type passed: {body}"
        );
        assert!(
            !stderr.contains("does not exist") && !stderr.contains("bad class file"),
            "negative case failed due to installation/import: {stderr}"
        );
    }
}

fn openrouter() -> (SdkPlan, PathBuf) {
    let path = std::env::var_os("OPENROUTER_WEB_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/Users/luke/github/openrouter-web".into())
        .join("projects/docs/openapi/openapi.yaml");
    assert!(
        path.is_file(),
        "actual OpenRouter source required: {}",
        path.display()
    );
    let contract = load(&path);
    let wanted = [
        "getCredits",
        "createKeys",
        "updateKeys",
        "listContainerFiles",
        "getContainerFile",
    ];
    let selected = contract
        .operations()
        .filter(|op| op.operation_id().is_some_and(|id| wanted.contains(&id)))
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 5);
    (plan_sdk(contract, &selected, config(), &[]).unwrap(), path)
}

#[test]
#[ignore = "required installed consumer of five actual OpenRouter operations with independent response bytes"]
fn native_five_actual_openrouter_operations_and_constructor_examples() {
    let _gate = NATIVE_GATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (plan, source) = openrouter();
    let before = std::fs::read(&source).unwrap();
    assert!(
        plan.models()
            .symbols()
            .iter()
            .any(|s| s.name() == "CreateKeysRequest")
    );
    assert!(
        plan.models()
            .symbols()
            .iter()
            .any(|s| s.name() == "GetCreditsResponse")
    );
    let root = native_package(&plan, include_str!("../src/java_sdk/NativeOpenRouter.java"));
    negative_types(
        &root,
        &[
            "Object value = CreateKeysRequest.builder();",
            "Object value = CreateKeysRequest.builder(\"key\").limitReset(\"monthly\");",
            "Object value = CreateKeysRequest.builder(\"key\").limit(1.0);",
            "Object value = GetContainerFileInput.builder(\"container\");",
        ],
    );
    assert_eq!(
        before,
        std::fs::read(&source).unwrap(),
        "OpenRouter source changed during the native gate"
    );
    println!("Java OpenRouter evidence retained at {}", root.display());
}

fn collisions() -> SdkPlan {
    let mut schemas = serde_json::Map::new();
    for name in [
        "Builder",
        "String",
        "Boolean",
        "Object",
        "Class",
        "Record",
        "Enum",
        "Void",
        "Thread",
        "HttpClient",
        "CompletableFuture",
        "Map",
        "map",
        "List",
        "Set",
        "TimeoutException",
        "JsonNumber",
        "OpenRouter",
        "CloseInput",
        "SdkExamples",
    ] {
        schemas.insert(
            name.into(),
            json!({"type":"object","properties":{"value":{"type":"string"}}}),
        );
    }
    let properties: serde_json::Map<_, _> = [
        "builder",
        "build",
        "body",
        "c",
        "input",
        "parameters",
        "value",
        "read",
        "write",
        "hashCode",
        "hash_code",
        "getClass",
        "class",
        "additionalProperties",
        "putAdditionalProperty",
        "foo-bar",
        "foo_bar",
        "fooBar",
        "properties",
        "items",
        "oneOf",
        "a/b~雪",
    ]
    .into_iter()
    .map(|name| (name.into(), json!({"type":"string"})))
    .collect();
    schemas.insert("Clash".into(), json!({"type":"object","properties":properties,"description":"Names and inert docs: */ \\u000a @link <script>&"}));
    let response = json!({"description":"OK","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Clash"}}}});
    let contract = document(enveloped(
        json!({
            "/names/{client}":{"post":{"operationId":"close","parameters":[
                {"name":"client","in":"path","required":true,"schema":{"type":"string","example":"source-client"}},
                {"name":"body","in":"query","schema":{"type":"string","example":"query-body"}},
                {"name":"input","in":"query","schema":{"type":"string","example":"query-input"}},
                {"name":"builder","in":"query","schema":{"type":"string","example":"query-builder"}}
            ],"requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Clash"},"example":{}}}},"responses":{"200":response,"201":response}}},
            "/async":{"get":{"operationId":"closeAsync","responses":{"200":response}}},
            "/class":{"get":{"operationId":"getClass","responses":{"200":response}}}
        }),
        serde_json::Value::Object(schemas),
    ));
    let selected = contract
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    plan_sdk(
        contract.clone(),
        &selected,
        config(),
        contract.schema_roots(),
    )
    .unwrap()
}

#[test]
fn symbols_are_readable_collision_safe_and_source_bound() {
    let plan = collisions();
    let names: std::collections::BTreeSet<_> = plan
        .models()
        .symbols()
        .iter()
        .map(|s| s.name().to_ascii_lowercase())
        .collect();
    assert_eq!(names.len(), plan.models().symbols().len());
    assert!(plan.models().symbols().iter().any(|s| s.source().pointer()
        == "/components/schemas/TimeoutException"
        && s.name() != "TimeoutException"));
    let close = plan
        .operations()
        .iter()
        .find(|o| o.operation_id == "close")
        .unwrap();
    assert_eq!(close.method_name, "close2");
    assert_ne!(close.input_type, "CloseInput");
    let methods = plan
        .operations()
        .iter()
        .flat_map(|o| [&o.method_name, &o.async_method_name])
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(methods.len(), plan.operations().len() * 2);
    let files = plan.render().unwrap();
    let paths = files
        .iter()
        .map(|f| f.path.to_ascii_lowercase())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(paths.len(), files.len());
    let examples = files
        .iter()
        .find(|f| f.path.ends_with("/SdkExamples.java"))
        .unwrap();
    assert!(
        examples.content.contains("Clash.builder()")
            && !examples.content.contains(".decode(\"{}\")")
    );
}

#[test]
#[ignore = "required native runtime/JDK/member/operation naming and executable-example collisions"]
fn native_names_status_alternatives_and_doc_examples() {
    use suspect_codegen::java_sdk::models::JavaDeclaration;
    let _gate = NATIVE_GATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let plan = collisions();
    let close = plan
        .operations()
        .iter()
        .find(|o| o.operation_id == "close")
        .unwrap();
    let clash = plan
        .models()
        .symbols()
        .iter()
        .find(|s| s.name() == "Clash")
        .unwrap();
    let JavaDeclaration::Object { fields, .. } = clash.declaration() else {
        panic!("Clash model")
    };
    let setters = fields
        .iter()
        .map(|f| format!(".{}(\"kept\")", f.name))
        .collect::<String>();
    let getter_checks = fields.iter().map(|f|format!("if (!value.{}().value().equals(\"kept\")) throw new AssertionError(\"member collision\");",f.name)).collect::<String>();
    let holder_checks = plan
        .models()
        .symbols()
        .iter()
        .filter(|s| {
            s.source().pointer().starts_with("/components/schemas/")
                && matches!(s.declaration(), JavaDeclaration::Object { .. })
        })
        .map(|s| format!("{}.CODEC.encode({}.builder().build());", s.name(), s.name()))
        .collect::<String>();
    let ordinary = "public final class Consumer { private Consumer() {} public static void main(String[] args) {} }";
    let root = native_package(&plan, ordinary);
    let source = format!(
        r#"
import com.example.generated.*;
import com.example.generated.OpenRouter.*;
import static com.example.generated.JsonRuntime.*;
public final class NativeNames {{
    private NativeNames() {{}}
    public static void main(String[] args) {{
        var value = Clash.builder(){setters}.build();
        {getter_checks}
        {holder_checks}
        var transport = new NativeSupport.Mock(new NativeSupport.Script(200, "application/json", "{{}}"));
        try (var client = new OpenRouter(HttpRuntime.Options.builder().credential("apiKey", "name-token").httpClient(transport).build())) {{
            var input = {input}.builder("a/b", value).{body}("query-body").{query_input}("query-input").{builder}("query-builder").build();
            var result = client.{method}(input);
            if (!(result instanceof {status})) throw new AssertionError("status alternatives lost");
            String uri = transport.requests.getFirst().uri();
            if (!uri.equals("https://example.test/api/v1/names/a%2Fb?body=query-body&input=query-input&builder=query-builder")) throw new AssertionError(uri);
            SdkExamples.verify();
            GettingStarted.run(client);
        }}
        System.out.println("JAVA_NAME_COLLISIONS_OK: native types, model members, operation inputs, statuses, docs and constructor examples");
    }}
}}
"#,
        input = close.input_type,
        body = close.parameters[1].native_name,
        query_input = close.parameters[2].native_name,
        builder = close.parameters[3].native_name,
        method = close.method_name,
        status = close
            .responses
            .iter()
            .find(|r| r.wire.status() == suspect_codegen::http_protocol::ResponseStatus::Exact(200))
            .unwrap()
            .variant_name
    );
    run_native(
        &root,
        "NativeNames",
        &[
            (
                "NativeSupport.java",
                include_str!("../src/java_sdk/NativeSupport.java"),
            ),
            ("NativeNames.java", &source),
        ],
        &[],
    );
    println!("Java name evidence retained at {}", root.display());
}

const CONSUMER: &str = r#"
import com.example.generated.*;
import com.example.generated.OpenRouter.*;
import static com.example.generated.JsonRuntime.*;
import java.net.*;
import java.nio.charset.StandardCharsets;
import java.time.Duration;
import java.util.*;
import java.util.concurrent.*;
import com.sun.net.httpserver.HttpServer;

public final class Consumer {
    static final String WIDGET = "{\"id\":\"w1\",\"amount\":9007199254740993.000000000000000001,\"meta\":null,\"payload\":{\"kind\":\"standard\",\"text\":\"plain\"},\"child\":{\"label\":\"root\"}}";
    static final String PAGE = "{\"items\":[{\"id\":\"w2\",\"amount\":1e-400,\"payload\":{\"kind\":\"secure\",\"vault\":\"v1\"}}]}";
    static void bad(Runnable action) { try { action.run(); } catch (IllegalArgumentException expected) { return; } throw new AssertionError("invalid value accepted"); }
    public static void main(String[] args) throws Exception {
        for (String token : List.of("-", "01", "1.", "1e", "1e+", "+1", "NaN", "--1")) bad(() -> JsonNumber.parse(token));
        for (String text : List.of("{}true", "[1,]", "{\"a\":1,\"\\u0061\":2}", "\"\\ud800\"", "\"\n\"", "true false")) bad(() -> JsonRuntime.parse(text));
        bad(() -> JsonRuntime.parse(new byte[]{(byte)0xC0, (byte)0xAF}));
        JsonNumber huge = JsonNumber.parse("1e9999999999999999999999999999");
        assert huge.isInteger() && huge.compareTo(JsonNumber.parse("9e9999999999999999999999999998")) > 0;
        assert JsonNumber.parse("100e-2").equals(JsonNumber.parse("1.00"));
        assert JsonNumber.parse("100e-2").hashCode() == JsonNumber.parse("1.00").hashCode();
        assert JsonNumber.parse("-0").hashCode() == JsonNumber.parse("0e99999999999").hashCode();
        bad(() -> huge.exactIntegerValue());
        try { JsonNumber.parse("1.2").exactIntegerValue(); throw new AssertionError("fraction truncated"); } catch (ArithmeticException expected) {}
        Widget model = Widget.decode(WIDGET);
        assert model.meta().isPresent() && model.meta().value() == null;
        assert !model.child().value().child().isPresent();
        assert model.payload() instanceof WidgetPayload.Variant1 branch && branch.value().text().equals("plain");
        assert Widget.encode(model).contains("9007199254740993.000000000000000001");
        bad(() -> WidgetInput.builder("").build());
        bad(() -> WidgetInput.builder((String)null).build());
        bad(() -> WidgetPatch.builder().amount(null).build());
        bad(() -> Widget.decode(WIDGET.replace("standard", "unknown")));
        var mutable = new ArrayList<Widget>(); mutable.add(model);
        WidgetList snapshot = WidgetList.builder(mutable).build(); mutable.clear();
        assert snapshot.items().size() == 1;
        try { snapshot.items().clear(); throw new AssertionError("mutable model list"); } catch (UnsupportedOperationException expected) {}
        var seen = Collections.synchronizedList(new ArrayList<String>());
        var entered = new CountDownLatch(1); var release = new CountDownLatch(1);
        HttpServer server = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
        var executor = Executors.newVirtualThreadPerTaskExecutor(); server.setExecutor(executor);
        server.createContext("/", exchange -> {
            try {
                String body = new String(exchange.getRequestBody().readAllBytes(), StandardCharsets.UTF_8);
                String uri = exchange.getRequestURI().toASCIIString();
                seen.add(exchange.getRequestMethod() + " " + uri + " " + exchange.getRequestHeaders().getFirst("Authorization") + " " + body);
                if (uri.endsWith("cancelled")) { entered.countDown(); release.await(3, TimeUnit.SECONDS); }
                if (uri.endsWith("timeout")) Thread.sleep(400);
                boolean deny = body.equals("{\"name\":\"deny\"}");
                byte[] bytes = (deny ? "{\"message\":\"rejected\"}" : uri.contains("?") ? PAGE : WIDGET).getBytes(StandardCharsets.UTF_8);
                exchange.getResponseHeaders().add("Content-Type", uri.endsWith("badmedia") ? "text/plain" : "Application/JSON; charset=\"utf-8\"");
                if (uri.endsWith("redirect")) exchange.getResponseHeaders().add("Location", "/should-not-follow");
                exchange.sendResponseHeaders(deny ? 422 : uri.endsWith("redirect") ? 307 : 200, 0);
                exchange.getResponseBody().write(bytes);
            } catch (InterruptedException error) { Thread.currentThread().interrupt(); }
            finally { exchange.close(); }
        });
        server.start();
        URI base = URI.create("http://127.0.0.1:" + server.getAddress().getPort() + "/api/v1");
        try (var client = new OpenRouter(HttpRuntime.Options.builder().credential("apiKey", "test-key").serverUrl(base).build())) {
            var created = client.createWidget(CreateWidgetInput.builder(WidgetInput.builder("alpha").build()).build());
            assert created.status() == 200 && created.data().amount().token().equals("9007199254740993.000000000000000001");
            var listed = client.listWidgets(ListWidgetsInput.builder().tag("a").tags(List.of("x", "y")).labels(List.of("a,b", "c")).limit(JsonNumber.of(2)).build());
            assert listed.data().items().getFirst().amount().token().equals("1e-400");
            assert !listed.data().items().getFirst().meta().isPresent();
            client.getWidget(GetWidgetInput.builder("a/b 雪!'()*").build());
            client.updateWidget(UpdateWidgetInput.builder("w1", WidgetPatch.builder().build()).build());
            assert seen.equals(List.of(
                "POST /api/v1/widgets Bearer test-key {\"name\":\"alpha\"}",
                "GET /api/v1/widgets?tag=a&tags=x&tags=y&labels=a%2Cb,c&limit=2 Bearer test-key ",
                "GET /api/v1/widgets/a%2Fb%20%E9%9B%AA%21%27%28%29%2A Bearer test-key ",
                "PATCH /api/v1/widgets/w1 Bearer test-key {}")) : seen;
            bad(() -> client.listWidgets(ListWidgetsInput.builder().limit(JsonNumber.of(0)).build()));
            assert seen.size() == 4;
            try { client.createWidget(CreateWidgetInput.builder(WidgetInput.builder("deny").build()).build()); throw new AssertionError("failure became success"); }
            catch (CreateWidgetStatus422 error) { assert error.data().message().equals("rejected") && error.status() == 422 && !error.toString().contains("rejected"); }
            assert client.getWidgetAsync(GetWidgetInput.builder("w1").build()).get().data().id().equals("w1");
            var pending = client.getWidgetAsync(GetWidgetInput.builder("cancelled").build());
            assert entered.await(3, TimeUnit.SECONDS); assert pending.cancel(true); assert pending.isCancelled(); release.countDown();
            for (String path : List.of("badmedia", "redirect")) {
                try { client.getWidget(GetWidgetInput.builder(path).build()); throw new AssertionError("unexpected response accepted"); }
                catch (SdkException error) { assert error.kind().startsWith("unexpected-"); }
            }
            assert seen.stream().noneMatch(s -> s.contains("should-not-follow"));
            try (var bounded = new OpenRouter(HttpRuntime.Options.builder().credential("apiKey", "test-key").serverUrl(base).maxResponseBytes(8).maxCaptureBytes(4).build())) {
                try { bounded.getWidget(GetWidgetInput.builder("limited").build()); throw new AssertionError("response ceiling ignored"); }
                catch (SdkException error) { assert error.kind().equals("resource-limit") && error.status() == 200 && error.capture().length <= 4 && error.truncated(); }
            }
            try (var timed = new OpenRouter(HttpRuntime.Options.builder().credential("apiKey", "test-key").serverUrl(base).timeout(Duration.ofMillis(100)).build())) {
                try { timed.getWidget(GetWidgetInput.builder("timeout").build()); throw new AssertionError("deadline ignored"); }
                catch (SdkException error) { assert error.kind().equals("timeout") : error.kind(); }
            }
        } finally { release.countDown(); server.stop(0); executor.close(); }
    }
}
"#;
