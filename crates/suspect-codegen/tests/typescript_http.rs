use serde_json::{Value, json};
use std::{process::Command, sync::Arc};
use suspect_codegen::typescript::http::{HttpConfig, plan_http};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn fixture(schema: Value) -> (Arc<Contract>, Uri) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(&path, schema.to_string()).unwrap();
    let uri = Uri::from_path(&path).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    (
        Arc::new(Contract::from_workspace(&workspace, &uri).unwrap()),
        uri,
    )
}

fn api(extra: Value) -> Value {
    let mut value = json!({"openapi":"3.1.0","info":{"title":"HTTP","version":"1"},"servers":[{"url":"https://openrouter.ai/api/v1"}],"security":[{"apiKey":[]}],"components":{"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}}},"paths":{"/credits":{"get":{"operationId":"getCredits","responses":{"200":{"description":"ok","content":{"application/json":{"schema":{"type":"object","properties":{"data":{"type":"string"}},"required":["data"]}}}},"401":{"description":"no","content":{"application/json":{"schema":{"type":"object","properties":{"error":{"type":"string"}},"required":["error"]}}}}}}}}});
    if let Some((k, v)) = extra.as_object().and_then(|o| o.iter().next()) {
        value
            .pointer_mut("/paths/~1credits/get")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert(k.clone(), v.clone());
    }
    value
}

#[test]
fn source_selected_plan_emits_one_codec_plan_and_literal_http_contract() {
    let (contract, _uri) = fixture(api(json!({})));
    let source = contract.operations().next().unwrap().source().clone();
    let plan = plan_http(contract, &[source], HttpConfig::default()).unwrap();
    let files = plan.render();
    // Actual compilation proves the public declarations remain consumable.
    let out = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&files, out.path()).unwrap();
    let result = Command::new("tsc")
        .current_dir(out.path().join("typescript"))
        .args([
            "--strict",
            "--target",
            "ES2022",
            "--module",
            "NodeNext",
            "--moduleResolution",
            "NodeNext",
            "--skipLibCheck",
            "operations.ts",
            "--noEmit",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn unsupported_security_is_located_and_direct_directional_annotations_are_admitted() {
    let (contract, _uri) = fixture(api(json!({"security":[]})));
    let source = contract.operations().next().unwrap().source().clone();
    let errors = plan_http(contract, &[source], HttpConfig::default()).unwrap_err();
    assert!(errors.iter().any(|e| e.code == "http-capability-required"
        && e.message.contains("AnonymousSecurity")
        && e.source.pointer().ends_with("/security")));

    let mut directional = api(json!({}));
    directional
        .pointer_mut(
            "/paths/~1credits/get/responses/200/content/application~1json/schema/properties/data",
        )
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert("readOnly".into(), json!(true));
    let (contract, _uri) = fixture(directional);
    let source = contract.operations().next().unwrap().source().clone();
    let plan = plan_http(contract, &[source], HttpConfig::default()).unwrap();
    assert!(
        plan.codecs()
            .models()
            .symbols()
            .iter()
            .any(|symbol| symbol.view() == suspect_codegen::typescript::ModelView::Response)
    );
}

#[test]
fn malformed_present_http_profile_metadata_is_never_defaulted_or_filtered() {
    let mut scopes = api(json!({}));
    scopes["security"] = json!([{"apiKey":[42]}]);
    let mut required = api(json!({}));
    let operation = required["paths"]
        .as_object_mut()
        .unwrap()
        .remove("/credits")
        .unwrap();
    required["paths"]["/credits/{id}"] = operation;
    required["paths"]["/credits/{id}"]["get"]["parameters"] =
        json!([{"name":"id","in":"path","required":"yes","schema":{"type":"string"}}]);
    let mut explode = required.clone();
    explode["paths"]["/credits/{id}"]["get"]["parameters"][0]["required"] = json!(true);
    explode["paths"]["/credits/{id}"]["get"]["parameters"][0]["explode"] = json!(0);
    let mut reserved = explode.clone();
    reserved["paths"]["/credits/{id}"]["get"]["parameters"][0]
        .as_object_mut()
        .unwrap()
        .remove("explode");
    reserved["paths"]["/credits/{id}"]["get"]["parameters"][0]["allowReserved"] = Value::Null;
    let mut body = api(json!({}));
    body["paths"]["/credits"]["get"]["requestBody"] =
        json!({"required":"yes","content":{"application/json":{"schema":{"type":"object"}}}});
    let mut scheme = api(json!({}));
    scheme["components"]["securitySchemes"]["apiKey"]["scheme"] = json!(42);
    for (value, code, suffix) in [
        (
            scopes,
            "http-security-permissions-invalid",
            "/security/0/apiKey/0",
        ),
        (required, "http-metadata-boolean", "/parameters/0/required"),
        (explode, "http-metadata-boolean", "/parameters/0/explode"),
        (
            reserved,
            "http-metadata-boolean",
            "/parameters/0/allowReserved",
        ),
        (body, "http-metadata-boolean", "/requestBody/required"),
        (
            scheme,
            "http-metadata-string",
            "/securitySchemes/apiKey/scheme",
        ),
    ] {
        let (contract, _) = fixture(value);
        let source = contract.operations().next().unwrap().source().clone();
        let errors = plan_http(contract, &[source], HttpConfig::default()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.code == code && error.source.pointer().ends_with(suffix)),
            "{code} {suffix}: {errors:?}"
        );
    }
}

#[test]
fn referenced_request_body_and_raw_response_shapes_are_checked_at_terminal_sources() {
    let directory = tempfile::tempdir().unwrap();
    let body_path = directory.path().join("body.json");
    std::fs::write(&body_path,json!({"Body":{"required":"false","content":{"application/json":{"schema":{"type":"object"}}}}}).to_string()).unwrap();
    let mut value = api(json!({}));
    value["paths"]["/credits"]["get"]["requestBody"] = json!({"$ref":"./body.json#/Body"});
    let path = directory.path().join("api.json");
    std::fs::write(&path, value.to_string()).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(directory.path())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap());
    let source = contract.operations().next().unwrap().source().clone();
    let errors = plan_http(contract, &[source], HttpConfig::default()).unwrap_err();
    let error = errors
        .iter()
        .find(|error| error.code == "http-metadata-boolean")
        .expect("terminal required diagnostic");
    assert!(error.source.document().to_string().ends_with("/body.json"));
    assert_eq!(error.source.pointer(), "/Body/required");

    for (value, suffix) in [
        {
            let mut v = api(json!({}));
            v["paths"]["/credits"]["get"]["responses"] = json!([]);
            (v, "/responses")
        },
        {
            let mut v = api(json!({}));
            v["paths"]["/credits"]["get"]["responses"]["200"] = json!(42);
            (v, "/responses/200")
        },
        {
            let mut v = api(json!({}));
            v["paths"]["/credits"]["get"]["responses"]["200"]["content"] = json!([]);
            (v, "/responses/200/content")
        },
        {
            let mut v = api(json!({}));
            v["paths"]["/credits"]["get"]["responses"]["200"]["headers"] = json!([]);
            (v, "/responses/200/headers")
        },
        {
            let mut v = api(json!({}));
            v["paths"]["/credits"]["get"]["responses"]["200"]["links"] = json!({"next":42});
            (v, "/responses/200/links/next")
        },
    ] {
        let (contract, _) = fixture(value);
        let source = contract.operations().next().unwrap().source().clone();
        let errors = plan_http(contract, &[source], HttpConfig::default()).unwrap_err();
        assert!(
            errors.iter().any(|error| matches!(
                error.code,
                "http-metadata-object" | "http-content-invalid" | "http-headers-invalid"
            ) && error.source.pointer().ends_with(suffix)),
            "{suffix}: {errors:?}"
        );
    }
    let mut invalid_schema = api(json!({}));
    invalid_schema["paths"]["/credits"]["get"]["responses"]["200"]["content"]["application/json"]
        ["schema"] = json!(42);
    let (contract, _) = fixture(invalid_schema);
    let source = contract.operations().next().unwrap().source().clone();
    let errors = plan_http(contract, &[source], HttpConfig::default()).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.code == "http-schema-invalid"
                && error
                    .source
                    .pointer()
                    .ends_with("/responses/200/content/application~1json/schema")),
        "{errors:?}"
    );

    let mut extension = api(json!({}));
    extension["paths"]["/credits"]["get"]["responses"]["x-review-note"] = json!(42);
    let (contract, _) = fixture(extension);
    let source = contract.operations().next().unwrap().source().clone();
    assert!(plan_http(contract, &[source], HttpConfig::default()).is_ok());
}

#[test]
fn malformed_servers_and_unsupported_fetch_methods_fail_before_emission() {
    for server in [
        "https://[::1",
        "https://example.com:bad",
        "https://user:password@example.com",
        "https://example.com/api?token=x",
        "https://example.com/api#section",
        "https://example.com\\other",
        "https://exa mple.com",
        "https://example.com\n",
    ] {
        let mut value = api(json!({}));
        value["servers"][0]["url"] = json!(server);
        let (contract, _) = fixture(value);
        let source = contract.operations().next().unwrap().source().clone();
        let errors = plan_http(contract, &[source], HttpConfig::default()).unwrap_err();
        assert!(
            errors.iter().any(|error| error.code == "http-server-url"),
            "{server}: {errors:?}"
        );
    }
    for method in ["head", "options", "trace"] {
        let mut value = api(json!({}));
        let operation = value["paths"]["/credits"]
            .as_object_mut()
            .unwrap()
            .remove("get")
            .unwrap();
        value["paths"]["/credits"][method] = operation;
        let (contract, _) = fixture(value);
        let source = contract.operations().next().unwrap().source().clone();
        let errors = plan_http(contract, &[source], HttpConfig::default()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.code == "http-capability-required"
                    && error.message.contains("AdditionalMethods")),
            "{method}: {errors:?}"
        );
    }
}

#[test]
fn source_named_credentials_and_configured_ceiling_are_enforced_by_generated_consumers() {
    let mut value = api(json!({}));
    value["security"] = json!([{"managementKey":[]}]);
    value["components"]["securitySchemes"] =
        json!({"managementKey":{"type":"http","scheme":"bearer"}});
    let (contract, _) = fixture(value);
    let source = contract.operations().next().unwrap().source().clone();
    let plan = plan_http(
        contract,
        &[source],
        HttpConfig {
            max_response_bytes: 256,
            ..HttpConfig::default()
        },
    )
    .unwrap();
    let operation = &plan.operations()[0];
    assert_eq!(operation.security_scheme_name, "managementKey");
    assert_eq!(
        operation.security_use_source.pointer(),
        "/security/0/managementKey"
    );
    assert_eq!(
        operation.security_definition_source.pointer(),
        "/components/securitySchemes/managementKey"
    );
    let directory = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&plan.render(), directory.path()).unwrap();
    let root = directory.path().join("typescript");
    std::fs::write(root.join("consumer.ts"), r#"
import { getCredits, isSdkError, type ClientOptions } from './operations.js';
declare function require(name: string): any;
const assert = require('node:assert/strict');
let requests = 0;
const client: ClientOptions = { auth: {managementKey:'fixture-secret'}, fetch: async (_url, init) => {
  requests++;
  assert.equal(new Headers(init?.headers).get('authorization'), 'Bearer fixture-secret');
  return new Response('{"data":"correct"}', {status:200,headers:{'content-type':'application/json'}});
}};
// @ts-expect-error: credentials use the source scheme's actual name
const wrongCredentials: ClientOptions = {auth:{apiKey:'wrong-field'}};
async function run() {
  const result = await getCredits(client, {});
  assert.equal(result.data.data, 'correct');
  await assert.rejects(getCredits({...client,maxResponseBytes:257}, {}), (error: unknown) => isSdkError(error) && error.kind === 'request-representation');
  assert.equal(requests, 1);
}
void run().catch((error: unknown) => { console.error(error); throw error; });
"#).unwrap();
    let compiler = Command::new("tsc")
        .current_dir(&root)
        .args([
            "--strict",
            "--exactOptionalPropertyTypes",
            "--noUncheckedIndexedAccess",
            "--target",
            "ES2022",
            "--module",
            "commonjs",
            "--outDir",
            "dist",
            "--pretty",
            "false",
            "consumer.ts",
        ])
        .output()
        .unwrap();
    assert!(
        compiler.status.success(),
        "{}{}",
        String::from_utf8_lossy(&compiler.stdout),
        String::from_utf8_lossy(&compiler.stderr)
    );
    let execution = Command::new("node")
        .current_dir(&root)
        .arg("dist/consumer.js")
        .output()
        .unwrap();
    assert!(
        execution.status.success(),
        "{}{}",
        String::from_utf8_lossy(&execution.stdout),
        String::from_utf8_lossy(&execution.stderr)
    );
}

#[test]
#[ignore = "requires the tracked openrouter-web checkout"]
fn tracked_openrouter_operations_and_query_inputs_plan_together() {
    let root = std::env::var_os("OPENROUTER_WEB_ROOT").expect("set OPENROUTER_WEB_ROOT");
    let path = std::path::Path::new(&root).join("projects/docs/openapi/openapi.yaml");
    let contract = {
        let workspace = Arc::new(
            WorkspaceBuilder::new()
                .root(path.parent().unwrap())
                .build()
                .unwrap(),
        );
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap())
    };
    let wanted = [
        "getCredits",
        "createKeys",
        "updateKeys",
        "listContainerFiles",
        "getContainerFile",
    ];
    let selected = contract
        .operations()
        .filter(|o| o.operation_id().is_some_and(|id| wanted.contains(&id)))
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), wanted.len());
    let plan = plan_http(contract, &selected, HttpConfig::default()).unwrap();
    assert_eq!(
        plan.operations()
            .iter()
            .map(|o| o.operation_id.as_str())
            .collect::<std::collections::BTreeSet<_>>(),
        wanted.into_iter().collect()
    );
}

#[test]
fn unsupported_query_wire_shapes_fail_at_the_parameter_before_artifacts() {
    for parameter in [
        json!({"name":"q","in":"query","explode":0,"schema":{"type":"string"}}),
        json!({"name":"q","in":"query","style":"deepObject","schema":{"type":"object"}}),
        json!({"name":"q","in":"query","style":"spaceDelimited","schema":{"type":"array","items":{"type":"string"}}}),
        json!({"name":"q","in":"query","style":"pipeDelimited","schema":{"type":"array","items":{"type":"string"}}}),
        json!({"name":"q","in":"query","allowReserved":true,"schema":{"type":"string"}}),
        json!({"name":"q","in":"query","allowEmptyValue":true,"schema":{"type":"string"}}),
        json!({"name":"q","in":"query","content":{"application/json":{"schema":{"type":"string"}}}}),
        json!({"name":"q","in":"query","schema":{"type":["integer","null"]}}),
        json!({"name":"q","in":"query","schema":{"type":"object","additionalProperties":{"type":"string"}}}),
        json!({"name":"q","in":"query","schema":{"type":"array","items":{"type":"array","items":{"type":"string"}}}}),
        json!({"name":"q","in":"query","schema":{"type":"array","items":{"type":["string","null"]}}}),
        json!({"name":"q","in":"query","schema":{"anyOf":[{"type":"string"},{"type":"boolean"}]}}),
    ] {
        let (contract, _) = fixture(api(json!({"parameters":[parameter]})));
        let source = contract.operations().next().unwrap().source().clone();
        let errors = plan_http(contract, &[source], HttpConfig::default()).unwrap_err();
        assert!(
            errors.iter().any(|error| matches!(
                error.code,
                "http-metadata-boolean"
                    | "http-capability-required"
                    | "http-parameter-combination-undefined"
                    | "http-empty-value-policy"
                    | "http-null-wire-policy"
                    | "http-scalar-shape-unsupported"
                    | "http-parameter-shape-unsupported"
            ) && error
                .source
                .pointer()
                .starts_with("/paths/~1credits/get/parameters/0")),
            "{errors:?}"
        );
    }
}

#[test]
fn query_names_do_not_bind_path_placeholders() {
    let mut value = api(
        json!({"parameters":[{"name":"id","in":"query","required":true,"schema":{"type":"string"}}]}),
    );
    let operation = value["paths"]
        .as_object_mut()
        .unwrap()
        .remove("/credits")
        .unwrap();
    value["paths"]["/credits/{id}"] = operation;
    let (contract, _) = fixture(value);
    let source = contract.operations().next().unwrap().source().clone();
    let errors = plan_http(contract, &[source], HttpConfig::default()).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.code == "http-path-parameters"
                && error.source.pointer() == "/paths/~1credits~1{id}")
    );
}
