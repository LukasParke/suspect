//! Native installed-module Kotlin/JVM gates. Fixtures and logs are retained;
//! generated source is never patched to make a consumer compile.
#![cfg(feature = "kotlin-sdk")]

use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::Arc,
};
use suspect_codegen::kotlin_sdk::{self, Plan, SdkConfig, models::Shape, plan_sdk};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn root(name: &str) -> PathBuf {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-kotlin-native");
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix(name)
        .tempdir_in(base)
        .unwrap()
        .keep()
        .canonicalize()
        .unwrap()
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

fn fixture(document: Value) -> Arc<Contract> {
    let path = root("admission-").join("api.json");
    std::fs::write(&path, document.to_string()).unwrap();
    load(&path)
}

fn selected(contract: &Contract) -> Vec<SourceId> {
    contract.operations().map(|o| o.source().clone()).collect()
}

fn envelope(schemas: Value) -> Value {
    let media = json!({"application/json":{"schema":{"$ref":"#/components/schemas/Value"}}});
    json!({
        "openapi":"3.1.0","info":{"title":"Kotlin contract","version":"1"},
        "servers":[{"url":"https://example.test/api/v1"}],"security":[{"apiKey":[]}],
        "components":{"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}},"schemas":schemas},
        "paths":{"/value":{"post":{
            "operationId":"echoValue",
            "requestBody":{"required":true,"content":media},
            "responses":{"200":{"description":"OK","content":media}}
        }}}
    })
}

fn config(artifact: &str) -> SdkConfig {
    SdkConfig {
        group_id: "test.suspect.kotlin".into(),
        artifact_id: artifact.into(),
        version: "0.1.0".into(),
        package_name: "example.sdk".into(),
        credential_env: None,
        sdk_defaults: None,
        attribution: None,
    }
}

fn canonical() -> Plan {
    let contract = load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m2/canonical.openapi.yaml"),
    );
    plan_sdk(contract.clone(), &selected(&contract), config("m2-sdk")).unwrap()
}

#[test]
fn canonical_plan_retains_native_descriptors_and_checked_program() {
    let plan = canonical();
    #[cfg(feature = "http-protocol")]
    {
        assert_eq!(
            plan.protocol().capabilities().adapter(),
            "kotlin-jvm-protocol-v1"
        );
        assert!(
            plan.protocol()
                .capabilities()
                .supports(suspect_codegen::http_protocol::Capability::ServerSentEvents)
        );
        assert!(plan.protocol().capabilities().profiles().is_empty());
        assert!(plan.protocol().is_admitted());
    }
    assert_eq!(plan.operations().len(), 4);
    assert!(plan.program().check().is_ok());
    let widget = plan
        .models()
        .symbols()
        .iter()
        .find(|s| s.name == "Widget")
        .unwrap();
    let Shape::Object { fields, .. } = &widget.shape else {
        panic!("native data model required")
    };
    assert!(
        fields
            .iter()
            .any(|f| f.wire_name == "meta" && f.kotlin_type == "Presence<String?>")
    );
    let standard = plan
        .models()
        .symbols()
        .iter()
        .find(|s| s.name == "StandardPayload")
        .unwrap();
    let Shape::Object { fields, .. } = &standard.shape else {
        panic!()
    };
    assert_eq!(
        fields
            .iter()
            .find(|f| f.wire_name == "kind")
            .unwrap()
            .constant
            .as_deref(),
        Some("StandardPayloadKind.STANDARD")
    );
    for op in plan.operations() {
        assert_eq!(op.constructor, op.input_type);
        assert!(!op.responses.is_empty());
        assert!(
            op.responses.iter().all(|r| !r.constructor.is_empty()
                && r.codec_name.as_ref().is_some_and(|n| !n.is_empty()))
        );
        assert!(
            op.parameters
                .iter()
                .all(|p| !p.kotlin_type.is_empty() && !p.codec_name.is_empty())
        );
    }
    let files = plan.render().unwrap();
    assert_eq!(files, plan.render().unwrap());
    assert_eq!(
        files.len(),
        files.iter().map(|f| &f.path).collect::<BTreeSet<_>>().len()
    );
    assert!(files.iter().all(|f| f.path.starts_with("kotlin/")));
    assert!(
        files
            .iter()
            .any(|f| f.path.ends_with("Client.kt") && f.content.contains("public suspend fun"))
    );
    assert!(
        files
            .iter()
            .any(|f| f.path.ends_with("pom.xml") && f.content.contains(kotlin_sdk::KOTLIN_VERSION))
    );
}

#[test]
#[cfg(feature = "http-protocol")]
fn shared_protocol_source_profiles_are_admitted_explicitly() {
    let base = envelope(json!({"Value":{"type":"string"}}));
    let mut cases = Vec::new();
    let mut no_auth = base.clone();
    no_auth["security"] = json!([]);
    cases.push(no_auth);
    let mut range = base.clone();
    range["paths"]["/value"]["post"]["responses"] =
        json!({"2XX":base["paths"]["/value"]["post"]["responses"]["200"]});
    cases.push(range);
    let mut fallback = base.clone();
    fallback["paths"]["/value"]["post"]["responses"] =
        json!({"default":base["paths"]["/value"]["post"]["responses"]["200"]});
    cases.push(fallback);
    let mut text = base.clone();
    text["paths"]["/value"]["post"]["responses"]["200"]["content"] =
        json!({"text/plain":{"schema":{"type":"string"}}});
    cases.push(text);
    let mut header = base.clone();
    header["paths"]["/value"]["post"]["parameters"] =
        json!([{"name":"X-Value","in":"header","schema":{"type":"string"}}]);
    cases.push(header);
    let mut empty = base.clone();
    empty["paths"]["/value"]["post"]["responses"] = json!({"204":{"description":"No body"}});
    cases.push(empty);
    for document in cases {
        let contract = fixture(document);
        let plan = plan_sdk(contract.clone(), &selected(&contract), config("fixture")).unwrap();
        assert!(plan.protocol().is_admitted());
        assert!(!plan.render().unwrap().is_empty());
    }
}

#[test]
fn fresh_generation_preserves_expected_m2_native_bindings() {
    let first = canonical();
    let second = canonical();
    assert_eq!(first.render().unwrap(), second.render().unwrap());
    let operation = first
        .operations()
        .iter()
        .find(|o| o.operation_id == "createWidget")
        .unwrap();
    assert_eq!(operation.method_name, "createWidget");
    assert_eq!(operation.input_type, "CreateWidgetInput");
    assert!(!operation.input_has_default());
    assert_eq!(operation.body.as_ref().unwrap().kotlin_type, "WidgetInput");
    assert_eq!(operation.result_data_type.as_deref(), Some("Widget"));
    let statuses = operation
        .responses
        .iter()
        .map(|r| r.constructor.as_str())
        .collect::<Vec<_>>();
    assert!(statuses.contains(&"CreateWidgetResult.Status200"));
    assert!(statuses.contains(&"CreateWidgetApiException.Status401"));
    assert!(statuses.contains(&"CreateWidgetApiException.Status422"));
    assert!(
        first
            .operations()
            .iter()
            .find(|o| o.operation_id == "listWidgets")
            .unwrap()
            .input_has_default()
    );
}

#[test]
fn unsupported_native_shapes_and_packaging_fail_before_artifacts() {
    let contract = fixture(envelope(json!({"Value":{"type":"string"}})));
    for package in [
        "java.sdk",
        "kotlin.sdk",
        "example.class",
        "a..b",
        "../escape",
    ] {
        let mut c = config("fixture");
        c.package_name = package.into();
        assert!(
            plan_sdk(contract.clone(), &selected(&contract), c)
                .unwrap_err()
                .iter()
                .any(|e| e.code == "kotlin-package-identity")
        );
    }
    for schema in [
        json!({"type":"object","properties":{"secret":{"type":"string","writeOnly":true}}}),
        json!({"type":"array","prefixItems":[{"type":"string"}]}),
        json!({"type":"object","patternProperties":{"(?<=x)y":{"type":"string"}}}),
    ] {
        let contract = fixture(envelope(json!({"Value":schema})));
        let errors =
            plan_sdk(contract.clone(), &selected(&contract), config("fixture")).unwrap_err();
        assert!(
            errors.iter().any(
                |e| e.source.pointer().starts_with("/components/schemas/Value")
                    && e.at.end > e.at.start
            ),
            "{errors:#?}"
        );
    }
    let contract = fixture(envelope(
        json!({"Value":{"type":"string","pattern":"^a[0-9]+$"}}),
    ));
    assert!(plan_sdk(contract.clone(), &selected(&contract), config("fixture")).is_ok());
}

#[test]
fn mutated_programs_are_checked_before_validation_emission() {
    let mut program = canonical().program().clone();
    program.nodes[0].checks[0].instruction =
        suspect_schema::ProgramInstruction::Ref { target: usize::MAX };
    assert!(kotlin_sdk::validation::emit_validation(&program, "example.sdk").is_err());
}

#[test]
fn malformed_paths_are_source_linked_before_native_emission() {
    for path in [
        "/x/{",
        "/x/}",
        "/x/%",
        "/x/%2e%2E",
        "/x/..",
        "/x?query",
        "/雪",
    ] {
        let mut document = envelope(json!({"Value":{"type":"string"}}));
        let operation = document["paths"]["/value"].clone();
        document["paths"] = json!({path:operation});
        let contract = fixture(document);
        let errors =
            plan_sdk(contract.clone(), &selected(&contract), config("fixture")).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.source.pointer().starts_with("/paths/") && e.at.end > e.at.start),
            "{path}: {errors:#?}"
        );
    }
    for server in [
        "https://user:password@example.test/api",
        "https://example.test/api/%",
    ] {
        let mut document = envelope(json!({"Value":{"type":"string"}}));
        document["servers"] = json!([{"url":server}]);
        let contract = fixture(document);
        assert!(
            plan_sdk(contract.clone(), &selected(&contract), config("fixture"))
                .unwrap_err()
                .iter()
                .any(|e| e.code.starts_with("http-server"))
        );
    }
}

fn java_homes() -> Vec<PathBuf> {
    if let Some(home) = std::env::var_os("SUSPECT_KOTLIN_JAVA_HOME") {
        return vec![home.into()];
    }
    ["temurin-21.0.12+101.0.LTS", "temurin-25.0.4+101.0.LTS"]
        .iter()
        .map(|name| Path::new("/Users/luke/.local/share/mise/installs/java").join(name))
        .collect()
}

fn maven(home: &Path) -> Command {
    let binary = std::env::var_os("SUSPECT_KOTLIN_MAVEN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            "/Users/luke/.local/share/mise/installs/maven/3.9.16/apache-maven-3.9.16/bin/mvn".into()
        });
    let mut command = Command::new(binary);
    command
        .args(["-B", "--no-transfer-progress"])
        .arg(format!("-Dmaven.repo.local={}", maven_repo().display()))
        .env("JAVA_HOME", home)
        .env("MAVEN_OPTS", "-Xmx2048m -Dfile.encoding=UTF-8");
    command
}

fn maven_repo() -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-kotlin-maven");
    std::fs::create_dir_all(&path).unwrap();
    path.canonicalize().unwrap()
}

fn checked(command: &mut Command, root: &Path, label: &str) -> Output {
    let output = command
        .output()
        .unwrap_or_else(|e| panic!("{command:?}: {e}"));
    let text = format!(
        "$ {command:?}\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::write(root.join(format!("{label}.log")), &text).unwrap();
    assert!(
        output.status.success(),
        "retained at {}\n{text}",
        root.display()
    );
    output
}

fn write_files(root: &Path, files: &[suspect_codegen::OutFile]) {
    for file in files {
        let path = root.join(&file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, &file.content).unwrap();
    }
}

fn consumer_pom(artifact: &str, main: &str) -> String {
    format!(
        r#"<project xmlns="http://maven.apache.org/POM/4.0.0"><modelVersion>4.0.0</modelVersion>
<groupId>test.suspect.consumer</groupId><artifactId>consumer</artifactId><version>1.0.0</version>
<properties><project.build.sourceEncoding>UTF-8</project.build.sourceEncoding><kotlin.compiler.daemon>false</kotlin.compiler.daemon><exec.mainClass>consumer.{main}Kt</exec.mainClass></properties>
<dependencies><dependency><groupId>test.suspect.kotlin</groupId><artifactId>{artifact}</artifactId><version>0.1.0</version></dependency></dependencies>
<build><sourceDirectory>src/main/kotlin</sourceDirectory><plugins>
<plugin><groupId>org.jetbrains.kotlin</groupId><artifactId>kotlin-maven-plugin</artifactId><version>{}</version>
<configuration><jvmTarget>21</jvmTarget><args><arg>-Werror</arg></args></configuration>
<executions><execution><id>compile</id><phase>compile</phase><goals><goal>compile</goal></goals></execution></executions></plugin>
<plugin><groupId>org.codehaus.mojo</groupId><artifactId>exec-maven-plugin</artifactId><version>3.6.3</version><configuration><executable>${{java.home}}/bin/java</executable><classpathScope>runtime</classpathScope><arguments><argument>-Xmx512m</argument><argument>-cp</argument><classpath/><argument>${{exec.mainClass}}</argument></arguments></configuration></plugin>
</plugins></build></project>"#,
        kotlin_sdk::KOTLIN_VERSION
    )
}

fn native_package(plan: &Plan, name: &str, source: &str) -> PathBuf {
    let root = root(&format!("{name}-"));
    write_files(&root, &plan.render().unwrap());
    let consumer = root.join("consumer");
    std::fs::create_dir_all(consumer.join("src/main/kotlin")).unwrap();
    std::fs::write(
        consumer.join("pom.xml"),
        consumer_pom(&plan.config().artifact_id, name),
    )
    .unwrap();
    std::fs::write(consumer.join(format!("src/main/kotlin/{name}.kt")), source).unwrap();
    if name == "NativeOpenRouter" {
        let docs = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/SDK-KOTLIN.md"),
        )
        .unwrap();
        let snippets = docs
            .split("```kotlin\n")
            .skip(1)
            .map(|part| part.split("\n```").next().unwrap())
            .filter(|part| part.contains("import example.sdk.*"))
            .flat_map(str::lines)
            .filter(|line| !line.starts_with("import "))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(
            consumer.join("src/main/kotlin/SDKGuide.kt"),
            format!("package consumer\nimport example.sdk.*\n{snippets}\n"),
        )
        .unwrap();
    }
    let readme = std::fs::read_to_string(root.join("kotlin/README.md")).unwrap();
    let guide = readme
        .split("```kotlin\n")
        .skip(1)
        .map(|part| part.split("\n```").next().unwrap())
        .find(|part| part.contains("public object Quickstart"))
        .expect("a native-constructor quickstart is required for the fixture");
    assert!(
        !guide.contains("Codecs."),
        "quickstart must use native constructors"
    );
    std::fs::write(
        consumer.join("src/main/kotlin/Quickstart.kt"),
        format!("package consumer\n{guide}\n"),
    )
    .unwrap();
    let quickstart: Value =
        serde_json::from_slice(&std::fs::read(root.join("kotlin/docs/quickstart.json")).unwrap())
            .unwrap();
    let response = serde_json::to_vec(&quickstart["response"]["value"]).unwrap();
    let bytes = response
        .iter()
        .map(|b| (*b as i8).to_string())
        .collect::<Vec<_>>()
        .join(",");
    let credentials = plan
        .credentials()
        .values()
        .map(|credential| format!("{} = \"guide-token\"", credential.name))
        .collect::<Vec<_>>()
        .join(", ");
    let status = quickstart["response"]["status"].as_u64().unwrap();
    std::fs::write(consumer.join("src/main/kotlin/GuideCheck.kt"), format!("package consumer\nimport example.sdk.*\nimport kotlinx.coroutines.runBlocking\nobject GuideCheck {{ @JvmStatic fun main(args: Array<String>) = runBlocking {{\n var calls = 0\n Client(Credentials({credentials}), Transport {{ request -> calls++; check(request.method == {}); HttpResponse({status}, mapOf(\"Content-Type\" to listOf(\"application/json\")), byteArrayOf({bytes})) }}).use {{ client -> check(Quickstart.firstRequest(client).response.status == {status}) }}\n check(calls == 1); println(\"KOTLIN_INSTALLED_README_QUICKSTART_PASSED\")\n}} }}\n", serde_json::to_string(quickstart["method"].as_str().unwrap()).unwrap())).unwrap();
    for (i, home) in java_homes().iter().enumerate() {
        checked(
            maven(home).arg("install").current_dir(root.join("kotlin")),
            &root,
            &format!("sdk-install-{i}"),
        );
        let jar_name = format!("{}-{}", plan.config().artifact_id, plan.config().version);
        for suffix in [".jar", "-sources.jar", "-javadoc.jar", "-examples.jar"] {
            assert!(
                root.join(format!("kotlin/target/{jar_name}{suffix}"))
                    .is_file()
            );
        }
        assert!(root.join("kotlin/target/dokka/index.html").is_file());
        native_docs(plan, &root, i);
        checked(
            maven(home)
                .args(["compile", "exec:exec"])
                .current_dir(&consumer),
            &root,
            &format!("consumer-{i}"),
        );
        checked(
            maven(home)
                .args(["exec:exec", "-Dexec.mainClass=consumer.GuideCheck"])
                .current_dir(&consumer),
            &root,
            &format!("quickstart-{i}"),
        );
    }
    println!("Kotlin native installed package passed: {}", root.display());
    root
}

fn native_docs(plan: &Plan, root: &Path, jdk: usize) {
    let docs = root.join("kotlin/target/dokka");
    let pages: Vec<Value> =
        serde_json::from_slice(&std::fs::read(docs.join("scripts/pages.json")).unwrap()).unwrap();
    let mut expected = BTreeSet::new();
    let mut add = |name: String| {
        expected.insert(format!("{}.{name}", plan.config().package_name));
    };
    for name in [
        "Client",
        "Credentials",
        "Json",
        "JsonNumber",
        "JsonLimits",
        "JsonException",
        "JsonValue",
        "JsonObject",
        "JsonArray",
        "JsonString",
        "JsonNull",
        "JsonBoolean",
        "Presence",
        "Presence.Present",
        "Presence.Absent",
        "ModelCodec",
        "CodecLimits",
        "SchemaValidation",
        "SourceLocation",
        "ValidationException",
        "EvaluationException",
        "Transport",
        "JdkTransport",
        "TransportException",
        "SdkException",
        "ApiException",
        "ResponseInfo",
        "ClientOptions",
        "RequestOptions",
    ] {
        add(name.into());
    }
    for op in plan.operations() {
        add(format!("Client.{}", op.method_name));
        for name in [&op.input_type, &op.result_type, &op.error_type] {
            add(name.clone());
        }
        for p in &op.parameters {
            add(format!("{}.{}", op.input_type, p.name));
        }
        if op.body.is_some() {
            add(format!("{}.body", op.input_type));
        }
        for r in &op.responses {
            add(r.constructor.clone());
            add(format!("{}.data", r.constructor));
        }
    }
    let reference = std::fs::read_to_string(root.join("kotlin/docs/reference.md")).unwrap();
    for s in plan.models().symbols() {
        add(format!("Codecs.{}", s.codec_name));
        assert!(reference.contains(&format!("### `{}`", s.name)));
        match &s.shape {
            Shape::Object { fields, additional } => {
                add(s.name.clone());
                for f in fields {
                    add(format!("{}.{}", s.name, f.name));
                }
                if !matches!(additional, kotlin_sdk::models::Additional::Closed) {
                    add(format!("{}.additionalProperties", s.name));
                }
            }
            Shape::StringEnum(cases) => {
                add(s.name.clone());
                for (case, _) in cases {
                    add(format!("{}.{case}", s.name));
                }
            }
            Shape::Union { variants, .. } => {
                add(s.name.clone());
                for (variant, _) in variants {
                    add(format!("{}.{variant}", s.name));
                }
            }
            _ => {}
        }
    }
    let mut locations = Vec::new();
    for symbol in &expected {
        let page = pages
            .iter()
            .find(|p| p["description"].as_str() == Some(symbol.as_str()))
            .unwrap_or_else(|| panic!("missing Dokka declaration {symbol}"));
        let location = page["location"].as_str().unwrap();
        let html = std::fs::read_to_string(docs.join(location.split('#').next().unwrap())).unwrap();
        assert!(
            html.contains("<html")
                && html.contains("id=\"content\"")
                && html.contains("class=\"paragraph\""),
            "missing rendered KDoc for {symbol}"
        );
        locations.push(json!({"symbol":symbol,"page":location}));
    }
    std::fs::write(
        root.join(format!("dokka-coverage-{jdk}.json")),
        serde_json::to_string_pretty(
            &json!({"expected":expected.len(),"missing":[],"pages":locations}),
        )
        .unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "requires Maven, Kotlin/Dokka artifacts and JDK 21/25; real installed native module"]
fn native_m2_installed_module_models_coroutines_wire_docs_and_types() {
    let plan = canonical();
    let root = native_package(
        &plan,
        "NativeM2",
        include_str!("../src/kotlin_sdk/native_m2.kt"),
    );
    native_negative_types(
        &root,
        &[
            "val invalid = WidgetInput()",
            "val invalid = WidgetInput(name = null)",
            "val invalid = WidgetInput(name = \"x\", amount = Presence.Present(null))",
            "val invalid = WidgetPayload.AsStandardPayload(SecurePayload(vault = \"x\"))",
            "val invalid = CreateWidgetInput()",
            "val invalid = StandardPayload(text = \"x\", kind = StandardPayloadKind.STANDARD)",
        ],
    );
}

fn native_negative_types(root: &Path, cases: &[&str]) {
    let consumer = root.join("consumer");
    let path = consumer.join("src/main/kotlin/Negative.kt");
    for (jdk, home) in java_homes().iter().enumerate() {
        // The identical installed module and import path must compile first.
        checked(
            maven(home).arg("compile").current_dir(&consumer),
            root,
            &format!("positive-types-{jdk}"),
        );
        for (index, source) in cases.iter().enumerate() {
            std::fs::write(
                &path,
                format!("package consumer\nimport example.sdk.*\n{source}\n"),
            )
            .unwrap();
            let command = maven(home)
                .arg("compile")
                .current_dir(&consumer)
                .output()
                .unwrap();
            let output = format!(
                "{}{}",
                String::from_utf8_lossy(&command.stdout),
                String::from_utf8_lossy(&command.stderr)
            );
            std::fs::write(root.join(format!("negative-{jdk}-{index}.log")), &output).unwrap();
            assert!(
                !command.status.success(),
                "negative Kotlin consumer compiled: {source}"
            );
            assert!(
                output.contains("Negative.kt")
                    && [
                        "No value passed for parameter",
                        "Argument type mismatch",
                        "Null cannot be a value",
                        "Cannot find a parameter",
                        "No parameter with name"
                    ]
                    .iter()
                    .any(|message| output.contains(message)),
                "uncontrolled negative failure:\n{output}"
            );
            assert!(
                !output.contains("Unresolved reference"),
                "negative check failed on import/symbol lookup:\n{output}"
            );
        }
        std::fs::remove_file(&path).unwrap();
    }
}

#[test]
#[ignore = "requires actual read-only OpenRouter corpus and native Maven/JDK toolchains"]
fn native_actual_five_openrouter_operations() {
    use sha2::Digest;
    let checkout = std::env::var_os("OPENROUTER_WEB_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/Users/luke/github/openrouter-web".into());
    let path = checkout.join("projects/docs/openapi/openapi.yaml");
    let bytes = std::fs::read(&path).expect("actual OpenRouter description is required");
    assert_eq!(
        format!("{:x}", sha2::Sha256::digest(&bytes)),
        "bd4953b29f34de134ed4be27b2803c5622a756c3c63bc8617636b6abe5de1821"
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
        .filter(|o| o.operation_id().is_some_and(|id| wanted.contains(&id)))
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 5);
    let plan = plan_sdk(contract, &selected, config("openrouter-sdk")).unwrap();
    let operation = |name: &str| {
        plan.operations()
            .iter()
            .find(|o| o.operation_id == name)
            .unwrap()
    };
    let create = operation("createKeys").body.as_ref().unwrap();
    let update = operation("updateKeys").body.as_ref().unwrap();
    let reset = plan
        .models()
        .symbol(
            &create.media[0]
                .schema
                .as_ref()
                .unwrap()
                .child("properties")
                .child("limit_reset"),
        )
        .unwrap();
    assert!(matches!(&reset.shape, Shape::StringEnum(_)) && reset.nullable);
    let fixtures: Value =
        serde_json::from_str(include_str!("fixtures/openrouter-five-responses.json")).unwrap();
    let mut source = include_str!("../src/kotlin_sdk/native_openrouter.kt")
        .replace("__CREATE_BODY__", &create.kotlin_type)
        .replace("__UPDATE_BODY__", &update.kotlin_type)
        .replace(
            "__CREATE_CODEC__",
            create.media[0].codec_name.as_ref().unwrap(),
        )
        .replace(
            "__UPDATE_CODEC__",
            update.media[0].codec_name.as_ref().unwrap(),
        )
        .replace("__RESET_ENUM__", &reset.name);
    for key in ["credits", "create", "update", "file", "list"] {
        source = source.replace(
            &format!("__{}_JSON__", key.to_ascii_uppercase()),
            &serde_json::to_string(fixtures[key].as_str().unwrap()).unwrap(),
        );
    }
    let root = native_package(&plan, "NativeOpenRouter", &source);
    native_negative_types(
        &root,
        &[
            &format!("val invalid = {}()", create.kotlin_type),
            &format!(
                "val invalid = {}(name = \"x\", limitReset = Presence.Present(\"yearly\"))",
                create.kotlin_type
            ),
            "val invalid = GetContainerFileInput(containerId = \"c\")",
            "val invalid = ListContainerFilesInput(containerId = \"c\", limit = Presence.Present(2.0))",
        ],
    );
    assert_eq!(
        std::fs::read(path).unwrap(),
        bytes,
        "upstream must stay byte-identical"
    );
}

#[test]
#[ignore = "standalone installed Kotlin validation module: all 17 shared vectors and resource controls"]
fn native_shared_17_validation_vectors() {
    let corpus: Value =
        serde_json::from_str(include_str!("fixtures/runtime-contract-v1.json")).unwrap();
    let cases = corpus["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 17);
    let mut schemas = serde_json::Map::new();
    for (index, case) in cases.iter().enumerate() {
        schemas.insert(format!("Case{index}"), case["schema"].clone());
    }
    schemas.insert("Cycle".into(), json!({"$ref":"#/components/schemas/Cycle"}));
    schemas.insert(
        "UnionFailure".into(),
        json!({"anyOf":[true,{"$ref":"#/components/schemas/Cycle"}]}),
    );
    schemas.insert(
        "NotFailure".into(),
        json!({"not":{"$ref":"#/components/schemas/Cycle"}}),
    );
    schemas.insert(
        "EqualityFailure".into(),
        json!({"anyOf":[true,{"const":"x"}]}),
    );
    schemas.insert(
        "PatternFailure".into(),
        json!({"not":{"type":"string","pattern":"^(a|aa)+$"}}),
    );
    let contract = fixture(
        json!({"openapi":"3.1.0","info":{"title":"Runtime vectors","version":"1"},"components":{"schemas":schemas}}),
    );
    let program = kotlin_sdk::validation::plan_validation(
        contract.clone(),
        contract.schema_roots(),
        suspect_schema::Config {
            max_depth: 128,
            ..Default::default()
        },
    )
    .unwrap();
    let root = root("Runtime-");
    write_files(
        &root,
        &kotlin_sdk::validation::emit_validation(&program, "example.sdk").unwrap(),
    );
    let mut pom = include_str!("../src/kotlin_sdk/pom.xml").to_owned();
    for (key, value) in [
        ("GROUP", "test.suspect.kotlin"),
        ("ARTIFACT", "runtime-sdk"),
        ("VERSION", "0.1.0"),
        ("PACKAGE", "example.sdk"),
        ("KOTLIN", kotlin_sdk::KOTLIN_VERSION),
        ("COROUTINES", kotlin_sdk::COROUTINES_VERSION),
        ("DOKKA", kotlin_sdk::DOKKA_VERSION),
    ] {
        pom = pom.replace(&format!("__{key}__"), value);
    }
    std::fs::write(root.join("kotlin/pom.xml"), pom).unwrap();
    std::fs::create_dir_all(root.join("kotlin/docs")).unwrap();
    std::fs::write(root.join("kotlin/docs/module.md"), "# Module runtime-sdk\n\nStandalone checked validation conformance module.\n\n# Package example.sdk\n\nExact JSON and checked portable evaluation.\n").unwrap();
    let consumer = root.join("consumer");
    std::fs::create_dir_all(consumer.join("src/main/kotlin")).unwrap();
    std::fs::create_dir_all(consumer.join("src/main/resources")).unwrap();
    std::fs::write(
        consumer.join("pom.xml"),
        consumer_pom("runtime-sdk", "NativeRuntime"),
    )
    .unwrap();
    std::fs::write(
        consumer.join("src/main/resources/vectors.json"),
        include_str!("fixtures/runtime-contract-v1.json"),
    )
    .unwrap();
    std::fs::write(
        consumer.join("src/main/kotlin/NativeRuntime.kt"),
        include_str!("../src/kotlin_sdk/native_runtime.kt"),
    )
    .unwrap();
    for (jdk, home) in java_homes().iter().enumerate() {
        checked(
            maven(home)
                .args(["install", "-Dexec.skip=true"])
                .current_dir(root.join("kotlin")),
            &root,
            &format!("runtime-install-{jdk}"),
        );
        checked(
            maven(home)
                .args(["compile", "exec:exec"])
                .current_dir(&consumer),
            &root,
            &format!("vectors-{jdk}"),
        );
    }
    println!("Kotlin 17-vector runtime gate passed: {}", root.display());
}

#[test]
#[ignore = "native model/union/presence/budget and hostile name/prose regression package"]
fn native_adversarial_models_names_and_examples() {
    let hostile_key = "$payload */ \u{000c} 😀";
    let hostile_text = "$message */ \u{000c} \u{2028} <script>alert('x')</script>";
    let response = json!({"required_nullable":null,hostile_key:hostile_text});
    let mut document = envelope(json!({
        "Value":{"description":"Untrusted prose */ /* @param [link](javascript:alert(1)) <script>alert('x')</script> $interpolation", "type":"object", "required":["required_nullable",hostile_key], "additionalProperties":false,
            "example":response,
            "properties":{
                "required_nullable":{"type":["string","null"]},
                hostile_key:{"type":"string","example":hostile_text},
                "token":{"type":"string","pattern":"^a[0-9]+$","example":"a1"},
                "flag":{"type":"boolean","default":true},
                "constructor":{"type":"string"},"get":{"type":"string"},"set":{"type":"string"},
                "by":{"type":"string"},"field":{"type":"string"},"data":{"type":"string"},
                "value":{"type":"string"},"copy":{"type":"string"},"component1":{"type":"string"},"_":{"type":"string"},
                "split":{"$ref":"#/components/schemas/Split"},
                "parent":{"$ref":"#/components/schemas/Parent"},
                "mixed":{"$ref":"#/components/schemas/Mixed"},
                "ambiguous":{"$ref":"#/components/schemas/Ambiguous"},
                "typed":{"$ref":"#/components/schemas/TypedMap"},
                "choices":{"type":"array","items":{"$ref":"#/components/schemas/Split"}},
                "é":{"type":"string"},"é":{"type":"string"},"雪/~/😀":{"type":"string"},
                "runtime":{"$ref":"#/components/schemas/System"},"runtime2":{"$ref":"#/components/schemas/Quickstart"},
                "runtime3":{"$ref":"#/components/schemas/Character"},"runtime4":{"$ref":"#/components/schemas/HeaderSnapshot"}
            }},
        "Split":{"oneOf":[{"type":"string","minLength":3},{"type":"string","maxLength":2}]},
        "Parent":{"oneOf":[{"type":"number","maximum":10},{"type":"number","minimum":20}],"minimum":5},
        "Mixed":{"oneOf":[{"type":"string","const":"auto"},{"$ref":"#/components/schemas/Configuration"}]},
        "Configuration":{"type":"object","required":["name"],"properties":{"name":{"type":"string"}}},
        "Ambiguous":{"oneOf":[{"type":"string"},{"type":"string"}]},
        "TypedMap":{"type":"object","additionalProperties":{"type":["number","null"]}},
        "System":{"type":"object","properties":{"class":{"type":"string"}}},
        "Quickstart":{"type":"object","properties":{"value":{"type":"string"}}},
        "Character":{"type":"object","properties":{"value":{"type":"string"}}},
        "HeaderSnapshot":{"type":"object","properties":{"value":{"type":"string"}}}
    }));
    let mut operation = document["paths"]["/value"]["post"].clone();
    operation["operationId"] = json!("close");
    operation["description"] = json!(
        "Hostile */ /* @throws [missing] <script>alert('x')</script> \u{000c} \u{2028} $code."
    );
    operation["parameters"] = json!([
        {"name":"client","in":"path","required":true,"schema":{"type":"string","example":"path"}},
        {"name":"body","in":"query","schema":{"type":"string","example":"query-body"}},
        {"name":"input","in":"query","schema":{"type":"string","example":"query-input"}},
        {"name":"é","in":"query","schema":{"type":"string"}},
        {"name":"é","in":"query","schema":{"type":"string"}}
    ]);
    let hostile_operation = "source */ \u{000c} $call \u{2028} 雪";
    document["paths"] = json!({"/echo/{client}":{"post":operation}, "/empty":{"get":{"operationId":hostile_operation,"responses":operation["responses"]}}});
    let contract = fixture(document);
    let plan = plan_sdk(
        contract.clone(),
        &selected(&contract),
        config("adversarial-sdk"),
    )
    .unwrap();
    let value = plan
        .models()
        .symbols()
        .iter()
        .find(|s| s.name == "Value")
        .unwrap();
    let Shape::Object { fields, .. } = &value.shape else {
        panic!()
    };
    let hostile_member = &fields
        .iter()
        .find(|f| f.wire_name == hostile_key)
        .unwrap()
        .name;
    let op = plan
        .operations()
        .iter()
        .find(|op| op.operation_id == "close")
        .unwrap();
    assert_eq!(op.method_name, "close2");
    assert_eq!(op.parameters[1].name, "body2");
    for name in ["System2", "Quickstart2", "Character2", "HeaderSnapshot2"] {
        assert!(plan.models().symbols().iter().any(|s| s.name == name));
    }
    let other = plan
        .operations()
        .iter()
        .find(|op| op.operation_id == hostile_operation)
        .unwrap();
    let source = include_str!("../src/kotlin_sdk/native_adversarial.kt")
        .replace("__HOSTILE_MEMBER__", hostile_member)
        .replace("__INPUT__", &op.input_type)
        .replace("__HOSTILE_METHOD__", &other.method_name)
        .replace("__COMPOSED__", &op.parameters[3].name)
        .replace("__DECOMPOSED__", &op.parameters[4].name);
    let root = native_package(&plan, "NativeAdversarial", &source);
    native_negative_types(
        &root,
        &[
            &format!("val invalid = Value({hostile_member} = \"x\")"),
            &format!(
                "val invalid = Value(requiredNullable = null, {hostile_member} = \"x\", token = Presence.Present(null))"
            ),
            "val invalid = TypedMap(additionalProperties = mapOf(\"x\" to \"wrong\"))",
            "val invalid = Split.AsVariant1(JsonNumber.of(1))",
        ],
    );
    let files = plan.render().unwrap();
    for file in files
        .iter()
        .filter(|f| f.path.ends_with("Models.kt") || f.path.ends_with("Client.kt"))
    {
        assert!(!file.content.contains("<script>alert('x')</script>"));
    }
}
