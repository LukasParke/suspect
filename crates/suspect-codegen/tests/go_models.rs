//! Canonical source -> actual Go consumer, presence wrappers and native docs.

use serde_json::{Value, json};
use std::{path::Path, process::Command, sync::Arc};
use suspect_codegen::go_models::{ModelPlan, plan_models};
use suspect_ir::contract::Contract;
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
fn fixture(schemas: Value) -> Arc<Contract> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(&path,json!({"openapi":"3.1.0","info":{"title":"Go models","version":"1"},"paths":{},"components":{"schemas":schemas}}).to_string()).unwrap();
    load(&path)
}
fn native(plan: &ModelPlan, source: &str) {
    assert!(!plan.release_ready());
    let directory = tempfile::tempdir().unwrap().keep();
    suspect_codegen::write_files(&plan.render().unwrap(), &directory).unwrap();
    let root = directory.join("go");
    std::fs::write(root.join("models_test.go"), source).unwrap();
    for args in [vec!["test", "./..."], vec!["doc", "-all", "."]] {
        let output = Command::new("go")
            .current_dir(&root)
            .args(args)
            .env("GOWORK", "off")
            .env(
                "GOTOOLCHAIN",
                std::env::var_os("SUSPECT_GO_TOOLCHAIN").unwrap_or_else(|| "local".into()),
            )
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "fixture {}\n{}{}",
            directory.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn unimplemented_go_shapes_block_artifacts_with_source_locations() {
    let contract = fixture(json!({"Rejected": {
        "type": "object",
        "required": ["missing"],
        "additionalProperties": false,
    }}));
    let plan = plan_models(&contract, contract.schema_roots());
    assert!(plan.has_errors(), "{:?}", plan.diagnostics());
    assert!(plan.render().is_err());
    assert!(
        plan.diagnostics().iter().any(|error| {
            error.kind == suspect_codegen::rust_models::DiagnosticKind::Error
                && error.code == "undeclared-required-field"
                && error.source.document() == contract.entry()
                && error.source.pointer() == "/components/schemas/Rejected/required"
                && !error.at.is_empty()
        }),
        "{:?}",
        plan.diagnostics()
    );
}

fn pattern_contract() -> Arc<Contract> {
    fixture(json!({"Patterned":{"type":"object","patternProperties":{"^x":{"type":"integer"}}}}))
}

#[test]
fn pattern_properties_use_scoped_codecs_and_retain_model_obligations() {
    let contract = pattern_contract();
    let models = plan_models(&contract, contract.schema_roots());
    assert!(!models.has_errors(), "{:?}", models.diagnostics());
    assert!(models.render().is_ok());
    assert!(!models.release_ready());
    assert!(models.diagnostics().iter().any(|finding| {
        finding.kind == suspect_codegen::rust_models::DiagnosticKind::CodecObligation
            && finding.code == "model-codec-unimplemented"
            && finding.source.document() == contract.entry()
            && finding.source.pointer() == "/components/schemas/Patterned"
            && !finding.at.is_empty()
    }));
    let codecs = suspect_codegen::go_codecs::plan_codecs(
        contract.clone(),
        contract.schema_roots(),
        Default::default(),
    )
    .unwrap();
    let program = codecs.validation_program();
    assert_eq!(program.version, suspect_schema::OwnedProgram::V2_VERSION);
    assert_eq!(program.profile, suspect_schema::OwnedProgram::V2_PROFILE);
    assert!(
        program
            .nodes
            .iter()
            .flat_map(|node| &node.checks)
            .any(|check| {
                matches!(
                    check.instruction,
                    suspect_schema::ProgramInstruction::PatternProperties { .. }
                ) && check.source.pointer == "/components/schemas/Patterned/patternProperties"
            })
    );
}

#[test]
#[ignore = "requires native Go 1.23.12/1.27.1"]
fn native_pattern_properties_validate_matching_values_without_stripping_extras() {
    let contract = pattern_contract();
    let codecs = suspect_codegen::go_codecs::plan_codecs(
        contract.clone(),
        contract.schema_roots(),
        Default::default(),
    )
    .unwrap();
    let root = tempfile::Builder::new()
        .prefix("suspect-go-pattern-admission-")
        .tempdir()
        .unwrap()
        .keep();
    suspect_codegen::write_files(&codecs.render(), &root).unwrap();
    let consumer = root.join("consumer");
    std::fs::create_dir(&consumer).unwrap();
    std::fs::write(consumer.join("go.mod"), "module example.com/pattern-consumer\n\ngo 1.23.0\nrequire example.com/generated-models v0.0.0\nreplace example.com/generated-models => ../go\n").unwrap();
    let source_document = serde_json::to_string(contract.entry().as_str()).unwrap();
    std::fs::write(consumer.join("pattern_test.go"), format!(r#"package consumer
import("bytes";"errors";"testing";sdk "example.com/generated-models")
func TestPatternedBoundary(t *testing.T){{
 exact,err:=sdk.ParseInteger("9007199254740993");if err!=nil{{t.Fatal(err)}}
 value:=sdk.Patterned{{"x-count":exact,"unmatched":"kept","optional-null":nil}}
 wire,err:=sdk.Codecs.Patterned.Encode(value);if err!=nil{{t.Fatal(err)}}
 if !bytes.Contains(wire,[]byte(`"x-count":9007199254740993`)){{t.Fatal(string(wire))}}
 decoded,err:=sdk.Codecs.Patterned.Decode(wire);if err!=nil{{t.Fatal(err)}}
 if decoded["unmatched"]!="kept"{{t.Fatal("unmatched extra was stripped",decoded)}}
 if v,exists:=decoded["optional-null"];!exists||v!=nil{{t.Fatal("null extra lost",decoded)}}
 if number,ok:=decoded["x-count"].(sdk.Number);!ok||number.String()!=exact.String(){{t.Fatal("exact matching value changed",decoded)}}
 decoded["x-count"]="invalid mutation"
 for _,encode:=range []func()error{{func()error{{_,err:=sdk.Codecs.Patterned.Encode(decoded);return err}},func()error{{_,err:=sdk.Codecs.Patterned.EncodeValue(decoded);return err}}}}{{
  err:=encode();var failure *sdk.CodecError
  if !errors.As(err,&failure)||failure.Kind!="invalid"||failure.Source.Document!={source_document}||failure.Source.Pointer!="/components/schemas/Patterned/patternProperties/^x/type"||failure.Path!="/x-count"{{t.Fatalf("mutable pattern constraint/source lost: %#v / %v",failure,err)}}
 }}
 _,err=sdk.Codecs.Patterned.Decode([]byte(`{{"x-count":"invalid input","unmatched":"kept"}}`));var failure *sdk.CodecError
 if !errors.As(err,&failure)||failure.Kind!="invalid"||failure.Source.Pointer!="/components/schemas/Patterned/patternProperties/^x/type"||failure.Path!="/x-count"{{t.Fatalf("decode skipped pattern constraint: %#v / %v",failure,err)}}
}}
"#)).unwrap();
    let toolchains = std::env::var("SUSPECT_GO_TOOLCHAIN")
        .map(|tool| vec![tool])
        .unwrap_or_else(|_| vec!["go1.23.12".into(), "go1.27.1".into()]);
    for toolchain in toolchains {
        let output = Command::new("go")
            .args(["test", "-count=1", "-v", "."])
            .current_dir(&consumer)
            .env("GOWORK", "off")
            .env("GOTOOLCHAIN", &toolchain)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{} {toolchain}\n{}{}",
            root.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        eprintln!(
            "{toolchain}: {}",
            String::from_utf8_lossy(&output.stdout).trim()
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn newly_represented_go_union_still_requires_source_codecs() {
    let contract = fixture(json!({"Choice":{"oneOf":[{"type":"string"},{"type":"integer"}]}}));
    let plan = plan_models(&contract, contract.schema_roots());
    assert!(!plan.has_errors(), "{:?}", plan.diagnostics());
    assert!(plan.render().is_ok());
    assert!(!plan.release_ready());
    assert!(
        plan.diagnostics()
            .iter()
            .any(|finding| finding.code == "model-codec-unimplemented")
    );
}

#[test]
#[ignore = "requires native Go"]
fn native_presence_recursion_literals_and_extra_guards_are_consumable() {
    let contract = fixture(json!({
        "Status":{"type":"string","enum":["active","inactive"]},
        "Record":{"type":"object","required":["name","nullable"],"properties":{
            "name":{"type":"string"},"nullable":{"type":["string","null"]},
            "optional":{"type":"string"},"maybe":{"type":["string","null"]},"next":{"$ref":"#/components/schemas/Record"}
        },"additionalProperties":{"type":"integer"}},
        "Tree":{"type":"object","required":["children"],"properties":{"children":{"type":"array","items":{"$ref":"#/components/schemas/Tree"}}}}
    }));
    let plan = plan_models(&contract, contract.schema_roots());
    assert!(!plan.has_errors(), "{:?}", plan.diagnostics());
    native(
        &plan,
        r#"package sdk
import ("encoding/json"; "testing")
func TestNativeModels(t *testing.T) {
    var status Status = StatusActive
    if status != StatusActive {t.Fatal("literal")}
    record:=NewRecord("root",NullableNull[string]())
    if record.Optional.IsSet || record.Maybe.IsSet || record.Nullable.IsValue {t.Fatal("presence collapsed")}
    record.Maybe=PresenceNull[string]()
    record.Optional=OptionalSome("value")
    if !record.Maybe.IsSet || !record.Maybe.Null || record.Optional.Value!="value" {t.Fatal("explicit states lost")}
    child:=NewRecord("child",NullableValue("non-null"))
    record.Next=OptionalSome(&child)
    if record.Next.Value.Name!="child" {t.Fatal("recursive reference lost")}
    tree:=NewTree([]Tree{})
    tree.Children=append(tree.Children,NewTree(nil))
    if len(tree.Children)!=1 {t.Fatal("recursive collection")}
    number,err:=ParseInteger("9007199254740993");if err!=nil {t.Fatal(err)}
    if err:=record.SetExtra("extra",number);err!=nil {t.Fatal(err)}
    if err:=record.SetExtra("name",number);err==nil {t.Fatal("extra shadowed wire name")}
    copy:=record.Extra();delete(copy,"extra")
    if record.Extra()["extra"].String()!="9007199254740993" {t.Fatal("extra map leaked mutation")}
    if _,err:=json.Marshal(record);err==nil {t.Fatal("missing codec silently serialized model")}
    if err:=json.Unmarshal([]byte(`{"name":"wire"}`),&record);err==nil {t.Fatal("missing codec silently decoded model")}
}
"#,
    );
}

#[test]
#[ignore = "requires native Go and the tracked OpenRouter checkout"]
fn tracked_credits_models_have_exact_native_number_fields() {
    let root = std::env::var_os("OPENROUTER_WEB_ROOT").expect("set OPENROUTER_WEB_ROOT");
    let contract = load(&Path::new(&root).join("projects/docs/openapi/openapi.yaml"));
    let response = contract
        .operations()
        .find(|op| op.operation_id() == Some("getCredits"))
        .unwrap()
        .responses()
        .into_iter()
        .find(|response| {
            response.status() == Some(suspect_ir::contract::ResponseStatus::Exact(200))
        })
        .unwrap()
        .content()
        .into_iter()
        .find(|media| media.name() == "application/json")
        .unwrap()
        .schema()
        .unwrap()
        .id()
        .clone();
    let plan = plan_models(&contract, &[response]);
    assert!(!plan.has_errors(), "{:?}", plan.diagnostics());
    let data = plan
        .fields()
        .iter()
        .find(|field| field.wire == "total_credits")
        .unwrap()
        .model
        .clone();
    native(
        &plan,
        &format!(
            r#"package sdk
import "testing"
func TestCredits(t *testing.T) {{
    credits,err:=ParseNumber("9007199254740993.0001");if err!=nil {{t.Fatal(err)}}
    usage,err:=ParseNumber("1e-400");if err!=nil {{t.Fatal(err)}}
    value:=New{data}(credits,usage)
    if value.TotalCredits.String()!="9007199254740993.0001" || value.TotalUsage.String()!="1e-400" {{t.Fatal("precision lost")}}
}}
"#
        ),
    );
}
