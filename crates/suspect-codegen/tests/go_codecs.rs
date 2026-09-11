//! Native source->Go models/codecs, including mutable revalidation and exact values.
use std::{path::Path, process::Command, sync::Arc};
use suspect_codegen::go_codecs::plan_codecs;
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn plan() -> suspect_codegen::go_codecs::CodecPlan {
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m2/canonical.openapi.yaml");
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(source.parent().unwrap())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&source).unwrap()).unwrap());
    let roots = contract.schema_roots().to_vec();
    plan_codecs(contract, &roots, Default::default()).unwrap()
}
#[test]
fn codec_package_has_native_models_and_checked_program() {
    let files = plan().render();
    for name in [
        "go/models.go",
        "go/codecs.go",
        "go/codec_runtime.go",
        "go/codec-plan.json",
        "go/validation.go",
        "go/validation_program.json",
    ] {
        assert!(files.iter().any(|file| file.path == name), "{name}");
    }
    let manifest: serde_json::Value = serde_json::from_str(
        &files
            .iter()
            .find(|file| file.path == "go/model-manifest.json")
            .unwrap()
            .content,
    )
    .unwrap();
    assert_eq!(manifest["sourceCodecs"], true);
    assert!(
        !manifest["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|finding| finding["kind"] == "codec_obligation")
    );
}
#[test]
#[ignore = "requires native Go"]
fn native_exact_models_union_presence_and_json_adapters() {
    let root = tempfile::tempdir().unwrap().keep();
    suspect_codegen::write_files(&plan().render(), &root).unwrap();
    let go = root.join("go");
    std::fs::write(go.join("codecs_test.go"),r#"package sdk
import("testing";"encoding/json";"errors")
func TestModelCodecs(t *testing.T){
 wire:=[]byte(`{"id":"w1","amount":9007199254740993.000000000000000001,"meta":null,"payload":{"kind":"standard","text":"ok"},"child":{"label":"root"}}`)
 widget,err:=Codecs.Widget.Decode(wire);if err!=nil{t.Fatal(err)}
 if widget.Amount.String()!="9007199254740993.000000000000000001"||!widget.Meta.IsSet||!widget.Meta.Null{t.Fatalf("wrong exact/presence model %#v",widget)}
 if !widget.Child.IsSet||widget.Child.Value.Label!="root"||widget.Child.Value.Child.IsSet{t.Fatal("recursive presence changed")}
 branch,ok:=widget.Payload.(WidgetPayloadStandardPayload);if !ok||branch.Value.Text!="ok"{t.Fatal("tagged branch changed")}
 output,err:=Codecs.Widget.Encode(widget);if err!=nil{t.Fatal(err)}
 repeated,err:=Codecs.Widget.Decode(output);if err!=nil||repeated.Amount.String()!=widget.Amount.String(){t.Fatal("roundtrip",err)}
 encoded,err:=json.Marshal(widget);if err!=nil{t.Fatal(err)}
 var decoded Widget;if err=json.Unmarshal(encoded,&decoded);err!=nil{t.Fatal(err)}
 if decoded.Amount.String()!=widget.Amount.String(){t.Fatal("encoding/json lost exact number")}
 input:=NewWidgetInput("valid");input.Name=""
 if _,err=Codecs.WidgetInput.Encode(input);err==nil{t.Fatal("mutated invalid input accepted")}
 if _,err=json.Marshal(input);err==nil{t.Fatal("json adapter bypassed validation")}
 if err=input.SetExtra("name",nil);err==nil{t.Fatal("extra key collision")}
 if _,err=Codecs.Widget.Decode([]byte(`{"id":"w1","amount":1,"payload":{"kind":"standard","vault":"wrong"}}`));err==nil{t.Fatal("wrong tagged payload accepted")}
 widget.Meta=Presence[string]{Null:true}
 if _,err=Codecs.Widget.Encode(widget);err==nil{t.Fatal("inconsistent presence accepted")}
 var codec *CodecError;if !errors.As(err,&codec){t.Fatalf("missing typed codec error: %v",err)}
}
"#).unwrap();
    let mut command = Command::new("go");
    command
        .args(["test", "./..."])
        .current_dir(&go)
        .env("GOWORK", "off");
    if let Some(toolchain) = std::env::var_os("SUSPECT_GO_TOOLCHAIN") {
        command.env("GOTOOLCHAIN", toolchain);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::remove_dir_all(root).unwrap();
}
