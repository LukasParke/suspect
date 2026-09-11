#![cfg(feature = "java-sdk")]
//! Independent native protocol gates. Expectations are literal HTTP bytes and
//! schema outcomes, never the shared serializer's output.
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::{
    http_protocol as wire,
    java_sdk::{self, MavenConfig, PackageConfig, ProtocolConfig},
};
use suspect_ir::contract::{Contract, SourceId};
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
fn selected(contract: &Contract) -> Vec<SourceId> {
    contract.operations().map(|o| o.source().clone()).collect()
}
fn crate_root() -> PathBuf {
    std::env::var_os("SUSPECT_JAVA_CRATE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_MANIFEST_DIR").into())
}
fn root() -> PathBuf {
    let base = crate_root().join("../../target/sdk-java-protocol/java");
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix("case-")
        .tempdir_in(base)
        .unwrap()
        .keep()
}
fn response(schema: Value) -> Value {
    json!({"description":"OK","content":{"application/json":{"schema":schema}}})
}
fn reply() -> Value {
    json!({"200":response(json!({"$ref":"#/components/schemas/Reply"}))})
}
fn op(id: &str) -> Value {
    json!({"operationId":id,"security":[],"responses":reply()})
}
fn styled_parts() -> Value {
    let color = json!({"type":"object","required":["g","r"],"properties":{"g":{"type":"integer","example":2},"r":{"type":"integer","example":1}},"additionalProperties":false});
    json!({"schema":{"type":"object","required":["list","color","deep","space","pipe","plain"],"properties":{"list":{"type":"array","items":{"type":"string","example":"blue"},"minItems":1},"color":color,"deep":color,"space":{"type":"array","items":{"type":"string","example":"a"},"minItems":1},"pipe":{"type":"array","items":{"type":"string","example":"a"},"minItems":1},"plain":{"type":"string","example":"text"}},"additionalProperties":false},"encoding":{"list":{"style":"form","explode":true},"color":{"style":"form","explode":true},"deep":{"style":"deepObject","explode":true},"space":{"style":"spaceDelimited","explode":false},"pipe":{"style":"pipeDelimited","explode":false},"plain":{"style":"form"}}})
}
fn fixture() -> Value {
    let mut paths = serde_json::Map::new();
    paths.insert("/public".into(), json!({"get":op("publicValue")}));
    paths.insert("/snow/雪/😀".into(), json!({"get":op("unicodePath")}));
    for (path, id, security) in [
        ("/optional", "optionalAuth", json!([{"bearer":[]},{}])),
        (
            "/and",
            "andAuth",
            json!([{"headerKey":[],"queryKey":[],"cookieKey":[]} ]),
        ),
        ("/or", "orAuth", json!([{"basic":[]},{"bearer":["reader"]}])),
        ("/oauth", "oauthValue", json!([{"oauth":["read"]}])),
        ("/oidc", "oidcValue", json!([{"oidc":["openid"]}])),
    ] {
        let mut value = op(id);
        value["security"] = security;
        paths.insert(path.into(), json!({"get":value}));
    }
    let mut servers = op("serverChoice");
    servers["servers"] = json!([{"url":"../{version}","name":"relative","variables":{"version":{"default":"v1","enum":["v1","v2"]}}},{"url":"https://{region}.example.test/{version}","name":"regional","variables":{"region":{"default":"eu","enum":["eu","us"]},"version":{"default":"v1"}}}]);
    paths.insert("/server".into(), json!({"get":servers}));
    let mut default = op("defaultServer");
    default["servers"] = json!([]);
    paths.insert("/default-server".into(), json!({"get":default}));
    let mut styles = op("styles");
    styles["parameters"] = json!([
        {"name":"label","in":"path","required":true,"style":"label","explode":true,"schema":{"type":"array","items":{"type":"string"}}},
        {"name":"matrix","in":"path","required":true,"style":"matrix","explode":true,"schema":{"$ref":"#/components/schemas/Color"}},
        {"name":"simple","in":"path","required":true,"style":"simple","schema":{"type":"string"}},
        {"name":"color","in":"query","explode":false,"schema":{"$ref":"#/components/schemas/Color"}},
        {"name":"multi","in":"query","schema":{"type":"array","items":{"type":"string"}}},
        {"name":"spaces","in":"query","style":"spaceDelimited","schema":{"type":"array","items":{"type":"string"}}},
        {"name":"pipes","in":"query","style":"pipeDelimited","schema":{"type":"array","items":{"type":"string"}}},
        {"name":"filter","in":"query","style":"deepObject","schema":{"$ref":"#/components/schemas/Color"}},
        {"name":"reserved","in":"query","allowReserved":true,"schema":{"type":"string"}},
        {"name":"content","in":"query","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Payload"}}}},
        {"name":"X-Flag","in":"header","schema":{"type":"boolean"}},
        {"name":"X-Tags","in":"header","schema":{"type":"array","items":{"type":"string"}}},
        {"name":"sid","in":"cookie","schema":{"type":"string"}},
        {"name":"crumb","in":"cookie","style":"cookie","schema":{"type":"array","items":{"type":"string"}}}
    ]);
    paths.insert(
        "/styles/{label}/{matrix}/{simple}".into(),
        json!({"get":styles}),
    );
    for (path, id, media, schema) in [
        (
            "/whole",
            "wholeQuery",
            "application/json",
            json!({"$ref":"#/components/schemas/Payload"}),
        ),
        (
            "/whole-text",
            "wholeText",
            "text/plain",
            json!({"type":"string"}),
        ),
        (
            "/whole-form",
            "wholeForm",
            "application/x-www-form-urlencoded",
            json!({"type":"object","properties":{"foo":{"type":"string"},"flag":{"type":"boolean"}},"required":["foo","flag"],"additionalProperties":false}),
        ),
    ] {
        let mut value = op(id);
        value["parameters"] = json!([{"name":"complete","in":"querystring","required":true,"content":{media:{"schema":schema}}}]);
        paths.insert(path.into(), json!({"get":value}));
    }
    let mut choose = op("chooseResponse");
    choose["responses"] = json!({
        "200":{"description":"exact","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Reply"}},"application/json;profile=Exact":{"schema":{"$ref":"#/components/schemas/Payload"}},"text/plain":{"schema":{"type":"integer"}},"application/octet-stream":{"schema":{"maxLength":50}},"image/*":{},"*/*":{}}},
        "2XX":{"description":"range","content":{"text/plain":{"schema":{"type":"string"}}}},"default":{"description":"fallback","content":{"application/problem+json":{"schema":{"$ref":"#/components/schemas/Payload"}}}}
    });
    paths.insert("/choose".into(), json!({"get":choose}));
    let mut fallback = op("defaultResponse");
    fallback["responses"] = json!({"default":{"description":"actual status","content":{"text/plain":{"schema":{"type":"string"}}}}});
    paths.insert("/fallback".into(), json!({"get":fallback}));
    let mut unspecified = op("unspecified");
    unspecified["responses"] = json!({"200":{"description":"undeclared bytes"},"418":{"description":"undeclared error bytes"},"204":{"description":"none","content":{"application/json":{"schema":false}}},"205":{"description":"none"},"304":{"description":"none"},"103":{"description":"none"}});
    paths.insert("/unspecified".into(), json!({"get":unspecified}));
    paths.insert(
        "/unknown".into(),
        json!({"get":{"operationId":"undeclaredResponses","security":[]}}),
    );
    let mut metadata = op("readMetadata");
    metadata["responses"]["200"]["headers"] = json!({"X-Count":{"required":true,"schema":{"type":"integer","minimum":0}},"X-Flags":{"schema":{"type":"array","items":{"type":"boolean"}}},"X-Color":{"explode":true,"schema":{"$ref":"#/components/schemas/Color"}},"X-JSON":{"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Payload"}}}},"X-Text-Int":{"content":{"text/plain":{"schema":{"type":"integer","minimum":0}}}},"X-Text-Bool":{"content":{"text/plain":{"schema":{"type":"boolean"}}}}});
    metadata["responses"]["200"]["links"] = json!({"next":{"operationId":"publicValue","parameters":{"id":"$response.header.X-Count","literal":{"$ref":"instance-data"}}}});
    paths.insert("/metadata".into(), json!({"get":metadata}));
    let mut head = op("headValue");
    head["responses"] = json!({"200":{"description":"metadata","headers":{"X-Count":{"required":true,"schema":{"type":"integer"}}},"content":{"application/json":{"schema":false}}}});
    paths.insert("/head".into(), json!({"head":head}));
    for method in ["put", "delete", "options", "trace", "query"] {
        paths.insert(
            format!("/method/{method}"),
            json!({method:op(&format!("{method}Method"))}),
        );
    }
    paths.insert("/custom".into(),json!({"additionalOperations":{"COPY":op("copyMethod"),"GeT":op("mixedGet"),"get":op("lowerGet"),"head":op("lowerHead"),"x-PING":op("customPing")}}));
    let mut body = op("sendBody");
    body["requestBody"] = json!({"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Payload"}},"text/plain":{"schema":{"type":"integer"}},"*/*":{}}});
    paths.insert("/body".into(), json!({"post":body}));
    let form = json!({"schema":{"type":"object","properties":{"title":{"type":"string","minLength":1,"example":"title"},"values":{"type":"array","minItems":1,"maxItems":3,"items":{"type":"integer","minimum":0,"example":2}},"payload":{"$ref":"#/components/schemas/Payload"},"codes":{"type":"array","items":{"type":"string"}}},"required":["title"],"additionalProperties":false,"maxProperties":4},"encoding":{"codes":{"style":"form","explode":false}}});
    let mut submit = op("submitForm");
    submit["requestBody"] =
        json!({"required":true,"content":{"application/x-www-form-urlencoded":form}});
    paths.insert("/form".into(), json!({"post":submit}));
    let mut read = op("readForm");
    read["responses"] =
        json!({"200":{"description":"form","content":{"application/x-www-form-urlencoded":form}}});
    paths.insert("/form-response".into(), json!({"get":read}));
    let multipart = json!({"schema":{"type":"object","properties":{"file":{"maxLength":32},"title":{"type":"string","minLength":1,"example":"title"},"payload":{"$ref":"#/components/schemas/Payload"},"tags":{"type":"array","minItems":1,"maxItems":2,"items":{"type":"string","example":"tag"}}},"required":["file","title"],"additionalProperties":false,"maxProperties":4},"encoding":{"file":{"contentType":"application/octet-stream","headers":{"X-Size":{"required":true,"schema":{"type":"integer","minimum":0,"example":2}}}},"payload":{"contentType":"application/json"}}});
    let mut upload = op("upload");
    upload["requestBody"] = json!({"required":true,"content":{"multipart/form-data":multipart}});
    paths.insert("/upload".into(), json!({"post":upload}));
    let mut parts = op("downloadParts");
    parts["responses"] =
        json!({"200":{"description":"parts","content":{"multipart/form-data":multipart}}});
    paths.insert("/parts".into(), json!({"get":parts}));
    let positional = json!({"schema":{"type":"array","minItems":1,"maxItems":4,"prefixItems":[{"type":"string","example":"prefix"},{"maxLength":8}],"items":{"$ref":"#/components/schemas/Line"}},"prefixEncoding":[{"contentType":"text/plain"},{"contentType":"application/octet-stream"}],"itemEncoding":{"contentType":"application/json"}});
    let mut position = op("sendPositional");
    position["requestBody"] = json!({"required":true,"content":{"multipart/mixed":positional}});
    paths.insert("/positional".into(), json!({"post":position}));
    let mut position = op("readPositional");
    position["responses"] =
        json!({"200":{"description":"parts","content":{"multipart/mixed":positional}}});
    paths.insert("/positional-response".into(), json!({"get":position}));
    for (path, id, media, schema) in [
        (
            "/events",
            "events",
            "text/event-stream",
            json!({"$ref":"#/components/schemas/Event","maxProperties":4}),
        ),
        (
            "/lines",
            "lines",
            "application/x-ndjson",
            json!({"$ref":"#/components/schemas/Line"}),
        ),
        (
            "/nullable-lines",
            "nullableLines",
            "application/jsonl",
            json!({"type":["string","null"]}),
        ),
    ] {
        let mut value = op(id);
        value["responses"] = json!({"200":{"description":"items","content":{media:{"itemSchema":schema}}},"400":response(json!({"$ref":"#/components/schemas/Payload"}))});
        paths.insert(path.into(), json!({"get":value}));
    }
    let mut failed_stream = op("errorEvents");
    failed_stream["responses"] = json!({"400":{"description":"source-declared error stream","content":{"text/event-stream":{"itemSchema":{"$ref":"#/components/schemas/Event"}}}}});
    paths.insert("/error-events".into(), json!({"get":failed_stream}));
    let mut events = op("sendEvents");
    events["requestBody"] = json!({"required":true,"content":{"text/event-stream":{"itemSchema":{"$ref":"#/components/schemas/Event"}}}});
    paths.insert("/send-events".into(), json!({"post":events}));
    let mut lines = op("sendLines");
    lines["requestBody"] = json!({"required":true,"content":{"application/x-ndjson":{"itemSchema":{"$ref":"#/components/schemas/Line"}}}});
    paths.insert("/send-lines".into(), json!({"post":lines}));
    let mut legacy = op("legacy30");
    legacy["requestBody"] = json!({"$ref":"legacy30.json#/components/requestBodies/Value"});
    paths.insert("/legacy30".into(), json!({"post":legacy}));
    let mut legacy = op("legacy30File");
    legacy["responses"] = json!({"200":{"$ref":"legacy30.json#/components/responses/File"}});
    paths.insert("/legacy30-file".into(), json!({"get":legacy}));
    let mut legacy = op("form31");
    legacy["requestBody"] = json!({"$ref":"form31.json#/components/requestBodies/Form"});
    paths.insert("/form31".into(), json!({"post":legacy}));
    let mut response_parts = styled_parts();
    for name in ["list", "pipe", "space"] {
        let item = response_parts["schema"]["properties"][name].clone();
        response_parts["schema"]["properties"][name] =
            json!({"type":"array","minItems":1,"maxItems":2,"items":item});
    }
    let mut styled = op("styledParts");
    styled["requestBody"] = json!({"$ref":"form31.json#/components/requestBodies/Styled"});
    styled["responses"] = json!({"200":{"description":"3.2 per-item composite parts","content":{"multipart/form-data":response_parts}}});
    paths.insert("/styled-parts".into(), json!({"post":styled}));
    let extras = json!({"schema":{"type":"object","required":["b","required-extra"],"properties":{"b":{"type":"string","example":"base"},"additional":{"type":"string","example":"optional"}},"additionalProperties":{"type":"string","example":"extra"},"minProperties":2,"maxProperties":3}});
    let mut extra = op("extraParts");
    extra["requestBody"] = json!({"required":true,"content":{"multipart/form-data":extras}});
    extra["responses"] =
        json!({"200":{"description":"typed extras","content":{"multipart/form-data":extras}}});
    paths.insert("/extra-parts".into(), json!({"post":extra}));
    json!({"openapi":"3.2.0","info":{"title":"Java independent protocol fixtures","version":"1"},"servers":[{"url":"https://example.test/api/v1"}],"paths":paths,"components":{"schemas":{
        "Reply":{"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false},
        "Payload":{"type":"object","properties":{"name":{"type":"string","minLength":1,"example":"example"},"amount":{"type":"number"}},"required":["name"],"additionalProperties":false},
        "Color":{"type":"object","properties":{"B":{"type":"integer"},"G":{"type":"integer"},"R":{"type":"integer"}},"required":["B","G","R"],"additionalProperties":false},
        "Event":{"type":"object","required":["data"],"properties":{"data":{"type":"string","example":"event","contentMediaType":"application/json","contentSchema":{"type":"object"}},"event":{"type":"string"},"id":{"type":"string"},"retry":{"type":"integer","minimum":0}},"additionalProperties":false},
        "Line":{"type":"object","required":["n"],"properties":{"n":{"type":"integer","example":1},"exact":{"type":"number"}},"additionalProperties":false}
    },"securitySchemes":{"bearer":{"type":"http","scheme":"BeArEr"},"basic":{"type":"http","scheme":"basic"},"headerKey":{"type":"apiKey","in":"header","name":"X-Key"},"queryKey":{"type":"apiKey","in":"query","name":"key"},"cookieKey":{"type":"apiKey","in":"cookie","name":"session"},"oauth":{"type":"oauth2","oauth2MetadataUrl":"https://auth.example/metadata","flows":{"authorizationCode":{"authorizationUrl":"https://auth.example/authorize","tokenUrl":"https://auth.example/token","refreshUrl":"https://auth.example/refresh","scopes":{"read":"Read"}}}},"oidc":{"type":"openIdConnect","openIdConnectUrl":"https://auth.example/.well-known/openid-configuration"}}}})
}
fn plan_at(root: &Path) -> java_sdk::SdkPlan {
    let legacy = json!({"openapi":"3.0.4","info":{"title":"Actual external OAS 3.0 context","version":"1"},"paths":{},"components":{
        "schemas":{"LegacyText":{"type":"string"},"LegacyBase":{"type":"object","required":["name","note"],"properties":{"name":{"type":"string"},"note":{"$ref":"#/components/schemas/LegacyText","type":"integer"},"amount":{"type":"number","nullable":true,"minimum":0,"exclusiveMinimum":true}}}},
        "requestBodies":{"Value":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/LegacyBase","type":"string","properties":{"ignored":{"dependentRequired":{"a":["b"]}}}}}}}},
        "responses":{"File":{"description":"3.0 binary","content":{"application/octet-stream":{"schema":{"type":"string","format":"binary"}}}}}
    }});
    std::fs::write(root.join("legacy30.json"), legacy.to_string()).unwrap();
    let styled = styled_parts();
    let form31 = json!({
        "openapi":"3.1.2","info":{"title":"3.1 whole-array style","version":"1"},"paths":{},
        "components":{
            "requestBodies":{
                "Form":{"required":true,"content":{"application/x-www-form-urlencoded":{
                    "schema":{"type":"object","required":["codes"],"properties":{"codes":{"type":"array","items":{"type":"string"},"minItems":1}},"additionalProperties":false},
                    "encoding":{"codes":{"style":"form","explode":false}}
                }}},
                "Styled":{"required":true,"content":{"multipart/form-data":styled}}
            }
        }
    });
    std::fs::write(root.join("form31.json"), form31.to_string()).unwrap();
    let path = root.join("api.json");
    std::fs::write(&path, fixture().to_string()).unwrap();
    let c = load(&path);
    java_sdk::plan_sdk_with_protocol(
        c.clone(),
        &selected(&c),
        PackageConfig {
            package: "example.protocol".into(),
            version: "1.0.0".into(),
            api_name: "Client".into(),
        },
        &[],
        MavenConfig {
            group_id: Some("example.protocol".into()),
            artifact_id: "java-protocol".into(),
            ..Default::default()
        },
        ProtocolConfig::default(),
    )
    .unwrap()
}

#[test]
fn protocol_admission_has_native_byte_part_item_roots_and_actual_statuses() {
    let root = root();
    let plan = plan_at(&root);
    assert!(plan.protocol().is_admitted());
    assert!(
        plan.protocol()
            .capabilities()
            .supports(wire::Capability::PositionalMultipart)
    );
    assert!(plan.protocol().capabilities().profiles().is_empty());
    let upload = plan
        .operations()
        .iter()
        .find(|o| o.operation_id == "upload")
        .unwrap();
    let body = upload.body.as_ref().unwrap();
    let java_sdk::protocol::JavaValue::Aggregate(parts) = &body.media[0].value else {
        panic!()
    };
    let file = parts
        .parts
        .iter()
        .find(|p| p.wire.name() == Some("file"))
        .unwrap();
    assert!(matches!(file.value, java_sdk::protocol::JavaValue::Bytes));
    assert!(
        !plan
            .protocol()
            .codec_roots()
            .contains(file.wire.schema().id())
    );
    assert!(plan.operations().iter().any(|o| o.http_method == "head"));
    let files = plan.render().unwrap();
    assert!(files.iter().all(|f| f.path.starts_with("java/")));
    assert!(files.iter().any(|f| f.path.ends_with("/EventStream.java")));
    assert!(
        files
            .iter()
            .any(|f| f.path.ends_with("/protocol-program.json"))
    );
    assert_eq!(files, plan.render().unwrap());
}
#[test]
fn rich_compatibility_retains_actual_media_part_header_and_stream_signatures() {
    use suspect_codegen::{
        backend::{Backend, TargetConfig},
        compatibility,
    };
    let root = root();
    let plan = plan_at(&root);
    let target = TargetConfig {
        backend: Backend::JavaHttp,
        package_name: "example.protocol:java-protocol".into(),
        package_version: "1.0.0".into(),
        import_name: Some("example.protocol".into()),
    };
    let snapshot =
        compatibility::snapshot(plan.contract().clone(), &[], std::slice::from_ref(&target))
            .unwrap()
            .native
            .remove(0);
    assert_eq!(
        snapshot.status,
        compatibility::PlanStatus::Planned,
        "{:?}",
        snapshot.findings
    );
    let model = |name: &str| {
        snapshot
            .models
            .iter()
            .find(|m| m.name == format!("example.protocol.{name}"))
            .unwrap()
            .descriptor
            .as_ref()
            .unwrap()
    };
    let file = model("UploadMultipartFilePart");
    assert_eq!(
        file["constructor"]["parameters"][0]["type"]["name"],
        "example.protocol.Bytes"
    );
    assert_eq!(
        file["constructor"]["parameters"][1]["type"]["name"],
        "example.protocol.UploadMultipartFileHeaders"
    );
    let headers = model("UploadMultipartFileHeaders");
    assert_eq!(
        headers["fields"][0]["getter"]["returns"]["name"],
        "example.protocol.JsonRuntime.JsonNumber"
    );
    assert_eq!(
        headers["builder"]["build"]["returns"]["name"],
        "example.protocol.UploadMultipartFileHeaders"
    );
    let extras = model("ExtraPartsMultipart");
    assert_eq!(extras["additional"]["name"], "putAdditionalPart");
    assert_eq!(
        extras["additional"]["parameters"][1]["type"]["name"],
        "example.protocol.ExtraPartsMultipartAdditionalPart"
    );
    let body = model("SendBodyBody");
    let json = body["variants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["contentType"] == "application/json")
        .unwrap();
    assert_eq!(json["constructors"].as_array().unwrap().len(), 2);
    assert_eq!(
        json["constructors"][0]["parameters"][0]["type"]["name"],
        "example.protocol.Payload"
    );
    let response = model("ChooseResponseResponse200Body");
    assert!(
        response["variants"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["constructorAccess"] == "package"
                && v["constructors"].as_array().unwrap().is_empty())
    );
    let stream = snapshot
        .operations
        .iter()
        .find(|o| o.operation_id == "nullableLines")
        .unwrap();
    assert_eq!(
        stream.descriptor["responses"][0]["data"]["name"],
        "example.protocol.EventStream"
    );
    assert_eq!(
        stream.descriptor["responses"][0]["data"]["arguments"][0]["kind"],
        "nullable"
    );
    assert!(
        snapshot
            .models
            .iter()
            .filter(|m| m.role == "wire-model")
            .all(|m| !m
                .descriptor
                .as_ref()
                .unwrap()
                .to_string()
                .contains("/operations/")),
        "runtime graph offsets leaked into API identity"
    );
    let report = compatibility::compare(
        plan.contract().clone(),
        plan.contract().clone(),
        &[],
        &[target],
    )
    .unwrap();
    assert!(
        report.native[0].changes.is_empty(),
        "{:?}",
        report.native[0].changes
    );
}
#[test]
fn undefined_wire_profiles_keep_source_linked_refusals() {
    let cases = [
        (
            "set-cookie",
            json!({"get":{"operationId":"x","responses":{"200":{"description":"x","headers":{"Set-Cookie":{"schema":{"type":"string"}}}}}}}),
            "java-set-cookie-header-unsupported",
        ),
        (
            "header-extras",
            json!({"get":{"operationId":"x","responses":{"200":{"description":"x","headers":{"X-Object":{"schema":{"type":"object"}}}}}}}),
            "java-header-decoding-ambiguous",
        ),
        (
            "connect",
            json!({"additionalOperations":{"CONNECT":{"operationId":"x","responses":{"200":{"description":"tunnel"}}}}}),
            "java-connect-tunnel-unsupported",
        ),
        (
            "unbounded-positional",
            json!({"post":{"operationId":"x","requestBody":{"content":{"multipart/mixed":{"schema":{"type":"array","items":{}} ,"itemEncoding":{"contentType":"application/octet-stream"}}}},"responses":{"200":{"description":"ok"}}}}),
            "java-positional-multipart-limit",
        ),
    ];
    for (label, path, code) in cases {
        let root = root();
        let source = root.join("api.json");
        std::fs::write(&source,json!({"openapi":"3.2.0","info":{"title":"Explicit boundary","version":"1"},"servers":[{"url":"https://example.test"}],"paths":{"/x":path}}).to_string()).unwrap();
        let c = load(&source);
        let errors = java_sdk::plan_sdk(c.clone(), &selected(&c), PackageConfig::default(), &[])
            .unwrap_err();
        assert!(
            errors.iter().any(|d| (d.code == code
                || label == "connect" && d.code == "http-connect-tunnel-unsupported")
                && d.at.end > d.at.start
                && d.source.pointer().starts_with("/paths/")),
            "{label}: {errors:?}"
        );
    }
}
fn checked(command: &mut Command, root: &Path) {
    let result = command.output().unwrap();
    let logs = root.join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    let number = std::fs::read_dir(&logs).unwrap().count();
    std::fs::write(
        logs.join(format!("{number:03}.log")),
        format!(
            "{command:?}\nstatus={}\n{}{}",
            result.status,
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        ),
    )
    .unwrap();
    assert!(
        result.status.success(),
        "{}\n{command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
fn java() -> PathBuf {
    std::env::var_os("JAVA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            "/Users/luke/.local/share/mise/installs/java/temurin-21.0.12+101.0.LTS".into()
        })
}
fn normative_vectors(root: &Path) {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/http-protocol-v1.json")).unwrap();
    let mut cases = Vec::new();
    for (i, test) in fixture["parameterCases"]
        .as_array()
        .unwrap()
        .iter()
        .chain(fixture["querystringCases"].as_array().unwrap())
        .enumerate()
    {
        let querystring = test["parameter"]["in"] == "querystring";
        let version = if querystring {
            "3.2.0"
        } else {
            test["version"].as_str().unwrap_or("3.1.2")
        };
        let path = if test["parameter"]["in"] == "path" {
            "/items/{color}"
        } else {
            "/items"
        };
        let document = json!({"openapi":version,"info":{"title":"Independent normative wire case","version":"1"},"paths":{path:{"get":{"operationId":format!("case{i}"),"parameters":[test["parameter"]],"responses":{"200":{"description":"OK"}}}}}});
        let file = root.join(format!("normative-{i}.json"));
        std::fs::write(&file, document.to_string()).unwrap();
        let contract = load(&file);
        let protocol = wire::plan(
            &contract,
            &selected(&contract),
            java_sdk::protocol::capabilities(&ProtocolConfig::default()),
        )
        .into_result()
        .unwrap();
        cases.push(json!({"name":format!("case{i}"),"descriptor":protocol.operations()[0].parameters()[0],"value":test["value"],"expected":test["wire"]}));
    }
    let response = root.join("normative-responses.json");
    std::fs::write(
        &response,
        fixture["responseSelection"]["document"].to_string(),
    )
    .unwrap();
    let contract = load(&response);
    let plan = wire::plan(
        &contract,
        &selected(&contract),
        java_sdk::protocol::capabilities(&ProtocolConfig::default()),
    )
    .into_result()
    .unwrap();
    std::fs::write(root.join("normative-wire.json"),json!({"parameters":cases,"responses":plan.operations()[0],"responseCases":fixture["responseSelection"]["cases"]}).to_string()).unwrap();
}
#[test]
#[ignore = "requires JDK21/25 and Maven; new full protocol artifact compilation"]
fn native_protocol_package() {
    let root = root();
    let plan = plan_at(&root);
    for file in plan.render().unwrap() {
        let path = root.join(file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, file.content).unwrap();
    }
    let gates = std::env::var("SUSPECT_JAVA_PROTOCOL_GATES").ok();
    let gate = |name: &str| {
        gates
            .as_ref()
            .is_none_or(|g| g.split(',').any(|v| v == name))
    };
    let maven = std::env::var_os("SUSPECT_MAVEN_BIN").unwrap_or_else(|| {
        "/Users/luke/.local/share/mise/installs/maven/3.9.16/apache-maven-3.9.16/bin/mvn".into()
    });
    checked(
        Command::new(maven)
            .args(["-B", "-q", "install"])
            .arg(format!(
                "-Dmaven.repo.local={}",
                crate_root()
                    .join("../../target/sdk-java-maven-cache/java/repository")
                    .display()
            ))
            .env("JAVA_HOME", java())
            .current_dir(root.join("java")),
        &root,
    );
    let jar = root.join("java/target/java-protocol-1.0.0.jar");
    if gate("examples") {
        checked(
            Command::new(java().join("bin/java"))
                .args([
                    "-ea",
                    "-cp",
                    "target/java-protocol-1.0.0.jar",
                    "example.protocol.SdkExamples",
                ])
                .current_dir(root.join("java")),
            &root,
        );
    }
    if gate("wire") {
        std::fs::write(
            root.join("NativeProtocol.java"),
            include_str!("../src/java_sdk/NativeProtocol.java"),
        )
        .unwrap();
        checked(
            Command::new(java().join("bin/javac"))
                .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
                .arg(&jar)
                .arg("NativeProtocol.java")
                .current_dir(&root),
            &root,
        );
        checked(
            Command::new(java().join("bin/java"))
                .args(["-ea", "-cp"])
                .arg(format!("{}:{}", jar.display(), root.display()))
                .arg("NativeProtocol")
                .current_dir(&root),
            &root,
        );
    }
    if gate("controls") || gate("edges") || gate("multipart") || gate("lifetime") {
        std::fs::write(
            root.join("NativeSupport.java"),
            include_str!("../src/java_sdk/NativeSupport.java"),
        )
        .unwrap();
        std::fs::write(
            root.join("NativeProtocolControls.java"),
            include_str!("../src/java_sdk/NativeProtocolControls.java"),
        )
        .unwrap();
        checked(
            Command::new(java().join("bin/javac"))
                .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
                .arg(&jar)
                .args(["NativeSupport.java", "NativeProtocolControls.java"])
                .current_dir(&root),
            &root,
        );
        if gate("controls") {
            checked(
                Command::new(java().join("bin/java"))
                    .args(["-ea", "-cp"])
                    .arg(format!("{}:{}", jar.display(), root.display()))
                    .arg("NativeProtocolControls")
                    .current_dir(&root),
                &root,
            );
        }
    }
    if gate("edges") {
        std::fs::write(
            root.join("NativeProtocolEdges.java"),
            include_str!("../src/java_sdk/NativeProtocolEdges.java"),
        )
        .unwrap();
        checked(
            Command::new(java().join("bin/javac"))
                .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
                .arg(format!("{}:{}", jar.display(), root.display()))
                .arg("NativeProtocolEdges.java")
                .current_dir(&root),
            &root,
        );
        checked(
            Command::new(java().join("bin/java"))
                .args(["-ea", "-cp"])
                .arg(format!("{}:{}", jar.display(), root.display()))
                .arg("NativeProtocolEdges")
                .current_dir(&root),
            &root,
        );
    }
    if gate("multipart") {
        std::fs::write(
            root.join("NativeMultipart.java"),
            include_str!("../src/java_sdk/NativeMultipart.java"),
        )
        .unwrap();
        checked(
            Command::new(java().join("bin/javac"))
                .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
                .arg(format!("{}:{}", jar.display(), root.display()))
                .arg("NativeMultipart.java")
                .current_dir(&root),
            &root,
        );
        checked(
            Command::new(java().join("bin/java"))
                .args(["-ea", "-cp"])
                .arg(format!("{}:{}", jar.display(), root.display()))
                .arg("NativeMultipart")
                .current_dir(&root),
            &root,
        );
    }
    if gate("lifetime") {
        std::fs::write(
            root.join("NativeStreamLifetime.java"),
            include_str!("../src/java_sdk/NativeStreamLifetime.java"),
        )
        .unwrap();
        checked(
            Command::new(java().join("bin/javac"))
                .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
                .arg(format!("{}:{}", jar.display(), root.display()))
                .arg("NativeStreamLifetime.java")
                .current_dir(&root),
            &root,
        );
        checked(
            Command::new(java().join("bin/java"))
                .args(["-ea", "-cp"])
                .arg(format!("{}:{}", jar.display(), root.display()))
                .arg("NativeStreamLifetime")
                .current_dir(&root),
            &root,
        );
    }
    if gate("types") {
        let examples = root.join("java/examples/GettingStarted.java");
        if examples.is_file() {
            checked(
                Command::new(java().join("bin/javac"))
                    .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
                    .arg(&jar)
                    .arg("-d")
                    .arg(&root)
                    .arg(&examples),
                &root,
            );
        }
        for (index,body) in [
            "new SendBodyBody.Binary(JsonNull.INSTANCE, \"application/octet-stream\");",
            "new SendBodyBody.Json(Bytes.empty());",
            "UploadMultipartFilePart.builder(JsonNull.INSTANCE, UploadMultipartFileHeaders.builder(JsonNumber.of(1)).build());",
            "String value=client.defaultResponse().data();",
            "JsonNull value=client.headValue().data();",
            "EventStream<String> value=client.nullableLines().data(); value.subscribe(new java.util.concurrent.Flow.Subscriber<String>() { public void onSubscribe(java.util.concurrent.Flow.Subscription s){} public void onNext(String value){} public void onError(Throwable e){} public void onComplete(){} });",
            "new ChooseResponseResponse200Body.Json(Reply.builder(true).build(), \"application/json\");",
            "UploadMultipart.builder(UploadMultipartTitlePart.builder(\"title\").build());",
        ].iter().enumerate(){
            let file=root.join(format!("Negative{index}.java"));std::fs::write(&file,format!("import example.protocol.*; import example.protocol.Client.*; import static example.protocol.JsonRuntime.*; final class Negative{index} {{ void test(Client client) {{ {body} }} }}")).unwrap();
            let output=Command::new(java().join("bin/javac")).args(["--release","21","-cp"]).arg(&jar).arg(&file).output().unwrap();
            std::fs::write(root.join(format!("negative-{index}.log")),format!("status={}\n{}{}",output.status,String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr))).unwrap();
            assert!(!output.status.success(),"negative native consumer {index} compiled");assert!(String::from_utf8_lossy(&output.stderr).contains("error:"));
        }
    }
    if gate("normative") {
        normative_vectors(&root);
        std::fs::write(
            root.join("NativeNormative.java"),
            include_str!("../src/java_sdk/NativeNormative.java"),
        )
        .unwrap();
        checked(
            Command::new(java().join("bin/javac"))
                .args(["--release", "21", "-Xlint:all", "-Werror", "-d", ".", "-cp"])
                .arg(&jar)
                .arg("NativeNormative.java")
                .current_dir(&root),
            &root,
        );
        checked(
            Command::new(java().join("bin/java"))
                .args(["-ea", "-cp"])
                .arg(format!("{}:{}", jar.display(), root.display()))
                .args(["example.protocol.NativeNormative", "normative-wire.json"])
                .current_dir(&root),
            &root,
        );
    }
    println!("JAVA_PROTOCOL_PACKAGE {}", root.display());
}

#[test]
#[ignore = "new actual OpenRouter operations; requires JDK21/25 and Maven"]
fn native_protocol_openrouter() {
    let root = root();
    let source = std::env::var_os("SUSPECT_OPENROUTER_OPENAPI")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            "/Users/luke/github/openrouter-web/projects/docs/openapi/openapi.yaml".into()
        });
    let contract = load(&source);
    let operations = contract
        .operations()
        .filter(|o| {
            o.operation_id().is_some_and(|id| {
                [
                    "createCoinbaseCharge",
                    "downloadContainerFileContent",
                    "downloadFileContent",
                ]
                .contains(&id)
            })
        })
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(operations.len(), 3);
    let package = PackageConfig {
        package: "example.openrouterprotocol".into(),
        version: "1.0.0".into(),
        api_name: "Client".into(),
    };
    let maven = MavenConfig {
        artifact_id: "java-openrouter-protocol".into(),
        ..Default::default()
    };
    let refusals = java_sdk::plan_sdk_with_protocol(
        contract.clone(),
        &operations,
        package.clone(),
        &[],
        maven.clone(),
        ProtocolConfig::default(),
    )
    .unwrap_err();
    assert!(refusals.iter().all(|d| d.at.end > d.at.start));
    std::fs::write(
        root.join("ordinary-mode-refusals.log"),
        format!("{refusals:#?}"),
    )
    .unwrap();
    let protocol = ProtocolConfig {
        compatibility_profiles: [wire::CompatibilityProfile::LegacyBinaryStringV1]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    let plan =
        java_sdk::plan_sdk_with_protocol(contract, &operations, package, &[], maven, protocol)
            .unwrap();
    assert!(
        plan.native_examples()
            .iter()
            .all(|e| e.input_expression.is_some())
    );
    for file in plan.render().unwrap() {
        let path = root.join(file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, file.content).unwrap();
    }
    let maven = std::env::var_os("SUSPECT_MAVEN_BIN").unwrap_or_else(|| {
        "/Users/luke/.local/share/mise/installs/maven/3.9.16/apache-maven-3.9.16/bin/mvn".into()
    });
    checked(
        Command::new(maven)
            .args(["-B", "-q", "install"])
            .arg(format!(
                "-Dmaven.repo.local={}",
                crate_root()
                    .join("../../target/sdk-java-maven-cache/java/repository")
                    .display()
            ))
            .env("JAVA_HOME", java())
            .current_dir(root.join("java")),
        &root,
    );
    let jar = root.join("java/target/java-openrouter-protocol-1.0.0.jar");
    checked(
        Command::new(java().join("bin/java"))
            .args(["-ea", "-cp"])
            .arg(&jar)
            .arg("example.openrouterprotocol.SdkExamples"),
        &root,
    );
    std::fs::write(
        root.join("NativeProtocolOpenRouter.java"),
        include_str!("../src/java_sdk/NativeProtocolOpenRouter.java"),
    )
    .unwrap();
    checked(
        Command::new(java().join("bin/javac"))
            .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
            .arg(&jar)
            .arg(root.join("NativeProtocolOpenRouter.java")),
        &root,
    );
    checked(
        Command::new(java().join("bin/java"))
            .args(["-ea", "-cp"])
            .arg(format!("{}:{}", jar.display(), root.display()))
            .arg("NativeProtocolOpenRouter"),
        &root,
    );
    println!("JAVA_PROTOCOL_OPENROUTER {}", root.display());
}

#[test]
#[ignore = "new serving-document server capability; JDK21/25 native consumer"]
fn native_protocol_document_servers() {
    use suspect_ref::{DocumentProvider, ProvidedDocument};
    let response = json!({"200":{"description":"OK","content":{"application/json":{"schema":{"type":"boolean"}}}}});
    let entry = json!({"openapi":"3.2.0","$self":"https://identity.example.test/not-a-server.json","info":{"title":"Physical serving URLs","version":"1"},"paths":{"/inherited":{"$ref":"https://parts.example.test/paths/items.json#/Inherited"},"/explicit":{"$ref":"https://parts.example.test/paths/items.json#/Explicit"},"/relative":{"$ref":"https://parts.example.test/paths/items.json#/Relative"}}});
    let external = json!({"Inherited":{"get":{"operationId":"inherited","responses":response}},"Explicit":{"servers":[],"get":{"operationId":"explicitDefault","responses":response}},"Relative":{"servers":[{"url":"../wire"}],"get":{"operationId":"relative","responses":response}}});
    let root = root();
    std::fs::write(root.join("entry.json"), entry.to_string()).unwrap();
    std::fs::write(root.join("external.json"), external.to_string()).unwrap();
    let uri = Uri::parse("https://entry.example.test/specs/root.json").unwrap();
    let external_uri = Uri::parse("https://parts.example.test/paths/items.json").unwrap();
    let provider = DocumentProvider::new([
        ProvidedDocument::new(uri.clone(), uri.clone(), entry.to_string().into_bytes()).unwrap(),
        ProvidedDocument::new(
            external_uri.clone(),
            external_uri,
            external.to_string().into_bytes(),
        )
        .unwrap(),
    ])
    .unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .document_provider(Arc::new(provider))
            .build()
            .unwrap(),
    );
    let contract = Arc::new(Contract::from_workspace(&workspace, &uri).unwrap());
    let plan = java_sdk::plan_sdk_with_maven(
        contract.clone(),
        &selected(&contract),
        PackageConfig {
            package: "example.documentservers".into(),
            version: "1.0.0".into(),
            api_name: "Client".into(),
        },
        &[],
        MavenConfig {
            artifact_id: "java-document-servers".into(),
            ..Default::default()
        },
    )
    .unwrap();
    for file in plan.render().unwrap() {
        let path = root.join(file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, file.content).unwrap();
    }
    let maven = std::env::var_os("SUSPECT_MAVEN_BIN").unwrap_or_else(|| {
        "/Users/luke/.local/share/mise/installs/maven/3.9.16/apache-maven-3.9.16/bin/mvn".into()
    });
    checked(
        Command::new(maven)
            .args(["-B", "-q", "install"])
            .arg(format!(
                "-Dmaven.repo.local={}",
                crate_root()
                    .join("../../target/sdk-java-maven-cache/java/repository")
                    .display()
            ))
            .env("JAVA_HOME", java())
            .current_dir(root.join("java")),
        &root,
    );
    let jar = root.join("java/target/java-document-servers-1.0.0.jar");
    std::fs::write(
        root.join("NativeSupport.java"),
        include_str!("../src/java_sdk/NativeSupport.java"),
    )
    .unwrap();
    std::fs::write(
        root.join("NativeDocumentServers.java"),
        include_str!("../src/java_sdk/NativeDocumentServers.java"),
    )
    .unwrap();
    checked(
        Command::new(java().join("bin/javac"))
            .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
            .arg(&jar)
            .args(["NativeSupport.java", "NativeDocumentServers.java"])
            .current_dir(&root),
        &root,
    );
    checked(
        Command::new(java().join("bin/java"))
            .args(["-ea", "-cp"])
            .arg(format!("{}:{}", jar.display(), root.display()))
            .arg("NativeDocumentServers")
            .current_dir(&root),
        &root,
    );
    println!("JAVA_DOCUMENT_SERVERS {}", root.display());
}
