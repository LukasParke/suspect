//! PHP through public backend/session/comparison APIs. Native SDK runtime
//! acceptance is retained separately; these checks exercise the new adapter.
#![cfg(feature = "php-sdk")]

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde_json::{Value, json};
use suspect_codegen::{
    backend::{self, Backend, TargetConfig},
    compatibility::{
        self, CompatibilityReport, NativeModel, NativeOperation, NativeSnapshot, PlanStatus,
    },
    generation_session::{Session, SessionConfig},
    php_sdk::{self, PhpConfig},
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

struct Fixture {
    _directory: tempfile::TempDir,
    path: PathBuf,
}
impl Fixture {
    fn new(value: &Value) -> Self {
        let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-php-integration");
        fs::create_dir_all(&parent).unwrap();
        let directory = tempfile::Builder::new()
            .prefix("case-")
            .tempdir_in(parent)
            .unwrap();
        let path = directory.path().join("api.json");
        let fixture = Self {
            _directory: directory,
            path,
        };
        fixture.write(value);
        fixture
    }
    fn write(&self, value: &Value) {
        fs::write(&self.path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    }
    fn load(&self) -> Arc<Contract> {
        let workspace = Arc::new(
            WorkspaceBuilder::new()
                .root(self.path.parent().unwrap())
                .build()
                .unwrap(),
        );
        Arc::new(
            Contract::from_workspace(&workspace, &Uri::from_path(&self.path).unwrap()).unwrap(),
        )
    }
}
fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::PhpHttp,
        package_name: "acme/widgets-sdk".into(),
        package_version: "1.0.0".into(),
        import_name: None,
    }
}
fn api() -> Value {
    json!({"openapi":"3.1.0","info":{"title":"PHP integration","version":"1"},
    "servers":[{"url":"https://api.example.test/v1"}],"security":[{"apiKey":[]}],
    "components":{"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}},"schemas":{
        "Tag":{"type":"string","const":"fixed"},"TagAlias":{"$ref":"#/components/schemas/Tag"},
        "TextList":{"type":"array","items":{"type":"string"}},
        "NumberList":{"type":"array","items":{"type":"integer"}},
        "ListAlias":{"$ref":"#/components/schemas/TextList"},
        "Payload":{"type":"object","required":["kind","name","required_nullable"],"properties":{
            "kind":{"$ref":"#/components/schemas/TagAlias"},"name":{"type":"string","minLength":1,"default":"not a constructor default"},
            "required_nullable":{"type":["string","null"]},"note":{"type":["string","null"]},
            "label":{"type":"string","default":"not inserted"},"child":{"$ref":"#/components/schemas/Payload"},
            "choice":{"$ref":"#/components/schemas/Choice"},"items":{"type":"array","items":{"type":"string"}},
            "list_alias":{"$ref":"#/components/schemas/ListAlias"},"other_list":{"$ref":"#/components/schemas/NumberList"}
        },"additionalProperties":{"type":"string"}},
        "Primary":{"type":"object","required":["kind","text"],"properties":{"kind":{"type":"string","const":"primary"},"text":{"type":"string"}}},
        "Secondary":{"type":"object","required":["kind","code"],"properties":{"kind":{"type":"string","const":"secondary"},"code":{"type":"integer"}}},
        "Choice":{"oneOf":[{"$ref":"#/components/schemas/Primary"},{"$ref":"#/components/schemas/Secondary"}]},
        "Failure":{"type":"object","required":["message"],"properties":{"message":{"type":"string"}}}
    }},
    "paths":{
        "/health":{"get":{"operationId":"getHealth","responses":{"200":{"description":"Healthy","content":{"application/json":{"schema":{"type":"string"}}}}}}},
        "/widgets":{"get":{"operationId":"listWidgets","parameters":[{"name":"options","in":"query","schema":{"type":"string"}}],
            "responses":{"200":{"description":"Page","content":{"application/json":{"schema":{"type":"array","items":{"$ref":"#/components/schemas/Payload"}}}}}}}},
        "/widgets/{id}":{"patch":{"operationId":"updateWidget","parameters":[{"name":"query-term","in":"query","schema":{"type":"string"}},{"name":"id","in":"path","required":true,"schema":{"type":"string"}}],
            "requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Payload"}}}},
            "responses":{"200":{"description":"Updated","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Payload"}}}},
                "422":{"description":"Rejected","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Failure"}}}}}}
        }
    }})
}
fn capture(fixture: &Fixture, target: &TargetConfig) -> NativeSnapshot {
    let snapshot = compatibility::snapshot(fixture.load(), &[], std::slice::from_ref(target))
        .unwrap()
        .native
        .remove(0);
    assert_eq!(
        snapshot.status,
        PlanStatus::Planned,
        "{:?}",
        snapshot.findings
    );
    assert!(snapshot.findings.is_empty(), "{:?}", snapshot.findings);
    snapshot
}
fn model<'a>(snapshot: &'a NativeSnapshot, pointer: &str, role: &str) -> &'a NativeModel {
    snapshot
        .models
        .iter()
        .find(|m| m.source.pointer == pointer && m.role == role)
        .unwrap_or_else(|| panic!("missing {pointer} / {role}"))
}
fn field<'a>(model: &'a NativeModel, name: &str) -> &'a Value {
    model.descriptor.as_ref().unwrap()["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap()
}
fn operation<'a>(snapshot: &'a NativeSnapshot, name: &str) -> &'a NativeOperation {
    snapshot
        .operations
        .iter()
        .find(|op| op.operation_id == name)
        .unwrap()
}
fn compare(before: &Value, after: &Value) -> CompatibilityReport {
    let fixture = Fixture::new(before);
    let old = fixture.load();
    fixture.write(after);
    compatibility::compare(old, fixture.load(), &[], &[target()]).unwrap()
}
fn method_parameters<'a>(operation: &'a NativeOperation, which: &str) -> &'a [Value] {
    operation.descriptor["parameters"][which]["parameters"]
        .as_array()
        .unwrap()
}
fn names(values: &Value) -> Vec<&str> {
    values
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect()
}

#[test]
fn public_backend_and_snapshot_use_the_same_composer_namespace_and_symbols() {
    assert!(Backend::ALL.contains(&Backend::PhpHttp));
    assert_eq!(Backend::PhpHttp.name(), "php-http");
    assert_eq!(Backend::PhpHttp.artifact_directory(), "php");
    let fixture = Fixture::new(&api());
    let contract = fixture.load();
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let files = backend::generate(contract.clone(), &selected, &target()).unwrap();
    assert!(files.iter().all(|file| file.path.starts_with("php/")));
    let manifest: Value = serde_json::from_str(
        &files
            .iter()
            .find(|f| f.path == "php/composer.json")
            .unwrap()
            .content,
    )
    .unwrap();
    assert_eq!(manifest["name"], "acme/widgets-sdk");
    assert_eq!(manifest["version"], "1.0.0");
    let coverage: Value = serde_json::from_str(
        &files
            .iter()
            .find(|f| f.path == "php/docs/coverage.json")
            .unwrap()
            .content,
    )
    .unwrap();
    assert_eq!(coverage["namespace"], "Acme\\WidgetsSdk");
    let attribution = suspect_codegen::attribution::AttributionDescriptor::plan(
        env!("CARGO_PKG_VERSION"),
        "acme/widgets-sdk",
        "1.0.0",
        contract.openapi_version(),
        "php",
    );
    let plan = php_sdk::plan_sdk(
        contract,
        &selected,
        PhpConfig {
            package_name: target().package_name,
            package_version: "1.0.0".into(),
            namespace: "Acme\\WidgetsSdk".into(),
            // Canonical backend generation compiles the ua/v1 attribution
            // descriptor from the same package identity and source.
            attribution: Some(attribution),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        files,
        plan.render(),
        "public dispatch must use the real PHP package policy"
    );
    let snapshot = capture(&fixture, &target());
    for op in &snapshot.operations {
        assert_eq!(op.symbols["client"], "Acme\\WidgetsSdk\\Client");
        assert_eq!(op.symbols["interface"], "Acme\\WidgetsSdk\\ClientInterface");
        let expected = plan
            .operations()
            .iter()
            .find(|p| p.source.pointer() == op.source.pointer)
            .unwrap();
        assert_eq!(op.symbols["method"], expected.method);
        assert_eq!(
            op.symbols["input"],
            format!("Acme\\WidgetsSdk\\{}", expected.input)
        );
    }
    for node in plan.models().nodes.values() {
        for (role, method) in [
            ("codec-decode", &node.codecs.decode),
            ("codec-encode", &node.codecs.encode),
            ("codec-from-value", &node.codecs.from_value),
            ("codec-to-value", &node.codecs.to_value),
        ] {
            let record = model(&snapshot, node.source.pointer(), role);
            assert_eq!(record.name, format!("Acme\\WidgetsSdk\\Codecs::{method}"));
            assert_eq!(record.descriptor.as_ref().unwrap()["member"], *method);
        }
    }
    assert!(
        snapshot
            .models
            .iter()
            .all(|m| m.descriptor.is_some() && m.source.span.is_some())
    );
    assert!(
        !snapshot
            .models
            .iter()
            .any(|m| m.role == "model" && m.name.ends_with("\\TagAlias"))
    );
    assert_eq!(
        model(&snapshot, "/components/schemas/TagAlias", "codec-decode")
            .descriptor
            .as_ref()
            .unwrap()["returns"]["shape"],
        json!({"kind":"named","name":"Acme\\WidgetsSdk\\Tag"})
    );
}

#[test]
fn constructor_order_literals_presence_and_phpdoc_are_actual_native_obligations() {
    let fixture = Fixture::new(&api());
    let snapshot = capture(&fixture, &target());
    let payload = model(&snapshot, "/components/schemas/Payload", "model");
    let descriptor = payload.descriptor.as_ref().unwrap();
    assert_eq!(
        names(&descriptor["constructor"]["parameters"]),
        [
            "name",
            "requiredNullable",
            "child",
            "choice",
            "items",
            "label",
            "listAlias",
            "note",
            "otherList",
            "extra"
        ]
    );
    assert_eq!(
        field(payload, "kind")["initialization"],
        json!({"kind":"literal","type":"Acme\\WidgetsSdk\\Tag","member":"Fixed","value":"fixed"})
    );
    assert_eq!(field(payload, "kind")["readonly"], true);
    assert_eq!(field(payload, "kind")["constructorParameter"], false);
    assert_eq!(field(payload, "requiredNullable")["required"], true);
    assert_eq!(
        field(payload, "requiredNullable")["type"]["phpNullAllowed"],
        true
    );
    assert_eq!(
        descriptor["constructor"]["parameters"][1]["hasDefault"],
        false
    );
    assert_eq!(
        field(payload, "note")["type"]["native"],
        "Absent|null|string"
    );
    assert_eq!(field(payload, "note")["initialization"]["kind"], "absent");
    assert_eq!(field(payload, "label")["type"]["native"], "Absent|string");
    assert_eq!(
        field(payload, "label")["initialization"]["kind"],
        "absent",
        "schema defaults are not PHP parameter defaults"
    );
    assert_eq!(field(payload, "items")["type"]["native"], "Absent|array");
    assert_eq!(
        field(payload, "items")["type"]["phpdoc"],
        "Absent|list<string>"
    );
    assert_eq!(
        field(payload, "extra")["type"]["phpdoc"],
        "array<array-key, string>"
    );
    assert_eq!(
        descriptor["constructor"]["parameters"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["initialization"],
        json!({"kind":"literal","value":[]})
    );
    assert_eq!(field(payload, "name")["mutable"], true);
    let update = operation(&snapshot, "updateWidget");
    assert_eq!(
        names(&update.descriptor["constructor"]["input"]["parameters"]),
        ["id", "body", "queryTerm"]
    );
    assert_eq!(
        update.descriptor["constructor"]["input"]["parameters"][2]["initialization"]["kind"],
        "absent"
    );
}

#[test]
fn interface_no_input_concrete_results_and_error_body_members_are_preserved() {
    let fixture = Fixture::new(&api());
    let snapshot = capture(&fixture, &target());
    let health = operation(&snapshot, "getHealth");
    let list = operation(&snapshot, "listWidgets");
    let update = operation(&snapshot, "updateWidget");
    assert_eq!(
        health.descriptor["constructor"]["input"]["parameters"],
        json!([])
    );
    assert_eq!(
        list.descriptor["parameters"]["members"][0]["member"],
        "options2"
    );
    for op in [health, list] {
        assert_eq!(
            op.descriptor["parameters"]["method"]["canCallWithoutInput"],
            true
        );
        assert_eq!(
            method_parameters(op, "method")[0]["initialization"]["kind"],
            "constructor-call"
        );
        assert_eq!(
            method_parameters(op, "interfaceMethod"),
            method_parameters(op, "method")
        );
    }
    assert_eq!(
        update.descriptor["parameters"]["method"]["canCallWithoutInput"],
        false
    );
    assert_eq!(method_parameters(update, "method")[1]["name"], "options");
    assert_eq!(
        method_parameters(update, "method")[1]["initialization"],
        json!({"kind":"literal","value":null})
    );
    assert_eq!(
        update.symbols["success"],
        "Acme\\WidgetsSdk\\UpdateWidgetStatus200"
    );
    assert_eq!(
        update.descriptor["responseUnions"]["success"]["kind"],
        "concrete-result"
    );
    assert_eq!(
        update.descriptor["responseUnions"]["apiError"]["type"],
        "Acme\\WidgetsSdk\\UpdateWidgetApiError"
    );
    assert_eq!(update.descriptor["credential"]["sourceSchemeKey"], "apiKey");
    for response in update.descriptor["responses"].as_array().unwrap() {
        assert_eq!(response["bodyMember"], "body");
        assert_eq!(response["metadataMember"], "response");
        assert_eq!(
            names(&response["fields"]),
            ["body", "response", "headers", "links"]
        );
        assert_eq!(
            names(&response["constructor"]["parameters"]),
            ["body", "response", "headers", "links"]
        );
        assert_eq!(response["fields"][2]["readonly"], true);
        assert_eq!(response["fields"][3]["type"]["phpdoc"], "list<Link>");
        assert_eq!(
            response["constructor"]["parameters"][3]["initialization"],
            json!({"kind":"literal","value":[]})
        );
    }
    let error = &update.descriptor["responses"][1];
    assert_eq!(
        error["type"],
        "Acme\\WidgetsSdk\\UpdateWidgetStatus422Error"
    );
    assert_eq!(error["base"], "Acme\\WidgetsSdk\\UpdateWidgetApiError");
    assert_eq!(error["fields"][1]["inherited"], true);
    assert_eq!(
        error["fields"][1]["type"]["native"],
        "HttpResponse|StreamResponse"
    );
    assert_eq!(error["fields"][1]["type"]["phpdoc"], "HttpResponse");
    assert_eq!(error["fields"][0]["type"]["native"], "Failure");
}

#[test]
#[cfg(feature = "http-protocol")]
fn rich_capture_preserves_iterable_direction_parts_headers_and_security_metadata() {
    let mut value = api();
    value["openapi"] = json!("3.2.0");
    value["components"]["schemas"]["Item"] = json!({"type":"string"});
    value["paths"] = json!({
        "/pipe":{"post":{"operationId":"pipe","security":[{"apiKey":["admin"]}],"requestBody":{"required":true,"content":{"application/x-ndjson":{"itemSchema":{"$ref":"#/components/schemas/Item"}},"text/plain":{"schema":{"type":"string"}}}},"responses":{"200":{"description":"Stream","content":{"application/x-ndjson":{"itemSchema":{"$ref":"#/components/schemas/Item"}}}}}}},
        "/upload": {"post": {
            "operationId":"upload", "security":[],
            "requestBody":{"required":true,"content":{"multipart/form-data":{
                "schema":{"type":"object","required":["file"],"properties":{"file":{},"note":{"type":"string"}},"additionalProperties":false},
                "encoding":{"file":{"contentType":"application/octet-stream","headers":{"X-Part":{"required":true,"schema":{"type":"integer"}}}}}
            }}},
            "responses":{"204":{"description":"Stored"}}
        }}
    });
    let fixture = Fixture::new(&value);
    let snapshot = capture(&fixture, &target());
    let pipe = operation(&snapshot, "pipe");
    let media = pipe.descriptor["body"]["media"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["contentType"] == "application/x-ndjson")
        .unwrap();
    assert_eq!(media["valueType"]["native"], "iterable");
    assert_eq!(media["valueType"]["phpdoc"], "iterable<string>");
    assert_eq!(media["valueType"]["shape"]["name"], "iterable");
    let selector = snapshot
        .models
        .iter()
        .find(|m| m.name == media["wrapper"].as_str().unwrap())
        .unwrap()
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(
        selector["constructor"]["parameters"][0]["type"],
        media["valueType"]
    );
    assert_eq!(selector["fields"][0]["readonly"], true);
    assert_eq!(selector["methods"][0]["parameters"][1]["name"], "maxBytes");
    assert_eq!(
        pipe.descriptor["responses"][0]["fields"][0]["type"]["phpdoc"],
        "ItemStream<string>"
    );
    assert_eq!(
        pipe.descriptor["responses"][0]["fields"][1]["type"]["native"],
        "StreamResponse"
    );
    assert_eq!(
        pipe.descriptor["responses"][0]["codecs"]["kind"],
        "per-item"
    );
    assert_eq!(pipe.descriptor["credential"]["sourceSchemeKey"], "apiKey");
    assert_eq!(
        pipe.descriptor["credential"]["stringMeaning"],
        "bearer-token"
    );
    assert_eq!(
        pipe.descriptor["credential"]["security"]["alternatives"][0][0]["permissionKind"],
        "roles"
    );
    assert_eq!(
        pipe.descriptor["credential"]["security"]["alternatives"][0][0]["permissions"],
        json!(["admin"])
    );
    let body = model(
        &snapshot,
        "/paths/~1upload/post/requestBody/content/multipart~1form-data/schema",
        "protocol-body",
    );
    let descriptor = body.descriptor.as_ref().unwrap();
    assert_eq!(
        names(&descriptor["constructor"]["parameters"]),
        ["file", "note"]
    );
    let file = snapshot
        .models
        .iter()
        .find(|m| m.role == "protocol-part" && m.source.pointer.ends_with("/properties/file"))
        .unwrap()
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(
        names(&file["constructor"]["parameters"]),
        [
            "value",
            "headers",
            "filename",
            "contentType",
            "extraHeaders"
        ]
    );
    assert_eq!(
        file["constructor"]["parameters"][0]["type"]["native"],
        "Bytes"
    );
    assert_eq!(file["fields"][0]["mutable"], true);
    assert_eq!(file["codecs"]["kind"], "bytes");
    let headers = snapshot
        .models
        .iter()
        .find(|m| m.name.ends_with("FileHeaders") && m.role == "protocol-headers")
        .unwrap();
    assert_eq!(field(headers, "xPart")["type"]["native"], "JsonNumber");
    assert_eq!(field(headers, "xPart")["required"], true);
    let mut changed = value.clone();
    changed["components"]["schemas"]["Item"] = json!({"type":"integer"});
    let report = compare(&value, &changed);
    assert!(!report.is_proven_compatible());
    assert!(
        !report.native[0].changes.is_empty(),
        "per-item PHPDoc and codecs are caller obligations"
    );
}

#[test]
#[cfg(feature = "http-protocol")]
fn portable_v2_and_v3_admission_preserve_checked_profiles() {
    let mut value = api();
    value["components"]["schemas"]["Payload"]["dependentRequired"] = json!({"label":["note"]});
    let fixture = Fixture::new(&value);
    let contract = fixture.load();
    let program = suspect_schema::OwnedCompiler::new(PhpConfig::default().validation)
        .compile_v2(contract.clone(), contract.schema_roots())
        .unwrap()
        .program();
    assert_eq!(program.check(), Ok(()), "shared v2 program itself is valid");
    assert!(php_sdk::emit_validation(&program, &PhpConfig::default()).is_ok());
    let snapshot = capture(&fixture, &target());
    let codec = model(&snapshot, "/components/schemas/Payload", "codec-encode");
    assert_eq!(
        codec.descriptor.as_ref().unwrap()["validationVersion"],
        "suspect.validation.experimental.v2"
    );
    value["components"]["schemas"]["Payload"]["$dynamicRef"] =
        json!("#/components/schemas/Payload");
    fixture.write(&value);
    let contract = fixture.load();
    let snapshot = compatibility::snapshot(contract, &[], &[target()])
        .unwrap()
        .native
        .remove(0);
    assert_eq!(
        snapshot.status,
        PlanStatus::Planned,
        "{:?}",
        snapshot.findings
    );
    assert!(snapshot.findings.is_empty());
    let codec = model(&snapshot, "/components/schemas/Payload", "codec-encode");
    assert_eq!(
        codec.descriptor.as_ref().unwrap()["validationVersion"],
        "suspect.validation.experimental.v3"
    );
    assert!(
        php_sdk::protocol::capabilities()
            .supports(suspect_codegen::http_protocol::Capability::SchemaResources)
    );
    assert!(
        php_sdk::protocol::capabilities()
            .supports(suspect_codegen::http_protocol::Capability::DynamicSchemaReferences)
    );
}

#[test]
#[cfg(feature = "http-protocol")]
fn scoped_capture_retains_fields_pattern_maps_and_checked_json_carriers() {
    let mut value = api();
    value["components"]["schemas"]["Payload"]["patternProperties"] =
        json!({"^x_":{"type":"integer"}});
    value["components"]["schemas"]["Payload"]["additionalProperties"] = json!(false);
    value["components"]["schemas"]["Conditional"] = json!({"if":{"type":"object"},"then":{"properties":{"a":true}},"else":{"type":"string"},"unevaluatedProperties":false});
    let schema = json!({"$ref":"#/components/schemas/Conditional"});
    value["paths"]["/conditional"] = json!({"post":{"operationId":"conditional","requestBody":{"required":true,"content":{"application/json":{"schema":schema}}},"responses":{"200":{"description":"Checked","content":{"application/json":{"schema":schema}}}}}});
    let fixture = Fixture::new(&value);
    let snapshot = capture(&fixture, &target());
    let payload = model(&snapshot, "/components/schemas/Payload", "model");
    assert_eq!(field(payload, "name")["type"]["native"], "string");
    assert_eq!(field(payload, "kind")["readonly"], true);
    assert_eq!(
        field(payload, "requiredNullable")["type"]["phpNullAllowed"],
        true
    );
    assert_eq!(
        field(payload, "extra")["type"]["phpdoc"],
        "array<array-key, JsonValue>"
    );
    let policy = &payload.descriptor.as_ref().unwrap()["extraKeyPolicy"];
    assert_eq!(policy["kind"], "pattern-dispatched-json");
    assert_eq!(policy["overlaps"], "all-matching-patterns");
    assert_eq!(policy["patterns"][0]["pattern"], "^x_");
    assert_eq!(policy["patterns"][0]["valueType"]["native"], "JsonNumber");
    let conditional = operation(&snapshot, "conditional");
    assert_eq!(
        conditional.descriptor["body"]["type"]["native"],
        "JsonValue"
    );
    assert!(
        !snapshot
            .models
            .iter()
            .any(|m| m.role == "model" && m.name.ends_with("\\Conditional")),
        "checked JSON carrier must not invent an alias class"
    );
    let mut after = value.clone();
    after["components"]["schemas"]["Payload"]["patternProperties"]["^x_"]["type"] = json!("string");
    let report = compare(&value, &after);
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|change| change.subject.ends_with("\\Payload")),
        "retained pattern dispatch participates in native obligations"
    );
}

#[test]
fn unchanged_relocated_and_documentation_only_sources_have_no_native_findings() {
    let before = api();
    let fixture = Fixture::new(&before);
    let original = compatibility::snapshot(fixture.load(), &[], &[target()]).unwrap();
    let same = compatibility::compare_snapshots(&original, &original);
    assert!(same.is_proven_compatible());
    assert!(same.wire.is_empty());
    assert!(same.native[0].changes.is_empty());
    let relocated = Fixture::new(&before);
    let next = compatibility::snapshot(relocated.load(), &[], &[target()]).unwrap();
    let report = compatibility::compare_snapshots(&original, &next);
    assert!(report.is_proven_compatible(), "{:?}", report.native);
    let mut after = before.clone();
    after["info"]["description"] = json!("longer source offsets");
    after["components"]["schemas"]["Payload"]["description"] = json!("native documentation only");
    after["components"]["schemas"]["Payload"]["properties"]["label"]["default"] =
        json!("still not inserted");
    let report = compare(&before, &after);
    assert!(report.is_proven_compatible(), "{:?}", report.native);
    assert!(report.native[0].changes.is_empty());
}

#[test]
fn requiredness_nullability_and_constant_changes_have_typed_migration_evidence() {
    let before = api();
    let mut after = before.clone();
    after["components"]["schemas"]["Payload"]["required"]
        .as_array_mut()
        .unwrap()
        .push(json!("note"));
    let report = compare(&before, &after);
    let changed = report.native[0]
        .changes
        .iter()
        .find(|c| {
            c.code == "native-model-shape-changed" && c.subject == "Acme\\WidgetsSdk\\Payload"
        })
        .unwrap();
    assert_eq!(
        names(&changed.after.as_ref().unwrap()["constructor"]["parameters"])[..3],
        ["name", "note", "requiredNullable"]
    );
    assert!(!report.is_proven_compatible());
    assert!(!report.wire.is_empty());
    let mut after = before.clone();
    after["components"]["schemas"]["Payload"]["properties"]["label"]["type"] =
        json!(["string", "null"]);
    let report = compare(&before, &after);
    let after = report.native[0].after.as_ref().unwrap();
    assert_eq!(
        field(
            model(after, "/components/schemas/Payload", "model"),
            "label"
        )["type"]["native"],
        "Absent|null|string"
    );
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|c| c.subject == "Acme\\WidgetsSdk\\Payload")
    );
    let mut after = before.clone();
    after["components"]["schemas"]["Tag"]["const"] = json!("changed");
    let report = compare(&before, &after);
    let tagged = model(
        report.native[0].after.as_ref().unwrap(),
        "/components/schemas/Payload",
        "model",
    );
    assert_eq!(field(tagged, "kind")["initialization"]["value"], "changed");
    assert!(report.native[0].changes.iter().any(
        |c| c.code == "native-model-shape-changed" && c.subject == "Acme\\WidgetsSdk\\Payload"
    ));
    let mut after = before.clone();
    after["components"]["schemas"]["Tag"] = json!({"type":"string","enum":["fixed","other"]});
    let report = compare(&before, &after);
    let value = model(
        report.native[0].after.as_ref().unwrap(),
        "/components/schemas/Payload",
        "model",
    );
    assert_eq!(field(value, "kind")["readonly"], false);
    assert_eq!(
        names(&value.descriptor.as_ref().unwrap()["constructor"]["parameters"])[0],
        "kind"
    );
}

#[test]
fn aliases_and_unions_capture_phpdoc_changes_and_actual_codec_names() {
    let before = api();
    let mut after = before.clone();
    after["components"]["schemas"]["ListAlias"]["$ref"] = json!("#/components/schemas/NumberList");
    let report = compare(&before, &after);
    let old = model(
        report.native[0].before.as_ref().unwrap(),
        "/components/schemas/ListAlias",
        "codec-decode",
    );
    let new = model(
        report.native[0].after.as_ref().unwrap(),
        "/components/schemas/ListAlias",
        "codec-decode",
    );
    assert_eq!(old.name, new.name);
    assert_eq!(old.name, "Acme\\WidgetsSdk\\Codecs::decodeListAlias");
    assert_eq!(
        old.descriptor.as_ref().unwrap()["returns"]["native"],
        "array"
    );
    assert_eq!(
        new.descriptor.as_ref().unwrap()["returns"]["native"],
        "array"
    );
    assert_eq!(
        old.descriptor.as_ref().unwrap()["returns"]["phpdoc"],
        "list<string>"
    );
    assert_eq!(
        new.descriptor.as_ref().unwrap()["returns"]["phpdoc"],
        "list<JsonNumber>"
    );
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|c| c.subject == old.name && c.code == "native-model-shape-changed")
    );
    let mut after = before.clone();
    after["components"]["schemas"]["Third"] =
        json!({"type":"object","required":["other"],"properties":{"other":{"type":"boolean"}}});
    after["components"]["schemas"]["Choice"]["oneOf"]
        .as_array_mut()
        .unwrap()
        .push(json!({"$ref":"#/components/schemas/Third"}));
    let report = compare(&before, &after);
    let codec = model(
        report.native[0].after.as_ref().unwrap(),
        "/components/schemas/Choice",
        "codec-decode",
    );
    assert_eq!(
        codec.descriptor.as_ref().unwrap()["returns"]["shape"]["variants"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|c| c.subject == codec.name && c.code == "native-model-shape-changed")
    );
}

#[test]
fn no_input_and_single_success_changes_are_visible_on_the_interface() {
    let before = api();
    let mut after = before.clone();
    after["paths"]["/widgets"]["get"]["parameters"][0]["required"] = json!(true);
    let report = compare(&before, &after);
    let op = operation(report.native[0].after.as_ref().unwrap(), "listWidgets");
    assert_eq!(
        op.descriptor["parameters"]["interfaceMethod"]["canCallWithoutInput"],
        false
    );
    assert_eq!(method_parameters(op, "method")[0]["hasDefault"], false);
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-input-changed")
    );
    let mut after = before.clone();
    after["paths"]["/widgets/{id}"]["patch"]["responses"]["201"] =
        before["paths"]["/widgets/{id}"]["patch"]["responses"]["200"].clone();
    let report = compare(&before, &after);
    let op = operation(report.native[0].after.as_ref().unwrap(), "updateWidget");
    assert_eq!(
        op.descriptor["responseUnions"]["success"]["kind"],
        "native-union"
    );
    assert_eq!(
        op.descriptor["responseUnions"]["success"]["directResult"],
        false
    );
    assert_eq!(
        op.descriptor["parameters"]["method"]["returns"]["native"],
        "UpdateWidgetStatus200|UpdateWidgetStatus201"
    );
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-responses-changed")
    );
}

#[test]
fn package_namespace_defaults_renames_versions_and_admission_are_shared() {
    let fixture = Fixture::new(&api());
    let old = target();
    let mut explicit = old.clone();
    explicit.import_name = Some("Acme\\WidgetsSdk".into());
    let report = compatibility::compare_with_targets(
        fixture.load(),
        fixture.load(),
        &[],
        std::slice::from_ref(&old),
        &[explicit.clone()],
    )
    .unwrap();
    assert!(report.is_proven_compatible(), "{:?}", report.native);
    let mut renamed = explicit.clone();
    renamed.import_name = Some("Other\\Sdk".into());
    let report = compatibility::compare_with_targets(
        fixture.load(),
        fixture.load(),
        &[],
        std::slice::from_ref(&old),
        &[renamed],
    )
    .unwrap();
    assert!(report.wire.is_empty());
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-operation-symbol-changed" && c.subject == "interface")
    );
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-model-renamed")
    );
    let mut version = old.clone();
    version.package_version = "1.0.1".into();
    let report = compatibility::compare_with_targets(
        fixture.load(),
        fixture.load(),
        &[],
        &[old],
        &[version],
    )
    .unwrap();
    assert_eq!(report.native[0].changes.len(), 1);
    assert_eq!(
        report.native[0].changes[0].code,
        "native-package-version-changed"
    );
    explicit.import_name = Some("Invalid\\class".into());
    let snapshot = compatibility::snapshot(fixture.load(), &[], &[explicit.clone()]).unwrap();
    assert_eq!(snapshot.native[0].status, PlanStatus::Unavailable);
    assert!(snapshot.native[0].operations.is_empty() && snapshot.native[0].models.is_empty());
    assert!(
        snapshot.native[0]
            .findings
            .iter()
            .any(|f| f.code == "php-namespace"
                && f.source.as_ref().is_some_and(|s| s.span.is_some()))
    );
    let contract = fixture.load();
    let selected = contract
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    assert!(
        backend::generate(contract, &selected, &explicit)
            .unwrap_err()
            .iter()
            .any(|d| d.code == "php-namespace")
    );
}

#[test]
fn internal_validation_indices_and_source_offsets_are_not_interface_identity() {
    let before = api();
    let fixture = Fixture::new(&before);
    let old = capture(&fixture, &target());
    let mut after = before.clone();
    after["components"]["schemas"]["Aardvark"] =
        json!({"type":"object","properties":{"x":{"type":"string"}}});
    after["paths"]["/aardvark"] = json!({"get":{"operationId":"getAardvark","responses":{"200":{"description":"Extra","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Aardvark"}}}}}}});
    fixture.write(&after);
    let next = capture(&fixture, &target());
    for previous in &old.models {
        let same = next
            .models
            .iter()
            .find(|m| m.source.pointer == previous.source.pointer && m.role == previous.role)
            .unwrap();
        assert_eq!(same.name, previous.name);
        assert_eq!(same.descriptor, previous.descriptor);
    }
    fn no_indices(value: &Value) {
        match value {
            Value::Object(object) => {
                for (key, value) in object {
                    assert!(
                        !["schemaIndex", "nodeIndex", "rootIndex", "validationIndex"]
                            .contains(&key.as_str())
                    );
                    no_indices(value);
                }
            }
            Value::Array(values) => {
                for value in values {
                    no_indices(value);
                }
            }
            _ => {}
        }
    }
    for record in next.models {
        no_indices(record.descriptor.as_ref().unwrap());
    }
}

#[test]
fn sessions_have_zero_warm_work_and_rerender_only_the_php_configuration() {
    let mut value = api();
    // Both selected targets admit this cache-isolation fixture. Typed PHP extras
    // are independently exercised by the descriptor cases above.
    value["components"]["schemas"]["Payload"]["additionalProperties"] = json!(true);
    let fixture = Fixture::new(&value);
    let mut config = SessionConfig {
        targets: vec![
            target(),
            TargetConfig {
                backend: Backend::TypescriptHttp,
                package_name: "reference-sdk".into(),
                package_version: "1.0.0".into(),
                import_name: None,
            },
        ],
        ..Default::default()
    };
    let mut session = Session::new(&fixture.path, config.clone()).unwrap();
    let first = session.generate().unwrap();
    assert_eq!((first.delta.compiles, first.delta.renders), (1, 2));
    let warm = session.generate().unwrap();
    assert_eq!(
        (
            warm.delta.compiles,
            warm.delta.renders,
            warm.delta.cache_hits
        ),
        (0, 0, 1)
    );
    assert!(warm.changed_paths.is_empty());
    assert!(Arc::ptr_eq(&first.contract, &warm.contract));
    assert!(Arc::ptr_eq(&first.files, &warm.files));
    let output = fixture.path.parent().unwrap().join("output");
    session.write(&warm, &output).unwrap();
    let before = fs::metadata(output.join("php/composer.json"))
        .unwrap()
        .modified()
        .unwrap();
    let unchanged = session.generate().unwrap();
    session.write(&unchanged, &output).unwrap();
    assert_eq!(
        before,
        fs::metadata(output.join("php/composer.json"))
            .unwrap()
            .modified()
            .unwrap()
    );
    config.targets[0].import_name = Some("Changed\\Widgets".into());
    session.set_config(config.clone()).unwrap();
    let changed = session.generate().unwrap();
    assert_eq!(
        (
            changed.delta.compiles,
            changed.delta.renders,
            changed.delta.cache_hits
        ),
        (0, 1, 1)
    );
    assert!(Arc::ptr_eq(&first.contract, &changed.contract));
    assert!(!changed.changed_paths.is_empty());
    assert!(changed.changed_paths.iter().all(|p| p.starts_with("php/")));
    let other = |files: &[suspect_codegen::OutFile]| {
        files
            .iter()
            .filter(|f| !f.path.starts_with("php/"))
            .map(|f| (f.path.clone(), f.content.clone()))
            .collect::<BTreeMap<_, _>>()
    };
    assert_eq!(other(&first.files), other(&changed.files));
    config.targets[0] = target();
    session.set_config(config).unwrap();
    let reverted = session.generate().unwrap();
    assert_eq!((reverted.delta.compiles, reverted.delta.renders), (0, 0));
    assert!(Arc::ptr_eq(&first.files, &reverted.files));
}

#[test]
fn schema_refusals_remain_located_unknowns_without_partial_native_records() {
    let mut value = api();
    value["components"]["schemas"]["Payload"]["properties"]["tuple"] = json!({
        "type":"array","prefixItems":[{"type":"string"}]
    });
    let fixture = Fixture::new(&value);
    let result = compatibility::snapshot(fixture.load(), &[], &[target()]).unwrap();
    let snapshot = &result.native[0];
    assert_eq!(snapshot.status, PlanStatus::Unavailable);
    assert!(snapshot.models.is_empty() && snapshot.operations.is_empty());
    assert!(snapshot.findings.iter().any(|finding| {
        finding.code == "php-tuple-native-unsupported"
            && finding.source.as_ref().is_some_and(|source| {
                source.pointer == "/components/schemas/Payload/properties/tuple/prefixItems"
                    && source.span.is_some()
            })
    }));
    let report = compatibility::compare_snapshots(&result, &result);
    assert!(!report.is_proven_compatible());
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|change| change.code == "native-plan-unknown")
    );
}
