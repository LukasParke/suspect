//! C++20 native package, installed-consumer, exact-runtime and independent wire gates.
#![cfg(feature = "cpp-sdk")]

use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::Arc,
};

use serde_json::{Value, json};
use suspect_codegen::cpp_sdk::{SdkConfig, SdkPlan, emit_validation_runtime, plan_sdk};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::WorkspaceBuilder;
use suspect_schema::{Config, OwnedCompiler};
use suspect_source::Uri;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}
fn root(label: &str) -> PathBuf {
    let base = repo().join("target/sdk-cpp-gates");
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix(&format!("{label}-"))
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
fn fixture(document: Value) -> Arc<Contract> {
    let path = root("source").join("api.json");
    std::fs::write(&path, document.to_string()).unwrap();
    load(&path)
}
fn selected(contract: &Contract) -> Vec<SourceId> {
    contract
        .operations()
        .map(|op| op.source().clone())
        .collect()
}
fn plan(document: Value) -> SdkPlan {
    let contract = fixture(document);
    plan_sdk(contract.clone(), &selected(&contract), SdkConfig::default()).unwrap()
}
fn envelope(schema: Value) -> Value {
    let media = json!({"application/json":{"schema":{"$ref":"#/components/schemas/Value"}}});
    json!({
        "openapi":"3.1.0", "info":{"title":"Native C++","version":"1"},
        "servers":[{"url":"https://example.test/api/v1"}], "security":[{"apiKey":[]}],
        "components":{"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}},"schemas":{"Value":schema}},
        "paths":{"/values":{"post":{
            "operationId":"echoValue",
            "requestBody":{"required":true,"content":media},
            "responses":{"200":{"description":"Value","content":media}}
        }}}
    })
}
fn m2() -> SdkPlan {
    let contract = load(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m2/canonical.openapi.yaml"),
    );
    plan_sdk(
        contract.clone(),
        &selected(&contract),
        SdkConfig {
            max_capture_bytes: 32,
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn m2_descriptors_and_deterministic_package_are_complete() {
    let plan = m2();
    assert_eq!(plan.operations().len(), 4);
    assert!(plan.validation_program().check().is_ok());
    for name in [
        "Widget",
        "WidgetInput",
        "WidgetNode",
        "WidgetPayload",
        "StandardPayload",
        "SecurePayload",
    ] {
        assert!(
            plan.models().symbols().any(|s| s.name == name),
            "missing {name}"
        );
    }
    let standard = plan
        .models()
        .symbols()
        .find(|s| s.name == "StandardPayload")
        .unwrap();
    let args = &standard.constructor.as_ref().unwrap().parameters;
    assert_eq!(args.len(), 1);
    assert_eq!(args[0].member_name, "text");
    assert_eq!(args[0].cpp_type, "std::string");
    let create = plan
        .operations()
        .iter()
        .find(|op| op.operation_id == "createWidget")
        .unwrap();
    assert_eq!(create.method_name, "create_widget");
    assert_eq!(create.input_type, "CreateWidgetInput");
    assert_eq!(create.constructor.parameters[0].member_name, "body");
    assert_eq!(
        create
            .responses
            .iter()
            .map(|r| r.status_key.as_str())
            .collect::<Vec<_>>(),
        ["200", "401", "422"]
    );
    assert_eq!(
        create.responses[0].cases[0].variant_type,
        "CreateWidgetStatus200"
    );
    let files = plan.render().unwrap();
    assert_eq!(files, plan.render().unwrap());
    for path in [
        "cpp/CMakeLists.txt",
        "cpp/cmake/generated_sdkConfig.cmake.in",
        "cpp/include/generated_sdk/sdk.hpp",
        "cpp/src/curl_transport.cpp",
        "cpp/Doxyfile",
        "cpp/examples/client.cpp",
        "cpp/examples/validated_examples.cpp",
    ] {
        assert!(files.iter().any(|f| f.path == path), "missing {path}");
    }
}

fn adversarial_document() -> Value {
    let mut document = envelope(json!({
        "type":"object", "required":["required_nullable","required_value"],
        "description":"Hostile prose stays inert: */\n#error injected\n\\\n<script>alert(1)</script> @include secret [link](javascript:bad)",
        "properties":{
            "required_nullable":{"type":["string","null"]},
            "required_value":{"type":"string","minLength":1},
            "optional_value":{"type":"string"},"optional_nullable":{"type":["string","null"]},
            "selected":{"$ref":"#/components/schemas/Selected"},
            "exclusive":{"$ref":"#/components/schemas/Exclusive"},
            "metadata":{"$ref":"#/components/schemas/Metadata"},
            "recursive":{"$ref":"#/components/schemas/Recursive"},
            "nullable_node":{"$ref":"#/components/schemas/NullableNode"},
            "closed":{"$ref":"#/components/schemas/Closed"},
            "false_list":{"$ref":"#/components/schemas/FalseList"},
            "literal":{"$ref":"#/components/schemas/Literal"},
            "free":true,
            "a/b~\u{0}":{"type":"string"},"a-b":{"type":"string"},
            "\n#error injected\n\\":{"type":"string","enum":["tag\u{0}\n*/\\\n#error injected"]},
            "class":{"type":"string"},"extra":{"type":"string"}
        }
    }));
    let schemas = document["components"]["schemas"].as_object_mut().unwrap();
    for (name, schema) in [
        (
            "Selected",
            json!({"anyOf":[{"type":"number","maximum":5},{"type":"number","minimum":10}],"minimum":2}),
        ),
        (
            "Exclusive",
            json!({"oneOf":[{"type":"integer"},{"type":"number"}]}),
        ),
        (
            "Metadata",
            json!({"type":"object","properties":{"known":{"type":"string"}},"additionalProperties":{"type":"integer"}}),
        ),
        (
            "Recursive",
            json!({"anyOf":[{"type":"string"},{"type":"array","items":{"$ref":"#/components/schemas/Recursive"}}]}),
        ),
        (
            "NullableNode",
            json!({"type":["object","null"],"required":["label"],"properties":{"label":{"type":"string"},"child":{"$ref":"#/components/schemas/NullableNode"}}}),
        ),
        (
            "Closed",
            json!({"type":"object","properties":{"forbidden":false},"additionalProperties":false}),
        ),
        ("FalseList", json!({"type":"array","items":false})),
        ("Literal", json!({"const":{"a":[1,null,true]}})),
        ("Client", json!({"type":"string"})),
        (
            "LongLiteral",
            json!({"type":"string","const":"x".repeat(70000)}),
        ),
    ] {
        schemas.insert(name.into(), schema);
    }
    document["components"]["schemas"]["Value"]["properties"]["client"] =
        json!({"$ref":"#/components/schemas/Client"});
    document["components"]["schemas"]["Value"]["properties"]["large_literal"] =
        json!({"$ref":"#/components/schemas/LongLiteral"});
    document["components"]["schemas"]["Value"]["properties"]
        ["very_long_property_name_".repeat(50)] = json!({"type":"string"});
    document["paths"]["/values"]["post"]["parameters"] = json!([
        {"name":"lead","in":"query","schema":{"type":"string"}},
        {"name":"/".repeat(1024),"in":"query","schema":{"type":"array","items":{"type":"string"}}},
        {"name":"enabled","in":"query","schema":{"type":"boolean"}},
        {"name":"amount","in":"query","schema":{"type":"number"}}
    ]);
    document
}

#[test]
fn native_names_tags_and_constructor_descriptors_preserve_hostile_source_data() {
    let plan = plan(adversarial_document());
    let value = plan
        .models()
        .symbols()
        .find(|s| s.source.pointer() == "/components/schemas/Value")
        .unwrap();
    assert_eq!(
        value
            .constructor
            .as_ref()
            .unwrap()
            .parameters
            .iter()
            .map(|p| p.member_name.as_str())
            .collect::<Vec<_>>(),
        ["required_nullable", "required_value"]
    );
    let suspect_codegen::cpp_sdk::Shape::Object { fields, .. } = &value.shape else {
        panic!("native object")
    };
    assert_eq!(
        fields
            .iter()
            .map(|f| f.name.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        fields.len()
    );
    assert!(fields.iter().any(|f| f.wire == "a/b~\0"));
    assert!(
        fields
            .iter()
            .any(|f| f.wire == "class" && f.name == "class_")
    );
    assert!(fields.iter().all(|field| field.name.len() <= 80));
    assert!(
        plan.models()
            .symbols()
            .any(|s| s.source.pointer() == "/components/schemas/Client" && s.name != "Client")
    );
    assert!(
        !plan
            .render()
            .unwrap()
            .iter()
            .filter(|f| f.path.ends_with(".hpp"))
            .any(|f| f.content.contains("\n#error injected"))
    );
}

#[test]
#[ignore = "C++20 installed core-only package; source-hostile names, recursive unions, typed extras, selected-arm and parent constraints"]
fn native_adversarial_models_and_consumers() {
    let plan = plan(adversarial_document());
    let root = root("adversarial");
    build_sdk(&plan, &root, true, false);
    let mut source = include_str!("../src/cpp_sdk/tests/native_adversarial.cpp").to_owned();
    for name in [
        "Value",
        "Selected",
        "Exclusive",
        "Metadata",
        "Recursive",
        "NullableNode",
        "Closed",
        "FalseList",
        "Literal",
        "LongLiteral",
    ] {
        let symbol = plan
            .models()
            .symbols()
            .find(|s| s.source.pointer() == format!("/components/schemas/{name}"))
            .unwrap();
        source = source
            .replace(&format!("__{name}__"), &symbol.cpp_type)
            .replace(&format!("__{name}Codec__"), &symbol.codec_name);
    }
    let executable = consumer(&root, "generated_sdk", &source);
    checked(&mut Command::new(executable), &root);
    let root_type = plan
        .models()
        .symbols()
        .find(|s| s.source.pointer() == "/components/schemas/Value")
        .unwrap();
    for (i, body) in [
        format!("{} value;", root_type.cpp_type),
        format!(
            "{} value(Null{{}},\"x\"); value.optional_value=Null{{}};",
            root_type.cpp_type
        ),
        "Metadata value; value.extra.emplace(\"x\",std::string(\"not an integer\"));".into(),
        "Closed value; value.extra.emplace(\"unknown\",JsonValue());".into(),
        "auto value=Recursive::alternative_0(JsonNumber(1));".into(),
    ]
    .iter()
    .enumerate()
    {
        let file = root.join(format!("negative-model-{i}.cpp"));
        std::fs::write(&file,format!("#include <generated_sdk/sdk.hpp>\nusing namespace generated_sdk;\nint main(){{{body} (void)value;}}\n")).unwrap();
        let result = Command::new(cxx())
            .args([
                "-std=c++20",
                "-Wall",
                "-Wextra",
                "-Wpedantic",
                "-Werror",
                "-fsyntax-only",
                "-I",
            ])
            .arg(root.join("install/include"))
            .arg(file)
            .output()
            .unwrap();
        std::fs::write(root.join(format!("negative-model-{i}.log")), &result.stderr).unwrap();
        assert!(
            !result.status.success()
                && !String::from_utf8_lossy(&result.stderr).contains("file not found")
        );
    }
    println!(
        "C++ installed core-only adversarial native types, values and documentation passed: {}",
        root.display()
    );
}

#[test]
fn unsupported_models_and_malformed_protocols_fail_at_source() {
    use suspect_codegen::cpp_sdk::{Extras, Shape};
    use suspect_schema::{OwnedProgram, ProgramInstruction};

    // Nested patternProperties is now a witnessed v2 native profile. Both open
    // and closed additional-property policies must retain its exact-value map.
    for closed in [false, true] {
        let mut nested = json!({"type":"object","patternProperties":{"^x":{"type":"string"}}});
        if closed {
            nested["additionalProperties"] = json!(false);
        }
        let plan = plan(envelope(
            json!({"type":"object","properties":{"patterned":nested}}),
        ));
        let pointer = "/components/schemas/Value/properties/patterned";
        let model = plan
            .models()
            .symbols()
            .find(|s| s.source.pointer() == pointer)
            .unwrap();
        assert!(matches!(
            model.shape,
            Shape::Object {
                extras: Extras::Patterned,
                ..
            }
        ));
        assert!(model.has_definition());
        let program = plan.validation_program();
        assert_eq!(program.version, OwnedProgram::V2_VERSION);
        let node = program
            .nodes
            .iter()
            .find(|node| node.source.pointer == pointer)
            .unwrap();
        assert!(node.checks.iter().any(|check| matches!(&check.instruction,
            ProgramInstruction::PatternProperties { patterns } if patterns.len() == 1 && patterns[0].0 == "^x")));
        if closed {
            assert!(node.checks.iter().any(|check| matches!(
                check.instruction,
                ProgramInstruction::AdditionalPropertiesWithPatterns { .. }
            )));
        }
        // Public package admission must reach real model/codec emission too.
        plan.render().unwrap();
    }

    for schema in [
        json!({"type":["string","number"]}),
        json!({"type":"array","prefixItems":[{"type":"string"}]}),
        json!({"type":"object","patternProperties":{"[":{"type":"string"}}}),
        json!({"allOf":[{"type":"object","properties":{"x":{"type":"string"}}},{"type":"object","properties":{"y":{"type":"string"}}}]}),
        json!({"type":"string","readOnly":true}),
    ] {
        let contract = fixture(envelope(
            json!({"type":"object","properties":{"unsupported":schema}}),
        ));
        let errors =
            plan_sdk(contract.clone(), &selected(&contract), Default::default()).unwrap_err();
        assert!(
            errors.iter().any(|e| e
                .source
                .pointer()
                .starts_with("/components/schemas/Value/properties/unsupported")
                && e.at.end > e.at.start),
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
        "/雪",
    ] {
        let mut document = envelope(json!({"type":"string"}));
        let operation = document["paths"]
            .as_object_mut()
            .unwrap()
            .remove("/values")
            .unwrap();
        document["paths"][path] = operation;
        let contract = fixture(document);
        let errors =
            plan_sdk(contract.clone(), &selected(&contract), Default::default()).unwrap_err();
        assert!(
            errors.iter().any(|d| d.at.end > d.at.start),
            "{path}: {errors:#?}"
        );
    }
}

#[test]
fn program_mutations_and_resource_policies_are_checked_before_artifacts() {
    let contract = fixture(envelope(json!({"type":"number","minimum":1})));
    let mut program = OwnedCompiler::new(Config::default())
        .compile(contract.clone(), contract.schema_roots())
        .unwrap()
        .program();
    program.version = "unrecognized";
    assert!(emit_validation_runtime(&contract, &program).is_err());
    program.version = "suspect.validation.experimental.v1";
    program.roots[0].target = usize::MAX;
    assert!(emit_validation_runtime(&contract, &program).is_err());
    for config in [
        SdkConfig {
            name: "class".into(),
            ..Default::default()
        },
        SdkConfig {
            namespace: "a::".into(),
            ..Default::default()
        },
        SdkConfig {
            version: "01.0.0".into(),
            ..Default::default()
        },
        SdkConfig {
            max_json_depth: 257,
            ..Default::default()
        },
        SdkConfig {
            max_request_bytes: 0,
            ..Default::default()
        },
        SdkConfig {
            max_capture_bytes: usize::MAX,
            ..Default::default()
        },
    ] {
        assert!(plan_sdk(contract.clone(), &selected(&contract), config).is_err());
    }
    assert!(plan_sdk(contract, &[], Default::default()).is_err());
}

#[test]
fn impossible_examples_remain_findings_without_noop_executables() {
    let plan = plan(envelope(json!(false)));
    assert!(!plan.examples().diagnostics().is_empty());
    let files = plan.render().unwrap();
    assert!(
        !files
            .iter()
            .any(|file| file.path.starts_with("cpp/examples/"))
    );
    assert!(
        !files
            .iter()
            .find(|file| file.path == "cpp/README.md")
            .unwrap()
            .content
            .contains("examples/client.cpp")
    );
}

#[test]
fn portable_literal_representation_limits_are_source_linked_before_emission() {
    let contract = fixture(envelope(json!({"const":0})));
    let original = OwnedCompiler::new(Config::default())
        .compile(contract.clone(), contract.schema_roots())
        .unwrap()
        .program();
    let mut deep = Value::Null;
    for _ in 0..257 {
        deep = json!([deep]);
    }
    for value in [
        deep,
        serde_json::from_str::<Value>(&format!("1e{}1", "0".repeat(65536))).unwrap(),
    ] {
        let mut program = original.clone();
        let check = program
            .nodes
            .iter_mut()
            .flat_map(|node| &mut node.checks)
            .find(|check| {
                matches!(
                    check.instruction,
                    suspect_schema::ProgramInstruction::Const { .. }
                )
            })
            .unwrap();
        check.instruction = suspect_schema::ProgramInstruction::Const { value };
        let errors = emit_validation_runtime(&contract, &program).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.code == "cpp-validation-literal-limit"
                    && e.source.pointer() == "/components/schemas/Value/const"
                    && e.at.end > e.at.start)
        );
    }
}

fn tool(variable: &str, fallback: &str) -> PathBuf {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| fallback.into())
}
fn cmake() -> PathBuf {
    tool("SUSPECT_CPP_CMAKE", "cmake")
}
fn cxx() -> PathBuf {
    tool("SUSPECT_CPP_CXX", "clang++")
}
fn checked(command: &mut Command, retained: &Path) -> Output {
    let output = command
        .output()
        .unwrap_or_else(|e| panic!("required native tool {command:?}: {e}"));
    let log = retained.join("commands.log");
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .unwrap();
    writeln!(
        file,
        "\n{command:?}\nstatus: {}\n{}{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .unwrap();
    assert!(
        output.status.success(),
        "native gate retained at {}\n{command:?}\n{}{}",
        retained.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
fn build_sdk(plan: &SdkPlan, root: &Path, docs: bool, curl: bool) {
    suspect_codegen::write_files(&plan.render().unwrap(), &root.join("generated")).unwrap();
    let mut configure = Command::new(cmake());
    configure
        .arg("-S")
        .arg(root.join("generated/cpp"))
        .arg("-B")
        .arg(root.join("build-sdk"))
        .arg(format!("-DCMAKE_CXX_COMPILER={}", cxx().display()))
        .arg("-DCMAKE_BUILD_TYPE=Release")
        .arg(format!(
            "-DCMAKE_INSTALL_PREFIX={}",
            root.join("install").display()
        ))
        .arg(format!(
            "-DSUSPECT_SDK_WITH_CURL={}",
            if curl { "ON" } else { "OFF" }
        ))
        .arg(format!(
            "-DSUSPECT_SDK_BUILD_DOCS={}",
            if docs { "ON" } else { "OFF" }
        ));
    if docs {
        configure.arg(format!(
            "-DDOXYGEN_EXECUTABLE={}",
            tool("SUSPECT_CPP_DOXYGEN", "doxygen").display()
        ));
    }
    checked(&mut configure, root);
    checked(
        Command::new(cmake())
            .arg("--build")
            .arg(root.join("build-sdk"))
            .args(["--parallel", "2"]),
        root,
    );
    let ctest = cmake().with_file_name("ctest");
    checked(
        Command::new(ctest)
            .arg("--test-dir")
            .arg(root.join("build-sdk"))
            .arg("--output-on-failure"),
        root,
    );
    if docs {
        checked(
            Command::new(cmake())
                .arg("--build")
                .arg(root.join("build-sdk"))
                .args(["--target", "sdk_docs"]),
            root,
        );
        assert!(root.join("build-sdk/docs/html/index.html").is_file());
        let index = std::fs::read_to_string(root.join("build-sdk/docs/xml/index.xml")).unwrap();
        for name in plan
            .operations()
            .iter()
            .map(|op| op.method_name.as_str())
            .chain(plan.models().symbols().map(|s| s.codec_name.as_str()))
        {
            assert!(
                index.contains(&format!(">{name}</name>"))
                    || index.contains(&format!("::{name}</name>")),
                "native documentation symbol missing: {name}"
            );
        }
    }
    checked(
        Command::new(cmake())
            .arg("--install")
            .arg(root.join("build-sdk")),
        root,
    );
    assert!(
        root.join(format!("install/include/{}/sdk.hpp", plan.config().name))
            .is_file()
    );
}
fn consumer(root: &Path, package: &str, source: &str) -> PathBuf {
    let path = root.join("consumer");
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(path.join("main.cpp"), source).unwrap();
    std::fs::write(path.join("CMakeLists.txt"),format!("cmake_minimum_required(VERSION 3.24)\nproject(InstalledConsumer LANGUAGES CXX)\nfind_package({package} 0.1.0 EXACT CONFIG REQUIRED)\nadd_executable(consumer main.cpp)\ntarget_link_libraries(consumer PRIVATE {package}::{package})\nset_target_properties(consumer PROPERTIES CXX_EXTENSIONS OFF)\ntarget_compile_options(consumer PRIVATE -Wall -Wextra -Wpedantic -Werror)\n")).unwrap();
    checked(
        Command::new(cmake())
            .arg("-S")
            .arg(&path)
            .arg("-B")
            .arg(root.join("build-consumer"))
            .arg(format!("-DCMAKE_CXX_COMPILER={}", cxx().display()))
            .arg("-DCMAKE_BUILD_TYPE=Debug")
            .arg("-DCMAKE_FIND_USE_PACKAGE_REGISTRY=OFF")
            .arg(format!(
                "-DCMAKE_PREFIX_PATH={}",
                root.join("install").display()
            )),
        root,
    );
    checked(
        Command::new(cmake())
            .arg("--build")
            .arg(root.join("build-consumer"))
            .args(["--parallel", "2"]),
        root,
    );
    root.join("build-consumer/consumer")
}
fn native_types(root: &Path) {
    let positive = root.join("positive.cpp");
    std::fs::write(&positive,"#include <generated_sdk/sdk.hpp>\nusing namespace generated_sdk;\nint main() { WidgetInput input(\"alpha\"); input.amount=JsonNumber(2); auto payload=WidgetPayload::alternative_0(StandardPayload(\"plain\")); CreateWidgetInput operation(input); (void)payload; (void)operation; }\n").unwrap();
    let compile = |path: &Path| {
        let mut c = Command::new(cxx());
        c.args([
            "-std=c++20",
            "-Wall",
            "-Wextra",
            "-Wpedantic",
            "-Werror",
            "-fsyntax-only",
            "-I",
        ])
        .arg(root.join("install/include"))
        .arg(path);
        c
    };
    checked(&mut compile(&positive), root);
    for (i, source) in [
        "WidgetInput value;",
        "WidgetInput value(Null{});",
        "WidgetInput value(\"x\"); value.amount=Null{};",
        "auto value=WidgetPayload::alternative_0(SecurePayload(\"vault\"));",
        "CreateWidgetInput value;",
        "JsonNumber value(0.1);",
        "JsonInteger value(true);",
        "JsonValue value(42);",
    ]
    .iter()
    .enumerate()
    {
        let path = root.join(format!("negative-{i}.cpp"));
        std::fs::write(&path,format!("#include <generated_sdk/sdk.hpp>\nusing namespace generated_sdk;\nint main() {{ {source} (void)value; }}\n")).unwrap();
        let output = compile(&path).output().unwrap();
        std::fs::write(root.join(format!("negative-{i}.log")), &output.stderr).unwrap();
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success(),
            "invalid native consumer compiled: {source}"
        );
        assert!(
            !error.contains("file not found") && error.contains("error:"),
            "negative must fail on actual types: {error}"
        );
    }
}

#[test]
#[ignore = "requires C++20, CMake, libcurl and Doxygen; actual installed package and independent HTTP"]
fn native_m2_cpp_cmake_codecs_http_and_docs() {
    let plan = m2();
    let root = root("m2");
    build_sdk(&plan, &root, true, true);
    let executable = consumer(
        &root,
        "generated_sdk",
        include_str!("../src/cpp_sdk/tests/native_m2.cpp"),
    );
    checked(Command::new(executable).arg("codecs"), &root);
    native_types(&root);
    let server = NativeServer::start(&root, false);
    checked(
        native_network_command(&root.join("build-consumer/consumer"), &server)
            .arg("wire")
            .arg(&server.base)
            .arg(&root),
        &root,
    );
    server.verify();
    native_tls(&root);
    println!(
        "C++ M2 installed native package and Doxygen passed: {}",
        root.display()
    );
}

#[test]
#[ignore = "requires the read-only OpenRouter checkout, C++20, libcurl, CMake and Doxygen"]
fn native_five_actual_openrouter_operations() {
    use sha2::{Digest, Sha256};
    let checkout = std::env::var_os("OPENROUTER_WEB_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/Users/luke/github/openrouter-web".into());
    let path = checkout.join("projects/docs/openapi/openapi.yaml");
    let before =
        Sha256::digest(std::fs::read(&path).expect("actual tracked OpenRouter source is required"));
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
    let plan = plan_sdk(contract, &selected, SdkConfig::default()).unwrap();
    let create = plan
        .operations()
        .iter()
        .find(|op| op.operation_id == "createKeys")
        .unwrap();
    let update = plan
        .operations()
        .iter()
        .find(|op| op.operation_id == "updateKeys")
        .unwrap();
    let construction = if update.body.as_ref().unwrap().required {
        "UpdateKeysInput update_input(\"fixture-hash\",update);"
    } else {
        "UpdateKeysInput update_input(\"fixture-hash\"); update_input.body=update;"
    };
    let source = include_str!("../src/cpp_sdk/tests/native_openrouter.cpp")
        .replace(
            "__CREATE_BODY__",
            &plan
                .models()
                .symbol(create.body.as_ref().unwrap().value.schema().unwrap())
                .unwrap()
                .cpp_type,
        )
        .replace(
            "__UPDATE_BODY__",
            &plan
                .models()
                .symbol(update.body.as_ref().unwrap().value.schema().unwrap())
                .unwrap()
                .cpp_type,
        )
        .replace("__UPDATE_CONSTRUCTION__", construction);
    let root = root("openrouter");
    build_sdk(&plan, &root, true, true);
    let executable = consumer(&root, "generated_sdk", &source);
    let server = NativeServer::start(&root, true);
    checked(
        native_network_command(&executable, &server).arg(&server.base),
        &root,
    );
    server.verify();
    assert_eq!(
        before,
        Sha256::digest(std::fs::read(&path).unwrap()),
        "OpenRouter source changed during the read-only gate"
    );
    println!(
        "C++ five actual OpenRouter operations, installed package, native wire and Doxygen passed: {}",
        root.display()
    );
}

#[derive(Debug)]
struct WireRequest {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
    body: String,
}
impl WireRequest {
    fn header(&self, name: &str) -> &str {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
            .unwrap_or("")
    }
}
struct NativeServer {
    base: String,
    proxy: std::net::TcpListener,
    stop: Arc<std::sync::atomic::AtomicBool>,
    records: Arc<std::sync::Mutex<Vec<WireRequest>>>,
    worker: Option<std::thread::JoinHandle<()>>,
    openrouter: bool,
    root: PathBuf,
}
const WIDGET_RESPONSE: &str = r#"{"amount":9007199254740993.000000000000000001,"id":"w1","meta":null,"payload":{"kind":"standard","text":"plain"},"child":{"label":"root","child":{"label":"leaf"}},"extra":{"n":1e999999999999999999999999}}"#;
impl NativeServer {
    fn start(root: &Path, openrouter: bool) -> Self {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}/api/v1", listener.local_addr().unwrap());
        let proxy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        proxy.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let records = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (stopped, recorded, directory, redirect) = (
            stop.clone(),
            records.clone(),
            root.to_owned(),
            format!("http://{}", proxy.local_addr().unwrap()),
        );
        let worker = std::thread::spawn(move || {
            let credits = Arc::new(AtomicUsize::new(0));
            let mut workers = Vec::new();
            while !stopped.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // Darwin can inherit O_NONBLOCK from the listening socket.
                        // The independent transfer fixture deliberately waits for
                        // peer EOF to prove native cancellation/timeout cleanup.
                        stream.set_nonblocking(false).unwrap();
                        let (recorded, directory, redirect, credits) = (
                            recorded.clone(),
                            directory.clone(),
                            redirect.clone(),
                            credits.clone(),
                        );
                        workers.push(std::thread::spawn(move|| {
                            use std::io::{Read,Write};
                            let Some(request)=read_request(&stream) else{return};
                            let method=request.method.clone();let target=request.target.clone();recorded.lock().unwrap().push(request);
                            if openrouter {
                                let fixtures:Value=serde_json::from_str(include_str!("fixtures/openrouter-five-responses.json")).unwrap();
                                let (status,body)=if target=="/api/v1/credits" {
                                    if credits.fetch_add(1,Ordering::SeqCst)==0 {(200,fixtures["credits"].as_str().unwrap())}
                                    else {(401,r#"{"error":{"code":401,"message":"Missing Authentication header"}}"#)}
                                } else if target=="/api/v1/keys" {(201,fixtures["create"].as_str().unwrap())}
                                else if target.starts_with("/api/v1/keys/") {(200,fixtures["update"].as_str().unwrap())}
                                else if target.contains("/files?") {(200,fixtures["list"].as_str().unwrap())}
                                else {(200,fixtures["file"].as_str().unwrap())};
                                write_response(stream,status,"Content-Type: application/json\r\n",body);return;
                            }
                            if target.starts_with("/api/v1/reset/") {return;}
                            let tail=target.rsplit('/').next().unwrap_or("");
                            match tail {
                                "cancelled"|"delay"|"body-cancel"=>{
                                    if tail=="body-cancel" {let _=stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n8\r\n{\"id\":\"x\r\n");}
                                    std::fs::write(directory.join(format!("{tail}-received")),b"received").unwrap();
                                    stream.set_read_timeout(Some(std::time::Duration::from_secs(3))).unwrap();
                                    let closed=match stream.read(&mut [0u8;1]) {Ok(0)=>true,Err(e)=>e.kind()==std::io::ErrorKind::ConnectionReset,_=>false};
                                    if closed {std::fs::write(directory.join(format!("{tail}-closed")),b"closed").unwrap();}
                                    return;
                                }
                                "redirect"=>{write_response(stream,302,&format!("Location: {redirect}/leak\r\nSet-Cookie: secret=forbidden\r\n"),"");return;}
                                "oversized"=>{
                                    let body="x".repeat(512);let _=write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n200\r\n{body}\r\n0\r\n\r\n");return;
                                }
                                "header-large"=>{write_response(stream,200,&format!("Content-Type: application/json\r\nX-Fill: {}\r\n","x".repeat(2048)),WIDGET_RESPONSE);return;}
                                "interim"=>{
                                    for _ in 0..12 {if stream.write_all(b"HTTP/1.1 103 Early Hints\r\nLink: </resource>\r\n\r\n").is_err(){return;}}
                                }
                                "wrong-media"=>{write_response(stream,200,"Content-Type: text/plain\r\n",WIDGET_RESPONSE);return;}
                                "duplicate-media"=>{write_response(stream,200,"Content-Type: application/json\r\nContent-Type: application/json\r\n",WIDGET_RESPONSE);return;}
                                "malformed"=>{write_response(stream,200,"Content-Type: application/json\r\n","{bad");return;}
                                "missing"=>{write_response(stream,404,"Content-Type: application/json\r\n",r#"{"message":"missing"}"#);return;}
                                "declared-large"=>{write_response(stream,404,"Content-Type: application/json\r\n",&format!(r#"{{"message":"{}"}}"#,"x".repeat(256)));return;}
                                "unknown"=>{write_response(stream,500,"Content-Type: application/json\r\n",&"x".repeat(256));return;}
                                "reset"=>return,
                                "cookie"=>{write_response(stream,200,"Content-Type: application/json\r\nSet-Cookie: ambient=forbidden; Path=/\r\n",WIDGET_RESPONSE);return;}
                                _=>{}
                            }
                            let body=if method=="GET" && target.starts_with("/api/v1/widgets?"){r#"{"items":[]}"#}else{WIDGET_RESPONSE};
                            write_response(stream,200,"Content-Type: application/json; charset=utf-8\r\n",body);
                        }));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(2))
                    }
                    Err(e) => panic!("native fixture listener: {e}"),
                }
            }
            for worker in workers {
                worker.join().unwrap();
            }
        });
        Self {
            base,
            proxy,
            stop,
            records,
            worker: Some(worker),
            openrouter,
            root: root.to_owned(),
        }
    }
    fn verify(&self) {
        let records = self.records.lock().unwrap();
        assert!(!records.is_empty(), "libcurl sent no wire requests");
        assert!(!records.iter().any(|r| r.target.contains("/leak")));
        assert!(
            matches!(self.proxy.accept(),Err(e) if e.kind()==std::io::ErrorKind::WouldBlock),
            "libcurl followed an ambient proxy"
        );
        for r in records.iter() {
            assert_eq!(
                r.header("authorization"),
                if self.openrouter {
                    "Bearer management-fixture-token"
                } else {
                    "Bearer m2-key"
                }
            );
            assert_eq!(r.header("accept"), "application/json");
            assert_eq!(r.header("cookie"), "");
            assert_eq!(r.header("referer"), "");
        }
        if self.openrouter {
            assert_eq!(records.len(), 6);
            assert_eq!(
                (&*records[0].method, &*records[0].target),
                ("GET", "/api/v1/credits")
            );
            assert_eq!(
                (&*records[2].method, &*records[2].target),
                ("POST", "/api/v1/keys")
            );
            assert_eq!(
                records[2].body,
                r#"{"limit":50.25,"limit_reset":null,"name":"Native Test Key"}"#
            );
            assert_eq!(
                (&*records[3].method, &*records[3].target),
                ("PATCH", "/api/v1/keys/fixture-hash")
            );
            assert_eq!(
                records[3].body,
                r#"{"disabled":true,"limit":75.50,"limit_reset":null,"name":"Updated Native Key"}"#
            );
            assert_eq!(
                records[4].target,
                "/api/v1/containers/sess_abc123/files/cfile_a%2Fb%20%E9%9B%AA%21%27%28%29%2A"
            );
            assert_eq!(
                records[5].target,
                "/api/v1/containers/sess_abc123/files?limit=2&after=a%2Fb%20%2B%E9%9B%AA"
            );
        } else {
            for (method, target) in [
                ("POST", "/api/v1/widgets"),
                ("PATCH", "/api/v1/widgets/w1"),
                ("GET", "/api/v1/widgets/wire-check"),
                ("GET", "/api/v1/widgets/redirect"),
                ("GET", "/api/v1/widgets/oversized"),
                ("GET", "/api/v1/widgets/header-large"),
                ("GET", "/api/v1/widgets/interim"),
                ("GET", "/api/v1/widgets/reset"),
                ("POST", "/api/v1/reset/widgets"),
            ] {
                assert_eq!(
                    records
                        .iter()
                        .filter(|r| r.target == target && r.method == method)
                        .count(),
                    1,
                    "single exchange required: {method} {target}"
                );
            }
            let post = records.iter().find(|r| r.method == "POST").unwrap();
            assert_eq!(
                post.body,
                r#"{"amount":9007199254740993.000000000000000001,"name":"alpha"}"#
            );
            let patch = records.iter().find(|r| r.method == "PATCH").unwrap();
            assert_eq!(patch.body, r#"{"amount":0.0000000000000000001}"#);
            assert!(records.iter().any(|r|r.target=="/api/v1/widgets?tag=a%2Fb%20%2B%E9%9B%AA&tags=one&tags=a%2Fb&labels=a%2Cb,%E9%9B%AA&limit=2"));
            for name in ["cancelled", "delay", "body-cancel"] {
                assert!(
                    self.root.join(format!("{name}-closed")).is_file(),
                    "I/O was not closed after {name}"
                );
            }
        }
        std::fs::write(self.root.join("wire-records.json"),serde_json::to_string_pretty(&records.iter().map(|r|json!({"method":r.method,"target":r.target,"headers":r.headers,"body":r.body})).collect::<Vec<_>>()).unwrap()).unwrap();
    }
}
impl Drop for NativeServer {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
        if let Ok(records) = self.records.lock() {
            let _=std::fs::write(self.root.join("wire-records.json"),serde_json::to_string_pretty(&records.iter().map(|r|json!({"method":r.method,"target":r.target,"headers":r.headers,"body":r.body})).collect::<Vec<_>>()).unwrap());
        }
    }
}
fn native_network_command(executable: &Path, server: &NativeServer) -> Command {
    let mut command = Command::new(executable);
    let proxy = format!("http://{}", server.proxy.local_addr().unwrap());
    for name in [
        "http_proxy",
        "https_proxy",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "all_proxy",
    ] {
        command.env(name, &proxy);
    }
    command.env("NO_PROXY", "").env("no_proxy", "");
    command
}
fn read_request(mut stream: &std::net::TcpStream) -> Option<WireRequest> {
    use std::io::Read;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok()?;
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    let end = loop {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        bytes.extend_from_slice(&chunk[..n]);
        if bytes.len() > 1024 * 1024 {
            return None;
        }
        if let Some(at) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
    };
    let text = String::from_utf8(bytes[..end].to_vec()).ok()?;
    let mut lines = text.lines();
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.into();
    let target = first.next()?.into();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.into(), v.trim().into()))
        .collect();
    let length = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    if length > 1024 * 1024 {
        return None;
    }
    while bytes.len() - end < length {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
    Some(WireRequest {
        method,
        target,
        headers,
        body: String::from_utf8(bytes[end..end + length].to_vec()).ok()?,
    })
}
fn write_response(mut stream: std::net::TcpStream, status: u16, headers: &str, body: &str) {
    use std::io::Write;
    let _ = write!(
        stream,
        "HTTP/1.1 {status} Fixture\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn native_tls(root: &Path) {
    use std::io::BufRead;
    let tls = root.join("tls");
    std::fs::create_dir_all(&tls).unwrap();
    std::fs::write(tls.join("openssl.cnf"),"[req]\nprompt=no\ndistinguished_name=dn\nx509_extensions=ext\n[dn]\nCN=localhost\n[ext]\nsubjectAltName=DNS:localhost\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,digitalSignature,keyEncipherment,keyCertSign\nextendedKeyUsage=serverAuth\n").unwrap();
    checked(
        Command::new("openssl")
            .current_dir(&tls)
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-sha256",
                "-days",
                "1",
                "-keyout",
                "server.key",
                "-out",
                "server.crt",
                "-config",
                "openssl.cnf",
            ])
            .env("RANDFILE", tls.join(".rnd")),
        root,
    );
    let script = tls.join("server.py");
    std::fs::write(&script, include_str!("../src/cpp_sdk/tests/tls_server.py")).unwrap();
    let mut child = Command::new("python3")
        .arg(&script)
        .arg(&tls)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .unwrap();
    let mut line = String::new();
    std::io::BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let port: u16 = line.trim().parse().unwrap();
    let result = Command::new(root.join("build-consumer/consumer"))
        .arg("tls")
        .arg(format!("https://localhost:{port}/api/v1"))
        .arg(tls.join("server.crt"))
        .output()
        .unwrap();
    let _ = child.kill();
    let _ = child.wait();
    std::fs::write(
        tls.join("consumer.log"),
        [result.stdout.clone(), result.stderr.clone()].concat(),
    )
    .unwrap();
    assert!(
        result.status.success(),
        "TLS native consumer: {}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(tls.join("requests.txt"))
            .unwrap()
            .lines()
            .count(),
        1,
        "only explicitly trusted, hostname-verified HTTPS may reach HTTP"
    );
}

fn cpp_string(value: &str) -> String {
    let mut out = String::from("std::string(\"");
    for byte in value.bytes() {
        match byte {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            32..=126 => out.push(byte as char),
            _ => write!(out, "\\{byte:03o}").unwrap(),
        }
    }
    write!(out, "\", {})", value.len()).unwrap();
    out
}

#[test]
#[ignore = "requires C++20; executes all 17 shared validation vectors independently of native model admission"]
fn native_shared_runtime_contract_vectors() {
    let corpus: Value =
        serde_json::from_str(include_str!("fixtures/runtime-contract-v1.json")).unwrap();
    let cases = corpus["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 17);
    let mut schemas = serde_json::Map::new();
    for (i, case) in cases.iter().enumerate() {
        schemas.insert(format!("V{i:02}"), case["schema"].clone());
    }
    schemas.insert("Cycle".into(), json!({"$ref":"#/components/schemas/Cycle"}));
    schemas.insert(
        "FailureInUnion".into(),
        json!({"anyOf":[true,{"$ref":"#/components/schemas/Cycle"}]}),
    );
    schemas.insert(
        "FailureInNot".into(),
        json!({"not":{"$ref":"#/components/schemas/Cycle"}}),
    );
    schemas.insert(
        "HugeCount".into(),
        serde_json::from_str(
            r#"{"type":"string","maxLength":1e9999999999999999999999999999999999999999999999999}"#,
        )
        .unwrap(),
    );
    schemas.insert("UnicodeEquality".into(), json!({"const":"é"}));
    let contract = fixture(
        json!({"openapi":"3.1.0","info":{"title":"C++ shared runtime","version":"1"},"paths":{},"components":{"schemas":schemas}}),
    );
    let program = OwnedCompiler::new(Config {
        max_depth: 128,
        ..Default::default()
    })
    .compile(contract.clone(), contract.schema_roots())
    .unwrap()
    .program();
    let index = |name: &str| {
        program
            .roots
            .iter()
            .find(|r| r.source.pointer == format!("/components/schemas/{name}"))
            .unwrap()
            .target
    };
    let root = root("runtime");
    suspect_codegen::write_files(
        &emit_validation_runtime(&contract, &program).unwrap(),
        &root,
    )
    .unwrap();
    let mut source = String::from(
        "#include <generated_sdk/runtime.hpp>\n#include <iostream>\n#include <cstdlib>\nusing namespace generated_sdk;\n#define CHECK(x) do { if (!(x)) { std::cerr << \"failed line \" << __LINE__ << '\\n'; std::abort(); } } while(false)\nint main() {\n",
    );
    let mut vectors = 0;
    for (i, case) in cases.iter().enumerate() {
        for (field, expected) in [("valid", true), ("invalid", false)] {
            for text in case[field].as_array().unwrap() {
                writeln!(source,"{{ auto value=parse_json({}); CHECK(value); detail::Validation validation; CHECK(validation.matches({},value.value(),\"\") == {expected}); }}",cpp_string(text.as_str().unwrap()),index(&format!("V{i:02}"))).unwrap();
                vectors += 1;
            }
        }
    }
    for name in ["FailureInUnion", "FailureInNot"] {
        writeln!(source,"{{ bool failed=false; try {{ detail::Validation validation; (void)validation.matches({},JsonValue(),\"\"); }} catch (const detail::Failure& f) {{ failed=f.error.kind==CodecError::Kind::EvaluationFailure; }} CHECK(failed); }}",index(name)).unwrap();
    }
    writeln!(source,"{{ detail::Validation validation; CHECK(validation.matches({},JsonValue(\"x\"),\"\")); CHECK(!validation.matches({},JsonValue(std::string(\"e\\314\\201\")),\"\")); }}",index("HugeCount"),index("UnicodeEquality")).unwrap();
    source.push_str(include_str!("../src/cpp_sdk/tests/runtime_adversarial.inc"));
    source.push_str("std::cout << \"shared-runtime vectors passed\\n\"; return 0; }\n");
    std::fs::write(root.join("main.cpp"), source).unwrap();
    checked(
        Command::new(cxx())
            .args([
                "-std=c++20",
                "-Wall",
                "-Wextra",
                "-Wpedantic",
                "-Werror",
                "-pthread",
                "-I",
            ])
            .arg(root.join("cpp/include"))
            .arg(root.join("cpp/src/runtime.cpp"))
            .arg(root.join("cpp/src/validation.cpp"))
            .arg(root.join("cpp/src/validation_v2.cpp"))
            .arg(root.join("cpp/src/validation_v3.cpp"))
            .arg(root.join("cpp/src/program.cpp"))
            .arg(root.join("main.cpp"))
            .arg("-o")
            .arg(root.join("vectors")),
        &root,
    );
    checked(&mut Command::new(root.join("vectors")), &root);
    println!(
        "C++ all 17 runtime cases ({vectors} valid/invalid instances) and adversarial budgets passed: {}",
        root.display()
    );
}
