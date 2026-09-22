#![cfg(feature = "kotlin-sdk")]
//! Kotlin through public backend, session and native compatibility interfaces.
//! Native JVM acceptance is retained in kotlin_sdk.rs; these checks need no JVM.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use serde_json::{Value, json};
use suspect_codegen::{
    backend::{self, Backend, TargetConfig},
    compatibility::{self, CompatibilityReport, NativeModel, NativeSnapshot, PlanStatus},
    generation_session::{Session, SessionConfig},
    kotlin_sdk::{self, SdkConfig},
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn root() -> PathBuf {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-kotlin-integration");
    std::fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix("case-")
        .tempdir_in(base)
        .unwrap()
        .keep()
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

fn fixture(value: &Value) -> (PathBuf, Arc<Contract>) {
    let path = root().join("api.json");
    std::fs::write(&path, value.to_string()).unwrap();
    let contract = load(&path);
    (path, contract)
}

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::KotlinHttp,
        package_name: "example.widgets:thing-sdk".into(),
        package_version: "1.0.0".into(),
        import_name: Some("example.sdk".into()),
    }
}

fn response(schema: Value) -> Value {
    json!({"description":"Declared response", "content":{"application/json":{"schema":schema}}})
}

fn api() -> Value {
    json!({"openapi":"3.1.0","info":{"title":"Kotlin integration","version":"1"},
    "servers":[{"url":"https://example.test/api/v1"}],"security":[{"apiKey":[]}],
    "components":{"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}},"schemas":{
        "Thing":{"type":"object","required":["kind","required_text","required_note"],
            "properties":{
                "kind":{"type":"string","const":"thing"},
                "required_text":{"type":"string"},
                "required_note":{"type":["string","null"]},
                "optional_text":{"type":"string","default":"annotation"},
                "optional_note":{"type":["string","null"]},
                "payload":{"$ref":"#/components/schemas/Choice"},
                "node":{"$ref":"#/components/schemas/Node"},
                "bag":{"$ref":"#/components/schemas/Bag"}
            }},
        "Patch":{"type":"object","properties":{"note":{"type":["string","null"]}}},
        "Choice":{"oneOf":[{"$ref":"#/components/schemas/TextChoice"},{"$ref":"#/components/schemas/SecretChoice"}]},
        "TextChoice":{"type":"object","required":["kind","text"],"properties":{"kind":{"type":"string","const":"text"},"text":{"type":"string"}}},
        "SecretChoice":{"type":"object","required":["kind","secret"],"properties":{"kind":{"type":"string","const":"secret"},"secret":{"type":"string"}}},
        "Node":{"type":"object","required":["label"],"properties":{"label":{"type":"string"},"child":{"$ref":"#/components/schemas/Node"}}},
        "Bag":{"type":"object","additionalProperties":{"type":["number","null"]}},
        "Failure":{"type":"object","required":["message"],"properties":{"message":{"type":"string"}}}
    }},
    "paths":{
        "/things":{"post":{"operationId":"createThing",
            "requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Thing"}}}},
            "responses":{"201":response(json!({"$ref":"#/components/schemas/Thing"})),"400":response(json!({"$ref":"#/components/schemas/Failure"}))}},
            "get":{"operationId":"listThings","parameters":[{"name":"tag","in":"query","schema":{"type":"string"}}],
                "responses":{"200":response(json!({"type":"array","items":{"$ref":"#/components/schemas/Thing"}}))}}},
        "/things/{id}":{"patch":{"operationId":"updateThing","parameters":[{"name":"id","in":"path","required":true,"schema":{"type":"string"}}],
            "requestBody":{"required":false,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Patch"}}}},
            "responses":{"200":response(json!({"$ref":"#/components/schemas/Thing"}))}}},
        "/status":{"get":{"operationId":"readStatus","responses":{"200":response(json!({"type":"string"}))}}}
    }})
}

fn snapshot(contract: Arc<Contract>, target: &TargetConfig) -> NativeSnapshot {
    let snapshot = compatibility::snapshot(contract, &[], std::slice::from_ref(target)).unwrap();
    let native = snapshot.native.into_iter().next().unwrap();
    assert_eq!(native.status, PlanStatus::Planned, "{:?}", native.findings);
    assert!(native.findings.is_empty(), "{:?}", native.findings);
    native
}

fn model<'a>(native: &'a NativeSnapshot, name: &str, role: &str) -> &'a NativeModel {
    native
        .models
        .iter()
        .find(|m| m.name == name && m.role == role)
        .unwrap_or_else(|| panic!("missing {role} {name}"))
}

fn operation<'a>(native: &'a NativeSnapshot, id: &str) -> &'a compatibility::NativeOperation {
    native
        .operations
        .iter()
        .find(|op| op.operation_id == id)
        .unwrap()
}

fn field<'a>(model: &'a Value, name: &str) -> &'a Value {
    model["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap()
}

fn argument<'a>(constructor: &'a Value, name: &str) -> &'a Value {
    constructor["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name)
        .unwrap()
}

fn compare_change(mut document: Value, edit: impl FnOnce(&mut Value)) -> CompatibilityReport {
    let (path, before) = fixture(&document);
    edit(&mut document);
    std::fs::write(&path, document.to_string()).unwrap();
    let report = compatibility::compare(before, load(&path), &[], &[target()]).unwrap();
    for side in [&report.native[0].before, &report.native[0].after] {
        let native = side.as_ref().unwrap();
        assert_eq!(native.status, PlanStatus::Planned, "{:?}", native.findings);
        assert!(native.findings.is_empty());
    }
    report
}

#[test]
fn public_backend_uses_maven_coordinates_and_real_kotlin_package_identity() {
    let (_, contract) = fixture(&api());
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let files = backend::generate(contract.clone(), &selected, &target()).unwrap();
    assert!(files.iter().all(|f| f.path.starts_with("kotlin/")));
    assert!(Backend::ALL.contains(&Backend::KotlinHttp));
    assert_eq!(Backend::KotlinHttp.name(), "kotlin-http");
    assert_eq!(Backend::KotlinHttp.artifact_directory(), "kotlin");
    // The shared backend registry plans this exact ua/v1 attribution descriptor
    // from the same target identity, so both paths must emit identical bytes.
    let shared = target();
    let direct = kotlin_sdk::plan_sdk(
        contract.clone(),
        &selected,
        SdkConfig {
            group_id: "example.widgets".into(),
            artifact_id: "thing-sdk".into(),
            version: "1.0.0".into(),
            package_name: "example.sdk".into(),
            credential_env: None,
            sdk_defaults: None,
            attribution: Some(suspect_codegen::attribution::AttributionDescriptor::plan(
                env!("CARGO_PKG_VERSION"),
                &shared.package_name,
                &shared.package_version,
                contract.openapi_version(),
                shared.backend.language_tag(),
            )),
        },
    )
    .unwrap()
    .render()
    .unwrap();
    assert_eq!(files, direct);
    let pom = &files
        .iter()
        .find(|f| f.path == "kotlin/pom.xml")
        .unwrap()
        .content;
    assert!(
        pom.contains("<groupId>example.widgets</groupId>")
            && pom.contains("<artifactId>thing-sdk</artifactId>")
    );
    let implicit = TargetConfig {
        import_name: None,
        ..target()
    };
    let defaults = backend::generate(contract, &selected, &implicit).unwrap();
    assert!(
        defaults
            .iter()
            .any(|f| f.path == "kotlin/src/main/kotlin/example/widgets/thing_sdk/Client.kt")
    );
    assert!(defaults.iter().any(|f| f.path.ends_with("Client.kt")
        && f.content.contains("package example.widgets.thing_sdk")));
}

#[test]
fn unchanged_snapshots_have_no_findings_or_native_differences() {
    let (path, contract) = fixture(&api());
    let first = snapshot(contract.clone(), &target());
    let second = snapshot(load(&path), &target());
    assert_eq!(first, second);
    let report = compatibility::compare(contract.clone(), contract, &[], &[target()]).unwrap();
    assert!(report.is_proven_compatible(), "{report:#?}");
    assert!(report.wire.is_empty());
    assert!(report.native[0].changes.is_empty());
    assert!(first.models.iter().all(|m| m.descriptor.is_some()));
}

#[test]
fn native_descriptors_preserve_presence_fixed_getters_aliases_and_union_constructors() {
    let (_, contract) = fixture(&api());
    let native = snapshot(contract, &target());
    let thing = model(&native, "example.sdk.Thing", "model")
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(thing["kind"], "data-class");
    let required = field(thing, "requiredText");
    assert_eq!(
        required["type"],
        json!({"kind":"primitive","name":"kotlin.String"})
    );
    let nullable = field(thing, "requiredNote");
    assert_eq!(nullable["type"]["kind"], "nullable");
    assert_eq!(
        argument(&thing["constructor"], "requiredNote")["hasDefault"],
        false
    );
    let optional = field(thing, "optionalText");
    assert_eq!(optional["type"]["name"], "example.sdk.Presence");
    assert_eq!(optional["type"]["arguments"][0]["name"], "kotlin.String");
    let optional_null = field(thing, "optionalNote");
    assert_eq!(optional_null["type"]["arguments"][0]["kind"], "nullable");
    assert_eq!(
        argument(&thing["constructor"], "optionalText")["initialization"],
        json!({"kind":"singleton","name":"example.sdk.Presence.Absent"})
    );
    let tag = field(thing, "kind");
    assert_eq!(tag["constructorParameter"], false);
    assert_eq!(tag["getterKind"], "fixed-enum-member");
    assert_eq!(
        tag["initialization"],
        json!({"kind":"literal","type":"example.sdk.ThingKind","member":"THING","value":"thing"})
    );
    assert!(
        !thing["constructor"]["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "kind")
    );
    assert_eq!(thing["codec"]["target"], "example.sdk.Codecs.thing");
    let node = model(&native, "example.sdk.Node", "model")
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(
        field(node, "child")["type"]["arguments"][0],
        json!({"kind":"named","name":"example.sdk.Node"})
    );
    let bag = model(&native, "example.sdk.Bag", "model")
        .descriptor
        .as_ref()
        .unwrap();
    let extras = argument(&bag["constructor"], "additionalProperties");
    assert_eq!(extras["type"]["name"], "kotlin.collections.Map");
    assert_eq!(
        extras["type"]["arguments"][1],
        json!({"kind":"nullable","type":{"kind":"named","name":"example.sdk.JsonNumber"}})
    );
    let union = model(&native, "example.sdk.Choice", "model")
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(union["kind"], "sealed-interface");
    let arm = model(&native, "example.sdk.Choice.AsTextChoice", "union-arm")
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(
        argument(&arm["constructor"], "value")["type"],
        json!({"kind":"named","name":"example.sdk.TextChoice"})
    );
    assert_eq!(arm["implements"], json!(["example.sdk.Choice"]));
    let primitive_codec = model(&native, "example.sdk.Codecs.thingRequiredText", "codec")
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(
        primitive_codec["type"]["arguments"][0],
        json!({"kind":"primitive","name":"kotlin.String"})
    );
    assert!(
        !native
            .models
            .iter()
            .any(|m| m.role == "model" && m.name == "example.sdk.ThingRequiredText")
    );
    let alias_codec = model(&native, "example.sdk.Codecs.thingPayload", "codec")
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(
        alias_codec["type"]["arguments"][0],
        json!({"kind":"named","name":"example.sdk.Choice"})
    );
    assert!(
        !native
            .models
            .iter()
            .any(|m| m.role == "model" && m.name == "example.sdk.ThingPayload")
    );
}

#[test]
fn method_input_controls_credentials_and_status_data_are_native_signatures() {
    let (_, contract) = fixture(&api());
    let native = snapshot(contract, &target());
    let create = operation(&native, "createThing");
    assert_eq!(create.symbols["client"], "example.sdk.Client");
    assert_eq!(
        create.symbols["input-constructor"],
        "example.sdk.CreateThingInput"
    );
    assert_eq!(
        create.descriptor["package"],
        json!({"groupId":"example.widgets","artifactId":"thing-sdk","version":"1.0.0","namespace":"example.sdk"})
    );
    let method = &create.descriptor["parameters"]["method"];
    assert_eq!(method["kind"], "suspend-method");
    assert_eq!(argument(method, "input")["hasDefault"], false);
    assert_eq!(
        argument(method, "requestOptions")["type"],
        json!({"kind":"named","name":"example.sdk.RequestOptions"})
    );
    assert_eq!(
        argument(method, "requestOptions")["initialization"]["name"],
        "example.sdk.RequestOptions"
    );
    assert_eq!(method["cancellation"], "caller-coroutine-context");
    let list = operation(&native, "listThings");
    assert_eq!(
        list.descriptor["parameters"]["method"]["canCallWithoutInputs"],
        true
    );
    assert_eq!(
        argument(&list.descriptor["parameters"]["method"], "input")["initialization"]["name"],
        "example.sdk.ListThingsInput"
    );
    let empty = operation(&native, "readStatus");
    assert_eq!(
        empty.descriptor["constructor"]["inputDeclaration"]["kind"],
        "class"
    );
    assert_eq!(
        empty.descriptor["parameters"]["method"]["canCallWithoutInputs"],
        true
    );
    let update = operation(&native, "updateThing");
    assert_eq!(
        update.descriptor["body"]["type"]["name"],
        "example.sdk.Presence"
    );
    assert_eq!(
        argument(&update.descriptor["constructor"]["input"], "body")["hasDefault"],
        true
    );
    let client = &create.descriptor["constructor"]["client"]["signature"];
    assert_eq!(
        client["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["credentials", "transport", "options"]
    );
    assert_eq!(
        argument(client, "transport")["initialization"]["name"],
        "example.sdk.JdkTransport"
    );
    let credential = &create.descriptor["credential"];
    assert_eq!(credential["selectedMember"], "apiKey");
    assert_eq!(
        argument(&credential["constructor"], "apiKey")["type"]["kind"],
        "nullable"
    );
    assert_eq!(
        argument(&credential["constructor"], "apiKey")["initialization"],
        json!({"kind":"literal","value":null})
    );
    assert_eq!(
        create.descriptor["responseUnions"]["success"]["data"]["type"],
        json!({"kind":"named","name":"example.sdk.Thing"})
    );
    let statuses = create.descriptor["responses"].as_array().unwrap();
    let success = statuses.iter().find(|v| v["status"] == 201).unwrap();
    assert_eq!(success["type"], "example.sdk.CreateThingResult.Status201");
    assert_eq!(success["dataOverrides"], true);
    let failure = statuses.iter().find(|v| v["status"] == 400).unwrap();
    assert_eq!(
        failure["type"],
        "example.sdk.CreateThingApiException.Status400"
    );
    assert_eq!(failure["base"], "example.sdk.CreateThingApiException");
    assert_eq!(failure["responseInherited"], true);
}

#[test]
fn required_nullability_getter_and_union_changes_are_reported() {
    let changed_type = compare_change(api(), |api| {
        api["components"]["schemas"]["Thing"]["properties"]["required_text"]["type"] =
            json!("number");
    });
    let changed = model(
        changed_type.native[0].after.as_ref().unwrap(),
        "example.sdk.Thing",
        "model",
    )
    .descriptor
    .as_ref()
    .unwrap();
    assert_eq!(
        argument(&changed["constructor"], "requiredText")["type"],
        json!({"kind":"named","name":"example.sdk.JsonNumber"})
    );
    assert!(
        changed_type.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-model-shape-changed"
                && c.subject == "example.sdk.Codecs.thingRequiredText")
    );
    let required = compare_change(api(), |api| {
        api["components"]["schemas"]["Thing"]["required"]
            .as_array_mut()
            .unwrap()
            .push(json!("optional_note"))
    });
    assert!(
        required.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-model-shape-changed" && c.subject == "example.sdk.Thing")
    );
    let after = required.native[0].after.as_ref().unwrap();
    let thing = model(after, "example.sdk.Thing", "model")
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(
        argument(&thing["constructor"], "optionalNote")["hasDefault"],
        false
    );
    assert_eq!(
        argument(&thing["constructor"], "optionalNote")["type"]["kind"],
        "nullable"
    );
    let nullability = compare_change(api(), |api| {
        api["components"]["schemas"]["Thing"]["properties"]["required_note"]["type"] =
            json!("string")
    });
    assert!(
        nullability.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-model-shape-changed" && c.subject == "example.sdk.Thing")
    );
    let getter = compare_change(api(), |api| {
        api["components"]["schemas"]["Thing"]["properties"]["kind"]
            .as_object_mut()
            .unwrap()
            .remove("const");
    });
    let before = model(
        getter.native[0].before.as_ref().unwrap(),
        "example.sdk.Thing",
        "model",
    )
    .descriptor
    .as_ref()
    .unwrap();
    let after = model(
        getter.native[0].after.as_ref().unwrap(),
        "example.sdk.Thing",
        "model",
    )
    .descriptor
    .as_ref()
    .unwrap();
    assert!(
        !before["constructor"]["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["name"] == "kind")
    );
    assert_eq!(argument(&after["constructor"], "kind")["required"], true);
    let tag = compare_change(api(), |api| {
        api["components"]["schemas"]["Thing"]["properties"]["kind"]["const"] = json!("thing-v2")
    });
    assert!(tag.native[0].changes.iter().any(|c|c.code == "native-model-shape-changed" && c.subject == "example.sdk.ThingKind"));
    let union = compare_change(api(), |api| {
        api["components"]["schemas"]["CountChoice"] = json!({"type":"integer"});
        api["components"]["schemas"]["Choice"]["oneOf"]
            .as_array_mut()
            .unwrap()
            .push(json!({"$ref":"#/components/schemas/CountChoice"}));
    });
    assert!(
        union.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-model-shape-changed" && c.subject == "example.sdk.Choice")
    );
    let count = model(
        union.native[0].after.as_ref().unwrap(),
        "example.sdk.Choice.AsCountChoice",
        "union-arm",
    )
    .descriptor
    .as_ref()
    .unwrap();
    assert_eq!(
        argument(&count["constructor"], "value")["type"],
        json!({"kind":"named","name":"example.sdk.JsonNumber"})
    );
}

#[test]
fn nullable_object_codecs_do_not_invent_nullable_class_constructors() {
    let report = compare_change(api(), |api| {
        api["components"]["schemas"]["Thing"]["type"] = json!(["object", "null"]);
    });
    let before = report.native[0].before.as_ref().unwrap();
    let after = report.native[0].after.as_ref().unwrap();
    let old = model(before, "example.sdk.Thing", "model")
        .descriptor
        .as_ref()
        .unwrap();
    let new = model(after, "example.sdk.Thing", "model")
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(old["constructor"], new["constructor"]);
    assert_eq!(
        new["codec"]["type"]["arguments"][0],
        json!({"kind":"nullable","type":{"kind":"named","name":"example.sdk.Thing"}})
    );
    let input = &operation(after, "createThing").descriptor["constructor"]["input"];
    assert_eq!(argument(input, "body")["hasDefault"], false);
    assert_eq!(argument(input, "body")["type"]["kind"], "nullable");
}

#[test]
fn collision_allocated_names_are_used_by_every_constructor_and_codec_reference() {
    let mut document = api();
    document["components"]["schemas"]["Client"] = json!({"type":"object","properties":{"copy":{"type":"string"},"a-b":{"type":"string"},"a_b":{"type":"string"}}});
    document["paths"]["/status"]["get"]["operationId"] = json!("close");
    document["paths"]["/status"]["get"]["responses"]["200"] =
        response(json!({"$ref":"#/components/schemas/Client"}));
    let (_, contract) = fixture(&document);
    let native = snapshot(contract, &target());
    let operation = operation(&native, "close");
    assert_eq!(operation.symbols["method"], "close2");
    let model = model(&native, "example.sdk.Client2", "model")
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(model["constructor"]["name"], "example.sdk.Client2");
    assert_eq!(model["codec"]["target"], "example.sdk.Codecs.client2");
    assert!(
        model["fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["name"] == "copyValue")
    );
    let names = model["constructor"]["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(names.contains("aB") && names.contains("aB2"));
    assert_eq!(
        operation.descriptor["responseUnions"]["success"]["data"]["type"],
        json!({"kind":"named","name":"example.sdk.Client2"})
    );
}

#[test]
fn input_defaults_and_direct_data_projection_changes_are_visible() {
    let required = compare_change(api(), |api| {
        api["paths"]["/things"]["get"]["parameters"][0]["required"] = json!(true)
    });
    assert!(
        required.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-input-constructor-changed"
                && c.operation_id_after.as_deref() == Some("listThings"))
    );
    let list = operation(required.native[0].after.as_ref().unwrap(), "listThings");
    assert_eq!(
        list.descriptor["parameters"]["method"]["canCallWithoutInputs"],
        false
    );
    let body = compare_change(api(), |api| {
        api["paths"]["/things/{id}"]["patch"]["requestBody"]["required"] = json!(true)
    });
    assert_eq!(
        operation(body.native[0].after.as_ref().unwrap(), "updateThing").descriptor["body"]["type"],
        json!({"kind":"named","name":"example.sdk.Patch"})
    );
    let mut document = api();
    document["paths"]["/status"]["get"]["responses"]["201"] = response(json!({"type":"string"}));
    let changed = compare_change(document, |api| {
        api["paths"]["/status"]["get"]["responses"]["201"] = response(json!({"type":"number"}))
    });
    let before = operation(changed.native[0].before.as_ref().unwrap(), "readStatus");
    let after = operation(changed.native[0].after.as_ref().unwrap(), "readStatus");
    assert_eq!(
        before.descriptor["responseUnions"]["success"]["data"]["type"],
        json!({"kind":"primitive","name":"kotlin.String"})
    );
    assert!(after.descriptor["responseUnions"]["success"]["data"].is_null());
    assert!(
        after.descriptor["responses"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["dataOverrides"] == false)
    );
    assert!(
        changed.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-responses-changed" && c.subject == "responseUnions")
    );
}

#[test]
fn prose_annotations_and_internal_program_indices_do_not_become_constructor_changes() {
    let report = compare_change(api(), |api| {
        api["info"]["description"] = json!("Changed docs");
        api["paths"]["/things"]["post"]["description"] =
            json!("Source prose with */ @param and constructor(value).");
        api["paths"]["/things"]["post"]["summary"] = json!("New summary");
        api["components"]["schemas"]["Thing"]["title"] = json!("Documentation title");
        let member = &mut api["components"]["schemas"]["Thing"]["properties"]["optional_text"];
        member["description"] = json!("Source prose");
        member["default"] = json!("different annotation");
        member["example"] = json!("valid example");
        member["readOnly"] = json!(false);
        member["writeOnly"] = json!(false);
    });
    assert!(
        report.native[0].changes.is_empty(),
        "{:?}",
        report.native[0].changes
    );
    let mut document = api();
    let (path, before) = fixture(&document);
    let program_index = |contract: Arc<Contract>| {
        let selected = contract
            .operations()
            .map(|op| op.source().clone())
            .collect::<Vec<_>>();
        let plan = kotlin_sdk::plan_sdk(contract, &selected, SdkConfig::default()).unwrap();
        plan.program()
            .roots
            .iter()
            .find(|root| root.source.pointer == "/components/schemas/Thing/properties/kind")
            .unwrap()
            .target
    };
    let old_index = program_index(before.clone());
    document["components"]["schemas"]["Thing"]["properties"]["aaa"] = json!({"type":"string"});
    std::fs::write(&path, document.to_string()).unwrap();
    let after = load(&path);
    assert_ne!(
        old_index,
        program_index(after.clone()),
        "control must really shift the internal graph index"
    );
    let changed = compatibility::compare(before, after, &[], &[target()]).unwrap();
    for name in [
        "example.sdk.ThingKind",
        "example.sdk.Codecs.thingKind",
        "example.sdk.Codecs.thingRequiredText",
    ] {
        let role = if name.contains("Codecs.") {
            "codec"
        } else {
            "model"
        };
        assert_eq!(
            model(changed.native[0].before.as_ref().unwrap(), name, role).descriptor,
            model(changed.native[0].after.as_ref().unwrap(), name, role).descriptor
        );
        assert!(
            !changed.native[0].changes.iter().any(|c| c.subject == name),
            "{name}: {:?}",
            changed.native[0].changes
        );
    }
}

#[test]
fn namespace_and_maven_identity_changes_use_effective_names_and_keep_version_metadata_separate() {
    let (_, contract) = fixture(&api());
    let implicit = TargetConfig {
        import_name: None,
        ..target()
    };
    let explicit = TargetConfig {
        import_name: Some("example.widgets.thing_sdk".into()),
        ..target()
    };
    let same = compatibility::compare_with_targets(
        contract.clone(),
        contract.clone(),
        &[],
        &[implicit],
        &[explicit],
    )
    .unwrap();
    assert!(
        same.native[0].changes.is_empty(),
        "{:?}",
        same.native[0].changes
    );
    let renamed = TargetConfig {
        import_name: Some("renamed.sdk".into()),
        ..target()
    };
    let rename = compatibility::compare_with_targets(
        contract.clone(),
        contract.clone(),
        &[],
        &[target()],
        &[renamed],
    )
    .unwrap();
    assert!(
        rename.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-operation-symbol-changed" && c.subject == "client")
    );
    assert!(
        rename.native[0]
            .after
            .as_ref()
            .unwrap()
            .models
            .iter()
            .all(|m| m.name.starts_with("renamed.sdk."))
    );
    let version = TargetConfig {
        package_version: "1.0.1".into(),
        ..target()
    };
    let bump = compatibility::compare_with_targets(
        contract.clone(),
        contract.clone(),
        &[],
        &[target()],
        &[version],
    )
    .unwrap();
    assert_eq!(
        bump.native[0].changes.len(),
        1,
        "{:?}",
        bump.native[0].changes
    );
    assert_eq!(
        bump.native[0].changes[0].code,
        "native-package-version-changed"
    );
    let coordinate = TargetConfig {
        package_name: "example.other:renamed-sdk".into(),
        ..target()
    };
    let package = compatibility::compare_with_targets(
        contract.clone(),
        contract,
        &[],
        &[target()],
        &[coordinate],
    )
    .unwrap();
    assert_eq!(
        package.native[0].changes.len(),
        1,
        "{:?}",
        package.native[0].changes
    );
    assert_eq!(
        package.native[0].changes[0].code,
        "native-package-name-changed"
    );
}

#[test]
fn credential_constructor_and_selected_member_changes_are_reported() {
    let report = compare_change(api(), |api| {
        let scheme = api["components"]["securitySchemes"]
            .as_object_mut()
            .unwrap()
            .remove("apiKey")
            .unwrap();
        api["components"]["securitySchemes"]["managementToken"] = scheme;
        api["security"] = json!([{"managementToken":[]}]);
    });
    let native = &report.native[0];
    assert!(
        native
            .changes
            .iter()
            .any(|c| c.code == "native-credentials-changed")
    );
    let credential =
        &operation(native.after.as_ref().unwrap(), "createThing").descriptor["credential"];
    assert_eq!(credential["selectedMember"], "managementToken");
    assert_eq!(
        argument(&credential["constructor"], "managementToken")["type"],
        json!({"kind":"nullable","type":{"kind":"primitive","name":"kotlin.String"}})
    );
}

#[test]
fn package_and_model_refusals_are_public_backend_and_snapshot_findings() {
    let (_, contract) = fixture(&api());
    let selected = contract
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    for invalid in [
        TargetConfig {
            package_name: "missing-coordinates".into(),
            ..target()
        },
        TargetConfig {
            package_version: "latest".into(),
            ..target()
        },
        TargetConfig {
            import_name: Some("example.class".into()),
            ..target()
        },
    ] {
        assert!(backend::generate(contract.clone(), &selected, &invalid).is_err());
        let native = compatibility::snapshot(contract.clone(), &[], &[invalid])
            .unwrap()
            .native
            .remove(0);
        assert_eq!(native.status, PlanStatus::Unavailable);
        assert!(!native.findings.is_empty());
        assert!(
            !native
                .findings
                .iter()
                .any(|f| f.code == "native-backend-unregistered")
        );
    }
    let mut document = api();
    document["components"]["schemas"]["Thing"]["properties"]["required_text"]["writeOnly"] =
        json!(true);
    let (_, contract) = fixture(&document);
    let native = compatibility::snapshot(contract, &[], &[target()])
        .unwrap()
        .native
        .remove(0);
    assert_eq!(native.status, PlanStatus::Unavailable);
    assert!(native.findings.iter().any(|f| {
        f.code == "kotlin-direction-unsupported"
            && f.source
                .as_ref()
                .is_some_and(|s| s.pointer.ends_with("/writeOnly"))
    }));
}

#[test]
fn sessions_do_zero_warm_work_and_only_render_the_changed_kotlin_target() {
    let (path, _) = fixture(&api());
    let other = TargetConfig {
        backend: Backend::TypescriptHttp,
        package_name: "thing-sdk".into(),
        package_version: "1.0.0".into(),
        import_name: None,
    };
    let mut config = SessionConfig {
        targets: vec![target(), other],
        ..Default::default()
    };
    let mut session = Session::new(&path, config.clone()).unwrap();
    let cold = session.generate().unwrap();
    assert_eq!((cold.delta.compiles, cold.delta.renders), (1, 2));
    let warm = session.generate().unwrap();
    assert_eq!(
        (
            warm.delta.compiles,
            warm.delta.renders,
            warm.delta.cache_hits
        ),
        (0, 0, 1)
    );
    assert!(Arc::ptr_eq(&cold.contract, &warm.contract) && Arc::ptr_eq(&cold.files, &warm.files));
    assert!(warm.changed_paths.is_empty());
    config.targets[0].import_name = Some("renamed.sdk".into());
    session.set_config(config.clone()).unwrap();
    let renamed = session.generate().unwrap();
    assert_eq!(
        (
            renamed.delta.compiles,
            renamed.delta.renders,
            renamed.delta.cache_hits
        ),
        (0, 1, 1)
    );
    assert!(Arc::ptr_eq(&cold.contract, &renamed.contract));
    assert!(
        !renamed.changed_paths.is_empty()
            && renamed
                .changed_paths
                .iter()
                .all(|p| p.starts_with("kotlin/"))
    );
    let typescript = |output: &suspect_codegen::generation_session::SessionOutput| {
        output
            .files
            .iter()
            .filter(|f| f.path.starts_with("typescript/"))
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(typescript(&cold), typescript(&renamed));
    config.targets[0].package_version = "1.0.1".into();
    session.set_config(config).unwrap();
    let version = session.generate().unwrap();
    assert_eq!(
        (
            version.delta.compiles,
            version.delta.renders,
            version.delta.cache_hits
        ),
        (0, 1, 1)
    );
    assert!(Arc::ptr_eq(&cold.contract, &version.contract));
    assert!(
        version
            .changed_paths
            .iter()
            .all(|p| p.starts_with("kotlin/"))
    );
    let warm = session.generate().unwrap();
    assert_eq!((warm.delta.compiles, warm.delta.renders), (0, 0));
    assert!(Arc::ptr_eq(&version.files, &warm.files));
}

fn rich_api() -> Value {
    let mut value = api();
    value["openapi"] = json!("3.2.0");
    value["components"]["securitySchemes"]["basic"] = json!({"type":"http","scheme":"basic"});
    value["components"]["securitySchemes"]["oidc"] = json!({"type":"openIdConnect","openIdConnectUrl":"https://example.test/.well-known/openid-configuration"});
    value["paths"]["/things"]["post"]["security"] = json!([{"basic":[]},{"oidc":["read"]},{}]);
    value["paths"]["/things"]["post"]["requestBody"]["content"] = json!({"application/octet-stream":{},"multipart/form-data":{"schema":{"type":"object","required":["file"],"additionalProperties":false,"properties":{"file":{}}},"encoding":{"file":{"contentType":"application/octet-stream","headers":{"X-Tag":{"required":true,"schema":{"type":"string","example":"tag"}}}}}}});
    value["paths"]["/things"]["post"]["responses"]["201"]["headers"] =
        json!({"X-Count":{"required":true,"schema":{"type":"integer","example":3}}});
    value["paths"]["/status"]["get"]["responses"] = json!({"200":{"description":"items","content":{"application/x-ndjson":{"itemSchema":{"$ref":"#/components/schemas/Thing"}}}}});
    value
}

#[test]
fn rich_protocol_capture_has_actual_wrappers_controls_and_cold_flow_signatures() {
    let (_, contract) = fixture(&rich_api());
    let native = snapshot(contract, &target());
    let create = &operation(&native, "createThing").descriptor;
    let body = create["body"]["mediaChoice"].as_str().unwrap();
    let choice = model(&native, body, "request-media-choice")
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(choice["variants"].as_array().unwrap().len(), 2);
    let form = native
        .models
        .iter()
        .find(|m| m.role == "wire-body")
        .unwrap()
        .descriptor
        .as_ref()
        .unwrap();
    let wrapper = argument(&form["constructor"], "file")["type"]["name"]
        .as_str()
        .unwrap();
    let file = model(&native, wrapper, "part-wrapper")
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(
        argument(&file["constructor"], "value")["type"]["name"],
        "example.sdk.Upload"
    );
    assert_eq!(
        argument(&file["constructor"], "headers")["hasDefault"],
        false
    );
    assert_eq!(
        file["componentMembers"],
        json!(["component1", "component2", "component3"])
    );
    let status = create["responses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["status"] == 201)
        .unwrap();
    assert_eq!(
        argument(&status["constructor"], "responseHeaders")["type"]["name"],
        "example.sdk.CreateThingStatus201Headers"
    );
    assert_eq!(
        argument(&create["credential"]["constructor"], "basic")["type"]["type"]["name"],
        "example.sdk.BasicCredentials"
    );
    assert_eq!(
        argument(&create["credential"]["constructor"], "oidc")["type"]["type"]["name"],
        "example.sdk.CredentialProvider"
    );
    let runtime = &create["parameters"]["controls"]["runtime"];
    assert_eq!(
        argument(&runtime["Upload"], "data")["type"]["name"],
        "kotlin.ByteArray"
    );
    assert_eq!(
        argument(&runtime["ClientOptions"], "maxStreamItemBytes")["initialization"]["value"],
        1048576
    );
    assert_eq!(
        argument(&runtime["StreamingResponse"], "body")["type"]["name"],
        "example.sdk.BodyReader"
    );
    let stream = &operation(&native, "readStatus").descriptor["parameters"]["method"];
    assert_eq!(stream["kind"], "cold-flow-method");
    assert_eq!(stream["returns"]["name"], "kotlinx.coroutines.flow.Flow");
    assert_eq!(
        stream["returns"]["arguments"][0]["name"],
        "example.sdk.ReadStatusResult"
    );
}

#[test]
fn rich_part_type_and_stream_method_changes_are_compatibility_changes() {
    let report = compare_change(rich_api(), |v| {
        v["paths"]["/things"]["post"]["requestBody"]["content"]["multipart/form-data"]["encoding"]
            ["file"]["headers"]["X-Tag"]["schema"] = json!({"type":"integer","example":7});
        v["paths"]["/status"]["get"]["responses"]["200"] =
            response(json!({"$ref":"#/components/schemas/Thing"}));
    });
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-model-shape-changed" && c.subject.ends_with("FileHeaders"))
    );
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-input-changed"
                && c.operation_id_after.as_deref() == Some("readStatus"))
    );
}

#[test]
fn explicit_binary_profile_is_retained_by_backend_snapshot_and_session() {
    use suspect_codegen::http_protocol::CompatibilityProfile::LegacyBinaryStringV1;
    let generation = backend::GenerationOptions {
        compatibility_profiles: [LegacyBinaryStringV1].into(),
        ..Default::default()
    };
    let mut doc = api();
    doc["paths"]["/status"]["get"]["responses"]["200"] = json!({"description":"bytes","content":{"application/octet-stream":{"schema":{"type":"string","format":"binary"}}}});
    let (path, contract) = fixture(&doc);
    let standard = compatibility::snapshot(contract.clone(), &[], &[target()]).unwrap();
    assert_eq!(standard.native[0].status, PlanStatus::Unavailable);
    let captured =
        compatibility::snapshot_with_options(contract, &[], &[target()], &generation).unwrap();
    assert_eq!(
        captured.native[0].status,
        PlanStatus::Planned,
        "{:?}",
        captured.native[0].findings
    );
    assert_eq!(captured.native[0].generation, generation);
    assert_eq!(
        operation(&captured.native[0], "readStatus").descriptor["responseUnions"]["success"]["data"]
            ["type"]["name"],
        "kotlin.ByteArray"
    );
    let mut config = SessionConfig {
        targets: vec![target()],
        generation: generation.clone(),
        ..Default::default()
    };
    let mut session = Session::new(&path, config.clone()).unwrap();
    let first = session.generate().unwrap();
    let warm = session.generate().unwrap();
    assert_eq!((warm.delta.compiles, warm.delta.renders), (0, 0));
    assert!(Arc::ptr_eq(&first.files, &warm.files));
    config.generation = Default::default();
    session.set_config(config).unwrap();
    assert!(
        session.generate().is_err(),
        "profile removal must not reuse an admitted binary generation"
    );
}
