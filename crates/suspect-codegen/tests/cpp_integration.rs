//! Public C++ backend/session/compatibility gates over the real native plans.
#![cfg(feature = "cpp-sdk")]

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde_json::{Value, json};
use suspect_codegen::http_protocol::CompatibilityProfile;
use suspect_codegen::{
    backend::{self, Backend, GenerationOptions, TargetConfig},
    compatibility::{self, CompatibilitySnapshot, Impact, NativeModel, NativeSnapshot, PlanStatus},
    cpp_sdk::{self, SdkConfig},
    generation_session::{Session, SessionConfig},
};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

struct Fixture {
    directory: tempfile::TempDir,
    entry: PathBuf,
}

impl Fixture {
    fn new(document: &Value) -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-cpp-integration");
        std::fs::create_dir_all(&root).unwrap();
        let directory = tempfile::tempdir_in(root).unwrap();
        let fixture = Self {
            entry: directory.path().join("api.json"),
            directory,
        };
        fixture.write(document);
        fixture
    }
    fn write(&self, document: &Value) {
        std::fs::write(&self.entry, serde_json::to_vec_pretty(document).unwrap()).unwrap();
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

fn document() -> Value {
    let value = json!({"application/json":{"schema":{"$ref":"#/components/schemas/Thing"}}});
    let failure = json!({"application/json":{"schema":{"$ref":"#/components/schemas/Failure"}}});
    json!({
        "openapi":"3.1.0","info":{"title":"C++ integration","version":"1"},
        "servers":[{"url":"https://example.test/api/v1"}],"security":[{"api-key":[]}],
        "components":{
            "securitySchemes":{"api-key":{"type":"http","scheme":"bearer"}},
            "schemas":{
                "Thing":{"type":"object","required":["fingerprint","kind","name"],"properties":{
                    "amount":{"type":"number"},"count":{"type":"integer"},"description":{"type":"string"},
                    "fingerprint":{"type":["string","null"]},"kind":{"type":"string","const":"fixed"},
                    "name":{"type":"string","minLength":1},"next":{"$ref":"#/components/schemas/Thing"},
                    "note":{"type":["string","null"]},
                    "payload":{"oneOf":[{"type":"string"},{"type":"integer"}]}
                },"additionalProperties":{"type":"integer"}},
                "Leaf":{"type":"object","required":["label"],"properties":{"label":{"type":"string"}},"additionalProperties":false},
                "Failure":{"type":"object","required":["message"],"properties":{"message":{"type":"string"}}}
            }
        },
        "paths":{
            "/things":{"get":{"operationId":"readThing","responses":{
                "200":{"description":"Read","content":value},"404":{"description":"Missing","content":failure}
            }}},
            "/things/{id}":{"patch":{"operationId":"updateThing",
                "parameters":[{"name":"id","in":"path","required":true,"schema":{"type":"string"}},
                    {"name":"user-name","in":"query","schema":{"type":"string"}}],
                "requestBody":{"required":true,"content":value},
                "responses":{"200":{"description":"Updated","content":value},"400":{"description":"Invalid","content":failure}}
            }}
        }
    })
}

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::CppHttp,
        package_name: "integration_sdk".into(),
        package_version: "1.0.0".into(),
        import_name: Some("integration::sdk".into()),
    }
}
fn selected(contract: &Contract) -> Vec<SourceId> {
    contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect()
}
fn snapshot(contract: Arc<Contract>, target: &TargetConfig) -> CompatibilitySnapshot {
    let snapshot = compatibility::snapshot(contract, &[], std::slice::from_ref(target)).unwrap();
    assert_eq!(snapshot.native.len(), 1);
    assert_eq!(
        snapshot.native[0].status,
        PlanStatus::Planned,
        "{:?}",
        snapshot.native[0].findings
    );
    assert!(
        snapshot.native[0].findings.is_empty(),
        "{:?}",
        snapshot.native[0].findings
    );
    snapshot
}
fn model<'a>(snapshot: &'a NativeSnapshot, pointer: &str, role: &str) -> &'a NativeModel {
    snapshot
        .models
        .iter()
        .find(|model| model.source.pointer == pointer && model.role == role)
        .unwrap_or_else(|| panic!("missing {role} at {pointer}"))
}
fn declaration(snapshot: &NativeSnapshot) -> &Value {
    model(snapshot, "/components/schemas/Thing", "model")
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

#[test]
fn public_backend_is_the_same_cpp_plan_with_the_configured_package_identity() {
    let fixture = Fixture::new(&document());
    let contract = fixture.load();
    let target = target();
    let selected = selected(&contract);
    assert!(Backend::ALL.contains(&Backend::CppHttp));
    assert_eq!(Backend::CppHttp.name(), "cpp-http");
    assert_eq!(Backend::CppHttp.artifact_directory(), "cpp");
    assert_eq!(
        serde_json::from_value::<TargetConfig>(serde_json::to_value(&target).unwrap()).unwrap(),
        target
    );
    let files = backend::generate(contract.clone(), &selected, &target).unwrap();
    let direct = cpp_sdk::plan_sdk(
        contract,
        &selected,
        SdkConfig {
            name: target.package_name.clone(),
            version: target.package_version.clone(),
            namespace: target.import_name.clone().unwrap(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(files, direct.render().unwrap());
    assert!(files.iter().all(|file| file.path.starts_with("cpp/")));
    for path in [
        "cpp/CMakeLists.txt",
        "cpp/cmake/integration_sdkConfig.cmake.in",
        "cpp/include/integration_sdk/sdk.hpp",
        "cpp/src/curl_transport.cpp",
        "cpp/Doxyfile",
        "cpp/examples/client.cpp",
    ] {
        assert!(
            files.iter().any(|file| file.path == path),
            "missing public artifact {path}"
        );
    }
    let manifest: Value = serde_json::from_str(
        &files
            .iter()
            .find(|file| file.path == "cpp/sdk-manifest.json")
            .unwrap()
            .content,
    )
    .unwrap();
    assert_eq!(manifest["package"], "integration_sdk");
    assert_eq!(manifest["namespace"], "integration::sdk");
}

#[test]
fn explicit_generation_profile_drives_the_public_backend_and_native_snapshot() {
    let document = json!({"openapi":"3.1.2","info":{"title":"Explicit binary profile","version":"1"},"servers":[{"url":"https://example.test"}],"paths":{"/file":{"get":{"operationId":"file","responses":{"200":{"description":"Raw bytes","content":{"application/octet-stream":{"schema":{"type":"string","format":"binary"}}}}}}}}});
    let fixture = Fixture::new(&document);
    let contract = fixture.load();
    let selected = selected(&contract);
    let target = target();
    let standard = GenerationOptions::default();
    assert!(
        backend::generate_with_options(contract.clone(), &selected, &target, &standard).is_err()
    );
    let unavailable = compatibility::snapshot_with_options(
        contract.clone(),
        &[],
        std::slice::from_ref(&target),
        &standard,
    )
    .unwrap();
    assert_eq!(unavailable.native[0].status, PlanStatus::Unavailable);
    assert!(
        unavailable.native[0]
            .generation
            .compatibility_profiles
            .is_empty()
    );
    let legacy = GenerationOptions {
        compatibility_profiles: BTreeSet::from([CompatibilityProfile::LegacyBinaryStringV1]),
        ..Default::default()
    };
    let files =
        backend::generate_with_options(contract.clone(), &selected, &target, &legacy).unwrap();
    let captured = compatibility::snapshot_with_options(
        contract.clone(),
        &[],
        std::slice::from_ref(&target),
        &legacy,
    )
    .unwrap();
    assert_eq!(captured.native[0].status, PlanStatus::Planned);
    assert_eq!(captured.native[0].generation, legacy);
    let response = &captured.native[0].operations[0].descriptor["responses"][0];
    assert_eq!(response["binding"]["type"]["kind"], "bytes");
    assert!(response["binding"]["codec"].is_null());
    let direct = cpp_sdk::plan_sdk(
        contract,
        &selected,
        SdkConfig {
            name: target.package_name.clone(),
            version: target.package_version.clone(),
            namespace: target.import_name.clone().unwrap(),
            legacy_binary_strings: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(files, direct.render().unwrap());
    let serialized = serde_json::to_value(&captured.native[0]).unwrap();
    let restored: NativeSnapshot = serde_json::from_value(serialized).unwrap();
    assert_eq!(restored.generation, legacy);
}

#[test]
fn protocol_snapshots_describe_true_bytes_parts_headers_and_stream_ownership() {
    let document: Value =
        serde_json::from_str(include_str!("../src/cpp_sdk/tests/protocol.openapi.json")).unwrap();
    let fixture = Fixture::new(&document);
    let contract = fixture.load();
    let target = target();
    let captured = snapshot(contract.clone(), &target);
    let plan = cpp_sdk::plan_sdk(
        contract.clone(),
        &selected(&contract),
        SdkConfig {
            name: target.package_name.clone(),
            version: target.package_version.clone(),
            namespace: target.import_name.clone().unwrap(),
            ..Default::default()
        },
    )
    .unwrap();
    let native = &captured.native[0];
    for operation in plan.operations() {
        let descriptor = &native
            .operations
            .iter()
            .find(|o| o.source.pointer == operation.source.pointer())
            .unwrap()
            .descriptor;
        for response in &operation.responses {
            for case in &response.cases {
                let record = descriptor["responses"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|v| v["name"] == format!("::integration::sdk::{}", case.variant_type))
                    .unwrap();
                assert_eq!(record["successMember"], response.can_succeed());
                assert_eq!(record["errorMember"], response.can_fail());
                if matches!(case.value.kind, cpp_sdk::ValueKind::Stream { .. }) {
                    assert_eq!(record["valueSemantics"]["copy"], false);
                    assert_eq!(record["valueSemantics"]["rangeBreakCloses"], true);
                }
            }
        }
    }
    let aggregate = plan.aggregates().iter().find(|a| a.multipart).unwrap();
    let captured_model = model(native, aggregate.source.pointer(), "aggregate")
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(captured_model["binaryJsonStandIn"], false);
    for part in aggregate
        .fields
        .iter()
        .filter(|part| part.part_type.is_some())
    {
        assert!(native.models.iter().any(|m| m.role == "part"
            && m.name == format!("::integration::sdk::{}", part.part_type.as_ref().unwrap())));
    }
    assert!(native.models.iter().any(|m| m.role == "response-headers"));
}

#[test]
fn scoped_cpp_native_records_keep_pattern_maps_fields_and_checked_carriers() {
    let document: Value =
        serde_json::from_str(include_str!("../src/cpp_sdk/tests/scoped.openapi.json")).unwrap();
    let fixture = Fixture::new(&document);
    let captured = snapshot(fixture.load(), &target());
    let native = &captured.native[0];
    let ledger = model(native, "/components/schemas/Ledger", "model")
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(ledger["kind"], "struct");
    assert_eq!(
        field(ledger, "count")["type"]["name"],
        "::integration::sdk::JsonInteger"
    );
    assert_eq!(
        ledger["additionalProperties"]["wholeObjectPatternValidation"],
        true
    );
    assert_eq!(
        ledger["additionalProperties"]["type"]["of"]["name"],
        "::integration::sdk::JsonValue"
    );
    for name in ["Choice", "Sequence"] {
        let descriptor = model(native, &format!("/components/schemas/{name}"), "model")
            .descriptor
            .as_ref()
            .unwrap();
        assert_eq!(descriptor["type"]["kind"], "validated-json");
        assert_eq!(descriptor["type"]["codecRequired"], true);
        let codec = model(native, &format!("/components/schemas/{name}"), "codec")
            .descriptor
            .as_ref()
            .unwrap();
        assert_eq!(codec["revalidatesMutableEncodes"], true);
    }
    let record = model(native, "/components/schemas/Record", "model")
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(
        record["constructor"]["parameters"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(
        native
            .operations
            .iter()
            .any(|op| op.operation_id == "items")
    );
}

#[test]
fn resource_cpp_native_records_do_not_type_dynamic_values_as_fallbacks() {
    let document: Value =
        serde_json::from_str(include_str!("../src/cpp_sdk/tests/resources.openapi.json")).unwrap();
    let fixture = Fixture::new(&document);
    let captured = snapshot(fixture.load(), &target());
    let native = &captured.native[0];
    for pointer in [
        "/components/schemas/Tree/properties/children/items",
        "/components/schemas/Switch/properties/value",
        "/components/schemas/Detached/$defs/start",
    ] {
        let descriptor = model(native, pointer, "model").descriptor.as_ref().unwrap();
        assert_eq!(descriptor["type"]["kind"], "validated-json");
        assert_eq!(descriptor["type"]["codecRequired"], true);
        assert_eq!(descriptor["cppType"], "JsonValue");
        let codec = model(native, pointer, "codec").descriptor.as_ref().unwrap();
        assert_eq!(codec["sourceBound"], true);
        assert_eq!(codec["revalidatesMutableEncodes"], true);
    }
    let tree = model(native, "/components/schemas/Tree", "model")
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(tree["kind"], "struct");
    assert_eq!(
        field(tree, "data")["type"]["name"],
        "::integration::sdk::JsonInteger"
    );
    assert!(
        native
            .models
            .iter()
            .all(|m| m.source.document.starts_with("file:"))
    );
    assert!(
        native
            .operations
            .iter()
            .any(|op| op.operation_id == "strict")
    );
}

#[test]
fn unchanged_capture_is_complete_native_value_data_without_program_indices() {
    let fixture = Fixture::new(&document());
    let contract = fixture.load();
    let settings = target();
    let captured = snapshot(contract.clone(), &settings);
    let native = &captured.native[0];
    let again = snapshot(contract.clone(), &settings);
    assert_eq!(native, &again.native[0]);
    let report = compatibility::compare_snapshots(&captured, &again);
    assert!(report.wire.is_empty());
    assert!(
        report.native[0].changes.is_empty(),
        "{:?}",
        report.native[0].changes
    );
    assert!(report.is_proven_compatible());
    assert!(native.models.iter().all(|model| model.descriptor.is_some()));
    let thing = declaration(native);
    assert_eq!(thing["kind"], "struct");
    assert_eq!(thing["cppType"], "::integration::sdk::Thing");
    assert_eq!(thing["include"], "integration_sdk/models.hpp");
    assert_eq!(thing["constructor"]["name"], "Thing");
    assert_eq!(
        thing["constructor"]["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["member"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["fingerprint", "name"]
    );
    assert_eq!(thing["constructor"]["parameters"][0]["name"], "arg0");
    assert_eq!(
        thing["constructor"]["parameters"][0]["cppType"],
        "Nullable<std::string>"
    );
    assert_eq!(thing["constructor"]["canInitializeWithoutArguments"], false);
    assert_eq!(field(thing, "fingerprint")["type"]["kind"], "nullable");
    assert_eq!(field(thing, "note")["type"]["kind"], "presence");
    assert_eq!(field(thing, "note")["type"]["of"]["kind"], "nullable");
    assert_eq!(
        field(thing, "description")["type"]["of"]["name"],
        "std::string"
    );
    assert_eq!(field(thing, "kind")["initialization"]["kind"], "literal");
    assert_eq!(
        field(thing, "kind")["initialization"]["model"],
        "::integration::sdk::ThingKind"
    );
    assert_eq!(field(thing, "kind")["initialization"]["case"], "Fixed");
    assert_eq!(field(thing, "kind")["initialization"]["value"], "fixed");
    assert_eq!(field(thing, "kind")["constructorParameter"], false);
    let boxed = &field(thing, "next")["type"]["of"];
    assert_eq!(boxed["kind"], "box");
    assert_eq!(boxed["copy"], "deep");
    assert_eq!(boxed["ownership"], "unique");
    assert_eq!(boxed["move"], "transfer-ownership");
    assert_eq!(boxed["destruction"], "raii");
    assert_eq!(boxed["encodeEmpty"], "model-error");
    assert_eq!(boxed["of"]["name"], "::integration::sdk::Thing");
    assert_eq!(thing["additionalProperties"]["name"], "extra");
    assert_eq!(
        thing["additionalProperties"]["type"]["of"]["name"],
        "::integration::sdk::JsonInteger"
    );
    let payload = model(
        native,
        "/components/schemas/Thing/properties/payload",
        "model",
    )
    .descriptor
    .as_ref()
    .unwrap();
    assert_eq!(payload["kind"], "union");
    assert_eq!(payload["storage"], "std::variant");
    assert_eq!(payload["variants"][0]["factory"], "alternative_0");
    assert_eq!(payload["variants"][1]["alias"], "Alternative1");
    assert_eq!(payload["validation"]["selectedArm"], true);
    assert_eq!(payload["validation"]["parent"], true);
    assert_eq!(
        payload["constructor"]["canInitializeWithoutArguments"],
        false
    );

    let plan = cpp_sdk::plan_sdk(
        contract.clone(),
        &selected(&contract),
        SdkConfig {
            name: settings.package_name,
            version: settings.package_version,
            namespace: settings.import_name.unwrap(),
            ..Default::default()
        },
    )
    .unwrap();
    for symbol in plan.models().symbols() {
        let codec = model(native, symbol.source.pointer(), "codec");
        let descriptor = codec.descriptor.as_ref().unwrap();
        let owner = symbol.codec();
        let qualified = format!("::integration::sdk::{}", owner.owner);
        assert_eq!(codec.name, qualified);
        assert_eq!(descriptor["owner"], qualified);
        assert_eq!(descriptor["valueAlias"]["name"], owner.value_alias);
        assert_eq!(descriptor["valueAlias"]["cppType"], owner.cpp_type);
        let names = descriptor["methods"]
            .as_array()
            .unwrap()
            .iter()
            .map(|method| {
                assert_eq!(method["owner"], qualified);
                assert_eq!(method["static"], true);
                method["name"].as_str().unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                owner.methods.decode,
                owner.methods.encode,
                owner.methods.to_json
            ]
        );
        assert_eq!(descriptor["revalidatesMutableEncodes"], true);
    }
    fn stable(value: &Value) {
        match value {
            Value::Array(values) => values.iter().for_each(stable),
            Value::Object(values) => {
                for (key, value) in values {
                    assert!(
                        ![
                            "index",
                            "schemaIndex",
                            "schema_index",
                            "targetIndex",
                            "nodeIndex",
                            "programIndex"
                        ]
                        .contains(&key.as_str()),
                        "unstable private index: {key}"
                    );
                    stable(value);
                }
            }
            _ => {}
        }
    }
    for model in &native.models {
        stable(model.descriptor.as_ref().unwrap());
    }
    for operation in &native.operations {
        stable(&operation.descriptor);
    }
}

#[test]
fn operations_preserve_positional_constructors_default_calls_and_typed_variants() {
    let fixture = Fixture::new(&document());
    let captured = snapshot(fixture.load(), &target());
    let native = &captured.native[0];
    let read = native
        .operations
        .iter()
        .find(|op| op.operation_id == "readThing")
        .unwrap();
    assert_eq!(read.symbols["method"], "read_thing");
    assert_eq!(read.symbols["client"], "::integration::sdk::Client");
    assert_eq!(read.symbols["include"], "integration_sdk/client.hpp");
    assert_eq!(read.symbols["entrypoint"], "integration_sdk/sdk.hpp");
    assert_eq!(
        read.symbols["cmake-target"],
        "integration_sdk::integration_sdk"
    );
    assert_eq!(read.descriptor["constructor"]["parameters"], json!([]));
    assert_eq!(
        read.descriptor["constructor"]["call"]["parameters"][0]["hasDefault"],
        true
    );
    let success = &read.descriptor["responseUnions"]["success"];
    assert_eq!(success["kind"], "variant");
    assert_eq!(success["storage"], "std::variant");
    assert_eq!(
        success["alternatives"],
        json!(["::integration::sdk::ReadThingStatus200"])
    );
    assert_eq!(success["singleAlternativeUnwrapped"], false);
    assert_eq!(
        read.descriptor["responseUnions"]["error"]["alternatives"],
        json!([
            "::integration::sdk::SdkError",
            "::integration::sdk::ReadThingStatus404"
        ])
    );
    assert_eq!(
        read.descriptor["responseUnions"]["result"]["name"],
        "::integration::sdk::Result"
    );
    assert_eq!(
        read.descriptor["responses"][0]["mediaType"],
        "application/json"
    );
    assert_eq!(
        read.descriptor["responses"][0]["binding"]["type"]["name"],
        "::integration::sdk::Thing"
    );
    assert_eq!(
        read.descriptor["responses"][1]["binding"]["model"],
        "::integration::sdk::ReadThingStatus404Body"
    );
    assert_eq!(
        read.descriptor["credential"]["fields"][0]["name"],
        "api_key"
    );
    assert_eq!(
        read.descriptor["credential"]["fields"][0]["wire"],
        "api-key"
    );
    assert_eq!(
        read.descriptor["credential"]["alternatives"][0]["all"][0],
        "api_key"
    );
    let update = native
        .operations
        .iter()
        .find(|op| op.operation_id == "updateThing")
        .unwrap();
    let parameters = update.descriptor["constructor"]["parameters"]
        .as_array()
        .unwrap();
    assert_eq!(
        parameters
            .iter()
            .map(|arg| arg["member"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["id", "body"]
    );
    assert_eq!(
        update.descriptor["constructor"]["call"]["parameters"][0]["hasDefault"],
        false
    );
    assert_eq!(
        update.descriptor["parameters"][1]["type"]["kind"],
        "presence"
    );
    assert_eq!(
        update.descriptor["body"]["media"][0]["mediaType"],
        "application/json"
    );
    assert_eq!(
        update.descriptor["body"]["cppType"],
        "::integration::sdk::Thing"
    );
}

#[test]
fn nullable_public_alias_and_non_null_declaration_keep_the_correct_codec_owner() {
    let mut document = document();
    let fixture = Fixture::new(&document);
    let before = snapshot(fixture.load(), &target());
    document["components"]["schemas"]["Thing"]["type"] = json!(["object", "null"]);
    fixture.write(&document);
    let after = snapshot(fixture.load(), &target());
    let native = &after.native[0];
    let value = model(native, "/components/schemas/Thing", "model");
    let descriptor = value.descriptor.as_ref().unwrap();
    assert_eq!(value.name, "::integration::sdk::Thing");
    assert_eq!(descriptor["kind"], "alias");
    assert_eq!(descriptor["type"]["kind"], "nullable");
    assert_eq!(
        descriptor["type"]["of"]["name"],
        "::integration::sdk::ThingValue"
    );
    let definition = model(native, "/components/schemas/Thing", "non-null-value");
    assert_eq!(definition.name, "::integration::sdk::ThingValue");
    assert_eq!(
        definition.descriptor.as_ref().unwrap()["constructor"]["name"],
        "ThingValue"
    );
    assert_eq!(
        field(definition.descriptor.as_ref().unwrap(), "next")["type"]["of"]["of"]["kind"],
        "nullable"
    );
    let codec = model(native, "/components/schemas/Thing", "codec");
    assert_eq!(codec.name, "::integration::sdk::ThingCodec");
    assert_eq!(
        codec.descriptor.as_ref().unwrap()["valueAlias"]["type"]["kind"],
        "nullable"
    );
    assert!(
        !native
            .models
            .iter()
            .any(|model| model.role == "codec"
                && model.name == "::integration::sdk::ThingValueCodec")
    );
    let report = compatibility::compare_snapshots(&before, &after);
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-model-shape-changed" && c.subject == value.name)
    );
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-model-shape-changed" && c.subject == codec.name)
    );
}

#[test]
fn required_nullable_and_singleton_tag_changes_update_the_actual_constructor_contract() {
    for kind in ["required", "tag-value", "tag-domain"] {
        let mut document = document();
        let fixture = Fixture::new(&document);
        let before = snapshot(fixture.load(), &target());
        let thing = &mut document["components"]["schemas"]["Thing"];
        match kind {
            "required" => thing["required"]
                .as_array_mut()
                .unwrap()
                .push(json!("note")),
            "tag-value" => thing["properties"]["kind"]["const"] = json!("other"),
            "tag-domain" => {
                thing["properties"]["kind"] = json!({"type":"string","enum":["fixed","other"]})
            }
            _ => unreachable!(),
        }
        fixture.write(&document);
        let after = snapshot(fixture.load(), &target());
        let old = declaration(&before.native[0]);
        let new = declaration(&after.native[0]);
        match kind {
            "required" => {
                assert_eq!(field(old, "note")["type"]["kind"], "presence");
                assert_eq!(field(new, "note")["type"]["kind"], "nullable");
                assert_eq!(field(new, "note")["initialization"]["kind"], "argument");
                assert_eq!(new["constructor"]["parameters"][2]["member"], "note");
            }
            "tag-value" => {
                assert_eq!(
                    old["constructor"]["parameters"],
                    new["constructor"]["parameters"]
                );
                assert_eq!(field(new, "kind")["initialization"]["case"], "Other");
                assert_eq!(field(new, "kind")["initialization"]["value"], "other");
            }
            "tag-domain" => {
                assert_eq!(field(new, "kind")["initialization"]["kind"], "argument");
                assert_eq!(new["constructor"]["parameters"][1]["member"], "kind");
                assert_eq!(new["constructor"]["parameters"][1]["hasDefault"], false);
            }
            _ => unreachable!(),
        }
        let report = compatibility::compare_snapshots(&before, &after);
        assert!(
            report.native[0]
                .changes
                .iter()
                .any(|c| c.code == "native-model-shape-changed"
                    && c.subject == "::integration::sdk::Thing"),
            "{kind}: {:?}",
            report.native[0].changes
        );
    }
}

#[test]
fn singleton_only_required_members_allow_the_emitted_default_constructor() {
    let mut document = document();
    document["components"]["schemas"]["Thing"]["required"] = json!(["kind"]);
    let fixture = Fixture::new(&document);
    let captured = snapshot(fixture.load(), &target());
    let thing = declaration(&captured.native[0]);
    assert_eq!(thing["constructor"]["parameters"], json!([]));
    assert_eq!(thing["constructor"]["defaulted"], true);
    assert_eq!(thing["constructor"]["explicit"], false);
    assert_eq!(thing["constructor"]["canInitializeWithoutArguments"], true);
    assert_eq!(field(thing, "kind")["initialization"]["value"], "fixed");
    assert_eq!(field(thing, "name")["initialization"]["kind"], "absent");
}

#[test]
fn codec_collisions_use_the_allocated_owner_instead_of_a_model_name_suffix() {
    let mut document = document();
    document["components"]["schemas"]["!ThingCodec"] = json!({"type":"string"});
    document["components"]["schemas"]["Thing"]["properties"]["reserved"] =
        json!({"$ref":"#/components/schemas/!ThingCodec"});
    let fixture = Fixture::new(&document);
    let captured = snapshot(fixture.load(), &target());
    let codec = model(&captured.native[0], "/components/schemas/Thing", "codec");
    assert_eq!(codec.name, "::integration::sdk::ThingCodec2");
    for method in codec.descriptor.as_ref().unwrap()["methods"]
        .as_array()
        .unwrap()
    {
        assert_eq!(method["owner"], codec.name);
    }
}

#[test]
fn recursive_layout_changes_do_not_hide_the_loss_of_box_ownership() {
    let mut document = document();
    let fixture = Fixture::new(&document);
    let before = snapshot(fixture.load(), &target());
    document["components"]["schemas"]["Thing"]["properties"]["next"]["$ref"] =
        json!("#/components/schemas/Leaf");
    fixture.write(&document);
    let after = snapshot(fixture.load(), &target());
    assert_eq!(
        field(declaration(&before.native[0]), "next")["type"]["of"]["kind"],
        "box"
    );
    assert_eq!(
        field(declaration(&after.native[0]), "next")["type"]["of"]["kind"],
        "named"
    );
    assert_eq!(
        field(declaration(&after.native[0]), "next")["cppType"],
        "Presence<::integration::sdk::Leaf>"
    );
    let report = compatibility::compare_snapshots(&before, &after);
    assert!(report.native[0].changes.iter().any(
        |c| c.code == "native-model-shape-changed" && c.subject == "::integration::sdk::Thing"
    ));
}

#[test]
fn archived_mutability_box_copy_and_codec_signatures_are_real_native_changes() {
    let fixture = Fixture::new(&document());
    let contract = fixture.load();
    let before = snapshot(contract.clone(), &target());
    for kind in ["mutable", "copy", "codec"] {
        let mut after = snapshot(contract.clone(), &target());
        let record = after.native[0]
            .models
            .iter_mut()
            .find(|record| {
                record.source.pointer == "/components/schemas/Thing"
                    && record.role == if kind == "codec" { "codec" } else { "model" }
            })
            .unwrap();
        let descriptor = record.descriptor.as_mut().unwrap();
        match kind {
            "mutable" => {
                descriptor["fields"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|field| field["name"] == "name")
                    .unwrap()["mutable"] = json!(false)
            }
            "copy" => {
                descriptor["fields"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|field| field["name"] == "next")
                    .unwrap()["type"]["of"]["copy"] = json!("shared")
            }
            "codec" => descriptor["methods"][1]["parameters"][0]["passing"] = json!("value"),
            _ => unreachable!(),
        }
        let report = compatibility::compare_snapshots(&before, &after);
        assert!(report.wire.is_empty());
        assert!(
            report.native[0]
                .changes
                .iter()
                .any(|change| change.code == "native-model-shape-changed"),
            "{kind}"
        );
        assert!(!report.native[0].summary.is_proven_compatible());
    }
}

#[test]
fn namespace_include_and_credentials_use_effective_public_identities() {
    let fixture = Fixture::new(&document());
    let contract = fixture.load();
    let mut default = target();
    default.import_name = None;
    let mut explicit = default.clone();
    explicit.import_name = Some(default.package_name.clone());
    let same = compatibility::compare_with_targets(
        contract.clone(),
        contract.clone(),
        &[],
        std::slice::from_ref(&default),
        std::slice::from_ref(&explicit),
    )
    .unwrap();
    assert!(same.native[0].changes.is_empty());
    let mut renamed = explicit.clone();
    renamed.import_name = Some("other::sdk".into());
    let changed = compatibility::compare_with_targets(
        contract.clone(),
        contract.clone(),
        &[],
        std::slice::from_ref(&explicit),
        std::slice::from_ref(&renamed),
    )
    .unwrap();
    assert!(changed.wire.is_empty());
    assert!(
        changed.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-operation-symbol-changed"
                && c.subject == "namespace"
                && c.impact == Impact::Breaking)
    );
    assert!(
        changed.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-model-renamed")
    );
    renamed.package_name = "other_package".into();
    let package = compatibility::compare_with_targets(
        contract.clone(),
        contract.clone(),
        &[],
        std::slice::from_ref(&explicit),
        std::slice::from_ref(&renamed),
    )
    .unwrap();
    assert!(
        package.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-operation-symbol-changed" && c.subject == "include")
    );
    let mut version = explicit.clone();
    version.package_version = "1.0.1".into();
    let version = compatibility::compare_with_targets(
        contract.clone(),
        contract,
        &[],
        &[explicit],
        &[version],
    )
    .unwrap();
    assert!(version.native[0].summary.is_proven_compatible());
    assert_eq!(
        version.native[0]
            .changes
            .iter()
            .map(|c| c.code.as_str())
            .collect::<Vec<_>>(),
        ["native-package-version-changed"]
    );
}

#[test]
fn source_security_renames_report_the_actual_cpp_credential_member() {
    let mut document = document();
    let fixture = Fixture::new(&document);
    let before = snapshot(fixture.load(), &target());
    let scheme = document["components"]["securitySchemes"]
        .as_object_mut()
        .unwrap()
        .remove("api-key")
        .unwrap();
    document["components"]["securitySchemes"]["management-token"] = scheme;
    document["security"] = json!([{"management-token":[]}]);
    fixture.write(&document);
    let after = snapshot(fixture.load(), &target());
    assert!(
        after.native[0]
            .operations
            .iter()
            .all(|op| op.descriptor["credential"]["fields"][0]["name"] == "management_token")
    );
    let report = compatibility::compare_snapshots(&before, &after);
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|change| change.code == "native-credentials-changed")
    );
}

#[test]
fn docs_relocation_and_same_native_wire_names_do_not_invent_native_breaks() {
    let mut document = document();
    let fixture = Fixture::new(&document);
    let before = snapshot(fixture.load(), &target());
    document["info"]["description"] = json!("Documentation moves all source byte spans.");
    document["components"]["schemas"]["Thing"]["description"] =
        json!("Human-facing prose, not a declaration change.");
    document["paths"]["/things/{id}"]["patch"]["parameters"][1]["name"] = json!("user_name");
    fixture.write(&document);
    let after = snapshot(fixture.load(), &target());
    let report = compatibility::compare_snapshots(&before, &after);
    assert!(
        report.native[0].changes.is_empty(),
        "{:?}",
        report.native[0].changes
    );
    let moved = Fixture::new(&document);
    let moved = snapshot(moved.load(), &target());
    let report = compatibility::compare_snapshots(&after, &moved);
    assert!(
        report.native[0].changes.is_empty(),
        "{:?}",
        report.native[0].changes
    );
}

#[test]
fn new_required_input_removes_the_no_input_call_default() {
    let mut document = document();
    let fixture = Fixture::new(&document);
    let before = snapshot(fixture.load(), &target());
    document["paths"]["/things"]["get"]["parameters"] =
        json!([{"name":"filter","in":"query","required":true,"schema":{"type":"string"}}]);
    fixture.write(&document);
    let after = snapshot(fixture.load(), &target());
    let read = after.native[0]
        .operations
        .iter()
        .find(|op| op.operation_id == "readThing")
        .unwrap();
    assert_eq!(
        read.descriptor["constructor"]["parameters"][0]["member"],
        "filter"
    );
    assert_eq!(
        read.descriptor["constructor"]["call"]["parameters"][0]["hasDefault"],
        false
    );
    let report = compatibility::compare_snapshots(&before, &after);
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-input-constructor-changed"
                && c.operation_id_after.as_deref() == Some("readThing"))
    );
}

#[test]
fn public_planner_refusals_are_located_native_findings() {
    let fixture = Fixture::new(&document());
    let contract = fixture.load();
    let mut invalid = target();
    invalid.import_name = Some("std".into());
    let sources = selected(&contract);
    let errors = backend::generate(contract.clone(), &sources, &invalid).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.code == "cpp-package-identity" && e.source.is_some())
    );
    let native = compatibility::snapshot(contract, &[], &[invalid]).unwrap();
    assert_eq!(native.native[0].status, PlanStatus::Unavailable);
    assert!(native.native[0].findings.iter().any(|finding| {
        finding.code == "cpp-package-identity"
            && finding
                .source
                .as_ref()
                .is_some_and(|source| source.span.is_some())
    }));
    let mut unsupported = document();
    unsupported["components"]["schemas"]["Thing"]["properties"]["tuple"] =
        json!({"type":"array","prefixItems":[{"type":"string"}]});
    fixture.write(&unsupported);
    let contract = fixture.load();
    let sources = selected(&contract);
    assert!(backend::generate(contract.clone(), &sources, &target()).is_err());
    let native = compatibility::snapshot(contract, &[], &[target()]).unwrap();
    assert_eq!(native.native[0].status, PlanStatus::Unavailable);
    assert!(native.native[0].findings.iter().any(|finding| {
        finding
            .source
            .as_ref()
            .is_some_and(|source| source.pointer.ends_with("/properties/tuple"))
    }));
}

#[test]
fn cpp_session_has_zero_unchanged_compiles_renders_or_owned_writes() {
    let document = document();
    let fixture = Fixture::new(&document);
    let config = SessionConfig {
        targets: vec![target()],
        ..Default::default()
    };
    let mut session = Session::new(&fixture.entry, config.clone()).unwrap();
    let first = session.generate().unwrap();
    assert_eq!(first.delta.compiles, 1);
    assert_eq!(first.delta.renders, 1);
    let next = session.generate().unwrap();
    assert_eq!(next.delta.compiles, 0);
    assert_eq!(next.delta.renders, 0);
    assert_eq!(next.delta.cache_hits, 1);
    assert!(next.changed_paths.is_empty());
    assert!(Arc::ptr_eq(&first.contract, &next.contract));
    assert!(Arc::ptr_eq(&first.files, &next.files));
    let output = fixture.directory.path().join("out");
    session.write(&first, &output).unwrap();
    let before = first
        .files
        .iter()
        .map(|file| {
            let metadata = std::fs::metadata(output.join(&file.path)).unwrap();
            (
                file.path.clone(),
                metadata.modified().unwrap(),
                std::fs::read(output.join(&file.path)).unwrap(),
            )
        })
        .collect::<Vec<_>>();
    session.write(&next, &output).unwrap();
    for (path, modified, bytes) in before {
        assert_eq!(
            std::fs::metadata(output.join(&path))
                .unwrap()
                .modified()
                .unwrap(),
            modified
        );
        assert_eq!(std::fs::read(output.join(path)).unwrap(), bytes);
    }
    let mut renamed = config.clone();
    renamed.targets[0].import_name = Some("changed::sdk".into());
    session.set_config(renamed).unwrap();
    let renamed = session.generate().unwrap();
    assert_eq!(renamed.delta.compiles, 0);
    assert_eq!(renamed.delta.renders, 1);
    assert!(Arc::ptr_eq(&next.contract, &renamed.contract));
    let changes = compatibility::compare_with_targets(
        next.contract.clone(),
        renamed.contract.clone(),
        &[],
        &config.targets,
        &[TargetConfig {
            import_name: Some("changed::sdk".into()),
            ..target()
        }],
    )
    .unwrap();
    assert!(!changes.native[0].changes.is_empty());
    session.set_config(config).unwrap();
    let reverted = session.generate().unwrap();
    assert_eq!(reverted.delta.compiles, 0);
    assert_eq!(reverted.delta.renders, 0);
    assert_eq!(reverted.files, first.files);
    let paths = reverted
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<BTreeSet<_>>();
    assert!(paths.iter().all(|path| path.starts_with("cpp/")));
    assert_eq!(document["openapi"], "3.1.0");
}
