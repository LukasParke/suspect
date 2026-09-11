//! Allocated HTTP names and the Go 1.23 consumer's construction/type boundary.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::Arc,
};

use serde_json::{Value, json};
use suspect_codegen::go_http::{
    HttpConfig, HttpPlan, PackageConfig, PlannedOperation, emit_http, plan_http,
};
use suspect_ir::contract::{Contract, ParameterLocation, SourceId};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn load(path: &Path) -> Arc<Contract> {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap())
}

fn contract(document: Value) -> Arc<Contract> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(&path, document.to_string()).unwrap();
    load(&path)
}

fn plan(contract: Arc<Contract>) -> HttpPlan {
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    plan_http(contract, &selected, HttpConfig::default()).unwrap()
}

fn operation<'a>(plan: &'a HttpPlan, id: &str) -> &'a PlannedOperation {
    plan.operations()
        .iter()
        .find(|op| op.operation_id == id)
        .unwrap()
}

fn model<'a>(plan: &'a HttpPlan, name: &str) -> &'a str {
    let source = SourceId::new(plan.contract().entry().clone(), Default::default())
        .child("components")
        .child("schemas")
        .child(name);
    &plan.symbols()[&source]
}

fn reference(name: &str) -> Value {
    let pointer = name.replace('~', "~0").replace('/', "~1");
    let fragment = pointer
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect::<String>();
    json!({"$ref":format!("#/components/schemas/{fragment}")})
}

// operationId, response model, bearer scheme. Distinct source names deliberately
// normalize together in each of the method, model and free-function namespaces.
const CASES: &[(&str, &str, &str)] = &[
    ("read-item", "CaseName", "ReadItemInput"),
    ("read_item", "case_name", "readItemInput"),
    ("ReadItem", "CASE_NAME", "NewReadItemInput2"),
    ("1-alpha", "1-alpha", "1-bearer"),
    ("雪", "雪", "雪"),
    ("u96ea", "u96ea", "u96ea"),
    ("café", "café", "café"),
    ("!!!", "!!!", "!!!"),
    ("literalVariant", "Plain", "LiteralVariantStatus200"),
    ("choiceVariant", "Plain", "NewChoiceVariantStatus200"),
    ("newBox", "Plain", "APIResponse"),
    ("closeIdleConnections", "Client", "NewClient"),
    ("jsonSyntax", "Plain", "JSONSyntax"),
    ("codecs", "Plain", "Codecs"),
    ("client", "Plain", "Client"),
];

const QUERIES: &[(&str, bool)] = &[
    ("x", false),
    ("with_x", false),
    ("with_x2", false),
    ("with_body", true),
    ("9-query", false),
    ("雪", false),
    ("tag-name", false),
    ("tag_name", false),
    ("TagName", false),
    ("body", false),
    ("mode", false),
];

fn naming_contract() -> Arc<Contract> {
    let mut schemas = serde_json::Map::new();
    for &(_, name, _) in CASES {
        schemas.insert(name.into(), json!({"type":"string"}));
    }
    // Reserve HTTP type families, a constructor whose input type stays safe,
    // and declarations derived from the model plan rather than GoSymbol alone.
    for name in [
        "ReadItemInput",
        "ReadItemResult",
        "ReadItemApiError",
        "ReadItemStatus200",
        "NewReadItemInput2",
        "NewBox",
    ] {
        schemas.insert(name.into(), json!({"type":"string"}));
    }
    schemas.insert(
        "Box".into(),
        json!({"type":"object","additionalProperties":false}),
    );
    schemas.insert(
        "BoxStatus200".into(),
        json!({"type":"object","additionalProperties":false}),
    );
    schemas.insert(
        "Literal".into(),
        json!({"type":"string","enum":["variantStatus200"]}),
    );
    schemas.insert("VariantStatus200".into(), json!({"type":"object","required":["kind"],"properties":{"kind":{"type":"string","const":"first"}},"additionalProperties":false}));
    schemas.insert("OtherVariant".into(), json!({"type":"object","required":["kind"],"properties":{"kind":{"type":"string","const":"other"}},"additionalProperties":false}));
    schemas.insert(
        "Choice".into(),
        json!({"oneOf":[reference("VariantStatus200"),reference("OtherVariant")]}),
    );
    let inventory = schemas
        .keys()
        .enumerate()
        .map(|(index, name)| (format!("item{index}"), reference(name)))
        .collect::<serde_json::Map<_, _>>();
    schemas.insert(
        "NameInventory".into(),
        json!({"type":"object","properties":inventory,"additionalProperties":false}),
    );

    let mut paths = serde_json::Map::new();
    let mut schemes = serde_json::Map::new();
    for (index, &(id, response, scheme)) in CASES.iter().enumerate() {
        let mut parameters =
            vec![json!({"name":"id","in":"path","required":true,"schema":{"type":"string"}})];
        parameters.extend(QUERIES.iter().map(|(name, required)| json!({"name":name,"in":"query","required":required,"schema":{"type":"string"}})));
        paths.insert(format!("/case{index:02}/{{id}}"), json!({"post":{
            "operationId":id,
            "security":[{(scheme):[]}],
            "parameters":parameters,
            "requestBody":{"required":false,"content":{"application/json":{"schema":reference("NameInventory")}}},
            "responses":{
                "200":{"description":"Accepted","content":{"application/json":{"schema":reference(response)}}},
                "409":{"description":"Rejected","content":{"application/json":{"schema":{"type":"string"}}}}
            }
        }}));
        schemes.insert(scheme.into(), json!({"type":"http","scheme":"bearer"}));
    }
    contract(
        json!({"openapi":"3.1.0","info":{"title":"Go naming","version":"1"},"servers":[{"url":"https://names.example.test/v1"}],"paths":paths,"components":{"schemas":schemas,"securitySchemes":schemes}}),
    )
}

#[test]
fn allocated_names_are_stable_and_retained_in_the_manifest() {
    let contract = naming_contract();
    let plan = plan(contract.clone());
    let mut selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    selected.reverse();
    selected.push(selected[0].clone());
    let reordered = plan_http(contract, &selected, HttpConfig::default()).unwrap();
    let artifacts = |plan: &HttpPlan| {
        plan.render()
            .into_iter()
            .map(|f| (f.path, f.content))
            .collect::<BTreeMap<_, _>>()
    };
    assert_eq!(artifacts(&plan), artifacts(&reordered));

    assert_eq!(operation(&plan, "read-item").method_name, "ReadItem");
    assert_eq!(operation(&plan, "read_item").method_name, "ReadItem2");
    assert_eq!(operation(&plan, "ReadItem").method_name, "ReadItem3");
    assert_eq!(operation(&plan, "1-alpha").method_name, "Op1Alpha");
    assert_eq!(operation(&plan, "雪").method_name, "U96ea");
    assert_eq!(operation(&plan, "u96ea").method_name, "U96ea2");
    let first = operation(&plan, "read-item");
    assert_ne!(first.input_constructor, format!("New{}", first.input_type));
    for scheme in ["Client", "NewClient", "Codecs", "JSONSyntax", "APIResponse"] {
        assert_ne!(plan.credentials()[scheme], scheme);
    }
    let x = first
        .parameters()
        .iter()
        .find(|p| p.field_name == "X")
        .unwrap();
    assert_ne!(x.setter_name.as_deref(), Some("WithX"));
    assert_ne!(
        first.body().unwrap().setter_name.as_deref(),
        Some("WithBody")
    );

    let manifest: Value = serde_json::from_str(&artifacts(&plan)["go/http-manifest.json"]).unwrap();
    for op in plan.operations() {
        let entry = manifest["operations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["operationId"] == op.operation_id)
            .unwrap();
        assert_eq!(entry["inputConstructor"], op.input_constructor);
        assert_eq!(
            entry["body"]["setter"],
            op.body().unwrap().setter_name.as_deref().unwrap()
        );
        for response in op.responses() {
            let entry = entry["responses"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| {
                    entry["status"].as_u64().map(|s| s.to_string()).as_deref()
                        == Some(response.wire().status_key())
                })
                .unwrap();
            assert_eq!(entry["nativeType"], response.type_name);
            assert_eq!(entry["model"], plan.symbols()[response.schema().unwrap()]);
        }
    }
}

#[test]
fn ordinary_m2_operation_names_remain_compatible() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m2/canonical.openapi.yaml");
    let plan = plan(load(&path));
    assert_eq!(plan.credentials()["apiKey"], "ApiKey");
    for (id, stem) in [
        ("createWidget", "CreateWidget"),
        ("updateWidget", "UpdateWidget"),
        ("listWidgets", "ListWidgets"),
        ("getWidget", "GetWidget"),
    ] {
        let op = operation(&plan, id);
        assert_eq!(op.method_name, stem);
        assert_eq!(op.input_type, format!("{stem}Input"));
        assert_eq!(op.input_constructor, format!("New{stem}Input"));
        assert_eq!(op.success_type, format!("{stem}Result"));
        assert_eq!(op.error_variant, format!("{stem}ApiError"));
        for response in op.responses() {
            assert_eq!(
                response.type_name,
                format!("{stem}Status{}", response.wire().status_key())
            );
        }
        for parameter in op.parameters() {
            if let Some(setter) = &parameter.setter_name {
                assert_eq!(*setter, format!("With{}", parameter.field_name));
            }
        }
    }
}

const MODULE: &str = "example.com/go-http-names";

fn install(plan: &HttpPlan) -> (PathBuf, PathBuf) {
    let root = tempfile::Builder::new()
        .prefix("suspect-go-http-names-")
        .tempdir()
        .unwrap()
        .keep();
    let files = emit_http(
        plan,
        &PackageConfig {
            module_path: MODULE.into(),
            ..PackageConfig::default()
        },
    )
    .unwrap();
    suspect_codegen::write_files(&files, &root).unwrap();
    let consumer = root.join("consumer");
    std::fs::create_dir(&consumer).unwrap();
    std::fs::write(consumer.join("go.mod"), format!("module example.com/consumer\n\ngo 1.23.0\nrequire {MODULE} v0.0.0\nreplace {MODULE} => ../go\n")).unwrap();
    (root, consumer)
}

fn go(directory: &Path, args: &[&str]) -> Output {
    Command::new("go")
        .current_dir(directory)
        .args(args)
        .env("GOWORK", "off")
        .env(
            "GOTOOLCHAIN",
            std::env::var_os("SUSPECT_GO_TOOLCHAIN").unwrap_or_else(|| "go1.23.12".into()),
        )
        .output()
        .unwrap()
}

fn succeeds(directory: &Path, args: &[&str]) -> Output {
    let output = go(directory, args);
    assert!(
        output.status.success(),
        "{}: go {}\n{}{}",
        directory.display(),
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn q(text: &str) -> String {
    serde_json::to_string(text).unwrap()
}

#[test]
#[ignore = "requires native Go 1.23"]
fn native_consumer_uses_allocated_names_and_preserves_wire_identities() {
    let plan = plan(naming_contract());
    let (root, consumer) = install(&plan);
    let mut source = format!(
        "package consumer\nimport(\"context\";\"errors\";\"fmt\";\"io\";\"net/http\";\"net/http/httptest\";\"sync/atomic\";\"testing\"; sdk {module})\n",
        module = q(MODULE)
    );
    for (index, &(id, schema, scheme)) in CASES.iter().enumerate() {
        let op = operation(&plan, id);
        let canonical = plan
            .contract()
            .operations()
            .find(|wire| wire.source() == &op.source)
            .unwrap();
        let parameters = canonical.parameters();
        let mut args = Vec::new();
        let mut setters = String::new();
        let mut checks = String::new();
        let mut mode = None;
        for parameter in op.parameters() {
            let wire = parameters
                .iter()
                .find(|p| p.schema().unwrap().id() == parameter.schema())
                .unwrap();
            let wire_name = wire.name().unwrap();
            let value = if wire.location() == Some(ParameterLocation::Path) {
                "a/b 雪!'()*".into()
            } else {
                format!("value:{wire_name} 雪")
            };
            if let Some(setter) = &parameter.setter_name {
                setters.push_str(&format!(".{setter}({})", q(&value)));
                if wire_name == "mode" {
                    mode = Some(setter);
                }
            } else {
                args.push(q(&value));
            }
            if wire.location() == Some(ParameterLocation::Query) {
                if wire_name == "mode" {
                    checks.push_str("if mode!=\"value:mode 雪\" && mode!=\"deny\"{t.Errorf(\"wire mode: %s\",mode)}\n");
                } else {
                    checks.push_str(&format!("if got:=r.URL.Query().Get({name});got!={expected}{{t.Errorf(\"wire parameter %s = %q\",{name},got)}}\n", name=q(wire_name), expected=q(&value)));
                }
            }
        }
        let success = op
            .responses()
            .iter()
            .find(|r| r.status() == suspect_codegen::http_protocol::ResponseStatus::Exact(200))
            .unwrap();
        let failure = op
            .responses()
            .iter()
            .find(|r| r.status() == suspect_codegen::http_protocol::ResponseStatus::Exact(409))
            .unwrap();
        let body_setter = op.body().unwrap().setter_name.as_deref().unwrap();
        source.push_str(&format!(r#"
func TestNames{index}(t *testing.T) {{
 var calls atomic.Int32
 server:=httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter,r *http.Request){{
  calls.Add(1)
  mode:=r.URL.Query().Get("mode")
  if r.Method!="POST" || r.URL.EscapedPath()!="/case{index:02}/a%2Fb%20%E9%9B%AA%21%27%28%29%2A" {{t.Errorf("wire route: %s %s",r.Method,r.URL.EscapedPath())}}
  if r.Header.Get("Authorization")!="Bearer token{index}" {{t.Errorf("wrong source scheme: %q",r.Header.Get("Authorization"))}}
  {checks}
  body,err:=io.ReadAll(r.Body);if err!=nil||string(body)!="{{}}"{{t.Errorf("body: %s %v",body,err)}}
  w.Header().Set("Content-Type","application/json")
  if mode=="deny" {{w.WriteHeader(409);_,_=fmt.Fprint(w,`"denied"`)}} else {{_,_=fmt.Fprint(w,`"accepted"`)}}
 }}));defer server.Close()
 client,err:=sdk.NewClient(sdk.{credential}("token{index}"),sdk.ClientOptions{{ServerURL:server.URL}});if err!=nil{{t.Fatal(err)}};defer client.CloseIdleConnections()
 input:=sdk.{constructor}({args}){setters}.{body_setter}(sdk.New{inventory}())
 var typedInput sdk.{input_type}=input
 reply,err:=client.{method}(context.Background(),typedInput);if err!=nil{{t.Fatal(err)}}
  var typedResult sdk.{result_type}=reply
  accepted,ok:=typedResult.(sdk.{success});if !ok || accepted.Status!=200 || accepted.Data!="accepted" {{t.Fatalf("allocated result: %#v",reply)}}
  var typedData sdk.{response_model}=accepted.Data
  decoded,err:=sdk.Codecs.{response_model}.Decode([]byte(`"accepted"`));if err!=nil||decoded!=typedData{{t.Fatalf("allocated schema: %v %v",decoded,err)}}
 _,err=client.{method}(context.Background(),input.{mode}("deny"))
 var rejected *sdk.{failure};if !errors.As(err,&rejected)||rejected.Status!=409||rejected.Data!="denied"{{t.Fatalf("allocated API error: %v",err)}}
 var typedError sdk.{error_type}=rejected
 if typedError.Error()=="" || calls.Load()!=2 {{t.Fatalf("calls=%d error=%v",calls.Load(),typedError)}}
}}
"#,
            credential=plan.credentials()[scheme], constructor=op.input_constructor, args=args.join(","),
            inventory=model(&plan,"NameInventory"), input_type=op.input_type, method=op.method_name,
            result_type=op.success_type, success=success.type_name, failure=failure.type_name,
            error_type=op.error_variant, mode=mode.unwrap(), response_model=model(&plan,schema),
        ));
    }
    std::fs::write(consumer.join("names_test.go"), source).unwrap();
    succeeds(&consumer, &["test", "./..."]);
    let docs = succeeds(&consumer, &["doc", "-all", MODULE]);
    let docs = String::from_utf8(docs.stdout).unwrap();
    for op in plan.operations() {
        assert!(
            docs.contains(&format!("func {}(", op.input_constructor)),
            "missing constructor docs: {}",
            op.input_constructor
        );
        for response in op.responses() {
            assert!(
                docs.contains(&format!("type {} struct", response.type_name)),
                "missing response docs: {}",
                response.type_name
            );
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "requires native Go 1.23"]
fn native_consumer_uses_model_names_or_source_linked_refusals() {
    // The model planner owns these identities. Its unsupported allocations may
    // be refused, but an accepted HTTP plan must compile without duplicate
    // declarations. This also permits the model planner to add native support.
    for (case, mut schemas) in [
        (
            "runtime constant",
            json!({"j_s_o_n_Syntax":{"type":"string"}}),
        ),
        (
            "model constructor",
            json!({"Node":{"type":"object","additionalProperties":false},"NewNode":{"type":"string"}}),
        ),
        (
            "literal constant",
            json!({"Token":{"type":"string","enum":["kind"]},"TokenKind":{"type":"string"}}),
        ),
        (
            "union variant and constructor",
            json!({
                "Node":{"type":"string"},"Other":{"type":"integer"},
                "Choice":{"oneOf":[reference("Node"),reference("Other")]},
                "ChoiceNode":{"type":"boolean"},"NewChoiceOther":{"type":"string"}
            }),
        ),
        (
            "model fields and methods",
            json!({"Fields":{"type":"object","properties":{"setExtra":{"type":"string"},"extra":{"type":"string"},"marshal_j_s_o_n":{"type":"string"},"unmarshal_j_s_o_n":{"type":"string"}}}}),
        ),
    ] {
        let properties = schemas
            .as_object()
            .unwrap()
            .keys()
            .enumerate()
            .map(|(index, name)| (format!("member{index}"), reference(name)))
            .collect::<serde_json::Map<_, _>>();
        schemas["Inventory"] =
            json!({"type":"object","properties":properties,"additionalProperties":false});
        let contract = contract(json!({
            "openapi":"3.1.0","info":{"title":"Model namespace","version":"1"},
            "servers":[{"url":"https://namespace.example.test"}],"security":[{"apiKey":[]}],
            "paths":{"/namespace":{"get":{"operationId":"getNamespace","responses":{
                "200":{"description":"Models","content":{"application/json":{"schema":reference("Inventory")}}}
            }}}},
            "components":{"schemas":schemas,"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}}}
        }));
        let selected = contract
            .operations()
            .map(|op| op.source().clone())
            .collect::<Vec<_>>();
        match plan_http(contract.clone(), &selected, HttpConfig::default()) {
            Ok(plan) => {
                let (root, consumer) = install(&plan);
                std::fs::write(
                    consumer.join("models.go"),
                    format!(
                        "package consumer\nimport sdk {module}\nvar _ = sdk.Codecs\n",
                        module = q(MODULE)
                    ),
                )
                .unwrap();
                succeeds(&consumer, &["test", "."]);
                std::fs::remove_dir_all(root).unwrap();
            }
            Err(errors) => panic!("representable namespace collision in {case}: {errors:?}"),
        }
    }
}

fn construction_contract() -> Arc<Contract> {
    contract(json!({
        "openapi":"3.1.0","info":{"title":"Go construction","version":"1"},
        "servers":[{"url":"https://constructor.example.test/v1"}],"security":[{"apiKey":[]}],
        "paths":{"/records/{id}":{"post":{
            "operationId":"createRecord",
            "parameters":[{"name":"id","in":"path","required":true,"schema":{"type":"string"}},{"name":"label","in":"query","schema":{"type":"string"}}],
            "requestBody":{"required":true,"content":{"application/json":{"schema":reference("RecordInput")}}},
            "responses":{
                "200":{"description":"Accepted","content":{"application/json":{"schema":reference("RecordInput")}}},
                "400":{"description":"Rejected","content":{"application/json":{"schema":{"type":"string"}}}}
            }
        }}},
        "components":{"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}},"schemas":{
            "RecordInput":{"type":"object","required":["name","nullable"],"properties":{
                "name":{"type":"string"},"nullable":{"type":["string","null"]},
                "optional":{"type":"string"},"maybe":{"type":["string","null"]},"amount":{"type":"number"},
                "choice":{"oneOf":[reference("TextChoice"),reference("NumberChoice")]}
            },"additionalProperties":false},
            "TextChoice":{"type":"object","required":["kind","value"],"properties":{"kind":{"type":"string","const":"text"},"value":{"type":"string"}},"additionalProperties":false},
            "NumberChoice":{"type":"object","required":["kind","amount"],"properties":{"kind":{"type":"string","const":"number"},"amount":{"type":"number"}},"additionalProperties":false}
        }}
    }))
}

#[test]
#[ignore = "requires native Go 1.23"]
fn native_constructors_presence_and_closed_unions_reject_invalid_consumers() {
    let plan = plan(construction_contract());
    let (root, consumer) = install(&plan);
    std::fs::write(consumer.join("allowed_test.go"), ALLOWED).unwrap();
    // Prove the package, dependencies and all valid counterparts compile first;
    // each later failure must be attributed to its own invalid consumer line.
    succeeds(&consumer, &["test", "."]);
    let cases = [
        (
            "missing_model_arguments",
            "var _ = sdk.NewRecordInput()",
            "not enough arguments",
        ),
        (
            "missing_http_arguments",
            "var _ = sdk.NewCreateRecordInput()",
            "not enough arguments",
        ),
        (
            "missing_http_body",
            "var _ = sdk.NewCreateRecordInput(\"id\")",
            "not enough arguments",
        ),
        (
            "wrong_http_body",
            "var _ = sdk.NewCreateRecordInput(\"id\", \"body\")",
            "cannot use",
        ),
        (
            "wrong_path_value",
            "var _ = sdk.NewCreateRecordInput(sdk.Integer{}, sdk.NewRecordInput(\"name\", sdk.NullableNull[string]()))",
            "cannot use",
        ),
        (
            "null_required_value",
            "var _ = sdk.NewRecordInput(nil, sdk.NullableNull[string]())",
            "cannot use nil",
        ),
        (
            "null_without_nullable_wrapper",
            "var _ = sdk.NewRecordInput(\"name\", nil)",
            "cannot use nil",
        ),
        (
            "plain_nullable_value",
            "var _ = sdk.NewRecordInput(\"name\", \"value\")",
            "cannot use",
        ),
        (
            "presence_for_required_nullable",
            "var _ = sdk.NewRecordInput(\"name\", sdk.PresenceNull[string]())",
            "cannot use",
        ),
        (
            "nullable_for_optional",
            "func bad(){v:=sdk.NewRecordInput(\"name\",sdk.NullableNull[string]());v.Optional=sdk.NullableNull[string]()} ",
            "cannot use",
        ),
        (
            "optional_for_presence",
            "func bad(){v:=sdk.NewRecordInput(\"name\",sdk.NullableNull[string]());v.Maybe=sdk.OptionalSome(\"value\")}",
            "cannot use",
        ),
        (
            "null_for_presence",
            "func bad(){v:=sdk.NewRecordInput(\"name\",sdk.NullableNull[string]());v.Maybe=nil}",
            "cannot use nil",
        ),
        (
            "rounded_number",
            "func bad(){v:=sdk.NewRecordInput(\"name\",sdk.NullableNull[string]());v.Amount=sdk.OptionalSome(1.5)}",
            "cannot use",
        ),
        (
            "wrapped_optional_setter",
            "var _ = sdk.NewCreateRecordInput(\"id\",sdk.NewRecordInput(\"name\",sdk.NullableNull[string]())).WithLabel(sdk.OptionalSome(\"label\"))",
            "cannot use",
        ),
        (
            "unwrapped_union_member",
            "var _ sdk.RecordInputChoice = sdk.TextChoice{}",
            "does not implement",
        ),
        (
            "foreign_closed_union",
            "type forged struct{}\nfunc(forged)isRecordInputChoice(){}\nvar _ sdk.RecordInputChoice = forged{}",
            "unexported method",
        ),
        (
            "error_in_success_union",
            "var _ sdk.CreateRecordResult = &sdk.CreateRecordStatus400{}",
            "does not implement",
        ),
        (
            "foreign_response_union",
            "type forged struct{}\nfunc(forged)isCreateRecordResult(){}\nfunc(forged)Close()error{return nil}\nvar _ sdk.CreateRecordResult = forged{}",
            "unexported method",
        ),
    ];
    for (name, invalid, diagnostic) in cases {
        let directory = consumer.join(name);
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(
            directory.join("invalid.go"),
            format!(
                "package invalid\nimport sdk {module}\n{invalid}\n",
                module = q(MODULE)
            ),
        )
        .unwrap();
        let output = go(&consumer, &["test", "-run=^$", &format!("./{name}")]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success(),
            "invalid consumer {name} compiled in {}",
            root.display()
        );
        assert!(
            stderr.contains("invalid.go:") && stderr.contains(diagnostic),
            "{name}: expected its own {diagnostic:?} diagnostic in {}\n{}{}",
            root.display(),
            String::from_utf8_lossy(&output.stdout),
            stderr
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

const ALLOWED: &str = r#"package consumer
import("bytes";"context";"testing";sdk "example.com/go-http-names")
func TestAllowed(t *testing.T){
 record:=sdk.NewRecordInput("native",sdk.NullableNull[string]())
 if record.Nullable.IsValue||record.Optional.IsSet||record.Maybe.IsSet||record.Choice.IsSet{t.Fatal("constructors inferred optional/null values")}
 record.Optional=sdk.OptionalSome("present")
 record.Maybe=sdk.PresenceNull[string]()
 record.Choice=sdk.OptionalSome(sdk.NewRecordInputChoiceTextChoice(sdk.NewTextChoice(sdk.TextChoiceKindText,"tagged")))
 exact,err:=sdk.ParseNumber("9007199254740993.000000000000000001");if err!=nil{t.Fatal(err)}
 record.Amount=sdk.OptionalSome(exact)
 input:=sdk.NewCreateRecordInput("record-id",record).WithLabel("label")
 if input.Id!="record-id"||!input.Label.IsSet||input.Label.Value!="label"{t.Fatal("typed HTTP construction")}
 var call func(*sdk.Client,context.Context,sdk.CreateRecordInput)(sdk.CreateRecordResult,error)=(*sdk.Client).CreateRecord
 _=call
 encoded,err:=sdk.Codecs.RecordInput.Encode(input.Body);if err!=nil{t.Fatal(err)}
 for _,fragment:=range []string{`"nullable":null`,`"maybe":null`,`9007199254740993.000000000000000001`,`"kind":"text"`}{if !bytes.Contains(encoded,[]byte(fragment)){t.Fatalf("lost %s: %s",fragment,encoded)}}
 decoded,err:=sdk.Codecs.RecordInput.Decode(encoded);if err!=nil{t.Fatal(err)}
 if !decoded.Maybe.IsSet||!decoded.Maybe.Null||decoded.Amount.Value.String()!=exact.String(){t.Fatal("presence or exact number changed")}
 if _,ok:=decoded.Choice.Value.(sdk.RecordInputChoiceTextChoice);!ok{t.Fatal("closed tagged union changed")}
}
"#;
