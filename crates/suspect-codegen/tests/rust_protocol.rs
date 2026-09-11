//! Public Contract -> expanded native Rust package -> installed Cargo consumer.
//! Literal protocol vectors are independent of the emitter/runtime. Byte, source,
//! typing and ownership witnesses exercise real generated operations.

use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::{
    http_protocol as protocol,
    rust_http::{self, HttpConfig, HttpPlan, PackageConfig, Payload, PlannedOperation},
};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/http-protocol-v1.json")).unwrap()
}
fn contract(files: &[(&str, Value)]) -> Arc<Contract> {
    let directory = tempfile::tempdir().unwrap();
    for (name, value) in files {
        std::fs::write(directory.path().join(name), value.to_string()).unwrap();
    }
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(directory.path())
            .build()
            .unwrap(),
    );
    Arc::new(
        Contract::from_workspace(
            &workspace,
            &Uri::from_path(&directory.path().join("api.json")).unwrap(),
        )
        .unwrap(),
    )
}
fn selected(contract: &Contract) -> Vec<SourceId> {
    contract
        .operations()
        .map(|op| op.source().clone())
        .collect()
}
fn plan(value: Value) -> HttpPlan {
    let c = contract(&[("api.json", value)]);
    rust_http::plan_http(c.clone(), &selected(&c), HttpConfig::default()).unwrap()
}
fn operation<'a>(plan: &'a HttpPlan, id: &str) -> &'a PlannedOperation {
    plan.operations()
        .iter()
        .find(|op| op.operation_id == id)
        .unwrap()
}

#[test]
fn rich_native_plan_and_examples_keep_actual_codec_roots() {
    let fixture = fixture();
    for (key, roots) in [
        ("multipart", "multipartCodecRoots"),
        ("form", "formCodecRoots"),
        ("positionalMultipart", "positionalCodecRoots"),
        ("stream", "streamCodecRoots"),
    ] {
        let plan = plan(fixture[key].clone());
        let pointers = plan
            .protocol()
            .codec_roots()
            .iter()
            .map(|id| id.pointer())
            .collect::<Vec<_>>();
        assert_eq!(
            pointers,
            fixture[roots]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            plan.protocol().capabilities().adapter(),
            "rust-http-protocol-v1"
        );
        assert_eq!(plan.examples().format(), "suspect-sdk-examples-v2");
        for example in plan
            .examples()
            .operations()
            .iter()
            .flat_map(|op| &op.entries)
        {
            assert!(
                plan.symbols().contains_key(&example.schema),
                "{} is not an actual codec root",
                example.schema.pointer()
            );
        }
        assert!(
            plan.examples()
                .diagnostics()
                .iter()
                .all(|d| !d.code.starts_with("http-")),
            "examples must not rerun strict HTTP admission"
        );
    }
}

#[test]
fn source_bound_headers_links_and_schema_refs_survive_the_native_plan() {
    let fixture = fixture();
    let files = &fixture["sourceReferences"];
    let contract = contract(&[
        ("api.json", files["api.json"].clone()),
        ("shared.json", files["shared.json"].clone()),
        ("headers.json", files["headers.json"].clone()),
    ]);
    let plan =
        rust_http::plan_http(contract.clone(), &selected(&contract), Default::default()).unwrap();
    let op = operation(&plan, "getItem");
    assert_eq!(op.source.pointer(), "/components/pathItems/Item/get");
    assert_eq!(
        op.wire().path_item().source().pointer(),
        "/paths/~1items~1{id}"
    );
    let response = &op.responses()[0];
    assert_eq!(response.wire().source().references().len(), 2);
    let header = &response.headers()[0];
    assert_eq!(
        header.wire().source().terminal().source().pointer(),
        "/Rate"
    );
    assert!(
        header
            .wire()
            .source()
            .terminal()
            .source()
            .document()
            .as_str()
            .ends_with("headers.json")
    );
    assert_eq!(
        header.wire().codec().schema().id().pointer(),
        "/Rate/schema"
    );
    assert!(header.wire().required());
    assert_eq!(
        response.wire().links()[0].request_body().unwrap().value()["$ref"],
        "literal-instance-data"
    );
    assert!(
        plan.protocol()
            .codec_roots()
            .iter()
            .all(|id| !id.pointer().contains("/links/"))
    );
    assert_eq!(
        op.interface()["responses"][0]["headers"][0]["model"],
        header.model
    );
}

#[test]
fn legacy_byte_profiles_and_unimplemented_conventions_are_explicit() {
    let mut source = json!({"openapi":"3.1.2","info":{"title":"Byte profile","version":"1"},"paths":{"/file":{"post":{"operationId":"file","requestBody":{"required":true,"content":{"application/octet-stream":{"schema":{"type":"string","format":"binary"}}}},"responses":{"204":{"description":"stored"}},"x-retries":5}}}});
    let c = contract(&[("api.json", source.clone())]);
    let errors = rust_http::plan_http(c.clone(), &selected(&c), Default::default()).unwrap_err();
    assert!(errors.iter().any(|d| d.code == "http-binary-legacy-marker"
        && d.source.pointer().ends_with("/schema/format")
        && !d.at.is_empty()));
    let plan = rust_http::plan_http(
        c.clone(),
        &selected(&c),
        HttpConfig {
            compatibility_profiles: vec![protocol::CompatibilityProfile::LegacyBinaryStringV1],
            ..Default::default()
        },
    )
    .unwrap();
    assert!(plan.protocol().codec_roots().is_empty());
    assert!(
        plan.protocol()
            .diagnostics()
            .iter()
            .any(|d| d.code() == "http-compatibility-profile")
    );
    assert!(
        plan.protocol().diagnostics().iter().any(|d| d
            .source()
            .source()
            .pointer()
            .ends_with("x-retries"))
    );
    source["paths"]["/file"]["post"]["requestBody"]["content"] =
        json!({"text/event-stream":{"schema":{"type":"object"}}});
    let c = contract(&[("api.json", source)]);
    let errors = rust_http::plan_http(c.clone(), &selected(&c), Default::default()).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|d| d.code == "http-stream-item-schema-required")
    );
    let mut positional = fixture()["positionalMultipart"].clone();
    positional["paths"]["/parts"]["post"]["requestBody"]["content"]["multipart/mixed"]["prefixEncoding"]
        [0] = json!({"style":"form"});
    let c = contract(&[("api.json", positional.clone())]);
    let ignored = rust_http::plan_http(c.clone(), &selected(&c), Default::default()).unwrap();
    let op = &ignored.operations()[0];
    let media = op.body().unwrap().media()[0].wire();
    let protocol::Representation::Multipart {
        multipart: protocol::MultipartPlan::Positional { prefix, .. },
    } = media.representation()
    else {
        panic!("source-defined positional multipart")
    };
    let encoding = op
        .source
        .child("requestBody")
        .child("content")
        .child("multipart/mixed")
        .child("prefixEncoding")
        .child("0");
    let schema = media
        .source()
        .terminal()
        .source()
        .child("schema")
        .child("prefixItems")
        .child("0");
    assert!(
        ignored
            .protocol()
            .capabilities()
            .supports(protocol::Capability::PositionalMultipart)
    );
    assert!(
        ignored
            .protocol()
            .capabilities()
            .supports(protocol::Capability::PartEncodings)
    );
    assert_eq!(prefix.len(), 1);
    assert_eq!(prefix[0].content_types()[0].declared(), "text/plain");
    assert_eq!(
        prefix[0].encoding_source().unwrap().terminal().source(),
        &encoding
    );
    assert!(
        matches!(prefix[0].representation(), protocol::PartRepresentation::Text {
        codec, scalar: protocol::ScalarType::String, outer_encoding: protocol::PercentEncoding::None,
    } if codec.schema().id() == &schema)
    );
    assert!(ignored.protocol().diagnostics().iter().any(|d| d.code()
        == "http-encoding-style-ignored"
        && d.severity() == protocol::Severity::Warning
        && d.source().source() == &encoding.child("style")
        && !d.source().span().is_empty()));
    assert_eq!(
        ignored
            .protocol()
            .codec_roots()
            .iter()
            .map(|s| s.pointer())
            .collect::<Vec<_>>(),
        fixture()["positionalCodecRoots"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect::<Vec<_>>()
    );
    // Ignored fields preserve the already-witnessed native content-codec
    // surface, including the whole structural aggregate's separate ownership.
    let baseline = self::plan(fixture()["positionalMultipart"].clone());
    assert_eq!(op.interface(), baseline.operations()[0].interface());
    for (field, value, code, suffix) in [
        (
            "style",
            json!(false),
            "http-metadata-string",
            "/prefixEncoding/0/style",
        ),
        (
            "explode",
            json!("false"),
            "http-metadata-boolean",
            "/prefixEncoding/0/explode",
        ),
        (
            "contentType",
            json!("application/json, text/plain"),
            "http-part-mixed-representations",
            "/prefixEncoding/0",
        ),
        (
            "contentType",
            json!("application/octet-stream"),
            "http-binary-schema-type",
            "/schema/prefixItems/0/type",
        ),
    ] {
        let mut invalid = positional.clone();
        invalid["paths"]["/parts"]["post"]["requestBody"]["content"]["multipart/mixed"]["prefixEncoding"]
            [0][field] = value;
        let c = contract(&[("api.json", invalid)]);
        let errors =
            rust_http::plan_http(c.clone(), &selected(&c), Default::default()).unwrap_err();
        assert!(
            errors.iter().any(|d| d.code == code
                && d.source.document() == c.entry()
                && d.source.pointer().ends_with(suffix)
                && !d.at.is_empty()),
            "{errors:?}"
        );
    }
    // RFC6570 fields are active for form-data. Supply its mandatory disposition
    // headers so the remaining refusal is Rust's actual positional style fence.
    let content = positional["paths"]["/parts"]["post"]["requestBody"]["content"]
        .as_object_mut()
        .unwrap();
    let mut form_data = content.remove("multipart/mixed").unwrap();
    let disposition = json!({"Content-Disposition":{"required":true,"schema":{"type":"string"}}});
    form_data["prefixEncoding"][0]["headers"] = disposition.clone();
    form_data["itemEncoding"]["headers"] = disposition;
    content.insert("multipart/form-data".into(), form_data);
    let c = contract(&[("api.json", positional)]);
    let errors = rust_http::plan_http(c.clone(), &selected(&c), Default::default()).unwrap_err();
    assert!(errors.iter().any(|d| {
        d.code == "http-rust-positional-style-unsupported"
            && d.source.document() == c.entry()
            && d.source
                .pointer()
                .ends_with("/multipart~1form-data/prefixEncoding/0")
            && !d.at.is_empty()
    }));
}

#[test]
fn method_and_querystring_additions_use_native_typed_descriptors() {
    let f = fixture();
    let methods = plan(f["oas32Methods"]["document"].clone());
    for vector in f["oas32Methods"]["expected"].as_array().unwrap() {
        let op = methods
            .operations()
            .iter()
            .find(|op| op.source.pointer() == vector["source"].as_str().unwrap())
            .unwrap();
        assert_eq!(
            op.wire().method().as_str(),
            vector["token"].as_str().unwrap()
        );
    }
    for case in f["querystringCases"].as_array().unwrap() {
        let p = plan(
            json!({"openapi":"3.2.0","info":{"title":"Querystring","version":"1"},"paths":{"/q":{"query":{"operationId":"query","parameters":[case["parameter"].clone()],"responses":{"204":{}}}}}}),
        );
        let parameter = &p.operations()[0].parameters()[0];
        assert_eq!(
            parameter.wire().location(),
            protocol::ParameterLocation::Querystring
        );
        assert!(
            p.symbols()
                .contains_key(parameter.wire().codec().schema().id())
        );
    }
}

#[test]
fn native_interface_is_stable_across_documentation_and_source_relocation() {
    let api = expanded_api();
    let before = plan(api.clone());
    let mut changed = api;
    changed["info"]["title"] = json!("A longer documentation title moves every source span");
    changed["components"]["schemas"]["Event"]["description"] = json!("Event documentation only");
    changed["paths"]["/headers"]["get"]["responses"]["200"]["headers"]["X-Rate"]["description"] =
        json!("Header prose");
    changed["paths"]["/upload"]["post"]["requestBody"]["description"] =
        json!("Native byte upload documentation");
    let after = plan(changed);
    for left in before.operations() {
        let right = operation(&after, &left.operation_id);
        assert_ne!(left.source.document(), right.source.document());
        assert_eq!(
            left.interface(),
            right.interface(),
            "native interface changed for {}",
            left.operation_id
        );
    }
}

fn expanded_api() -> Value {
    let f = fixture();
    let mut api = json!({"openapi":"3.2.0","info":{"title":"Expanded native protocol","version":"1"},"servers":[{"url":"https://example.test/v1"}],"paths":{},"components":{"schemas":{
        "Event":{"type":"object","required":["data"],"additionalProperties":false,"properties":{"data":{"type":"string","minLength":1},"id":{"type":"string"},"event":{"type":"string"},"retry":{"type":"integer","minimum":0}}},
        "Record":{"type":"object","required":["n"],"additionalProperties":false,"properties":{"n":{"type":"integer","minimum":1}}}
    },"securitySchemes":f["securityCases"]["schemes"].clone()}});
    let paths = api["paths"].as_object_mut().unwrap();
    for (index, case) in f["parameterCases"].as_array().unwrap().iter().enumerate() {
        let path = if case["parameter"]["in"] == "path" {
            format!("/vector{index}/{{color}}")
        } else {
            format!("/vector{index}")
        };
        paths.insert(path,json!({"get":{"operationId":format!("vector{index}"),"parameters":[case["parameter"].clone()],"responses":{"204":{}}}}));
    }
    for (index, case) in f["querystringCases"].as_array().unwrap().iter().enumerate() {
        paths.insert(format!("/querystring{index}"),json!({"query":{"operationId":format!("querystring{index}"),"parameters":[case["parameter"].clone()],"responses":{"204":{}}}}));
    }
    paths.insert(
        "/methods".into(),
        f["oas32Methods"]["document"]["paths"]["/methods"].clone(),
    );
    paths.insert(
        "/status".into(),
        json!({"get":{"operationId":"status","responses":{
            "200":{"content":{"application/json":{"schema":{"type":"integer","minimum":1}}}},
            "2XX":{"content":{"text/plain":{"schema":{"type":"string"}}}},
            "default":{"content":{"application/octet-stream":{}}}
        }}}),
    );
    paths.insert("/fallback".into(),json!({"get":{"operationId":"fallback","responses":{"default":{"content":{"application/json":{"schema":{"type":"boolean"}}}}}}}));
    let mut media =
        f["responseSelection"]["document"]["paths"]["/items"]["get"]["responses"]["200"]["content"]
            .clone();
    media["application/json; profile=V1"] = json!({"schema":{"type":"object","required":["value"],"additionalProperties":false,"properties":{"value":{"type":"string"}}}});
    paths.insert(
        "/media".into(),
        json!({"get":{"operationId":"media","responses":{"200":{"content":media}}}}),
    );
    paths.insert("/headers".into(),json!({"get":{"operationId":"headers","responses":{"200":{"headers":{
        "X-Rate":{"required":true,"schema":{"type":"integer","minimum":0}},
        "X-Note":{"schema":{"type":"string"}},
        "X-Flags":{"schema":{"type":"array","items":{"type":"boolean"}}},
        "X-Object":{"explode":true,"schema":{"type":"object","properties":{"a":{"type":"integer"},"b":{"type":"boolean"}},"additionalProperties":false}},
        "X-Content":{"content":{"text/plain":{"schema":{"type":"integer"}}}},
        "X-Json":{"content":{"application/json":{"schema":{"type":"array","items":{"type":"integer"}}}}},
        "Set-Cookie":{"schema":{"type":"string"}}
    },"links":{"same":{"operationId":"headers","parameters":{"x":"$response.header.X-Rate"},"requestBody":{"$ref":"literal"}}},"content":{"text/plain":{"schema":{"type":"string"}}}}}}}));
    paths.insert("/head".into(),json!({"head":{"operationId":"head","responses":{"200":{"content":{"application/json":{"schema":{"type":"object","required":["absent"],"properties":{"absent":{"type":"string"}}}}}}}}}));
    paths.insert(
        "/undeclared".into(),
        json!({"get":{"operationId":"undeclared"}}),
    );
    paths.insert(
        "/anonymous".into(),
        json!({"options":{"operationId":"anonymous","responses":{"204":{}}}}),
    );
    paths.insert("/security".into(),json!({"trace":{"operationId":"security","security":f["securityCases"]["security"].clone(),"responses":{"204":{}}}}));
    paths.insert("/key-locations".into(),json!({"get":{"operationId":"keyLocations","security":[{"queryKey":[],"cookieKey":[]}],"responses":{"204":{}}}}));
    paths.insert("/servers".into(),json!({"get":{"operationId":"servers","servers":f["serverCases"]["servers"].clone(),"responses":{"204":{}}}}));
    paths.insert("/http".into(),json!({"get":{"operationId":"http","servers":[{"url":"http://api.example.test/base"}],"responses":{"204":{}}}}));
    paths.insert("/binary".into(),json!({"put":{"operationId":"binary","requestBody":{"required":true,"content":{"application/octet-stream":{"schema":{"maxLength":8}}}},"responses":{"200":{"content":{"application/octet-stream":{"schema":{"maxLength":8}}}}}}}));
    paths.insert("/text".into(),json!({"put":{"operationId":"text","requestBody":{"required":true,"content":{"text/plain; charset=utf-8":{"schema":{"type":"boolean"}}}},"responses":{"200":{"content":{"text/plain":{"schema":{"type":"boolean"}}}}}}}));
    paths.insert("/request-choice".into(),json!({"post":{"operationId":"requestChoice","requestBody":{"required":true,"content":{"application/json":{"schema":{"type":"integer","minimum":1}},"*/*":{}}},"responses":{"204":{}}}}));
    let multipart =
        f["multipart"]["paths"]["/upload"]["post"]["requestBody"]["content"]["multipart/form-data"]
            .clone();
    paths.insert("/upload".into(),json!({"post":{"operationId":"upload","requestBody":{"required":true,"content":{"multipart/form-data":multipart.clone()}},"responses":{"204":{}}}}));
    paths.insert("/download-parts".into(),json!({"get":{"operationId":"downloadParts","responses":{"200":{"content":{"multipart/form-data":multipart}}}}}));
    let form=f["form"]["paths"]["/forms"]["post"]["requestBody"]["content"]["application/x-www-form-urlencoded"].clone();
    // This fixture specifically witnesses 3.1 whole-property style encoding;
    // the separate 3.2 cases below exercise per-item style application.
    paths.insert("/form".into(),json!({"post":{"operationId":"form","requestBody":{"required":true,"content":{"application/x-www-form-urlencoded":form.clone()}},"responses":{"200":{"content":{"application/x-www-form-urlencoded":form}}}}}));
    let positional = f["positionalMultipart"]["paths"]["/parts"]["post"]["requestBody"]["content"]
        ["multipart/mixed"]
        .clone();
    paths.insert("/parts".into(),json!({"post":{"operationId":"parts","requestBody":{"required":true,"content":{"multipart/mixed":positional.clone()}},"responses":{"200":{"content":{"multipart/mixed":positional}}}}}));
    paths.insert("/files".into(),json!({"post":{"operationId":"files","requestBody":{"required":true,"content":{"multipart/form-data":{"schema":{"type":"object","required":["files"],"additionalProperties":false,"properties":{"files":{"type":"array","minItems":1,"maxItems":2,"items":{"maxLength":3}}}}}}},"responses":{"204":{}}}}));
    let styled = json!({"schema":{"type":"object","required":["filters","tree"],"additionalProperties":false,"properties":{
        "filters":{"type":"array","minItems":1,"items":{"type":"array","minItems":1,"items":{"type":"string"}}},
        "tree":{"type":"object","required":["a"],"additionalProperties":false,"properties":{"a":{"type":"string"}}}
    }},"encoding":{"filters":{"style":"pipeDelimited","explode":false},"tree":{"style":"deepObject","explode":true}}});
    paths.insert("/styled".into(),json!({"post":{"operationId":"styled","requestBody":{"required":true,"content":{"multipart/form-data":styled.clone()}},"responses":{"200":{"content":{"multipart/form-data":styled}}}}}));
    paths.insert("/events".into(),json!({"get":{"operationId":"events","responses":{"200":{"content":{"text/event-stream":{"itemSchema":{"$ref":"#/components/schemas/Event"}}}}}}}));
    paths.insert("/lines".into(),json!({"get":{"operationId":"lines","responses":{"200":{"content":{"application/x-ndjson":{"itemSchema":{"$ref":"#/components/schemas/Record"}}}}}}}));
    paths.insert("/send-events".into(),json!({"post":{"operationId":"sendEvents","requestBody":{"required":true,"content":{"text/event-stream":{"itemSchema":{"$ref":"#/components/schemas/Event"}}}},"responses":{"204":{}}}}));
    paths.insert("/send-lines".into(),json!({"post":{"operationId":"sendLines","requestBody":{"required":true,"content":{"application/jsonl":{"itemSchema":{"$ref":"#/components/schemas/Record"}}}},"responses":{"204":{}}}}));
    api["components"]["securitySchemes"]["queryKey"] =
        json!({"type":"apiKey","in":"query","name":"key"});
    api["components"]["securitySchemes"]["cookieKey"] =
        json!({"type":"apiKey","in":"cookie","name":"session"});
    api
}

fn checked(command: &mut Command, retained: &Path) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "fixture {}\n{command:?}\n{}{}",
        retained.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn cargo(mode: &str, manifest: &Path, target: &Path) -> Command {
    let mut command = Command::new("cargo");
    command
        .args([mode, "--offline", "--quiet", "--manifest-path"])
        .arg(manifest)
        .arg("--target-dir")
        .arg(target)
        .env_remove("RUST_MIN_STACK")
        .env("RUSTFLAGS", "-D warnings")
        .env("RUSTDOCFLAGS", "-D warnings");
    if let Some(toolchain) = std::env::var_os("SUSPECT_NATIVE_RUST_TOOLCHAIN") {
        command.env("RUSTUP_TOOLCHAIN", toolchain);
    }
    command
}
fn aliases(plan: &HttpPlan) -> String {
    let mut code = String::new();
    for (id, alias) in [
        ("upload", "UploadBody"),
        ("form", "FormBody"),
        ("parts", "PartsBody"),
        ("files", "FilesBody"),
        ("styled", "StyledBody"),
    ] {
        let op = operation(plan, id);
        let Payload::Parts(a) = &op.body().unwrap().media()[0].payload else {
            panic!("native aggregate")
        };
        code.push_str(&format!(
            "use sdk::operations::{}::{} as {alias};\n",
            op.module_name, a.type_name
        ));
        if id == "upload" {
            let p = a
                .parts()
                .iter()
                .find(|p| p.wire().name() == Some("file"))
                .unwrap();
            code.push_str(&format!(
                "use sdk::operations::{}::{} as FileHeaders;\n",
                op.module_name,
                p.headers_type.as_ref().unwrap()
            ));
        }
        if id == "styled" {
            let tree = a
                .parts()
                .iter()
                .find(|p| p.wire().name() == Some("tree"))
                .unwrap();
            code.push_str(&format!(
                "use sdk::models::{} as StyledTree;\n",
                tree.model.as_ref().unwrap()
            ));
        }
    }
    code
}

fn consumer(plan: &HttpPlan) -> String {
    let mut code = format!(
        "#[cfg(test)]\nmod acceptance {{\n{}\n{SUPPORT}\n{BEHAVIOR}\n",
        aliases(plan)
    );
    let fixture = fixture();
    for (index, case) in fixture["parameterCases"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let op = operation(plan, &format!("vector{index}"));
        let p = &op.parameters()[0];
        let value = case["value"].to_string();
        let wire = case["wire"].as_str().unwrap();
        let input = if p.wire().required() {
            format!(
                "sdk::operations::{}::{}::new(value)",
                op.module_name, op.input_type
            )
        } else {
            format!(
                "sdk::operations::{}::{}::new().with_{}(value)",
                op.module_name, op.input_type, p.name
            )
        };
        let url = if case["parameter"]["in"] == "path" {
            format!("https://example.test/v1/vector{index}/{wire}")
        } else if case["parameter"]["in"] == "query" {
            format!("https://example.test/v1/vector{index}?{wire}")
        } else {
            format!("https://example.test/v1/vector{index}")
        };
        let header = if case["parameter"]["in"] == "header" {
            Some(case["parameter"]["name"].as_str().unwrap())
        } else if case["parameter"]["in"] == "cookie" {
            Some("cookie")
        } else {
            None
        };
        code.push_str(&format!("#[tokio::test]\nasync fn normative_vector_{index}(){{let (client,seen,_)=mock(204,None,vec![]);let value=sdk::codecs::{}Codec::decode({value:?}).unwrap();client.{}({input}).await.unwrap();let records=seen.lock().unwrap();assert_eq!(records[0].url,{url:?});{} }}\n",p.model,op.function_name,header.map(|h|format!("assert_eq!(header(&records[0],{h:?}),{wire:?}.as_bytes());")).unwrap_or_default()));
    }
    for (index, case) in fixture["querystringCases"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let op = operation(plan, &format!("querystring{index}"));
        let p = &op.parameters()[0];
        let value = case["value"].to_string();
        let url = format!(
            "https://example.test/v1/querystring{index}?{}",
            case["wire"].as_str().unwrap()
        );
        let input = if p.wire().required() {
            format!(
                "sdk::operations::{}::{}::new(value)",
                op.module_name, op.input_type
            )
        } else {
            format!(
                "sdk::operations::{}::{}::new().with_{}(value)",
                op.module_name, op.input_type, p.name
            )
        };
        code.push_str(&format!("#[tokio::test]\nasync fn whole_querystring_{index}(){{let (client,seen,_)=mock(204,None,vec![]);let value=sdk::codecs::{}Codec::decode({value:?}).unwrap();client.{}({input}).await.unwrap();let records=seen.lock().unwrap();assert_eq!(records[0].url,{url:?});assert_eq!(records[0].method,\"QUERY\");}}\n",p.model,op.function_name));
        if index == 2 {
            code.push_str(&format!("#[tokio::test]\nasync fn whole_query_form_honors_lower_part_budget(){{let (client,seen,_)=mock(204,None,vec![]);let client=client.with_options(ClientOptions{{max_part_bytes:Some(1),..Default::default()}});let value=sdk::codecs::{}Codec::decode({value:?}).unwrap();assert!(matches!(client.{}({input}).await.unwrap_err(),sdk::operations::{}::{}::Sdk(e) if e.kind==SdkErrorKind::ResourceLimit));assert!(seen.lock().unwrap().is_empty());}}\n",p.model,op.function_name,op.module_name,op.error_type));
        }
    }
    code.push_str("}\n");
    code.push_str(NEGATIVE_TYPING);
    code
}

#[test]
#[ignore = "requires native Cargo, Rust 1.88/current, pinned reqwest cache and tar"]
fn installed_native_protocol_consumer_wire_bytes_docs_and_ownership() {
    let plan = plan(expanded_api());
    let directory = tempfile::Builder::new()
        .prefix("rust-protocol-")
        .tempdir()
        .unwrap()
        .keep();
    let files = rust_http::emit_http(
        &plan,
        &PackageConfig {
            name: "native-protocol-sdk".into(),
            version: "0.0.0".into(),
        },
    )
    .unwrap();
    suspect_codegen::write_files(&files, &directory).unwrap();
    // Re-emission is deterministic; owned-file checks cover the expanded runtime
    // tree as well as operation/model artifacts.
    let repeated = rust_http::emit_http(
        &plan,
        &PackageConfig {
            name: "native-protocol-sdk".into(),
            version: "0.0.0".into(),
        },
    )
    .unwrap();
    assert_eq!(
        files
            .iter()
            .map(|f| (&f.path, &f.content))
            .collect::<Vec<_>>(),
        repeated
            .iter()
            .map(|f| (&f.path, &f.content))
            .collect::<Vec<_>>()
    );
    let target = std::env::var_os("SUSPECT_RUST_PROTOCOL_TARGET")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
                if std::env::var_os("SUSPECT_NATIVE_RUST_TOOLCHAIN").is_some() {
                    "../../target/native-rust-protocol-msrv"
                } else {
                    "../../target/native-rust-protocol"
                },
            )
        });
    let manifest = directory.join("rust/Cargo.toml");
    checked(
        cargo("check", &manifest, &target).arg("--no-default-features"),
        &directory,
    );
    checked(
        cargo("test", &manifest, &target).args(["--doc", "--features", "reqwest-rustls"]),
        &directory,
    );
    checked(
        cargo("doc", &manifest, &target).args(["--no-deps", "--features", "reqwest-rustls"]),
        &directory,
    );
    checked(
        cargo("run", &manifest, &target).args(["--example", "validated", "--features", "http"]),
        &directory,
    );
    checked(
        cargo("package", &manifest, &target).args(["--allow-dirty", "--no-verify"]),
        &directory,
    );
    let consumer_dir = directory.join("consumer");
    std::fs::create_dir_all(consumer_dir.join("vendor")).unwrap();
    std::fs::create_dir_all(consumer_dir.join("src")).unwrap();
    checked(
        Command::new("tar")
            .arg("-xzf")
            .arg(target.join("package/native-protocol-sdk-0.0.0.crate"))
            .arg("-C")
            .arg(consumer_dir.join("vendor")),
        &directory,
    );
    std::fs::write(consumer_dir.join("Cargo.toml"),"[package]\nname=\"protocol-consumer\"\nversion=\"0.0.0\"\nedition=\"2024\"\n[workspace]\n[dependencies]\nsdk={package=\"native-protocol-sdk\",path=\"vendor/native-protocol-sdk-0.0.0\",features=[\"reqwest-rustls\"]}\ntokio={version=\"=1.53.1\",features=[\"macros\",\"rt\",\"time\"]}\n").unwrap();
    std::fs::write(consumer_dir.join("src/lib.rs"), consumer(&plan)).unwrap();
    // Same package names with normalized archive mtimes cannot reuse stale rlibs.
    let archive = std::fs::read(target.join("package/native-protocol-sdk-0.0.0.crate")).unwrap();
    use sha2::{Digest, Sha256};
    let digest = format!("{:x}", Sha256::digest(&archive));
    checked(
        &mut cargo(
            "test",
            &consumer_dir.join("Cargo.toml"),
            &target.join("installed").join(digest),
        ),
        &directory,
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
#[ignore = "requires native Cargo and the pinned URL dependency"]
fn native_rootless_bytes_empty_forms_and_oas30_packages() {
    let rootless = json!({"openapi":"3.2.0","info":{"title":"Rootless","version":"1"},"servers":[{"url":"https://example.test"}],"paths":{
        "/bytes":{"put":{"operationId":"bytes","requestBody":{"required":true,"content":{"application/octet-stream":{}}},"responses":{"204":{}}}},
        "/empty":{"post":{"operationId":"empty","requestBody":{"required":true,"content":{"application/x-www-form-urlencoded":{"schema":{"type":"object","additionalProperties":false}}}},"responses":{"204":{"content":{"application/json":{"schema":{"type":"object"}}}}}}},
        "/unknown":{"get":{"operationId":"unknown"}}
    }});
    let legacy = json!({"openapi":"3.0.4","info":{"title":"Legacy byte context","version":"1"},"servers":[{"url":"https://example.test"}],"paths":{
        "/file":{"post":{"operationId":"file","requestBody":{"required":true,"content":{"application/octet-stream":{"schema":{"type":"string","format":"binary"}}}},"responses":{"200":{"description":"raw","content":{"application/octet-stream":{"schema":{"type":"string","format":"binary"}}}}}}}
    }});
    for (index, api) in [rootless, legacy].into_iter().enumerate() {
        let plan = plan(api);
        assert!(plan.protocol().codec_roots().is_empty());
        let directory = tempfile::Builder::new()
            .prefix("rust-protocol-empty-")
            .tempdir()
            .unwrap()
            .keep();
        suspect_codegen::write_files(
            &rust_http::emit_http(
                &plan,
                &PackageConfig {
                    name: format!("rootless-protocol-{index}"),
                    version: "0.0.0".into(),
                },
            )
            .unwrap(),
            &directory,
        )
        .unwrap();
        let target = std::env::var_os("SUSPECT_RUST_PROTOCOL_TARGET")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/native-rust-protocol")
            });
        let manifest = directory.join("rust/Cargo.toml");
        checked(
            cargo("check", &manifest, &target).arg("--no-default-features"),
            &directory,
        );
        checked(
            cargo("test", &manifest, &target).args(["--doc", "--features", "http"]),
            &directory,
        );
        checked(
            cargo("run", &manifest, &target).args(["--example", "validated", "--features", "http"]),
            &directory,
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
#[ignore = "requires native Cargo/current or 1.88, the pinned URL dependency and tar"]
fn installed_native_multipart_ignored_styles_preserve_content() {
    // Use the shared literal source unchanged, including both directions, a
    // referenced Media Type Object and ignored fields on prefix/item encodings.
    let plan = plan(fixture()["ignoredMultipartEncoding"]["document"].clone());
    let op = &plan.operations()[0];
    let Payload::Parts(body) = &op.body().unwrap().media()[0].payload else {
        panic!("native request aggregate")
    };
    let Payload::Parts(response) = &op.responses()[0].variants()[0].payload else {
        panic!("native response aggregate")
    };
    assert_eq!(body.parts().len(), 4);
    assert!(
        body.parts()
            .iter()
            .all(|p| p.wire().multiplicity() == protocol::PartMultiplicity::One)
    );
    let headers = body.parts()[0].headers_type.as_ref().unwrap();
    let tail = body.additional().unwrap();
    let mut text = IGNORED_MULTIPART_CONSUMER.to_owned();
    for (key, value) in [
        ("__MODULE__", op.module_name.as_str()),
        ("__INPUT__", op.input_type.as_str()),
        ("__FUNCTION__", op.function_name.as_str()),
        ("__ERROR__", op.error_type.as_str()),
        ("__BODY__", body.type_name.as_str()),
        ("__HEADERS__", headers.as_str()),
        ("__TAIL_MODEL__", tail.model.as_deref().unwrap()),
        ("__ITEMS__", tail.name.as_str()),
        ("__PART0__", body.parts()[0].name.as_str()),
        ("__OUT0__", response.parts()[0].name.as_str()),
        ("__OUT1__", response.parts()[1].name.as_str()),
        ("__OUT2__", response.parts()[2].name.as_str()),
        ("__OUT3__", response.parts()[3].name.as_str()),
        (
            "__OUT_ITEMS__",
            response.additional().unwrap().name.as_str(),
        ),
        (
            "__HEADER_MEMBER__",
            response.parts()[0].headers()[0].name.as_str(),
        ),
    ] {
        text = text.replace(key, value);
    }
    let directory = tempfile::Builder::new()
        .prefix("rust-ignored-multipart-")
        .tempdir()
        .unwrap()
        .keep();
    suspect_codegen::write_files(
        &rust_http::emit_http(
            &plan,
            &PackageConfig {
                name: "ignored-multipart-sdk".into(),
                version: "0.0.0".into(),
            },
        )
        .unwrap(),
        &directory,
    )
    .unwrap();
    let target = std::env::var_os("SUSPECT_RUST_IGNORED_MULTIPART_TARGET")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
                if std::env::var_os("SUSPECT_NATIVE_RUST_TOOLCHAIN").is_some() {
                    "../../target/native-rust-ignored-multipart-msrv"
                } else {
                    "../../target/native-rust-ignored-multipart"
                },
            )
        });
    let check = |command: &mut Command| {
        use std::io::Write;
        let output = command.output().unwrap();
        let mut log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(directory.join("commands.log"))
            .unwrap();
        writeln!(
            log,
            "{command:?}\nstatus: {}\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
        .unwrap();
        assert!(
            output.status.success(),
            "retained {}\n{command:?}\n{}{}",
            directory.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    };
    let manifest = directory.join("rust/Cargo.toml");
    check(cargo("test", &manifest, &target).args(["--doc", "--features", "http"]));
    check(cargo("package", &manifest, &target).args(["--allow-dirty", "--no-verify"]));
    let consumer = directory.join("consumer");
    std::fs::create_dir_all(consumer.join("src")).unwrap();
    std::fs::create_dir_all(consumer.join("vendor")).unwrap();
    let archive = target.join("package/ignored-multipart-sdk-0.0.0.crate");
    check(
        Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(consumer.join("vendor")),
    );
    std::fs::write(consumer.join("Cargo.toml"),"[package]\nname=\"ignored-multipart-consumer\"\nversion=\"0.0.0\"\nedition=\"2024\"\n[workspace]\n[dependencies]\nsdk={package=\"ignored-multipart-sdk\",path=\"vendor/ignored-multipart-sdk-0.0.0\",features=[\"http\"]}\ntokio={version=\"=1.53.1\",features=[\"macros\",\"rt\"]}\n").unwrap();
    std::fs::write(consumer.join("src/lib.rs"), text).unwrap();
    use sha2::{Digest, Sha256};
    let digest = format!("{:x}", Sha256::digest(std::fs::read(&archive).unwrap()));
    check(
        cargo(
            "test",
            &consumer.join("Cargo.toml"),
            &target.join("installed").join(digest),
        )
        .args(["--", "--test-threads=1"]),
    );
    eprintln!("native ignored multipart retained {}", directory.display());
}

const IGNORED_MULTIPART_CONSUMER: &str = r###"
#[cfg(test)]
mod witness {
    use sdk::{Client,ClientOptions,Credentials,http::{Part,BoxError,Request,ResponseBody,Transport,TransportResponse,SdkErrorKind}};
    use sdk::operations::__MODULE__::{self as op,__INPUT__ as Input,__BODY__ as InputBody,__HEADERS__ as PartHeaders,__ERROR__ as OperationError};
    use sdk::models::__TAIL_MODEL__ as Tail;
    use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};
    const PARTS: &[(&str,&[u8])] = &[
        ("application/json",br#""literal & =""#),
        ("text/plain",b"9007199254740993"),
        ("application/json",br#"["x & y","z"]"#),
        ("application/octet-stream",&[0,255,42]),
        ("application/json",br#"{"n":7}"#),
    ];
    fn body()->InputBody {
        let mut body=InputBody::new(
            Part::with_headers("literal & =".into(),PartHeaders::new("11".parse().unwrap())),
            Part::new("9007199254740993".parse().unwrap()),
            Part::new(vec!["x & y".into(),"z".into()]),
            Part::new(vec![0,255,42]),
        );
        let mut tail=Tail::new();tail.n=Some("7".parse().unwrap());body.__ITEMS__.push(Part::new(tail));body
    }
    // Independent literal MIME oracle: source content encodings must survive
    // ignored style/explode/allowReserved without invented part names/grouping.
    fn assert_request(request:&Request) {
        assert_eq!(request.method,"POST");assert_eq!(request.url,"https://example.test/parts");
        let media=request.headers.iter().find(|(key,_)|key.eq_ignore_ascii_case("content-type")).unwrap();
        let content_type=std::str::from_utf8(&media.1).unwrap();assert!(content_type.starts_with("multipart/mixed;"));
        let boundary=content_type.split("boundary=").nth(1).unwrap().trim_matches('"');
        let marker=format!("--{boundary}\r\n");let mut remaining=request.body.as_deref().unwrap();
        for (index,(media,body)) in PARTS.iter().enumerate() {
            remaining=remaining.strip_prefix(marker.as_bytes()).expect("one actual MIME part per schema position");
            let end=remaining.windows(4).position(|v|v==b"\r\n\r\n").unwrap();
            let headers=std::str::from_utf8(&remaining[..end]).unwrap();
            let header=|name:&str|headers.lines().filter_map(|line|line.split_once(':')).find(|(key,_)|key.eq_ignore_ascii_case(name)).map(|(_,v)|v.trim());
            assert_eq!(header("content-type"),Some(*media));
            assert_eq!(header("x-part"),if index==0{Some("11")}else{None});
            assert_eq!(header("content-disposition"),None,"mixed positions must not invent form variable names");
            remaining=remaining[end+4..].strip_prefix(*body).expect("content codec wire bytes");
            remaining=remaining.strip_prefix(b"\r\n").unwrap();
        }
        assert_eq!(remaining,format!("--{boundary}--\r\n").as_bytes());
    }
    fn response_bytes(invalid:bool)->Vec<u8> {
        let mut out=Vec::new();
        for (index,(media,body)) in PARTS.iter().enumerate() {
            out.extend_from_slice(format!("--response\r\nContent-Type: {media}\r\n").as_bytes());
            if index==0 {out.extend_from_slice(b"X-Part: 11\r\n");}
            out.extend_from_slice(b"\r\n");out.extend_from_slice(if invalid&&index==0{b"false"}else{*body});out.extend_from_slice(b"\r\n");
        }
        out.extend_from_slice(b"--response--\r\n");out
    }
    struct Bytes(Option<Vec<u8>>);
    impl ResponseBody for Bytes {async fn next_chunk(&mut self)->Result<Option<Vec<u8>>,BoxError>{Ok(self.0.take())}}
    struct LiteralTransport {sent:Arc<AtomicUsize>,invalid:bool}
    impl Transport for LiteralTransport {
        type Body=Bytes;
        async fn send(&self,request:Request)->Result<TransportResponse<Bytes>,BoxError> {
            assert_request(&request);self.sent.fetch_add(1,Ordering::SeqCst);
            Ok(TransportResponse{status:200,headers:vec![("Content-Type".into(),b"multipart/mixed; boundary=response".to_vec())],body:Bytes(Some(response_bytes(self.invalid)))})
        }
    }
    fn client(invalid:bool)->(Client<LiteralTransport>,Arc<AtomicUsize>) {
        let sent=Arc::new(AtomicUsize::new(0));
        (Client::with_transport(LiteralTransport{sent:sent.clone(),invalid},Credentials::new()).with_options(ClientOptions{server_url:Some("https://example.test".into()),..Default::default()}),sent)
    }
    #[tokio::test]
    async fn content_codecs_and_whole_array_grouping_survive_both_directions() {
        let (client,sent)=client(false);
        let output=op::__FUNCTION__(&client,Input::new(body())).await.unwrap().into_data();
        assert_eq!(output.__OUT0__.data,"literal & =");assert_eq!(output.__OUT0__.headers.__HEADER_MEMBER__.as_str(),"11");
        assert_eq!(output.__OUT1__.data.as_str(),"9007199254740993");
        assert_eq!(output.__OUT2__.data,vec!["x & y".to_owned(),"z".to_owned()]);assert_eq!(output.__OUT3__.data,[0,255,42]);
        assert_eq!(output.__OUT_ITEMS__.len(),1);assert_eq!(output.__OUT_ITEMS__[0].data.n.as_ref().unwrap().as_str(),"7");
        let mut invalid=body();invalid.__PART0__.content_type=Some("text/plain".into());
        let OperationError::Sdk(error)=op::__FUNCTION__(&client,Input::new(invalid)).await.unwrap_err() else{panic!("part media refusal")};
        assert_eq!(error.kind,SdkErrorKind::RequestRepresentation);assert_eq!(sent.load(Ordering::SeqCst),1);
    }
    #[tokio::test]
    async fn ignored_fields_never_bypass_the_actual_response_schema() {
        let (client,sent)=client(true);
        let OperationError::Sdk(error)=op::__FUNCTION__(&client,Input::new(body())).await.unwrap_err() else{panic!("response codec refusal")};
        assert_eq!(error.kind,SdkErrorKind::ResponseDecoding);assert_eq!(error.status,Some(200));
        assert_eq!(error.source.document,op::OPERATION.source.document);
        assert_eq!(error.source.pointer,"/components/mediaTypes/Mixed/schema/prefixItems/0");
        assert_eq!(sent.load(Ordering::SeqCst),1);
    }
}
"###;

const SUPPORT: &str = r#"
use sdk::{Client,ClientOptions,Credentials,JsonInteger};
use sdk::http::{BoxError,Headers,Request,ResponseBody,Transport,TransportResponse,SdkErrorKind};
use std::{collections::VecDeque,sync::{Arc,Mutex,atomic::{AtomicUsize,Ordering}},future::Future,task::{Context,Poll,Wake,Waker}};
#[derive(Clone)] enum Chunk{Bytes(Vec<u8>),Pending,Fail}
struct Body{chunks:VecDeque<Chunk>,drops:Arc<AtomicUsize>,polls:Arc<AtomicUsize>}
impl Drop for Body{fn drop(&mut self){self.drops.fetch_add(1,Ordering::SeqCst);}}
impl ResponseBody for Body{async fn next_chunk(&mut self)->Result<Option<Vec<u8>>,BoxError>{self.polls.fetch_add(1,Ordering::SeqCst);match self.chunks.pop_front(){Some(Chunk::Bytes(v))=>Ok(Some(v)),Some(Chunk::Pending)=>std::future::pending().await,Some(Chunk::Fail)=>Err("private body failure".into()),None=>Ok(None)}}}
struct Mock{status:u16,headers:Headers,chunks:Vec<Chunk>,seen:Arc<Mutex<Vec<Request>>>,drops:Arc<AtomicUsize>,polls:Arc<AtomicUsize>}
impl Transport for Mock{type Body=Body;async fn send(&self,request:Request)->Result<TransportResponse<Body>,BoxError>{self.seen.lock().unwrap().push(request);Ok(TransportResponse{status:self.status,headers:self.headers.clone(),body:Body{chunks:self.chunks.clone().into(),drops:self.drops.clone(),polls:self.polls.clone()}})}}
fn raw_mock(status:u16,headers:Headers,chunks:Vec<Chunk>,credentials:Credentials)->(Client<Mock>,Arc<Mutex<Vec<Request>>>,Arc<AtomicUsize>,Arc<AtomicUsize>){let seen=Arc::new(Mutex::new(Vec::new()));let drops=Arc::new(AtomicUsize::new(0));let polls=Arc::new(AtomicUsize::new(0));(Client::with_transport(Mock{status,headers,chunks,seen:seen.clone(),drops:drops.clone(),polls:polls.clone()},credentials),seen,drops,polls)}
fn mock(status:u16,media:Option<&str>,bytes:Vec<u8>)->(Client<Mock>,Arc<Mutex<Vec<Request>>>,Arc<AtomicUsize>){let (client,seen,drops,_)=raw_mock(status,media.map(|m|vec![("Content-Type".into(),m.as_bytes().to_vec())]).unwrap_or_default(),vec![Chunk::Bytes(bytes)],Credentials::new());(client,seen,drops)}
fn header<'a>(request:&'a Request,name:&str)->&'a [u8]{request.headers.iter().find(|(n,_)|n.eq_ignore_ascii_case(name)).map(|(_,v)|v.as_slice()).unwrap_or(&[])}
struct Noop;impl Wake for Noop{fn wake(self:Arc<Self>){}}
"#;

const BEHAVIOR: &str = r##"
#[tokio::test]
async fn actual_status_exact_range_default_and_forbidden_content(){
    use sdk::operations::status::{StatusSuccess,StatusError,StatusApiError};
    let (client,_,_)=mock(200,Some("application/json"),b"9007199254740993".to_vec());
    let StatusSuccess::Status200(value)=client.status_default().await.unwrap()else{panic!("exact")};assert_eq!(value.data.as_str(),"9007199254740993");
    let (client,_,_)=mock(201,Some("text/plain; charset=UTF-8"),b"created".to_vec());
    let StatusSuccess::Range2XX(value)=client.status_default().await.unwrap()else{panic!("range")};assert_eq!(value.data,"created");assert_eq!(value.status,201);
    let (client,_,_)=mock(404,Some("application/octet-stream"),vec![0,255,42]);
    let StatusError::Api(error)=client.status_default().await.unwrap_err()else{panic!("API error")};let StatusApiError::Default(value)=*error else{panic!("default")};assert_eq!(value.status,404);assert_eq!(value.data,[0,255,42]);
    let (client,_,drops,polls)=raw_mock(204,vec![],vec![Chunk::Pending],Credentials::new());
    assert!(matches!(client.status_default().await.unwrap(),StatusSuccess::Range2XXNoContent(_)));assert_eq!(polls.load(Ordering::SeqCst),0);assert_eq!(drops.load(Ordering::SeqCst),1);
    let (client,_,_)=mock(200,Some("text/plain"),b"not a fallback".to_vec());
    assert!(matches!(client.status_default().await.unwrap_err(),StatusError::Sdk(e) if e.kind==SdkErrorKind::UnexpectedResponse&&e.raw_capture==b"not a fallback"));
    let (client,_,_)=mock(201,Some("application/json"),b"true".to_vec());
    assert!(matches!(client.fallback_default().await.unwrap(),sdk::operations::fallback::FallbackSuccess::Default(v) if v.status==201&&v.data));
    let (client,_,drops,polls)=raw_mock(200,vec![],vec![Chunk::Pending],Credentials::new());
    assert_eq!(client.head_default().await.unwrap().status,200);assert_eq!(polls.load(Ordering::SeqCst),0);assert_eq!(drops.load(Ordering::SeqCst),1);
    let (client,_,_)=mock(200,Some("application/json"),b"true".to_vec());
    assert!(matches!(client.undeclared_default().await.unwrap_err(),sdk::operations::undeclared::UndeclaredError::Sdk(e) if e.kind==SdkErrorKind::UnexpectedResponse));
}

#[tokio::test]
async fn media_precedence_parameters_json_text_and_native_bytes(){
    use sdk::operations::media::{MediaSuccess,MediaError};
    let (client,_,_)=mock(200,Some("Application/JSON; profile=V1; charset=UTF-8"),br#"{"value":"profile"}"#.to_vec());
    let MediaSuccess::Status200ApplicationJsonProfileV1(v)=client.media_default().await.unwrap()else{panic!("parameterized media")};assert_eq!(v.data.value,"profile");
    let (client,_,_)=mock(200,Some("application/json; profile=v1"),b"{}".to_vec());assert!(matches!(client.media_default().await.unwrap(),MediaSuccess::Status200ApplicationJson(_)));
    let (client,_,_)=mock(200,Some("application/pdf"),vec![0,255,0]);assert!(matches!(client.media_default().await.unwrap(),MediaSuccess::Status200ApplicationAny(v) if v.data==[0,255,0]));
    let (client,_,_)=mock(200,Some("image/png"),vec![255,0]);assert!(matches!(client.media_default().await.unwrap(),MediaSuccess::Status200Any(v) if v.data==[255,0]));
    let (client,_,_)=mock(200,Some("application/problem+json"),b"{}".to_vec());assert!(matches!(client.media_default().await.unwrap(),MediaSuccess::Status200ApplicationProblemJson(_)));
    let (client,_,_)=mock(200,Some("text/plain; charset=UTF-8"),"snow 雪".as_bytes().to_vec());assert!(matches!(client.media_default().await.unwrap(),MediaSuccess::Status200TextPlain(v) if v.data=="snow 雪"));
    for media in [None,Some("text/plain; charset=latin-1"),Some("application/json;p=x;P=y"),Some("application/*")]{let (client,_,_)=mock(200,media,b"{}".to_vec());assert!(matches!(client.media_default().await.unwrap_err(),MediaError::Sdk(e) if e.kind==SdkErrorKind::UnexpectedResponse));}
}

#[tokio::test]
async fn typed_headers_and_links_are_source_bound_metadata(){
    let headers=vec![("content-type".into(),b"text/plain".to_vec()),("X-Rate".into(),b"9007199254740993".to_vec()),("X-Note".into(),"\u{a0}kept\u{a0}".as_bytes().to_vec()),("X-Flags".into(),b"true".to_vec()),("x-flags".into(),b"false".to_vec()),("X-Object".into(),b"a=42,b=true".to_vec()),("X-Content".into(),b"42".to_vec()),("X-Json".into(),b"[1,2]".to_vec()),("Set-Cookie".into(),b"sid=x; Path=/".to_vec())];
    let (client,_,_,_)=raw_mock(200,headers.clone(),vec![Chunk::Bytes(b"ok".to_vec())],Credentials::new());let response=client.headers_default().await.unwrap().into_response();
    assert_eq!(response.typed_headers.x_rate.as_str(),"9007199254740993");assert_eq!(response.typed_headers.x_note.as_deref(),Some("\u{a0}kept\u{a0}"));assert_eq!(response.typed_headers.x_flags,Some(vec![true,false]));
    assert_eq!(response.typed_headers.x_object.as_ref().unwrap().a.as_ref().unwrap().as_str(),"42");assert_eq!(response.typed_headers.x_object.as_ref().unwrap().b,Some(true));assert_eq!(response.typed_headers.x_content.unwrap().as_str(),"42");assert_eq!(response.typed_headers.x_json.unwrap().len(),2);assert_eq!(response.links.len(),1);assert_eq!(response.links[0].name,"same");assert!(matches!(response.links[0].request_body.unwrap().value,sdk::http::MetadataValue::Object(_)));
    for invalid in [vec![("content-type".into(),b"text/plain".to_vec())],{let mut h=headers;h.push(("set-cookie".into(),b"sid=y".to_vec()));h}]{let (client,_,_,_)=raw_mock(200,invalid,vec![Chunk::Bytes(b"capture".to_vec())],Credentials::new());assert!(matches!(client.headers_default().await.unwrap_err(),sdk::operations::headers::HeadersError::Sdk(e) if e.kind==SdkErrorKind::ResponseDecoding&&e.status==Some(200)&&e.raw_capture==b"capture"&&e.source.pointer.contains("/headers/")));}
}

#[tokio::test]
async fn wire_hazards_and_omitted_composites_are_checked_before_transport(){
    let (client,seen,_)=mock(204,None,vec![]);
    assert!(client.vector21(sdk::operations::vector21::Vector21::new().with_q("raw&extra=1".into())).await.is_err());
    assert!(client.vector16(sdk::operations::vector16::Vector16::new().with_color(vec!["ambiguous space".into()])).await.is_err());
    assert!(client.vector23(sdk::operations::vector23::Vector23::new().with_x_tag("bad\r\nheader".into())).await.is_err());
    assert!(client.vector26(sdk::operations::vector26::Vector26::new().with_color(vec!["cookie space".into()])).await.is_err());
    assert!(client.vector0(sdk::operations::vector0::Vector0::new("..".into())).await.is_err());assert!(seen.lock().unwrap().is_empty());
    client.vector14(sdk::operations::vector14::Vector14::new().with_color(vec![])).await.unwrap();assert_eq!(seen.lock().unwrap()[0].url,"https://example.test/v1/vector14");
    let credentials=Credentials::new().with_api_key("queryKey","key").with_api_key("cookieKey","raw space");let (client,seen,_,_)=raw_mock(204,vec![],vec![],credentials);assert!(client.key_locations_default().await.is_err());assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn security_alternatives_conjunctions_basic_keys_and_caller_hooks(){
    let (client,seen,_)=mock(204,None,vec![]);client.security_default().await.unwrap();assert!(seen.lock().unwrap()[0].headers.is_empty());
    let creds=Credentials::new().with_bearer("token","opaque").with_api_key("key","also-opaque").select_alternative(1);
    let (client,seen,_,_)=raw_mock(204,vec![],vec![],creds);client.security_default().await.unwrap();let records=seen.lock().unwrap();assert_eq!(header(&records[0],"authorization"),b"Bearer opaque");assert_eq!(header(&records[0],"x-key"),b"also-opaque");drop(records);
    let (client,seen,_,_)=raw_mock(204,vec![],vec![],Credentials::basic("Aladdin","open sesame").select_alternative(4));client.security_default().await.unwrap();assert_eq!(header(&seen.lock().unwrap()[0],"authorization"),b"Basic QWxhZGRpbjpvcGVuIHNlc2FtZQ==");
    let (client,seen,_,_)=raw_mock(204,vec![],vec![],Credentials::new().with_api_key("queryKey","a +雪").with_api_key("cookieKey","raw/secret"));client.key_locations_default().await.unwrap();let records=seen.lock().unwrap();assert_eq!(records[0].url,"https://example.test/v1/key-locations?key=a%20%2B%E9%9B%AA");assert_eq!(header(&records[0],"cookie"),b"session=raw/secret");assert!(!format!("{:?}",records[0]).contains("secret"));drop(records);
    let calls=Arc::new(AtomicUsize::new(0));let called=calls.clone();
    let credentials=Credentials::new().select_alternative(2).with_hook(move |r:&sdk::http::CredentialRequirement|->Result<Option<sdk::http::Credential>,BoxError>{called.fetch_add(1,Ordering::SeqCst);assert_eq!(r.name,"oauth");assert!(r.scheme.terminal.pointer.ends_with("/securitySchemes/oauth"));let sdk::http::CredentialKind::OAuth2{flows,..}=r.kind else{panic!("OAuth metadata")};assert_eq!(flows[0].token_url.unwrap().value,"https://auth.example.test/token");assert!(matches!(r.permissions,sdk::http::Permissions::Scopes(v) if v[0].value=="read:items"));Ok(Some(sdk::http::Credential::Authorization("Custom explicit-token".into())))});
    let (client,seen,_,_)=raw_mock(204,vec![],vec![],credentials);client.security_default().await.unwrap();assert_eq!(calls.load(Ordering::SeqCst),1);assert_eq!(header(&seen.lock().unwrap()[0],"authorization"),b"Custom explicit-token");
    let (client,seen,_,_)=raw_mock(204,vec![],vec![],Credentials::oidc("DPoP caller-owned").select_alternative(3));client.security_default().await.unwrap();assert_eq!(header(&seen.lock().unwrap()[0],"authorization"),b"DPoP caller-owned");
    for credentials in [Credentials::new().select_alternative(1),Credentials::new().select_alternative(99),Credentials::basic("bad:name","x").select_alternative(4),Credentials::new().with_bearer("token","x\r\nsecret").with_api_key("key","x").select_alternative(1)]{let (client,seen,_,_)=raw_mock(204,vec![],vec![],credentials);assert!(client.security_default().await.is_err());assert!(seen.lock().unwrap().is_empty());}
}

#[tokio::test]
async fn native_server_selection_relative_document_base_and_variable_guards(){
    let (client,seen,_)=mock(204,None,vec![]);client.servers_default().await.unwrap();assert_eq!(seen.lock().unwrap()[0].url,"https://demo.example.test/v2/servers");
    let (client,seen,_)=mock(204,None,vec![]);let client=client.with_options(ClientOptions{server_variables:std::collections::BTreeMap::from([("tenant".into(),"customer".into()),("port".into(),"8443".into()),("basePath".into(),"v3".into())]),..Default::default()});client.servers_default().await.unwrap();assert_eq!(seen.lock().unwrap()[0].url,"https://customer.example.test:8443/v3/servers");
    let (client,seen,_)=mock(204,None,vec![]);let client=client.with_options(ClientOptions{server_index:Some(1),document_url:Some("https://docs.example.test/specs/openapi.json".into()),..Default::default()});client.servers_default().await.unwrap();assert_eq!(seen.lock().unwrap()[0].url,"https://docs.example.test/api/servers");
    let (client,seen,_)=mock(204,None,vec![]);client.http_default().await.unwrap();assert_eq!(seen.lock().unwrap()[0].url,"http://api.example.test/base/http");
    for options in [ClientOptions{server_index:Some(1),..Default::default()},ClientOptions{server_index:Some(5),..Default::default()},ClientOptions{server_variables:std::collections::BTreeMap::from([("port".into(),"1234".into())]),..Default::default()},ClientOptions{server_variables:std::collections::BTreeMap::from([("typo".into(),"x".into())]),..Default::default()}]{let (client,seen,_)=mock(204,None,vec![]);assert!(client.with_options(options).servers_default().await.is_err());assert!(seen.lock().unwrap().is_empty());}
}

#[tokio::test]
async fn binary_and_text_requests_are_not_json_conversions_and_media_cannot_bypass_codecs(){
    let bytes=vec![0,255,1,128,13,10];let (client,seen,_)=mock(200,Some("application/octet-stream"),bytes.clone());let result=client.binary(sdk::operations::binary::Binary::new(bytes.clone())).await.unwrap();assert_eq!(result.data,bytes);let records=seen.lock().unwrap();assert_eq!(records[0].body.as_ref().unwrap(),&bytes);assert_eq!(header(&records[0],"content-type"),b"application/octet-stream");drop(records);
    let (client,seen,_)=mock(200,Some("text/plain"),b"false".to_vec());assert!(!client.text(sdk::operations::text::Text::new(true)).await.unwrap().data);assert_eq!(seen.lock().unwrap()[0].body,Some(b"true".to_vec()));
    let (client,seen,_)=mock(204,None,vec![]);let request=sdk::operations::request_choice::RequestChoice::new(sdk::operations::request_choice::RequestChoiceBody::Any(b"0".to_vec())).with_content_type("application/json");assert!(client.request_choice(request).await.is_err());assert!(seen.lock().unwrap().is_empty());
    let (client,seen,_)=mock(204,None,vec![]);let request=sdk::operations::request_choice::RequestChoice::new(sdk::operations::request_choice::RequestChoiceBody::Any(vec![0,255])).with_content_type("image/png");client.request_choice(request).await.unwrap();assert_eq!(seen.lock().unwrap()[0].body,Some(vec![0,255]));
    let (client,seen,_)=mock(200,Some("application/octet-stream"),vec![]);assert!(matches!(client.binary(sdk::operations::binary::Binary::new(vec![1;9])).await.unwrap_err(),sdk::operations::binary::BinaryError::Sdk(e) if e.kind==SdkErrorKind::ResourceLimit));assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn multipart_files_headers_repeated_items_and_structural_validation(){
    use sdk::http::Part;
    let file=Part::with_headers(vec![0,255,1,13,10],FileHeaders::new("part-1".into())).with_filename("x.bin").with_content_type("image/png");
    // Metadata's ordinary JSON codec validates it, while file bytes have none.
    let mut body=UploadBody::new(file,Part::new(Default::default()));
    body.metadata.data.title=Some("snow".into());body.labels=Some(vec![Part::new("a".into()),Part::new("b".into())]);
    let (client,seen,_)=mock(204,None,vec![]);client.upload(sdk::operations::upload::Upload::new(body)).await.unwrap();
    let mut expected=b"--suspect-boundary-0\r\nContent-Disposition: form-data; name=\"file\"; filename=\"x.bin\"\r\nContent-Type: image/png\r\nX-Part-Id: part-1\r\n\r\n".to_vec();expected.extend_from_slice(&[0,255,1,13,10]);expected.extend_from_slice(b"\r\n--suspect-boundary-0\r\nContent-Disposition: form-data; name=\"labels\"\r\nContent-Type: text/plain\r\n\r\na\r\n--suspect-boundary-0\r\nContent-Disposition: form-data; name=\"labels\"\r\nContent-Type: text/plain\r\n\r\nb\r\n--suspect-boundary-0\r\nContent-Disposition: form-data; name=\"metadata\"\r\nContent-Type: application/json\r\n\r\n{\"title\":\"snow\"}\r\n--suspect-boundary-0--\r\n");
    assert_eq!(seen.lock().unwrap()[0].body.as_ref().unwrap(),&expected);
    let (client,_,_)=mock(200,Some("multipart/form-data; boundary=suspect-boundary-0"),expected);let response=client.download_parts_default().await.unwrap().into_response();assert_eq!(response.data.file.data,[0,255,1,13,10]);assert_eq!(response.data.file.filename.as_deref(),Some("x.bin"));assert_eq!(response.data.file.headers.x_part_id,"part-1");assert_eq!(response.data.metadata.data.title.as_deref(),Some("snow"));assert_eq!(response.data.labels.unwrap()[1].data,"b");
    let body=FilesBody::new(vec![Part::new(vec![0,255]),Part::new(vec![1,2,3])]);let (client,seen,_)=mock(204,None,vec![]);client.files(sdk::operations::files::Files::new(body)).await.unwrap();let bytes=seen.lock().unwrap()[0].body.clone().unwrap();assert_eq!(bytes.windows(b"name=\"files\"".len()).filter(|w|*w==b"name=\"files\"").count(),2);assert!(bytes.windows(3).any(|w|w==[1,2,3]));
    for files in [vec![],vec![Part::new(vec![0;4])],vec![Part::new(vec![1]);3]]{let (client,seen,_)=mock(204,None,vec![]);assert!(client.files(sdk::operations::files::Files::new(FilesBody::new(files))).await.is_err());assert!(seen.lock().unwrap().is_empty());}
}

#[tokio::test]
async fn form_and_positional_parts_have_per_item_codecs_and_no_fake_aggregate(){
    let body=FormBody::new("a + b".into()).with_codes(vec!["1".parse().unwrap(),"2".parse().unwrap()]).with_tags(vec!["x y".into(),"z".into()]);
    let expected=b"codes=1&codes=2&id=a+%2B+b&tags=x+y&tags=z".to_vec();
    let (client,seen,_)=mock(200,Some("application/x-www-form-urlencoded"),expected.clone());let response=client.form(sdk::operations::form::Form::new(body)).await.unwrap().into_response();assert_eq!(seen.lock().unwrap()[0].body,Some(expected));assert_eq!(response.data.id,"a + b");assert_eq!(response.data.tags,Some(vec!["x y".into(),"z".into()]));assert_eq!(response.data.codes.unwrap().len(),2);
    let body=PartsBody::new(sdk::http::Part::new("hello".into()));
    let response=b"ignored preamble\r\n--unit\r\nContent-Type: text/plain\r\n\r\nhello\r\n--unit\r\nContent-Type: application/json\r\n\r\n{\"n\":5}\r\n--unit--\r\nignored epilogue".to_vec();
    let (client,seen,_)=mock(200,Some("multipart/mixed; boundary=unit"),response);let response=client.parts(sdk::operations::parts::Parts::new(body)).await.unwrap().into_response();assert_eq!(response.data.part_0.data,"hello");assert_eq!(response.data.items[0].data.n.as_ref().unwrap().as_str(),"5");assert!(seen.lock().unwrap()[0].body.as_ref().unwrap().windows(5).any(|w|w==b"hello"));
}

#[tokio::test]
async fn named_multipart_rfc6570_item_encodings_decode_fixed_delimiters_once(){
    use sdk::http::Part;
    let body=StyledBody::new(vec![Part::new(vec!["a".into(),"b".into()])],Part::new(StyledTree::new("x".into())));
    let incoming=b"--style\r\nContent-Disposition: form-data; name=\"filters\"\r\n\r\nfilters=a%7Cb\r\n--style\r\nContent-Disposition: form-data; name=\"tree\"\r\n\r\ntree%5Ba%5D=x\r\n--style--\r\n".to_vec();
    let (client,seen,_)=mock(200,Some("multipart/form-data; boundary=style"),incoming);let response=client.styled(sdk::operations::styled::Styled::new(body)).await.unwrap().into_response();
    assert_eq!(response.data.filters[0].data,vec!["a".to_owned(),"b".to_owned()]);assert_eq!(response.data.tree.data.a,"x");
    let records=seen.lock().unwrap();let outgoing=records[0].body.as_ref().unwrap();assert!(outgoing.windows(13).any(|w|w==b"filters=a%7Cb"));assert!(outgoing.windows(13).any(|w|w==b"tree%5Ba%5D=x"));drop(records);
    let body=StyledBody::new(vec![Part::new(vec!["a%7Cb".into()])],Part::new(StyledTree::new("x".into())));let (client,seen,_)=mock(200,Some("multipart/form-data; boundary=x"),vec![]);assert!(client.styled(sdk::operations::styled::Styled::new(body)).await.is_err());assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn standard_sse_envelopes_and_json_lines_keep_backpressure_bytes_and_drop_cleanup(){
    let wire="\u{feff}: comment\r\nid: 7\r\nevent: update\r\nretry: 0042\r\ndata: snow 雪\r\ndata: next\r\nunknown: ignored\r\n\r\ndata: [DONE]\n\n".as_bytes().to_vec();
    let chunks=wire.into_iter().map(|b|Chunk::Bytes(vec![b])).chain([Chunk::Pending]).collect();
    let (client,_,drops,polls)=raw_mock(200,vec![("content-type".into(),b"text/event-stream".to_vec())],chunks,Credentials::new());let mut response=client.events_default().await.unwrap().into_response();assert_eq!(polls.load(Ordering::SeqCst),0);
    let first=response.data.next().await.unwrap().unwrap();assert_eq!(first.data,"snow 雪\nnext");assert_eq!(first.id.as_deref(),Some("7"));assert_eq!(first.event.as_deref(),Some("update"));assert_eq!(first.retry.unwrap().as_str(),"42");
    let next=response.data.next().await.unwrap().unwrap();assert_eq!(next.data,"[DONE]");assert!(next.retry.is_none());
    let waker=Waker::from(Arc::new(Noop));let mut future=Box::pin(response.data.next());assert!(matches!(future.as_mut().poll(&mut Context::from_waker(&waker)),Poll::Pending));drop(future);drop(response);assert_eq!(drops.load(Ordering::SeqCst),1);
    let (client,_,drops,_)=raw_mock(200,vec![("content-type".into(),b"application/x-ndjson".to_vec())],vec![Chunk::Bytes(b"{\"n\":1}\r\n{\"n\":9007199254740993}".to_vec())],Credentials::new());let mut lines=client.lines_default().await.unwrap().into_response().data;assert_eq!(lines.next().await.unwrap().unwrap().n.as_str(),"1");assert_eq!(lines.next().await.unwrap().unwrap().n.as_str(),"9007199254740993");assert!(lines.next().await.is_none());assert_eq!(drops.load(Ordering::SeqCst),1);
    let (client,_,drops,_)=raw_mock(200,vec![("content-type".into(),b"application/x-ndjson".to_vec())],vec![Chunk::Bytes(b"{\"n\":0}\n".to_vec()),Chunk::Pending],Credentials::new());let mut lines=client.lines_default().await.unwrap().into_response().data;let error=lines.next().await.unwrap().unwrap_err();assert_eq!(error.kind,SdkErrorKind::ResponseDecoding);assert!(error.source.pointer.ends_with("/itemSchema"));assert!(lines.is_closed());assert_eq!(drops.load(Ordering::SeqCst),1);
    let (client,_,drops,_)=raw_mock(200,vec![("content-type".into(),b"text/event-stream".to_vec())],vec![Chunk::Bytes(b"data: too long\n\n".to_vec())],Credentials::new());let client=client.with_options(ClientOptions{max_stream_item_bytes:Some(8),max_error_capture_bytes:3,..Default::default()});let mut events=client.events_default().await.unwrap().into_response().data;let error=events.next().await.unwrap().unwrap_err();assert_eq!(error.kind,SdkErrorKind::ResourceLimit);assert_eq!(error.raw_capture,b"dat");assert_eq!(drops.load(Ordering::SeqCst),1);
    let (client,_,drops,_)=raw_mock(200,vec![("content-type".into(),b"text/event-stream".to_vec())],vec![Chunk::Fail],Credentials::new());let mut events=client.events_default().await.unwrap().into_response().data;let error=events.next().await.unwrap().unwrap_err();assert_eq!(error.kind,SdkErrorKind::Transport);assert!(!format!("{error:?}").contains("private"));assert_eq!(drops.load(Ordering::SeqCst),1);
    for options in [ClientOptions{max_chunk_bytes:Some(3),..Default::default()},ClientOptions{max_response_bytes:Some(8),..Default::default()}]{let (client,_,drops,_)=raw_mock(200,vec![("content-type".into(),b"application/x-ndjson".to_vec())],vec![Chunk::Bytes(b"{\"n\":1}\n".to_vec()),Chunk::Bytes(b"{\"n\":2}\n".to_vec())],Credentials::new());let mut lines=client.with_options(options).lines_default().await.unwrap().into_response().data;let first=lines.next().await.unwrap();let error=match first{Err(error)=>error,Ok(_)=>lines.next().await.unwrap().unwrap_err()};assert_eq!(error.kind,SdkErrorKind::ResourceLimit);assert!(lines.is_closed());assert_eq!(drops.load(Ordering::SeqCst),1);}
}

#[tokio::test]
async fn finite_stream_request_uses_item_schema_and_standard_envelope_framing(){
    let (client,seen,_)=mock(204,None,vec![]);let event=sdk::models::Event::new("first\nsecond".into());client.send_events(sdk::operations::send_events::SendEvents::new(vec![event])).await.unwrap();assert_eq!(seen.lock().unwrap()[0].body,Some(b"data: first\ndata: second\n\n".to_vec()));
    let (client,seen,_)=mock(204,None,vec![]);client.send_lines(sdk::operations::send_lines::SendLines::new(vec![sdk::models::Record::new("9007199254740993".parse::<JsonInteger>().unwrap())])).await.unwrap();assert_eq!(seen.lock().unwrap()[0].body,Some(b"{\"n\":9007199254740993}\n".to_vec()));
}

#[tokio::test]
async fn custom_and_fixed_method_tokens_survive_native_requests(){
    let (client,seen,_)=mock(200,Some("application/json"),b"{}".to_vec());
    client.fixed_get_default().await.unwrap();client.fixed_query_default().await.unwrap();
    client.copy_default().await.unwrap();client.mixed_get_default().await.unwrap();
    client.lower_get_default().await.unwrap();client.lower_head_default().await.unwrap();client.extension_named_method_default().await.unwrap();
    assert_eq!(seen.lock().unwrap().iter().map(|r|r.method).collect::<Vec<_>>(),["GET","QUERY","COPY","GeT","get","head","x-PING"]);
}

mod socket {
    use std::{io::{Read,Write},net::{TcpListener,TcpStream},sync::mpsc::{Receiver,channel},time::Duration};
    pub struct Record {pub method:String,pub target:String,pub headers:Vec<(String,String)>,pub body:Vec<u8>}
    impl Record {pub fn header(&self,name:&str)->&str{self.headers.iter().find(|(key,_)|key.eq_ignore_ascii_case(name)).map(|(_,value)|value.as_str()).unwrap_or("")}}
    fn read(mut stream:&TcpStream)->Record {
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();stream.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut bytes=Vec::new();let mut buf=[0u8;1024];
        let end=loop{let count=stream.read(&mut buf).unwrap();assert!(count>0);bytes.extend_from_slice(&buf[..count]);if let Some(end)=bytes.windows(4).position(|w|w==b"\r\n\r\n"){break end+4;}assert!(bytes.len()<65536);};
        let text=std::str::from_utf8(&bytes[..end]).unwrap();let mut lines=text.lines();let mut request=lines.next().unwrap().split(' ');let method=request.next().unwrap().to_owned();let target=request.next().unwrap().to_owned();
        let headers=lines.filter_map(|line|line.split_once(':')).map(|(name,value)|(name.into(),value.trim().into())).collect::<Vec<(String,String)>>();
        let length=headers.iter().find(|(name,_)|name.eq_ignore_ascii_case("content-length")).map(|(_,v)|v.parse::<usize>().unwrap()).unwrap_or(0);assert!(length<1024*1024);
        while bytes.len()-end<length{let count=stream.read(&mut buf).unwrap();assert!(count>0);bytes.extend_from_slice(&buf[..count]);}
        Record{method,target,headers,body:bytes[end..end+length].to_vec()}
    }
    pub fn one(media:&str,body:Vec<u8>)->(String,Receiver<Record>,std::thread::JoinHandle<()>) {
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();let base=format!("http://{}/v1",listener.local_addr().unwrap());let (tx,rx)=channel();let media=media.to_owned();
        let thread=std::thread::spawn(move||{let (mut stream,_)=listener.accept().unwrap();let record=read(&stream);write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: {media}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).unwrap();stream.write_all(&body).unwrap();stream.flush().unwrap();tx.send(record).unwrap();});
        (base,rx,thread)
    }
    pub fn unfinished_events()->(String,Receiver<bool>,std::thread::JoinHandle<()>) {
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();let base=format!("http://{}/v1",listener.local_addr().unwrap());let (tx,rx)=channel();
        let thread=std::thread::spawn(move||{let (mut stream,_)=listener.accept().unwrap();let record=read(&stream);assert_eq!(record.target,"/v1/events");stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\nd\r\ndata: hello\n\n\r\n").unwrap();stream.flush().unwrap();let mut byte=[0u8;1];let closed=matches!(stream.read(&mut byte),Ok(0)) ;tx.send(closed).unwrap();});
        (base,rx,thread)
    }
}

#[tokio::test]
async fn installed_reqwest_socket_wire_preserves_case_native_bytes_and_stream_cancel(){
    use std::time::Duration;
    for method in ["get","GeT","head","x-PING","QUERY"] {
        let (base,record,thread)=socket::one("application/json",b"{}".to_vec());let client=Client::with_reqwest(Credentials::new()).unwrap().with_options(ClientOptions{server_url:Some(base),..Default::default()});
        match method {"get"=>{client.lower_get_default().await.unwrap();},"GeT"=>{client.mixed_get_default().await.unwrap();},"head"=>{client.lower_head_default().await.unwrap();},"x-PING"=>{client.extension_named_method_default().await.unwrap();},_=>{client.fixed_query_default().await.unwrap();}}
        let record=record.recv_timeout(Duration::from_secs(5)).unwrap();assert_eq!(record.method,method);assert_eq!(record.target,"/v1/methods");assert!(record.body.is_empty());assert_eq!(record.header("authorization"),"");thread.join().unwrap();
    }
    let bytes=vec![0,255,128,13,10];let (base,record,thread)=socket::one("application/octet-stream",bytes.clone());let client=Client::with_reqwest(Credentials::new()).unwrap().with_options(ClientOptions{server_url:Some(base),..Default::default()});assert_eq!(client.binary(sdk::operations::binary::Binary::new(bytes.clone())).await.unwrap().data,bytes);let record=record.recv_timeout(Duration::from_secs(5)).unwrap();assert_eq!(record.method,"PUT");assert_eq!(record.body,bytes);assert_eq!(record.header("content-type"),"application/octet-stream");thread.join().unwrap();
    let (base,closed,thread)=socket::unfinished_events();let client=Client::with_reqwest(Credentials::new()).unwrap().with_options(ClientOptions{server_url:Some(base),..Default::default()});let mut stream=client.events_default().await.unwrap().into_response().data;assert_eq!(stream.next().await.unwrap().unwrap().data,"hello");stream.close();assert!(tokio::task::spawn_blocking(move||closed.recv_timeout(Duration::from_secs(5)).unwrap()).await.unwrap());thread.join().unwrap();
}

"##;

const NEGATIVE_TYPING: &str = r##"
/// Required body bytes are native bytes, not String or JSON null.
/// ```compile_fail
/// sdk::operations::binary::Binary::new(String::new());
/// ```
/// Required typed multipart headers cannot be omitted.
/// ```compile_fail
/// use sdk::http::Part;
/// let _: sdk::http::Part<Vec<u8>, sdk::operations::upload::UploadBodyMultipartFormDataFileHeaders> = Part::new(vec![0u8]);
/// ```
/// Exact-number inputs cannot silently accept a floating point cast.
/// ```compile_fail
/// sdk::models::Record::new(9007199254740993.0f64);
/// ```
/// A stream item is a native model, not unchecked JSON.
/// ```compile_fail
/// fn bad(value:sdk::http::ItemStream<sdk::models::Event>)->sdk::JsonValue { value }
/// ```
pub struct NativeTyping;
#[test]fn negative_type_docs_are_part_of_the_native_consumer(){let _=NativeTyping;let _:sdk::http::Part<Vec<u8>,sdk::operations::upload::UploadBodyMultipartFormDataFileHeaders>=sdk::http::Part::with_headers(vec![0],sdk::operations::upload::UploadBodyMultipartFormDataFileHeaders::new("part".into()));}
"##;
