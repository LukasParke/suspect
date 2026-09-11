//! Public Dart backend/session integration and typed native compatibility.
#![cfg(feature = "dart-sdk")]

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde_json::{Value, json};
use suspect_codegen::{
    backend::{self, Backend, TargetConfig},
    compatibility::{self, CompatibilityReport, Impact, NativeSnapshot, PlanStatus},
    dart_sdk::{self, DartConfig, PackageConfig},
    generation_session::{Session, SessionConfig, Stats},
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
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("api.json");
        let fixture = Self {
            _directory: directory,
            path,
        };
        fixture.write(value);
        fixture
    }
    fn write(&self, value: &Value) {
        std::fs::write(&self.path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    }
    fn load(&self) -> Arc<Contract> {
        load(&self.path)
    }
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

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::DartHttp,
        package_name: "widgets_sdk".into(),
        package_version: "1.2.3".into(),
        import_name: None,
    }
}

#[test]
fn scoped_v2_capture_matches_native_carriers_and_full_codec_policy() {
    let mut document: Value =
        serde_json::from_str(include_str!("fixtures/dart-applicators-v2.json")).unwrap();
    let fixture = Fixture::new(&document);
    let before = snapshot(fixture.load());
    let native = &before.native[0];
    assert_planned(native);
    let patch = descriptor(native, "Patch", "model");
    let mixed = descriptor(native, "MixedExtras", "model");
    for value in [patch, mixed] {
        assert_eq!(value["extraFields"]["representation"], "checked-exact-json");
        assert_eq!(value["extraFields"]["validation"], "complete-object-schema");
        assert_eq!(
            value["extraFields"]["type"],
            json!({"kind":"generic","name":"Map","arguments":[{"kind":"primitive","name":"String"},{"kind":"named","name":"JsonValue"}]})
        );
    }
    assert_eq!(argument(patch, "xN")["required"], true);
    assert_eq!(
        argument(patch, "xN")["type"],
        json!({"kind":"named","name":"JsonValue"})
    );
    let envelope = descriptor(native, "Envelope", "model");
    assert_eq!(
        field(envelope, "patch")["type"],
        json!({"kind":"nullable","of":{"kind":"named","name":"Patch"}})
    );
    assert_eq!(
        field(envelope, "optional")["type"],
        json!({"kind":"generic","name":"Presence","arguments":[{"kind":"nullable","of":{"kind":"primitive","name":"String"}}]})
    );
    assert_eq!(
        field(envelope, "flexible")["type"],
        json!({"kind":"named","name":"Flexible"})
    );
    assert_eq!(
        descriptor(native, "EnvelopeFlexible", "model")["signatureType"],
        "Flexible"
    );
    assert_eq!(
        descriptor(native, "flexibleCodec", "codec")["validation"],
        json!({"version":suspect_schema::OwnedProgram::V2_VERSION,"profile":suspect_schema::OwnedProgram::V2_PROFILE,"decode":"validate-before-conversion","encode":"revalidate-current-value","jsonNullAccepted":true})
    );
    for (operation, container) in [("scopedEcho", "Future"), ("scopedRows", "Stream")] {
        let op = native
            .operations
            .iter()
            .find(|o| o.operation_id == operation)
            .unwrap();
        assert_eq!(op.descriptor["responseUnions"]["result"]["name"], container);
    }
    // A newly reached earlier node changes program indices, not another model's
    // native identity or its public codec obligations.
    document["components"]["schemas"]["Envelope"]["properties"]["aEarlier"] =
        json!({"type":"boolean"});
    fixture.write(&document);
    let after = snapshot(fixture.load());
    assert_eq!(
        descriptor(native, "MixedExtras", "model"),
        descriptor(&after.native[0], "MixedExtras", "model")
    );
    assert_eq!(
        descriptor(native, "mixedExtrasCodec", "codec"),
        descriptor(&after.native[0], "mixedExtrasCodec", "codec")
    );
}

fn api() -> Value {
    json!({"openapi":"3.1.0","info":{"title":"Dart integration","version":"1"},
    "servers":[{"url":"https://example.test/api/v1"}],"security":[{"apiKey":[]}],
    "components":{"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}},
        "schemas":{"Thing":{"type":"object","required":["name","kind","requiredNullable"],"additionalProperties":false,
            "properties":{"name":{"type":"string"},"kind":{"type":"string","const":"widget"},
                "requiredNullable":{"type":["string","null"]},"note":{"type":["string","null"]},
                "label":{"type":"string","default":"server-owned default"}}}}},
    "paths":{
        "/things":{"get":{"operationId":"getThings","responses":{
            "200":{"description":"OK","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Thing"}}}},
            "401":{"description":"Denied","content":{"application/json":{"schema":{"type":"string"}}}}
        }}},
        "/things/{id}":{"patch":{"operationId":"updateThing",
            "parameters":[{"name":"id","in":"path","required":true,"schema":{"type":"string"}},
                {"name":"body","in":"query","schema":{"type":"string"}}],
            "requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Thing"}}}},
            "responses":{"200":{"description":"Updated","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Thing"}}}}}
        }}
    }})
}

fn rich_api() -> Value {
    let mut value = api();
    for (name, schema) in [
        ("Identity", json!({"type":"string"})),
        ("Amount", json!({"type":"number"})),
        ("Count", json!({"type":"integer"})),
        (
            "Identities",
            json!({"type":"array","items":{"$ref":"#/components/schemas/Identity"}}),
        ),
        (
            "NullableObject",
            json!({"type":["object","null"],"properties":{"text":{"type":"string"}},"additionalProperties":false}),
        ),
        ("Free", json!(true)),
        (
            "Node",
            json!({"type":"object","required":["label"],"properties":{"label":{"type":"string"},"child":{"$ref":"#/components/schemas/Node"}}}),
        ),
        (
            "Cat",
            json!({"type":"object","required":["kind","text"],"properties":{"kind":{"type":"string","const":"cat"},"text":{"type":"string"}},"additionalProperties":false}),
        ),
        (
            "Dog",
            json!({"type":"object","required":["kind","name"],"properties":{"kind":{"type":"string","const":"dog"},"name":{"type":"string"}},"additionalProperties":false}),
        ),
        (
            "Tagged",
            json!({"oneOf":[{"$ref":"#/components/schemas/Cat"},{"$ref":"#/components/schemas/Dog"}]}),
        ),
        (
            "Choice",
            json!({"oneOf":[{"type":"string"},{"type":"integer"}]}),
        ),
        (
            "Checked",
            json!({"allOf":[{"type":"object","required":["a"],"properties":{"a":{"type":"string"}}},{"type":"object","required":["b"],"properties":{"b":{"type":"number"}}}]}),
        ),
        (
            "Bag",
            json!({"type":"object","properties":{"label":{"type":"string"}},"additionalProperties":{"type":"integer"}}),
        ),
        ("Label", json!({"type":"string","enum":["one","two"]})),
    ] {
        value["components"]["schemas"][name] = schema;
        value["components"]["schemas"]["Thing"]["properties"][name.to_lowercase()] =
            json!({"$ref":format!("#/components/schemas/{name}")});
    }
    value
}

fn snapshot(contract: Arc<Contract>) -> compatibility::CompatibilitySnapshot {
    compatibility::snapshot(contract, &[], &[target()]).unwrap()
}

fn descriptor<'a>(snapshot: &'a NativeSnapshot, name: &str, role: &str) -> &'a Value {
    snapshot
        .models
        .iter()
        .find(|model| model.name == name && model.role == role)
        .unwrap_or_else(|| panic!("missing {role} {name}: {:?}", snapshot.findings))
        .descriptor
        .as_ref()
        .unwrap()
}

fn field<'a>(descriptor: &'a Value, name: &str) -> &'a Value {
    descriptor["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["name"] == name)
        .unwrap()
}

fn argument<'a>(descriptor: &'a Value, name: &str) -> &'a Value {
    descriptor["constructor"]["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["name"] == name)
        .unwrap()
}

fn assert_planned(snapshot: &NativeSnapshot) {
    assert_eq!(
        snapshot.status,
        PlanStatus::Planned,
        "{:?}",
        snapshot.findings
    );
    assert!(snapshot.findings.is_empty(), "{:?}", snapshot.findings);
    assert!(
        snapshot
            .models
            .iter()
            .all(|model| model.descriptor.is_some())
    );
}

fn compare_edit(document: &Value, edit: impl FnOnce(&mut Value)) -> CompatibilityReport {
    let fixture = Fixture::new(document);
    let before = fixture.load();
    let mut after = document.clone();
    edit(&mut after);
    fixture.write(&after);
    let report = compatibility::compare(before, fixture.load(), &[], &[target()]).unwrap();
    assert_planned(report.native[0].before.as_ref().unwrap());
    assert_planned(report.native[0].after.as_ref().unwrap());
    report
}

#[test]
fn public_backend_uses_the_dart_package_identity_and_the_actual_plan() {
    let fixture = Fixture::new(&api());
    let contract = fixture.load();
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let config = target();
    assert!(Backend::ALL.contains(&Backend::DartHttp));
    assert_eq!(config.backend.name(), "dart-http");
    assert_eq!(config.backend.artifact_directory(), "dart");
    let files = backend::generate(contract.clone(), &selected, &config).unwrap();
    let direct = dart_sdk::plan_sdk(
        contract,
        &selected,
        DartConfig {
            package: PackageConfig {
                name: config.package_name.clone(),
                version: config.package_version.clone(),
            },
            ..Default::default()
        },
    )
    .unwrap()
    .render();
    assert_eq!(files, direct);
    assert!(files.iter().all(|file| file.path.starts_with("dart/")));
    assert!(
        files
            .iter()
            .any(|file| file.path == "dart/lib/widgets_sdk.dart")
    );
    assert!(
        files
            .iter()
            .any(|file| file.path == "dart/lib/widgets_sdk_io.dart")
    );
    assert!(
        files
            .iter()
            .any(|file| file.path == "dart/example/quickstart.dart")
    );
    assert!(
        files
            .iter()
            .find(|file| file.path == "dart/pubspec.yaml")
            .unwrap()
            .content
            .contains("version: 1.2.3")
    );
}

#[test]
fn unchanged_relocated_and_doc_only_inputs_have_complete_compatible_native_records() {
    let document = rich_api();
    let first = Fixture::new(&document);
    let second = Fixture::new(&document);
    let before = snapshot(first.load());
    let after = snapshot(second.load());
    assert_planned(&before.native[0]);
    assert_planned(&after.native[0]);
    for report in [
        compatibility::compare_snapshots(&before, &before),
        compatibility::compare_snapshots(&before, &after),
    ] {
        assert!(
            report.native[0].changes.is_empty(),
            "{:?}",
            report.native[0].changes
        );
        assert!(report.is_proven_compatible(), "{:?}", report.wire);
        assert_eq!(report.native[0].summary.unknowns, 0);
    }
    let report = compare_edit(&document, |after| {
        after["info"]["description"] = json!("new prose");
        after["components"]["schemas"]["Thing"]["description"] = json!("new model prose");
        after["components"]["schemas"]["Thing"]["properties"]["label"]["default"] =
            json!("different server default");
    });
    assert!(
        report.native[0].changes.is_empty(),
        "{:?}",
        report.native[0].changes
    );
}

#[test]
fn typedefs_erased_signatures_checked_wrappers_and_codecs_are_real_declarations() {
    let fixture = Fixture::new(&rich_api());
    let snapshot = snapshot(fixture.load());
    let native = &snapshot.native[0];
    assert_planned(native);
    let id = descriptor(native, "Identity", "model");
    assert_eq!(id["kind"], "type-alias");
    assert_eq!(id["type"], json!({"kind":"primitive","name":"String"}));
    assert!(id["constructor"].is_null());
    assert_eq!(
        descriptor(native, "Amount", "model")["type"],
        json!({"kind":"named","name":"JsonNumber"})
    );
    assert_eq!(
        descriptor(native, "Count", "model")["type"],
        json!({"kind":"named","name":"JsonInteger"})
    );
    assert_eq!(
        descriptor(native, "Identities", "model")["type"],
        json!({"kind":"generic","name":"List","arguments":[{"kind":"primitive","name":"String"}]})
    );
    let thing = descriptor(native, "Thing", "model");
    assert_eq!(
        field(thing, "identity")["type"],
        json!({"kind":"generic","name":"Presence","arguments":[{"kind":"primitive","name":"String"}]})
    );
    assert_eq!(
        descriptor(native, "Checked", "model")["representation"],
        "checked-exact-json"
    );
    assert_eq!(
        descriptor(native, "Checked", "model")["constructor"]["name"],
        "Checked.fromJson"
    );
    assert_eq!(
        descriptor(native, "Checked", "model")["constructor"]["factory"],
        true
    );
    assert!(
        field(descriptor(native, "Checked", "model"), "value")["readOnly"]
            .as_bool()
            .unwrap()
    );
    let bag = descriptor(native, "Bag", "model");
    assert_eq!(
        bag["extraFields"]["type"],
        json!({"kind":"generic","name":"Map","arguments":[{"kind":"primitive","name":"String"},{"kind":"named","name":"JsonInteger"}]})
    );
    assert_eq!(
        argument(bag, "extraFields")["initialization"],
        json!({"kind":"literal","value":null})
    );
    assert_eq!(bag["extraFields"]["mutableEntries"], true);
    let codec = descriptor(native, "identityCodec", "codec");
    assert_eq!(
        codec["type"],
        json!({"kind":"generic","name":"ModelCodec","arguments":[{"kind":"primitive","name":"String"}]})
    );
    assert_eq!(
        codec["methods"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "validate",
            "decode",
            "decodeBytes",
            "fromJson",
            "toJson",
            "encode",
            "encodeBytes"
        ]
    );
    assert_eq!(
        field(descriptor(native, "NullableObject", "model"), "text")["type"]["name"],
        "Presence"
    );
    assert_eq!(
        descriptor(native, "nullableObjectCodec", "codec")["type"]["arguments"][0]["kind"],
        "nullable"
    );
    assert!(native.models.iter().all(|model| {
        !model
            .descriptor
            .as_ref()
            .unwrap()
            .to_string()
            .contains("schema_index")
    }));
}

#[test]
fn native_fields_keep_required_nullable_absent_defaults_and_fixed_getters_distinct() {
    let fixture = Fixture::new(&api());
    let snap = snapshot(fixture.load());
    let thing = descriptor(&snap.native[0], "Thing", "model");
    assert_eq!(
        field(thing, "requiredNullable")["type"],
        json!({"kind":"nullable","of":{"kind":"primitive","name":"String"}})
    );
    assert_eq!(
        field(thing, "note")["type"],
        json!({"kind":"generic","name":"Presence","arguments":[{"kind":"nullable","of":{"kind":"primitive","name":"String"}}]})
    );
    assert_eq!(argument(thing, "note")["initialization"]["kind"], "absent");
    assert_eq!(argument(thing, "label")["initialization"]["kind"], "absent");
    assert_eq!(thing["schemaDefaultsApplied"], false);
    assert_eq!(field(thing, "kind")["storage"], "getter");
    assert_eq!(field(thing, "kind")["readOnly"], true);
    assert_eq!(
        field(thing, "kind")["initialization"],
        json!({"kind":"literal","value":"widget"})
    );
    assert!(
        !thing["constructor"]["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|arg| arg["name"] == "kind")
    );

    let presence = compare_edit(&api(), |after| {
        after["components"]["schemas"]["Thing"]["required"] =
            json!(["name", "kind", "requiredNullable", "note"]);
    });
    let next = descriptor(presence.native[0].after.as_ref().unwrap(), "Thing", "model");
    assert_eq!(field(next, "note")["type"]["kind"], "nullable");
    assert_eq!(argument(next, "note")["required"], true);
    assert!(
        presence.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-model-shape-changed" && c.subject == "Thing")
    );
    let nullability = compare_edit(&api(), |after| {
        after["components"]["schemas"]["Thing"]["properties"]["note"]["type"] = json!("string");
    });
    let next = descriptor(
        nullability.native[0].after.as_ref().unwrap(),
        "Thing",
        "model",
    );
    assert_eq!(
        field(next, "note")["type"]["arguments"][0],
        json!({"kind":"primitive","name":"String"})
    );
    assert!(!nullability.native[0].summary.is_proven_compatible());
}

#[test]
fn literal_getter_changes_and_real_constructor_changes_are_both_reported() {
    let changed = compare_edit(&api(), |after| {
        after["components"]["schemas"]["Thing"]["properties"]["kind"]["const"] = json!("different");
    });
    assert!(
        changed.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-model-shape-changed" && c.subject == "Thing")
    );
    assert_eq!(
        field(
            descriptor(changed.native[0].after.as_ref().unwrap(), "Thing", "model"),
            "kind"
        )["fixed"],
        "different"
    );
    let constructor = compare_edit(&api(), |after| {
        after["components"]["schemas"]["Thing"]["properties"]["kind"]
            .as_object_mut()
            .unwrap()
            .remove("const");
    });
    let next = descriptor(
        constructor.native[0].after.as_ref().unwrap(),
        "Thing",
        "model",
    );
    assert_eq!(field(next, "kind")["readOnly"], false);
    assert_eq!(argument(next, "kind")["required"], true);
    assert_eq!(
        argument(next, "kind")["type"],
        json!({"kind":"primitive","name":"String"})
    );
    let impossible = compare_edit(&api(), |after| {
        after["components"]["schemas"]["Thing"]["properties"]["kind"]["const"] = json!(false);
    });
    assert_eq!(
        field(
            descriptor(
                impossible.native[0].after.as_ref().unwrap(),
                "Thing",
                "model"
            ),
            "kind"
        )["storage"],
        "mutable-field"
    );
}

#[test]
fn union_membership_and_allocated_wrappers_follow_the_actual_plan() {
    let report = compare_edit(&rich_api(), |after| {
        after["components"]["schemas"]["Dog"]["required"] = json!(["name"]);
    });
    let before = report.native[0].before.as_ref().unwrap();
    let after = report.native[0].after.as_ref().unwrap();
    let tagged = descriptor(before, "Tagged", "model");
    assert_eq!(tagged["direct"], true);
    assert!(tagged["constructor"].is_null());
    assert_eq!(
        tagged["variants"][0]["type"],
        json!({"kind":"named","name":"Cat"})
    );
    assert_eq!(
        descriptor(before, "Cat", "model")["implements"],
        json!(["Tagged"])
    );
    assert_eq!(descriptor(after, "Cat", "model")["implements"], json!([]));
    assert_eq!(descriptor(after, "Tagged", "model")["direct"], false);
    assert_eq!(
        descriptor(after, "Tagged", "model")["variants"][0]["name"],
        "TaggedVariant1"
    );
    let variant = descriptor(after, "TaggedVariant1", "union-variant");
    assert_eq!(variant["extends"], "Tagged");
    assert_eq!(variant["constructor"]["arguments"], "positional");
    assert_eq!(
        variant["constructor"]["parameters"][0]["type"],
        json!({"kind":"named","name":"Cat"})
    );
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-model-shape-changed" && c.subject == "Cat")
    );
    assert_eq!(
        descriptor(before, "ChoiceVariant2", "union-variant")["constructor"]["parameters"][0]["type"],
        json!({"kind":"named","name":"JsonInteger"})
    );
}

#[test]
fn direct_success_statuses_named_calls_and_credential_properties_are_captured() {
    let fixture = Fixture::new(&api());
    let snapshot = snapshot(fixture.load());
    let native = &snapshot.native[0];
    let get = native
        .operations
        .iter()
        .find(|op| op.operation_id == "getThings")
        .unwrap();
    assert_eq!(get.symbols["method"], "getThings");
    assert!(!get.symbols.contains_key("input"));
    assert_eq!(get.descriptor["call"]["emptyAllowed"], true);
    assert_eq!(get.descriptor["call"]["wireInputs"], 0);
    assert_eq!(get.descriptor["parameters"].as_array().unwrap().len(), 4);
    assert_eq!(
        get.descriptor["responseUnions"]["result"],
        json!({"kind":"generic","name":"Future","arguments":[{"kind":"named","name":"GetThingsStatus200"}]})
    );
    assert_eq!(
        get.descriptor["responseUnions"]["directSingleSuccess"],
        true
    );
    assert!(descriptor(native, "GetThingsStatus200", "success-response")["constructor"].is_null());
    assert_eq!(
        descriptor(native, "GetThingsStatus401", "error-response")["membership"]["type"],
        "GetThingsApiException"
    );
    let update = native
        .operations
        .iter()
        .find(|op| op.operation_id == "updateThing")
        .unwrap();
    let query = update.descriptor["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["wire"] == "body")
        .unwrap();
    assert_eq!(query["member"], "body2");
    assert_eq!(query["type"]["name"], "Presence");
    assert_eq!(
        update.descriptor["body"]["type"],
        json!({"kind":"named","name":"Thing"})
    );
    let credentials = descriptor(native, "Credentials", "credentials");
    assert_eq!(credentials["constructor"]["const"], true);
    assert_eq!(argument(credentials, "apiKey")["required"], false);
    assert_eq!(field(credentials, "apiKey")["readOnly"], true);
    assert_eq!(
        get.descriptor["credential"]["alternatives"][0][0]["name"],
        "apiKey"
    );
    assert_eq!(get.descriptor["credential"]["anonymous"], false);
    let multiple = compare_edit(&api(), |after| {
        after["paths"]["/things"]["get"]["responses"]["202"] =
            after["paths"]["/things"]["get"]["responses"]["200"].clone();
    });
    let next = multiple.native[0]
        .after
        .as_ref()
        .unwrap()
        .operations
        .iter()
        .find(|op| op.operation_id == "getThings")
        .unwrap();
    assert_eq!(
        next.descriptor["responseUnions"]["result"]["arguments"][0]["name"],
        "GetThingsSuccess"
    );
    assert_eq!(
        next.descriptor["responseUnions"]["directSingleSuccess"],
        false
    );
    assert!(
        multiple.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-responses-changed")
    );
}

#[test]
fn unrelated_selected_nodes_do_not_leak_program_indices_into_existing_signatures() {
    let document = rich_api();
    let fixture = Fixture::new(&document);
    let before = fixture.load();
    let selected = before
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let old_plan = dart_sdk::plan_sdk(before.clone(), &selected, Default::default()).unwrap();
    let mut next = document.clone();
    next["components"]["schemas"]["AAAdditional"] = json!({"type":"string"});
    next["paths"]["/aaa"] = json!({"get":{"operationId":"unrelated","responses":{"200":{"description":"OK","content":{"application/json":{"schema":{"$ref":"#/components/schemas/AAAdditional"}}}}}}});
    fixture.write(&next);
    let after = fixture.load();
    let selected = after
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let new_plan = dart_sdk::plan_sdk(after.clone(), &selected, Default::default()).unwrap();
    let index = |plan: &dart_sdk::Plan| {
        plan.models()
            .symbols()
            .iter()
            .find(|m| m.name == "Thing")
            .unwrap()
            .index
    };
    assert_ne!(
        index(&old_plan),
        index(&new_plan),
        "fixture must shift internal node indices"
    );
    let report = compatibility::compare(before, after, &[], &[target()]).unwrap();
    let old = report.native[0].before.as_ref().unwrap();
    let new = report.native[0].after.as_ref().unwrap();
    for model in &old.models {
        let next = new
            .models
            .iter()
            .find(|next| next.role == model.role && next.name == model.name)
            .unwrap();
        assert_eq!(
            model.descriptor, next.descriptor,
            "unstable descriptor for {}",
            model.name
        );
    }
    assert!(
        report.native[0].changes.iter().all(|change| matches!(
            change.code.as_str(),
            "native-operation-added" | "native-model-added"
        )),
        "{:?}",
        report.native[0].changes
    );
}

#[test]
fn package_version_import_and_unsupported_schema_results_are_truthful() {
    let fixture = Fixture::new(&api());
    let contract = fixture.load();
    let original = target();
    let mut changed = original.clone();
    changed.package_version = "1.2.4".into();
    let report = compatibility::compare_with_targets(
        contract.clone(),
        contract.clone(),
        &[],
        std::slice::from_ref(&original),
        &[changed.clone()],
    )
    .unwrap();
    assert_eq!(report.native[0].changes.len(), 1);
    assert_eq!(
        report.native[0].changes[0].code,
        "native-package-version-changed"
    );
    assert_eq!(report.native[0].changes[0].impact, Impact::Compatible);
    changed.package_name = "renamed_widgets".into();
    let report = compatibility::compare_with_targets(
        contract.clone(),
        contract.clone(),
        &[],
        std::slice::from_ref(&original),
        &[changed],
    )
    .unwrap();
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-package-name-changed" && c.impact == Impact::Breaking)
    );
    for import in ["", "other_library", "widgets_sdk"] {
        let mut invalid = original.clone();
        invalid.import_name = Some(import.into());
        let selected = contract
            .operations()
            .map(|op| op.source().clone())
            .collect::<Vec<_>>();
        let errors = backend::generate(contract.clone(), &selected, &invalid).unwrap_err();
        assert!(errors.iter().any(|error| error.code == "sdk-package"));
        let snap = compatibility::snapshot(contract.clone(), &[], &[invalid]).unwrap();
        assert_eq!(snap.native[0].status, PlanStatus::Unavailable);
        assert!(snap.native[0].models.is_empty());
        assert!(
            snap.native[0]
                .findings
                .iter()
                .any(|f| f.code == "sdk-package")
        );
        assert!(!compatibility::compare_snapshots(&snap, &snap).is_proven_compatible());
    }
    let mut unsupported = api();
    unsupported["components"]["schemas"]["Thing"]["properties"]["name"]["readOnly"] = json!(true);
    fixture.write(&unsupported);
    let snap = snapshot(fixture.load());
    assert_eq!(snap.native[0].status, PlanStatus::Unavailable);
    assert!(
        snap.native[0]
            .findings
            .iter()
            .any(|f| f.code == "http-directional-codec-unsupported"
                && f.source.as_ref().unwrap().pointer.ends_with("/readOnly"))
    );
}

#[test]
fn credentials_follow_allocated_properties_without_inventing_constructor_requirements() {
    let report = compare_edit(&api(), |after| {
        after["security"] = json!([{"runtimeType":[]}]);
        after["components"]["securitySchemes"] =
            json!({"runtimeType":{"type":"http","scheme":"bearer"}});
    });
    let native = &report.native[0];
    let credentials = descriptor(native.after.as_ref().unwrap(), "Credentials", "credentials");
    assert_eq!(
        field(credentials, "runtimeType2")["type"],
        json!({"kind":"nullable","of":{"kind":"primitive","name":"String"}})
    );
    assert_eq!(argument(credentials, "runtimeType2")["required"], false);
    assert!(
        native
            .changes
            .iter()
            .any(|c| c.code == "native-credentials-changed")
    );
}

#[test]
fn sessions_have_zero_warm_work_and_render_only_the_reconfigured_target() {
    let fixture = Fixture::new(&api());
    let mut config = SessionConfig {
        targets: vec![
            target(),
            TargetConfig {
                backend: Backend::TypescriptHttp,
                package_name: "widgets-js".into(),
                package_version: "1.2.3".into(),
                import_name: None,
            },
        ],
        ..Default::default()
    };
    let original = config.clone();
    let mut session = Session::new(&fixture.path, config.clone()).unwrap();
    let cold = session.generate().unwrap();
    assert_eq!(
        cold.delta,
        Stats {
            compiles: 1,
            renders: 2,
            cache_hits: 0
        }
    );
    let warm = session.generate().unwrap();
    assert_eq!(
        warm.delta,
        Stats {
            compiles: 0,
            renders: 0,
            cache_hits: 1
        }
    );
    assert!(warm.changed_paths.is_empty());
    assert!(Arc::ptr_eq(&cold.files, &warm.files));
    config.targets[0].package_name = "renamed_widgets".into();
    session.set_config(config).unwrap();
    let renamed = session.generate().unwrap();
    assert_eq!(
        renamed.delta,
        Stats {
            compiles: 0,
            renders: 1,
            cache_hits: 1
        }
    );
    assert!(Arc::ptr_eq(&cold.contract, &renamed.contract));
    assert!(!renamed.changed_paths.is_empty());
    assert!(
        renamed
            .changed_paths
            .iter()
            .all(|path| path.starts_with("dart/"))
    );
    let ts = |files: &[suspect_codegen::OutFile]| {
        files
            .iter()
            .filter(|file| file.path.starts_with("typescript/"))
            .map(|file| (file.path.clone(), file.content.clone()))
            .collect::<BTreeMap<_, _>>()
    };
    assert_eq!(ts(&cold.files), ts(&renamed.files));
    assert_eq!(
        session.generate().unwrap().delta,
        Stats {
            compiles: 0,
            renders: 0,
            cache_hits: 1
        }
    );
    session.set_config(original).unwrap();
    let reverted = session.generate().unwrap();
    assert_eq!(
        reverted.delta,
        Stats {
            compiles: 0,
            renders: 0,
            cache_hits: 1
        }
    );
    assert!(Arc::ptr_eq(&cold.files, &reverted.files));
}

#[test]
fn generation_options_are_explicit_snapshot_and_session_identity() {
    use suspect_codegen::{backend::GenerationOptions, http_protocol::CompatibilityProfile};
    let fixture = Fixture::new(&api());
    let contract = fixture.load();
    let old = compatibility::snapshot(contract.clone(), &[], &[target()]).unwrap();
    let mut options = GenerationOptions::default();
    options
        .compatibility_profiles
        .insert(CompatibilityProfile::LegacyBinaryStringV1);
    let new = compatibility::snapshot_with_options(contract, &[], &[target()], &options).unwrap();
    assert_eq!(new.native[0].generation, options);
    assert!(old.native[0].generation.compatibility_profiles.is_empty());
    let mut config = SessionConfig {
        targets: vec![target()],
        ..Default::default()
    };
    let mut session = Session::new(&fixture.path, config.clone()).unwrap();
    let cold = session.generate().unwrap();
    assert_eq!(session.generate().unwrap().delta.renders, 0);
    config.generation = options;
    session.set_config(config).unwrap();
    let changed = session.generate().unwrap();
    assert_eq!((changed.delta.compiles, changed.delta.renders), (0, 1));
    assert_ne!(cold.revision, changed.revision);
    assert!(Arc::ptr_eq(&cold.contract, &changed.contract));
    assert_eq!(session.generate().unwrap().delta.renders, 0);
}
