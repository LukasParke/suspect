//! Native recipes retain the grouping of validated whole JSON aggregates.
use serde_json::{Value, json};
use std::{process::Command, sync::Arc};
use suspect_codegen::go_http;
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

#[test]
#[ignore = "requires native Go 1.23.12/1.27.1"]
fn declared_form_and_positional_groups_keep_items_extras_and_absence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.json");
    let form = json!({"type":"object","required":["first"],"properties":{"first":{"type":"string"},"optional":{"type":"string"},"tags":{"type":"array","items":{"type":"string"}}},"additionalProperties":{"type":"string"}});
    let mut paths = json!({});
    for (name, media, schema, example) in [
        (
            "formGroup",
            "application/x-www-form-urlencoded",
            form.clone(),
            json!({"first":"x","tags":["one","two"],"x-extra":"exact"}),
        ),
        (
            "emptyGroup",
            "application/x-www-form-urlencoded",
            form,
            json!({"first":"x","tags":[]}),
        ),
        (
            "positionalGroup",
            "multipart/mixed",
            json!({"type":"array","minItems":1,"prefixItems":[{"type":"string"}],"items":{"type":"integer"}}),
            json!(["prefix", 3, 8]),
        ),
    ] {
        let mut representation = json!({"schema":schema,"example":example});
        if media == "multipart/mixed" {
            representation["prefixEncoding"] = json!([{"contentType":"text/plain"}]);
            representation["itemEncoding"] = json!({"contentType":"text/plain"});
        }
        paths[format!("/{name}")] = json!({"post":{"operationId":name,"requestBody":{"required":true,"content":{media:representation}},"responses":{"204":{}}}});
    }
    std::fs::write(&path,json!({"openapi":"3.2.0","info":{"title":"Grouped native examples","version":"1"},"servers":[{"url":"https://example.test"}],"paths":paths}).to_string()).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(dir.path()).build().unwrap());
    let c =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap());
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
    let mut call_ids = Vec::new();
    for (index, operation) in plan.operations().iter().enumerate() {
        let source = plan
            .examples()
            .operations()
            .iter()
            .find(|e| e.source == operation.source)
            .unwrap();
        assert!(!source.validated_aggregates.is_empty());
        let example = docs["examples"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["method"] == operation.method_name)
            .unwrap();
        assert_eq!(example["available"], true);
        let bindings = example["bindings"].as_array().unwrap();
        assert_eq!(bindings.len(), 1, "one aggregate input binding: {example}");
        let binding = &bindings[0];
        assert_eq!(binding["member"], "Body");
        let aggregate =
            &source.validated_aggregates[binding["validatedAggregate"].as_u64().unwrap() as usize];
        assert_eq!(
            binding["container"]["pointer"],
            aggregate.container.pointer()
        );
        assert_eq!(
            binding["declaredSource"]["pointer"],
            aggregate.declared_source.as_ref().unwrap().pointer()
        );
        call_ids.push((operation.operation_id.clone(), index));
    }
    let root = tempfile::Builder::new()
        .prefix("suspect-go-aggregate-examples-")
        .tempdir()
        .unwrap()
        .keep();
    suspect_codegen::write_files(&files, &root).unwrap();
    let call = |name: &str| call_ids.iter().find(|(n, _)| n == name).unwrap().1;
    let source = format!(
        r#"package main
import("bytes";"context";"errors";"io";"mime";"mime/multipart";"net/http";"reflect";"strings";"testing";sdk "example.com/generated-sdk")
type doer func(*http.Request)(*http.Response,error)
func(f doer)Do(r *http.Request)(*http.Response,error){{return f(r)}}
func TestActualGroupedRecipes(t *testing.T){{calls:=0;client,err:=sdk.NewClient(sdk.Credentials{{}},sdk.ClientOptions{{Transport:doer(func(r *http.Request)(*http.Response,error){{calls++;wire,err:=io.ReadAll(r.Body);if err!=nil{{t.Fatal(err)}};if strings.HasSuffix(r.URL.Path,"/formGroup"){{if string(wire)!="first=x&tags=one&tags=two&x-extra=exact"{{t.Fatal(string(wire))}}}}else{{_,params,err:=mime.ParseMediaType(r.Header.Get("Content-Type"));if err!=nil{{t.Fatal(err)}};reader:=multipart.NewReader(bytes.NewReader(wire),params["boundary"]);var values []string;for{{part,err:=reader.NextRawPart();if err==io.EOF{{break}};if err!=nil{{t.Fatal(err)}};data,err:=io.ReadAll(part);if err!=nil{{t.Fatal(err)}};values=append(values,string(data))}};if !reflect.DeepEqual(values,[]string{{"prefix","3","8"}}){{t.Fatal(values)}}}};return &http.Response{{StatusCode:204,Header:make(http.Header),Body:io.NopCloser(strings.NewReader(""))}},nil}})}});if err!=nil{{t.Fatal(err)}};ctx:=context.Background();if _,err=call{form}(ctx,client);err!=nil{{t.Fatal(err)}};if _,err=call{positional}(ctx,client);err!=nil{{t.Fatal(err)}};_,err=call{empty}(ctx,client);var failure *sdk.SDKError;if !errors.As(err,&failure)||failure.Kind!="request-validation"{{t.Fatal("empty repeated array was fabricated or silently omitted",err)}};if calls!=2{{t.Fatal(calls)}}}}
"#,
        form = call("formGroup"),
        positional = call("positionalGroup"),
        empty = call("emptyGroup")
    );
    std::fs::write(root.join("go/examples/validated/group_test.go"), source).unwrap();
    let tools = std::env::var("SUSPECT_GO_TOOLCHAIN")
        .map(|t| vec![t])
        .unwrap_or_else(|_| vec!["go1.23.12".into(), "go1.27.1".into()]);
    for tool in tools {
        let output = Command::new("go")
            .args(["test", "-count=1", "-v", "./examples/validated"])
            .env("GOWORK", "off")
            .env("GOTOOLCHAIN", &tool)
            .current_dir(root.join("go"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{} {tool}\n{}{}",
            root.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        eprintln!("{tool}: {}", String::from_utf8_lossy(&output.stdout).trim());
    }
    std::fs::remove_dir_all(root).unwrap();
}
