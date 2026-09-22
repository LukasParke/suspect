#![cfg(feature = "java-sdk")]
//! Complete declared aggregate recipes remain native part values, never JSON codec roots.
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::java_sdk::{self, MavenConfig, PackageConfig, SdkPlan};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn root() -> PathBuf {
    let base =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-java-aggregate-examples/java");
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix("case-")
        .tempdir_in(base)
        .unwrap()
        .keep()
}
fn plan(root: &Path) -> SdkPlan {
    let payload = json!({"type":"object","required":["id"],"properties":{"id":{"type":"integer"}},"additionalProperties":false});
    let form = json!({"schema":{"type":"object","required":["label"],"properties":{"label":{"type":"string"},"tags":{"type":"array","items":{"type":"string"}},"omitted":{"type":"string"},"payload":payload},"additionalProperties":{"type":"string"}},"examples":{"bad":{"value":{"label":5}},"good":{"value":{"label":"declared","tags":[],"extra-one":"first","extra-two":"second","payload":{"id":7}}}}});
    let named = json!({"schema":{"type":"object","required":["label"],"properties":{"label":{"type":"string"},"tags":{"type":"array","items":{"type":"string"}},"omitted":{"type":"string"},"payload":payload},"additionalProperties":{"type":"array","items":{"type":"string"}}},"example":{"label":"named","tags":["one","two"],"custom":["x","y"],"payload":{"id":9}}});
    let positional = json!({"schema":{"type":"array","minItems":1,"maxItems":5,"prefixItems":[{"type":"string"}],"items":{"type":"integer"}},"prefixEncoding":[{"contentType":"text/plain"}],"itemEncoding":{"contentType":"application/json"},"example":["prefix",1,2,3]});
    let reused = json!({"schema":{"type":"array","minItems":1,"maxItems":4,"items":{"type":"string"}},"prefixEncoding":[{"contentType":"text/plain"},{"contentType":"application/json"}],"itemEncoding":{"contentType":"application/json"},"example":["first","second","tail"]});
    let mut sparse = reused.clone();
    sparse["example"] = json!(["only"]);
    let binary = json!({"schema":{"type":"object","required":["file","label"],"properties":{"file":{},"label":{"type":"string","example":"fallback label"}},"additionalProperties":false},"encoding":{"file":{"contentType":"application/octet-stream"}},"example":{"file":null,"label":"not native bytes"}});
    let mut paths = serde_json::Map::new();
    for (path, name, media, value) in [
        ("/form", "form", "application/x-www-form-urlencoded", form),
        ("/parts", "parts", "multipart/form-data", named),
        ("/sequence", "sequence", "multipart/mixed", positional),
        ("/binary", "binary", "multipart/form-data", binary),
        ("/reused", "reused", "multipart/mixed", reused),
        ("/sparse", "sparse", "multipart/mixed", sparse),
    ] {
        paths.insert(path.into(),json!({"post":{"operationId":name,"requestBody":{"required":true,"content":{media:value}},"responses":{"200":{"description":"OK","content":{"application/json":{"schema":{"type":"boolean"},"example":true}}}}}}));
    }
    let file = root.join("api.json");
    std::fs::write(&file,json!({"openapi":"3.2.0","info":{"title":"Declared aggregate recipes","version":"1"},"servers":[{"url":"https://example.test"}],"paths":paths}).to_string()).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(root).build().unwrap());
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&file).unwrap()).unwrap());
    let operations = contract
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    java_sdk::plan_sdk_with_maven(
        contract,
        &operations,
        PackageConfig {
            package: "example.aggregate".into(),
            version: "1.0.0".into(),
            api_name: "Client".into(),
        },
        &[],
        MavenConfig {
            artifact_id: "java-aggregate-examples".into(),
            ..Default::default()
        },
    )
    .unwrap()
}
#[test]
fn complete_declared_groups_preserve_values_and_native_roots() {
    let root = root();
    let plan = plan(&root);
    assert!(
        plan.native_examples()
            .iter()
            .all(|e| e.input_expression.is_some())
    );
    for name in ["form", "parts", "sequence", "reused", "sparse"] {
        let examples = plan
            .examples()
            .operations()
            .iter()
            .find(|e| e.operation_id == name)
            .unwrap();
        assert_eq!(examples.validated_aggregates.len(), 1);
        let aggregate = &examples.validated_aggregates[0];
        assert!(!plan.protocol().codec_roots().contains(&aggregate.schema));
        assert!(aggregate.declared_source.is_some());
    }
    assert!(
        plan.examples()
            .diagnostics()
            .iter()
            .any(|d| d.source.pointer().ends_with("/examples/bad/value")),
        "invalid declaration disappeared"
    );
    assert!(
        plan.examples()
            .diagnostics()
            .iter()
            .any(|d| d.code == "examples-declared-unavailable"
                && d.source.pointer().contains("~1binary")),
        "byte declaration became JSON stand-in"
    );
    let binary = plan
        .native_examples()
        .iter()
        .find(|e| e.operation_id == "binary")
        .unwrap();
    assert!(binary.synthesized_native_bytes);
    let form = plan
        .native_examples()
        .iter()
        .find(|e| e.operation_id == "form")
        .unwrap()
        .input_expression
        .as_ref()
        .unwrap();
    assert!(form.contains(".tags(java.util.List.of())") && !form.contains(".omitted("));
    let reused = plan
        .examples()
        .operations()
        .iter()
        .find(|o| o.operation_id == "reused")
        .unwrap();
    assert!(reused.entries.iter().any(
        |e| e.part_position == Some(suspect_codegen::examples::ExamplePartPosition::Prefix(1))
    ));
    let sparse = plan
        .native_examples()
        .iter()
        .find(|o| o.operation_id == "sparse")
        .unwrap()
        .input_expression
        .as_ref()
        .unwrap();
    assert!(!sparse.contains(".part2(") && !sparse.contains(".addItem("));
}

#[test]
fn aggregate_factories_refuse_jvm_arity_overflow_at_source() {
    let root = root();
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for index in 0..256 {
        let name = format!("field{index}");
        properties.insert(name.clone(), json!({"type":"string"}));
        required.push(name);
    }
    let file = root.join("arity.json");
    std::fs::write(&file,json!({"openapi":"3.2.0","info":{"title":"JVM factory arity","version":"1"},"servers":[{"url":"https://example.test"}],"paths":{"/parts":{"post":{"requestBody":{"required":true,"content":{"multipart/form-data":{"schema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}}}},"responses":{"200":{"description":"OK"}}}}}}).to_string()).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(&root).build().unwrap());
    let c =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&file).unwrap()).unwrap());
    let operations = c
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    let errors = java_sdk::plan_sdk(c, &operations, PackageConfig::default(), &[]).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.code == "java-constructor-resource-limit"
                && e.source.pointer().contains("/content/multipart~1form-data")
                && e.at.end > e.at.start),
        "{errors:?}"
    );
}
fn checked(command: &mut Command, root: &Path) {
    let result = command.output().unwrap();
    let logs = root.join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    let n = std::fs::read_dir(&logs).unwrap().count();
    std::fs::write(
        logs.join(format!("{n:03}.log")),
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
#[test]
#[ignore = "focused changed aggregate-example seam; installed SDK on JDK21/25"]
fn native_declared_aggregate_examples() {
    let root = root();
    let plan = plan(&root);
    for file in plan.render().unwrap() {
        let path = root.join(file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, file.content).unwrap();
    }
    let java = PathBuf::from(std::env::var_os("JAVA_HOME").expect("select JDK21/25"));
    let maven = std::env::var_os("SUSPECT_MAVEN_BIN").unwrap_or_else(|| {
        "/Users/luke/.local/share/mise/installs/maven/3.9.16/apache-maven-3.9.16/bin/mvn".into()
    });
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/sdk-java-maven-cache/java/repository");
    checked(
        Command::new(maven)
            .args(["-B", "-q", "install"])
            .arg(format!("-Dmaven.repo.local={}", repo.display()))
            .env("JAVA_HOME", &java)
            .current_dir(root.join("java")),
        &root,
    );
    let jar = repo
        .join("example/aggregate/java-aggregate-examples/1.0.0/java-aggregate-examples-1.0.0.jar");
    checked(
        Command::new(java.join("bin/java"))
            .args(["-ea", "-cp"])
            .arg(&jar)
            .arg("example.aggregate.SdkExamples"),
        &root,
    );
    std::fs::write(
        root.join("NativeAggregateExamples.java"),
        include_str!("../src/java_sdk/NativeAggregateExamples.java"),
    )
    .unwrap();
    checked(
        Command::new(java.join("bin/javac"))
            .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
            .arg(&jar)
            .arg(root.join("NativeAggregateExamples.java")),
        &root,
    );
    checked(
        Command::new(java.join("bin/java"))
            .args(["-ea", "-cp"])
            .arg(format!("{}:{}", jar.display(), root.display()))
            .arg("NativeAggregateExamples"),
        &root,
    );
    println!("JAVA_DECLARED_AGGREGATE_EXAMPLES {}", root.display());
}
