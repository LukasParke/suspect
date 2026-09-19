//! Native naming collisions must not bind a call to another operation or value.
use serde_json::json;
use std::{path::Path, process::Command, sync::Arc};
use suspect_codegen::python_http::{PackageConfig, emit_http, plan_http};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

#[test]
#[ignore = "requires native Python with httpx/mypy tools"]
fn keyword_arguments_and_status_classes_survive_source_name_collisions() {
    let root = tempfile::tempdir().unwrap().keep();
    let path = root.join("api.json");
    let response = |schema| json!({"200":{"description":"value","content":{"application/json":{"schema":schema}}}});
    let parameters=["codecs","parameters","body_bytes","raw","models","isinstance","body","FOO","foo"].into_iter().map(|name|json!({"name":name,"in":"query","required":name!="isinstance","schema":{"type":"string"}})).collect::<Vec<_>>();
    std::fs::write(&path,json!({"openapi":"3.1.0","info":{"title":"Native names","version":"1"},"servers":[{"url":"https://names.example/v1"}],"security":[{"key":[]}],"components":{"securitySchemes":{"key":{"type":"http","scheme":"bearer"}}},"paths":{"/first":{"get":{"operationId":"getThing","parameters":parameters,"responses":response(json!({"type":"string"}))}},"/second":{"get":{"operationId":"get-thing","responses":response(json!({"type":"integer"}))}},"/third":{"get":{"operationId":"123雪","responses":response(json!({"type":"boolean"}))}}}}).to_string()).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(&root).build().unwrap());
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap());
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let plan = plan_http(contract, &selected, Default::default()).unwrap();
    let first = plan
        .operations()
        .iter()
        .find(|op| op.operation_id == "getThing")
        .unwrap();
    let second = plan
        .operations()
        .iter()
        .find(|op| op.operation_id == "get-thing")
        .unwrap();
    let third = plan
        .operations()
        .iter()
        .find(|op| op.operation_id == "123雪")
        .unwrap();
    assert_ne!(first.success_type, second.success_type);
    assert_ne!(
        first.responses()[0].class_name,
        second.responses()[0].class_name
    );
    let args = first
        .parameters()
        .iter()
        .enumerate()
        .map(|(i, p)| format!("{}='value{i}'", p.name))
        .collect::<Vec<_>>()
        .join(",");
    let script = CONSUMER
        .replace("__FIRST__", &first.snake_name)
        .replace("__SECOND__", &second.snake_name)
        .replace("__THIRD__", &third.snake_name)
        .replace("__FIRST_CLASS__", &first.responses()[0].class_name)
        .replace("__SECOND_CLASS__", &second.responses()[0].class_name)
        .replace("__ARGS__", &args);
    suspect_codegen::write_files(
        &emit_http(
            &plan,
            &PackageConfig {
                name: "names-sdk".into(),
                version: "1.0.0".into(),
                import_name: "names_sdk".into(),
            },
        )
        .unwrap(),
        &root,
    )
    .unwrap();
    std::fs::write(root.join("consumer.py"), script).unwrap();
    let python = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/sdk-native-python-tools/bin/python");
    for args in [
        vec!["consumer.py"],
        vec![
            "-m",
            "mypy",
            "--strict",
            "--python-version",
            "3.11",
            "python/src/names_sdk",
        ],
    ] {
        let output = Command::new(&python)
            .args(args)
            .env("PYTHONPATH", root.join("python/src"))
            .current_dir(&root)
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
const CONSUMER: &str = r#"
import httpx
from urllib.parse import parse_qsl
from names_sdk import Client
from names_sdk.operations import __FIRST_CLASS__, __SECOND_CLASS__
seen=[]
def transport(request):
    seen.append(request)
    return httpx.Response(200,headers={'Content-Type':'application/json'},content=b'"first"' if request.url.path.endswith('/first') else b'42' if request.url.path.endswith('/second') else b'true')
with Client(auth={'key':'test-key'},transport=httpx.MockTransport(transport)) as client:
    first=client.__FIRST__(__ARGS__)
    second=client.__SECOND__()
    third=client.__THIRD__()
    assert type(first) is __FIRST_CLASS__ and first.data=='first'
    assert type(second) is __SECOND_CLASS__ and second.data==42
    assert third.data is True
assert parse_qsl(seen[0].url.query.decode())==[(name,f'value{i}') for i,name in enumerate(['codecs','parameters','body_bytes','raw','models','isinstance','body','FOO','foo'])]
assert [request.url.path for request in seen]==['/v1/first','/v1/second','/v1/third']
"#;
