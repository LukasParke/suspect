//! Resource/dynamic candidate gates over real compile_v3 source inputs.
use super::*;
use serde_json::{Value, json};
use std::{
    fmt::Write as _,
    io::Write as _,
    path::{Path, PathBuf},
    process::Command,
};
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

const ENTRY: &str = "https://physical.test/api.json";
const SUBJECT: &str = "https://physical.test/official-schema.json";
fn source(uri: &str, pointer: &str) -> SourceId {
    let mut id = SourceId::new(Uri::parse(uri).unwrap(), Default::default());
    for token in pointer.split('/').skip(1) {
        id = id.child(&token.replace("~1", "/").replace("~0", "~"));
    }
    id
}
fn load(document: Value, extras: Vec<(&str, Value)>) -> Arc<Contract> {
    let mut files = vec![(ENTRY, document)];
    files.extend(extras);
    let provider = Arc::new(
        DocumentProvider::new(files.into_iter().map(|(uri, value)| {
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
    Arc::new(Contract::from_workspace(&workspace, &Uri::parse(ENTRY).unwrap()).unwrap())
}
fn root(label: &str) -> PathBuf {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-cpp-v3-gates");
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix(label)
        .tempdir_in(base)
        .unwrap()
        .keep()
        .canonicalize()
        .unwrap()
}
fn cxx() -> PathBuf {
    std::env::var_os("SUSPECT_CPP_CXX")
        .map(PathBuf::from)
        .unwrap_or_else(|| "clang++".into())
}
fn checked(command: &mut Command, root: &Path) -> std::process::Output {
    let output = command.output().unwrap();
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(root.join("commands.log"))
        .unwrap();
    writeln!(
        file,
        "{command:?}\n{}\n{}{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}: {}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
fn compile_native(root: &Path, extra: &str) {
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
                    extra,
                ]
                .map(|f| root.join("cpp/src").join(f)),
            )
            .arg("-o")
            .arg(root.join("consumer")),
        root,
    );
}
#[test]
#[ignore = "C++20 44 unmodified official dynamicRef cases compiled through compile_v3 and a closed provider"]
fn native_v3_official_dynamic_ref_44() {
    let groups: Vec<Value> = serde_json::from_str(include_str!(
        "../../../suspect-schema/tests/fixtures/resource-conformance/dynamicRef.json"
    ))
    .unwrap();
    let remotes = [
        (
            "http://localhost:1234/draft2020-12/tree.json",
            include_str!("../../../suspect-schema/tests/fixtures/resource-conformance/tree.json"),
        ),
        (
            "http://localhost:1234/draft2020-12/extendible-dynamic-ref.json",
            include_str!(
                "../../../suspect-schema/tests/fixtures/resource-conformance/extendible-dynamic-ref.json"
            ),
        ),
        (
            "http://localhost:1234/draft2020-12/detached-dynamicref.json",
            include_str!(
                "../../../suspect-schema/tests/fixtures/resource-conformance/detached-dynamicref.json"
            ),
        ),
    ];
    let root = root("official-");
    let mut programs = String::new();
    let mut declarations = String::new();
    let mut calls = String::new();
    let mut expectations = Vec::new();
    let mut count = 0;
    for (index, group) in groups.iter().enumerate() {
        let mut extras = vec![(SUBJECT, group["schema"].clone())];
        extras.extend(
            remotes
                .iter()
                .map(|(uri, raw)| (*uri, serde_json::from_str(raw).unwrap())),
        );
        let contract = load(
            json!({"openapi":"3.2.0","info":{"title":"Official native resource witness","version":"1"},"components":{"schemas":{"Use":{"$ref":SUBJECT}}}}),
            extras,
        );
        let selected = source(SUBJECT, "");
        let program = suspect_schema::OwnedCompiler::new(Default::default())
            .compile_v3(contract.clone(), std::slice::from_ref(&selected))
            .unwrap_or_else(|e| panic!("{}: {e:#?}", group["description"]))
            .program();
        program.check().unwrap();
        assert_eq!(program.version, OwnedProgram::V3_VERSION);
        check_program_resources(&contract, &program, true, true).unwrap();
        assert!(
            check_program_resources(&contract, &program, true, false).is_err(),
            "v1/v2-only profile remains fenced"
        );
        let name = if index == 0 {
            "validation_program".into()
        } else {
            format!("official_{index}")
        };
        if index == 0 {
            crate::write_files(
                &emit_validation_runtime(&contract, &program).unwrap(),
                &root,
            )
            .unwrap();
        } else {
            programs.push_str(&emit::validation_program_named(
                &contract,
                &program,
                &Default::default(),
                &name,
            ));
        }
        writeln!(declarations, "const Program& {name}();").unwrap();
        for test in group["tests"].as_array().unwrap() {
            let id = format!(
                "{} / {}",
                group["description"].as_str().unwrap(),
                test["description"].as_str().unwrap()
            );
            writeln!(
                calls,
                "run({count}, {}, detail::{name}(), {}, {}, {});",
                emit::string(&id),
                program.roots[0].target,
                emit::string(&test["data"].to_string()),
                test["valid"].as_bool().unwrap()
            )
            .unwrap();
            expectations.push((contract.clone(), test["valid"].as_bool().unwrap()));
            count += 1;
        }
        std::fs::write(
            root.join(format!("program-{index}.json")),
            serde_json::to_string_pretty(&program).unwrap(),
        )
        .unwrap();
    }
    assert_eq!(count, 44);
    std::fs::write(root.join("cpp/src/official.cpp"), programs).unwrap();
    let driver = format!(
        r#"#include <generated_sdk/runtime.hpp>
#include <iostream>
#include <cstdlib>
using namespace generated_sdk;
namespace generated_sdk::detail {{{declarations}}}
void run(int index,const std::string& label,const detail::Program& p,std::size_t root,const std::string& input,bool expected){{auto parsed=parse_json(input);if(!parsed)std::abort();bool valid=true;JsonValue::Object result;result.emplace("case",JsonValue(JsonInteger(index)));
try{{detail::Validation validator(p);validator.require(root,parsed.value(),"");}}
catch(detail::Failure& failure){{valid=false;auto& e=failure.error;if(e.kind!=CodecError::Kind::Validation){{std::cerr<<label<<" incomplete "<<e.source.pointer<<" "<<e.message;std::abort();}}
result.emplace("document",JsonValue(e.source.document));result.emplace("pointer",JsonValue(e.source.pointer));result.emplace("instancePath",JsonValue(e.instance_path));result.emplace("begin",JsonValue(JsonInteger(e.source.begin)));result.emplace("end",JsonValue(JsonInteger(e.source.end)));}}
if(valid!=expected){{std::cerr<<label<<" mismatch";std::abort();}}result.emplace("valid",JsonValue(valid));auto bytes=write_json(JsonValue(std::move(result)));if(!bytes)std::abort();std::cout<<bytes.value()<<'\n';}}
int main(){{{calls}}}
"#
    );
    std::fs::write(root.join("driver.cpp"), driver).unwrap();
    compile_native(&root, "official.cpp");
    let output = checked(&mut Command::new(root.join("consumer")), &root);
    let results = String::from_utf8(output.stdout).unwrap();
    let mut seen = 0;
    for line in results.lines() {
        let value: Value = serde_json::from_str(line).unwrap();
        let (contract, valid) = &expectations[value["case"].as_u64().unwrap() as usize];
        assert_eq!(value["valid"], *valid);
        if !valid {
            let id = source(
                value["document"].as_str().unwrap(),
                value["pointer"].as_str().unwrap(),
            );
            let span = contract.source_span(&id).unwrap();
            assert_eq!(value["begin"], span.start);
            assert_eq!(value["end"], span.end);
            assert!(contract.source(&id).is_some());
        }
        seen += 1;
    }
    assert_eq!(seen, 44);
    std::fs::write(root.join("native-results.jsonl"), results).unwrap();
    println!("C++ official 44 source-driven v3 cases: {}", root.display());
}

fn controls() -> Arc<Contract> {
    load(
        json!({"openapi":"3.2.0","info":{"title":"Independent dynamic controls","version":"1"},"components":{"schemas":{
            "Base":{"$id":"urn:cpp:base","$dynamicAnchor":"n","type":"string"},
            "Outer":{"$id":"urn:cpp:outer","$defs":{"binding":{"$dynamicAnchor":"n","type":"integer"}},"properties":{"value":{"$dynamicRef":"urn:cpp:base#n"},"pointer":{"$dynamicRef":"urn:cpp:base#"},"static":{"$ref":"urn:cpp:base#n"}}},
            "Poison":{"$id":"urn:cpp:poison","$dynamicAnchor":"n","not":{}},
            "Fallback":{"$dynamicRef":"urn:cpp:base#n"},
            "Trial":{"anyOf":[{"$ref":"urn:cpp:poison"},{"$dynamicRef":"urn:cpp:base#n"}]},
            "Initial":{"$id":"urn:cpp:initial","$dynamicAnchor":"n","$defs":{"flag":{"$dynamicAnchor":"flag","type":"boolean"}}},
            "Second":{"$id":"urn:cpp:second","$dynamicAnchor":"flag","type":"integer"},
            "NoPremature":{"$id":"urn:cpp:no-premature","$defs":{"binding":{"$dynamicAnchor":"n","$dynamicRef":"urn:cpp:second#flag"}},"$dynamicRef":"urn:cpp:initial#n"},
            "BadFlag":{"$id":"urn:cpp:bad-flag","$dynamicAnchor":"flag","not":{}},
            "Context":{"$id":"urn:cpp:context","if":{"$dynamicRef":"urn:cpp:bad-flag#flag"},"then":true,"else":{"$ref":"urn:cpp:new-context"}},
            "NewContext":{"$id":"urn:cpp:new-context","$defs":{"binding":{"$dynamicAnchor":"flag"}},"$ref":"urn:cpp:context"},
            "Detached":{"$id":"urn:cpp:detached","const":false,"$defs":{"binding":{"$dynamicAnchor":"n","type":"integer"},"start":{"$dynamicRef":"urn:cpp:base#n"}}},
            "Number":{"$id":"urn:cpp:number","$dynamicAnchor":"n","maximum":0},
            "Incomplete":{"anyOf":[true,{"not":{"$dynamicRef":"urn:cpp:number#n"}}]},
            "Cycle":{"$id":"urn:cpp:cycle","$dynamicAnchor":"n","not":{"$dynamicRef":"#n"}},
            "Simple":{"$id":"urn:cpp:simple","const":true},
            "Lookup":{"$id":"urn:cpp:lookup","$defs":{"a":{"$dynamicAnchor":"a","type":"string"},"n":{"$dynamicAnchor":"n","type":"integer"}},"$dynamicRef":"urn:cpp:base#n"}
        }}}),
        Vec::new(),
    )
}

#[test]
#[ignore = "native dynamic scope/branch/context-cycle/budget/control and normal-stack resource depth witnesses"]
fn native_v3_scope_and_resource_controls() {
    let root = root("controls-");
    let contract = controls();
    let selected = [
        "Outer",
        "Poison",
        "Fallback",
        "Trial",
        "NoPremature",
        "Context",
        "Detached/$defs/start",
        "Incomplete",
        "Cycle",
        "Simple",
        "Lookup",
        "Lookup/$defs/a",
    ]
    .map(|name| source(ENTRY, &format!("/components/schemas/{name}")));
    let program = suspect_schema::OwnedCompiler::new(Config {
        max_depth: 512,
        max_number_bytes: 3,
        ..Default::default()
    })
    .compile_v3(contract.clone(), &selected)
    .unwrap()
    .program();
    program.check().unwrap();
    crate::write_files(&emit::validation_runtime(&contract, &program), &root).unwrap();
    let target = |name: &str| {
        program
            .roots
            .iter()
            .find(|r| r.source.pointer == format!("/components/schemas/{name}"))
            .unwrap()
            .target
    };
    let cases = [
        (
            "outermost",
            "Outer",
            r#"{"value":7,"pointer":"s","static":"s"}"#,
            "Valid",
            "",
            "",
        ),
        (
            "outer-value",
            "Outer",
            r#"{"value":"s"}"#,
            "Invalid",
            "Outer/$defs/binding/type",
            "/value",
        ),
        (
            "pointer-fallback",
            "Outer",
            r#"{"pointer":7}"#,
            "Invalid",
            "Base/type",
            "/pointer",
        ),
        (
            "static-fallback",
            "Outer",
            r#"{"static":7}"#,
            "Invalid",
            "Base/type",
            "/static",
        ),
        ("unentered-poison", "Fallback", r#""s""#, "Valid", "", ""),
        ("trial-restoration", "Trial", r#""s""#, "Valid", "", ""),
        ("do-not-enter-fallback", "NoPremature", "7", "Valid", "", ""),
        (
            "late-fallback-type",
            "NoPremature",
            "true",
            "Invalid",
            "Second/type",
            "",
        ),
        ("changed-context-not-cycle", "Context", "7", "Valid", "", ""),
        (
            "detached-resource-without-root-eval",
            "Detached/$defs/start",
            "7",
            "Valid",
            "",
            "",
        ),
        (
            "detached-type",
            "Detached/$defs/start",
            r#""s""#,
            "Invalid",
            "Detached/$defs/binding/type",
            "",
        ),
        (
            "noninvertible-numeric",
            "Incomplete",
            "12345",
            "EvaluationFailure",
            "Number/maximum",
            "",
        ),
        (
            "context-cycle",
            "Cycle",
            "null",
            "EvaluationFailure",
            "Cycle",
            "",
        ),
    ];
    let mut calls = String::new();
    for (id, name, input, expected, src, path) in cases {
        writeln!(
            calls,
            "run({},p,{}, {}, {}, {}, {});",
            emit::string(id),
            target(name),
            emit::string(input),
            emit::string(expected),
            emit::string(&format!("/components/schemas/{src}")),
            emit::string(path)
        )
        .unwrap();
    }
    let mut schemas = serde_json::Map::new();
    for index in 0..550 {
        let mut schema = json!({"$id":format!("urn:cpp:depth:{index}")});
        if index < 549 {
            schema["$ref"] = json!(format!("urn:cpp:depth:{}", index + 1));
        }
        schemas.insert(format!("R{index}"), schema);
    }
    let deep = load(
        json!({"openapi":"3.2.0","info":{"title":"Deep resource ownership","version":"1"},"components":{"schemas":schemas}}),
        Vec::new(),
    );
    let deep_source = source(ENTRY, "/components/schemas/R0");
    let deep_program = suspect_schema::OwnedCompiler::new(Config {
        max_depth: 512,
        ..Default::default()
    })
    .compile_v3(deep.clone(), &[deep_source])
    .unwrap()
    .program();
    deep_program.check().unwrap();
    std::fs::write(
        root.join("cpp/src/deep.cpp"),
        emit::validation_program_named(&deep, &deep_program, &Default::default(), "deep_program"),
    )
    .unwrap();
    let driver = format!(
        r#"#include <generated_sdk/runtime.hpp>
#include <iostream>
#include <cstdlib>
#include <pthread.h>
using namespace generated_sdk;
namespace generated_sdk::detail {{const Program& deep_program();}}
#define CHECK(x) do{{if(!(x)){{std::cerr<<__LINE__<<" "<<#x;std::abort();}}}}while(false)
void run(const std::string& id,const detail::Program& p,std::size_t root,const std::string& input,const std::string& expected,const std::string& src,const std::string& path){{auto value=parse_json(input);CHECK(value);
try{{detail::Validation v(p);v.require(root,value.value(),"");CHECK(expected=="Valid");}}catch(detail::Failure& f){{auto kind=f.error.kind==CodecError::Kind::Validation?"Invalid":f.error.kind==CodecError::Kind::EvaluationFailure?"EvaluationFailure":"Other";
if(expected!=kind||f.error.source.pointer!=src||f.error.instance_path!=path){{std::cerr<<id<<" "<<kind<<" "<<f.error.source.pointer<<" "<<f.error.instance_path;std::abort();}}}}}}
void* depth(void*){{try{{detail::Validation v(detail::deep_program());v.require({deep_root},JsonValue(Null{{}}),"");CHECK(false);}}catch(detail::Failure& f){{CHECK(f.error.kind==CodecError::Kind::EvaluationFailure);}}return nullptr;}}
int main(){{auto p=detail::validation_program();{calls}
auto simple=p;simple.max_evaluation_steps=3;run("resource-entry-charge",simple,{simple_root},"true","Valid","","");simple.max_evaluation_steps=2;run("resource-entry-exhaustion",simple,{simple_root},"true","EvaluationFailure","/components/schemas/Simple/const","");
auto lookup=p;lookup.max_evaluation_steps=8;run("binding-scan-charge",lookup,{lookup_root},"7","Valid","","");lookup.max_evaluation_steps=7;run("binding-scan-exhaustion",lookup,{lookup_root},"7","EvaluationFailure","/components/schemas/Lookup/$defs/n/type","");
detail::Validation reuse(p);auto outer=parse_json("{{\"value\":7}}");reuse.require({outer_root},outer.value(),"");reuse.require({fallback_root},JsonValue("s"),"");
try{{reuse.require({cycle_root},JsonValue(Null{{}}),"");CHECK(false);}}catch(detail::Failure& f){{CHECK(f.error.kind==CodecError::Kind::EvaluationFailure);}}reuse.require({fallback_root},JsonValue("s"),"");
Cancellation stop;stop.cancel();try{{detail::Validation v(p,detail::Control{{stop.token(),std::nullopt}});v.require({fallback_root},JsonValue("s"),"");CHECK(false);}}catch(detail::Failure& f){{CHECK(f.error.kind==CodecError::Kind::Cancelled);}}
try{{detail::Validation v(p,detail::Control{{{{}},std::chrono::steady_clock::now()-std::chrono::seconds(1)}});v.require({fallback_root},JsonValue("s"),"");CHECK(false);}}catch(detail::Failure& f){{CHECK(f.error.kind==CodecError::Kind::Timeout);}}
for(int mode=0;mode<4;++mode){{auto bad=p;if(mode==0)bad.resource_context.reset();else if(mode==1)bad.resource_context->node_scopes.pop_back();else if(mode==2)bad.resource_context->node_scopes[0].resource=999999;else bad.profile="oas31-jsonschema202012-static-applicators";
try{{detail::Validation v(bad);CHECK(false);}}catch(detail::Failure& f){{CHECK(f.error.kind==CodecError::Kind::EvaluationFailure);}}}}
pthread_attr_t attrs;CHECK(pthread_attr_init(&attrs)==0);CHECK(pthread_attr_setstacksize(&attrs,2*1024*1024)==0);pthread_t thread;CHECK(pthread_create(&thread,&attrs,depth,nullptr)==0);pthread_attr_destroy(&attrs);CHECK(pthread_join(thread,nullptr)==0);
std::cout<<"dynamic contexts/branches/cycles/annotations/budgets/interruption/malformed metadata and 512-depth resources passed\n";
}}
"#,
        deep_root = deep_program.roots[0].target,
        simple_root = target("Simple"),
        lookup_root = target("Lookup"),
        outer_root = target("Outer"),
        fallback_root = target("Fallback"),
        cycle_root = target("Cycle")
    );
    std::fs::write(root.join("driver.cpp"), driver).unwrap();
    std::fs::write(
        root.join("program.json"),
        serde_json::to_string_pretty(&program).unwrap(),
    )
    .unwrap();
    compile_native(&root, "deep.cpp");
    checked(&mut Command::new(root.join("consumer")), &root);
    println!("C++ independent v3 resource controls: {}", root.display());
}

#[test]
fn v3_resource_metadata_mutations_are_source_linked_refusals() {
    let contract = controls();
    let roots = [
        source(ENTRY, "/components/schemas/Outer"),
        source(ENTRY, "/components/schemas/Poison"),
    ];
    let program = suspect_schema::OwnedCompiler::new(Default::default())
        .compile_v3(contract.clone(), &roots)
        .unwrap()
        .program();
    check_program_resources(&contract, &program, true, true).unwrap();
    for mode in 0..10 {
        let mut bad = program.clone();
        match mode {
            0 => bad.resource_context = None,
            1 => {
                bad.resource_context.as_mut().unwrap().node_scopes.pop();
            }
            2 => bad.resource_context.as_mut().unwrap().node_scopes[0].0 = usize::MAX,
            3 => bad.resource_context.as_mut().unwrap().node_scopes[0].2 = "urn:wrong".into(),
            4 => bad.resource_context.as_mut().unwrap().resources[0]
                .aliases
                .clear(),
            5 => {
                let context = bad.resource_context.as_mut().unwrap();
                let uri = context.resources[0].canonical_uri.clone();
                context.resources[1].aliases.push(uri);
            }
            6 => {
                let resource = bad
                    .resource_context
                    .as_mut()
                    .unwrap()
                    .resources
                    .iter_mut()
                    .find(|r| !r.dynamic_anchors.is_empty())
                    .unwrap();
                resource.dynamic_anchors[0].2 = usize::MAX;
            }
            7 => {
                for check in bad.nodes.iter_mut().flat_map(|n| &mut n.checks) {
                    if let ProgramInstruction::DynamicRef {
                        initial_resource, ..
                    } = &mut check.instruction
                    {
                        *initial_resource = usize::MAX;
                        break;
                    }
                }
            }
            8 => {
                bad.version = OwnedProgram::V2_VERSION;
                bad.profile = OwnedProgram::V2_PROFILE;
            }
            _ => bad.profile = "unknown",
        }
        assert!(bad.check().is_err());
        assert!(check_program_resources(&contract, &bad, true, true).is_err());
    }
}

#[test]
#[ignore = "installed C++ checked resource/dynamic SDK, native models/examples/docs/types and live operations"]
fn native_v3_sdk_operations() {
    use std::io::Read;
    let root = root("sdk-");
    let document: Value =
        serde_json::from_str(include_str!("tests/resources.openapi.json")).unwrap();
    let contract = load(document, Vec::new());
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let config = SdkConfig::default();
    let plan = plan_sdk(contract, &selected, config).unwrap_or_else(|e| panic!("{e:#?}"));
    assert_eq!(plan.program.version, OwnedProgram::V3_VERSION);
    super::v2_tests::package(&plan, &root);
    assert!(root.join("generated/cpp/examples/client.cpp").is_file());
    let directory = root.join("consumer");
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(
        directory.join("main.cpp"),
        include_str!("tests/native_resources.cpp"),
    )
    .unwrap();
    std::fs::write(directory.join("CMakeLists.txt"),"cmake_minimum_required(VERSION 3.24)\nproject(ResourceConsumer LANGUAGES CXX)\nfind_package(generated_sdk CONFIG REQUIRED)\nadd_executable(consumer main.cpp)\ntarget_link_libraries(consumer PRIVATE generated_sdk::generated_sdk)\ntarget_compile_options(consumer PRIVATE -Wall -Wextra -Wpedantic -Werror)\n").unwrap();
    let cmake = std::env::var_os("SUSPECT_CPP_CMAKE")
        .map(PathBuf::from)
        .unwrap_or_else(|| "cmake".into());
    checked(
        Command::new(&cmake)
            .arg("-S")
            .arg(&directory)
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
        Command::new(&cmake)
            .arg("--build")
            .arg(root.join("build-consumer")),
        &root,
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/api", listener.local_addr().unwrap());
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stopping = stop.clone();
    let log = root.join("wire.json");
    let server = std::thread::spawn(move || {
        let mut records = Vec::new();
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
                .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0u8; 4096];
            let end = loop {
                let n = stream.read(&mut buffer).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(at) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                    break at + 4;
                }
            };
            let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
            let first = headers.lines().next().unwrap();
            let length = headers
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .map(|n| n.parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            while bytes.len() < end + length {
                let n = stream.read(&mut buffer).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
            }
            let body = &bytes[end..end + length];
            records.push(json!({"request":first,"body":String::from_utf8(body.to_vec()).unwrap()}));
            let (media, body) = if first.contains("/items ") {
                ("application/x-ndjson",b"{\"data\":1,\"children\":[{\"data\":2}]}\n{\"data\":1,\"children\":[{\"data\":2,\"unexpected\":true}]}\n".to_vec())
            } else if first.contains("/strict/bad ") {
                (
                    "application/json",
                    b"{\"data\":1,\"children\":[{\"data\":2,\"unexpected\":true}]}".to_vec(),
                )
            } else {
                ("application/json", body.to_vec())
            };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {media}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(&body);
        }
        std::fs::write(log, serde_json::to_string_pretty(&records).unwrap()).unwrap();
        records
    });
    let result = Command::new(root.join("build-consumer/consumer"))
        .arg(url)
        .output()
        .unwrap();
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let requests = server.join().unwrap();
    std::fs::write(
        root.join("native.log"),
        [result.stdout.clone(), result.stderr.clone()].concat(),
    )
    .unwrap();
    assert!(
        result.status.success(),
        "{}: {}{}",
        root.display(),
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(requests.len(), 6);
    assert_eq!(
        requests[0]["body"],
        "{\"children\":[{\"data\":2}],\"data\":1}"
    );
    assert_eq!(requests[3]["body"], "{\"value\":7}");
    assert_eq!(requests[4]["body"], "9");
    for (i, code) in [
        "Tree value;",
        "Tree value(JsonInteger(1));value.data=std::string(\"not integer\");",
        "Tree value(JsonInteger(1));value.children=std::vector<Tree>{Tree(JsonInteger(2))};",
    ]
    .iter()
    .enumerate()
    {
        let path = root.join(format!("negative-{i}.cpp"));
        std::fs::write(&path,format!("#include <generated_sdk/sdk.hpp>\nusing namespace generated_sdk;int main(){{{code}(void)value;}}")).unwrap();
        let output = Command::new(cxx())
            .args(["-std=c++20", "-fsyntax-only", "-I"])
            .arg(root.join("install/include"))
            .arg(&path)
            .output()
            .unwrap();
        std::fs::write(root.join(format!("negative-{i}.log")), &output.stderr).unwrap();
        assert!(!output.status.success());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("file not found"));
    }
    println!(
        "C++ resource/dynamic installed SDK 6-wire gate: {}",
        root.display()
    );
}

#[test]
fn public_v3_selection_preserves_ordinary_programs_and_candidate_emission() {
    let contract = load(
        serde_json::from_str(include_str!("tests/resources.openapi.json")).unwrap(),
        Vec::new(),
    );
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let config = SdkConfig::default();
    let candidate = protocol::plan_capabilities(
        contract.clone(),
        &selected,
        config.clone(),
        true,
        capabilities(&config)
            .with(http_protocol::Capability::SchemaResources)
            .with(http_protocol::Capability::DynamicSchemaReferences),
    )
    .unwrap();
    let public = plan_sdk(contract, &selected, config).unwrap();
    assert_eq!(public.program.version, OwnedProgram::V3_VERSION);
    assert_eq!(public.render().unwrap(), emit::package(&candidate).unwrap());
    let doc: Value = serde_json::from_str(include_str!("tests/scoped.openapi.json")).unwrap();
    let contract = load(doc, Vec::new());
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let config = SdkConfig::default();
    let old_capabilities = http_protocol::Capabilities::for_adapter(
        capabilities(&config).adapter(),
        capabilities(&config).enabled().iter().copied().filter(|c| {
            !matches!(
                c,
                http_protocol::Capability::SchemaResources
                    | http_protocol::Capability::DynamicSchemaReferences
            )
        }),
    )
    .with_limits(capabilities(&config).limits());
    let old = protocol::plan_capabilities(
        contract.clone(),
        &selected,
        config.clone(),
        true,
        old_capabilities,
    )
    .unwrap();
    let public = plan_sdk(contract, &selected, config).unwrap();
    assert_eq!(public.program.version, OwnedProgram::V2_VERSION);
    assert_eq!(old.program, public.program);
    assert!(public.program.resource_context.is_none());
    for (name, op, version) in [
        ("V1", json!({"type":"integer"}), OwnedProgram::V1_VERSION),
        (
            "V2",
            json!({"if":true,"then":{"type":"integer"}}),
            OwnedProgram::V2_VERSION,
        ),
    ] {
        let contract = load(
            json!({"openapi":"3.2.0","info":{"title":"Stable public selection","version":"1"},"servers":[{"url":"https://example.test"}],"paths":{"/x":{"get":{"operationId":"x","responses":{"200":{"content":{"application/json":{"schema":op}}}}}}}}),
            Vec::new(),
        );
        let selected = contract
            .operations()
            .map(|op| op.source().clone())
            .collect::<Vec<_>>();
        let plan = plan_sdk(contract, &selected, Default::default()).unwrap();
        assert_eq!(plan.program.version, version, "{name}");
        assert!(plan.program.resource_context.is_none());
    }
}
