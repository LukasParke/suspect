//! Fresh C++20 protocol packages and independent native wire/resource witnesses.
#![cfg(all(feature = "cpp-sdk", feature = "http-protocol"))]
use serde_json::{Value, json};
use std::fmt::Write as _;
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::cpp_sdk::{SdkConfig, SdkPlan, plan_sdk};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}
fn root(label: &str) -> PathBuf {
    let base = repo().join("target/sdk-cpp-protocol-gates");
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix(label)
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
fn plan(document: &Value, root: &Path, config: SdkConfig) -> SdkPlan {
    let file = root.join("api.json");
    std::fs::write(&file, serde_json::to_vec_pretty(document).unwrap()).unwrap();
    let contract = load(&file);
    let selected = contract
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    plan_sdk(contract, &selected, config).unwrap_or_else(|errors| panic!("{errors:#?}"))
}
fn tool(variable: &str, default: &str) -> PathBuf {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| default.into())
}
fn cmake() -> PathBuf {
    tool("SUSPECT_CPP_CMAKE", "cmake")
}
fn cxx() -> PathBuf {
    tool("SUSPECT_CPP_CXX", "clang++")
}
fn checked(command: &mut Command, root: &Path) -> std::process::Output {
    use std::io::Write;
    let output = command.output().unwrap();
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("commands.log"))
        .unwrap();
    writeln!(
        file,
        "\n{command:?}\nstatus={}\n{}{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .unwrap();
    assert!(
        output.status.success(),
        "gate retained at {}\n{command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
fn package(plan: &SdkPlan, root: &Path) {
    package_with_curl(plan, root, true);
}
fn package_with_curl(plan: &SdkPlan, root: &Path, with_curl: bool) {
    suspect_codegen::write_files(&plan.render().unwrap(), &root.join("generated")).unwrap();
    checked(
        Command::new(cmake())
            .arg("-S")
            .arg(root.join("generated/cpp"))
            .arg("-B")
            .arg(root.join("build-sdk"))
            .arg(format!("-DCMAKE_CXX_COMPILER={}", cxx().display()))
            .arg("-DCMAKE_BUILD_TYPE=Release")
            .arg(format!(
                "-DSUSPECT_SDK_WITH_CURL={}",
                if with_curl { "ON" } else { "OFF" }
            ))
            .arg(format!(
                "-DCMAKE_INSTALL_PREFIX={}",
                root.join("install").display()
            ))
            .arg("-DSUSPECT_SDK_BUILD_DOCS=ON")
            .arg(format!(
                "-DDOXYGEN_EXECUTABLE={}",
                tool("SUSPECT_CPP_DOXYGEN", "doxygen").display()
            )),
        root,
    );
    checked(
        Command::new(cmake())
            .arg("--build")
            .arg(root.join("build-sdk"))
            .args(["--parallel", "2"]),
        root,
    );
    checked(
        Command::new(cmake().with_file_name("ctest"))
            .arg("--test-dir")
            .arg(root.join("build-sdk"))
            .arg("--output-on-failure"),
        root,
    );
    checked(
        Command::new(cmake())
            .arg("--build")
            .arg(root.join("build-sdk"))
            .args(["--target", "sdk_docs"]),
        root,
    );
    checked(
        Command::new(cmake())
            .arg("--install")
            .arg(root.join("build-sdk")),
        root,
    );
    assert!(root.join("build-sdk/docs/html/index.html").is_file());
}
fn consumer(root: &Path, source: &str) -> PathBuf {
    let directory = root.join("consumer");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("main.cpp"), source).unwrap();
    std::fs::write(directory.join("CMakeLists.txt"),"cmake_minimum_required(VERSION 3.24)\nproject(ProtocolConsumer LANGUAGES CXX)\nfind_package(generated_sdk 0.1.0 EXACT CONFIG REQUIRED)\nadd_executable(consumer main.cpp)\ntarget_link_libraries(consumer PRIVATE generated_sdk::generated_sdk)\ntarget_compile_options(consumer PRIVATE -Wall -Wextra -Wpedantic -Werror)\nset_target_properties(consumer PROPERTIES CXX_EXTENSIONS OFF)\n").unwrap();
    checked(
        Command::new(cmake())
            .arg("-S")
            .arg(&directory)
            .arg("-B")
            .arg(root.join("build-consumer"))
            .arg(format!("-DCMAKE_CXX_COMPILER={}", cxx().display()))
            .arg(format!(
                "-DCMAKE_PREFIX_PATH={}",
                root.join("install").display()
            ))
            .arg("-DCMAKE_FIND_USE_PACKAGE_REGISTRY=OFF"),
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

#[test]
fn shared_protocol_roots_preserve_real_parts_headers_and_items() {
    let witnesses: Value =
        serde_json::from_str(include_str!("fixtures/http-protocol-v1.json")).unwrap();
    for name in ["baseline", "multipart", "form", "stream"] {
        let root = root(name);
        let plan = plan(&witnesses[name], &root, Default::default());
        assert!(plan.protocol().is_admitted());
        assert_eq!(plan.examples().format(), "suspect-sdk-examples-v2");
        for id in plan.protocol().codec_roots() {
            assert!(plan.models().symbol(id).is_some());
        }
        if name == "multipart" {
            assert_eq!(plan.aggregates().len(), 1);
            let file = &plan.aggregates()[0]
                .fields
                .iter()
                .find(|p| p.name.as_deref() == Some("file"))
                .unwrap();
            assert!(matches!(
                file.value.kind,
                suspect_codegen::cpp_sdk::ValueKind::Bytes
            ));
            assert!(
                !plan
                    .protocol()
                    .codec_roots()
                    .contains(file.wire.schema().id())
            );
        }
    }
}

#[test]
#[ignore = "C++20/libcurl/CMake/Doxygen: fresh expanded-runtime regression package"]
fn native_fresh_m2_codecs_and_installed_package() {
    let input =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m2/canonical.openapi.yaml");
    let contract = load(&input);
    let selected = contract
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    let plan = plan_sdk(
        contract,
        &selected,
        SdkConfig {
            max_capture_bytes: 32,
            ..Default::default()
        },
    )
    .unwrap();
    let root = root("m2-");
    package(&plan, &root);
    let exe = consumer(&root, include_str!("../src/cpp_sdk/tests/native_m2.cpp"));
    checked(Command::new(exe).arg("codecs"), &root);
    println!(
        "fresh C++ expanded-runtime M2 package/installed consumer: {}",
        root.display()
    );
}

#[test]
fn undefined_stream_and_positional_profiles_are_source_located_refusals() {
    let document = json!({"openapi":"3.1.2","info":{"title":"No inferred stream","version":"1"},"paths":{"/x":{"get":{"responses":{"200":{"description":"legacy","content":{"text/event-stream":{"schema":{"type":"object"},"x-speakeasy-sse-sentinel":"[DONE]"}}}}}}}});
    let root = root("unsupported-");
    let file = root.join("api.json");
    std::fs::write(&file, document.to_string()).unwrap();
    let contract = load(&file);
    let sources = contract
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    let errors = plan_sdk(contract, &sources, Default::default()).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.code == "http-stream-item-schema-required" && !e.at.is_empty())
    );
    let witnesses: Value =
        serde_json::from_str(include_str!("fixtures/http-protocol-v1.json")).unwrap();
    let positional = root.join("positional.json");
    std::fs::write(&positional, witnesses["positionalMultipart"].to_string()).unwrap();
    let contract = load(&positional);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let errors = plan_sdk(contract, &selected, Default::default()).unwrap_err();
    assert!(errors.iter().any(|e| e.code == "http-capability-required"
        && e.message.contains("PositionalMultipart")
        && !e.at.is_empty()));
}

#[test]
#[ignore = "requires the actual read-only OpenRouter checkout"]
fn actual_openrouter_expanded_selection_has_real_byte_and_form_roots() {
    let checkout = std::env::var_os("OPENROUTER_WEB_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/Users/luke/github/openrouter-web".into());
    let file = checkout.join("projects/docs/openapi/openapi.yaml");
    assert!(file.is_file(), "actual OpenRouter corpus required");
    let contract = load(&file);
    let names = [
        "downloadFileContent",
        "downloadContainerFileContent",
        "createCoinbaseCharge",
    ];
    let selected = contract
        .operations()
        .filter(|op| op.operation_id().is_some_and(|name| names.contains(&name)))
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), names.len());
    let plan = plan_sdk(
        contract,
        &selected,
        SdkConfig {
            legacy_binary_strings: true,
            ..Default::default()
        },
    )
    .unwrap_or_else(|e| panic!("{e:#?}"));
    assert_eq!(plan.operations().len(), 3);
    assert!(
        plan.operations()
            .iter()
            .flat_map(|op| &op.responses)
            .flat_map(|r| &r.cases)
            .any(|case| matches!(case.value.kind, suspect_codegen::cpp_sdk::ValueKind::Bytes))
    );
    let contract = load(&file);
    let oauth = contract
        .operations()
        .filter(|op| op.operation_id() == Some("createOauthToken"))
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let refused = plan_sdk(contract, &oauth, Default::default()).unwrap_err();
    assert!(refused.iter().any(|e| e.code == "http-form-untyped-extras"
        && e.source.pointer() == "/components/schemas/TokenExchangeRequest"));
}

#[test]
#[ignore = "actual OpenRouter input and installed native C++ bytes/no-content HTTP operations"]
fn native_additional_openrouter_operations() {
    use sha2::{Digest, Sha256};
    let checkout = std::env::var_os("OPENROUTER_WEB_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/Users/luke/github/openrouter-web".into());
    let path = checkout.join("projects/docs/openapi/openapi.yaml");
    let before = Sha256::digest(std::fs::read(&path).unwrap());
    let contract = load(&path);
    let names = [
        "downloadFileContent",
        "downloadContainerFileContent",
        "createCoinbaseCharge",
    ];
    let selected = contract
        .operations()
        .filter(|op| op.operation_id().is_some_and(|id| names.contains(&id)))
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 3);
    let plan = plan_sdk(
        contract,
        &selected,
        SdkConfig {
            legacy_binary_strings: true,
            ..Default::default()
        },
    )
    .unwrap();
    let root = root("openrouter-protocol-");
    package(&plan, &root);
    let exe = consumer(
        &root,
        include_str!("../src/cpp_sdk/tests/native_openrouter_protocol.cpp"),
    );
    let server = ProtocolServer::start(&root, Vec::new());
    checked(Command::new(exe).arg(&server.base), &root);
    {
        let records = server.records.lock().unwrap();
        assert_eq!(records.len(), 4);
        assert_eq!(
            records[0].target,
            "/base/containers/sess_abc123/files/cfile_a%2Fb%20%E9%9B%AA/content"
        );
        assert_eq!(
            records[1].target,
            "/base/files/file_1/content?workspace_id=workspace%201"
        );
        for request in &records[..2] {
            assert_eq!(request.method, "GET");
            assert_eq!(
                request.header("authorization"),
                "Bearer openrouter-fixture-token"
            );
        }
        for request in &records[2..] {
            assert_eq!(request.target, "/base/credits/coinbase");
            assert_eq!(request.method, "POST");
            assert_eq!(request.header("authorization"), "");
            assert!(request.body.is_empty());
        }
    }
    assert_eq!(before, Sha256::digest(std::fs::read(&path).unwrap()));
    println!(
        "additional OpenRouter native package/wire gate: {}",
        root.display()
    );
}

fn cpp_string(value: &str) -> String {
    let mut literal = String::from("std::string(\"");
    for byte in value.bytes() {
        match byte {
            b'"' => literal.push_str("\\\""),
            b'\\' => literal.push_str("\\\\"),
            32..=126 => literal.push(byte as char),
            _ => write!(literal, "\\{byte:03o}").unwrap(),
        }
    }
    write!(literal, "\", {})", value.len()).unwrap();
    literal
}
fn rich_document() -> (Value, Vec<Value>) {
    let mut document: Value =
        serde_json::from_str(include_str!("../src/cpp_sdk/tests/protocol.openapi.json")).unwrap();
    let normative: Value =
        serde_json::from_str(include_str!("fixtures/http-protocol-v1.json")).unwrap();
    let cases = normative["parameterCases"].as_array().unwrap().clone();
    for (i, case) in cases.iter().enumerate() {
        let mut parameter = case["parameter"].clone();
        parameter["required"] = json!(true);
        // The 3.1 form-cookie example requests form explicitly in this 3.2 OAD.
        if parameter["in"] == "cookie"
            && parameter.get("style").is_none()
            && parameter.get("content").is_none()
        {
            parameter["style"] = json!("form");
        }
        let path = if parameter["in"] == "path" {
            format!("/p{i}/{{color}}")
        } else {
            format!("/p{i}")
        };
        document["paths"][path] = json!({"get":{"operationId":format!("parameter{i}"),"parameters":[parameter],"responses":{"200":{"content":{"text/plain":{"schema":{"type":"string"}}}}}}});
    }
    (document, cases)
}
fn native_aliases(plan: &SdkPlan) -> String {
    let operation = |name: &str| {
        plan.operations()
            .iter()
            .find(|op| op.operation_id == name)
            .unwrap()
    };
    let aggregate = |name: &str| {
        let body = operation(name).body.as_ref().unwrap();
        let suspect_codegen::cpp_sdk::ValueKind::Aggregate(name) = &body.value.kind else {
            panic!("aggregate")
        };
        plan.aggregates()
            .iter()
            .find(|a| &a.type_name == name)
            .unwrap()
    };
    let mut aliases = Vec::new();
    aliases.push((
        "TestWholeJson",
        operation("queryJson").parameters[0].value.cpp_type.clone(),
    ));
    aliases.push((
        "TestWholeForm",
        operation("queryForm").parameters[0].value.cpp_type.clone(),
    ));
    let form = aggregate("form");
    aliases.push(("TestPackedForm", aggregate("packedForm").type_name.clone()));
    let styled = aggregate("styledUpload");
    aliases.push(("TestStyled", styled.type_name.clone()));
    for (key, name) in [
        ("coordinates", "TestCoordinatesPart"),
        ("deep", "TestDeepPart"),
        ("labels", "TestStyledLabelPart"),
    ] {
        aliases.push((
            name,
            styled
                .fields
                .iter()
                .find(|p| p.name.as_deref() == Some(key))
                .unwrap()
                .part_type
                .clone()
                .unwrap(),
        ));
    }
    aliases.push(("TestForm", form.type_name.clone()));
    aliases.push((
        "TestAddress",
        form.fields
            .iter()
            .find(|p| p.name.as_deref() == Some("address"))
            .unwrap()
            .value
            .cpp_type
            .clone(),
    ));
    let upload = aggregate("upload");
    aliases.push(("TestUpload", upload.type_name.clone()));
    for (wire, name, headers, value) in [
        ("file", "TestFilePart", Some("TestPartHeaders"), None),
        ("metadata", "TestMetadataPart", None, Some("TestMetadata")),
        ("labels", "TestLabelPart", None, None),
    ] {
        let part = upload
            .fields
            .iter()
            .find(|p| p.name.as_deref() == Some(wire))
            .unwrap();
        aliases.push((name, part.part_type.clone().unwrap()));
        if let Some(name) = headers {
            aliases.push((name, part.headers_type.clone().unwrap()));
        }
        if let Some(name) = value {
            aliases.push((name, part.value.cpp_type.clone()));
        }
    }
    for (media, name) in [
        ("application/json", "TestJsonBody"),
        ("text/plain", "TestTextBody"),
        ("application/octet-stream", "TestBinaryBody"),
        ("*/*", "TestAnyBody"),
    ] {
        let medium = operation("media")
            .body
            .as_ref()
            .unwrap()
            .media
            .iter()
            .find(|m| m.wire.media_type().declared() == media)
            .unwrap();
        aliases.push((name, medium.wrapper_type.clone()));
    }
    for (media, name) in [
        ("application/json", "TestJsonResponse"),
        ("application/json;profile=v2", "TestProfileResponse"),
        ("application/problem+json", "TestProblemResponse"),
        ("text/plain", "TestTextResponse"),
        ("application/*", "TestApplicationResponse"),
        ("*/*", "TestAnyResponse"),
    ] {
        let case = operation("media").responses[0]
            .cases
            .iter()
            .find(|c| {
                c.media
                    .as_ref()
                    .is_some_and(|m| m.media_type().declared() == media)
            })
            .unwrap();
        aliases.push((name, case.variant_type.clone()));
    }
    for (op, pattern, forbidden, name) in [
        ("precedence", "2XX", false, "TestRangeResponse"),
        ("precedence", "2XX", true, "TestRangeNone"),
        ("precedence", "default", false, "TestDefaultResponse"),
        ("defaultStatus", "default", false, "TestDefaultSuccess"),
        ("form", "200", false, "TestFormResponse"),
        ("upload", "200", false, "TestUploadResponse"),
        ("packedForm", "200", false, "TestPackedResponse"),
        ("styledUpload", "200", false, "TestStyledResponse"),
    ] {
        let response = operation(op)
            .responses
            .iter()
            .find(|r| r.status_key == pattern)
            .unwrap();
        let case = response
            .cases
            .iter()
            .find(|c| c.forbidden == forbidden)
            .unwrap();
        aliases.push((name, case.variant_type.clone()));
    }
    aliases
        .into_iter()
        .map(|(name, ty)| format!("using {name} = {ty};\n"))
        .collect()
}

#[test]
#[ignore = "installed native C++20 rich protocol, normative styles, real libcurl wire and live streams"]
fn native_rich_protocol_installed_wire_and_streams() {
    let (document, cases) = rich_document();
    let root = root("rich-");
    let plan = plan(&document, &root, Default::default());
    package(&plan, &root);
    let mut calls = String::new();
    for (i, case) in cases.iter().enumerate() {
        let op = plan
            .operations()
            .iter()
            .find(|o| o.operation_id == format!("parameter{i}"))
            .unwrap();
        let schema = op.parameters[0].value.schema().unwrap();
        let model = plan.models().symbol(schema).unwrap();
        writeln!(calls,"{{ auto value = {}::decode({}); CHECK(value); CHECK(client.{}({}(std::move(value).value()))); }}",model.codec_name,cpp_string(&case["value"].to_string()),op.method_name,op.input_type).unwrap();
    }
    let source = include_str!("../src/cpp_sdk/tests/native_protocol.cpp")
        .replace("__ALIASES__", &native_aliases(&plan))
        .replace("__PARAMETER_CALLS__", &calls)
        .replace(
            "__EDGE_CASES__",
            include_str!("../src/cpp_sdk/tests/native_protocol_edges.cpp"),
        );
    let exe = consumer(&root, &source);
    negative_consumers(&root, &native_aliases(&plan));
    let server = ProtocolServer::start(&root, cases);
    checked(Command::new(exe).arg(&server.base).arg(&root), &root);
    server.verify();
    println!(
        "C++ rich protocol installed/native wire/stream gate passed: {}",
        root.display()
    );
}

fn negative_consumers(root: &Path, aliases: &str) {
    for(i,body)in [
        "MediaInput value(Record(\"missing explicit media choice\"));",
        "TestFilePart value(Bytes{1,2});",
        "TestFilePart value(Bytes{1,2},TestPartHeaders(\"x\")); value.data=std::string(\"not bytes\");",
        "TestUpload value;",
        "Credentials value; value.basic=std::string(\"not a username/password pair\");",
        "using Stream=decltype(std::declval<EventsStatus200>().data); static_assert(std::is_copy_constructible_v<Stream>); int value=0;",
    ].iter().enumerate(){
        let file=root.join(format!("negative-protocol-{i}.cpp"));std::fs::write(&file,format!("#include <generated_sdk/sdk.hpp>\nusing namespace generated_sdk;\n{aliases}\nint main(){{{body} (void)value;}}\n")).unwrap();
        let output=Command::new(cxx()).args(["-std=c++20","-Wall","-Wextra","-Wpedantic","-Werror","-Dgenerated_sdk_HAS_CURL=1","-fsyntax-only","-I"]).arg(root.join("install/include")).arg(&file).output().unwrap();
        std::fs::write(root.join(format!("negative-protocol-{i}.log")),&output.stderr).unwrap();
        assert!(!output.status.success(),"negative protocol consumer compiled: {body}");assert!(!String::from_utf8_lossy(&output.stderr).contains("file not found"));
    }
}

#[test]
#[ignore = "installed C++ dialect witnesses: OAS 3.1 form style and 3.0 nullable/binary semantics"]
fn native_dialect_form_and_nullable_bytes() {
    for version in ["3.1.2", "3.0.4"] {
        let schema = if version.starts_with("3.0") {
            json!({"type":"object","required":["note"],"properties":{"note":{"type":"string","nullable":true}}})
        } else {
            json!({"type":"object","required":["id"],"properties":{"id":{"type":"string"},"codes":{"type":"array","items":{"type":"integer"}}},"additionalProperties":false})
        };
        let media = if version.starts_with("3.0") {
            "application/json"
        } else {
            "application/x-www-form-urlencoded"
        };
        let response_schema = schema.clone();
        let mut representation = json!({"schema":schema});
        if version.starts_with("3.1") {
            representation["encoding"] = json!({"codes":{"style":"form","explode":false}});
        }
        let mut document = json!({"openapi":version,"info":{"title":"Native dialect witness","version":"1"},"servers":[{"url":"https://example.test"}],"security":[],"paths":{"/submit":{"post":{"operationId":"submit","requestBody":{"required":true,"content":{media:representation}},"responses":{"200":{"description":"JSON response","content":{"application/json":{"schema":response_schema}}}}}}}});
        if version.starts_with("3.0") {
            document["paths"]["/binary"] = json!({"get":{"operationId":"binary","responses":{"200":{"description":"bytes","content":{"application/octet-stream":{"schema":{"type":"string","format":"binary"}}}}}}});
        }
        let root = root("dialect-");
        let plan = plan(&document, &root, Default::default());
        package(&plan, &root);
        let submit = plan
            .operations()
            .iter()
            .find(|op| op.operation_id == "submit")
            .unwrap();
        let ty = &submit.body.as_ref().unwrap().value.cpp_type;
        let (setup, expected, verification) = if version.starts_with("3.0") {
            (
                format!("{ty} body(Null{{}});"),
                "{\"note\":null}",
                "CHECK(std::holds_alternative<Null>(value.data.note)); auto binary=client.binary(); CHECK(binary); CHECK(std::get<BinaryStatus200>(binary.value()).data==Bytes({0,255}));",
            )
        } else {
            (
                format!(
                    "{ty} body(\"a + b\"); body.codes=std::vector<JsonInteger>{{JsonInteger(1),JsonInteger(2)}};"
                ),
                "codes=1,2&id=a+%2B+b",
                "CHECK(value.data.id==\"a + b\"); CHECK(value.data.codes->at(1).token()==\"2\");",
            )
        };
        let source = format!(
            r#"#include <generated_sdk/sdk.hpp>
#include <cstdlib>
#include <iostream>
using namespace generated_sdk;
#define CHECK(x) do {{ if(!(x)) {{std::cerr<<__LINE__<<" "<<#x;std::abort();}} }} while(false)
class Fixture final:public Transport {{public: Result<HttpResponse,TransportError> send(const HttpRequest& request,const TransportOptions&)const override {{
if(request.url.ends_with("/binary"))return Result<HttpResponse,TransportError>::success({{200,{{{{"Content-Type","application/octet-stream"}}}},std::string("\0\377",2)}});
CHECK(request.method=="POST");CHECK(request.body&&*request.body=={expected});return Result<HttpResponse,TransportError>::success({{200,{{{{"Content-Type","application/json"}}}},{response}}});}}}};
int main(){{Client client(std::make_shared<Fixture>());{setup}auto result=client.submit(SubmitInput(body));CHECK(result);const auto& value=std::get<SubmitStatus200>(result.value());{verification}std::cout<<"native dialect passed\n";}}
"#,
            expected = cpp_string(expected),
            response = cpp_string(if version.starts_with("3.0") {
                r#"{"note":null}"#
            } else {
                r#"{"codes":[1,2],"id":"a + b"}"#
            })
        );
        let exe = consumer(&root, &source);
        checked(&mut Command::new(exe), &root);
        println!("C++ {version} dialect package passed: {}", root.display());
    }
}

#[test]
#[ignore = "installed C++ resource witnesses: request and response items share codec/evaluation budgets"]
fn native_stream_budgets_are_shared_across_request_and_every_item() {
    let response = json!({"200":{"content":{"application/x-ndjson":{"itemSchema":{"$ref":"#/components/schemas/Count"}}}}});
    let document = json!({"openapi":"3.2.0","info":{"title":"One call budget","version":"1"},"servers":[{"url":"https://example.test"}],"components":{"schemas":{"Count":{"type":"integer","const":1}}},"paths":{
        "/collect":{"get":{"operationId":"collect","responses":response}},
        "/warm":{"post":{"operationId":"warm","requestBody":{"required":true,"content":{"application/json":{"schema":{"type":"array","minItems":4,"items":{"type":"integer"}}}}},"responses":response}}
    }});
    for (label, steps, work) in [("evaluation", 100, 8192), ("codec-work", 100000, 700)] {
        let root = root(&format!("budget-{label}-"));
        let plan = plan(
            &document,
            &root,
            SdkConfig {
                max_json_work: work,
                validation: suspect_schema::Config {
                    max_evaluation_steps: steps,
                    ..SdkConfig::default().validation
                },
                ..Default::default()
            },
        );
        package(&plan, &root);
        let exe = consumer(
            &root,
            include_str!("../src/cpp_sdk/tests/native_protocol_budgets.cpp"),
        );
        checked(&mut Command::new(exe), &root);
        println!("C++ shared {label} budget package: {}", root.display());
    }
}

#[test]
#[ignore = "fresh advanced C++ HTTPS streams, cookie style, request JSON-lines and typed aggregate extras"]
fn native_advanced_protocol_https_and_remaining_representations() {
    use std::io::BufRead;
    let mut document = json!({"openapi":"3.2.0","info":{"title":"Advanced native protocol","version":"1"},"servers":[{"url":"https://example.test"}],"components":{"schemas":{
        "Colors":{"type":"object","required":["G","R"],"properties":{"G":{"type":"string"},"R":{"type":"string"}},"additionalProperties":false},
        "Event":{"type":"object","required":["data"],"properties":{"data":{"type":"string"}}},
        "Line":{"type":"object","required":["n"],"properties":{"n":{"type":"integer"}},"additionalProperties":false}
    }},"paths":{
        "/events/{mode}":{"get":{"operationId":"events","parameters":[{"name":"mode","in":"path","required":true,"schema":{"type":"string"}}],"responses":{"200":{"content":{"text/event-stream":{"itemSchema":{"$ref":"#/components/schemas/Event"}}}}}}},
        "/cookie":{"get":{"operationId":"cookie","parameters":[{"name":"color","in":"cookie","style":"cookie","required":true,"schema":{"$ref":"#/components/schemas/Colors"}}],"responses":{"204":{}}}},
        "/arrayCookie":{"get":{"operationId":"arrayCookie","parameters":[{"name":"tags","in":"cookie","style":"cookie","required":true,"schema":{"type":"array","items":{"type":"string"}}}],"responses":{"204":{}}}},
        "/emitLines":{"post":{"operationId":"emitLines","requestBody":{"required":true,"content":{"application/x-ndjson":{"itemSchema":{"$ref":"#/components/schemas/Line"}}}},"responses":{"204":{}}}}
    }});
    for (path, id, media) in [
        (
            "/extraForm",
            "extraForm",
            "application/x-www-form-urlencoded",
        ),
        ("/extraMultipart", "extraMultipart", "multipart/form-data"),
    ] {
        let content = json!({media:{"schema":{"type":"object","required":["name"],"minProperties":2,"maxProperties":3,"properties":{"name":{"type":"string"}},"additionalProperties":{"type":"integer"}}}});
        document["paths"][path] = json!({"post":{"operationId":id,"requestBody":{"required":true,"content":content},"responses":{"200":{"content":content}}}});
    }
    let root = root("advanced-");
    let plan = plan(&document, &root, Default::default());
    package(&plan, &root);
    let aggregate = |id: &str| {
        let op = plan
            .operations()
            .iter()
            .find(|op| op.operation_id == id)
            .unwrap();
        let suspect_codegen::cpp_sdk::ValueKind::Aggregate(name) =
            &op.body.as_ref().unwrap().value.kind
        else {
            unreachable!()
        };
        plan.aggregates()
            .iter()
            .find(|a| &a.type_name == name)
            .unwrap()
    };
    let form = aggregate("extraForm");
    let multipart = aggregate("extraMultipart");
    let aliases = format!(
        "using TestExtraForm = {};\nusing TestExtraMultipart = {};\nusing TestKnownPart = {};\nusing TestExtraPart = {};\n",
        form.type_name,
        multipart.type_name,
        multipart.fields[0].part_type.as_ref().unwrap(),
        multipart
            .additional
            .as_ref()
            .unwrap()
            .part_type
            .as_ref()
            .unwrap()
    );
    let source = include_str!("../src/cpp_sdk/tests/native_protocol_advanced.cpp")
        .replace("__ADVANCED_ALIASES__", &aliases);
    let exe = consumer(&root, &source);
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
        &root,
    );
    let script = tls.join("server.py");
    std::fs::write(
        &script,
        include_str!("../src/cpp_sdk/tests/native_protocol_tls.py"),
    )
    .unwrap();
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut server = Child(
        Command::new("python3")
            .arg(script)
            .arg(&tls)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let mut line = String::new();
    std::io::BufReader::new(server.0.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let port: u16 = line.trim().parse().unwrap();
    checked(Command::new(exe).arg(port.to_string()).arg(&tls), &root);
    let records = std::fs::read_to_string(tls.join("wire.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        records.len(),
        8,
        "only explicitly trusted hostname-verified calls reach HTTP; invalid mutable inputs stay local"
    );
    let request = |path: &str| records.iter().find(|r| r["path"] == path).unwrap();
    assert_eq!(request("/cookie")["headers"]["cookie"], "G=green; R=red");
    assert_eq!(
        request("/arrayCookie")["headers"]["cookie"],
        "tags=one; tags=two"
    );
    let bytes = |record: &Value| {
        record["body"]
            .as_array()
            .unwrap()
            .iter()
            .map(|byte| byte.as_u64().unwrap() as u8)
            .collect::<Vec<_>>()
    };
    assert_eq!(request("/emitLines")["method"], "POST");
    assert_eq!(
        bytes(request("/emitLines")),
        b"{\"n\":1.0}\n{\"n\":1e100000000000000000000}\n"
    );
    assert_eq!(
        bytes(request("/extraForm")),
        b"name=known&count=9007199254740993"
    );
    let parts = mime_parts(
        &bytes(request("/extraMultipart")),
        request("/extraMultipart")["headers"]["content-type"]
            .as_str()
            .unwrap(),
    );
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].2, b"known");
    assert_eq!(parts[1].0, "雪");
    assert_eq!(parts[1].2, b"2");
    for mode in ["lease", "cancel", "timeout"] {
        assert!(tls.join(format!("{mode}-closed")).is_file());
    }
    println!(
        "C++ advanced verified HTTPS/new representation gate: {}",
        root.display()
    );
}

#[test]
#[ignore = "fresh installed core-only C++ byte/Unit package with zero JSON codec roots and native constructor example"]
fn native_byte_only_core_package_has_no_json_standins() {
    let document = json!({"openapi":"3.2.0","info":{"title":"Real bytes without a JSON model","version":"1"},"servers":[{"url":"https://example.test"}],"paths":{
        "/bytes":{"post":{"operationId":"roundtripBytes","requestBody":{"required":true,"content":{"application/octet-stream":{}}},"responses":{"200":{"content":{"application/octet-stream":{}}}}}},
        "/empty":{"get":{"operationId":"empty","responses":{"204":{}}}}
    }});
    let root = root("bytes-only-");
    let plan = plan(&document, &root, Default::default());
    assert!(plan.protocol().codec_roots().is_empty());
    assert_eq!(plan.models().symbols().count(), 0);
    let files = plan.render().unwrap();
    let example = &files
        .iter()
        .find(|file| file.path == "cpp/examples/client.cpp")
        .unwrap()
        .content;
    assert!(example.contains("Bytes{0x00, 0xff}"));
    assert!(!example.contains("JsonValue("));
    package_with_curl(&plan, &root, false);
    let source = r#"#include <generated_sdk/sdk.hpp>
#include <cstdlib>
using namespace generated_sdk;
class Echo final:public Transport{public: Result<HttpResponse,TransportError> send(const HttpRequest& request,const TransportOptions&) const override {
if(request.url.ends_with("/empty"))return Result<HttpResponse,TransportError>::success(HttpResponse{204,{},{}});
if(!request.body)std::abort();return Result<HttpResponse,TransportError>::success(HttpResponse{200,{{"Content-Type","application/octet-stream"}},*request.body});}};
int main(){Client client(std::make_shared<Echo>());auto value=client.roundtrip_bytes(RoundtripBytesInput(Bytes{0,255,0,1}));if(!value||std::get<RoundtripBytesStatus200>(value.value()).data!=Bytes({0,255,0,1}))return 1;return client.empty()?0:2;}
"#;
    let exe = consumer(&root, source);
    checked(&mut Command::new(exe), &root);
    println!(
        "C++ zero-codec-root installed core-only byte/Unit package: {}",
        root.display()
    );
}

#[test]
#[ignore = "native OAS 3.2 deepObject part ignores explode while preserving typed multipart values"]
fn native_oas32_deep_object_part_ignores_explode() {
    let mut document: Value =
        serde_json::from_str(include_str!("../src/cpp_sdk/tests/protocol.openapi.json")).unwrap();
    let paths = document["paths"].as_object_mut().unwrap();
    paths.retain(|path, _| path == "/styledUpload");
    let op = &mut paths["/styledUpload"]["post"];
    op["requestBody"]["content"]["multipart/form-data"]["encoding"]["deep"]["explode"] =
        json!(false);
    op["responses"]["200"]["content"]["multipart/form-data"]["encoding"]["deep"]["explode"] =
        json!(false);
    let root = root("deep-object-");
    let plan = plan(&document, &root, Default::default());
    package(&plan, &root);
    let aggregate = plan
        .aggregates()
        .iter()
        .find(|a| a.source.pointer().contains("requestBody"))
        .unwrap();
    let mut aliases = format!("using Body = {};\n", aggregate.type_name);
    for (wire, alias) in [
        ("coordinates", "CoordinatesPart"),
        ("deep", "DeepPart"),
        ("labels", "LabelPart"),
    ] {
        writeln!(
            aliases,
            "using {alias} = {};",
            aggregate
                .fields
                .iter()
                .find(|p| p.name.as_deref() == Some(wire))
                .unwrap()
                .part_type
                .as_ref()
                .unwrap()
        )
        .unwrap();
    }
    let source = format!(
        r#"#include <generated_sdk/sdk.hpp>
#include <cstdlib>
using namespace generated_sdk;
{aliases}
class Echo final:public Transport {{public: mutable unsigned calls=0;Result<HttpResponse,TransportError> send(const HttpRequest& request,const TransportOptions&) const override {{
++calls;if(!request.body||request.body->find("deep%5Bx%5D=3&deep%5By%5D=deep")==std::string::npos)std::abort();
return Result<HttpResponse,TransportError>::success(HttpResponse{{200,request.headers,*request.body}});}}}};
int main(){{auto transport=std::make_shared<Echo>();Client client(transport);Body body(CoordinatesPart(Coordinates(JsonInteger(2),"packed")),DeepPart(Coordinates(JsonInteger(3),"deep")),std::vector<LabelPart>{{LabelPart("one")}});
auto result=client.styled_upload(StyledUploadInput(body));if(!result||std::get<StyledUploadStatus200>(result.value()).data.deep.data.y!="deep")return 1;
body.deep.data.y="bad&delimiter";if(client.styled_upload(StyledUploadInput(body))||transport->calls!=1)return 2;
}}
"#
    );
    let exe = consumer(&root, &source);
    checked(&mut Command::new(exe), &root);
    println!(
        "C++ OAS 3.2 deepObject false-explode part gate: {}",
        root.display()
    );
}

#[derive(Clone, Debug)]
struct Request {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}
impl Request {
    fn header(&self, name: &str) -> &str {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
            .unwrap_or("")
    }
}
struct ProtocolServer {
    base: String,
    stop: Arc<std::sync::atomic::AtomicBool>,
    records: Arc<std::sync::Mutex<Vec<Request>>>,
    worker: Option<std::thread::JoinHandle<()>>,
    root: PathBuf,
    cases: Vec<Value>,
}
impl ProtocolServer {
    fn start(root: &Path, cases: Vec<Value>) -> Self {
        use std::sync::atomic::{AtomicBool, Ordering};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}/base", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let records = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (stopped, recorded, directory) = (stop.clone(), records.clone(), root.to_owned());
        let worker = std::thread::spawn(move || {
            let mut workers = Vec::new();
            while !stopped.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        let (recorded, directory) = (recorded.clone(), directory.clone());
                        workers.push(std::thread::spawn(move || {
                            if let Some(request) = read_request(&stream) {
                                recorded.lock().unwrap().push(request.clone());
                                serve(stream, &request, &directory);
                            }
                        }));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(2))
                    }
                    Err(e) => panic!("{e}"),
                }
            }
            for worker in workers {
                worker.join().unwrap();
            }
        });
        Self {
            base,
            stop,
            records,
            worker: Some(worker),
            root: root.to_owned(),
            cases,
        }
    }
    fn verify(&self) {
        let records = self.records.lock().unwrap();
        self.save(&records);
        for (i, case) in self.cases.iter().enumerate() {
            let location = case["parameter"]["in"].as_str().unwrap();
            let path = if location == "path" {
                format!("/base/p{i}/{}", case["wire"].as_str().unwrap())
            } else if location == "query" {
                format!("/base/p{i}?{}", case["wire"].as_str().unwrap())
            } else {
                format!("/base/p{i}")
            };
            let request = records
                .iter()
                .find(|r| r.target == path)
                .unwrap_or_else(|| panic!("missing normative wire {path}"));
            assert_eq!(request.method, "GET");
            if location == "header" {
                assert_eq!(
                    request.header(case["parameter"]["name"].as_str().unwrap()),
                    case["wire"].as_str().unwrap()
                );
            }
            if location == "cookie" {
                assert_eq!(request.header("cookie"), case["wire"].as_str().unwrap());
            }
        }
        assert!(
            records
                .iter()
                .any(|r| r.target == "/base/queryText?a%3Db%20%26%20%E9%9B%AA%252F")
        );
        assert!(
            records
                .iter()
                .any(|r| r.target == "/base/queryJson?%7B%22active%22%3Atrue%7D")
        );
        assert!(
            records
                .iter()
                .any(|r| r.target == "/base/queryForm?bar=true&foo=a+%2B+b&items=1&items=2")
        );
        assert!(records.iter().any(|r| r.target == "/v2/servers"));
        assert!(records.iter().any(|r| r.target == "/api/servers"));
        let secure = records
            .iter()
            .filter(|r| r.target.starts_with("/base/secure"))
            .collect::<Vec<_>>();
        assert_eq!(secure.len(), 5);
        assert_eq!(secure[0].header("authorization"), "");
        assert_eq!(secure[1].header("authorization"), "Bearer native-token");
        assert_eq!(secure[1].header("x-api-key"), "header-token");
        assert_eq!(secure[1].target, "/base/secure?api_key=a%2F%2B%20b");
        assert_eq!(secure[1].header("cookie"), "session=cookie%20value");
        assert_eq!(
            secure[2].header("authorization"),
            "Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ=="
        );
        assert_eq!(secure[3].header("authorization"), "Bearer oauth-token");
        assert_eq!(secure[4].header("authorization"), "Bearer oidc-token");
        let form = records.iter().find(|r| r.target == "/base/form").unwrap();
        assert_eq!(form.method, "POST");
        // OAS 3.2 encoding-by-name applies the Encoding Object to each array
        // item, including style/explode. The independent 3.1 form witness below
        // exercises its whole-array non-exploded representation separately.
        assert_eq!(form.body,b"address=%7B%22city%22%3A%22New+York%22%7D&codes=1&codes=2&id=a+%2B+b&tags=two+words&tags=a%2Bb");
        let packed = records
            .iter()
            .find(|r| r.target == "/base/packedForm")
            .unwrap();
        assert_eq!(packed.body, b"pair=x,2,y,a%2Cb%20%2B%20%E9%9B%AA");
        let styled = records
            .iter()
            .find(|r| r.target == "/base/styledUpload")
            .unwrap();
        let styled = mime_parts(&styled.body, styled.header("content-type"));
        assert_eq!(styled.len(), 4);
        assert_eq!(styled[0].2, b"x=2&y=two % words");
        assert_eq!(styled[1].2, b"deep%5Bx%5D=3&deep%5By%5D=deep value");
        assert_eq!(styled[2].2, b"labels=one");
        assert_eq!(styled[3].2, b"labels=two");
        assert!(
            styled
                .iter()
                .all(|p| !p.1.to_ascii_lowercase().contains("content-type"))
        );
        assert_eq!(
            records
                .iter()
                .filter(|r| r.target == "/base/styledUpload")
                .count(),
            1
        );
        let upload = records.iter().find(|r| r.target == "/base/upload").unwrap();
        assert_eq!(upload.method, "POST");
        let parts = mime_parts(&upload.body, upload.header("content-type"));
        assert_eq!(parts.len(), 4);
        assert_eq!(parts[0].2, [0, 255, 1, 2, 13, 10]);
        assert_eq!(parts[0].0, "file");
        assert!(parts[0].1.contains("filename=\"file.bin\""));
        assert!(parts[0].1.contains("X-Part-Id: part-1"));
        assert_eq!(parts[1].0, "labels");
        assert_eq!(parts[1].2, b"one");
        assert_eq!(parts[2].2, "雪".as_bytes());
        assert_eq!(parts[3].2, b"{\"title\":\"native\"}");
        assert_eq!(
            records
                .iter()
                .filter(|r| r.target == "/base/upload")
                .count(),
            1,
            "invalid mutable multipart input must not send"
        );
        for method in ["QUERY", "COPY", "head"] {
            assert!(
                records
                    .iter()
                    .any(|r| r.target == "/base/methods" && r.method == method)
            );
        }
        let emit = records.iter().find(|r| r.target == "/base/emit").unwrap();
        assert_eq!(
            emit.body,
            b"event: tick\nid: 7\nretry: 1000\ndata: first\ndata: second\n\n"
        );
        for mode in ["break", "cancel", "slow", "window"] {
            assert!(self.root.join(format!("sse-{mode}-closed")).is_file());
        }
        self.save(&records);
    }
    fn save(&self, records: &[Request]) {
        let _=std::fs::write(self.root.join("wire-records.json"),serde_json::to_string_pretty(&records.iter().map(|r|json!({"method":r.method,"target":r.target,"headers":r.headers,"bodyBytes":r.body})).collect::<Vec<_>>()).unwrap());
    }
}
impl Drop for ProtocolServer {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
        self.save(
            &self
                .records
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()),
        );
    }
}
fn read_request(mut stream: &std::net::TcpStream) -> Option<Request> {
    use std::io::Read;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .ok()?;
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    let end = loop {
        let n = stream.read(&mut buffer).ok()?;
        if n == 0 {
            return None;
        }
        bytes.extend_from_slice(&buffer[..n]);
        if bytes.len() > 1024 * 1024 {
            return None;
        }
        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let text = String::from_utf8(bytes[..end].to_vec()).ok()?;
    let mut lines = text.lines();
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.into();
    let target = first.next()?.into();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.into(), v.trim().into()))
        .collect::<Vec<(String, String)>>();
    let length = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    if length > 1024 * 1024 {
        return None;
    }
    while bytes.len() < end + length {
        let n = stream.read(&mut buffer).ok()?;
        if n == 0 {
            return None;
        }
        bytes.extend_from_slice(&buffer[..n]);
    }
    Some(Request {
        method,
        target,
        headers,
        body: bytes[end..end + length].to_vec(),
    })
}
fn respond(mut stream: std::net::TcpStream, status: u16, headers: &str, body: &[u8]) {
    use std::io::Write;
    let _ = write!(
        stream,
        "HTTP/1.1 {status} Fixture\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(body);
}
fn serve(mut stream: std::net::TcpStream, request: &Request, root: &Path) {
    use std::io::{Read, Write};
    let path = request.target.split('?').next().unwrap();
    if path.starts_with("/base/containers/") || path.starts_with("/base/files/") {
        respond(
            stream,
            200,
            "Content-Type: application/octet-stream\r\n",
            &[0, 255, 1, 2, 13, 10],
        );
        return;
    }
    if path == "/base/credits/coinbase" {
        let first = root.join("coinbase-first");
        if first.exists() {
            respond(
                stream,
                410,
                "Content-Type: application/json\r\n",
                b"{\"error\":{\"code\":410,\"message\":\"removed\"}}",
            );
        } else {
            std::fs::write(first, b"seen").unwrap();
            respond(stream, 200, "", b"");
        }
        return;
    }
    if path.starts_with("/base/sse/") {
        let mode = path.rsplit('/').next().unwrap();
        let _=stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n");
        let mut chunk = |bytes: &[u8]| {
            let _ = write!(stream, "{:x}\r\n", bytes.len());
            let _ = stream.write_all(bytes);
            let _ = stream.write_all(b"\r\n");
        };
        match mode {
            "normal" => {
                for bytes in [b"\xef\xbb\xbfevent: tick\r\nid: 7\r\nretry: 0005\r\ndata: {\"n\":1}\r\ndata: \xe9".as_slice(),b"\x9b\xaa\r\n\r\n: ignored\nid: bad\0id\ndata: [DONE]\n\n".as_slice()]{chunk(bytes);std::thread::sleep(std::time::Duration::from_millis(15));}
                let _ = stream.write_all(b"0\r\n\r\n");
                return;
            }
            "large" => {
                chunk(format!("data: {}\n\n", "x".repeat(512)).as_bytes());
                let _ = stream.write_all(b"0\r\n\r\n");
                return;
            }
            "slow" => {}
            "window" => chunk("data: x\n\n".repeat(10000).as_bytes()),
            _ => chunk(b"data: one\n\n"),
        }
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        let closed = match stream.read(&mut [0u8; 1]) {
            Ok(0) => true,
            Err(error) => error.kind() == std::io::ErrorKind::ConnectionReset,
            _ => false,
        };
        if closed {
            std::fs::write(root.join(format!("sse-{mode}-closed")), b"closed").unwrap();
        }
        return;
    }
    if path.starts_with("/base/lines/") {
        let body = if path.ends_with("bad") {
            b"[DONE]\n".as_slice()
        } else {
            b"{\"n\":1.0}\n{\"n\":9007199254740993}\r\n{\"n\":1e100000000000000000000}".as_slice()
        };
        respond(stream, 200, "Content-Type: application/x-ndjson\r\n", body);
        return;
    }
    match path {
        "/base/schemaFree" => {
            respond(
                stream,
                200,
                "Content-Type: application/json\r\n",
                b"{\"large\":1e1000000000000000000,\"null\":null}",
            );
            return;
        }
        "/base/textNumber" => {
            respond(
                stream,
                200,
                "Content-Type: text/plain\r\n",
                b"9007199254740993.00000000000001",
            );
            return;
        }
        "/base/media" => {
            let media = request.header("content-type");
            if media.starts_with("application/json") {
                let value: Value = serde_json::from_slice(&request.body).unwrap();
                let header = match value["message"].as_str() {
                    Some("problem") => "application/problem+json",
                    Some("profile") => "Application/JSON;profile=v2; charset=UTF-8",
                    _ => "application/json",
                };
                respond(
                    stream,
                    200,
                    &format!("Content-Type: {header}\r\n"),
                    &request.body,
                );
            } else if media == "text/plain" {
                respond(
                    stream,
                    200,
                    "Content-Type: text/plain; charset=UTF-8\r\n",
                    &request.body,
                );
            } else {
                respond(
                    stream,
                    200,
                    if media == "image/png" {
                        "Content-Type: image/png\r\n"
                    } else {
                        "Content-Type: application/pdf\r\n"
                    },
                    &request.body,
                );
            }
            return;
        }
        "/base/precedence/exact-wrong-media" => {
            respond(stream, 200, "Content-Type: text/plain\r\n", b"not json");
            return;
        }
        "/base/precedence/range" => {
            respond(stream, 207, "Content-Type: text/plain\r\n", b"range");
            return;
        }
        "/base/precedence/range-empty" | "/base/empty" | "/base/emit" => {
            respond(stream, 204, "", b"");
            return;
        }
        "/base/precedence/default-error" => {
            respond(stream, 404, "", &[0, 255, 4]);
            return;
        }
        "/base/default" => {
            respond(stream, 201, "", b"default success");
            return;
        }
        "/base/head" => {
            let _=stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 999999999999\r\nX-Count: 99999999999999999999\r\nConnection: close\r\n\r\n");
            return;
        }
        "/base/headers" => {
            respond(
                stream,
                200,
                "Content-Type: application/json\r\nX-Rate: 1.0\r\nX-Flags: one,two\r\nX-Meta: {\"active\":true}\r\n",
                b"{\"message\":\"headers\"}",
            );
            return;
        }
        "/base/form" | "/base/packedForm" => {
            respond(
                stream,
                200,
                "Content-Type: application/x-www-form-urlencoded\r\n",
                &request.body,
            );
            return;
        }
        "/base/upload" | "/base/styledUpload" => {
            respond(
                stream,
                200,
                &format!("Content-Type: {}\r\n", request.header("content-type")),
                &request.body,
            );
            return;
        }
        "/base/methods" if request.method == "COPY" => {
            respond(stream, 204, "", b"");
            return;
        }
        "/base/methods" if request.method == "head" => {
            respond(
                stream,
                200,
                "Content-Type: text/plain\r\n",
                b"lowercase head has a body",
            );
            return;
        }
        _ => {}
    }
    respond(stream, 200, "Content-Type: text/plain\r\n", b"ok");
}
fn mime_parts(body: &[u8], media: &str) -> Vec<(String, String, Vec<u8>)> {
    let boundary = media.split("boundary=").nth(1).unwrap().trim_matches('"');
    let marker = format!("--{boundary}").into_bytes();
    let mut result = Vec::new();
    let mut offset = 0;
    while let Some(at) = body[offset..]
        .windows(marker.len())
        .position(|w| w == marker)
    {
        let begin = offset + at + marker.len();
        if body[begin..].starts_with(b"--") {
            break;
        }
        let header_start = begin + 2;
        let head_end = header_start
            + body[header_start..]
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .unwrap();
        let headers = String::from_utf8(body[header_start..head_end].to_vec()).unwrap();
        let data_start = head_end + 4;
        let next = data_start
            + body[data_start..]
                .windows(marker.len())
                .position(|w| w == marker)
                .unwrap();
        let name = headers
            .split("name=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap()
            .to_owned();
        result.push((name, headers, body[data_start..next - 2].to_vec()));
        offset = next;
    }
    result
}
