//! Independent protocol fixtures through the public Swift SDK entry point.
//! Expected wire values are literal, not produced by the shared interpreter.
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use suspect_codegen::{
    http_protocol::{Capability, CompatibilityProfile, Representation},
    swift_sdk::{SdkPlan, SwiftConfig, plan_sdk},
};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

#[path = "../src/swift_sdk/protocol_next_fixture.rs"]
mod remaining_fixture;
#[path = "../src/swift_sdk/protocol_next_harness.rs"]
mod remaining_harness;

fn remaining_config() -> SwiftConfig {
    SwiftConfig {
        max_request_bytes: 16_384,
        max_response_bytes: 16_384,
        max_part_bytes: 128,
        max_stream_item_bytes: 256,
        max_stream_buffer_bytes: 1024,
        max_stream_capture_bytes: 64,
        ..Default::default()
    }
}

#[test]
fn remaining_standard_capabilities_preserve_typed_slots_and_barriers() {
    let contract = fixture(&remaining_fixture::document());
    let plan = plan_sdk(contract.clone(), &selected(&contract), remaining_config()).unwrap();
    for cap in [
        Capability::CustomMethods,
        Capability::QuerystringParameters,
        Capability::QuerystringForm,
        Capability::PositionalMultipart,
    ] {
        assert!(plan.protocol().capabilities().supports(cap));
    }
    let operation = |name: &str| {
        plan.operations()
            .iter()
            .find(|o| o.operation_id == name)
            .unwrap()
    };
    assert_eq!(operation("lowerHead").protocol().method().as_str(), "head");
    assert!(!operation("lowerHead").responses()[0].always_empty);
    assert!(operation("resetContent").responses()[0].always_empty);
    let whole = &operation("wholeJSON").parameters[0];
    assert_eq!(whole.wire().name(), "criteria");
    assert!(whole.wire().required());
    assert!(
        whole
            .wire()
            .source()
            .use_site()
            .source()
            .pointer()
            .ends_with("/query/parameters/1")
    );
    assert!(matches!(
        whole.wire().content_media().unwrap().representation(),
        Representation::Json { .. }
    ));
    let form = operation("wholeForm")
        .parameters
        .iter()
        .find(|p| {
            p.wire().location() == suspect_codegen::http_protocol::ParameterLocation::Querystring
        })
        .unwrap();
    assert!(form.query_form.is_some());
    assert!(
        plan.protocol()
            .codec_roots()
            .contains(form.wire().codec().schema().id())
    );
    let mixed = operation("sendOrdered").body().unwrap().media[0]
        .positional
        .as_ref()
        .unwrap();
    assert_eq!(mixed.prefix.len(), 3);
    assert!(!plan.protocol().codec_roots().contains(mixed.schema.id()));
    assert!(
        !plan
            .protocol()
            .codec_roots()
            .contains(mixed.prefix[1].wire.schema().id())
    );
    assert_eq!(
        mixed.prefix[0].headers[0]
            .wire
            .source()
            .terminal()
            .source()
            .pointer(),
        "/components/headers/Slot"
    );
    let barrier = operation("barrier").body().unwrap().media[0]
        .positional
        .as_ref()
        .unwrap();
    assert_eq!(barrier.prefix.len(), 1);
    assert!(barrier.items.is_none());
    let extended = operation("extendedPrefix").body().unwrap().media[0]
        .positional
        .as_ref()
        .unwrap();
    assert_eq!(extended.prefix.len(), 3);
    assert!(
        extended.prefix[1]
            .wire
            .schema()
            .id()
            .pointer()
            .ends_with("/items")
    );
    let files = plan.render();
    let quickstart = &files
        .iter()
        .find(|f| f.path.ends_with("GettingStarted.md"))
        .unwrap()
        .content;
    assert!(quickstart.contains("SendOrderedMultipartBody("));
    assert!(quickstart.contains("WholeJSONInput(criteria: Query("));
}

#[test]
fn remaining_native_snapshot_has_positional_and_query_descriptors() {
    use suspect_codegen::{
        backend::{Backend, TargetConfig},
        compatibility,
    };
    let contract = fixture(&remaining_fixture::document());
    let names = contract
        .operations()
        .map(|o| o.operation_id().unwrap().to_owned())
        .collect::<Vec<_>>();
    let snapshot = compatibility::snapshot(
        contract,
        &names,
        &[TargetConfig {
            backend: Backend::SwiftHttp,
            package_name: "GeneratedSDK".into(),
            package_version: "0.1.0".into(),
            import_name: None,
        }],
    )
    .unwrap();
    let native = &snapshot.native[0];
    assert_eq!(native.status, compatibility::PlanStatus::Planned);
    let op = |name: &str| {
        &native
            .operations
            .iter()
            .find(|o| o.operation_id == name)
            .unwrap()
            .descriptor
    };
    assert_eq!(op("mixedGet")["httpMethod"], "GeT");
    assert_eq!(
        op("wholeForm")["parameters"][1]["queryForm"]["fields"][0]["name"],
        "bar"
    );
    let positional = &op("sendOrdered")["body"]["media"][0]["positional"];
    assert_eq!(positional["prefix"][1]["valueType"], "Data");
    assert_eq!(positional["constructorParameters"][3]["name"], "items");
    assert_eq!(
        op("barrier")["body"]["media"][0]["positional"]["items"],
        Value::Null
    );
}

#[test]
fn remaining_standard_undefined_inputs_decline_before_emission() {
    for (case, expected) in [
        (0, "http-connect-tunnel-unsupported"),
        (1, "http-querystring-query-conflict"),
        (2, "http-multipart-streaming"),
    ] {
        let mut value = remaining_fixture::document();
        match case {
            0 => {
                let op = value["paths"]["/case"]["additionalOperations"]["COPY"].take();
                value["paths"] = json!({"/x":{"additionalOperations":{"CONNECT":op}}});
            }
            1 => {
                value["paths"]["/query/text"]["get"]["parameters"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"name":"extra","in":"query","schema":{"type":"string"}}));
            }
            _ => {
                value["paths"]["/ordered"]["post"]["requestBody"]["content"]["multipart/mixed"]["itemSchema"] =
                    json!({"type":"string"});
            }
        }
        let contract = fixture(&value);
        let errors =
            plan_sdk(contract.clone(), &selected(&contract), remaining_config()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.code == expected && e.at.end > e.at.start),
            "{expected}: {errors:#?}"
        );
    }
}

#[test]
#[ignore = "requires Swift, DocC and real socket/TLS fixtures for additional standard capabilities"]
fn native_remaining_standard_custom_query_positional() {
    let contract = fixture(&remaining_fixture::document());
    let plan = plan_sdk(contract.clone(), &selected(&contract), remaining_config()).unwrap();
    remaining_harness::run(
        |root| suspect_codegen::write_files(&plan.render(), root).unwrap(),
        include_str!("../src/swift_sdk/protocol_next_native.swift"),
        "standard-",
    );
}

fn load(path: &Path) -> Arc<Contract> {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap())
}
fn fixture(value: &Value) -> Arc<Contract> {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("protocol.json");
    std::fs::write(&path, value.to_string()).unwrap();
    load(&path)
}
fn selected(c: &Contract) -> Vec<SourceId> {
    c.operations().map(|o| o.source().clone()).collect()
}
fn success() -> Value {
    json!({"200":{"description":"OK","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Reply"}}}}})
}
fn operation(name: &str) -> Value {
    json!({"operationId":name,"security":[],"responses":success()})
}
fn document() -> Value {
    let mut paths = serde_json::Map::new();
    paths.insert("/public".into(), json!({"get":operation("publicValue")}));
    let mut optional = operation("optionalAuth");
    optional["security"] = json!([{"bearer":[]},{}]);
    paths.insert("/optional".into(), json!({"get":optional}));
    let mut and = operation("andAuth");
    and["security"] = json!([{"headerKey":[],"queryKey":[],"cookieKey":[]}]);
    paths.insert("/and".into(), json!({"get":and}));
    let mut or = operation("orAuth");
    or["security"] = json!([{"basic":[]},{"bearer":["admin"]}]);
    paths.insert("/or".into(), json!({"get":or}));
    for (path, name, scheme) in [
        ("/oauth", "oauthValue", "oauth"),
        ("/oidc", "oidcValue", "oidc"),
    ] {
        let mut op = operation(name);
        op["security"] = json!([{scheme:["read"]}]);
        paths.insert(path.into(), json!({"get":op}));
    }
    let mut servers = operation("serverChoice");
    servers["servers"] = json!([
        {"url":"../{version}","name":"relative","variables":{"version":{"default":"v1","enum":["v1","v2"]}}},
        {"url":"https://{region}.example.test/{version}","name":"regional","variables":{"region":{"default":"eu","enum":["eu","us"]},"version":{"default":"v1"}}},
        {"url":"http://plain.example.test/v1","name":"plain"}
    ]);
    paths.insert("/server".into(), json!({"get":servers}));
    let mut default_server = operation("defaultServer");
    default_server["servers"] = json!([]);
    paths.insert("/default-server".into(), json!({"get":default_server}));
    let mut methods = serde_json::Map::new();
    for method in [
        "get", "put", "post", "delete", "options", "head", "patch", "trace", "query",
    ] {
        let mut op = operation(&format!("{method}Method"));
        if method == "head" {
            op["responses"] = json!({"200":{"description":"metadata","headers":{"X-Count":{"required":true,"schema":{"type":"integer"}}},"content":{"application/json":{"schema":false}}}});
        }
        methods.insert(method.into(), op);
    }
    paths.insert("/method".into(), Value::Object(methods));
    let mut styles = operation("styles");
    styles["parameters"] = json!([
        {"in":"path","name":"label","required":true,"style":"label","explode":true,"schema":{"type":"array","items":{"type":"string"}}},
        {"in":"path","name":"matrix","required":true,"style":"matrix","explode":true,"schema":{"$ref":"#/components/schemas/Color"}},
        {"in":"path","name":"simple","required":true,"schema":{"type":"string"}},
        {"in":"query","name":"color","style":"form","explode":false,"schema":{"$ref":"#/components/schemas/Color"}},
        {"in":"query","name":"multi","schema":{"type":"array","items":{"type":"string"}}},
        {"in":"query","name":"spaces","style":"spaceDelimited","schema":{"type":"array","items":{"type":"string"}}},
        {"in":"query","name":"pipes","style":"pipeDelimited","schema":{"type":"array","items":{"type":"string"}}},
        {"in":"query","name":"filter","style":"deepObject","explode":false,"schema":{"$ref":"#/components/schemas/Color"}},
        {"in":"query","name":"reserved","allowReserved":true,"schema":{"type":"string"}},
        {"in":"query","name":"content","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Payload"}}}},
        {"in":"header","name":"X-Flag","schema":{"type":"boolean"}},
        {"in":"header","name":"X-Tags","schema":{"type":"array","items":{"type":"string"}}},
        {"in":"cookie","name":"sid","schema":{"type":"string"}},
        {"in":"cookie","name":"crumb","style":"cookie","explode":true,"schema":{"type":"array","items":{"type":"string"}}}
    ]);
    paths.insert(
        "/styles/{label}/{matrix}/{simple}".into(),
        json!({"get":styles}),
    );
    let mut scalar_styles = operation("scalarStyles");
    scalar_styles["parameters"] = json!([
        {"in":"path","name":"label","required":true,"style":"label","schema":{"type":"integer"}},
        {"in":"path","name":"matrix","required":true,"style":"matrix","schema":{"type":"string"}},
        {"in":"path","name":"simple","required":true,"style":"simple","explode":true,"schema":{"$ref":"#/components/schemas/Color"}},
        {"in":"query","name":"flat","explode":true,"schema":{"type":"object","properties":{"fixed":{"type":"string"}},"additionalProperties":true}}
    ]);
    paths.insert(
        "/scalar/{label}/{matrix}/{simple}".into(),
        json!({"get":scalar_styles}),
    );
    let mut choose = operation("chooseResponse");
    choose["responses"] = json!({
        "200":{"description":"exact","content":{"Application/JSON":{"schema":{"$ref":"#/components/schemas/Reply"}},"text/plain; profile=Exact":{"schema":{"type":"integer"}},"application/octet-stream":{},"image/*":{},"*/*":{}}},
        "2XX":{"description":"range","content":{"text/plain":{"schema":{"type":"string"}}}},
        "default":{"description":"fallback","content":{"application/problem+json":{"schema":{"$ref":"#/components/schemas/Payload"}}}}
    });
    paths.insert("/choose".into(), json!({"get":choose}));
    let mut fallback = operation("defaultResponse");
    fallback["responses"] = json!({"default":{"description":"any actual status","content":{"text/plain":{"schema":{"type":"string"}}}}});
    paths.insert("/fallback".into(), json!({"get":fallback}));
    let mut unspecified = operation("unspecified");
    unspecified["responses"] = json!({"200":{"description":"unspecified body"},"418":{"description":"unspecified failure"},"204":{"description":"no content","content":{"application/json":{"schema":false}}}});
    paths.insert("/unspecified".into(), json!({"get":unspecified}));
    let mut metadata = operation("readMetadata");
    metadata["responses"]["200"]["headers"] = json!({
        "X-Count":{"$ref":"#/components/headers/Quota"},
        "X-Flags":{"schema":{"type":"array","items":{"type":"boolean"}}},
        "X-Color":{"explode":true,"schema":{"$ref":"#/components/schemas/Color"}},
        "X-JSON":{"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Payload"}}}}
    });
    metadata["responses"]["200"]["links"] = json!({"next":{"$ref":"#/components/links/Next"}});
    paths.insert("/metadata".into(), json!({"get":metadata}));
    let mut body = operation("sendBody");
    body["requestBody"] = json!({"required":true,"content":{
        "application/json":{"schema":{"$ref":"#/components/schemas/Payload"}},"text/plain":{"schema":{"type":"integer"}},"*/*":{}
    }});
    paths.insert("/body".into(), json!({"post":body}));
    let form = json!({"schema":{"type":"object","properties":{"title":{"type":"string","minLength":1},"values":{"type":"array","minItems":1,"maxItems":3,"items":{"type":"integer","minimum":0}},"payload":{"$ref":"#/components/schemas/Payload"}},"required":["title"],"additionalProperties":false,"minProperties":1,"maxProperties":3}});
    let mut submit = operation("submitForm");
    submit["requestBody"] =
        json!({"required":true,"content":{"application/x-www-form-urlencoded":form}});
    paths.insert("/form".into(), json!({"post":submit}));
    let mut read_form = operation("readForm");
    read_form["responses"] =
        json!({"200":{"description":"form","content":{"application/x-www-form-urlencoded":form}}});
    paths.insert("/form-response".into(), json!({"get":read_form}));
    let multipart = json!({"schema":{"type":"object","properties":{
        "file":{"maxLength":32},"files":{"type":"array","minItems":1,"maxItems":2,"items":{"maxLength":16}},"title":{"type":"string","minLength":1},"payload":{"$ref":"#/components/schemas/Payload"},"tag":{"type":"array","minItems":1,"maxItems":2,"items":{"type":"integer","minimum":1}}},
        "required":["file","title"],"additionalProperties":false,"maxProperties":5},
        "encoding":{"file":{"contentType":"application/octet-stream","headers":{"X-Size":{"required":true,"schema":{"type":"integer","minimum":0}}}},"files":{"contentType":"application/octet-stream"},"payload":{"contentType":"application/json"},"tag":{"style":"form","explode":false}}
    });
    let mut upload = operation("upload");
    upload["requestBody"] = json!({"required":true,"content":{"multipart/form-data":multipart}});
    paths.insert("/upload".into(), json!({"post":upload}));
    let mut download = operation("downloadParts");
    download["responses"] =
        json!({"200":{"description":"parts","content":{"multipart/form-data":multipart}}});
    paths.insert("/parts-response".into(), json!({"get":download}));
    let mut extras = operation("extraParts");
    extras["requestBody"] = json!({"required":true,"content":{"multipart/form-data":{"schema":{"type":"object","properties":{"title":{"type":"string"}},"required":["title"],"additionalProperties":{"type":"integer"},"minProperties":2,"maxProperties":3}}}});
    paths.insert("/extra-parts".into(), json!({"post":extras}));
    for (path, name, media, schema) in [
        ("/events", "events", "text/event-stream", "Event"),
        ("/lines", "lines", "application/x-ndjson", "Line"),
        ("/jsonl", "jsonl", "application/jsonl", "Line"),
        (
            "/socket-events",
            "socketEvents",
            "text/event-stream",
            "Event",
        ),
    ] {
        let mut stream = operation(name);
        stream["responses"] = json!({"200":{"description":"standard items","content":{media:{"itemSchema":{"$ref":format!("#/components/schemas/{schema}")}}}},"400":{"description":"failure","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Payload"}}}}});
        paths.insert(path.into(), json!({"get":stream}));
    }
    let mut mixed = operation("mixedStream");
    mixed["responses"] = json!({"200":{"description":"typed media choice","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Reply"}},"text/event-stream":{"itemSchema":{"$ref":"#/components/schemas/Event"}}}}});
    paths.insert("/mixed".into(), json!({"get":mixed}));
    let mut unnamed = operation("unused");
    unnamed.as_object_mut().unwrap().remove("operationId");
    paths.insert("/unnamed".into(), json!({"get":unnamed}));
    let mut free_json = operation("freeJSON");
    free_json["requestBody"] = json!({"required":true,"content":{"application/json":{}}});
    free_json["responses"] = json!({"200":{"description":"explicit unconstrained JSON","content":{"application/json":{}}}});
    paths.insert("/json-free".into(), json!({"post":free_json}));
    let mut free_text = operation("freeText");
    free_text["requestBody"] = json!({"required":true,"content":{"text/plain":{}}});
    free_text["responses"] =
        json!({"200":{"description":"UTF-8 text","content":{"text/plain":{}}}});
    paths.insert("/text-free".into(), json!({"post":free_text}));
    let mut parameter_media = operation("parameterMedia");
    parameter_media["responses"] = json!({"200":{"description":"parameter precedence","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Reply"}},"application/json; profile=Exact":{"schema":{"$ref":"#/components/schemas/Payload"}}}}});
    paths.insert("/parameter-media".into(), json!({"get":parameter_media}));
    let mut unicode = operation("unicodeParams");
    unicode["parameters"] = json!([{"name":"map","in":"query","required":true,"style":"deepObject","schema":{"type":"object","properties":{"é":{"type":"integer"},"e\u{301}":{"type":"boolean"}},"required":["é","e\u{301}"],"additionalProperties":false}}]);
    paths.insert("/unicode".into(), json!({"get":unicode}));
    json!({"openapi":"3.2.0","info":{"title":"Independent Swift protocol witnesses","version":"1"},"servers":[{"url":"https://example.test/api"}],"security":[{"bearer":[]}],"paths":paths,
    "components":{"schemas":{
        "Reply":{"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false},
        "Payload":{"type":"object","properties":{"name":{"type":"string","minLength":1},"amount":{"type":"number"}},"required":["name"],"additionalProperties":false},
        "Color":{"type":"object","properties":{"B":{"type":"integer"},"G":{"type":"integer"},"R":{"type":"integer"}},"required":["B","G","R"],"additionalProperties":false},
        "Event":{"type":"object","properties":{"data":{"type":"string","minLength":1},"id":{"type":"string"},"event":{"type":"string"},"retry":{"type":"integer","minimum":0}},"required":["data"],"additionalProperties":false},
        "Line":{"type":"object","properties":{"n":{"type":"integer"}},"required":["n"],"additionalProperties":false}
    },"headers":{"Quota":{"required":true,"schema":{"type":"integer","minimum":0}}},
    "links":{"Next":{"operationId":"publicValue","parameters":{"value":"$response.header.X-Count","literal":{"$ref":"instance","schema":false}},"requestBody":{"literal":true},"description":"Metadata only","server":{"url":"../v2"}}},
    "securitySchemes":{
        "bearer":{"type":"http","scheme":"BeArEr","bearerFormat":"opaque"},"basic":{"type":"http","scheme":"BaSiC"},
        "headerKey":{"type":"apiKey","in":"header","name":"X-Key"},"queryKey":{"type":"apiKey","in":"query","name":"key"},"cookieKey":{"type":"apiKey","in":"cookie","name":"session"},
        "oauth":{"type":"oauth2","oauth2MetadataUrl":"https://auth.example.test/metadata","flows":{"authorizationCode":{"authorizationUrl":"https://auth.example.test/authorize","tokenUrl":"https://auth.example.test/token","refreshUrl":"https://auth.example.test/refresh","scopes":{"read":"Read values"}}}},
        "oidc":{"type":"openIdConnect","openIdConnectUrl":"https://auth.example.test/.well-known/openid-configuration"}
    }}})
}
fn config() -> SwiftConfig {
    SwiftConfig {
        max_request_bytes: 16_384,
        max_response_bytes: 4096,
        max_stream_capture_bytes: 64,
        max_part_bytes: 1024,
        max_stream_item_bytes: 256,
        max_stream_buffer_bytes: 1024,
        ..Default::default()
    }
}

#[test]
fn typed_protocol_roots_capabilities_and_dxcalls() {
    let c = fixture(&document());
    let plan = plan_sdk(c.clone(), &selected(&c), config()).unwrap();
    assert!(plan.protocol().is_admitted());
    assert_eq!(
        plan.protocol().capabilities().adapter(),
        "swift-http-protocol-v1"
    );
    for capability in [
        Capability::AdditionalMethods,
        Capability::AnonymousSecurity,
        Capability::FormBodies,
        Capability::MultipartBodies,
        Capability::ServerSentEvents,
        Capability::JsonLines,
        Capability::ResponseHeaders,
        Capability::ResponseLinks,
    ] {
        assert!(plan.protocol().capabilities().supports(capability));
    }
    assert!(
        plan.protocol()
            .capabilities()
            .supports(Capability::PositionalMultipart)
    );
    assert!(plan.protocol().capabilities().profiles().is_empty());
    let upload = plan
        .operations()
        .iter()
        .find(|o| o.operation_id == "upload")
        .unwrap();
    let body = upload.body().unwrap();
    let parts = body.media[0].parts.as_ref().unwrap();
    let file = parts
        .fields
        .iter()
        .find(|p| p.wire.name() == Some("file"))
        .unwrap();
    assert_eq!(file.value_type, "Data");
    assert!(
        !plan
            .protocol()
            .codec_roots()
            .contains(file.wire.schema().id())
    );
    assert!(
        !plan
            .protocol()
            .codec_roots()
            .contains(parts.rules.schema().id())
    );
    let events = plan
        .operations()
        .iter()
        .find(|o| o.operation_id == "events")
        .unwrap();
    let Representation::Stream { stream } = events.responses()[0].wire.media()[0].representation()
    else {
        panic!()
    };
    assert!(
        plan.protocol()
            .codec_roots()
            .contains(stream.item_codec().schema().id())
    );
    assert!(
        plan.operations()
            .iter()
            .find(|o| o.operation_id == "publicValue")
            .unwrap()
            .default_input()
    );
    let files = plan.render();
    let quickstart = &files
        .iter()
        .find(|f| f.path.ends_with("GettingStarted.md"))
        .unwrap()
        .content;
    assert!(quickstart.contains("Payload(name:"));
    assert!(!quickstart.contains("Codecs."));
    assert!(quickstart.contains("client.publicValue()"));
    assert_eq!(files, plan.render());
}

#[test]
fn located_declines_and_versioned_profiles_are_atomic() {
    for (edit, code) in [
        (0, "http-stream-item-schema-required"),
        (1, "http-parameter-combination-undefined"),
        (2, "http-security-attachment-conflict"),
        (3, "swift-set-cookie-header-unsupported"),
        (4, "http-binary-legacy-marker"),
    ] {
        let mut v = document();
        match edit {
            0 => {
                v["openapi"] = json!("3.1.0");
                v["paths"] = json!({"/s":{"get":{"operationId":"oldStream","security":[],"responses":{"200":{"description":"vendor","content":{"text/event-stream":{"schema":{"type":"object"}}}}}}}});
            }
            1 => {
                v["paths"] = json!({"/bad":{"get":{"operationId":"bad","security":[],"parameters":[{"name":"q","in":"query","style":"deepObject","schema":{"type":"array","items":{"type":"string"}}}],"responses":success()}}});
            }
            2 => {
                v["paths"] = json!({"/bad":{"get":{"operationId":"bad","security":[{"basic":[],"bearer":[]}],"responses":success()}}});
            }
            3 => {
                v["paths"] = json!({"/bad":{"get":{"operationId":"bad","security":[],"responses":{"200":{"description":"opaque repeat field","headers":{"Set-Cookie":{"schema":{"type":"string"}}}}}}}});
            }
            _ => {
                v["paths"] = json!({"/bad":{"get":{"operationId":"bad","security":[],"responses":{"200":{"description":"legacy","content":{"application/octet-stream":{"schema":{"type":"string","format":"binary"}}}}}}}});
            }
        }
        let c = fixture(&v);
        let errors = plan_sdk(c.clone(), &selected(&c), config()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|d| d.code == code && d.at.end > d.at.start),
            "{code}: {errors:#?}"
        );
        if edit == 4 {
            let mut cfg = config();
            cfg.compatibility_profiles
                .insert(CompatibilityProfile::LegacyBinaryStringV1);
            assert!(plan_sdk(c.clone(), &selected(&c), cfg).is_ok());
        }
    }
}

fn root(name: &str) -> PathBuf {
    if let Some(path) = std::env::var_os("SUSPECT_SWIFT_PROTOCOL_CASE_DIR") {
        let path = PathBuf::from(path);
        std::fs::create_dir_all(&path).unwrap();
        return path.canonicalize().unwrap();
    }
    let base = std::env::var_os("SUSPECT_SWIFT_PROTOCOL_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("opencode/swift-protocol-gates"));
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix(name)
        .tempdir_in(base)
        .unwrap()
        .keep()
        .canonicalize()
        .unwrap()
}
fn checked(command: &mut Command, root: &Path) -> Output {
    let output = command.output().expect("required native tool unavailable");
    assert!(
        output.status.success(),
        "retained at {}\n{command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
fn swift() -> PathBuf {
    std::env::var_os("SUSPECT_SWIFT_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/usr/bin/swift".into())
}
fn swiftc() -> PathBuf {
    std::env::var_os("SUSPECT_SWIFTC_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if swift() == Path::new("/usr/bin/swift") {
                let found = Command::new("xcrun")
                    .args(["--find", "swiftc"])
                    .output()
                    .expect("xcrun required for the default Apple toolchain");
                assert!(found.status.success());
                PathBuf::from(String::from_utf8(found.stdout).unwrap().trim())
            } else {
                swift().with_file_name("swiftc")
            }
        })
}
fn swift_command(action: &str) -> Command {
    let mut c = Command::new(swift());
    c.arg(action).env("SWIFT_EXEC", swiftc());
    if action == "test" {
        c.arg("--disable-swift-testing");
    }
    if let Some(sdk) = std::env::var_os("SUSPECT_SWIFT_SDKROOT") {
        c.arg("--sdk").arg(&sdk).env("SDKROOT", sdk);
    }
    c
}
fn compiler() -> Command {
    let mut c = Command::new(swiftc());
    if let Some(sdk) = std::env::var_os("SUSPECT_SWIFT_SDKROOT") {
        c.arg("-sdk").arg(&sdk).env("SDKROOT", sdk);
    } else {
        let found = Command::new("xcrun")
            .args(["--sdk", "macosx", "--show-sdk-path"])
            .output()
            .expect("an Apple SDK is required");
        assert!(found.status.success());
        let sdk = String::from_utf8(found.stdout).unwrap();
        c.arg("-sdk").arg(sdk.trim()).env("SDKROOT", sdk.trim());
    }
    c
}
fn directory(root: &Path, name: &str) -> Option<PathBuf> {
    for e in std::fs::read_dir(root).ok()? {
        let p = e.ok()?.path();
        if p.is_dir() {
            if p.file_name()?.to_str()? == name {
                return Some(p);
            }
            if let Some(found) = directory(&p, name) {
                return Some(found);
            }
        }
    }
    None
}
fn native_package(plan: &SdkPlan, root: &Path, consumer: &str) {
    suspect_codegen::write_files(&plan.render(), &root.join("sdk")).unwrap();
    checked(
        swift_command("test")
            .arg("--package-path")
            .arg(root.join("sdk"))
            .arg("--scratch-path")
            .arg(root.join("build/sdk"))
            .args(["-Xswiftc", "-warnings-as-errors"]),
        root,
    );
    let path = root.join("consumer");
    std::fs::create_dir_all(path.join("Tests/ProtocolConsumer")).unwrap();
    std::fs::write(path.join("Package.swift"),"// swift-tools-version: 6.0\nimport PackageDescription\nlet package = Package(name: \"ProtocolConsumer\", platforms: [.macOS(.v13)], dependencies: [.package(path: \"../sdk\")], targets: [.testTarget(name: \"ProtocolConsumer\", dependencies: [.product(name: \"GeneratedSDK\", package: \"sdk\")], swiftSettings: [.swiftLanguageMode(.v6)])])\n").unwrap();
    std::fs::write(
        path.join("Tests/ProtocolConsumer/ProtocolTests.swift"),
        consumer,
    )
    .unwrap();
}
fn docs(root: &Path) {
    checked(
        swift_command("package")
            .arg("--package-path")
            .arg(root.join("sdk"))
            .arg("--scratch-path")
            .arg(root.join("build/sdk"))
            .args(["dump-symbol-graph", "--minimum-access-level", "public"]),
        root,
    );
    let graph = directory(&root.join("build/sdk"), "symbolgraph").unwrap();
    let mut cmd = if let Some(bin) = std::env::var_os("SUSPECT_SWIFT_DOCC_BIN") {
        Command::new(bin)
    } else {
        let mut c = Command::new("xcrun");
        c.arg("docc");
        c
    };
    checked(
        cmd.arg("convert")
            .arg(root.join("sdk/Sources/GeneratedSDK/GeneratedSDK.docc"))
            .arg("--additional-symbol-graph-dir")
            .arg(graph)
            .arg("--output-path")
            .arg(root.join("GeneratedSDK.doccarchive"))
            .arg("--warnings-as-errors"),
        root,
    );
    assert!(root.join("GeneratedSDK.doccarchive/index.html").is_file());
}
fn typechecks(root: &Path) {
    let modules = directory(&root.join("build/sdk"), "Modules").unwrap();
    let positive = root.join("positive.swift");
    std::fs::write(&positive,"import Foundation\nimport GeneratedSDK\nfunc positive(_ client: Client) async throws { let _: Bool = try await client.publicValue().data.ok; let _: HTTPEventStream<Event> = try await client.events().data; let _ = SendBodyInput(body: .json(Payload(name: \"x\"))); let _ = HTTPPart(Data([0, 255]), filename: \"a.bin\") }\n").unwrap();
    checked(
        compiler()
            .args([
                "-typecheck",
                "-swift-version",
                "6",
                "-warnings-as-errors",
                "-module-cache-path",
            ])
            .arg(root.join("frontend-cache"))
            .arg("-I")
            .arg(&modules)
            .arg(&positive),
        root,
    );
    for(i,code)in[
        "let _ = SendBodyInput()",
        "let _ = SendBodyInput(body: .json(\"untyped\"))",
        "let _ = SendBodyInput(body: .bytes(\"file-path\", contentType: \"application/octet-stream\"))",
        "let _ = Credentials(basic: \"unstructured\")",
        "let _ = Credentials(oauth: \"inferred-token-type\")",
        "func fail(_ client: Client) async throws { let _: String = try await client.readMetadata().typedHeaders.xCount }",
        "func fail(_ client: Client) async throws { let _: [Event] = try await client.events().data }",
        "let _ = UploadMultipartFileHeaders()",
    ].iter().enumerate(){let path=root.join(format!("negative-{i}.swift"));std::fs::write(&path,format!("import Foundation\nimport GeneratedSDK\n{code}\n")).unwrap();
        let output=compiler().args(["-typecheck","-swift-version","6","-module-cache-path"]).arg(root.join("frontend-cache")).arg("-I").arg(&modules).arg(&path).output().unwrap();assert!(!output.status.success(),"negative compiled: {code}");let stderr=String::from_utf8_lossy(&output.stderr);assert!(!stderr.contains("no such module")&&!stderr.contains("unable to load standard library"),"{stderr}");}
}

#[test]
#[ignore = "requires installed Swift 6, DocC and real loopback sockets"]
fn native_protocol_spm_types_wire_stream_lifetimes_and_docs() {
    let root = root("protocol-");
    let path = root.join("protocol.json");
    std::fs::write(&path, document().to_string()).unwrap();
    let c = load(&path);
    let plan = plan_sdk(c.clone(), &selected(&c), config()).unwrap();
    native_package(
        &plan,
        &root,
        include_str!("../src/swift_sdk/protocol_native.swift"),
    );
    let server = WireServer::start(&root);
    let output = checked(
        swift_command("test")
            .arg("--package-path")
            .arg(root.join("consumer"))
            .arg("--scratch-path")
            .arg(root.join("build/consumer"))
            .args(["-Xswiftc", "-warnings-as-errors"])
            .env("SUSPECT_SWIFT_PROTOCOL_BASE", &server.base)
            .env("SUSPECT_SWIFT_PROTOCOL_MARKERS", &root),
        &root,
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
    server.verify();
    typechecks(&root);
    docs(&root);
    println!("Swift protocol native gate passed: {}", root.display());
}

#[test]
#[ignore = "requires the original OpenRouter checkout, Swift 6, DocC and loopback sockets"]
fn native_new_openrouter_operations_and_explicit_legacy_binary_profile() {
    let checkout = std::env::var_os("OPENROUTER_WEB_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| "/Users/luke/github/openrouter-web".into());
    let path = checkout.join("projects/docs/openapi/openapi.yaml");
    assert!(path.is_file(), "the actual source corpus is required");
    let contract = load(&path);
    let wanted = [
        "createCoinbaseCharge",
        "downloadContainerFileContent",
        "downloadFileContent",
    ];
    let selected = contract
        .operations()
        .filter(|o| o.operation_id().is_some_and(|n| wanted.contains(&n)))
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), wanted.len());
    let ordinary = plan_sdk(contract.clone(), &selected, Default::default()).unwrap_err();
    assert!(
        ordinary
            .iter()
            .any(|e| e.code == "http-binary-legacy-marker")
    );
    let mut config = SwiftConfig::default();
    config
        .compatibility_profiles
        .insert(CompatibilityProfile::LegacyBinaryStringV1);
    let plan = plan_sdk(contract.clone(), &selected, config.clone()).unwrap();
    for op in plan
        .operations()
        .iter()
        .filter(|o| o.operation_id.starts_with("download"))
    {
        let response = op
            .responses()
            .iter()
            .find(|r| r.wire.status_key() == "200")
            .unwrap();
        assert_eq!(response.type_name, "Data");
        let Representation::Binary {
            schema: Some(schema),
            ..
        } = response.wire.media()[0].representation()
        else {
            panic!("original byte schema must remain metadata")
        };
        assert!(!plan.protocol().codec_roots().contains(schema.id()));
        assert_eq!(schema.id().document(), contract.entry());
    }
    for (name, code) in [
        ("uploadFile", "http-form-untyped-extras"),
        ("createOauthToken", "http-form-untyped-extras"),
        (
            "sendChatCompletionRequest",
            "http-stream-item-schema-required",
        ),
    ] {
        let source = contract
            .operations()
            .find(|o| o.operation_id() == Some(name))
            .unwrap()
            .source()
            .clone();
        let findings = plan_sdk(contract.clone(), &[source], config.clone()).unwrap_err();
        assert!(
            findings
                .iter()
                .any(|f| f.code == code && f.at.end > f.at.start),
            "{name}: {findings:#?}"
        );
    }
    let root = root("new-openrouter-");
    native_package(
        &plan,
        &root,
        include_str!("../src/swift_sdk/protocol_openrouter.swift"),
    );
    let server = WireServer::start_with(&root, serve_openrouter);
    let output = checked(
        swift_command("test")
            .arg("--package-path")
            .arg(root.join("consumer"))
            .arg("--scratch-path")
            .arg(root.join("build/consumer"))
            .args(["-Xswiftc", "-warnings-as-errors"])
            .env("SUSPECT_SWIFT_NEW_OPENROUTER_BASE", &server.base),
        &root,
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
    for marker in ["new-file", "new-container", "new-anonymous"] {
        assert!(
            root.join(marker).is_file(),
            "missing actual-operation native wire witness {marker}"
        );
    }
    let modules = directory(&root.join("build/sdk"), "Modules").unwrap();
    let positive = root.join("positive.swift");
    std::fs::write(&positive, "import Foundation\nimport GeneratedSDK\nfunc probe(_ client: Client) async throws { let _: Data = try await client.downloadFileContent(DownloadFileContentInput(fileId: \"or_file_1\")).data; let _: Data = try await client.createCoinbaseCharge().data }\n").unwrap();
    checked(
        compiler()
            .args(["-typecheck", "-swift-version", "6", "-I"])
            .arg(&modules)
            .arg(&positive),
        &root,
    );
    let negative = root.join("negative.swift");
    std::fs::write(&negative, "import Foundation\nimport GeneratedSDK\nlet _ = DownloadFileContentInput(fileId: Data())\n").unwrap();
    let failure = compiler()
        .args(["-typecheck", "-swift-version", "6", "-I"])
        .arg(&modules)
        .arg(&negative)
        .output()
        .unwrap();
    assert!(!failure.status.success());
    assert!(String::from_utf8_lossy(&failure.stderr).contains("cannot convert"));
    docs(&root);
    println!(
        "Swift new actual OpenRouter protocol package, wire, types and DocC passed: {}",
        root.display()
    );
}

#[test]
#[ignore = "requires installed Swift 6 and DocC; OAS 3.0 source projection witness"]
fn native_openapi30_binary_and_nullable_http_projection() {
    let document = json!({"openapi":"3.0.3","info":{"title":"OAS 3.0 native HTTP witness","version":"1"},"servers":[{"url":"https://old.example.test/v0"}],
        "paths":{"/legacy/{id}":{"post":{"operationId":"legacy","parameters":[{"name":"id","in":"path","required":true,"schema":{"type":"string"}}],
            "requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/OldPayload"}},"application/octet-stream":{"schema":{"type":"string","format":"binary"}}}},
            "responses":{"200":{"description":"normative legacy bytes","content":{"application/octet-stream":{"schema":{"type":"string","format":"binary"}}}}}}}},
        "components":{"schemas":{"OldPayload":{"type":"object","properties":{"name":{"type":"string","minLength":1},"nullableValue":{"type":"string","nullable":true}},"required":["name","nullableValue"],"additionalProperties":false}}}});
    let root = root("oas30-");
    let path = root.join("oas30.json");
    std::fs::write(&path, document.to_string()).unwrap();
    let contract = load(&path);
    let plan = plan_sdk(contract.clone(), &selected(&contract), Default::default()).unwrap();
    assert!(plan.protocol().capabilities().profiles().is_empty());
    native_package(
        &plan,
        &root,
        include_str!("../src/swift_sdk/protocol_oas30.swift"),
    );
    let output = checked(
        swift_command("test")
            .arg("--package-path")
            .arg(root.join("consumer"))
            .arg("--scratch-path")
            .arg(root.join("build/consumer"))
            .args(["-Xswiftc", "-warnings-as-errors"]),
        &root,
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
    docs(&root);
    println!(
        "Swift OAS 3.0 HTTP projection package and DocC passed: {}",
        root.display()
    );
}

fn serve_openrouter(mut stream: TcpStream, root: &Path) {
    stream.set_nonblocking(false).unwrap();
    let Some((method, path)) = request(&mut stream) else {
        return;
    };
    let marker = match (method.as_str(), path.as_str()) {
        ("GET", "/api/v1/files/or_file_1/content") => "new-file",
        ("GET", "/api/v1/containers/sess%2Fa/files/cfile_1/content") => "new-container",
        ("POST", "/api/v1/credits/coinbase") => "new-anonymous",
        _ => panic!("unexpected actual-operation wire request: {method} {path}"),
    };
    std::fs::write(root.join(marker), &path).unwrap();
    let body = [0, 255, 128, 13, 10, 65];
    let content_type = if marker == "new-anonymous" {
        ""
    } else {
        "Content-Type: application/octet-stream\r\n"
    };
    stream
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\n{content_type}Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .as_bytes(),
        )
        .unwrap();
    stream.write_all(&body).unwrap();
}

struct WireServer {
    base: String,
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
    root: PathBuf,
}
impl WireServer {
    fn start(root: &Path) -> Self {
        Self::start_with(root, serve)
    }
    fn start_with(root: &Path, handler: fn(TcpStream, &Path)) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let dir = root.to_path_buf();
        let join = thread::spawn(move || {
            let mut clients = Vec::new();
            while !flag.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((socket, _)) => {
                        let root = dir.clone();
                        clients.push(thread::spawn(move || handler(socket, &root)));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(e) => panic!("accept: {e}"),
                }
            }
            for client in clients {
                client.join().unwrap();
            }
        });
        Self {
            base,
            stop,
            join: Some(join),
            root: root.to_path_buf(),
        }
    }
    fn verify(&self) {
        for name in [
            "get",
            "put",
            "post",
            "delete",
            "options",
            "head",
            "patch",
            "trace",
            "query",
            "stream-break",
            "stream-cancel",
            "stream-complete",
            "stream-buffer",
            "stream-limit",
            "stream-timeout",
        ] {
            assert!(
                self.root.join(format!("wire-{name}")).is_file(),
                "missing native wire witness {name}"
            );
        }
    }
}
impl Drop for WireServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            let result = join.join();
            if !thread::panicking() {
                result.unwrap();
            }
        }
    }
}
fn request(stream: &mut TcpStream) -> Option<(String, String)> {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .ok()?;
    let mut bytes = Vec::new();
    let mut byte = [0];
    while !bytes.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).ok()? == 0 {
            return None;
        }
        bytes.push(byte[0]);
        if bytes.len() > 65_536 {
            return None;
        }
    }
    let head = String::from_utf8(bytes).ok()?;
    let mut first = head.lines().next()?.split_whitespace();
    Some((first.next()?.into(), first.next()?.into()))
}
fn serve(mut stream: TcpStream, root: &Path) {
    stream.set_nonblocking(false).unwrap();
    let Some((method, path)) = request(&mut stream) else {
        return;
    };
    if path == "/api/method" {
        let name = method.to_ascii_lowercase();
        assert!(
            [
                "get", "put", "post", "delete", "options", "head", "patch", "trace", "query"
            ]
            .contains(&name.as_str()),
            "wrong method {method}"
        );
        std::fs::write(root.join(format!("wire-{name}")), &method).unwrap();
        let body = if method == "HEAD" {
            ""
        } else {
            r#"{"ok":true}"#
        };
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nX-Count: 7\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            if method == "HEAD" {
                1_000_000
            } else {
                body.len()
            }
        );
        stream.write_all(header.as_bytes()).unwrap();
        stream.write_all(body.as_bytes()).unwrap();
        return;
    }
    assert_eq!(method, "GET");
    if path == "/complete/socket-events" {
        let body = "data: one\r\n\r\ndata: two\n\n";
        stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).as_bytes()).unwrap();
        for b in body.bytes() {
            if stream.write_all(&[b]).is_err() {
                return;
            }
            thread::sleep(Duration::from_millis(1));
        }
        std::fs::write(root.join("wire-stream-complete"), "closed").unwrap();
        return;
    }
    let marker = if path == "/break/socket-events" {
        "wire-stream-break"
    } else if path == "/cancel/socket-events" {
        "wire-stream-cancel"
    } else if path == "/buffer/socket-events" {
        "wire-stream-buffer"
    } else if path == "/limit/socket-events" {
        "wire-stream-limit"
    } else if path == "/timeout/socket-events" {
        "wire-stream-timeout"
    } else {
        panic!("unexpected path {path}")
    };
    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").unwrap();
    {
        let value: &[u8] = if marker.ends_with("break") {
            b"data: first\n\n"
        } else if marker.ends_with("buffer") || marker.ends_with("limit") {
            b": 1234567890123456789012345678901234567890123456789012345678901234\n\n"
        } else {
            b": ready\n\n"
        };
        stream
            .write_all(format!("{:X}\r\n", value.len()).as_bytes())
            .unwrap();
        stream.write_all(value).unwrap();
        stream.write_all(b"\r\n").unwrap();
    }
    let start = Instant::now();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut buf = [0; 1];
    match stream.read(&mut buf) {
        Ok(0) => {
            std::fs::write(root.join(marker), start.elapsed().as_millis().to_string()).unwrap();
        }
        Err(e)
            if [
                std::io::ErrorKind::ConnectionReset,
                std::io::ErrorKind::BrokenPipe,
            ]
            .contains(&e.kind()) =>
        {
            std::fs::write(root.join(marker), "reset").unwrap();
        }
        other => panic!("stream transfer not cleaned up: {other:?}"),
    }
}
