//! Maintained source-driven public scoped-validation gates. Candidate evidence
//! was recorded before enabling public admission; no frozen target fixture is read.
use super::*;
use serde_json::{Value, json};
use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
    process::Command,
};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn root(label: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-cpp-v2-gates");
    std::fs::create_dir_all(&dir).unwrap();
    tempfile::Builder::new()
        .prefix(label)
        .tempdir_in(dir)
        .unwrap()
        .keep()
        .canonicalize()
        .unwrap()
}
fn contract(root: &Path, document: Value) -> Arc<Contract> {
    let path = root.join("api.json");
    std::fs::write(&path, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(root).build().unwrap());
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap())
}
fn cxx() -> PathBuf {
    std::env::var_os("SUSPECT_CPP_CXX")
        .map(PathBuf::from)
        .unwrap_or_else(|| "clang++".into())
}
fn checked(command: &mut Command, root: &Path) -> std::process::Output {
    use std::io::Write;
    let output = command.output().unwrap();
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("commands.log"))
        .unwrap();
    writeln!(
        log,
        "{command:?}\n{}\n{}{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}\n{command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
fn config(value: &Value) -> Config {
    let mut result = Config::default();
    if let Some(v) = value["maxNumberBytes"].as_u64() {
        result.max_number_bytes = v as usize;
    }
    if let Some(v) = value["maxEvaluationSteps"].as_u64() {
        result.max_evaluation_steps = v as usize;
    }
    if let Some(v) = value["maxEqualitySteps"].as_u64() {
        result.max_equality_steps = v as usize;
    }
    if let Some(v) = value["maxDepth"].as_u64() {
        result.max_depth = v as usize;
    }
    if let Some(v) = value["maxErrors"].as_u64() {
        result.max_errors = v as usize;
    }
    result
}
fn expected_path(id: &str) -> &'static str {
    match id {
        "contains-zero-does-not-mark-unmatched" | "contains-exact-integrality" => "/0",
        "contains-failure-after-exceeded-maximum" => "/1",
        "pattern-overlap-rejects" | "named-and-pattern-both-apply" => "/x",
        "property-names-checks-key-not-value" => "/long",
        "property-names-does-not-annotate-values" => "/ok",
        "failed-anyof-branch-does-not-leak"
        | "allof-cousins-have-independent-scopes"
        | "not-discards-annotations"
        | "required-is-not-an-evaluation" => "/a",
        "nested-members-do-not-mark-parent" => "/inner",
        "prefix-and-contains-leave-unmatched-item" => "/2",
        _ => "",
    }
}

#[test]
#[ignore = "C++20 source-driven independent scoped-applicator runtime gate"]
fn native_v2_independent_32() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../suspect-schema/tests/fixtures/owned-applicators-v2.json"
    ))
    .unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 32);
    let root = root("vectors-");
    let mut programs = String::new();
    let mut declarations = String::new();
    let mut calls = String::new();
    let mut snapshots = Vec::new();
    for (i, case) in cases.iter().enumerate() {
        let directory = root.join(format!("case-{i}"));
        std::fs::create_dir(&directory).unwrap();
        let schema: Value = serde_json::from_str(case["schemaJson"].as_str().unwrap()).unwrap();
        let contract = contract(
            &directory,
            json!({"openapi":"3.1.0","info":{"title":"Independent v2 witness","version":"1"},"components":{"schemas":{"Root":schema}}}),
        );
        let source = SourceId::new(contract.entry().clone(), Default::default())
            .child("components")
            .child("schemas")
            .child("Root");
        let program = suspect_schema::OwnedCompiler::new(config(&case["limits"]))
            .compile_v2(contract.clone(), std::slice::from_ref(&source))
            .unwrap()
            .program();
        program.check().unwrap();
        assert_eq!(program.version, OwnedProgram::V2_VERSION);
        assert!(
            check_program_profile(&contract, &program, false)
                .unwrap_err()
                .iter()
                .any(|e| e.code == "cpp-validation-opcode-unsupported")
        );
        let name = if i == 0 {
            "validation_program".into()
        } else {
            format!("vector_{i}")
        };
        if i == 0 {
            crate::write_files(
                &emit_validation_runtime(&contract, &program).unwrap(),
                &root,
            )
            .unwrap();
        } else {
            programs.push_str(&emit::validation_program_named(
                &contract,
                &program,
                &SdkConfig::default(),
                &name,
            ));
        }
        writeln!(declarations, "const Program& {name}();").unwrap();
        let id = case["id"].as_str().unwrap();
        let expected = case["expected"].as_str().unwrap();
        let src = case["source"].as_str().unwrap_or("");
        let span = if src.is_empty() {
            0..0
        } else {
            contract
                .source_span(&locate(
                    &contract,
                    &suspect_schema::ProgramSource {
                        document: contract.entry().to_string(),
                        pointer: src.into(),
                    },
                ))
                .unwrap()
        };
        writeln!(
            calls,
            "run({}, detail::{name}(), {}, {}, {}, {}, {}, {}, {});",
            emit::string(id),
            program.roots[0].target,
            emit::string(case["instanceJson"].as_str().unwrap()),
            emit::string(expected),
            emit::string(src),
            emit::string(expected_path(id)),
            span.start,
            span.end
        )
        .unwrap();
        snapshots.push(json!({"id":id,"program":program,"instance":case["instanceJson"],"expected":expected,"source":src,"instancePath":expected_path(id)}));
    }
    std::fs::write(root.join("cpp/src/vectors.cpp"), programs).unwrap();
    let driver = format!(
        r#"#include <generated_sdk/runtime.hpp>
#include <cstdlib>
#include <iostream>
using namespace generated_sdk;
namespace generated_sdk::detail {{ {declarations} }}
void run(const std::string& id,const detail::Program& program,std::size_t root,const std::string& text,const std::string& expected,const std::string& source,const std::string& path,std::size_t begin,std::size_t end) {{
auto parsed=parse_json(text);if(!parsed)std::abort();
try{{detail::Validation validation(program);validation.require(root,parsed.value(),"");if(expected!="Valid"){{std::cerr<<id<<" expected "<<expected;std::abort();}}}}
catch(detail::Failure& failure){{const auto& e=failure.error;const auto actual=e.kind==CodecError::Kind::Validation?"Invalid":e.kind==CodecError::Kind::EvaluationFailure?"EvaluationFailure":"Other";
if(expected!=actual||e.source.pointer!=source||e.instance_path!=path||e.source.begin!=begin||e.source.end!=end){{std::cerr<<id<<" expected="<<expected<<" actual="<<actual<<" source="<<e.source.pointer<<" path="<<e.instance_path<<" span="<<e.source.begin<<":"<<e.source.end<<" "<<e.message;std::abort();}}}}
std::cout<<id<<": "<<expected<<"\n";
}}
int main(){{{calls}}}
"#
    );
    std::fs::write(root.join("driver.cpp"), driver).unwrap();
    std::fs::write(
        root.join("programs.json"),
        serde_json::to_string_pretty(&snapshots).unwrap(),
    )
    .unwrap();
    checked(
        Command::new(cxx())
            .args([
                "-std=c++20",
                "-O0",
                "-g",
                "-Wall",
                "-Wextra",
                "-Wpedantic",
                "-Werror",
                "-I",
            ])
            .arg(root.join("cpp/include"))
            .arg(root.join("driver.cpp"))
            .args(
                [
                    "runtime.cpp",
                    "validation.cpp",
                    "validation_v2.cpp",
                    "validation_v3.cpp",
                    "program.cpp",
                    "vectors.cpp",
                ]
                .map(|f| root.join("cpp/src").join(f)),
            )
            .arg("-o")
            .arg(root.join("vectors")),
        &root,
    );
    checked(&mut Command::new(root.join("vectors")), &root);
    println!("C++ independent 32 v2 vectors: {}", root.display());
}

fn cmake() -> PathBuf {
    std::env::var_os("SUSPECT_CPP_CMAKE")
        .map(PathBuf::from)
        .unwrap_or_else(|| "cmake".into())
}
pub(super) fn package(plan: &SdkPlan, root: &Path) {
    package_files(plan.render().unwrap(), root);
}
pub(super) fn package_files(files: Vec<crate::OutFile>, root: &Path) {
    crate::write_files(&files, &root.join("generated")).unwrap();
    checked(
        Command::new(cmake())
            .arg("-S")
            .arg(root.join("generated/cpp"))
            .arg("-B")
            .arg(root.join("build"))
            .arg(format!("-DCMAKE_CXX_COMPILER={}", cxx().display()))
            .arg("-DCMAKE_BUILD_TYPE=Debug")
            .arg(format!(
                "-DCMAKE_INSTALL_PREFIX={}",
                root.join("install").display()
            ))
            .arg("-DSUSPECT_SDK_BUILD_DOCS=ON")
            .arg(format!(
                "-DDOXYGEN_EXECUTABLE={}",
                std::env::var("SUSPECT_CPP_DOXYGEN").unwrap_or_else(|_| "doxygen".into())
            )),
        root,
    );
    checked(
        Command::new(cmake())
            .arg("--build")
            .arg(root.join("build"))
            .args(["--parallel", "2"]),
        root,
    );
    checked(
        Command::new(cmake().with_file_name("ctest"))
            .arg("--test-dir")
            .arg(root.join("build"))
            .arg("--output-on-failure"),
        root,
    );
    checked(
        Command::new(cmake())
            .arg("--build")
            .arg(root.join("build"))
            .args(["--target", "sdk_docs"]),
        root,
    );
    checked(
        Command::new(cmake())
            .arg("--install")
            .arg(root.join("build")),
        root,
    );
}
#[test]
#[ignore = "installed CMake/Doxygen scoped models, codecs, mutable validation and real libcurl operations"]
fn native_v2_sdk_operations() {
    use std::io::{Read, Write};
    let root = root("sdk-");
    let document: Value = serde_json::from_str(include_str!("tests/scoped.openapi.json")).unwrap();
    let contract = contract(&root, document);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let plan =
        plan_sdk(contract, &selected, Default::default()).unwrap_or_else(|e| panic!("{e:#?}"));
    assert_eq!(plan.program.version, OwnedProgram::V2_VERSION);
    assert!(plan.models().symbols().any(|s| matches!(
        s.shape,
        Shape::Object {
            extras: Extras::Patterned,
            ..
        }
    )));
    assert!(
        plan.models()
            .symbols()
            .any(|s| matches!(s.shape, Shape::ValidatedJson))
    );
    assert!(plan.examples().operations().iter().any(|op| {
        op.entries
            .iter()
            .any(|e| e.role == examples::ExampleRole::RequestBody)
    }));
    package(&plan, &root);
    assert!(root.join("generated/cpp/examples/client.cpp").is_file());
    let consumer = root.join("consumer");
    std::fs::create_dir(&consumer).unwrap();
    std::fs::write(
        consumer.join("main.cpp"),
        include_str!("tests/native_scoped.cpp"),
    )
    .unwrap();
    std::fs::write(consumer.join("CMakeLists.txt"),"cmake_minimum_required(VERSION 3.24)\nproject(ScopedConsumer LANGUAGES CXX)\nfind_package(generated_sdk CONFIG REQUIRED)\nadd_executable(consumer main.cpp)\ntarget_link_libraries(consumer PRIVATE generated_sdk::generated_sdk)\ntarget_compile_options(consumer PRIVATE -Wall -Wextra -Wpedantic -Werror)\n").unwrap();
    checked(
        Command::new(cmake())
            .arg("-S")
            .arg(&consumer)
            .arg("-B")
            .arg(root.join("build-consumer"))
            .arg(format!(
                "-DCMAKE_PREFIX_PATH={}",
                root.join("install").display()
            ))
            .arg(format!("-DCMAKE_CXX_COMPILER={}", cxx().display())),
        &root,
    );
    checked(
        Command::new(cmake())
            .arg("--build")
            .arg(root.join("build-consumer")),
        &root,
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}/api", listener.local_addr().unwrap());
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stopping = stop.clone();
    let log = root.join("wire.json");
    let server = std::thread::spawn(move || {
        let mut requests = Vec::new();
        while !stopping.load(std::sync::atomic::Ordering::SeqCst) {
            let mut stream = match listener.accept() {
                Ok((s, _)) => s,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                    continue;
                }
                Err(e) => panic!("{e}"),
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(4)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0u8; 4096];
            let end = loop {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(at) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                    break at + 4;
                }
            };
            let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
            let length = headers
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .map(|v| v.parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            while bytes.len() < end + length {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
            }
            let first = headers.lines().next().unwrap().to_owned();
            let request = &bytes[end..end + length];
            requests
                .push(json!({"request":first,"body":String::from_utf8(request.to_vec()).unwrap()}));
            let (media, body) = if first.contains("/items ") {
                (
                    "application/x-ndjson",
                    b"{\"count\":1,\"s-label\":\"yes\"}\n{\"count\":1,\"other\":\"bad\"}\n"
                        .to_vec(),
                )
            } else if first.contains("/bad-response ") {
                (
                    "application/json",
                    b"{\"base\":3,\"mode\":\"num\",\"x-positive\":0}".to_vec(),
                )
            } else {
                ("application/json", request.to_vec())
            };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {media}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(&body);
        }
        std::fs::write(log, serde_json::to_string_pretty(&requests).unwrap()).unwrap();
        requests
    });
    let output = Command::new(root.join("build-consumer/consumer"))
        .arg(base)
        .output()
        .unwrap();
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let requests = server.join().unwrap();
    std::fs::write(
        root.join("native.log"),
        [output.stdout.clone(), output.stderr.clone()].concat(),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}: {}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(requests.len(), 7);
    assert_eq!(
        requests[0]["body"],
        "{\"base\":3,\"mode\":\"num\",\"x-positive\":7}"
    );
    assert_eq!(
        requests[2]["body"],
        "{\"count\":1,\"other\":2,\"s-label\":\"yes\"}"
    );
    for (i, code) in [
        "Record value;",
        "Record value(JsonInteger(3),RecordMode::Num);value.base=\"bad\";",
        "Ledger value(JsonInteger(1));value.count=1.5;",
        "Node value;value.next=std::string(\"not an owning node\");",
    ]
    .iter()
    .enumerate()
    {
        let path = root.join(format!("negative-{i}.cpp"));
        std::fs::write(&path,format!("#include <generated_sdk/sdk.hpp>\nusing namespace generated_sdk;int main(){{{code}(void)value;}}")).unwrap();
        let output = Command::new(cxx())
            .args(["-std=c++20", "-fsyntax-only", "-I"])
            .arg(root.join("install/include"))
            .arg(path)
            .output()
            .unwrap();
        std::fs::write(root.join(format!("negative-{i}.log")), &output.stderr).unwrap();
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("file not found"));
    }
    println!(
        "C++ scoped SDK installed/native/7-wire gate: {}",
        root.display()
    );
}

#[test]
#[ignore = "C++ scoped visit boundaries, lexical keys, identity, failure propagation and 2MiB-stack RAII"]
fn native_v2_scope_resource_edges() {
    let cases = [
        (
            "eight-visits",
            r#"{"properties":{"a":true},"unevaluatedProperties":false}"#,
            r#"{"a":1}"#,
            8,
            "Valid",
            "",
            "",
        ),
        (
            "seventh-visit",
            r#"{"properties":{"a":true},"unevaluatedProperties":false}"#,
            r#"{"a":1}"#,
            7,
            "EvaluationFailure",
            "/unevaluatedProperties",
            "",
        ),
        (
            "allof-duplicate-merge",
            r#"{"allOf":[{"properties":{"a":true}},{"properties":{"a":true}}],"unevaluatedProperties":false}"#,
            r#"{"a":1}"#,
            21,
            "Valid",
            "",
            "",
        ),
        (
            "allof-merge-exhaustion",
            r#"{"allOf":[{"properties":{"a":true}},{"properties":{"a":true}}],"unevaluatedProperties":false}"#,
            r#"{"a":1}"#,
            20,
            "EvaluationFailure",
            "/unevaluatedProperties",
            "",
        ),
        (
            "oneof-no-union-cost",
            r#"{"oneOf":[{"properties":{"a":true}},{"properties":{"a":true}}],"unevaluatedProperties":false}"#,
            r#"{"a":1}"#,
            20,
            "Invalid",
            "/oneOf",
            "",
        ),
        (
            "key-identity",
            r##"{"propertyNames":{"$ref":"#/components/schemas/Root"}}"##,
            r#"{"a/b~":1,"x":null}"#,
            1000,
            "Valid",
            "",
            "",
        ),
        (
            "decoded-key-path",
            r#"{"propertyNames":{"maxLength":1}}"#,
            r#"{"a/b~":1}"#,
            1000,
            "Invalid",
            "/propertyNames/maxLength",
            "/a~1b~0",
        ),
        (
            "no-normalization",
            r#"{"propertyNames":{"pattern":"^é$"}}"#,
            r#"{"e\u0301":1}"#,
            1000,
            "Invalid",
            "/propertyNames/pattern",
            "/é",
        ),
        (
            "nonproductive-condition",
            r##"{"if":{"$ref":"#/components/schemas/Root"},"then":true}"##,
            "1",
            1000,
            "EvaluationFailure",
            "",
            "",
        ),
        (
            "symbolic-contains-bound",
            r#"{"contains":true,"minContains":1e100000000000000000000}"#,
            "[1]",
            1000,
            "Invalid",
            "/minContains",
            "",
        ),
        (
            "cache-and-control",
            r#"{"if":{"type":"integer"},"then":true,"else":false}"#,
            "1",
            1000,
            "Valid",
            "",
            "",
        ),
        (
            "depth",
            r##"{"type":"object","properties":{"next":{"$ref":"#/components/schemas/Root"}},"unevaluatedProperties":false}"##,
            "{}",
            100000,
            "Valid",
            "",
            "",
        ),
    ];
    let root = root("edges-");
    let mut programs = String::new();
    let mut decls = String::new();
    let mut calls = String::new();
    let mut root_indices = Vec::new();
    for (i, (id, schema, instance, steps, outcome, source, path)) in cases.iter().enumerate() {
        let dir = root.join(id);
        std::fs::create_dir(&dir).unwrap();
        let contract = contract(
            &dir,
            json!({"openapi":"3.1.0","info":{"title":"Scoped edge","version":"1"},"components":{"schemas":{"Root":serde_json::from_str::<Value>(schema).unwrap()}}}),
        );
        let source_id = SourceId::new(contract.entry().clone(), Default::default())
            .child("components")
            .child("schemas")
            .child("Root");
        let program = suspect_schema::OwnedCompiler::new(Config {
            max_evaluation_steps: *steps,
            max_depth: 512,
            ..Default::default()
        })
        .compile_v2(contract.clone(), std::slice::from_ref(&source_id))
        .unwrap()
        .program();
        root_indices.push(program.roots[0].target);
        program.check().unwrap();
        let name = if i == 0 {
            "validation_program".into()
        } else {
            format!("edge_{i}")
        };
        if i == 0 {
            crate::write_files(&emit::validation_runtime(&contract, &program), &root).unwrap();
        } else {
            programs.push_str(&emit::validation_program_named(
                &contract,
                &program,
                &Default::default(),
                &name,
            ));
        }
        writeln!(decls, "const Program& {name}();").unwrap();
        writeln!(
            calls,
            "run({},detail::{name}(),{}, {}, {}, {}, {});",
            emit::string(id),
            program.roots[0].target,
            emit::string(instance),
            emit::string(outcome),
            emit::string(&format!("/components/schemas/Root{source}")),
            emit::string(path)
        )
        .unwrap();
    }
    std::fs::write(root.join("cpp/src/edges.cpp"), programs).unwrap();
    let source = format!(
        r#"#include <generated_sdk/runtime.hpp>
#include <iostream>
#include <cstdlib>
#include <pthread.h>
using namespace generated_sdk;
namespace generated_sdk::detail {{{decls}}}
#define CHECK(x) do{{if(!(x)){{std::cerr<<__LINE__<<" "<<#x;std::abort();}}}}while(false)
void run(const std::string& id,const detail::Program& p,std::size_t root,const std::string& input,const std::string& expected,const std::string& source,const std::string& path){{auto value=parse_json(input);CHECK(value);
try{{detail::Validation v(p);v.require(root,value.value(),"");CHECK(expected=="Valid");}}
catch(detail::Failure& f){{const auto& e=f.error;auto kind=e.kind==CodecError::Kind::Validation?"Invalid":e.kind==CodecError::Kind::EvaluationFailure?"EvaluationFailure":"Other";if(expected!=kind||e.source.pointer!=source||e.instance_path!=path){{std::cerr<<id<<" "<<kind<<" "<<e.source.pointer<<" "<<e.instance_path;std::abort();}}}}}}
void* depth(void*){{JsonValue value(JsonValue::Object{{}});for(int i=0;i<300;++i){{JsonValue::Object next;next.emplace("next",std::move(value));value=JsonValue(std::move(next));}}
try{{detail::Validation v(detail::edge_11());v.require({depth_root},value,"");CHECK(false);}}catch(detail::Failure& f){{CHECK(f.error.kind==CodecError::Kind::EvaluationFailure);}}return nullptr;}}
int main(){{{calls}
auto p=detail::edge_10();p.max_number_bytes=3;detail::Validation v(p);JsonValue value(JsonInteger(1));v.require({cache_root},value,"");value=JsonValue(JsonInteger(12345));
try{{v.require({cache_root},value,"");CHECK(false);}}catch(detail::Failure& f){{CHECK(f.error.kind==CodecError::Kind::EvaluationFailure);}}
value=JsonValue(JsonInteger(2));v.require({cache_root},value,"");
Cancellation cancel;cancel.cancel();try{{detail::Validation stopped(p,detail::Control{{cancel.token(),std::nullopt}});stopped.require({cache_root},value,"");CHECK(false);}}catch(detail::Failure& f){{CHECK(f.error.kind==CodecError::Kind::Cancelled);}}
try{{detail::Validation expired(p,detail::Control{{{{}},std::chrono::steady_clock::now()-std::chrono::seconds(1)}});expired.require({cache_root},value,"");CHECK(false);}}catch(detail::Failure& f){{CHECK(f.error.kind==CodecError::Kind::Timeout);}}
for(int bad=0;bad<2;++bad){{auto corrupt=p;if(bad==0)corrupt.profile="unknown";else{{corrupt.version="suspect.validation.experimental.v1";corrupt.profile="oas31-jsonschema202012-static-subset";}}
try{{detail::Validation invalid(corrupt);CHECK(false);}}catch(detail::Failure& f){{CHECK(f.error.kind==CodecError::Kind::EvaluationFailure);}}}}
pthread_attr_t attrs;CHECK(pthread_attr_init(&attrs)==0);CHECK(pthread_attr_setstacksize(&attrs,2*1024*1024)==0);pthread_t thread;CHECK(pthread_create(&thread,&attrs,depth,nullptr)==0);pthread_attr_destroy(&attrs);CHECK(pthread_join(thread,nullptr)==0);
std::cout<<"v2 exact visit/set-merge, key identity, symbolic counts, cache mutation, interruption and normal-stack depth gates passed\n";
}}
"#,
        depth_root = root_indices[11],
        cache_root = root_indices[10]
    );
    std::fs::write(root.join("driver.cpp"), source).unwrap();
    checked(
        Command::new(cxx())
            .args([
                "-std=c++20",
                "-O0",
                "-g",
                "-Wall",
                "-Wextra",
                "-Wpedantic",
                "-Werror",
                "-I",
            ])
            .arg(root.join("cpp/include"))
            .arg(root.join("driver.cpp"))
            .args(
                [
                    "runtime.cpp",
                    "validation.cpp",
                    "validation_v2.cpp",
                    "validation_v3.cpp",
                    "program.cpp",
                    "edges.cpp",
                ]
                .map(|f| root.join("cpp/src").join(f)),
            )
            .arg("-o")
            .arg(root.join("edges")),
        &root,
    );
    checked(&mut Command::new(root.join("edges")), &root);
    println!("C++ v2 resource/scope edges: {}", root.display());
}

#[test]
fn scoped_program_envelopes_and_operands_are_checked_before_emission() {
    let root = root("guards-");
    let contract = contract(
        &root,
        json!({"openapi":"3.1.0","info":{"title":"Program guard","version":"1"},"components":{"schemas":{"Root":{
            "if":true,"then":true,"else":false,"dependentRequired":{"a":["b"]},"dependentSchemas":{"a":true},"contains":true,"minContains":1,"maxContains":2,
            "patternProperties":{"^x":true},"additionalProperties":false,"propertyNames":true,"unevaluatedProperties":false,"unevaluatedItems":false
        }}}}),
    );
    let source = SourceId::new(contract.entry().clone(), Default::default())
        .child("components")
        .child("schemas")
        .child("Root");
    let program = suspect_schema::OwnedCompiler::new(Default::default())
        .compile_v2(contract.clone(), &[source])
        .unwrap()
        .program();
    check_program_profile(&contract, &program, true).unwrap();
    for kind in 0..7 {
        let mut bad = program.clone();
        match kind {
            0 => bad.version = "unknown",
            1 => bad.profile = OwnedProgram::V1_PROFILE,
            2 => {
                bad.version = OwnedProgram::V1_VERSION;
                bad.profile = OwnedProgram::V1_PROFILE;
            }
            3 => {
                for check in bad.nodes.iter_mut().flat_map(|n| &mut n.checks) {
                    if let ProgramInstruction::If { condition, .. } = &mut check.instruction {
                        *condition = usize::MAX;
                        break;
                    }
                }
            }
            4 => {
                for check in bad.nodes.iter_mut().flat_map(|n| &mut n.checks) {
                    if let ProgramInstruction::Contains { minimum, .. } = &mut check.instruction {
                        *minimum = Some("1e-400".into());
                        break;
                    }
                }
            }
            5 => {
                for check in bad.nodes.iter_mut().flat_map(|n| &mut n.checks) {
                    if let ProgramInstruction::PatternProperties { patterns } =
                        &mut check.instruction
                    {
                        patterns[0].1.start = usize::MAX;
                        break;
                    }
                }
            }
            _ => {
                for check in bad.nodes.iter_mut().flat_map(|n| &mut n.checks) {
                    if let ProgramInstruction::AdditionalPropertiesWithPatterns {
                        declared,
                        target,
                    } = &check.instruction
                    {
                        check.instruction = ProgramInstruction::AdditionalProperties {
                            declared: declared.clone(),
                            target: *target,
                        };
                        break;
                    }
                }
            }
        }
        assert!(bad.check().is_err());
        let errors = check_program_profile(&contract, &bad, true).unwrap_err();
        assert!(errors.iter().all(|e| e.code == "cpp-validation-program"
            || e.code == "cpp-validation-profile-unsupported"));
    }
}

#[test]
fn public_scoped_admission_preserves_base_programs_and_candidate_bytes() {
    let root = root("public-");
    let contract = contract(
        &root,
        serde_json::from_str(include_str!("tests/scoped.openapi.json")).unwrap(),
    );
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let candidate =
        protocol::plan_profile(contract.clone(), &selected, Default::default(), true).unwrap();
    let public = plan_sdk(contract, &selected, Default::default()).unwrap();
    assert_eq!(public.render().unwrap(), emit::package(&candidate).unwrap());
    let simple = root.join("base");
    std::fs::create_dir(&simple).unwrap();
    let schema = json!({"type":"object","required":["value"],"properties":{"value":{"type":"integer"}},"additionalProperties":false});
    let contract = self::contract(
        &simple,
        json!({"openapi":"3.1.0","info":{"title":"Frozen ordinary closure","version":"1"},"servers":[{"url":"https://example.test"}],"paths":{"/x":{"get":{"operationId":"x","responses":{"200":{"description":"Value","content":{"application/json":{"schema":schema}}}}}}}}),
    );
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let base =
        protocol::plan_profile(contract.clone(), &selected, Default::default(), false).unwrap();
    let public = plan_sdk(contract, &selected, Default::default()).unwrap();
    assert_eq!(base.program, public.program);
    assert_eq!(public.program.version, OwnedProgram::V1_VERSION);
    assert_eq!(base.render().unwrap(), public.render().unwrap());
}
