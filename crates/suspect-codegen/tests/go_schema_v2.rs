//! Scoped validation and source/codec/native-consumer adoption for Go.
//! Programs are compiled from maintained source fixtures, never target reports.
use serde_json::{Value, json};
use std::{path::Path, process::Command, sync::Arc};
use suspect_codegen::{OutFile, go_http, go_validation};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_schema::{Config, OwnedCompiler, OwnedProgram};
use suspect_source::Uri;

fn contract(document: Value) -> Arc<Contract> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.json");
    std::fs::write(&path, document.to_string()).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(dir.path()).build().unwrap());
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap())
}
fn schema(text: &str, limits: Config) -> OwnedProgram {
    let c = contract(
        json!({"openapi":"3.1.0","info":{"title":"Scoped Go witness","version":"1"},"paths":{},"components":{"schemas":{"Root":serde_json::from_str::<Value>(text).unwrap()}}}),
    );
    let root = c
        .schema_roots()
        .iter()
        .find(|id| id.pointer() == "/components/schemas/Root")
        .unwrap()
        .clone();
    OwnedCompiler::new(limits)
        .compile_v2(c, &[root])
        .unwrap()
        .program()
}
fn toolchains() -> Vec<String> {
    std::env::var("SUSPECT_GO_TOOLCHAIN")
        .map(|t| vec![t])
        .unwrap_or_else(|_| vec!["go1.23.12".into(), "go1.27.1".into()])
}
fn go(dir: &Path, toolchain: &str, args: &[&str]) -> std::process::Output {
    Command::new("go")
        .current_dir(dir)
        .args(args)
        .env("GOWORK", "off")
        .env("GOTOOLCHAIN", toolchain)
        .output()
        .unwrap()
}
fn checked(dir: &Path, toolchain: &str, args: &[&str]) {
    let output = go(dir, toolchain, args);
    assert!(
        output.status.success(),
        "{} {toolchain} {args:?}\n{}{}",
        dir.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    eprintln!(
        "{toolchain}: {}",
        String::from_utf8_lossy(&output.stdout).trim()
    );
}
fn q(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}

#[test]
fn ordinary_media_examples_have_one_entry_binding_and_rich_recipes_remain_explicit() {
    let mut paths = serde_json::Map::new();
    for (name, content) in [
        (
            "json",
            json!({"application/json":{"schema":{"type":"integer"},"example":7}}),
        ),
        (
            "text",
            json!({"text/plain":{"schema":{"type":"string"},"example":"native"}}),
        ),
        (
            "choice",
            json!({"application/json":{"schema":{"type":"integer"},"example":9},"text/plain":{"schema":{"type":"string"},"example":"other"}}),
        ),
        ("bytes", json!({"application/octet-stream":{}})),
        (
            "parts",
            json!({"multipart/form-data":{"schema":{"type":"object","required":["file","name"],"properties":{"file":{},"name":{"type":"string","examples":["native"]}},"additionalProperties":false}}}),
        ),
    ] {
        paths.insert(format!("/{name}"),json!({"post":{"operationId":name,"requestBody":{"required":true,"content":content},"responses":{"204":{}}}}));
    }
    let c = contract(
        json!({"openapi":"3.2.0","info":{"title":"Body binding witnesses","version":"1"},"paths":paths}),
    );
    let selected = c
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    let plan = go_http::plan_http(c, &selected, Default::default()).unwrap();
    let files = plan.render();
    let docs: Value = serde_json::from_str(
        &files
            .iter()
            .find(|f| f.path == "go/docs/source-bindings.json")
            .unwrap()
            .content,
    )
    .unwrap();
    for name in ["json", "text", "choice"] {
        let op = plan
            .operations()
            .iter()
            .find(|o| o.operation_id == name)
            .unwrap();
        let record = docs["examples"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["method"] == op.method_name)
            .unwrap();
        let bindings = record["bindings"].as_array().unwrap();
        assert_eq!(bindings.len(), 1, "{name}: {bindings:?}");
        let binding = &bindings[0];
        assert_eq!(binding["member"], "Body");
        let index = binding["entry"].as_u64().unwrap() as usize;
        let examples = plan
            .examples()
            .operations()
            .iter()
            .find(|e| e.source == op.source)
            .unwrap();
        let entry = &examples.entries[index];
        assert_eq!(binding["container"]["pointer"], entry.container.pointer());
        assert_eq!(
            binding["container"]["document"],
            entry.container.document().as_str()
        );
        assert_eq!(binding["schema"]["pointer"], entry.schema.pointer());
        assert_eq!(binding["mediaType"], entry.media_type);
    }
    for name in ["bytes", "parts"] {
        let op = plan
            .operations()
            .iter()
            .find(|o| o.operation_id == name)
            .unwrap();
        let record = docs["examples"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["method"] == op.method_name)
            .unwrap();
        let bindings = record["bindings"].as_array().unwrap();
        assert_eq!(bindings.iter().filter(|v| v["member"] == "Body").count(), 1);
        assert!(
            bindings
                .iter()
                .any(|v| v["construction"] == "explicit-native-byte-recipe"
                    && v.get("entry").is_none()),
            "{name}: {bindings:?}"
        );
        if name == "parts" {
            assert!(bindings.iter().any(
                |v| v["construction"] == "native-part-constructor" && v.get("schema").is_some()
            ));
        }
    }
}

fn fixtures() -> Value {
    serde_json::from_str(include_str!(
        "../../suspect-schema/tests/fixtures/owned-applicators-v2.json"
    ))
    .unwrap()
}
fn config(value: &Value) -> Config {
    let mut c = Config::default();
    for (key, target) in [
        ("maxNumberBytes", &mut c.max_number_bytes),
        ("maxEvaluationSteps", &mut c.max_evaluation_steps),
        ("maxEqualitySteps", &mut c.max_equality_steps),
        ("maxDepth", &mut c.max_depth),
        ("maxErrors", &mut c.max_errors),
    ] {
        if let Some(n) = value[key].as_u64() {
            *target = n as usize
        }
    }
    c
}

#[test]
#[ignore = "requires native Go 1.23.12/1.27.1"]
fn scoped_32_source_vectors_execute_in_installed_consumers() {
    let fixture = fixtures();
    let mut cases = fixture["cases"].as_array().unwrap().clone();
    assert_eq!(cases.len(), 32);
    cases.extend([
        json!({"id":"nfa-exact-merge-boundary","schemaJson":"{\"patternProperties\":{\"^x$\":true},\"additionalProperties\":false}","instanceJson":"{\"x\":1}","limits":{"maxEvaluationSteps":33},"expected":"EvaluationFailure","source":"/components/schemas/Root/patternProperties"}),
        json!({"id":"nfa-exact-complete-boundary","schemaJson":"{\"patternProperties\":{\"^x$\":true},\"additionalProperties\":false}","instanceJson":"{\"x\":1}","limits":{"maxEvaluationSteps":34},"expected":"Valid"}),
        json!({"id":"report-cap-does-not-suppress-equality-failure","schemaJson":"{\"allOf\":[false,{\"propertyNames\":{\"const\":\"x\"}}],\"unevaluatedProperties\":true}","instanceJson":"{\"x\":1}","limits":{"maxErrors":1,"maxEqualitySteps":0},"expected":"EvaluationFailure","source":"/components/schemas/Root/allOf/1/propertyNames/const","instancePath":"/x"}),
        json!({"id":"key-string-has-fresh-instance-identity","schemaJson":"{\"propertyNames\":{\"$ref\":\"#/components/schemas/Root\"},\"unevaluatedProperties\":true}","instanceJson":"{\"x\":[]}","expected":"Valid"}),
        json!({"id":"decoded-name-pointer","schemaJson":"{\"propertyNames\":{\"const\":\"a/b~雪\"},\"unevaluatedProperties\":true}","instanceJson":"{\"a/b~雪!\":null}","expected":"Invalid","source":"/components/schemas/Root/propertyNames/const","instancePath":"/a~1b~0雪!"}),
        json!({"id":"decoded-unicode-order-no-normalization","schemaJson":"{\"propertyNames\":{\"maxLength\":1},\"unevaluatedProperties\":true}","instanceJson":"{\"é\":1,\"é\":2,\"b\":3,\"aa\":4}","expected":"Invalid","source":"/components/schemas/Root/propertyNames/maxLength","instancePath":"/aa","findings":["/aa","/é"]}),
        json!({"id":"implicit-contains-minimum-is-not-a-number-operand","schemaJson":"{\"contains\":true,\"unevaluatedItems\":false}","instanceJson":"[1]","limits":{"maxNumberBytes":0},"expected":"Valid"}),
        json!({"id":"equality-pending-work-is-shared","schemaJson":"{\"if\":{\"const\":{\"x\":[1,2]}},\"then\":true,\"unevaluatedProperties\":true}","instanceJson":"{\"x\":[1,2]}","limits":{"maxEqualitySteps":2},"expected":"EvaluationFailure","source":"/components/schemas/Root/if/const"}),
        json!({"id":"report-cap-one","schemaJson":"{\"dependentRequired\":{\"seed\":[\"a\",\"b\"]}}","instanceJson":"{\"seed\":null}","limits":{"maxErrors":1},"expected":"Invalid","source":"/components/schemas/Root/dependentRequired/seed","findings":[""]}),
        json!({"id":"report-zero-means-unlimited","schemaJson":"{\"dependentRequired\":{\"seed\":[\"a\",\"b\"]}}","instanceJson":"{\"seed\":null}","limits":{"maxErrors":0},"expected":"Invalid","source":"/components/schemas/Root/dependentRequired/seed","findings":["",""]}),
    ]);
    let root = tempfile::Builder::new()
        .prefix("suspect-go-schema-v2-")
        .tempdir()
        .unwrap()
        .keep();
    let mut files = vec![OutFile {
        path: "sdk/go.mod".into(),
        content: "module example.com/scoped-validation\n\ngo 1.23.0\n".into(),
    }];
    let mut native = String::from("package consumer\nimport(\"errors\";\"testing\"\n");
    let mut functions = String::new();
    for (i, case) in cases.iter().enumerate() {
        let program = schema(
            case["schemaJson"].as_str().unwrap(),
            config(&case["limits"]),
        );
        assert_eq!(program.version, OwnedProgram::V2_VERSION);
        let mut generated = go_validation::emit(&program).unwrap();
        generated.extend(suspect_codegen::go_json::emit());
        for file in generated {
            if file.path == "go/go.mod" {
                continue;
            };
            files.push(OutFile {
                path: format!("sdk/case{i}/{}", file.path.strip_prefix("go/").unwrap()),
                content: file.content,
            });
        }
        native.push_str(&format!("s{i} \"example.com/scoped-validation/case{i}\"\n"));
        let expected = case["expected"].as_str().unwrap();
        let wanted = if expected == "Invalid" {
            "invalid"
        } else {
            "evaluation_failure"
        };
        let source = case["source"].as_str().unwrap_or("");
        functions.push_str(&format!("func TestCase{i}(t *testing.T){{value,err:=s{i}.Parse([]byte({}),s{i}.DefaultLimits());if err!=nil{{t.Fatal(err)}};err=s{i}.Validate({},value);",q(case["instanceJson"].as_str().unwrap()),program.roots[0].target));
        if expected == "Valid" {
            functions.push_str(&format!(
                "if err!=nil{{t.Fatalf(\"{}: %v\",err)}}",
                case["id"].as_str().unwrap()
            ));
        } else {
            let path = case["instancePath"].as_str().unwrap_or(match i {
                8 | 9 => "/0",
                11 => "/1",
                12 | 14 => "/x",
                15 => "/long",
                16 => "/ok",
                17 | 18 | 22 | 25 => "/a",
                20 => "/inner",
                27 => "/2",
                _ => "",
            });
            functions.push_str(&format!("var failure *s{i}.ValidationError;if !errors.As(err,&failure)||failure.Kind!={}||failure.Source.Pointer!={}||failure.Source.Document!={}||failure.InstancePath!={}{{t.Fatalf(\"{}: %#v / %v\",failure,err)}};",q(wanted),q(source),q(&program.roots[0].source.document),q(path),case["id"].as_str().unwrap()));
            if let Some(findings) = case["findings"].as_array() {
                functions.push_str(&format!(
                    "if len(failure.Findings)!={}{{t.Fatal(failure.Findings)}};",
                    findings.len()
                ));
                for (index, path) in findings.iter().enumerate() {
                    functions.push_str(&format!("if failure.Findings[{index}].InstancePath!={}{{t.Fatal(failure.Findings)}};",q(path.as_str().unwrap())));
                }
            }
        }
        functions.push_str("}\n");
    }
    native.push_str(")\n");
    native.push_str(&functions);
    files.push(OutFile{path:"consumer/go.mod".into(),content:"module example.com/scoped-consumer\n\ngo 1.23.0\nrequire example.com/scoped-validation v0.0.0\nreplace example.com/scoped-validation => ../sdk\n".into()});
    files.push(OutFile {
        path: "consumer/scoped_test.go".into(),
        content: native,
    });
    suspect_codegen::write_files(&files, &root).unwrap();
    for toolchain in toolchains() {
        checked(
            &root.join("consumer"),
            &toolchain,
            &["test", "-count=1", "-timeout=60s", "-v", "."],
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn checked_program_fences_and_base_v1_program_identity() {
    let program = schema(r#"{"if":true,"then":false}"#, Config::default());
    assert!(go_validation::emit(&program).is_ok());
    let mut wrong = program.clone();
    wrong.profile = OwnedProgram::V1_PROFILE;
    assert!(go_validation::emit(&wrong).is_err());
    let mut wrong = program.clone();
    wrong.version = "future.invalid";
    assert!(go_validation::emit(&wrong).is_err());
    let mut wrong = program.clone();
    wrong.version = OwnedProgram::V1_VERSION;
    wrong.profile = OwnedProgram::V1_PROFILE;
    let refusal = go_validation::emit(&wrong).unwrap_err();
    assert!(refusal.source.is_some());
    let mut wrong = program.clone();
    for node in &mut wrong.nodes {
        for check in &mut node.checks {
            if let suspect_schema::ProgramInstruction::If { condition, .. } = &mut check.instruction
            {
                *condition = usize::MAX
            }
        }
    }
    assert!(go_validation::emit(&wrong).is_err());
    let c = contract(
        json!({"openapi":"3.1.0","info":{"title":"Base identity","version":"1"},"paths":{},"components":{"schemas":{"Root":{"type":"string","minLength":1}}}}),
    );
    let roots = c.schema_roots();
    let first = OwnedCompiler::new(Config::default())
        .compile(c.clone(), roots)
        .unwrap()
        .program();
    let second = OwnedCompiler::new(Config::default())
        .compile_v2(c.clone(), roots)
        .unwrap()
        .program();
    assert_eq!(first.version, OwnedProgram::V1_VERSION);
    assert_eq!(
        serde_json::to_value(first).unwrap(),
        serde_json::to_value(second).unwrap()
    );
    let codecs =
        suspect_codegen::go_codecs::plan_codecs(c.clone(), roots, Default::default()).unwrap();
    assert_eq!(
        codecs.validation_program().version,
        OwnedProgram::V1_VERSION
    );
}

fn sdk_document() -> Value {
    json!({"openapi":"3.1.0","info":{"title":"Scoped native API","version":"1"},"servers":[{"url":"https://example.test/v1"}],"paths":{
        "/record":{"post":{"operationId":"saveRecord","requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Record"},"example":{"name":"native","mode":"business","amount":serde_json::from_str::<Value>("9007199254740993.01").unwrap(),"nullable":null,"billing":"b","x-count":2}}}},"responses":{"200":{"description":"record","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Record"}}}}}}},
        "/sequence":{"post":{"operationId":"saveSequence","requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Sequence"},"example":["label",1,2]}}},"responses":{"200":{"description":"sequence","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Sequence"}}}}}}},
        "/choice":{"post":{"operationId":"saveChoice","requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Choice"},"example":{"a":"yes"}}}},"responses":{"200":{"description":"choice","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Choice"}}}}}}},
        "/relative":{"get":{"operationId":"relative","servers":[{"url":"../sdk"}],"responses":{"204":{"description":"document base witness"}}}}
    },"components":{"schemas":{
        "Record":{"type":"object","required":["name","mode","amount","nullable"],"properties":{"name":{"type":"string","minLength":1},"mode":{"type":"string","enum":["business","personal"]},"amount":{"type":"number","multipleOf":0.01},"nullable":{"type":["string","null"]},"billing":{"type":"string"},"card":{"type":["string","null"]},"next":{"$ref":"#/components/schemas/Record"}},"if":{"properties":{"mode":{"const":"business"}},"required":["mode"]},"then":{"required":["billing"]},"dependentRequired":{"card":["billing"]},"dependentSchemas":{"billing":{"properties":{"billing":{"minLength":1}}}},"patternProperties":{"^x-":{"type":"number","minimum":0},"-count$":{"type":"integer"}},"additionalProperties":false,"propertyNames":{"minLength":1},"unevaluatedProperties":false},
        "Sequence":{"type":"array","prefixItems":[{"type":"string"}],"contains":{"type":"integer","minimum":1},"minContains":1,"maxContains":2,"unevaluatedItems":false},
        "Choice":{"anyOf":[{"type":"object","properties":{"a":{"type":"string"}},"required":["a"]},{"type":"object","properties":{"b":{"type":"integer"}},"required":["b"]}],"unevaluatedProperties":false}
    }}})
}

#[test]
#[ignore = "requires native Go 1.23.12/1.27.1 and optional Sphinx documentation tools"]
fn scoped_models_codecs_and_sdk_operations_preserve_native_data() {
    let c = contract(sdk_document());
    let selected = c
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let plan = go_http::plan_http(c, &selected, Default::default()).unwrap();
    assert_eq!(
        plan.codecs().validation_program().version,
        OwnedProgram::V2_VERSION
    );
    let declarations = plan.codecs().models().fields();
    for name in ["amount", "mode", "name", "nullable", "billing", "card"] {
        assert!(
            declarations
                .iter()
                .any(|f| f.model == "Record" && f.wire == name),
            "named field lost: {name}"
        );
    }
    let output = plan.render();
    let metadata: Value = serde_json::from_str(
        &output
            .iter()
            .find(|f| f.path == "go/codec-plan.json")
            .unwrap()
            .content,
    )
    .unwrap();
    assert_eq!(metadata["models"]["Record"]["extras"]["name"], "Value");
    let bindings: Value = serde_json::from_str(
        &output
            .iter()
            .find(|f| f.path == "go/docs/source-bindings.json")
            .unwrap()
            .content,
    )
    .unwrap();
    assert_eq!(
        bindings["examples"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["available"] == true)
            .count(),
        4,
        "scoped examples must use v2 admission: {bindings}"
    );
    let root = tempfile::Builder::new()
        .prefix("suspect-go-scoped-sdk-")
        .tempdir()
        .unwrap()
        .keep();
    suspect_codegen::write_files(&output, &root).unwrap();
    let consumer = root.join("consumer");
    std::fs::create_dir(&consumer).unwrap();
    std::fs::write(consumer.join("go.mod"),"module example.com/scoped-sdk-consumer\n\ngo 1.23.0\nrequire example.com/generated-sdk v0.0.0\nreplace example.com/generated-sdk => ../go\n").unwrap();
    std::fs::write(consumer.join("scoped_test.go"), SCOPED_SDK).unwrap();
    for toolchain in toolchains() {
        checked(&root.join("go"), &toolchain, &["test", "./..."]);
        checked(
            &root.join("go"),
            &toolchain,
            &["run", "./examples/validated"],
        );
        checked(
            &consumer,
            &toolchain,
            &["test", "-count=1", "-timeout=60s", "-v", "."],
        );
        for (i,text) in [
            "var _ = sdk.NewRecord()",
            "var _ = sdk.NewRecord(1.5,sdk.RecordModeBusiness,\"name\",sdk.NullableNull[string]())",
            "var _ = sdk.NewRecord(sdk.Number{},sdk.RecordModeBusiness,\"name\",nil)",
            "var _ = sdk.NewSaveRecordInput(\"not the named record\")",
            "func invalid(){v:=sdk.NewRecord(sdk.Number{},sdk.RecordModePersonal,\"name\",sdk.NullableNull[string]());v.Card=sdk.OptionalSome(\"wrong absence/null wrapper\")}",
        ].iter().enumerate(){let path=consumer.join(format!("invalid{i}"));std::fs::create_dir_all(&path).unwrap();std::fs::write(path.join("invalid.go"),format!("package invalid\nimport sdk \"example.com/generated-sdk\"\n{text}\n")).unwrap();let output=go(&consumer,&toolchain,&["test","-run=^$",&format!("./invalid{i}")]);assert!(!output.status.success()&&String::from_utf8_lossy(&output.stderr).contains("invalid.go:"),"invalid native consumer {i} did not fail at its boundary");}
        let documentation = go(
            &consumer,
            &toolchain,
            &["doc", "-all", "example.com/generated-sdk"],
        );
        assert!(documentation.status.success());
        let docs = String::from_utf8(documentation.stdout).unwrap();
        for symbol in [
            "type Record struct",
            "func NewRecord(",
            "type ValidationFinding struct",
        ] {
            assert!(
                docs.contains(symbol),
                "{toolchain}: missing public docs {symbol}"
            );
        }
        checked(
            &consumer,
            &toolchain,
            &["doc", "example.com/generated-sdk", "Record.SetExtra"],
        );
    }
    if let Some(python) = std::env::var_os("SUSPECT_SPHINX_PYTHON") {
        let output = Command::new(python)
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
            .current_dir(root.join("go"))
            .env("SUSPECT_GO_TOOLCHAIN", "go1.23.12")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}{}",
            root.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

const SCOPED_SDK: &str = r#"package consumer
import("bytes";"context";"encoding/json";"errors";"io";"net/http";"net/http/httptest";"reflect";"strings";"testing";sdk "example.com/generated-sdk")
func must[T any](value T,err error)T{if err!=nil{panic(err)};return value}
func invalid(t *testing.T,err error){t.Helper();var failure *sdk.CodecError;if !errors.As(err,&failure)||failure.Kind!="invalid"{t.Fatalf("wanted checked schema rejection: %v",err)}}
func record()sdk.Record{return sdk.NewRecord(must(sdk.ParseNumber("9007199254740993.01")),sdk.RecordModeBusiness,"native",sdk.NullableNull[string]())}
func TestNamedFieldsPatternExtrasAndMutableEncode(t *testing.T){
 value:=record();value.Billing=sdk.OptionalSome("billing");value.Card=sdk.PresenceNull[string]();if err:=value.SetExtra("x-count",must(sdk.ParseInteger("2")));err!=nil{t.Fatal(err)}
 encoded,err:=sdk.Codecs.Record.Encode(value);if err!=nil{t.Fatal(err)};if !bytes.Contains(encoded,[]byte("9007199254740993.01"))||!bytes.Contains(encoded,[]byte(`"card":null`))||!bytes.Contains(encoded,[]byte(`"nullable":null`))||!bytes.Contains(encoded,[]byte(`"x-count":2`)){t.Fatalf("wire distinctions lost: %s",encoded)}
 decoded,err:=sdk.Codecs.Record.Decode(encoded);if err!=nil{t.Fatal(err)};if decoded.Amount.String()!="9007199254740993.01"||!decoded.Card.IsSet||!decoded.Card.Null||decoded.Nullable.IsValue{t.Fatal(decoded)};extra,ok:=decoded.Extra()["x-count"];if !ok{t.Fatal("pattern extra was dropped")};if n,ok:=extra.(sdk.Number);!ok||n.String()!="2"{t.Fatal(extra)}
 if err=value.SetExtra("name","shadow");err==nil{t.Fatal("declared name shadowed")}
 if err=value.SetExtra("x-count",must(sdk.ParseNumber("2.5")));err!=nil{t.Fatal(err)};_,err=sdk.Codecs.Record.Encode(value);invalid(t,err)
 if err=value.SetExtra("x-count",must(sdk.ParseInteger("2")));err!=nil{t.Fatal(err)};value.Billing=sdk.OptionalAbsent[string]();_,err=sdk.Codecs.Record.EncodeValue(value);invalid(t,err)
 value.Mode=sdk.RecordModePersonal;_,err=sdk.Codecs.Record.Encode(value);invalid(t,err) // null card still triggers dependentRequired
 value.Card=sdk.PresenceMissing[string]();if _,err=sdk.Codecs.Record.Encode(value);err!=nil{t.Fatal(err)}
 value.Billing=sdk.OptionalSome("");_,err=sdk.Codecs.Record.Encode(value);invalid(t,err) // dependentSchemas validates the whole object
 value.Billing=sdk.OptionalSome("ok");if err=value.SetExtra("unmatched",nil);err!=nil{t.Fatal(err)};_,err=sdk.Codecs.Record.Encode(value);invalid(t,err)
 for _,text:=range []string{`{"name":"n","mode":"personal","amount":1,"nullable":null,"x-count":2.5}`,`{"name":"n","mode":"business","amount":1,"nullable":null}`,`{"name":"n","mode":"personal","amount":1.001,"nullable":null}`,`{"name":"n","mode":"personal","amount":1,"nullable":null,"outside":1}`} {_,err=sdk.Codecs.Record.Decode([]byte(text));invalid(t,err)}
}
func TestScopedJSONCarriersAndInstanceIsolation(t *testing.T){
 sequence:=sdk.Sequence{"prefix",must(sdk.ParseInteger("1")),must(sdk.ParseInteger("2"))};wire,err:=sdk.Codecs.Sequence.Encode(sequence);if err!=nil{t.Fatal(err)};decoded,err:=sdk.Codecs.Sequence.Decode(wire);if err!=nil||len(decoded)!=3{t.Fatal(decoded,err)}
 sequence=append(sequence,false);_,err=sdk.Codecs.Sequence.Encode(sequence);invalid(t,err)
 choice:=must(sdk.Codecs.Choice.Decode([]byte(`{"a":"yes","b":2}`)));wire,err=sdk.Codecs.Choice.Encode(choice);if err!=nil||string(wire)!=`{"a":"yes","b":2}`{t.Fatal(string(wire),err)}
 choice.(map[string]sdk.Value)["unmarked"]=true;_,err=sdk.Codecs.Choice.Encode(choice);invalid(t,err)
 for i:=0;i<3;i++{if _,err=sdk.Codecs.Choice.Decode([]byte(`{"a":"yes"}`));err!=nil{t.Fatal("failed prior call leaked scope",err)}}
 var rec sdk.Record;if err=json.Unmarshal([]byte(`{"name":"n","mode":"personal","amount":1,"nullable":null,"x-note":2}`),&rec);err!=nil{t.Fatal(err)};if _,ok:=rec.Extra()["x-note"];!ok{t.Fatal("encoding/json stripped pattern field")}
}
func TestRealSDKBoundaries(t *testing.T){
 var seen []string
 server:=httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter,r *http.Request){data,err:=io.ReadAll(r.Body);if err!=nil{t.Fatal(err)};seen=append(seen,string(data));w.Header().Set("Content-Type","application/json");_,_=w.Write(data)}));defer server.Close()
 client:=must(sdk.NewClient(sdk.Credentials{},sdk.ClientOptions{ServerURL:server.URL}));defer client.CloseIdleConnections();ctx:=context.Background()
 value:=record();value.Billing=sdk.OptionalSome("bill");_ = value.SetExtra("x-count",must(sdk.ParseInteger("2")))
 response,err:=client.SaveRecordData(ctx,sdk.NewSaveRecordInput(value));if err!=nil||response.Name!="native"||response.Amount.String()!=value.Amount.String(){t.Fatal(response,err)};if _,exists:=response.Extra()["x-count"];!exists{t.Fatal("response pattern extra lost")}
 list:=sdk.Sequence{"first",must(sdk.ParseNumber("1"))};if response,err:=client.SaveSequenceData(ctx,sdk.NewSaveSequenceInput(list));err!=nil||!reflect.DeepEqual(response,list){t.Fatal(response,err)}
 any:=must(sdk.Codecs.Choice.Decode([]byte(`{"a":"yes"}`)));if _,err=client.SaveChoiceData(ctx,sdk.NewSaveChoiceInput(any));err!=nil{t.Fatal(err)}
 before:=len(seen);value.Billing=sdk.OptionalAbsent[string]();_,err=client.SaveRecord(ctx,sdk.NewSaveRecordInput(value));var failure *sdk.SDKError;if !errors.As(err,&failure)||failure.Kind!="request-validation"||len(seen)!=before{t.Fatal("invalid mutable record reached transport",err)}
 if !strings.Contains(seen[0],"9007199254740993.01"){t.Fatal("numeric token rounded",seen)}
}
type doer func(*http.Request)(*http.Response,error)
func(f doer)Do(r *http.Request)(*http.Response,error){return f(r)}
func TestDocumentBaseDescriptor(t *testing.T){client:=must(sdk.NewClient(sdk.Credentials{},sdk.ClientOptions{DocumentURL:"https://example.test/specs/api.json",Transport:doer(func(r *http.Request)(*http.Response,error){if r.URL.String()!="https://example.test/sdk/relative"{t.Fatal(r.URL)};return &http.Response{StatusCode:204,Header:make(http.Header),Body:io.NopCloser(strings.NewReader(""))},nil})}));if _,err:=client.Relative(context.Background());err!=nil{t.Fatal(err)}}
"#;

#[test]
#[ignore = "requires native Go 1.23.12/1.27.1"]
fn scoped_native_recursion_depth_and_call_isolation() {
    let program = schema(
        r##"{"type":"object","properties":{"next":{"$ref":"#/components/schemas/Root"},"left":{"$ref":"#/components/schemas/Root"},"right":{"$ref":"#/components/schemas/Root"}},"unevaluatedProperties":false}"##,
        Config::default(),
    );
    let root = tempfile::Builder::new()
        .prefix("suspect-go-scoped-depth-")
        .tempdir()
        .unwrap()
        .keep();
    let mut files = go_validation::emit(&program).unwrap();
    files.extend(suspect_codegen::go_json::emit());
    suspect_codegen::write_files(&files, &root).unwrap();
    let consumer = root.join("consumer");
    std::fs::create_dir(&consumer).unwrap();
    std::fs::write(consumer.join("go.mod"),"module example.com/scoped-depth-consumer\n\ngo 1.23.0\nrequire example.com/generated-json v0.0.0\nreplace example.com/generated-json => ../go\n").unwrap();
    std::fs::write(consumer.join("depth_test.go"),format!(r#"package consumer
import("errors";"strings";"sync";"testing";sdk "example.com/generated-json")
func TestDepthCycleAndSharedAcyclicInstances(t *testing.T){{
 var deep sdk.Value=map[string]sdk.Value{{}};for i:=0;i<700;i++{{deep=map[string]sdk.Value{{"next":deep}}}}
 err:=sdk.Validate({target},deep);var failure *sdk.ValidationError;if !errors.As(err,&failure)||failure.Kind!="evaluation_failure"||!failure.ResourceLimited()||strings.Count(failure.InstancePath,"/next")!=256{{t.Fatalf("normal-stack depth refusal: %#v",failure)}}
 cycle:=map[string]sdk.Value{{}};cycle["next"]=cycle;err=sdk.Validate({target},cycle);if !errors.As(err,&failure)||failure.Kind!="evaluation_failure"||failure.InstancePath!="/next"{{t.Fatalf("active identity: %#v",failure)}}
 shared:=map[string]sdk.Value{{}};if err=sdk.Validate({target},map[string]sdk.Value{{"left":shared,"right":shared}});err!=nil{{t.Fatal("acyclic alias rejected",err)}}
 if err=sdk.Validate({target},map[string]sdk.Value{{}});err!=nil{{t.Fatal("previous failure leaked state",err)}}
}}
func TestConcurrentScopesArePerCall(t *testing.T){{var wait sync.WaitGroup;for i:=0;i<64;i++{{wait.Add(1);go func(){{defer wait.Done();if err:=sdk.Validate({target},map[string]sdk.Value{{"next":map[string]sdk.Value{{}}}});err!=nil{{t.Error(err)}};var failure *sdk.ValidationError;if err:=sdk.Validate({target},map[string]sdk.Value{{"unexpected":true}});!errors.As(err,&failure)||failure.Kind!="invalid"{{t.Error(err)}}}}()}};wait.Wait()}}
"#,target=program.roots[0].target)).unwrap();
    for toolchain in toolchains() {
        checked(
            &consumer,
            &toolchain,
            &["test", "-race", "-count=1", "-timeout=60s", "-v", "."],
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "requires native Go 1.23.12/1.27.1"]
fn native_physical_document_base_redirect_and_encoded_path_witness() {
    let document = json!({"openapi":"3.2.0","$self":"https://logical.example/catalog/api.json","info":{"title":"Physical server base","version":"1"},"servers":[{"url":"../service/%2e/%2Fkeep/"}],"security":[{"oauth":["read"]}],"components":{"securitySchemes":{"oauth":{"type":"oauth2","flows":{"clientCredentials":{"tokenUrl":"../token","scopes":{"read":"Read"}}}}}},"paths":{"/probe":{"get":{"operationId":"probe","responses":{"204":{}}}}}});
    let provided = |document: &Value| {
        let provider = Arc::new(
            suspect_ref::DocumentProvider::new([suspect_ref::ProvidedDocument::new(
                Uri::parse("https://requested.example/original.json").unwrap(),
                Uri::parse("https://physical.example/specs/api.json").unwrap(),
                serde_json::to_vec(document).unwrap(),
            )
            .unwrap()])
            .unwrap(),
        );
        let workspace = Arc::new(
            WorkspaceBuilder::new()
                .allowed_documents(provider.logical_uris())
                .document_provider(provider)
                .build()
                .unwrap(),
        );
        Arc::new(
            Contract::from_workspace(
                &workspace,
                &Uri::parse("https://requested.example/original.json").unwrap(),
            )
            .unwrap(),
        )
    };
    let c = provided(&document);
    let selected = c
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    let plan = go_http::plan_http(c, &selected, Default::default()).unwrap();
    let server = &plan.protocol().operations()[0].servers().candidates()[0];
    assert_eq!(
        server.document_base().source().document().as_str(),
        "https://physical.example/specs/api.json"
    );
    assert_eq!(
        server
            .source()
            .unwrap()
            .terminal_resource()
            .unwrap()
            .base_uri(),
        "https://logical.example/catalog/api.json"
    );
    assert!(
        plan.protocol()
            .capabilities()
            .supports(suspect_codegen::http_protocol::Capability::DocumentRelativeServers)
    );
    let root = tempfile::Builder::new()
        .prefix("suspect-go-physical-base-")
        .tempdir()
        .unwrap()
        .keep();
    suspect_codegen::write_files(&plan.render(), &root).unwrap();
    let consumer = root.join("consumer");
    std::fs::create_dir(&consumer).unwrap();
    std::fs::write(consumer.join("go.mod"),"module example.com/base-consumer\n\ngo 1.23.0\nrequire example.com/generated-sdk v0.0.0\nreplace example.com/generated-sdk => ../go\n").unwrap();
    std::fs::write(consumer.join("base_test.go"),r#"package consumer
import("context";"io";"net/http";"strings";"testing";sdk "example.com/generated-sdk")
type doer func(*http.Request)(*http.Response,error)
func(f doer)Do(r *http.Request)(*http.Response,error){return f(r)}
func TestPhysicalRetrievalAndExplicitOverride(t *testing.T){for _,item:=range []struct{override,base string}{{"","https://physical.example/service/%2e/%2Fkeep/"},{"https://override.example/a/b/api.json","https://override.example/a/service/%2e/%2Fkeep/"}}{
 hooks,calls:=0,0
 credentials:=sdk.Credentials{}.WithHook("oauth",func(ctx context.Context,request sdk.CredentialRequest)(sdk.Authorization,error){hooks++;if request.ServerURL!=item.base||request.Requirement.URLBase!="effective-server"||request.Requirement.Flows[0].URLBase!="effective-server"||request.Requirement.Flows[0].TokenURL!="../token"||request.Requirement.Scheme.Definition.Document!="https://physical.example/specs/api.json"{t.Fatalf("logical address leaked into native URL metadata: %#v",request)};return sdk.Authorization{Scheme:"Bearer",Value:"caller-token"},nil})
 client,err:=sdk.NewClient(credentials,sdk.ClientOptions{DocumentURL:item.override,Transport:doer(func(request *http.Request)(*http.Response,error){calls++;if request.URL.String()!=item.base+"probe"||request.Header.Get("Authorization")!="Bearer caller-token"{t.Fatal(request.URL,request.Header)};return &http.Response{StatusCode:204,Header:make(http.Header),Body:io.NopCloser(strings.NewReader(""))},nil})});if err!=nil{t.Fatal(err)};if _,err=client.Probe(context.Background());err!=nil{t.Fatal(err)};if hooks!=1||calls!=1{t.Fatal(hooks,calls)}
}}
"#).unwrap();
    for toolchain in toolchains() {
        checked(&consumer, &toolchain, &["test", "-count=1", "-v", "."]);
    }
    let mut resource = document.clone();
    resource["paths"]["/probe"]["get"]["responses"] = json!({"200":{"content":{"application/json":{"schema":{"$id":"https://logical.example/schema","type":"string"}}}}});
    let c = provided(&resource);
    let selected = c
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    let resources = go_http::plan_http(c.clone(), &selected, Default::default()).unwrap();
    assert_eq!(
        resources.codecs().validation_program().version,
        OwnedProgram::V3_VERSION
    );
    assert!(
        OwnedCompiler::new(Config::default())
            .compile_v2(c, resources.protocol().codec_roots())
            .is_err(),
        "the frozen v2 compiler must still refuse resource semantics after Go v3 promotion"
    );
    std::fs::remove_dir_all(root).unwrap();
}
