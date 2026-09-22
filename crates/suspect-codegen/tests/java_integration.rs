#![cfg(feature = "java-sdk")]
//! Java's canonical registry, session and typed compatibility adapter. The
//! accepted JDK/runtime matrix remains in java_sdk.rs; this is its integration gate.

use std::{
    collections::BTreeMap,
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
    java_sdk::{self, MavenConfig, PackageConfig},
};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

fn crate_root() -> PathBuf {
    std::env::var_os("SUSPECT_JAVA_CRATE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_MANIFEST_DIR").into())
}

fn root() -> PathBuf {
    let base = crate_root().join("../../target/sdk-java-integration/java");
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
    std::fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    let contract = load(&path);
    (path, contract)
}

fn selected(contract: &Contract) -> Vec<SourceId> {
    contract
        .operations()
        .map(|op| op.source().clone())
        .collect()
}

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::JavaHttp,
        package_name: "example.widgets:thing-sdk".into(),
        package_version: "1.0.0".into(),
        import_name: Some("example.sdk".into()),
    }
}

fn response(schema: Value) -> Value {
    json!({"description":"Declared response","content":{"application/json":{"schema":schema}}})
}

fn api() -> Value {
    json!({"openapi":"3.1.0","info":{"title":"Java integration","version":"1"},
        "servers":[{"url":"https://example.test/api/v1"}],"security":[{"apiKey":[]}],
        "components":{"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}},"schemas":{
            "Thing":{"type":"object","required":["kind","required_text","required_note"],"properties":{
                "kind":{"type":"string","const":"thing"},"required_text":{"type":"string"},"required_note":{"type":["string","null"]},
                "optional_text":{"type":"string","default":"annotation"},"optional_note":{"type":["string","null"]},"json_value":true,
                "payload":{"$ref":"#/components/schemas/Choice"},"null_choice":{"$ref":"#/components/schemas/NullChoice"},
                "literal":{"$ref":"#/components/schemas/Literal"},"node":{"$ref":"#/components/schemas/Node"},"bag":{"$ref":"#/components/schemas/Bag"},
                "list_alias":{"$ref":"#/components/schemas/ListAlias"},"numbers":{"$ref":"#/components/schemas/NumberList"},
                "nullable_object":{"$ref":"#/components/schemas/NullableObject"}
            }},
            "Choice":{"oneOf":[{"$ref":"#/components/schemas/TextChoice"},{"$ref":"#/components/schemas/SecretChoice"}]},
            "NullChoice":{"oneOf":[{"type":"null"},{"type":"string"}]},
            "Literal":{"enum":["auto",true,1,null,{"wire":"literal","source":true}]},
            "TextChoice":{"type":"object","required":["kind","text"],"properties":{"kind":{"const":"text"},"text":{"type":"string"}}},
            "SecretChoice":{"type":"object","required":["kind","secret"],"properties":{"kind":{"const":"secret"},"secret":{"type":"string"}}},
            "Node":{"type":"object","required":["label"],"properties":{"label":{"type":"string"},"child":{"$ref":"#/components/schemas/Node"}}},
            "Bag":{"type":"object","additionalProperties":{"type":["number","null"]}},
            "TextList":{"type":"array","items":{"type":"string"}},"NumberList":{"type":"array","items":{"type":"number"}},
            "ListAlias":{"$ref":"#/components/schemas/TextList"},
            "NullableObject":{"type":["object","null"],"required":["name"],"properties":{"name":{"type":"string"}}},
            "Patch":{"type":"object","properties":{"note":{"type":["string","null"]}}},
            "Failure":{"type":"object","required":["message"],"properties":{"message":{"type":"string"}}}
        }},
        "paths":{
            "/things":{"post":{"operationId":"createThing","requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Thing"}}}},
                "responses":{"201":response(json!({"$ref":"#/components/schemas/Thing"})),"400":response(json!({"$ref":"#/components/schemas/Failure"}))}},
                "get":{"operationId":"listThings","parameters":[{"name":"tag","in":"query","schema":{"type":"string"}}],
                    "responses":{"200":response(json!({"type":"array","items":{"$ref":"#/components/schemas/Thing"}}))}}},
            "/things/{id}":{"patch":{"operationId":"updateThing","parameters":[{"name":"id","in":"path","required":true,"schema":{"type":"string"}}],
                "requestBody":{"required":false,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Patch"}}}},
                "responses":{"200":response(json!({"$ref":"#/components/schemas/Thing"}))}}},
            "/status":{"get":{"operationId":"readStatus","responses":{"200":response(json!({"type":"string"}))}}}
        }
    })
}

fn snapshot(contract: Arc<Contract>, target: &TargetConfig) -> NativeSnapshot {
    let native = compatibility::snapshot(contract, &[], std::slice::from_ref(target))
        .unwrap()
        .native
        .remove(0);
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

fn operation<'a>(native: &'a NativeSnapshot, id: &str) -> &'a NativeOperation {
    native
        .operations
        .iter()
        .find(|op| op.operation_id == id)
        .unwrap()
}

fn field<'a>(value: &'a Value, name: &str) -> &'a Value {
    value["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == name || f["member"] == name)
        .unwrap()
}

fn argument<'a>(value: &'a Value, name: &str) -> &'a Value {
    value["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == name)
        .unwrap()
}

fn names(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect()
}

fn edit(mut value: Value, change: impl FnOnce(&mut Value)) -> CompatibilityReport {
    let (path, before) = fixture(&value);
    change(&mut value);
    std::fs::write(&path, value.to_string()).unwrap();
    let report = compatibility::compare(before, load(&path), &[], &[target()]).unwrap();
    for native in [&report.native[0].before, &report.native[0].after] {
        let native = native.as_ref().unwrap();
        assert_eq!(native.status, PlanStatus::Planned, "{:?}", native.findings);
        assert!(native.findings.is_empty());
    }
    report
}

#[test]
fn java_registry_generation_and_separate_maven_import_identity() {
    assert!(Backend::ALL.contains(&Backend::JavaHttp));
    assert_eq!(Backend::JavaHttp.name(), "java-http");
    assert_eq!(Backend::JavaHttp.artifact_directory(), "java");
    assert_eq!(Backend::JavaHttp.owner(), "suspect-sdk:java-http");
    assert_eq!(
        serde_json::to_value(Backend::JavaHttp).unwrap(),
        json!("java-http")
    );
    let (_, contract) = fixture(&api());
    let selected = selected(&contract);
    let files = backend::generate(contract.clone(), &selected, &target()).unwrap();
    let direct = java_sdk::plan_sdk_with_protocol(
        contract.clone(),
        &selected,
        PackageConfig {
            package: "example.sdk".into(),
            version: "1.0.0".into(),
            api_name: "Client".into(),
        },
        &[],
        MavenConfig {
            group_id: Some("example.widgets".into()),
            artifact_id: "thing-sdk".into(),
            ..Default::default()
        },
        // The adapter compiles ua/v1 attribution constants from the same
        // package identity; the direct plan must retain them to stay identical.
        java_sdk::ProtocolConfig {
            attribution: Some(suspect_codegen::attribution::AttributionDescriptor::plan(
                env!("CARGO_PKG_VERSION"),
                "example.widgets:thing-sdk",
                "1.0.0",
                contract.openapi_version(),
                Backend::JavaHttp.language_tag(),
            )),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(files, direct.render().unwrap());
    assert!(files.iter().all(|file| file.path.starts_with("java/")));
    let pom = &files
        .iter()
        .find(|f| f.path == "java/pom.xml")
        .unwrap()
        .content;
    assert!(pom.contains("<groupId>example.widgets</groupId><artifactId>thing-sdk</artifactId><version>1.0.0</version>"));
    assert!(
        files
            .iter()
            .any(|f| f.path == "java/src/main/java/example/sdk/Client.java")
    );
    let manifest: Value = serde_json::from_str(
        &files
            .iter()
            .find(|f| f.path.ends_with("/sdk-manifest.json"))
            .unwrap()
            .content,
    )
    .unwrap();
    assert_eq!(manifest["groupId"], "example.widgets");
    assert_eq!(manifest["package"], "example.sdk");
    let implicit = TargetConfig {
        import_name: None,
        ..target()
    };
    let default_files = backend::generate(contract, &selected, &implicit).unwrap();
    assert!(
        default_files
            .iter()
            .any(|f| f.path == "java/src/main/java/example/widgets/Client.java")
    );
}

#[test]
fn absent_maven_group_preserves_every_existing_default_artifact_byte() {
    let uri = Uri::parse("https://fixtures.example/java/default-maven.yaml").unwrap();
    let bytes =
        std::fs::read(crate_root().join("tests/fixtures/m2/canonical.openapi.yaml")).unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(uri.clone(), uri.clone(), bytes).unwrap()])
            .unwrap(),
    );
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    let contract = Arc::new(Contract::from_workspace(&workspace, &uri).unwrap());
    let plan = java_sdk::plan_sdk_with_maven(
        contract.clone(),
        &selected(&contract),
        PackageConfig {
            package: "com.example.generated".into(),
            version: "0.1.0".into(),
            api_name: "OpenRouter".into(),
        },
        &[],
        MavenConfig::default(),
    )
    .unwrap();
    assert!(plan.maven().group_id.is_none());
    assert_eq!(plan.maven_group_id(), "com.example.generated");
    let files: BTreeMap<_, _> = plan
        .render()
        .unwrap()
        .into_iter()
        .map(|file| (file.path, file.content))
        .collect();
    // Phase 2's 68-file byte baseline remains retained. Protocol v3 deliberately
    // adds real media/stream runtimes; the Maven default itself must still leave
    // every source, codec, runtime and POM byte identical to an explicit same-group choice.
    let explicit = java_sdk::plan_sdk_with_maven(
        contract.clone(),
        &selected(&contract),
        plan.package().clone(),
        &[],
        MavenConfig {
            group_id: Some("com.example.generated".into()),
            ..Default::default()
        },
    )
    .unwrap()
    .render()
    .unwrap();
    assert_eq!(files.len(), explicit.len());
    assert!(
        files.contains_key("java/src/main/resources/com/example/generated/protocol-program.json")
    );
    for file in explicit {
        if file.path.ends_with("/sdk-manifest.json") {
            let original: Value = serde_json::from_str(&files[&file.path]).unwrap();
            let mut explicit: Value = serde_json::from_str(&file.content).unwrap();
            assert_eq!(explicit["groupId"], "com.example.generated");
            explicit.as_object_mut().unwrap().remove("groupId");
            assert_eq!(original, explicit);
        } else {
            assert_eq!(
                files[&file.path], file.content,
                "Maven default changed {}",
                file.path
            );
        }
    }
}

#[test]
fn unchanged_and_relocated_inputs_have_complete_native_snapshots_without_unknowns() {
    let (path, contract) = fixture(&api());
    let first = snapshot(contract.clone(), &target());
    let second = snapshot(load(&path), &target());
    assert_eq!(first, second);
    let report = compatibility::compare(contract.clone(), contract, &[], &[target()]).unwrap();
    assert!(report.is_proven_compatible(), "{report:#?}");
    assert!(report.wire.is_empty() && report.native[0].changes.is_empty());
    assert!(
        first
            .models
            .iter()
            .all(|model| model.descriptor.is_some() && model.source.span.is_some())
    );
    let (_, relocated) = fixture(&api());
    let original = compatibility::snapshot(load(&path), &[], &[target()]).unwrap();
    let relocated = compatibility::snapshot(relocated, &[], &[target()]).unwrap();
    let report = compatibility::compare_snapshots(&original, &relocated);
    assert!(report.is_proven_compatible(), "{report:#?}");
}

#[test]
fn retained_models_builders_omitters_nulls_literals_refs_and_codecs_are_actual_signatures() {
    let (_, contract) = fixture(&api());
    let native = snapshot(contract, &target());
    let thing = model(&native, "example.sdk.Thing", "model")
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(thing["kind"], "class");
    assert_eq!(thing["constructorAccess"], "private");
    assert_eq!(thing["constructor"]["name"], "example.sdk.Thing.builder");
    assert_eq!(
        names(&thing["constructor"]["parameters"]),
        ["requiredNote", "requiredText"]
    );
    assert_eq!(
        argument(&thing["constructor"], "requiredText")["type"],
        json!({"kind":"primitive","name":"java.lang.String"})
    );
    assert_eq!(
        argument(&thing["constructor"], "requiredNote")["type"]["kind"],
        "nullable"
    );
    assert_eq!(
        field(thing, "optionalText")["type"]["name"],
        "example.sdk.Presence"
    );
    assert_eq!(
        field(thing, "optionalText")["type"]["arguments"][0]["name"],
        "java.lang.String"
    );
    assert_eq!(
        field(thing, "optionalNote")["type"]["arguments"][0]["kind"],
        "nullable"
    );
    assert_eq!(
        field(thing, "optionalText")["initialization"]["kind"],
        "absent"
    );
    assert_eq!(
        field(thing, "optionalText")["omitter"]["name"],
        "omitOptionalText"
    );
    assert_eq!(
        field(thing, "optionalText")["builderSetter"]["returns"]["name"],
        "example.sdk.Thing.Builder"
    );
    assert_eq!(field(thing, "optionalText")["mutable"], false);
    assert_eq!(
        field(thing, "kind")["fixed"],
        json!({"kind":"literal","type":"example.sdk.ThingKind","member":"THING","value":"thing"})
    );
    assert!(field(thing, "kind")["builderSetter"].is_null());
    assert_eq!(
        thing["builder"]["buildResult"],
        "independent-deep-immutable-snapshot"
    );
    assert_eq!(thing["builder"]["constructorAccess"], "private");
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
    assert_eq!(
        field(bag, "additionalProperties")["type"]["arguments"][1]["kind"],
        "nullable"
    );
    assert_eq!(
        field(bag, "additionalProperties")["type"]["arguments"][1]["type"]["name"],
        "example.sdk.JsonRuntime.JsonNumber"
    );
    let arm = model(&native, "example.sdk.Choice.Variant1", "union-arm")
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(
        argument(&arm["constructor"], "value")["type"]["name"],
        "example.sdk.TextChoice"
    );
    assert_eq!(arm["constructor"]["snapshot"], "deep-immutable");
    assert_eq!(arm["encodingValidation"], "selected-arm-and-parent");
    let null_union = model(&native, "example.sdk.NullChoice.CODEC", "codec")
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(null_union["type"]["arguments"][0]["kind"], "named");
    let null_arm = model(&native, "example.sdk.NullChoice.Variant1", "union-arm")
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(
        argument(&null_arm["constructor"], "value")["type"]["name"],
        "example.sdk.JsonRuntime.JsonNull"
    );
    let literal = model(&native, "example.sdk.Literal", "model")
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(literal["kind"], "literal-class");
    assert!(
        literal["constants"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["value"] == json!({"wire":"literal","source":true}))
    );
    let alias = model(&native, "example.sdk.ListAlias", "model")
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(alias["kind"], "codec-holder-class");
    assert_eq!(alias["nativeType"]["name"], "java.util.List");
    let codec = model(&native, "example.sdk.ListAlias.CODEC", "codec")
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(codec["owner"], "example.sdk.ListAlias");
    assert_eq!(codec["member"], "CODEC");
    assert_eq!(codec["final"], true);
    assert_eq!(
        codec["type"]["arguments"][0]["arguments"][0]["name"],
        "java.lang.String"
    );
    assert_eq!(
        names(&codec["methods"]),
        [
            "decode",
            "decode",
            "decodeValue",
            "encode",
            "encodeValue",
            "snapshot",
            "withLimits",
            "limits",
            "source"
        ]
    );
}

#[test]
fn nested_input_status_sync_async_no_input_and_control_apis_are_captured() {
    let (_, contract) = fixture(&api());
    let native = snapshot(contract, &target());
    let create = operation(&native, "createThing");
    assert_eq!(create.symbols["client"], "example.sdk.Client");
    assert_eq!(create.symbols["method"], "createThing");
    assert_eq!(create.symbols["async-method"], "createThingAsync");
    assert_eq!(
        create.symbols["input"],
        "example.sdk.Client.CreateThingInput"
    );
    assert_eq!(
        create.symbols["input-constructor"],
        "example.sdk.Client.CreateThingInput.builder"
    );
    assert_eq!(
        create.symbols["success"],
        "example.sdk.Client.CreateThingStatus201"
    );
    assert_eq!(
        create.descriptor["package"],
        json!({"groupId":"example.widgets","artifactId":"thing-sdk","version":"1.0.0","namespace":"example.sdk","javaRelease":21})
    );
    let constructor = &create.descriptor["constructor"]["input"];
    assert_eq!(names(&constructor["parameters"]), ["body"]);
    assert_eq!(
        argument(constructor, "body")["type"]["name"],
        "example.sdk.Thing"
    );
    assert_eq!(
        constructor["returns"]["name"],
        "example.sdk.Client.CreateThingInput.Builder"
    );
    let asynchronous = &create.descriptor["parameters"]["asyncMethod"];
    assert_eq!(
        asynchronous["returns"]["name"],
        "java.util.concurrent.CompletableFuture"
    );
    assert_eq!(
        asynchronous["returns"]["arguments"][0]["name"],
        "example.sdk.Client.CreateThingStatus201"
    );
    assert_eq!(
        create.descriptor["parameters"]["canCallWithoutInput"],
        false
    );
    assert_eq!(
        create.descriptor["responseUnions"]["success"]["kind"],
        "concrete-result"
    );
    let status = create.descriptor["responses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["status"] == 201)
        .unwrap();
    assert_eq!(status["type"], "example.sdk.Client.CreateThingStatus201");
    assert_eq!(status["constructorAccess"], "private");
    assert_eq!(field(status, "data")["type"]["name"], "example.sdk.Thing");
    let error = create.descriptor["responses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["status"] == 400)
        .unwrap();
    assert_eq!(error["type"], "example.sdk.Client.CreateThingStatus400");
    assert_eq!(error["base"], "example.sdk.SdkException");
    assert_eq!(error["statusInherited"], true);
    let no_input = operation(&native, "readStatus");
    assert_eq!(no_input.symbols["no-input-method"], "readStatus");
    assert_eq!(
        no_input.descriptor["parameters"]["noInputMethod"]["parameters"],
        json!([])
    );
    assert_eq!(
        no_input.descriptor["parameters"]["noInputAsyncMethod"]["parameters"],
        json!([])
    );
    // An operation with optional query members still has the actual explicit
    // input signature. Java does not invent a default argument or overload.
    let list = operation(&native, "listThings");
    assert_eq!(list.descriptor["parameters"]["canCallWithoutInput"], false);
    assert_eq!(
        list.descriptor["constructor"]["input"]["canInitializeWithoutArguments"],
        true
    );
    let update = operation(&native, "updateThing");
    assert_eq!(
        update.descriptor["body"]["type"]["name"],
        "example.sdk.Presence"
    );
    assert_eq!(update.descriptor["body"]["omitter"]["name"], "omitBody");
    assert_eq!(
        names(&update.descriptor["constructor"]["input"]["parameters"]),
        ["id"]
    );
    let client = &create.descriptor["constructor"]["client"];
    assert_eq!(
        client["constructor"]["parameters"][0]["type"]["name"],
        "example.sdk.HttpRuntime.Options"
    );
    assert_eq!(client["implements"], json!(["java.lang.AutoCloseable"]));
    let options = &client["controls"]["options"];
    assert_eq!(
        options["factory"]["returns"]["name"],
        "example.sdk.HttpRuntime.Options.Builder"
    );
    assert_eq!(
        names(&options["builder"]["methods"]),
        [
            "serverUrl",
            "timeout",
            "maxResponseBytes",
            "maxRequestBytes",
            "maxCaptureBytes",
            "maxUrlBytes",
            "httpClient",
            "codecLimits",
            "documentUrl",
            "serverIndex",
            "serverName",
            "securityAlternative",
            "accept",
            "maxHeaderBytes",
            "maxStreamBufferBytes",
            "serverVariable"
        ]
    );
    assert_eq!(options["deadline"], "whole-call-including-codecs-and-body");
    assert_eq!(create.descriptor["credential"]["sourceSchemeKey"], "apiKey");
    assert_eq!(
        names(&create.descriptor["credential"]["method"]["parameters"]),
        ["sourceScheme", "token"]
    );
    assert_eq!(
        create.descriptor["credential"]["method"]["returns"]["name"],
        "example.sdk.HttpRuntime.Options.Builder"
    );
}

#[test]
fn presence_nullability_and_fixed_field_changes_produce_native_migration_evidence() {
    let required = edit(api(), |api| {
        api["components"]["schemas"]["Thing"]["required"]
            .as_array_mut()
            .unwrap()
            .push(json!("optional_note"))
    });
    assert!(!required.is_proven_compatible());
    assert!(
        required.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-model-shape-changed" && c.subject == "example.sdk.Thing")
    );
    let thing = model(
        required.native[0].after.as_ref().unwrap(),
        "example.sdk.Thing",
        "model",
    )
    .descriptor
    .as_ref()
    .unwrap();
    assert_eq!(
        names(&thing["constructor"]["parameters"]),
        ["optionalNote", "requiredNote", "requiredText"]
    );
    assert_eq!(field(thing, "optionalNote")["type"]["kind"], "nullable");
    assert!(field(thing, "optionalNote")["omitter"].is_null());
    let nullable = edit(api(), |api| {
        api["components"]["schemas"]["Thing"]["properties"]["optional_text"]["type"] =
            json!(["string", "null"])
    });
    let thing = model(
        nullable.native[0].after.as_ref().unwrap(),
        "example.sdk.Thing",
        "model",
    )
    .descriptor
    .as_ref()
    .unwrap();
    assert_eq!(
        field(thing, "optionalText")["type"]["arguments"][0]["kind"],
        "nullable"
    );
    assert!(
        nullable.native[0]
            .changes
            .iter()
            .any(|c| c.subject == "example.sdk.Thing")
    );
    let tag = edit(api(), |api| {
        api["components"]["schemas"]["Thing"]["properties"]["kind"]["const"] = json!("other")
    });
    assert!(tag.native[0].changes.iter().any(|c|c.code=="native-model-shape-changed" && c.subject=="example.sdk.ThingKind"));
    let unfixed = edit(api(), |api| {
        api["components"]["schemas"]["Thing"]["properties"]["kind"]
            .as_object_mut()
            .unwrap()
            .remove("const");
    });
    let thing = model(
        unfixed.native[0].after.as_ref().unwrap(),
        "example.sdk.Thing",
        "model",
    )
    .descriptor
    .as_ref()
    .unwrap();
    assert_eq!(
        names(&thing["constructor"]["parameters"]),
        ["kind", "requiredNote", "requiredText"]
    );
    assert_eq!(field(thing, "kind")["builderSetter"]["name"], "kind");
}

#[test]
fn nullable_objects_preserve_nonnull_builder_construction_and_nullable_codecs() {
    let report = edit(api(), |api| {
        api["components"]["schemas"]["Thing"]["type"] = json!(["object", "null"])
    });
    let old = model(
        report.native[0].before.as_ref().unwrap(),
        "example.sdk.Thing",
        "model",
    )
    .descriptor
    .as_ref()
    .unwrap();
    let new = model(
        report.native[0].after.as_ref().unwrap(),
        "example.sdk.Thing",
        "model",
    )
    .descriptor
    .as_ref()
    .unwrap();
    assert_eq!(old["constructor"], new["constructor"]);
    assert_eq!(old["builder"], new["builder"]);
    assert_eq!(
        new["builder"]["build"]["returns"],
        json!({"kind":"named","name":"example.sdk.Thing"})
    );
    assert_eq!(new["codec"]["type"]["arguments"][0]["kind"], "nullable");
    assert_eq!(
        operation(report.native[0].after.as_ref().unwrap(), "createThing").descriptor["body"]["type"]
            ["kind"],
        "nullable"
    );
}

#[test]
fn literal_instance_keys_union_membership_and_alias_element_types_are_not_erased() {
    let literals = edit(api(), |api| {
        api["components"]["schemas"]["Literal"]["enum"][4]["wire"] = json!("changed-instance-key")
    });
    assert!(
        literals.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-model-shape-changed" && c.subject == "example.sdk.Literal")
    );
    let union = edit(api(), |api| {
        api["components"]["schemas"]["CountChoice"] = json!({"type":"integer"});
        api["components"]["schemas"]["Choice"]["oneOf"]
            .as_array_mut()
            .unwrap()
            .push(json!({"$ref":"#/components/schemas/CountChoice"}));
    });
    let arm = model(
        union.native[0].after.as_ref().unwrap(),
        "example.sdk.Choice.Variant3",
        "union-arm",
    )
    .descriptor
    .as_ref()
    .unwrap();
    assert_eq!(
        argument(&arm["constructor"], "value")["type"]["name"],
        "example.sdk.JsonRuntime.JsonNumber"
    );
    assert!(
        union.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-model-shape-changed" && c.subject == "example.sdk.Choice")
    );
    let inclusive = edit(api(), |api| {
        let branches = api["components"]["schemas"]["Choice"]
            .as_object_mut()
            .unwrap()
            .remove("oneOf")
            .unwrap();
        api["components"]["schemas"]["Choice"]["anyOf"] = branches;
    });
    let choice = model(
        inclusive.native[0].after.as_ref().unwrap(),
        "example.sdk.Choice",
        "model",
    )
    .descriptor
    .as_ref()
    .unwrap();
    assert_eq!(choice["exclusive"], false);
    let alias = edit(api(), |api| {
        api["components"]["schemas"]["ListAlias"]["$ref"] = json!("#/components/schemas/NumberList")
    });
    let old = model(
        alias.native[0].before.as_ref().unwrap(),
        "example.sdk.ListAlias.CODEC",
        "codec",
    )
    .descriptor
    .as_ref()
    .unwrap();
    let new = model(
        alias.native[0].after.as_ref().unwrap(),
        "example.sdk.ListAlias.CODEC",
        "codec",
    )
    .descriptor
    .as_ref()
    .unwrap();
    assert_eq!(
        old["type"]["arguments"][0]["arguments"][0]["name"],
        "java.lang.String"
    );
    assert_eq!(
        new["type"]["arguments"][0]["arguments"][0]["name"],
        "example.sdk.JsonRuntime.JsonNumber"
    );
    assert!(
        alias.native[0]
            .changes
            .iter()
            .any(|c| c.subject == "example.sdk.ListAlias.CODEC"
                && c.code == "native-model-shape-changed")
    );
}

#[test]
fn methods_no_input_overloads_and_status_return_shape_changes_are_reported() {
    let renamed = edit(api(), |api| {
        api["paths"]["/status"]["get"]["operationId"] = json!("readHealth")
    });
    for role in [
        "method",
        "async-method",
        "input",
        "input-constructor",
        "no-input-method",
        "no-input-async-method",
    ] {
        assert!(
            renamed.native[0]
                .changes
                .iter()
                .any(|c| c.code == "native-operation-symbol-changed" && c.subject == role),
            "{role}: {:?}",
            renamed.native[0].changes
        );
    }
    let no_input =
        edit(
            api(),
            |api| {
                api["paths"]["/status"]["get"]["parameters"] =
                    json!([{"name":"optional","in":"query","schema":{"type":"string"}}])
            },
        );
    let next = operation(no_input.native[0].after.as_ref().unwrap(), "readStatus");
    assert_eq!(next.descriptor["parameters"]["canCallWithoutInput"], false);
    assert!(!next.symbols.contains_key("no-input-method"));
    assert!(
        no_input.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-operation-symbol-changed" && c.subject == "no-input-method")
    );
    let required = edit(api(), |api| {
        api["paths"]["/things/{id}"]["patch"]["requestBody"]["required"] = json!(true)
    });
    assert!(
        required.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-input-constructor-changed"
                && c.operation_id_after.as_deref() == Some("updateThing"))
    );
    let update = operation(required.native[0].after.as_ref().unwrap(), "updateThing");
    assert_eq!(
        names(&update.descriptor["constructor"]["input"]["parameters"]),
        ["id", "body"]
    );
    assert!(update.descriptor["body"]["omitter"].is_null());
    let multiple = edit(api(), |api| {
        api["paths"]["/things"]["post"]["responses"]["200"] =
            response(json!({"$ref":"#/components/schemas/Thing"}))
    });
    let create = operation(multiple.native[0].after.as_ref().unwrap(), "createThing");
    assert_eq!(
        create.symbols["success"],
        "example.sdk.Client.CreateThingSuccess"
    );
    assert_eq!(
        create.descriptor["responseUnions"]["success"]["kind"],
        "sealed-interface"
    );
    assert_eq!(
        create.descriptor["responseUnions"]["success"]["directResult"],
        false
    );
    assert!(
        multiple.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-responses-changed")
    );
    assert!(
        create.descriptor["responses"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["success"] == true)
            .all(|r| r["implements"] == json!(["example.sdk.Client.CreateThingSuccess"]))
    );
}

#[test]
fn collision_allocated_model_field_operation_and_codec_names_are_retained() {
    let mut value = api();
    value["components"]["schemas"]["Client"] = json!({"type":"object","properties":{"builder":{"type":"string"},"getClass":{"type":"string"},"a-b":{"type":"string"},"a_b":{"type":"string"}}});
    value["paths"]["/status"]["get"]["operationId"] = json!("close");
    value["paths"]["/status"]["get"]["responses"]["200"] =
        response(json!({"$ref":"#/components/schemas/Client"}));
    let (_, contract) = fixture(&value);
    let native = snapshot(contract, &target());
    assert_eq!(operation(&native, "close").symbols["method"], "close2");
    let client_model = model(&native, "example.sdk.Client2", "model")
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(
        client_model["constructor"]["name"],
        "example.sdk.Client2.builder"
    );
    assert_eq!(client_model["codec"]["owner"], "example.sdk.Client2");
    assert_eq!(
        field(client_model, "builder2")["builderSetter"]["name"],
        "builder2"
    );
    assert_eq!(
        field(client_model, "getClass2")["getter"]["name"],
        "getClass2"
    );
    assert!(
        client_model["fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["name"] == "aB2")
    );
    assert_eq!(
        operation(&native, "close").descriptor["responses"][0]["fields"][0]["type"]["name"],
        "example.sdk.Client2"
    );
}

#[test]
fn documentation_annotations_and_program_index_shifts_are_not_native_api_changes() {
    let report = edit(api(), |api| {
        api["info"]["description"] = json!("Different source offsets");
        api["paths"]["/things"]["post"]["description"] = json!("Docs */ @param source");
        api["components"]["schemas"]["Thing"]["title"] = json!("No native naming effect");
        let field = &mut api["components"]["schemas"]["Thing"]["properties"]["optional_text"];
        field["default"] = json!("different annotation");
        field["description"] = json!("docs");
        field["example"] = json!("valid");
        field["readOnly"] = json!(false);
        field["writeOnly"] = json!(false);
    });
    assert!(
        report.native[0].changes.is_empty(),
        "{:?}",
        report.native[0].changes
    );
    let mut value = api();
    let (path, before) = fixture(&value);
    let index = |contract: Arc<Contract>| {
        let plan = java_sdk::plan_sdk(
            contract.clone(),
            &selected(&contract),
            PackageConfig::default(),
            &[],
        )
        .unwrap();
        plan.models()
            .symbols()
            .iter()
            .find(|s| s.source().pointer() == "/components/schemas/Thing/properties/kind")
            .unwrap()
            .codec()
            .root
    };
    let original_index = index(before.clone());
    value["components"]["schemas"]["Thing"]["properties"]["aaa"] = json!({"type":"string"});
    std::fs::write(&path, value.to_string()).unwrap();
    let after = load(&path);
    assert_ne!(
        original_index,
        index(after.clone()),
        "the control must shift actual validation indices"
    );
    let report = compatibility::compare(before, after, &[], &[target()]).unwrap();
    for (name, role) in [
        ("example.sdk.ThingKind", "model"),
        ("example.sdk.ThingKind.CODEC", "codec"),
        ("example.sdk.ThingRequiredText.CODEC", "codec"),
    ] {
        assert_eq!(
            model(report.native[0].before.as_ref().unwrap(), name, role).descriptor,
            model(report.native[0].after.as_ref().unwrap(), name, role).descriptor
        );
        assert!(
            !report.native[0].changes.iter().any(|c| c.subject == name),
            "{:?}",
            report.native[0].changes
        );
    }
}

#[test]
fn import_maven_and_version_changes_use_effective_native_identities() {
    let (_, contract) = fixture(&api());
    let implicit = TargetConfig {
        import_name: None,
        ..target()
    };
    let explicit = TargetConfig {
        import_name: Some("example.widgets".into()),
        ..target()
    };
    let equivalent = compatibility::compare_with_targets(
        contract.clone(),
        contract.clone(),
        &[],
        &[implicit],
        &[explicit],
    )
    .unwrap();
    assert!(
        equivalent.native[0].changes.is_empty(),
        "{:?}",
        equivalent.native[0].changes
    );
    let renamed = TargetConfig {
        import_name: Some("other.sdk".into()),
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
    assert!(rename.wire.is_empty());
    assert!(
        rename.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-operation-symbol-changed" && c.subject == "client")
    );
    assert!(
        rename.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-model-renamed")
    );
    assert!(
        rename.native[0]
            .after
            .as_ref()
            .unwrap()
            .models
            .iter()
            .all(|m| m.name.starts_with("other.sdk."))
    );
    let coordinate = TargetConfig {
        package_name: "another.group:renamed-sdk".into(),
        ..target()
    };
    let package = compatibility::compare_with_targets(
        contract.clone(),
        contract.clone(),
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
    let version = TargetConfig {
        package_version: "1.0.1".into(),
        ..target()
    };
    let bumped = compatibility::compare_with_targets(
        contract.clone(),
        contract,
        &[],
        &[target()],
        &[version],
    )
    .unwrap();
    assert_eq!(
        bumped.native[0].changes.len(),
        1,
        "{:?}",
        bumped.native[0].changes
    );
    assert_eq!(
        bumped.native[0].changes[0].code,
        "native-package-version-changed"
    );
    assert!(bumped.is_proven_compatible());
}

#[test]
fn credential_scheme_changes_and_admission_failures_have_explicit_findings() {
    let changed = edit(api(), |api| {
        let scheme = api["components"]["securitySchemes"]
            .as_object_mut()
            .unwrap()
            .remove("apiKey")
            .unwrap();
        api["components"]["securitySchemes"]["managementToken"] = scheme;
        api["security"] = json!([{"managementToken":[]}]);
    });
    assert!(
        changed.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-credentials-changed")
    );
    assert_eq!(
        operation(changed.native[0].after.as_ref().unwrap(), "createThing").descriptor["credential"]
            ["sourceSchemeKey"],
        "managementToken"
    );
    let (_, contract) = fixture(&api());
    let selected = selected(&contract);
    for invalid in [
        TargetConfig {
            package_name: "missing-coordinates".into(),
            ..target()
        },
        TargetConfig {
            package_name: "group:artifact:extra".into(),
            ..target()
        },
        TargetConfig {
            package_name: ":artifact".into(),
            ..target()
        },
        TargetConfig {
            package_name: "group:".into(),
            ..target()
        },
        TargetConfig {
            package_name: "bad/group:artifact".into(),
            ..target()
        },
        TargetConfig {
            import_name: Some("example.class".into()),
            ..target()
        },
        TargetConfig {
            package_version: "latest".into(),
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
    let mut invalid = api();
    invalid["components"]["schemas"]["Thing"]["properties"]["required_text"]["writeOnly"] =
        json!(true);
    let (_, contract) = fixture(&invalid);
    let native = compatibility::snapshot(contract, &[], &[target()])
        .unwrap()
        .native
        .remove(0);
    assert_eq!(native.status, PlanStatus::Unavailable);
    assert!(native.findings.iter().any(|f| {
        f.code == "java-directional-codec-unsupported"
            && f.source
                .as_ref()
                .is_some_and(|s| s.pointer.ends_with("/writeOnly"))
    }));
}

#[test]
fn backend_and_capture_use_only_selected_operation_roots() {
    let mut document = api();
    document["components"]["schemas"]["UnusedUnsupported"] =
        json!({"dependentRequired":{"a":["b"]}});
    document["paths"]["/unselected"] = json!({"post":{"operationId":"unselectedUpload","requestBody":{"content":{"multipart/form-data":{"schema":{"type":"object"}}}},"responses":{"204":{"description":"Empty"}}}});
    let (_, contract) = fixture(&document);
    let selected = contract
        .operations()
        .filter(|op| op.operation_id() == Some("readStatus"))
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let files = backend::generate(contract.clone(), &selected, &target()).unwrap();
    assert!(
        !files
            .iter()
            .any(|f| f.path.ends_with("/UnusedUnsupported.java"))
    );
    let native = compatibility::snapshot(contract, &["readStatus".into()], &[target()])
        .unwrap()
        .native
        .remove(0);
    assert_eq!(native.status, PlanStatus::Planned);
    assert!(native.findings.is_empty());
    assert_eq!(native.operations.len(), 1);
    assert!(
        !native
            .models
            .iter()
            .any(|m| m.name.contains("UnusedUnsupported"))
    );
}

#[test]
fn canonical_sessions_reuse_warm_files_contracts_and_other_targets_on_java_config_changes() {
    let (path, _) = fixture(&api());
    let other = TargetConfig {
        backend: Backend::TypescriptHttp,
        package_name: "thing-sdk".into(),
        package_version: "1.0.0".into(),
        import_name: None,
    };
    let original = SessionConfig {
        targets: vec![target(), other],
        operation_ids: vec!["readStatus".into()],
        ..Default::default()
    };
    let mut config = original.clone();
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
    let out = root().join("out");
    session.write(&cold, &out).unwrap();
    let before = std::fs::metadata(out.join("java/pom.xml"))
        .unwrap()
        .modified()
        .unwrap();
    session.write(&warm, &out).unwrap();
    assert_eq!(
        before,
        std::fs::metadata(out.join("java/pom.xml"))
            .unwrap()
            .modified()
            .unwrap()
    );
    let other_files = |output: &suspect_codegen::generation_session::SessionOutput| {
        output
            .files
            .iter()
            .filter(|f| f.path.starts_with("typescript/"))
            .cloned()
            .collect::<Vec<_>>()
    };
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
    assert_eq!(other_files(&cold), other_files(&renamed));
    assert!(
        !renamed.changed_paths.is_empty()
            && renamed.changed_paths.iter().all(|p| p.starts_with("java/"))
    );
    config.targets[0].package_name = "other.group:other-artifact".into();
    session.set_config(config.clone()).unwrap();
    let coordinates = session.generate().unwrap();
    assert_eq!(
        (
            coordinates.delta.compiles,
            coordinates.delta.renders,
            coordinates.delta.cache_hits
        ),
        (0, 1, 1)
    );
    assert!(Arc::ptr_eq(&cold.contract, &coordinates.contract));
    // Maven coordinates drive pom/manifest bytes, and the ua/v1 attribution
    // constants intentionally embed the sanitized package identity; every
    // other generated .java source byte stays coordinate-independent.
    assert!(
        coordinates
            .changed_paths
            .iter()
            .all(|p| p.starts_with("java/")
                && (!p.ends_with(".java") || p.ends_with("/Attribution.java")))
    );
    assert_eq!(other_files(&cold), other_files(&coordinates));
    session.set_config(original).unwrap();
    let reverted = session.generate().unwrap();
    assert_eq!(
        (
            reverted.delta.compiles,
            reverted.delta.renders,
            reverted.delta.cache_hits
        ),
        (0, 0, 1)
    );
    assert!(Arc::ptr_eq(&cold.files, &reverted.files));
    let warm = session.generate().unwrap();
    assert!(warm.changed_paths.is_empty());
    assert_eq!((warm.delta.compiles, warm.delta.renders), (0, 0));
}

fn checked_native(command: &mut std::process::Command, root: &Path) {
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("{command:?}: {error}"));
    let logs = root.join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    let index = std::fs::read_dir(&logs).unwrap().count();
    std::fs::write(
        logs.join(format!("{index:03}.log")),
        format!(
            "{command:?}\nstatus={}\n{}{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}\n{command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "new canonical Client/Maven coordinate and JVM public-metadata check; does not rerun accepted runtime matrix"]
fn native_canonical_package_and_compatibility_metadata_match_public_jvm_types() {
    let mut value = api();
    value["paths"]["/multi"] = json!({"get":{"operationId":"multipleStatuses","responses":{"200":response(json!({"type":"string"})),"201":response(json!({"type":"number"}))}}});
    value["components"]["schemas"]["Client"] = json!({"type":"object","properties":{"builder":{"type":"string"},"getClass":{"type":"string"},"a-b":{"type":"string"},"a_b":{"type":"string"}}});
    value["components"]["schemas"]["Thing"]["properties"]["client"] =
        json!({"$ref":"#/components/schemas/Client"});
    let (path, contract) = fixture(&value);
    let root = path.parent().unwrap();
    let native = snapshot(contract.clone(), &target());
    for file in backend::generate(contract.clone(), &selected(&contract), &target()).unwrap() {
        let path = root.join(file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, file.content).unwrap();
    }
    std::fs::write(
        root.join("native-snapshot.json"),
        serde_json::to_vec_pretty(&native).unwrap(),
    )
    .unwrap();
    let home = std::env::var_os("SUSPECT_JAVA_HOME")
        .or_else(|| std::env::var_os("JAVA_HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            "/Users/luke/.local/share/mise/installs/java/temurin-21.0.12+101.0.LTS".into()
        });
    let maven = std::env::var_os("SUSPECT_MAVEN_BIN").unwrap_or_else(|| {
        "/Users/luke/.local/share/mise/installs/maven/3.9.16/apache-maven-3.9.16/bin/mvn".into()
    });
    let repository = crate_root().join("../../target/sdk-java-maven-cache/java/repository");
    checked_native(
        std::process::Command::new(&maven)
            .args(["-B", "-q", "install"])
            .arg(format!("-Dmaven.repo.local={}", repository.display()))
            .env("JAVA_HOME", &home)
            .current_dir(root.join("java")),
        root,
    );
    let installed = repository.join("example/widgets/thing-sdk/1.0.0/thing-sdk-1.0.0.jar");
    assert_eq!(
        std::fs::read(&installed).unwrap(),
        std::fs::read(root.join("java/target/thing-sdk-1.0.0.jar")).unwrap(),
        "Maven installed group must be independent of example.sdk Java imports"
    );
    assert!(
        root.join("java/target/thing-sdk-1.0.0-sources.jar")
            .is_file()
    );
    assert!(
        root.join("java/target/thing-sdk-1.0.0-javadoc.jar")
            .is_file()
    );
    std::fs::write(
        root.join("NativeMetadata.java"),
        include_str!("../src/java_sdk/NativeMetadata.java"),
    )
    .unwrap();
    checked_native(
        std::process::Command::new(home.join("bin/javac"))
            .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
            .arg(&installed)
            .arg("NativeMetadata.java")
            .current_dir(root),
        root,
    );
    checked_native(
        std::process::Command::new(home.join("bin/java"))
            .args(["-ea", "-cp"])
            .arg(format!("{}:{}", installed.display(), root.display()))
            .args(["NativeMetadata", "native-snapshot.json"])
            .current_dir(root),
        root,
    );
    println!(
        "Java canonical integration/native metadata evidence: {}",
        root.display()
    );
}
