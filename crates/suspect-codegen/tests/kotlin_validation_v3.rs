#![cfg(all(feature = "kotlin-sdk", feature = "http-protocol"))]
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use suspect_codegen::kotlin_sdk::{self, validation};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_schema::{Config, OwnedProgram};
use suspect_source::Uri;
#[path = "kotlin_support/mod.rs"]
#[allow(dead_code)]
mod support;

const ENTRY: &str = "https://physical.test/api.json";
fn root() -> PathBuf {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-kotlin-resources");
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix("gate-")
        .tempdir_in(base)
        .unwrap()
        .keep()
        .canonicalize()
        .unwrap()
}
fn id(uri: &str, pointer: &str) -> SchemaId {
    pointer.split('/').skip(1).fold(
        SchemaId::new(Uri::parse(uri).unwrap(), Default::default()),
        |at, part| at.child(&part.replace("~1", "/").replace("~0", "~")),
    )
}
fn load(documents: Vec<(&str, Value)>) -> Arc<Contract> {
    let provider = Arc::new(
        DocumentProvider::new(documents.into_iter().map(|(uri, value)| {
            ProvidedDocument::new(
                Uri::parse(uri).unwrap(),
                Uri::parse(uri).unwrap(),
                serde_json::to_vec(&value).unwrap(),
            )
            .unwrap()
        }))
        .unwrap(),
    );
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::parse(ENTRY).unwrap()).unwrap());
    assert!(workspace.failed_document_uris().is_empty());
    contract
}
fn config() -> Config {
    Config {
        max_depth: 128,
        ..Default::default()
    }
}
fn official() -> Vec<Value> {
    use sha2::{Digest, Sha256};
    let main =
        include_str!("../../suspect-schema/tests/fixtures/resource-conformance/dynamicRef.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(main.as_bytes())),
        "dabad36a92ad5747f6b4f77907addf64795422589bbebdea7af8ff1a2b61879d"
    );
    let groups: Vec<Value> = serde_json::from_str(main).unwrap();
    let mut result = Vec::new();
    for group in groups {
        let uri = "https://physical.test/official-schema.json";
        let contract=load(vec![(ENTRY,json!({"openapi":"3.2.0","info":{"title":"Kotlin official resources","version":"1"},"paths":{},"components":{"schemas":{"Use":{"$ref":uri}}}})),(uri,group["schema"].clone()),
            ("http://localhost:1234/draft2020-12/tree.json",serde_json::from_str(include_str!("../../suspect-schema/tests/fixtures/resource-conformance/tree.json")).unwrap()),
            ("http://localhost:1234/draft2020-12/extendible-dynamic-ref.json",serde_json::from_str(include_str!("../../suspect-schema/tests/fixtures/resource-conformance/extendible-dynamic-ref.json")).unwrap()),
            ("http://localhost:1234/draft2020-12/detached-dynamicref.json",serde_json::from_str(include_str!("../../suspect-schema/tests/fixtures/resource-conformance/detached-dynamicref.json")).unwrap())]);
        let program = validation::plan_validation_v3(contract, &[id(uri, "")], config()).unwrap();
        assert_eq!(program.version, OwnedProgram::V3_VERSION);
        program.check().unwrap();
        for case in group["tests"].as_array().unwrap() {
            result.push(json!({"id":format!("{} / {}",group["description"].as_str().unwrap(),case["description"].as_str().unwrap()),"program":program,"rootTarget":program.roots[0].target,"instanceJson":case["data"].to_string(),"expected":if case["valid"].as_bool().unwrap(){"Valid"}else{"Invalid"}}));
        }
    }
    assert_eq!(result.len(), 44);
    result
}

fn controls() -> Vec<Value> {
    let groups: Vec<Value> =
        serde_json::from_str(include_str!("kotlin_support/resource-controls.json")).unwrap();
    let mut result = Vec::new();
    for group in groups {
        let contract = load(vec![(
            ENTRY,
            json!({"openapi":"3.2.0","info":{"title":"Independent resource controls","version":"1"},"paths":{},"components":{"schemas":group["schemas"]}}),
        )]);
        for (index, case) in group["cases"].as_array().unwrap().iter().enumerate() {
            let root = id(
                ENTRY,
                &format!("/components/schemas/{}", case["root"].as_str().unwrap()),
            );
            let mut selected = vec![root.clone()];
            for extra in case["extraRoots"].as_array().into_iter().flatten() {
                selected.push(id(
                    ENTRY,
                    &format!("/components/schemas/{}", extra.as_str().unwrap()),
                ));
            }
            let mut config = config();
            if let Some(cap) = case["limits"]["maxEvaluationSteps"].as_u64() {
                config.max_evaluation_steps = cap as usize;
            }
            if let Some(cap) = case["limits"]["maxNumberBytes"].as_u64() {
                config.max_number_bytes = cap as usize;
            }
            let program =
                validation::plan_validation_v3(contract.clone(), &selected, config).unwrap();
            let target = program
                .roots
                .iter()
                .find(|r| r.source.pointer == root.pointer())
                .unwrap()
                .target;
            result.push(json!({"id":format!("{}-{index}",group["name"].as_str().unwrap()),"program":program,"rootTarget":target,"instanceJson":case["data"].to_string(),"expected":case["expected"],"source":case["source"].as_str().map(|s|format!("/components/schemas/{s}")),"instancePath":case["path"]}));
        }
    }
    result
}

fn malformed_programs() -> Vec<Value> {
    let original = official()
        .into_iter()
        .find(|case| {
            case["program"]["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|node| {
                    node["checks"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|c| c["op"] == "dynamicRef" && c["anchor"].is_string())
                })
        })
        .unwrap()["program"]
        .clone();
    (0..9)
        .map(|kind| {
            let mut value = original.clone();
            match kind {
                0 => value["resourceContext"]["nodeScopes"]
                    .as_array_mut()
                    .unwrap()
                    .pop()
                    .map(|_| ())
                    .unwrap(),
                1 => value["resourceContext"]["nodeScopes"][0][0] = json!(999999),
                2 => value["resourceContext"]["nodeScopes"][0][2] = json!("urn:wrong"),
                3 => value["resourceContext"]["resources"][0]["aliases"] = json!([]),
                4 => {
                    value["version"] = json!(OwnedProgram::V2_VERSION);
                    value["profile"] = json!(OwnedProgram::V2_PROFILE);
                }
                5 => value["resourceContext"] = Value::Null,
                6 => {
                    let check = value["nodes"]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .flat_map(|n| n["checks"].as_array_mut().unwrap())
                        .find(|c| c["op"] == "dynamicRef")
                        .unwrap();
                    check["initialResource"] = json!(999999);
                }
                7 => {
                    let check = value["nodes"]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .flat_map(|n| n["checks"].as_array_mut().unwrap())
                        .find(|c| c["op"] == "dynamicRef")
                        .unwrap();
                    check["anchor"] = json!("no-such-anchor");
                }
                _ => {
                    let r = value["resourceContext"]["resources"]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .find(|r| !r["dynamicAnchors"].as_array().unwrap().is_empty())
                        .unwrap();
                    r["dynamicAnchors"][0][1]["pointer"] = json!("/not-an-anchor");
                }
            }
            value
        })
        .collect()
}

fn deep_program() -> OwnedProgram {
    let mut schemas = serde_json::Map::new();
    for index in 0..160 {
        schemas.insert(
            format!("N{index}"),
            if index == 159 {
                json!({"$id":format!("urn:n{index}"),"type":"string"})
            } else {
                json!({"$id":format!("urn:n{index}"),"$ref":format!("urn:n{}",index+1)})
            },
        );
    }
    let contract = load(vec![(
        ENTRY,
        json!({"openapi":"3.2.0","info":{"title":"Resource depth","version":"1"},"paths":{},"components":{"schemas":schemas}}),
    )]);
    validation::plan_validation_v3(contract, &[id(ENTRY, "/components/schemas/N0")], config())
        .unwrap()
}

#[test]
fn original_44_resource_cases_compile_from_closed_source_documents() {
    let cases = official();
    assert_eq!(cases.len(), 44);
}

#[test]
fn v2_runtime_template_matches_the_recorded_baseline() {
    use sha2::{Digest, Sha256};
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(include_bytes!("../src/kotlin_sdk/ValidationV2.kt"))
        ),
        // Includes the separately verified large-program metadata loader.
        "c433750e3214fdbe2f795df99fc73127846e959d3b70fa7740fb9eb22753ce98"
    );
}

fn sdk_plan(root: &Path) -> kotlin_sdk::Plan {
    let path = root.join("sdk.json");
    std::fs::write(&path, include_str!("kotlin_support/resource-sdk.json")).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(root).build().unwrap());
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap());
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    kotlin_sdk::plan_sdk_v3(
        contract,
        &selected,
        kotlin_sdk::SdkConfig {
            group_id: "test.suspect.kotlin".into(),
            artifact_id: "resource-sdk".into(),
            version: "0.4.0".into(),
            package_name: "example.resources.sdk".into(),
            credential_env: None,
        },
    )
    .unwrap()
}

#[test]
fn resource_sdk_dynamic_inputs_are_checked_carriers() {
    let root = root();
    let plan = sdk_plan(&root);
    assert_eq!(plan.program().version, OwnedProgram::V3_VERSION);
    for source in [
        "/components/schemas/Tree/properties/children/items",
        "/components/schemas/Numbers/items",
    ] {
        let symbol = plan
            .models()
            .symbols()
            .iter()
            .find(|s| s.source.pointer() == source)
            .unwrap();
        assert!(matches!(
            symbol.shape,
            kotlin_sdk::models::Shape::CheckedJson
        ));
    }
    assert_eq!(plan.examples().operations().len(), 6);
    // The referenced numeric resource's example is invalid in the string
    // override context. Preserve that declared-source finding rather than
    // flattening the fallback's example into the overriding operation.
    assert!(!plan.examples().diagnostics().is_empty());
    assert!(
        plan.examples()
            .diagnostics()
            .iter()
            .all(|finding| finding.code == "examples-declared-invalid"
                && finding.source.pointer() == "/components/schemas/Numbers/example"
                && finding.at.end > finding.at.start)
    );
    let strings = plan
        .examples()
        .operations()
        .iter()
        .find(|op| op.operation_id == "strings")
        .unwrap();
    assert!(
        strings
            .entries
            .iter()
            .any(|entry| entry.value == json!(["alpha", "beta"]))
    );
    assert!(!plan.render().unwrap().is_empty());
}

#[test]
fn resource_metadata_cannot_enter_frozen_profiles_or_invalid_native_programs() {
    let root = root();
    let plan = sdk_plan(&root);
    let original = plan.program();
    for change in 0..7 {
        let mut program = original.clone();
        match change {
            0 => {
                program.version = OwnedProgram::V1_VERSION;
                program.profile = OwnedProgram::V1_PROFILE;
            }
            1 => {
                program.version = OwnedProgram::V2_VERSION;
                program.profile = OwnedProgram::V2_PROFILE;
            }
            2 => program.resource_context = None,
            3 => {
                program.resource_context.as_mut().unwrap().node_scopes.pop();
            }
            4 => program.resource_context.as_mut().unwrap().node_scopes[0].0 = usize::MAX,
            5 => program.resource_context.as_mut().unwrap().node_scopes[0].2 = "urn:wrong".into(),
            _ => program.resource_context.as_mut().unwrap().resources[0]
                .aliases
                .clear(),
        }
        assert!(
            validation::emit_validation(&program, "example.resources").is_err(),
            "malformed resource program {change}"
        );
    }
}

#[test]
#[ignore = "installed native resource/dynamic SDK operations, types, examples and Dokka on JDK21/25"]
fn native_resource_sdk_operations() {
    let root = root();
    let plan = sdk_plan(&root);
    for file in plan.render().unwrap() {
        let p = root.join(file.path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, file.content).unwrap();
    }
    let consumer = root.join("consumer");
    std::fs::create_dir_all(consumer.join("src/main/kotlin")).unwrap();
    std::fs::write(
        consumer.join("src/main/kotlin/NativeResources.kt"),
        include_str!("../src/kotlin_sdk/native_resource_sdk.kt"),
    )
    .unwrap();
    let readme = std::fs::read_to_string(root.join("kotlin/README.md")).unwrap();
    let snippet = readme
        .split("```kotlin\n")
        .skip(1)
        .map(|p| p.split("\n```").next().unwrap())
        .find(|p| p.contains("public object Quickstart"))
        .unwrap();
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
        .filter(|p| p.contains("import example.resources.sdk.*"))
        .collect::<Vec<_>>();
    assert!(!blocks.is_empty());
    let body = blocks
        .iter()
        .flat_map(|b| b.lines())
        .filter(|l| !l.starts_with("import "))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(
        consumer.join("src/main/kotlin/ResourceGuide.kt"),
        format!("package consumer\nimport example.resources.sdk.*\n{body}\n"),
    )
    .unwrap();
    std::fs::write(consumer.join("pom.xml"),format!(r#"<project xmlns="http://maven.apache.org/POM/4.0.0"><modelVersion>4.0.0</modelVersion><groupId>test.suspect</groupId><artifactId>resource-consumer</artifactId><version>0.4.0</version><properties><kotlin.compiler.daemon>false</kotlin.compiler.daemon><project.build.sourceEncoding>UTF-8</project.build.sourceEncoding></properties><dependencies><dependency><groupId>test.suspect.kotlin</groupId><artifactId>resource-sdk</artifactId><version>0.4.0</version></dependency></dependencies><build><sourceDirectory>src/main/kotlin</sourceDirectory><plugins><plugin><groupId>org.jetbrains.kotlin</groupId><artifactId>kotlin-maven-plugin</artifactId><version>{}</version><configuration><jvmTarget>21</jvmTarget><args><arg>-Werror</arg></args></configuration><executions><execution><id>compile</id><phase>compile</phase><goals><goal>compile</goal></goals></execution></executions></plugin><plugin><groupId>org.codehaus.mojo</groupId><artifactId>exec-maven-plugin</artifactId><version>3.6.3</version><configuration><executable>${{java.home}}/bin/java</executable><arguments><argument>-Xmx512m</argument><argument>-cp</argument><classpath/><argument>consumer.NativeResourcesKt</argument></arguments></configuration></plugin></plugins></build></project>"#,kotlin_sdk::KOTLIN_VERSION)).unwrap();
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
            "val bad = Tree(children = Presence.Present(listOf(Tree())))",
            "val bad = NumbersInput(body = listOf(JsonNumber.of(1)))",
            "val bad = Poly(value = \"string\")",
            "val bad = Tree(data = null)",
            "val bad = CounterInput(body = 2L)",
            "val bad = TreeChildrenItem(value = mapOf(\"data\" to \"x\"))",
        ]
        .iter()
        .enumerate()
        {
            let path = consumer.join("src/main/kotlin/Negative.kt");
            std::fs::write(
                &path,
                format!("package consumer\nimport example.resources.sdk.*\n{code}\n"),
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
                "uncontrolled type failure {code}\n{log}"
            );
            std::fs::remove_file(path).unwrap();
        }
    }
    println!("Kotlin resource SDK: {}", root.display());
}

#[test]
#[ignore = "source-driven unmodified official V3 cases on JDK21/25"]
fn native_resource_44_vectors() {
    let root = root();
    let cases = official();
    let uri = "https://physical.test/program.json";
    let contract = load(vec![
        (
            ENTRY,
            json!({"openapi":"3.2.0","info":{"title":"V3","version":"1"},"paths":{},"components":{"schemas":{"Use":{"$ref":uri}}}}),
        ),
        (uri, json!({"$id":"urn:resource","type":"string"})),
    ]);
    let program = validation::plan_validation_v3(contract, &[id(uri, "")], config()).unwrap();
    for file in validation::emit_validation(&program, "example.resources").unwrap() {
        let p = root.join(file.path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, file.content).unwrap();
    }
    let package = root.join("kotlin");
    let mut pom = include_str!("../src/kotlin_sdk/pom.xml").to_owned();
    for (key, value) in [
        ("GROUP", "test.suspect.kotlin"),
        ("ARTIFACT", "resource-vectors"),
        ("VERSION", "0.4.0"),
        ("PACKAGE", "example.resources"),
        ("KOTLIN", kotlin_sdk::KOTLIN_VERSION),
        ("COROUTINES", kotlin_sdk::COROUTINES_VERSION),
        ("DOKKA", kotlin_sdk::DOKKA_VERSION),
    ] {
        pom = pom.replace(&format!("__{key}__"), value);
    }
    std::fs::write(package.join("pom.xml"), pom).unwrap();
    std::fs::create_dir_all(package.join("docs")).unwrap();
    std::fs::write(package.join("docs/module.md"),"# Module resource-vectors\n\nIndexed resource validation.\n\n# Package example.resources\n\nChecked native V3 APIs.\n").unwrap();
    std::fs::create_dir_all(package.join("src/test/kotlin/example/resources")).unwrap();
    std::fs::create_dir_all(package.join("src/test/resources/example/resources")).unwrap();
    std::fs::write(
        package.join("src/test/kotlin/example/resources/GeneratedExamples.kt"),
        include_str!("../src/kotlin_sdk/native_validation_v3.kt"),
    )
    .unwrap();
    std::fs::write(
        package.join("src/test/resources/example/resources/vectors.json"),
        json!({"cases":cases,"controls":controls(),"invalidPrograms":malformed_programs(),"deep":deep_program()})
            .to_string(),
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
    }
    println!("Kotlin resource vectors: {}", root.display());
}
