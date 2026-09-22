//! Native resource/dynamic execution from original supplied source documents.
use serde_json::{Value, json};
use std::{path::Path, process::Command, sync::Arc};
use suspect_codegen::{OutFile, go_validation};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_schema::{Config, OwnedCompiler, OwnedProgram};
use suspect_source::Uri;

const ENTRY: &str = "https://physical.test/api.json";
fn id(uri: &str, pointer: &str) -> SchemaId {
    let mut source = SchemaId::new(Uri::parse(uri).unwrap(), Default::default());
    for token in pointer.split('/').skip(1) {
        source = source.child(&token.replace("~1", "/").replace("~0", "~"));
    }
    source
}
fn root(name: &str) -> SchemaId {
    id(ENTRY, "/components/schemas").child(name)
}
fn api(schemas: Value) -> Value {
    json!({"openapi":"3.2.0","info":{"title":"Native resource witnesses","version":"1"},"paths":{},"components":{"schemas":schemas}})
}
fn load(document: Value, external: Vec<(&str, Value)>) -> Arc<Contract> {
    let provider = Arc::new(
        DocumentProvider::new(std::iter::once((ENTRY, document)).chain(external).map(
            |(uri, value)| {
                ProvidedDocument::new(
                    Uri::parse(uri).unwrap(),
                    Uri::parse(uri).unwrap(),
                    value.to_string().into_bytes(),
                )
                .unwrap()
            },
        ))
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
fn compile(c: Arc<Contract>, roots: &[SchemaId], config: Config) -> OwnedProgram {
    OwnedCompiler::new(config)
        .compile_v3(c, roots)
        .unwrap()
        .program()
}
fn tools() -> Vec<String> {
    std::env::var("SUSPECT_GO_TOOLCHAIN")
        .map(|t| vec![t])
        .unwrap_or_else(|_| vec!["go1.23.12".into(), "go1.27.1".into()])
}
fn run(dir: &Path, tool: &str, args: &[&str]) -> std::process::Output {
    Command::new("go")
        .current_dir(dir)
        .args(args)
        .env("GOWORK", "off")
        .env("GOTOOLCHAIN", tool)
        .output()
        .unwrap()
}
fn success(dir: &Path, tool: &str, args: &[&str]) {
    let output = run(dir, tool, args);
    assert!(
        output.status.success(),
        "{} {tool} {args:?}\n{}{}",
        dir.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!("{tool}: {}", String::from_utf8_lossy(&output.stdout).trim());
}
fn q(value: &str) -> String {
    serde_json::to_string(value).unwrap()
}
fn package(program: &OwnedProgram, index: usize, files: &mut Vec<OutFile>) {
    let mut output = go_validation::emit(program).unwrap();
    output.extend(suspect_codegen::go_json::emit());
    for file in output {
        if file.path == "go/go.mod" {
            continue;
        };
        files.push(OutFile {
            path: format!("sdk/case{index}/{}", file.path.strip_prefix("go/").unwrap()),
            content: file.content,
        });
    }
}
fn files() -> Vec<OutFile> {
    vec![OutFile{path:"sdk/go.mod".into(),content:"module example.com/go-v3\n\ngo 1.23.0\n".into()},OutFile{path:"consumer/go.mod".into(),content:"module example.com/go-v3-consumer\n\ngo 1.23.0\nrequire example.com/go-v3 v0.0.0\nreplace example.com/go-v3 => ../sdk\n".into()}]
}
fn native(files: Vec<OutFile>, code: String) {
    let root = tempfile::Builder::new()
        .prefix("suspect-go-v3-")
        .tempdir()
        .unwrap()
        .keep();
    let mut files = files;
    files.push(OutFile {
        path: "consumer/resource_test.go".into(),
        content: code,
    });
    suspect_codegen::write_files(&files, &root).unwrap();
    for tool in tools() {
        success(
            &root.join("consumer"),
            &tool,
            &["test", "-count=1", "-timeout=60s", "-v", "."],
        )
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "requires native Go 1.23.12/1.27.1"]
fn official_44_unmodified_resource_cases_execute_with_closed_supplied_documents() {
    let groups: Value = serde_json::from_str(include_str!(
        "../../suspect-schema/tests/fixtures/resource-conformance/dynamicRef.json"
    ))
    .unwrap();
    let mut files = files();
    let mut imports = String::from("package consumer\nimport(\"errors\";\"testing\"\n");
    let mut tests = String::new();
    let mut count = 0;
    for (index, group) in groups.as_array().unwrap().iter().enumerate() {
        let document = "https://physical.test/official-schema.json";
        let c=load(api(json!({"Use":{"$ref":document}})),vec![
            (document,group["schema"].clone()),
            ("http://localhost:1234/draft2020-12/tree.json",serde_json::from_str(include_str!("../../suspect-schema/tests/fixtures/resource-conformance/tree.json")).unwrap()),
            ("http://localhost:1234/draft2020-12/extendible-dynamic-ref.json",serde_json::from_str(include_str!("../../suspect-schema/tests/fixtures/resource-conformance/extendible-dynamic-ref.json")).unwrap()),
            ("http://localhost:1234/draft2020-12/detached-dynamicref.json",serde_json::from_str(include_str!("../../suspect-schema/tests/fixtures/resource-conformance/detached-dynamicref.json")).unwrap()),
        ]);
        let program = compile(c, &[id(document, "")], Config::default());
        assert_eq!(program.version, OwnedProgram::V3_VERSION);
        assert!(program.resource_context.is_some());
        package(&program, index, &mut files);
        imports.push_str(&format!("s{index} \"example.com/go-v3/case{index}\"\n"));
        for case in group["tests"].as_array().unwrap() {
            let label = format!(
                "{} / {}",
                group["description"].as_str().unwrap(),
                case["description"].as_str().unwrap()
            );
            tests.push_str(&format!("func TestOfficial{count}(t *testing.T){{value,err:=s{index}.Parse([]byte({}),s{index}.DefaultLimits());if err!=nil{{t.Fatal(err)}};err=s{index}.Validate({},value);",q(&case["data"].to_string()),program.roots[0].target));
            if case["valid"] == true {
                tests.push_str(&format!(
                    "if err!=nil{{t.Fatalf(\"%s: %v\",{},err)}}",
                    q(&label)
                ));
            } else {
                tests.push_str(&format!("var failure *s{index}.ValidationError;if !errors.As(err,&failure)||failure.Kind!=\"invalid\"{{t.Fatalf(\"%s: %#v / %v\",{},failure,err)}}",q(&label)));
            }
            tests.push_str("}\n");
            count += 1;
        }
    }
    assert_eq!(count, 44);
    imports.push_str(")\n");
    imports.push_str(&tests);
    native(files, imports);
}

#[test]
fn malformed_v3_contexts_never_emit_and_old_envelopes_reject_resources() {
    let c = load(
        api(
            json!({"Tree":{"$id":"urn:tree","$dynamicAnchor":"node","properties":{"next":{"$dynamicRef":"#node"}}},"Strict":{"$id":"urn:strict","$dynamicAnchor":"node","$ref":"urn:tree","unevaluatedProperties":false}}),
        ),
        vec![],
    );
    let original = compile(c, &[root("Strict")], Config::default());
    assert!(go_validation::emit(&original).is_ok());
    for mutation in 0..8 {
        let mut program = original.clone();
        let context = program.resource_context.as_mut().unwrap();
        match mutation {
            0 => {
                context.node_scopes.pop();
            }
            1 => context.node_scopes[0].0 = usize::MAX,
            2 => context.node_scopes[0].2 = "urn:wrong".into(),
            3 => context.resources[0].aliases.clear(),
            4 => {
                let alias = context.resources[0].canonical_uri.clone();
                context.resources[1].aliases.push(alias)
            }
            5 => {
                context
                    .resources
                    .iter_mut()
                    .find(|r| !r.dynamic_anchors.is_empty())
                    .unwrap()
                    .dynamic_anchors[0]
                    .2 = usize::MAX
            }
            6 => program.resource_context = None,
            _ => {
                for node in &mut program.nodes {
                    for check in &mut node.checks {
                        if let suspect_schema::ProgramInstruction::DynamicRef {
                            initial_resource,
                            ..
                        } = &mut check.instruction
                        {
                            *initial_resource = usize::MAX
                        }
                    }
                }
            }
        };
        assert!(
            go_validation::emit(&program).is_err(),
            "mutation {mutation}"
        );
    }
    for (version, profile) in [
        (OwnedProgram::V1_VERSION, OwnedProgram::V1_PROFILE),
        (OwnedProgram::V2_VERSION, OwnedProgram::V2_PROFILE),
    ] {
        let mut changed = original.clone();
        changed.version = version;
        changed.profile = profile;
        assert!(go_validation::emit(&changed).is_err());
        changed.resource_context = None;
        assert!(go_validation::emit(&changed).is_err());
    }
}

#[test]
#[ignore = "requires native Go 1.23.12/1.27.1"]
fn independent_dynamic_scope_branch_cycle_budget_and_annotation_controls() {
    struct Case {
        c: Arc<Contract>,
        roots: Vec<SchemaId>,
        selected: SchemaId,
        value: Value,
        kind: &'static str,
        pointer: &'static str,
        path: &'static str,
        config: Config,
    }
    let mut cases = Vec::new();
    let c = load(
        api(json!({
            "Base":{"$id":"urn:base","$dynamicAnchor":"node","type":"object","properties":{"outer":true,"middle":true,"children":{"type":"array","items":{"$dynamicRef":"#node"}}}},
            "Middle":{"$id":"urn:middle","$dynamicAnchor":"node","$ref":"urn:base","required":["middle"]},
            "Outer":{"$id":"urn:outer","$dynamicAnchor":"node","$ref":"urn:middle","required":["outer"]},
            "Unentered":{"$id":"urn:unentered","$dynamicAnchor":"node","not":{}}
        })),
        vec![],
    );
    for (value, kind, pointer, path) in [
        (
            json!({"outer":true,"middle":true,"children":[{"outer":true,"middle":true}]}),
            "valid",
            "",
            "",
        ),
        (
            json!({"outer":true,"middle":true,"children":[{"middle":true}]}),
            "invalid",
            "/components/schemas/Outer/required",
            "/children/0",
        ),
    ] {
        cases.push(Case {
            c: c.clone(),
            roots: vec![root("Outer"), root("Unentered")],
            selected: root("Outer"),
            value,
            kind,
            pointer,
            path,
            config: Config::default(),
        });
    }
    let c = load(
        api(json!({
            "Base":{"$id":"urn:base","type":"boolean","$defs":{"T":{"$dynamicAnchor":"n","$anchor":"plain","type":"string"}}},
            "Root":{"$id":"urn:root","$defs":{"N":{"$dynamicAnchor":"n","type":"integer"}},"properties":{"dynamic":{"$dynamicRef":"urn:base#n"},"pointer":{"$dynamicRef":"urn:base#/$defs/T"},"plain":{"$dynamicRef":"urn:base#plain"},"empty":{"$dynamicRef":"urn:base#"},"static":{"$ref":"urn:base#n"}}}
        })),
        vec![],
    );
    cases.push(Case {
        c: c.clone(),
        roots: vec![root("Root")],
        selected: root("Root"),
        value: json!({"dynamic":7,"pointer":"p","plain":"p","empty":true,"static":"p"}),
        kind: "valid",
        pointer: "",
        path: "",
        config: Config::default(),
    });
    cases.push(Case {
        c,
        roots: vec![root("Root")],
        selected: root("Root"),
        value: json!({"pointer":7}),
        kind: "invalid",
        pointer: "/components/schemas/Base/$defs/T/type",
        path: "/pointer",
        config: Config::default(),
    });
    let c = load(
        api(json!({
            "Outer":{"$id":"urn:outer","const":false,"$defs":{"binding":{"$dynamicAnchor":"n","type":"integer"},"start":{"$ref":"urn:base#/$defs/use"}}},
            "Base":{"$id":"urn:base","$defs":{"binding":{"$dynamicAnchor":"n","type":"string"},"use":{"$dynamicRef":"#n"}}}
        })),
        vec![],
    );
    let selected = root("Outer").child("$defs").child("start");
    for (value, kind, pointer) in [
        (json!(7), "valid", ""),
        (
            json!("fallback"),
            "invalid",
            "/components/schemas/Outer/$defs/binding/type",
        ),
    ] {
        cases.push(Case {
            c: c.clone(),
            roots: vec![selected.clone()],
            selected: selected.clone(),
            value,
            kind,
            pointer,
            path: "",
            config: Config::default(),
        });
    }
    let c = load(
        api(json!({
            "Fallback":{"$id":"urn:fallback","$dynamicAnchor":"flag","not":{}},
            "Outer":{"$id":"urn:outer","if":{"$dynamicRef":"urn:fallback#flag"},"then":true,"else":{"$ref":"urn:changed"}},
            "Changed":{"$id":"urn:changed","$defs":{"flag":{"$dynamicAnchor":"flag"}},"$ref":"urn:outer"},
            "Plain":{"$id":"urn:plain","$dynamicAnchor":"node","type":"string"},
            "Failed":{"$id":"urn:failed","$dynamicAnchor":"node","not":{}},
            "Trial":{"anyOf":[{"$ref":"urn:failed"},{"$dynamicRef":"urn:plain#node"}]}
        })),
        vec![],
    );
    cases.push(Case {
        c: c.clone(),
        roots: vec![root("Outer"), root("Trial")],
        selected: root("Outer"),
        value: json!(7),
        kind: "valid",
        pointer: "",
        path: "",
        config: Config::default(),
    });
    cases.push(Case {
        c,
        roots: vec![root("Outer"), root("Trial")],
        selected: root("Trial"),
        value: json!("survives failed trial"),
        kind: "valid",
        pointer: "",
        path: "",
        config: Config::default(),
    });
    let c = load(
        api(
            json!({"Root":{"$id":"urn:root","$dynamicAnchor":"n","anyOf":[true,{"$dynamicRef":"#n"}]}}),
        ),
        vec![],
    );
    cases.push(Case {
        c,
        roots: vec![root("Root")],
        selected: root("Root"),
        value: Value::Null,
        kind: "evaluation_failure",
        pointer: "/components/schemas/Root",
        path: "",
        config: Config::default(),
    });
    let c = load(
        api(
            json!({"Root":{"$id":"urn:root","$dynamicRef":"urn:base#n"},"Base":{"$id":"urn:base","$dynamicAnchor":"n","type":"number"}}),
        ),
        vec![],
    );
    for (steps, kind, pointer) in [
        (7, "valid", ""),
        (6, "evaluation_failure", "/components/schemas/Base/type"),
    ] {
        cases.push(Case {
            c: c.clone(),
            roots: vec![root("Root")],
            selected: root("Root"),
            value: json!(12345),
            kind,
            pointer,
            path: "",
            config: Config {
                max_evaluation_steps: steps,
                max_number_bytes: 3,
                ..Config::default()
            },
        });
    }
    let c = load(
        api(json!({
            "Root":{"$id":"urn:root","$defs":{"N":{"$dynamicAnchor":"n","type":"integer"}},"anyOf":[true,{"$dynamicRef":"urn:base#n"}]},
            "Base":{"$id":"urn:base","$dynamicAnchor":"n","type":"string"}
        })),
        vec![],
    );
    cases.push(Case {
        c,
        roots: vec![root("Root")],
        selected: root("Root"),
        value: json!(12345),
        kind: "evaluation_failure",
        pointer: "/components/schemas/Root/$defs/N/type",
        path: "",
        config: Config {
            max_number_bytes: 3,
            ..Config::default()
        },
    });
    let c = load(
        api(json!({
            "Root":{"$id":"urn:root","$defs":{"A":{"$dynamicAnchor":"a"},"Z":{"$dynamicAnchor":"z","type":"integer"}},"$dynamicRef":"urn:base#z"},
            "Base":{"$id":"urn:base","$dynamicAnchor":"z","type":"string"}
        })),
        vec![],
    );
    for (steps, kind, pointer) in [
        (8, "valid", ""),
        (
            7,
            "evaluation_failure",
            "/components/schemas/Root/$defs/Z/type",
        ),
    ] {
        cases.push(Case {
            c: c.clone(),
            roots: vec![root("Root"), root("Root").child("$defs").child("A")],
            selected: root("Root"),
            value: json!(1),
            kind,
            pointer,
            path: "",
            config: Config {
                max_evaluation_steps: steps,
                ..Config::default()
            },
        });
    }
    let c = load(
        api(json!({
            "Root":{"$id":"urn:root","$defs":{"N":{"$dynamicAnchor":"n","unevaluatedProperties":false}},"if":{"properties":{"a":true}},"then":{"$dynamicRef":"urn:base#n"}},
            "Base":{"$id":"urn:base","$dynamicAnchor":"n"}
        })),
        vec![],
    );
    cases.push(Case {
        c,
        roots: vec![root("Root")],
        selected: root("Root"),
        value: json!({"a":1}),
        kind: "invalid",
        pointer: "/components/schemas/Root/$defs/N/unevaluatedProperties",
        path: "/a",
        config: Config::default(),
    });
    let c = load(
        api(
            json!({"Root":{"$id":"urn:root","$defs":{"N":{"$dynamicAnchor":"n","properties":{"a":true}}},"$dynamicRef":"urn:base#n","unevaluatedProperties":false},"Base":{"$id":"urn:base","$dynamicAnchor":"n"}}),
        ),
        vec![],
    );
    cases.push(Case {
        c,
        roots: vec![root("Root")],
        selected: root("Root"),
        value: json!({"a":1}),
        kind: "valid",
        pointer: "",
        path: "",
        config: Config::default(),
    });
    let mut files = files();
    let mut code = String::from("package consumer\nimport(\"errors\";\"testing\"\n");
    let mut tests = String::new();
    for (index, case) in cases.iter().enumerate() {
        let program = compile(case.c.clone(), &case.roots, case.config.clone());
        let target = program
            .roots
            .iter()
            .find(|r| {
                r.source.pointer == case.selected.pointer()
                    && r.source.document == case.selected.document().as_str()
            })
            .unwrap()
            .target;
        package(&program, index, &mut files);
        code.push_str(&format!("s{index} \"example.com/go-v3/case{index}\"\n"));
        tests.push_str(&format!("func TestControl{index}(t *testing.T){{value,err:=s{index}.Parse([]byte({}),s{index}.DefaultLimits());if err!=nil{{t.Fatal(err)}};for attempt:=0;attempt<2;attempt++{{err=s{index}.Validate({target},value);",q(&case.value.to_string())));
        if case.kind == "valid" {
            tests.push_str("if err!=nil{t.Fatal(err)}");
        } else {
            tests.push_str(&format!("var failure *s{index}.ValidationError;if !errors.As(err,&failure)||failure.Kind!={}||failure.Source.Document!={}||failure.Source.Pointer!={}||failure.InstancePath!={}{{t.Fatalf(\"%#v / %v\",failure,err)}}",q(case.kind),q(ENTRY),q(case.pointer),q(case.path)));
        };
        tests.push_str("}}\n");
    }
    code.push_str(")\n");
    code.push_str(&tests);
    native(files, code);
}

fn codec_contract() -> Arc<Contract> {
    load(
        api(json!({
            "Fallback":{"$id":"urn:fallback","$dynamicAnchor":"n","type":"string"},
            "Use":{"$id":"urn:use","$defs":{"Value":{"$dynamicRef":"urn:fallback#n"}}},
            "Record":{"$id":"urn:record","$defs":{"N":{"$dynamicAnchor":"n","type":"integer"}},"type":"object","required":["name","value"],"properties":{"name":{"type":"string"},"value":{"$ref":"urn:use#/$defs/Value"}},"additionalProperties":false}
        })),
        vec![],
    )
}

#[test]
#[ignore = "requires native Go 1.23.12/1.27.1"]
fn native_codec_boundaries_preserve_outer_dynamic_context_and_base_profile_selection() {
    let c = codec_contract();
    let selected = root("Record");
    let plan = suspect_codegen::go_codecs::plan_codecs(
        c.clone(),
        std::slice::from_ref(&selected),
        Default::default(),
    )
    .unwrap();
    assert_eq!(plan.validation_program().version, OwnedProgram::V3_VERSION);
    let dynamic = root("Use").child("$defs").child("Value");
    let name = plan
        .models()
        .symbols()
        .iter()
        .find(|s| {
            s.source() == &dynamic
                && s.role() == suspect_codegen::rust_models::RepresentationRole::Model
        })
        .unwrap()
        .name();
    let temp = tempfile::Builder::new()
        .prefix("suspect-go-v3-codecs-")
        .tempdir()
        .unwrap()
        .keep();
    suspect_codegen::write_files(&plan.render(), &temp).unwrap();
    let consumer = temp.join("consumer");
    std::fs::create_dir(&consumer).unwrap();
    std::fs::write(consumer.join("go.mod"),"module example.com/v3-codec-consumer\n\ngo 1.23.0\nrequire example.com/generated-models v0.0.0\nreplace example.com/generated-models => ../go\n").unwrap();
    std::fs::write(consumer.join("codec_test.go"),format!(r#"package consumer
import("encoding/json";"errors";"strings";"testing";sdk "example.com/generated-models")
func TestContextualModelBoundary(t *testing.T){{
 value,err:=sdk.Codecs.Record.Decode([]byte(`{{"name":"native","value":7}}`));if err!=nil{{t.Fatal("nested codec evaluated fallback without outer context",err)}};if value.Name!="native"||!value.Value.IsValue{{t.Fatal(value)}}
 number,err:=sdk.ParseInteger("9");if err!=nil{{t.Fatal(err)}};value.Value.Value=number
 wire,err:=sdk.Codecs.Record.Encode(value);if err!=nil||string(wire)!=`{{"name":"native","value":9}}`{{t.Fatal(string(wire),err)}}
 var restored sdk.Record;if err=json.Unmarshal(wire,&restored);err!=nil{{t.Fatal(err)}}
 value.Value.Value="fallback-only";_,err=sdk.Codecs.Record.EncodeValue(value);var invalid *sdk.CodecError;if !errors.As(err,&invalid)||invalid.Kind!="invalid"||!strings.Contains(invalid.Source.Pointer,"/$defs/N/type"){{t.Fatal("outer binding lost on mutable encode",err)}}
 standalone,err:=sdk.Codecs.{name}.Decode([]byte(`"standalone"`));if err!=nil{{t.Fatal("unentered outer candidate overrode fallback",err)}};if !standalone.IsValue{{t.Fatal(standalone)}}
 _,err=sdk.Codecs.{name}.Decode([]byte(`7`));if !errors.As(err,&invalid)||invalid.Kind!="invalid"{{t.Fatal(err)}}
 for i:=0;i<3;i++{{if _,err=sdk.Codecs.Record.Decode([]byte(`{{"name":"native","value":7}}`));err!=nil{{t.Fatal("resource context leaked across codec calls",err)}}}}
}}
"#)).unwrap();
    for tool in tools() {
        success(&consumer, &tool, &["test", "-count=1", "-v", "."]);
    }
    let basic = load(api(json!({"Root":{"type":"string"}})), vec![]);
    let basic = suspect_codegen::go_codecs::plan_codecs(basic, &[root("Root")], Default::default())
        .unwrap();
    assert_eq!(basic.validation_program().version, OwnedProgram::V1_VERSION);
    assert!(basic.validation_program().resource_context.is_none());
    let scoped = load(
        api(json!({"Root":{"if":true,"then":{"type":"string"}}})),
        vec![],
    );
    let scoped =
        suspect_codegen::go_codecs::plan_codecs(scoped, &[root("Root")], Default::default())
            .unwrap();
    assert_eq!(
        scoped.validation_program().version,
        OwnedProgram::V2_VERSION
    );
    assert!(scoped.validation_program().resource_context.is_none());
    std::fs::remove_dir_all(temp).unwrap();
}

#[test]
#[ignore = "requires native Go 1.23.12/1.27.1; Sphinx enabled with SUSPECT_SPHINX_PYTHON"]
fn installed_resource_sdk_keeps_typed_fields_dynamic_wire_checks_and_examples() {
    let schemas = json!({
        "Fallback":{"$id":"urn:fallback","$dynamicAnchor":"n","type":"string"},
        "Use":{"$id":"urn:use","$defs":{"Value":{"$dynamicRef":"urn:fallback#n"}}},
        "Record":{"$id":"urn:record","$defs":{"N":{"$dynamicAnchor":"n","type":"integer"}},"type":"object","required":["name","value"],"properties":{"name":{"type":"string","minLength":1},"value":{"$ref":"urn:use#/$defs/Value"}},"additionalProperties":false},
        "Tree":{"$id":"urn:tree","$dynamicAnchor":"node","type":"object","properties":{"data":{"type":"string"},"children":{"type":"array","items":{"$dynamicRef":"#node"}}}},
        "Strict":{"$id":"urn:strict","$dynamicAnchor":"node","$ref":"urn:tree","unevaluatedProperties":false}
    });
    let mut document = api(schemas);
    document["$self"] = json!("https://logical.test/description/api.json");
    document["servers"] = json!([{"url":"../service/%2e"}]);
    for (name, schema, example) in [
        (
            "saveRecord",
            "urn:record",
            json!({"name":"native","value":7}),
        ),
        (
            "saveTree",
            "urn:strict",
            json!({"data":"root","children":[{"data":"child"}]}),
        ),
    ] {
        document["paths"][&format!("/{name}")] = json!({"post":{"operationId":name,"requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":schema},"example":example}}},"responses":{"200":{"description":"source-bound response","content":{"application/json":{"schema":{"$ref":schema}}}}}}});
    }
    let c = load(document, vec![]);
    let selected = c
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    let plan = suspect_codegen::go_http::plan_http(c, &selected, Default::default()).unwrap();
    assert_eq!(
        plan.codecs().validation_program().version,
        OwnedProgram::V3_VERSION
    );
    for capability in [
        suspect_codegen::http_protocol::Capability::SchemaResources,
        suspect_codegen::http_protocol::Capability::DynamicSchemaReferences,
        suspect_codegen::http_protocol::Capability::DocumentRelativeServers,
    ] {
        assert!(plan.protocol().capabilities().supports(capability));
    }
    let generated = plan.render();
    let bindings: Value = serde_json::from_str(
        &generated
            .iter()
            .find(|f| f.path == "go/docs/source-bindings.json")
            .unwrap()
            .content,
    )
    .unwrap();
    assert!(
        bindings["examples"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["available"] == true),
        "v3 examples must keep their valid source data: {bindings}"
    );
    let temp = tempfile::Builder::new()
        .prefix("suspect-go-resource-sdk-")
        .tempdir()
        .unwrap()
        .keep();
    suspect_codegen::write_files(&generated, &temp).unwrap();
    let consumer = temp.join("consumer");
    std::fs::create_dir(&consumer).unwrap();
    std::fs::write(consumer.join("go.mod"),"module example.com/resource-sdk-consumer\n\ngo 1.23.0\nrequire example.com/generated-sdk v0.0.0\nreplace example.com/generated-sdk => ../go\n").unwrap();
    std::fs::write(consumer.join("sdk_test.go"), RESOURCE_SDK).unwrap();
    for tool in tools() {
        success(&temp.join("go"), &tool, &["test", "./..."]);
        success(&temp.join("go"), &tool, &["run", "./examples/validated"]);
        success(
            &consumer,
            &tool,
            &["test", "-race", "-count=1", "-timeout=60s", "-v", "."],
        );
        for (index, invalid) in [
            "var _ = sdk.NewRecord()",
            "var _ = sdk.NewRecord(7,sdk.NullableNull[sdk.Value]())",
            "var _ = sdk.NewSaveRecordInput(\"not a record\")",
        ]
        .iter()
        .enumerate()
        {
            let directory = consumer.join(format!("invalid{index}"));
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(
                directory.join("invalid.go"),
                format!("package invalid\nimport sdk \"example.com/generated-sdk\"\n{invalid}\n"),
            )
            .unwrap();
            let result = run(
                &consumer,
                &tool,
                &["test", "-run=^$", &format!("./invalid{index}")],
            );
            assert!(
                !result.status.success()
                    && String::from_utf8_lossy(&result.stderr).contains("invalid.go:")
            );
        }
        success(
            &consumer,
            &tool,
            &["doc", "example.com/generated-sdk", "Record"],
        );
    }
    if let Some(python) = std::env::var_os("SUSPECT_SPHINX_PYTHON") {
        let result = Command::new(python)
            .current_dir(temp.join("go"))
            .args([
                "-m",
                "sphinx",
                "-W",
                "--keep-going",
                "-b",
                "html",
                "docs",
                "docs/_build/html",
            ])
            .env("SUSPECT_GO_TOOLCHAIN", "go1.23.12")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}\n{}{}",
            temp.display(),
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
    std::fs::remove_dir_all(temp).unwrap();
}

#[test]
#[ignore = "requires native Go 1.23.12/1.27.1"]
fn native_distinct_resource_depth_and_concurrent_contexts_are_bounded() {
    let mut schemas = serde_json::Map::new();
    for i in 0..550 {
        schemas.insert(
            format!("N{i}"),
            json!({"$id":format!("urn:resource-{i}"),"$ref":format!("urn:resource-{}",i+1)}),
        );
    }
    schemas.insert("N550".into(), json!({"$id":"urn:resource-550"}));
    let c = load(api(Value::Object(schemas)), vec![]);
    let program = compile(c, &[root("N0"), root("N550")], Config::default());
    let deep = program
        .roots
        .iter()
        .find(|r| r.source.pointer.ends_with("/N0"))
        .unwrap()
        .target;
    let leaf = program
        .roots
        .iter()
        .find(|r| r.source.pointer.ends_with("/N550"))
        .unwrap()
        .target;
    let mut files = files();
    package(&program, 0, &mut files);
    native(
        files,
        format!(
            r#"package consumer
import("errors";"sync";"testing";sdk "example.com/go-v3/case0")
func TestDistinctResourceDepthAndIsolation(t *testing.T){{
 var failure *sdk.ValidationError
 if err:=sdk.Validate({deep},nil);!errors.As(err,&failure)||failure.Kind!="evaluation_failure"||failure.Source.Pointer!="/components/schemas/N512"{{t.Fatalf("normal-stack depth bound: %#v / %v",failure,err)}}
 if err:=sdk.Validate({leaf},nil);err!=nil{{t.Fatal("failed deep context leaked",err)}}
 var wait sync.WaitGroup;for i:=0;i<32;i++{{wait.Add(1);go func(){{defer wait.Done();if err:=sdk.Validate({leaf},nil);err!=nil{{t.Error(err)}}}}()}};wait.Wait()
}}
"#
        ),
    );
}

const RESOURCE_SDK: &str = r#"package consumer
import("context";"errors";"io";"net/http";"net/http/httptest";"strings";"sync";"testing";sdk "example.com/generated-sdk")
func must[T any](value T,err error)T{if err!=nil{panic(err)};return value}
func TestResourceSDKWireAndMutableEncoding(t *testing.T){
 var seen []string;var mutex sync.Mutex
 server:=httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter,r *http.Request){data:=must(io.ReadAll(r.Body));mutex.Lock();seen=append(seen,r.Method+" "+r.RequestURI+" "+string(data));mutex.Unlock();w.Header().Set("Content-Type","application/json");if strings.Contains(string(data),"bad-response"){_,_=io.WriteString(w,`{"name":"bad-response","value":"fallback-only"}`)}else{_,_=w.Write(data)}}));defer server.Close()
 client:=must(sdk.NewClient(sdk.Credentials{},sdk.ClientOptions{ServerURL:server.URL+"/api/%2e"}));defer client.CloseIdleConnections();ctx:=context.Background()
 record:=sdk.NewRecord("native",sdk.NullableValue[sdk.Value](must(sdk.ParseInteger("9007199254740993"))))
 returned,err:=client.SaveRecordData(ctx,sdk.NewSaveRecordInput(record));if err!=nil||returned.Name!="native"{t.Fatal(returned,err)};if n,ok:=returned.Value.Value.(sdk.Number);!ok||n.String()!="9007199254740993"{t.Fatal(returned.Value)}
 tree:=must(sdk.Codecs.Strict.Decode([]byte(`{"data":"root","children":[{"data":"child"}]}`)));if _,err=client.SaveTreeData(ctx,sdk.NewSaveTreeInput(tree));err!=nil{t.Fatal(err)}
 record.Value.Value="fallback-only";_,err=client.SaveRecord(ctx,sdk.NewSaveRecordInput(record));var failure *sdk.SDKError;if !errors.As(err,&failure)||failure.Kind!="request-validation"{t.Fatal("dynamic mutation escaped validation",err)}
 record.Value.Value=must(sdk.ParseInteger("7"));record.Name="bad-response";_,err=client.SaveRecord(ctx,sdk.NewSaveRecordInput(record));if !errors.As(err,&failure)||failure.Kind!="response-decoding"{t.Fatal("response used standalone fallback",err)}
 var codec *sdk.CodecError;if !errors.As(err,&codec)||codec.Source.Document!="https://physical.test/api.json"||!strings.HasSuffix(codec.Source.Pointer,"/Record/$defs/N/type"){t.Fatal("logical URI replaced physical failure source",err)}
 if len(seen)!=3||!strings.Contains(seen[0],"POST /api/%2e/saveRecord")||!strings.Contains(seen[0],"9007199254740993"){t.Fatal(seen)}
 if _,err=sdk.Codecs.Strict.Decode([]byte(`{"children":[{"unexpected":true}]}`));err==nil{t.Fatal("strict dynamic tree became permissive fallback")}
}
"#;
