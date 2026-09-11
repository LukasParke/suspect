//! Public-contract compatibility cases. Expected wire variance and consumer
//! symbols are specified independently of the comparison implementation.

use std::{path::PathBuf, sync::Arc};

use serde_json::{Value, json};
use suspect_codegen::{
    backend::{Backend, TargetConfig},
    compatibility::{self, CompatibilityReport, Direction, Impact, PlanStatus},
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

struct Fixture {
    directory: tempfile::TempDir,
    entry: PathBuf,
}
impl Fixture {
    fn new(api: &Value, models: &Value) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let entry = directory.path().join("api.json");
        let fixture = Self { directory, entry };
        fixture.write(api, models);
        fixture
    }
    fn write(&self, api: &Value, models: &Value) {
        std::fs::write(&self.entry, serde_json::to_vec_pretty(api).unwrap()).unwrap();
        std::fs::write(
            self.directory.path().join("models.json"),
            serde_json::to_vec_pretty(models).unwrap(),
        )
        .unwrap();
    }
    fn load(&self) -> Arc<Contract> {
        let workspace = Arc::new(
            WorkspaceBuilder::new()
                .root(self.directory.path())
                .build()
                .unwrap(),
        );
        Arc::new(
            Contract::from_workspace(&workspace, &Uri::from_path(&self.entry).unwrap()).unwrap(),
        )
    }
}

fn models() -> Value {
    json!({"$defs":{
        "Thing":{"type":"object","additionalProperties":false,"required":["name"],"properties":{
            "name":{"type":"string","minLength":1},
            "note":{"type":["string","null"]},
            "description":{"type":"string"}
        }},
        "Failure":{"type":"object","additionalProperties":false,"required":["message"],"properties":{"message":{"type":"string"}}}
    }})
}
fn api() -> Value {
    json!({"openapi":"3.1.0","info":{"title":"Things","version":"1"},
        "servers":[{"url":"https://api.example.com/v1"}],"security":[{"Bearer":[]}],
        "components":{"securitySchemes":{"Bearer":{"type":"http","scheme":"bearer"}}},
        "paths":{"/things/{id}":{"patch":{
            "operationId":"updateThing",
            "parameters":[{"name":"id","in":"path","required":true,"schema":{"type":"string"}},
                {"name":"user-name","in":"query","schema":{"type":"string"}}],
            "requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"./models.json#/$defs/Thing"}}}},
            "responses":{
                "200":{"description":"Updated","content":{"application/json":{"schema":{"$ref":"./models.json#/$defs/Thing"}}}},
                "400":{"description":"Invalid","content":{"application/json":{"schema":{"$ref":"./models.json#/$defs/Failure"}}}}
            }
        }}}
    })
}
fn op(api: &mut Value) -> &mut Value {
    &mut api["paths"]["/things/{id}"]["patch"]
}
fn selection() -> Vec<String> {
    vec!["updateThing".into()]
}
fn target(backend: Backend) -> TargetConfig {
    TargetConfig {
        backend,
        package_name: match backend {
            Backend::GoHttp => "example.com/things-sdk",
            Backend::SwiftHttp => "ThingsSDK",
            _ => "things-sdk",
        }
        .into(),
        package_version: "1.0.0".into(),
        import_name: None,
    }
}
fn targets() -> Vec<TargetConfig> {
    [
        Backend::TypescriptHttp,
        Backend::RustHttp,
        Backend::PythonHttp,
        Backend::GoHttp,
        Backend::SwiftHttp,
    ]
    .into_iter()
    .map(target)
    .collect()
}
fn schema_impact(report: &CompatibilityReport, direction: Direction) -> Impact {
    report
        .wire
        .iter()
        .find(|change| change.code == "wire-schema-changed" && change.direction == Some(direction))
        .unwrap_or_else(|| panic!("missing {direction:?} schema change: {:?}", report.wire))
        .impact
}

fn schema_model<'a>(
    snapshot: &'a compatibility::NativeSnapshot,
    pointer: &str,
) -> &'a compatibility::NativeModel {
    snapshot
        .models
        .iter()
        .find(|model| model.role != "codec" && model.source.pointer == pointer)
        .unwrap_or_else(|| panic!("missing native model {pointer}: {:?}", snapshot.findings))
}

#[test]
fn external_targets_are_compared_even_when_the_entry_and_ref_text_are_unchanged() {
    let api = api();
    let mut models = models();
    let fixture = Fixture::new(&api, &models);
    let before = fixture.load();
    models["$defs"]["Thing"]["properties"]["name"]["minLength"] = json!(3);
    fixture.write(&api, &models);
    let report = compatibility::compare(before, fixture.load(), &selection(), &[]).unwrap();
    assert_eq!(
        schema_impact(&report, Direction::Request),
        Impact::PotentiallyBreaking
    );
    assert_eq!(
        schema_impact(&report, Direction::Response),
        Impact::Compatible
    );
    assert!(!report.is_proven_compatible());
    let entry = |metadata: &compatibility::SnapshotMetadata| {
        metadata
            .documents
            .iter()
            .find(|document| document.document.ends_with("/api.json"))
            .unwrap()
            .normalized_sha256
            .clone()
    };
    assert_eq!(entry(&report.before), entry(&report.after));
    let delta = report
        .wire
        .iter()
        .flat_map(|change| &change.schema_deltas)
        .find(|delta| delta.keyword == "minLength")
        .unwrap();
    assert!(
        delta
            .source_after
            .as_ref()
            .unwrap()
            .document
            .ends_with("/models.json")
    );
    assert_eq!(
        delta.source_after.as_ref().unwrap().pointer,
        "/$defs/Thing/properties/name/minLength"
    );
    assert!(delta.source_after.as_ref().unwrap().span.is_some());
}

#[test]
fn docs_only_changes_preserve_wire_and_actual_native_operation_names() {
    let mut api = api();
    let mut models = models();
    let fixture = Fixture::new(&api, &models);
    let before = fixture.load();
    api["info"]["title"] = json!("A better guide");
    op(&mut api)["description"] = json!("Update a thing. Literal `names` and <tags> are prose.");
    models["$defs"]["Thing"]["description"] = json!("A documented model");
    models["$defs"]["Thing"]["properties"]["name"]["description"] = json!("Human-readable name");
    fixture.write(&api, &models);
    let report = compatibility::compare(before, fixture.load(), &selection(), &targets()).unwrap();
    assert!(report.wire.is_empty(), "{:?}", report.wire);
    assert!(report.is_proven_compatible(), "{:?}", report.native);
    for native in &report.native {
        let before = native.before.as_ref().unwrap();
        let after = native.after.as_ref().unwrap();
        assert_eq!(before.status, PlanStatus::Planned, "{:?}", before.findings);
        assert_eq!(after.status, PlanStatus::Planned, "{:?}", after.findings);
        let expected = match native.backend {
            Backend::TypescriptHttp | Backend::SwiftHttp => "updateThing",
            Backend::GoHttp => "UpdateThing",
            _ => "update_thing",
        };
        assert_eq!(before.operations[0].symbols["method"], expected);
        assert_eq!(before.operations[0].symbols, after.operations[0].symbols);
        assert!(before.findings.is_empty(), "{:?}", before.findings);
        assert!(after.findings.is_empty(), "{:?}", after.findings);
        assert!(native.changes.is_empty(), "{:?}", native.changes);
        assert!(before.models.iter().all(|model| model.descriptor.is_some()));
    }
}

#[test]
fn unchanged_and_relocated_sources_are_compatible_when_native_names_stay_equal() {
    let before_fixture = Fixture::new(&api(), &models());
    let after_fixture = Fixture::new(&api(), &models());
    let before = compatibility::snapshot(before_fixture.load(), &selection(), &targets()).unwrap();
    assert!(compatibility::compare_snapshots(&before, &before).is_proven_compatible());
    let after = compatibility::snapshot(after_fixture.load(), &selection(), &targets()).unwrap();
    let report = compatibility::compare_snapshots(&before, &after);
    assert_ne!(report.before.entry, report.after.entry);
    assert!(report.is_proven_compatible(), "{:?}", report.native);
    for native in &report.native {
        assert!(native.changes.is_empty(), "{:?}", native.changes);
        assert_ne!(
            native.before.as_ref().unwrap().operations[0]
                .source
                .document,
            native.after.as_ref().unwrap().operations[0].source.document
        );
    }
}

#[test]
fn relocating_a_colliding_schema_does_not_equate_different_native_names() {
    let mut api = api();
    let models = models();
    op(&mut api)["requestBody"]["content"]["application/json"]["schema"]["$ref"] =
        json!("./a.json#/$defs/Thing");
    op(&mut api)["responses"]["200"]["content"]["application/json"]["schema"]["$ref"] =
        json!("./z.json#/$defs/Thing");
    let fixture = Fixture::new(&api, &models);
    let definition =
        serde_json::to_vec(&json!({"$defs":{"Thing":models["$defs"]["Thing"]}})).unwrap();
    std::fs::write(fixture.directory.path().join("a.json"), &definition).unwrap();
    std::fs::write(fixture.directory.path().join("z.json"), &definition).unwrap();
    let before = fixture.load();
    std::fs::write(fixture.directory.path().join("zz.json"), &definition).unwrap();
    op(&mut api)["requestBody"]["content"]["application/json"]["schema"]["$ref"] =
        json!("./zz.json#/$defs/Thing");
    fixture.write(&api, &models);
    let report = compatibility::compare(before, fixture.load(), &selection(), &targets()).unwrap();
    assert!(report.wire.is_empty(), "{:?}", report.wire);
    for native in &report.native {
        let before = native
            .before
            .as_ref()
            .unwrap()
            .models
            .iter()
            .find(|model| {
                model.role != "codec"
                    && model.source.document.ends_with("/a.json")
                    && model.source.pointer == "/$defs/Thing"
            })
            .unwrap();
        let after = native
            .after
            .as_ref()
            .unwrap()
            .models
            .iter()
            .find(|model| {
                model.role != "codec"
                    && model.source.document.ends_with("/zz.json")
                    && model.source.pointer == "/$defs/Thing"
            })
            .unwrap();
        assert_ne!(
            before.name, after.name,
            "the relocated definition changes collision allocation"
        );
        assert!(
            native
                .changes
                .iter()
                .any(|change| change.code == "native-model-renamed"
                    && change.before == Some(json!(before.name))
                    && change.after == Some(json!(after.name))),
            "{:?}",
            native.changes
        );
        assert!(!native.summary.is_proven_compatible());
    }
}

#[test]
fn rust_constructor_arguments_and_default_impl_follow_planned_singleton_initializers() {
    let api = api();
    let mut models = models();
    models["$defs"]["Thing"] = json!({"type":"object","additionalProperties":false,"required":["kind"],"properties":{
        "kind":{"type":"string","const":"fixed"}, "note":{"type":["string","null"]}, "label":{"type":"string"}
    }});
    let fixture = Fixture::new(&api, &models);
    let before = fixture.load();
    models["$defs"]["Thing"]["properties"]["kind"] =
        json!({"type":"string","enum":["fixed","other"]});
    fixture.write(&api, &models);
    let report = compatibility::compare(
        before,
        fixture.load(),
        &selection(),
        &[target(Backend::RustHttp)],
    )
    .unwrap();
    let changed = report.native[0]
        .changes
        .iter()
        .find(|change| change.code == "native-model-shape-changed" && change.subject == "Thing")
        .unwrap();
    let before = changed.before.as_ref().unwrap();
    let after = changed.after.as_ref().unwrap();
    assert_eq!(before["constructor"]["parameters"], json!([]));
    assert_eq!(before["defaultAvailable"], true);
    assert_eq!(after["constructor"]["parameters"][0]["name"], "kind");
    assert_eq!(after["defaultAvailable"], false);
    let initializer = |name: &str| {
        before["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|field| field["name"] == name)
            .unwrap()["initialization"]["kind"]
            .clone()
    };
    assert_eq!(initializer("kind"), "literal");
    assert_eq!(initializer("note"), "absent");
    assert_eq!(initializer("label"), "none");
    assert!(
        report.native[0]
            .before
            .as_ref()
            .unwrap()
            .findings
            .is_empty()
    );
    assert!(report.native[0].after.as_ref().unwrap().findings.is_empty());
}

#[test]
fn python_keyword_arguments_and_status_classes_use_allocated_plan_symbols() {
    let mut api = api();
    let models = models();
    op(&mut api)["parameters"][1]["name"] = json!("body");
    let fixture = Fixture::new(&api, &models);
    let before = fixture.load();
    let mut added = op(&mut api).clone();
    added["operationId"] = json!("update-thing");
    api["paths"]["/aaa/{id}"] = json!({"patch":added});
    fixture.write(&api, &models);
    let report =
        compatibility::compare(before, fixture.load(), &[], &[target(Backend::PythonHttp)])
            .unwrap();
    let native = &report.native[0];
    let before = &native.before.as_ref().unwrap().operations[0];
    let after = native
        .after
        .as_ref()
        .unwrap()
        .operations
        .iter()
        .find(|operation| operation.operation_id == "updateThing")
        .unwrap();
    let parameter = before.descriptor["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|parameter| parameter["wire"] == "body")
        .unwrap();
    assert_eq!(parameter["member"], "body_2");
    let response = |operation: &compatibility::NativeOperation| {
        operation.descriptor["responses"]
            .as_array()
            .unwrap()
            .iter()
            .find(|response| response["status"] == 200)
            .unwrap()["class"]
            .clone()
    };
    assert_eq!(response(before), "UpdateThingStatus200");
    assert_eq!(response(after), "UpdateThingStatus200_2");
    assert!(
        native
            .changes
            .iter()
            .any(|change| change.code == "native-responses-changed"
                && change.operation_id_after.as_deref() == Some("updateThing"))
    );
    assert!(native.before.as_ref().unwrap().findings.is_empty());
    assert!(native.after.as_ref().unwrap().findings.is_empty());
}

#[test]
fn typescript_directional_members_are_compared_from_the_actual_view_expressions() {
    let api = api();
    let mut models = models();
    models["$defs"]["Thing"]["properties"]["name"]["readOnly"] = json!(true);
    models["$defs"]["Thing"]["properties"]["note"]["writeOnly"] = json!(true);
    let fixture = Fixture::new(&api, &models);
    let before = fixture.load();
    models["$defs"]["Thing"]["required"] = json!(["name", "note"]);
    fixture.write(&api, &models);
    let report = compatibility::compare(
        before,
        fixture.load(),
        &selection(),
        &[target(Backend::TypescriptHttp)],
    )
    .unwrap();
    let native = &report.native[0];
    let model = |snapshot: &compatibility::NativeSnapshot, role: &str| {
        snapshot
            .models
            .iter()
            .find(|model| model.role == role && model.source.pointer == "/$defs/Thing")
            .unwrap()
            .clone()
    };
    let before_request = model(native.before.as_ref().unwrap(), "request");
    let after_request = model(native.after.as_ref().unwrap(), "request");
    let before_response = model(native.before.as_ref().unwrap(), "response");
    let after_response = model(native.after.as_ref().unwrap(), "response");
    let required = |model: &compatibility::NativeModel| {
        model.descriptor.as_ref().unwrap()["type"]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|field| field["name"] == "note")
            .unwrap()["required"]
            .clone()
    };
    assert_eq!(required(&before_request), false);
    assert_eq!(required(&after_request), true);
    assert_eq!(required(&before_response), false);
    assert_eq!(required(&after_response), false);
    assert!(
        native
            .changes
            .iter()
            .any(|change| change.code == "native-model-shape-changed"
                && change.subject == after_request.name)
    );
    assert!(
        !native
            .changes
            .iter()
            .any(|change| change.code == "native-model-shape-changed"
                && change.subject == after_response.name)
    );
    assert!(native.before.as_ref().unwrap().findings.is_empty());
    assert!(native.after.as_ref().unwrap().findings.is_empty());
}

#[test]
fn a_wire_parameter_rename_is_not_a_native_signature_change_when_the_member_stays_equal() {
    let mut api = api();
    let models = models();
    let fixture = Fixture::new(&api, &models);
    let before = fixture.load();
    op(&mut api)["parameters"][1]["name"] = json!("user_name");
    fixture.write(&api, &models);
    let report = compatibility::compare(
        before,
        fixture.load(),
        &selection(),
        &[target(Backend::PythonHttp), target(Backend::RustHttp)],
    )
    .unwrap();
    assert!(
        report
            .wire
            .iter()
            .any(|change| change.code == "wire-parameter-removed")
    );
    for native in &report.native {
        assert!(native.changes.is_empty(), "{:?}", native.changes);
        assert!(native.summary.is_proven_compatible());
        let parameter = &native.after.as_ref().unwrap().operations[0].descriptor["parameters"][1];
        assert_eq!(parameter["member"], "user_name");
        assert_eq!(parameter["wire"], "user_name");
    }
}

#[test]
fn go_constructor_and_response_type_collisions_are_reported_from_allocated_names() {
    let api = api();
    let mut models = models();
    let fixture = Fixture::new(&api, &models);
    let before = fixture.load();
    // These reachable alias declarations occupy Go's package namespace but do
    // not collide with the input struct or the result/error interface names.
    models["$defs"]["NewUpdateThingInput"] = json!({"type":"string"});
    models["$defs"]["UpdateThingStatus200"] = json!({"type":"string"});
    models["$defs"]["Thing"]["properties"]["constructor_name"] =
        json!({"$ref":"#/$defs/NewUpdateThingInput"});
    models["$defs"]["Thing"]["properties"]["response_name"] =
        json!({"$ref":"#/$defs/UpdateThingStatus200"});
    fixture.write(&api, &models);
    let report = compatibility::compare(
        before,
        fixture.load(),
        &selection(),
        &[target(Backend::GoHttp)],
    )
    .unwrap();
    let native = &report.native[0];
    assert_eq!(
        native.after.as_ref().unwrap().status,
        PlanStatus::Planned,
        "{:?}",
        native.after.as_ref().unwrap().findings
    );
    let before = &native.before.as_ref().unwrap().operations[0];
    let after = &native.after.as_ref().unwrap().operations[0];
    assert_eq!(before.symbols["input"], "UpdateThingInput");
    assert_eq!(after.symbols["input"], "UpdateThingInput");
    assert_eq!(before.symbols["input-constructor"], "NewUpdateThingInput");
    assert_eq!(after.symbols["input-constructor"], "NewUpdateThingInput2");
    assert_eq!(
        after.descriptor["constructor"]["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .map(|argument| argument["member"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["Id", "Body"]
    );
    assert!(
        native
            .changes
            .iter()
            .any(|change| change.code == "native-operation-symbol-changed"
                && change.subject == "input-constructor"
                && change.impact == Impact::Breaking)
    );
    let response = |operation: &compatibility::NativeOperation, status: u16| {
        operation.descriptor["responses"]
            .as_array()
            .unwrap()
            .iter()
            .find(|response| response["status"] == status)
            .unwrap()
            .clone()
    };
    assert_eq!(response(before, 200)["type"], "UpdateThingStatus200");
    assert_eq!(response(after, 200)["type"], "UpdateThingStatus2002");
    assert_eq!(
        response(after, 200)["membership"]["interface"],
        "UpdateThingResult"
    );
    assert_eq!(response(after, 200)["membership"]["receiver"], "value");
    assert_eq!(response(after, 400)["type"], "UpdateThingStatus400");
    assert_eq!(
        response(after, 400)["membership"]["interface"],
        "UpdateThingApiError"
    );
    assert_eq!(response(after, 400)["membership"]["receiver"], "pointer");
    let unions = &after.descriptor["responseUnions"];
    assert_eq!(unions["success"]["privateMarker"], "isUpdateThingResult");
    assert_eq!(unions["apiError"]["privateMarker"], "isUpdateThingApiError");
    assert_eq!(unions["apiError"]["embedsError"], true);
    assert!(
        native
            .changes
            .iter()
            .any(|change| change.code == "native-responses-changed")
    );
}

#[test]
fn go_optional_parameter_and_body_setters_can_break_source_without_breaking_wire() {
    let mut api = api();
    let models = models();
    op(&mut api)["requestBody"]["required"] = json!(false);
    let fixture = Fixture::new(&api, &models);
    let before = fixture.load();
    // Go fields and methods share the input receiver's namespace. New optional
    // wire inputs can therefore displace existing fluent setters.
    op(&mut api)["parameters"].as_array_mut().unwrap().extend([
        json!({"name":"with_user_name","in":"query","schema":{"type":"string"}}),
        json!({"name":"with_body","in":"query","schema":{"type":"string"}}),
    ]);
    fixture.write(&api, &models);
    let report = compatibility::compare(
        before,
        fixture.load(),
        &selection(),
        &[target(Backend::GoHttp)],
    )
    .unwrap();
    assert!(!report.wire.is_empty());
    assert!(
        report
            .wire
            .iter()
            .all(|change| change.impact == Impact::Compatible),
        "{:?}",
        report.wire
    );
    let native = &report.native[0];
    assert_eq!(
        native.after.as_ref().unwrap().status,
        PlanStatus::Planned,
        "{:?}",
        native.after.as_ref().unwrap().findings
    );
    let before = &native.before.as_ref().unwrap().operations[0];
    let after = &native.after.as_ref().unwrap().operations[0];
    let setter = |operation: &compatibility::NativeOperation| {
        operation.descriptor["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|parameter| parameter["wire"] == "user-name")
            .unwrap()["setter"]
            .clone()
    };
    assert_eq!(setter(before), "WithUserName");
    assert_eq!(setter(after), "WithUserName2");
    assert_eq!(before.descriptor["body"]["member"], "Body");
    assert_eq!(after.descriptor["body"]["member"], "Body");
    assert_eq!(before.descriptor["body"]["setter"], "WithBody");
    assert_eq!(after.descriptor["body"]["setter"], "WithBody2");
    assert_eq!(
        after.descriptor["constructor"]["parameters"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "optional body must not become a constructor argument"
    );
    assert!(
        native
            .changes
            .iter()
            .any(|change| change.code == "native-input-changed" && change.subject == "parameters")
    );
    assert!(
        native
            .changes
            .iter()
            .any(|change| change.code == "native-input-changed" && change.subject == "body")
    );
    assert!(!native.summary.is_proven_compatible());
}

#[test]
fn nullable_widening_is_safe_for_old_requests_but_not_old_response_consumers() {
    let api = api();
    let mut models = models();
    let fixture = Fixture::new(&api, &models);
    let before = fixture.load();
    models["$defs"]["Thing"]["properties"]["name"]["type"] = json!(["string", "null"]);
    fixture.write(&api, &models);
    let report = compatibility::compare(before, fixture.load(), &selection(), &targets()).unwrap();
    assert_eq!(
        schema_impact(&report, Direction::Request),
        Impact::Compatible
    );
    assert_eq!(
        schema_impact(&report, Direction::Response),
        Impact::PotentiallyBreaking
    );
    for native in &report.native {
        let before = schema_model(native.before.as_ref().unwrap(), "/$defs/Thing");
        let after = schema_model(native.after.as_ref().unwrap(), "/$defs/Thing");
        assert!(
            native
                .changes
                .iter()
                .any(|change| change.code == "native-model-shape-changed"
                    && change.subject == after.name)
        );
        let field_type = |model: &compatibility::NativeModel| {
            let value = model.descriptor.as_ref().unwrap();
            let object = if native.backend == Backend::TypescriptHttp {
                &value["type"]
            } else {
                value
            };
            let name = if native.backend == Backend::GoHttp {
                "Name"
            } else {
                "name"
            };
            object["fields"]
                .as_array()
                .unwrap()
                .iter()
                .find(|field| field["name"] == name)
                .unwrap()["type"]
                .clone()
        };
        let after_type = field_type(after);
        if native.backend == Backend::TypescriptHttp {
            assert_eq!(after_type["kind"], "union");
            assert!(
                after_type["variants"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|variant| variant["name"] == "null")
            );
        } else {
            assert_eq!(after_type["kind"], "nullable");
        }
        assert_ne!(field_type(before), after_type);
        assert!(native.before.as_ref().unwrap().findings.is_empty());
        assert!(native.after.as_ref().unwrap().findings.is_empty());
    }
}

#[test]
fn relaxing_property_presence_has_opposite_request_and_response_variance() {
    let api = api();
    let mut models = models();
    let fixture = Fixture::new(&api, &models);
    let before = fixture.load();
    models["$defs"]["Thing"]["required"] = json!([]);
    fixture.write(&api, &models);
    let report = compatibility::compare(before, fixture.load(), &selection(), &targets()).unwrap();
    assert_eq!(
        schema_impact(&report, Direction::Request),
        Impact::Compatible
    );
    assert_eq!(
        schema_impact(&report, Direction::Response),
        Impact::PotentiallyBreaking
    );
    assert!(
        report
            .wire
            .iter()
            .flat_map(|change| &change.schema_deltas)
            .any(|delta| delta.keyword == "required")
    );
    for native in &report.native {
        let model = schema_model(native.after.as_ref().unwrap(), "/$defs/Thing");
        assert!(
            native
                .changes
                .iter()
                .any(|change| change.code == "native-model-shape-changed"
                    && change.subject == model.name)
        );
        if native.backend == Backend::SwiftHttp {
            assert_eq!(
                model.descriptor.as_ref().unwrap()["constructor"]["canInitializeWithoutArguments"],
                true
            );
        }
        assert!(native.after.as_ref().unwrap().findings.is_empty());
    }
}

#[test]
fn model_field_renames_preserve_each_languages_actual_member_spelling() {
    let api = api();
    let mut models = models();
    let fixture = Fixture::new(&api, &models);
    let before = fixture.load();
    let field = models["$defs"]["Thing"]["properties"]
        .as_object_mut()
        .unwrap()
        .remove("name")
        .unwrap();
    models["$defs"]["Thing"]["properties"]["display_name"] = field;
    models["$defs"]["Thing"]["required"] = json!(["display_name"]);
    fixture.write(&api, &models);
    let report = compatibility::compare(before, fixture.load(), &selection(), &targets()).unwrap();
    assert!(!report.wire.is_empty());
    for native in &report.native {
        let model = schema_model(native.after.as_ref().unwrap(), "/$defs/Thing");
        let descriptor = model.descriptor.as_ref().unwrap();
        let fields = if native.backend == Backend::TypescriptHttp {
            &descriptor["type"]["fields"]
        } else {
            &descriptor["fields"]
        };
        let expected = match native.backend {
            Backend::GoHttp => "DisplayName",
            Backend::SwiftHttp => "displayName",
            _ => "display_name",
        };
        assert!(
            fields
                .as_array()
                .unwrap()
                .iter()
                .any(|field| field["name"] == expected),
            "{expected}: {fields}"
        );
        assert!(
            native
                .changes
                .iter()
                .any(|change| change.code == "native-model-shape-changed"
                    && change.subject == model.name)
        );
        assert!(native.after.as_ref().unwrap().findings.is_empty());
    }
}

#[test]
fn native_union_variants_change_while_wire_exclusivity_stays_unknown() {
    let api = api();
    let mut models = models();
    models["$defs"]["Thing"] = json!({"oneOf":[{"type":"string"},{"type":"integer"}]});
    let fixture = Fixture::new(&api, &models);
    let before = fixture.load();
    models["$defs"]["Thing"]["oneOf"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"boolean"}));
    fixture.write(&api, &models);
    let report = compatibility::compare(before, fixture.load(), &selection(), &targets()).unwrap();
    assert_eq!(schema_impact(&report, Direction::Request), Impact::Unknown);
    assert_eq!(schema_impact(&report, Direction::Response), Impact::Unknown);
    for native in &report.native {
        let before = schema_model(native.before.as_ref().unwrap(), "/$defs/Thing");
        let after = schema_model(native.after.as_ref().unwrap(), "/$defs/Thing");
        let variants = |model: &compatibility::NativeModel| {
            let value = model.descriptor.as_ref().unwrap();
            let union = if matches!(
                native.backend,
                Backend::TypescriptHttp | Backend::PythonHttp
            ) {
                &value["type"]
            } else {
                value
            };
            union["variants"].as_array().unwrap().len()
        };
        assert_eq!(variants(before), 2);
        assert_eq!(variants(after), 3);
        assert!(
            native
                .changes
                .iter()
                .any(|change| change.code == "native-model-shape-changed"
                    && change.subject == after.name)
        );
        assert!(native.before.as_ref().unwrap().findings.is_empty());
        assert!(native.after.as_ref().unwrap().findings.is_empty());
    }
}

#[test]
fn swift_initializer_defaults_order_and_extra_fields_come_from_the_declaration_plan() {
    let api = api();
    let mut models = models();
    models["$defs"]["Thing"] = json!({"type":"object","required":["a_default","z_required"],"properties":{
        "a_default":{"type":"string","enum":["first"]}, "z_required":{"type":"string"}, "note":{"type":["string","null"]}
    },"additionalProperties":{"type":"string"}});
    let fixture = Fixture::new(&api, &models);
    let before = fixture.load();
    models["$defs"]["Thing"]["properties"]["a_default"]["enum"] = json!(["first", "other"]);
    fixture.write(&api, &models);
    let report = compatibility::compare(
        before,
        fixture.load(),
        &selection(),
        &[target(Backend::SwiftHttp)],
    )
    .unwrap();
    let native = &report.native[0];
    let before = schema_model(native.before.as_ref().unwrap(), "/$defs/Thing")
        .descriptor
        .as_ref()
        .unwrap();
    let after = schema_model(native.after.as_ref().unwrap(), "/$defs/Thing")
        .descriptor
        .as_ref()
        .unwrap();
    let arguments = |value: &Value| {
        value["constructor"]["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .map(|argument| argument["name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        arguments(before),
        ["zRequired", "aDefault", "note", "additionalProperties"]
    );
    assert_eq!(
        arguments(after),
        ["aDefault", "zRequired", "note", "additionalProperties"]
    );
    let defaulted = &before["constructor"]["parameters"][1];
    assert_eq!(defaulted["hasDefault"], true);
    assert_eq!(defaulted["initialization"]["kind"], "literal");
    assert_eq!(defaulted["initialization"]["model"], "ThingADefault");
    assert_eq!(defaulted["initialization"]["case"], "first");
    assert_eq!(defaulted["initialization"]["value"], "first");
    assert_eq!(after["constructor"]["parameters"][0]["hasDefault"], false);
    assert_eq!(
        before["constructor"]["parameters"][2]["type"]["kind"],
        "presence"
    );
    assert_eq!(
        before["constructor"]["parameters"][2]["initialization"]["kind"],
        "missing"
    );
    assert_eq!(
        before["constructor"]["parameters"][3]["initialization"]["kind"],
        "empty-object"
    );
    assert!(
        native
            .changes
            .iter()
            .any(|change| change.code == "native-model-shape-changed" && change.subject == "Thing")
    );
}

#[test]
fn swift_nullable_codec_and_non_null_declaration_keep_separate_source_roles() {
    let api = api();
    let mut models = models();
    let fixture = Fixture::new(&api, &models);
    let before = fixture.load();
    models["$defs"]["Thing"]["type"] = json!(["object", "null"]);
    fixture.write(&api, &models);
    let report = compatibility::compare(
        before,
        fixture.load(),
        &selection(),
        &[target(Backend::SwiftHttp)],
    )
    .unwrap();
    let native = &report.native[0];
    let before = native.before.as_ref().unwrap();
    let after = native.after.as_ref().unwrap();
    let declaration = schema_model(after, "/$defs/Thing");
    assert_eq!(
        schema_model(before, "/$defs/Thing").descriptor,
        declaration.descriptor
    );
    let codec = after
        .models
        .iter()
        .find(|record| record.role == "codec" && record.source.pointer == "/$defs/Thing")
        .unwrap();
    assert_eq!(codec.name, "Codecs.thing");
    assert_eq!(
        codec.descriptor.as_ref().unwrap()["type"]["of"]["kind"],
        "nullable"
    );
    assert_eq!(
        declaration.descriptor.as_ref().unwrap()["codec"]["type"]["of"]["kind"],
        "named"
    );
    assert!(
        native
            .changes
            .iter()
            .any(|change| change.code == "native-model-shape-changed"
                && change.subject == "Codecs.thing")
    );
    assert!(
        !native
            .changes
            .iter()
            .any(|change| change.code == "native-model-shape-changed" && change.subject == "Thing")
    );
    let response = &after.operations[0].descriptor["responses"][0];
    assert_eq!(response["case"], "status200");
    assert_eq!(response["type"]["of"]["kind"], "nullable");
    assert!(response["codec"].as_str().unwrap().starts_with("Codecs."));
}

#[test]
fn swift_component_reuse_and_reordered_model_codec_records_match_by_role() {
    let mut api = api();
    let mut models = models();
    let fixture = Fixture::new(&api, &models);
    let before =
        compatibility::snapshot(fixture.load(), &selection(), &[target(Backend::SwiftHttp)])
            .unwrap();
    let mut reordered =
        compatibility::snapshot(fixture.load(), &selection(), &[target(Backend::SwiftHttp)])
            .unwrap();
    reordered.native[0].models.reverse();
    assert!(compatibility::compare_snapshots(&before, &reordered).is_proven_compatible());
    assert_eq!(
        before.native[0]
            .models
            .iter()
            .filter(|record| record.source.pointer == "/$defs/Thing")
            .count(),
        2
    );
    // Keep Thing for responses while moving the request use to an equivalent
    // component. Both the original declaration and its codec remain exported.
    models["$defs"]["RequestThing"] = models["$defs"]["Thing"].clone();
    op(&mut api)["requestBody"]["content"]["application/json"]["schema"]["$ref"] =
        json!("./models.json#/$defs/RequestThing");
    fixture.write(&api, &models);
    let after =
        compatibility::snapshot(fixture.load(), &selection(), &[target(Backend::SwiftHttp)])
            .unwrap();
    let report = compatibility::compare_snapshots(&before, &after);
    assert!(report.wire.is_empty(), "{:?}", report.wire);
    let native = &report.native[0];
    assert!(
        !native.changes.iter().any(|change| change
            .source_before
            .as_ref()
            .is_some_and(|source| source.pointer == "/$defs/Thing")),
        "{:?}",
        native.changes
    );
    for role in ["model", "codec"] {
        assert_eq!(
            native
                .after
                .as_ref()
                .unwrap()
                .models
                .iter()
                .filter(|record| record.source.pointer == "/$defs/Thing" && record.role == role)
                .count(),
            1
        );
    }
    assert!(
        native
            .changes
            .iter()
            .any(|change| change.code == "native-input-changed" && change.subject == "body")
    );
    assert!(
        !native.summary.is_proven_compatible(),
        "wire equivalence does not equate nominal Swift types"
    );
}

#[test]
fn swift_import_defaults_and_invalid_identifiers_use_backend_package_admission() {
    let fixture = Fixture::new(&api(), &models());
    let contract = fixture.load();
    let before = target(Backend::SwiftHttp);
    let mut after = before.clone();
    after.import_name = Some(before.package_name.clone());
    let report = compatibility::compare_with_targets(
        contract.clone(),
        contract.clone(),
        &selection(),
        std::slice::from_ref(&before),
        &[after.clone()],
    )
    .unwrap();
    assert!(report.is_proven_compatible(), "{:?}", report.native);
    after.import_name = Some("RenamedSDK".into());
    let report = compatibility::compare_with_targets(
        contract.clone(),
        contract.clone(),
        &selection(),
        std::slice::from_ref(&before),
        &[after.clone()],
    )
    .unwrap();
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|change| change.code == "native-import-name-changed"
                && change.impact == Impact::Breaking)
    );
    after.import_name = Some("not-a-swift-module".into());
    let report = compatibility::compare_with_targets(
        contract.clone(),
        contract,
        &selection(),
        &[before],
        &[after],
    )
    .unwrap();
    assert_eq!(
        report.native[0].after.as_ref().unwrap().status,
        PlanStatus::Unavailable
    );
    assert!(
        report.native[0]
            .after
            .as_ref()
            .unwrap()
            .findings
            .iter()
            .any(|finding| finding.code == "native-package-invalid")
    );
    assert!(!report.is_proven_compatible());
}

#[test]
fn operation_id_renames_keep_the_route_but_change_language_specific_source_apis() {
    let mut api = api();
    let models = models();
    let fixture = Fixture::new(&api, &models);
    let before = fixture.load();
    op(&mut api)["operationId"] = json!("replaceThing");
    fixture.write(&api, &models);
    let report = compatibility::compare(before, fixture.load(), &selection(), &targets()).unwrap();
    assert!(
        report
            .wire
            .iter()
            .any(|change| change.code == "wire-operation-id-renamed"
                && change.impact == Impact::Compatible)
    );
    assert!(
        !report
            .wire
            .iter()
            .any(|change| change.code == "wire-operation-removed")
    );
    for target in &report.native {
        assert!(
            target
                .changes
                .iter()
                .any(|change| change.code == "native-operation-symbol-changed"
                    && change.impact == Impact::Breaking),
            "{:?}",
            target.changes
        );
        assert_eq!(
            target.after.as_ref().unwrap().operations[0].operation_id,
            "replaceThing"
        );
    }
}

#[test]
fn source_correspondence_reports_model_renames_without_inventing_wire_breaks() {
    let mut api = api();
    let mut models = models();
    let fixture = Fixture::new(&api, &models);
    let before = fixture.load();
    let thing = models["$defs"]
        .as_object_mut()
        .unwrap()
        .remove("Thing")
        .unwrap();
    models["$defs"]["RenamedThing"] = thing;
    op(&mut api)["requestBody"]["content"]["application/json"]["schema"]["$ref"] =
        json!("./models.json#/$defs/RenamedThing");
    op(&mut api)["responses"]["200"]["content"]["application/json"]["schema"]["$ref"] =
        json!("./models.json#/$defs/RenamedThing");
    fixture.write(&api, &models);
    let report = compatibility::compare(before, fixture.load(), &selection(), &targets()).unwrap();
    assert!(report.wire.is_empty(), "{:?}", report.wire);
    for native in &report.native {
        let (old_name, new_name) = if native.backend == Backend::TypescriptHttp {
            ("_$defs_Thing", "_$defs_RenamedThing")
        } else {
            ("Thing", "RenamedThing")
        };
        let renamed = native
            .changes
            .iter()
            .find(|change| {
                change.code == "native-model-renamed" && change.before == Some(json!(old_name))
            })
            .unwrap_or_else(|| panic!("{:?}", native.changes));
        assert_eq!(renamed.after, Some(json!(new_name)));
        assert_eq!(renamed.impact, Impact::Breaking);
        assert!(
            renamed
                .source_after
                .as_ref()
                .unwrap()
                .document
                .ends_with("models.json")
        );
    }
}

#[test]
fn a_selected_operation_removal_is_a_report_and_not_a_selection_error() {
    let mut api = api();
    let models = models();
    let fixture = Fixture::new(&api, &models);
    let before = fixture.load();
    api["paths"] = json!({});
    fixture.write(&api, &models);
    let after = fixture.load();
    let report = compatibility::compare(
        before,
        after.clone(),
        &selection(),
        &[target(Backend::GoHttp)],
    )
    .unwrap();
    assert!(
        report
            .wire
            .iter()
            .any(|change| change.code == "wire-operation-removed"
                && change.impact == Impact::Breaking)
    );
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|change| change.code == "native-operation-removed")
    );
    assert!(compatibility::compare(after.clone(), after, &["misspelled".into()], &[]).is_err());
}

#[test]
fn method_path_security_and_requiredness_changes_survive_native_plan_refusal() {
    let mut api = api();
    let models = models();
    let fixture = Fixture::new(&api, &models);
    let before = fixture.load();
    let mut operation = op(&mut api).take();
    operation["parameters"][1]["required"] = json!(true);
    api["paths"] = json!({"/widgets/{id}":{"put":operation}});
    api["security"] = json!([{"Bearer":[],"Other":[]}]);
    api["components"]["securitySchemes"]["Other"] = json!({"type":"http","scheme":"bearer"});
    fixture.write(&api, &models);
    let report = compatibility::compare(
        before,
        fixture.load(),
        &selection(),
        &[target(Backend::GoHttp)],
    )
    .unwrap();
    for code in [
        "wire-method-changed",
        "wire-path-changed",
        "wire-security-changed",
        "wire-requiredness-changed",
    ] {
        assert!(
            report.wire.iter().any(|change| change.code == code),
            "{code}: {:?}",
            report.wire
        );
    }
    let native = &report.native[0];
    assert_eq!(
        native.after.as_ref().unwrap().status,
        PlanStatus::Unavailable
    );
    assert!(
        native
            .changes
            .iter()
            .any(|change| change.impact == Impact::Unknown)
    );
    assert!(
        !native
            .changes
            .iter()
            .any(|change| change.code == "native-operation-removed")
    );
}

#[test]
fn composition_changes_and_exact_number_proofs_decline_honestly() {
    for (old_schema, new_schema, reason) in [
        (
            json!({"oneOf":[{"type":"string"},{"type":"integer"}]}),
            json!({"oneOf":[{"type":"string"},{"type":"integer"},{"type":"boolean"}]}),
            "oneOf",
        ),
        (
            json!({"type":"number","minimum":0}),
            serde_json::from_str::<Value>(r#"{"type":"number","minimum":1e400}"#).unwrap(),
            "exact decimal",
        ),
        (
            json!({"type":"string","pattern":"^a"}),
            json!({"type":"string","pattern":"^ab"}),
            "pattern",
        ),
    ] {
        let api = api();
        let mut models = models();
        models["$defs"]["Thing"] = old_schema;
        let fixture = Fixture::new(&api, &models);
        let before = fixture.load();
        models["$defs"]["Thing"] = new_schema;
        fixture.write(&api, &models);
        let report = compatibility::compare(before, fixture.load(), &selection(), &[]).unwrap();
        assert_eq!(schema_impact(&report, Direction::Request), Impact::Unknown);
        assert!(!report.is_proven_compatible());
        assert!(
            report
                .wire
                .iter()
                .flat_map(|change| &change.reasoning)
                .any(|text| text.contains(reason)),
            "{:?}",
            report.wire
        );
    }
}

#[test]
fn contradictory_literal_intersections_are_not_mistaken_for_equivalence() {
    let api = api();
    let mut models = models();
    models["$defs"]["Thing"] = json!({"const":"a"});
    let fixture = Fixture::new(&api, &models);
    let before = fixture.load();
    models["$defs"]["Thing"] = json!({"const":"a","enum":["b"]});
    fixture.write(&api, &models);
    let report = compatibility::compare(before, fixture.load(), &selection(), &[]).unwrap();
    assert_ne!(
        schema_impact(&report, Direction::Request),
        Impact::Compatible
    );
}

#[test]
fn equal_malformed_schema_keywords_are_unknown_even_in_wire_only_mode() {
    for schema in [
        json!({"type":"string","minLength":-1}),
        json!({"type":"object","required":"name"}),
        json!({"type":"string","enum":"not-an-array"}),
    ] {
        let api = api();
        let mut models = models();
        models["$defs"]["Thing"] = schema;
        let fixture = Fixture::new(&api, &models);
        let contract = fixture.load();
        let report = compatibility::compare(contract.clone(), contract, &selection(), &[]).unwrap();
        assert!(
            report
                .wire
                .iter()
                .any(|change| change.impact == Impact::Unknown),
            "{:?}",
            report.wire
        );
        assert!(!report.is_proven_compatible());
    }
}

#[test]
fn annotations_are_not_stripped_from_wire_property_names_or_literal_instance_keys() {
    for (old_schema, new_schema) in [
        (
            json!({"type":"object","properties":{"description":{"type":"string"}}}),
            json!({"type":"object","properties":{"description":{"type":"integer"}}}),
        ),
        (
            json!({"const":{"description":"old"}}),
            json!({"const":{"description":"new"}}),
        ),
    ] {
        let api = api();
        let mut models = models();
        models["$defs"]["Thing"] = old_schema;
        let fixture = Fixture::new(&api, &models);
        let before = fixture.load();
        models["$defs"]["Thing"] = new_schema;
        fixture.write(&api, &models);
        let report = compatibility::compare(before, fixture.load(), &selection(), &[]).unwrap();
        assert_ne!(
            schema_impact(&report, Direction::Request),
            Impact::Compatible
        );
    }
}

#[test]
fn recursive_graphs_terminate_and_docs_changes_do_not_expand_references() {
    let api = api();
    let mut models = models();
    models["$defs"]["Thing"]["properties"]["next"] = json!({"$ref":"#/$defs/Thing"});
    let fixture = Fixture::new(&api, &models);
    let before = fixture.load();
    models["$defs"]["Thing"]["description"] = json!("Recursive node");
    fixture.write(&api, &models);
    let report = compatibility::compare(before, fixture.load(), &selection(), &[]).unwrap();
    assert!(report.wire.is_empty(), "{:?}", report.wire);
    assert!(report.is_proven_compatible());
}

#[test]
fn response_defaults_cover_exact_statuses_and_body_removal_is_not_a_safe_narrowing() {
    let mut api = api();
    let models = models();
    let response = op(&mut api)["responses"]["200"].clone();
    op(&mut api)["responses"] = json!({"default":response});
    let fixture = Fixture::new(&api, &models);
    let before = fixture.load();
    op(&mut api)["responses"]["418"] = response;
    fixture.write(&api, &models);
    let after = fixture.load();
    let report = compatibility::compare(before, after.clone(), &selection(), &[]).unwrap();
    assert!(report.wire.is_empty(), "{:?}", report.wire);
    op(&mut api)["responses"]["418"] = json!({"description":"No body"});
    fixture.write(&api, &models);
    let report = compatibility::compare(after, fixture.load(), &selection(), &[]).unwrap();
    assert!(report.wire.iter().any(|change| change.code
        == "wire-response-content-presence-changed"
        && change.impact == Impact::PotentiallyBreaking));
}

#[test]
fn package_configuration_and_archived_native_provenance_are_source_inputs() {
    let fixture = Fixture::new(&api(), &models());
    let contract = fixture.load();
    let before = target(Backend::GoHttp);
    let mut version = before.clone();
    version.package_version = "1.0.1".into();
    let report = compatibility::compare_with_targets(
        contract.clone(),
        contract.clone(),
        &selection(),
        std::slice::from_ref(&before),
        &[version.clone()],
    )
    .unwrap();
    assert!(report.wire.is_empty());
    assert!(report.is_proven_compatible(), "{:?}", report.native);
    version.package_name = "example.com/new-sdk".into();
    let report = compatibility::compare_with_targets(
        contract.clone(),
        contract.clone(),
        &selection(),
        std::slice::from_ref(&before),
        &[version],
    )
    .unwrap();
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|change| change.code == "native-package-name-changed"
                && change.impact == Impact::Breaking)
    );
    let old = compatibility::snapshot(
        contract.clone(),
        &selection(),
        std::slice::from_ref(&before),
    )
    .unwrap();
    let mut new = compatibility::snapshot(contract, &selection(), &[before]).unwrap();
    let encoded = serde_json::to_vec(&new.native[0]).unwrap();
    new.native[0] = serde_json::from_slice(&encoded).unwrap();
    new.native[0].runtime.generator_version = "future-generator".into();
    let report = compatibility::compare_snapshots(&old, &new);
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|change| change.code == "native-runtime-provenance-changed"
                && change.impact == Impact::Unknown)
    );
    let decoded: CompatibilityReport =
        serde_json::from_slice(&serde_json::to_vec(&report).unwrap()).unwrap();
    assert_eq!(report, decoded);
    assert!(
        report.migration_notes().contains("future-generator")
            || report.migration_notes().contains("provenance")
    );
}
