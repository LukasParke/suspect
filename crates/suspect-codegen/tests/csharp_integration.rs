#![cfg(feature = "csharp-sdk")]
//! C# through public backend/session/snapshot APIs; expected native obligations
//! are independent of the compatibility adapter and checked against native .NET metadata.
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde_json::{Value, json};
use suspect_codegen::{
    backend::{self, Backend, TargetConfig},
    compatibility::{self, CompatibilityReport, NativeModel, NativeSnapshot, PlanStatus},
    csharp_sdk::{
        self,
        models::{CsDecl, FieldInitialization},
    },
    generation_session::{Session, SessionConfig},
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
        let parent =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-csharp-integration");
        fs::create_dir_all(&parent).unwrap();
        let directory = tempfile::Builder::new()
            .prefix("case-")
            .tempdir_in(parent)
            .unwrap();
        let path = directory.path().join("api.json");
        let result = Self {
            _directory: directory,
            path,
        };
        result.write(value);
        result
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
        backend: Backend::CsharpHttp,
        package_name: "acme.widgets-sdk".into(),
        package_version: "1.0.0".into(),
        import_name: None,
    }
}
fn api() -> Value {
    json!({"openapi":"3.1.0","info":{"title":"C# integration","version":"1"},"servers":[{"url":"https://api.example.test/v1"}],"security":[{"getType":[]}],
    "components":{"securitySchemes":{"getType":{"type":"http","scheme":"bearer"},"get_type":{"type":"http","scheme":"bearer"}},"schemas":{
        "Kind":{"type":"string","const":"fixed"},"KindAlias":{"$ref":"#/components/schemas/Kind"},
        "NullableKind":{"type":["string","null"],"enum":["fixed",null]},
        "Count":{"type":"integer"},
        "Thing":{"type":"object","required":["kind","name","requiredNullable","requiredNullableKind"],"properties":{
            "kind":{"$ref":"#/components/schemas/KindAlias"},"optionalKind":{"$ref":"#/components/schemas/KindAlias"},
            "name":{"type":"string","minLength":1},"note":{"type":["string","null"]},"label":{"type":"string"},
            "requiredNullable":{"type":["string","null"]},"requiredNullableKind":{"$ref":"#/components/schemas/NullableKind"},
            "count":{"$ref":"#/components/schemas/Count"},"child":{"$ref":"#/components/schemas/Thing"},"choice":{"$ref":"#/components/schemas/Choice"}
        }},
        "A":{"type":"object","required":["a"],"properties":{"a":{"type":"string"}},"additionalProperties":false},
        "B":{"type":"object","required":["b"],"properties":{"b":{"type":"integer"}},"additionalProperties":false},
        "Choice":{"oneOf":[{"$ref":"#/components/schemas/A"},{"$ref":"#/components/schemas/B"}]},
        "Failure":{"type":"object","required":["message"],"properties":{"message":{"type":"string"}}}
    }},
    "paths":{
        "/things":{"get":{"operationId":"listThings","security":[{"get_type":[]}],"parameters":[{"name":"user-name","in":"query","schema":{"type":"string"}}],
            "responses":{"200":{"description":"Page","content":{"application/json":{"schema":{"type":"array","items":{"$ref":"#/components/schemas/Thing"}}}}}}}},
        "/things/{id}":{"patch":{"operationId":"upsertThing","parameters":[{"name":"id","in":"path","required":true,"schema":{"type":"string"}}],
            "requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Thing"}}}},
            "responses":{"200":{"description":"Updated","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Thing"}}}},
                "404":{"description":"Missing","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Failure"}}}}}}}
    }})
}
fn native(fixture: &Fixture, target: &TargetConfig) -> NativeSnapshot {
    let snapshot =
        compatibility::snapshot(fixture.load(), &[], std::slice::from_ref(target)).unwrap();
    let native = snapshot.native.into_iter().next().unwrap();
    assert_eq!(native.status, PlanStatus::Planned, "{:?}", native.findings);
    assert!(native.findings.is_empty(), "{:?}", native.findings);
    native
}
fn model<'a>(native: &'a NativeSnapshot, pointer: &str, role: &str) -> &'a NativeModel {
    native
        .models
        .iter()
        .find(|model| model.source.pointer == pointer && model.role == role)
        .unwrap_or_else(|| panic!("missing {pointer} / {role}"))
}
fn field<'a>(model: &'a NativeModel, name: &str) -> &'a Value {
    model.descriptor.as_ref().unwrap()["fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|field| field["name"] == name)
        .unwrap()
}
fn compare(before: Value, after: Value) -> CompatibilityReport {
    let fixture = Fixture::new(&before);
    let before = fixture.load();
    fixture.write(&after);
    compatibility::compare(before, fixture.load(), &[], &[target()]).unwrap()
}

#[test]
fn public_backend_and_snapshot_share_package_defaults_and_erased_native_types() {
    let fixture = Fixture::new(&api());
    let contract = fixture.load();
    let selected = contract
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    let files = backend::generate(contract, &selected, &target()).unwrap();
    assert!(files.iter().all(|file| file.path.starts_with("csharp/")));
    let manifest: Value = serde_json::from_str(
        &files
            .iter()
            .find(|file| file.path == "csharp/http-manifest.json")
            .unwrap()
            .content,
    )
    .unwrap();
    assert_eq!(manifest["package"]["namespace"], "Acme.WidgetsSdk");
    assert_eq!(manifest["package"]["name"], "acme.widgets-sdk");
    let snapshot = native(&fixture, &target());
    assert!(
        snapshot
            .operations
            .iter()
            .all(|op| op.symbols["client"] == "Acme.WidgetsSdk.Client")
    );
    assert_eq!(
        model(&snapshot, "/components/schemas/KindAlias", "erased-alias").name,
        "global::Acme.WidgetsSdk.Kind"
    );
    assert!(
        !snapshot
            .models
            .iter()
            .any(|m| m.role == "model" && m.name == "Acme.WidgetsSdk.KindAlias")
    );
    assert_eq!(
        model(&snapshot, "/components/schemas/Count", "erased-alias").name,
        "global::Acme.WidgetsSdk.JsonInteger"
    );
    let decode = model(&snapshot, "/components/schemas/KindAlias", "codec-decode");
    assert_eq!(decode.name, "Acme.WidgetsSdk.Codecs.DecodeKindAlias");
    assert_eq!(
        decode.descriptor.as_ref().unwrap()["overloads"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        model(&snapshot, "/components/schemas/KindAlias", "codec-encode").name,
        "Acme.WidgetsSdk.Codecs.EncodeKindAlias"
    );
    assert!(
        snapshot
            .models
            .iter()
            .all(|m| m.descriptor.is_some() && m.source.span.is_some())
    );
}

#[test]
fn construction_descriptor_uses_the_emitters_singleton_and_nullability_decisions() {
    let fixture = Fixture::new(&api());
    let contract = fixture.load();
    let selected = contract
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    let plan = csharp_sdk::plan_sdk(
        contract,
        &selected,
        csharp_sdk::SdkConfig {
            name: "acme.widgets-sdk".into(),
            version: "1.0.0".into(),
            namespace: "Acme.WidgetsSdk".into(),
        },
    )
    .unwrap();
    let fields = plan
        .models()
        .declarations()
        .iter()
        .find_map(|(key, decl)| match decl {
            CsDecl::Record { fields, .. } if key.0.pointer() == "/components/schemas/Thing" => {
                Some(fields)
            }
            _ => None,
        })
        .unwrap();
    let source_field = |name: &str| fields.iter().find(|f| f.name == name).unwrap();
    assert!(source_field("Kind").required);
    assert!(
        matches!(plan.models().field_initialization(source_field("Kind")),FieldInitialization::Singleton {ref member,..} if member=="Fixed")
    );
    assert_eq!(
        plan.models()
            .field_initialization(source_field("OptionalKind")),
        FieldInitialization::Absent
    );
    assert_eq!(
        plan.models()
            .field_initialization(source_field("RequiredNullableKind")),
        FieldInitialization::Required
    );
    let snapshot = native(&fixture, &target());
    let thing = model(&snapshot, "/components/schemas/Thing", "model");
    assert_eq!(field(thing, "Kind")["sourceRequired"], true);
    assert_eq!(field(thing, "Kind")["required"], false);
    assert_eq!(
        field(thing, "Kind")["initialization"],
        json!({"kind":"literal","type":"Acme.WidgetsSdk.Kind","member":"Fixed","token":"\"fixed\""})
    );
    assert_eq!(field(thing, "RequiredNullableKind")["required"], true);
    assert_eq!(
        field(thing, "RequiredNullableKind")["type"]["kind"],
        "nullable"
    );
    assert_eq!(field(thing, "Note")["type"]["kind"], "optional");
    assert_eq!(field(thing, "Note")["type"]["type"]["kind"], "nullable");
    assert_eq!(field(thing, "Label")["setter"], "set");
    assert_eq!(field(thing, "Label")["readonly"], false);
    assert_eq!(
        field(thing, "Extra")["initialization"]["kind"],
        "empty-dictionary"
    );
    assert_eq!(
        thing.descriptor.as_ref().unwrap()["constructor"]["requiredMembers"],
        json!(["Name", "RequiredNullable", "RequiredNullableKind"])
    );
}

#[test]
fn union_roles_get_only_results_and_init_only_credentials_are_real_symbols() {
    let fixture = Fixture::new(&api());
    let snapshot = native(&fixture, &target());
    let arm = model(&snapshot, "/components/schemas/Choice/oneOf/0", "union-arm");
    assert_eq!(arm.name, "Acme.WidgetsSdk.Choice.A");
    assert_eq!(field(arm, "Value")["type"]["name"], "Acme.WidgetsSdk.A");
    assert_eq!(field(arm, "Value")["setter"], Value::Null);
    assert_eq!(field(arm, "Value")["readonly"], true);
    assert_eq!(
        arm.descriptor.as_ref().unwrap()["constructor"]["access"],
        "public"
    );
    let upsert = snapshot
        .operations
        .iter()
        .find(|o| o.operation_id == "upsertThing")
        .unwrap();
    assert_eq!(
        upsert.descriptor["responseUnions"]["success"]["directResult"],
        true
    );
    assert_eq!(
        upsert.descriptor["responses"][0]["class"],
        "Acme.WidgetsSdk.UpsertThingResult"
    );
    assert_eq!(
        upsert.descriptor["responses"][0]["fields"][0]["readonly"],
        true
    );
    assert_eq!(
        upsert.descriptor["responses"][0]["constructor"]["access"],
        "internal"
    );
    assert_eq!(
        upsert.descriptor["responses"][1]["class"],
        "Acme.WidgetsSdk.UpsertThingApiException.Status404"
    );
    assert_eq!(
        upsert.descriptor["responses"][1]["fields"][0]["hidesInheritedMember"],
        true
    );
    let list = snapshot
        .operations
        .iter()
        .find(|o| o.operation_id == "listThings")
        .unwrap();
    assert_eq!(
        list.descriptor["parameters"]["overloads"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        list.descriptor["parameters"]["overloads"][1]["parameters"][0]["type"]["name"],
        "System.Threading.CancellationToken"
    );
    assert_eq!(
        upsert.descriptor["parameters"]["overloads"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        upsert.descriptor["parameters"]["overloads"][0]["parameters"][1]["type"]["type"]["name"],
        "Acme.WidgetsSdk.RequestOptions"
    );
    let credentials = snapshot
        .models
        .iter()
        .filter(|m| m.role == "credential-property")
        .collect::<Vec<_>>();
    assert_eq!(credentials.len(), 2);
    assert!(
        credentials
            .iter()
            .any(|c| c.name == "Acme.WidgetsSdk.Credentials.GetType2")
    );
    assert!(
        credentials
            .iter()
            .any(|c| c.name == "Acme.WidgetsSdk.Credentials.GetType3")
    );
    assert!(
        credentials
            .iter()
            .all(|c| c.descriptor.as_ref().unwrap()["setter"] == "init"
                && c.descriptor.as_ref().unwrap()["required"] == false)
    );
}

#[test]
fn unchanged_docs_and_relocated_sources_have_no_spurious_native_findings() {
    let before = api();
    let mut after = before.clone();
    after["info"]["description"] = json!("A longer guide");
    after["components"]["schemas"]["Thing"]["description"] =
        json!("Changed source offsets must not become native API changes");
    after["components"]["schemas"]["Thing"]["properties"]["label"]["default"] =
        json!("not inserted");
    let report = compare(before.clone(), after);
    assert!(
        report.native[0].changes.is_empty(),
        "{:?}",
        report.native[0].changes
    );
    assert!(report.native[0].after.as_ref().unwrap().findings.is_empty());
    let a = Fixture::new(&before);
    let b = Fixture::new(&before);
    let a = compatibility::snapshot(a.load(), &[], &[target()]).unwrap();
    let b = compatibility::snapshot(b.load(), &[], &[target()]).unwrap();
    let report = compatibility::compare_snapshots(&a, &b);
    assert!(report.is_proven_compatible(), "{:?}", report.native);
}

#[test]
fn constraint_and_wire_spelling_changes_do_not_invent_native_type_changes() {
    let before = api();
    let mut after = before.clone();
    after["components"]["schemas"]["Thing"]["properties"]["name"]["minLength"] = json!(3);
    let report = compare(before.clone(), after);
    assert!(!report.wire.is_empty());
    assert!(
        report.native[0].changes.is_empty(),
        "{:?}",
        report.native[0].changes
    );
    let mut after = before.clone();
    after["paths"]["/things"]["get"]["parameters"][0]["name"] = json!("user_name");
    let report = compare(before, after);
    assert!(!report.wire.is_empty());
    assert!(
        report.native[0].changes.is_empty(),
        "{:?}",
        report.native[0].changes
    );
}

#[test]
fn null_presence_singleton_and_union_changes_retain_native_migration_evidence() {
    let before = api();
    let mut after = before.clone();
    after["components"]["schemas"]["Kind"] = json!({"type":"string","enum":["fixed","other"]});
    let report = compare(before.clone(), after);
    let changed = report.native[0]
        .changes
        .iter()
        .find(|c| c.code == "native-model-shape-changed" && c.subject == "Acme.WidgetsSdk.Thing")
        .unwrap();
    assert_eq!(
        changed.before.as_ref().unwrap()["constructor"]["requiredMembers"],
        json!(["Name", "RequiredNullable", "RequiredNullableKind"])
    );
    assert_eq!(
        changed.after.as_ref().unwrap()["constructor"]["requiredMembers"],
        json!(["Kind", "Name", "RequiredNullable", "RequiredNullableKind"])
    );
    for (wire_name, replacement) in [
        ("label", json!({"type":["string","null"]})),
        ("note", json!({"type":"string"})),
    ] {
        let mut after = before.clone();
        after["components"]["schemas"]["Thing"]["properties"][wire_name] = replacement;
        let report = compare(before.clone(), after);
        assert!(report.native[0].changes.iter().any(
            |c| c.code == "native-model-shape-changed" && c.subject == "Acme.WidgetsSdk.Thing"
        ));
        assert!(!report.wire.is_empty());
    }
    let mut after = before.clone();
    after["components"]["schemas"]["Thing"]["required"]
        .as_array_mut()
        .unwrap()
        .push(json!("note"));
    let report = compare(before.clone(), after);
    let thing = model(
        report.native[0].after.as_ref().unwrap(),
        "/components/schemas/Thing",
        "model",
    );
    assert_eq!(field(thing, "Note")["required"], true);
    assert_eq!(field(thing, "Note")["type"]["kind"], "nullable");
    let mut after = before.clone();
    after["components"]["schemas"]["Choice"]["oneOf"]
        .as_array_mut()
        .unwrap()
        .reverse();
    let report = compare(before, after);
    assert!(
        report.native[0].changes.iter().any(
            |c| c.code == "native-model-shape-changed" && c.subject == "Acme.WidgetsSdk.Choice"
        )
    );
    assert!(
        report.native[0]
            .after
            .as_ref()
            .unwrap()
            .models
            .iter()
            .any(|m| m.role == "union-arm"
                && m.source.pointer.ends_with("/oneOf/0")
                && m.name.ends_with(".Choice.B"))
    );
}

#[test]
fn operation_names_no_input_and_direct_success_shapes_are_compared() {
    let before = api();
    let mut after = before.clone();
    after["paths"]["/things"]["get"]["operationId"] = json!("readThings");
    let report = compare(before.clone(), after);
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-operation-symbol-changed"
                && c.subject == "method"
                && c.after == Some(json!("ReadThingsAsync")))
    );
    let mut after = before.clone();
    after["paths"]["/things"]["get"]["parameters"][0]["required"] = json!(true);
    let report = compare(before.clone(), after);
    let list = report.native[0]
        .after
        .as_ref()
        .unwrap()
        .operations
        .iter()
        .find(|o| o.operation_id == "listThings")
        .unwrap();
    assert_eq!(
        list.descriptor["parameters"]["overloads"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-input-changed")
    );
    let mut after = before.clone();
    after["paths"]["/things/{id}"]["patch"]["responses"]["201"] =
        before["paths"]["/things/{id}"]["patch"]["responses"]["200"].clone();
    let report = compare(before, after);
    let operation = report.native[0]
        .after
        .as_ref()
        .unwrap()
        .operations
        .iter()
        .find(|o| o.operation_id == "upsertThing")
        .unwrap();
    assert_eq!(
        operation.descriptor["responseUnions"]["success"]["directResult"],
        false
    );
    assert_eq!(
        operation.descriptor["responses"][0]["class"],
        "Acme.WidgetsSdk.UpsertThingResult.Status200"
    );
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-responses-changed")
    );
}

#[test]
fn package_namespace_and_source_obsolete_attribute_changes_are_visible() {
    let fixture = Fixture::new(&api());
    let old = target();
    let mut new = old.clone();
    new.import_name = Some("Another.Widgets".into());
    let report = compatibility::compare_with_targets(
        fixture.load(),
        fixture.load(),
        &[],
        std::slice::from_ref(&old),
        &[new],
    )
    .unwrap();
    assert!(report.wire.is_empty());
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-operation-symbol-changed" && c.subject == "client")
    );
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-model-renamed")
    );
    let mut explicit = old.clone();
    explicit.import_name = Some("Acme.WidgetsSdk".into());
    let report = compatibility::compare_with_targets(
        fixture.load(),
        fixture.load(),
        &[],
        &[old],
        std::slice::from_ref(&explicit),
    )
    .unwrap();
    assert!(report.native[0].changes.is_empty());
    let mut renamed = explicit.clone();
    renamed.package_name = "another-package".into();
    let report = compatibility::compare_with_targets(
        fixture.load(),
        fixture.load(),
        &[],
        &[explicit],
        &[renamed],
    )
    .unwrap();
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-package-name-changed")
    );
    let mut after = api();
    after["components"]["schemas"]["Thing"]["deprecated"] = json!(true);
    let report = compare(api(), after);
    assert!(report.native[0].changes.iter().any(|c|c.code=="native-model-shape-changed" && c.subject=="Acme.WidgetsSdk.Thing"));
}

#[test]
fn sessions_reuse_the_contract_and_rerender_only_the_csharp_configuration() {
    let fixture = Fixture::new(&api());
    let mut config = SessionConfig {
        targets: vec![
            target(),
            TargetConfig {
                backend: Backend::TypescriptHttp,
                package_name: "widget-sdk".into(),
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
    assert_eq!((warm.delta.compiles, warm.delta.renders), (0, 0));
    assert!(Arc::ptr_eq(&first.files, &warm.files));
    config.targets[0].import_name = Some("Renamed.Widgets".into());
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
    assert!(Arc::ptr_eq(&first.contract, &renamed.contract));
    assert!(
        renamed
            .changed_paths
            .iter()
            .all(|p| p.starts_with("csharp/"))
    );
    config.targets[0] = target();
    session.set_config(config).unwrap();
    let reverted = session.generate().unwrap();
    assert_eq!(reverted.delta.renders, 0);
    assert!(Arc::ptr_eq(&first.files, &reverted.files));
}

#[test]
fn unavailable_package_or_schema_is_located_instead_of_an_incomplete_native_snapshot() {
    let fixture = Fixture::new(&api());
    let mut invalid = target();
    invalid.import_name = Some("Not.A-Namespace".into());
    let snapshot = compatibility::snapshot(fixture.load(), &[], &[invalid]).unwrap();
    assert_eq!(snapshot.native[0].status, PlanStatus::Unavailable);
    assert!(
        snapshot.native[0]
            .findings
            .iter()
            .any(|f| f.code == "http-packaging-identity"
                && f.source.as_ref().is_some_and(|s| s.span.is_some()))
    );
    let mut value = api();
    value["components"]["schemas"]["Thing"]["properties"]["unsupported"] = json!({"enum":["x",1]});
    fixture.write(&value);
    let snapshot = compatibility::snapshot(fixture.load(), &[], &[target()]).unwrap();
    assert_eq!(snapshot.native[0].status, PlanStatus::Unavailable);
    assert!(
        snapshot.native[0]
            .findings
            .iter()
            .any(|f| f.code == "csharp-literal-representation-unsupported")
    );
    assert!(snapshot.native[0].models.is_empty() && snapshot.native[0].operations.is_empty());
}

#[test]
#[ignore = "narrow .NET 8 metadata/constructor gate for the new snapshot descriptors; preserves native evidence"]
fn native_package_metadata_agrees_with_snapshot_construction_and_accessors() {
    use std::process::Command;
    let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-csharp-integration");
    fs::create_dir_all(&parent).unwrap();
    let parent = fs::canonicalize(parent).unwrap();
    let root = tempfile::Builder::new()
        .prefix("native-descriptors-")
        .tempdir_in(parent)
        .unwrap()
        .keep();
    fs::write(
        root.join("global.json"),
        r#"{"sdk":{"version":"8.0.424","rollForward":"disable"}}"#,
    )
    .unwrap();
    fs::create_dir(root.join("feed")).unwrap();
    fs::write(root.join("NuGet.Config"),"<configuration><packageSources><clear/><add key=\"local\" value=\"feed\"/></packageSources></configuration>").unwrap();
    let run = |directory: &Path, label: &str, args: &[&str], success: bool| {
        let mut command = Command::new(
            std::env::var_os("SUSPECT_DOTNET_BIN")
                .unwrap_or_else(|| "/Users/luke/.local/share/mise/dotnet-root/dotnet".into()),
        );
        command
            .args(args)
            .current_dir(directory)
            .env("DOTNET_NOLOGO", "1")
            .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
            .env("DOTNET_CLI_HOME", root.join("dotnet-home"))
            .env("NUGET_PACKAGES", root.join("nuget-cache"));
        let output = command.output().unwrap();
        fs::write(
            root.join(format!("{label}.log")),
            [output.stdout.as_slice(), output.stderr.as_slice()].concat(),
        )
        .unwrap();
        fs::write(
            root.join(format!("{label}.command.json")),
            json!({"command":format!("{command:?}"),"status":output.status.code()}).to_string(),
        )
        .unwrap();
        assert_eq!(
            output.status.success(),
            success,
            "{label} in {}\n{}{}",
            root.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };
    let fixture = Fixture::new(&api());
    for (case, singleton) in [("singleton", true), ("required", false)] {
        let mut source = api();
        if !singleton {
            source["components"]["schemas"]["Kind"] =
                json!({"type":"string","enum":["fixed","other"]});
        }
        fixture.write(&source);
        let contract = fixture.load();
        let selected = contract
            .operations()
            .map(|o| o.source().clone())
            .collect::<Vec<_>>();
        let target = TargetConfig {
            backend: Backend::CsharpHttp,
            package_name: "Native.Descriptors".into(),
            package_version: if singleton { "1.0.0" } else { "2.0.0" }.into(),
            import_name: Some("NativeCases".into()),
        };
        let directory = root.join(case);
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("source.openapi.json"),
            serde_json::to_vec_pretty(&source).unwrap(),
        )
        .unwrap();
        suspect_codegen::write_files(
            &backend::generate(contract, &selected, &target).unwrap(),
            &directory,
        )
        .unwrap();
        let snapshot = native(&fixture, &target);
        fs::write(
            directory.join("snapshot.json"),
            serde_json::to_vec_pretty(&snapshot).unwrap(),
        )
        .unwrap();
        run(
            &directory.join("csharp"),
            &format!("{case}-restore"),
            &["restore", "--configfile", "../../NuGet.Config"],
            true,
        );
        run(
            &directory.join("csharp"),
            &format!("{case}-pack"),
            &["pack", "-c", "Release", "--no-restore", "-o", "../../feed"],
            true,
        );
        let consumer = directory.join("consumer");
        fs::create_dir(&consumer).unwrap();
        fs::write(consumer.join("Consumer.csproj"),format!("<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net8.0</TargetFramework><OutputType>Exe</OutputType><LangVersion>12.0</LangVersion><Nullable>enable</Nullable><ImplicitUsings>enable</ImplicitUsings><TreatWarningsAsErrors>true</TreatWarningsAsErrors></PropertyGroup><ItemGroup><PackageReference Include=\"Native.Descriptors\" Version=\"[{}]\" /></ItemGroup></Project>",target.package_version)).unwrap();
        fs::write(consumer.join("Program.cs"), NATIVE_METADATA).unwrap();
        run(
            &consumer,
            &format!("{case}-consumer-restore"),
            &["restore", "--configfile", "../../NuGet.Config"],
            true,
        );
        run(
            &consumer,
            &format!("{case}-metadata"),
            &[
                "run",
                "-c",
                "Release",
                "--no-restore",
                "--",
                "../snapshot.json",
            ],
            true,
        );
        fs::write(consumer.join("Construction.cs"),"using NativeCases; public static class Construction { public static Thing Make() => new Thing { Name = \"ok\", RequiredNullable = null, RequiredNullableKind = null }; }\n").unwrap();
        let output = run(
            &consumer,
            &format!("{case}-construction"),
            &["build", "-c", "Release", "--no-restore"],
            singleton,
        );
        if !singleton {
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("CS9035"),
                "missing required Kind must be the rejection"
            );
        }
    }
    fs::write(root.join("PASS.json"),json!({"gate":"csharp-native-snapshot-metadata","sdk":"8.0.424","cases":["singleton","required"],"result":"passed"}).to_string()).unwrap();
    println!("C# snapshot native metadata evidence: {}", root.display());
}

const NATIVE_METADATA: &str = r#"
using NativeCases;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Text.Json;
var assembly = typeof(Client).Assembly;
var checks = 0;
void Check(bool condition, string label) { if (!condition) throw new Exception(label); checks++; }
Type Nominal(string name) => assembly.GetTypes().FirstOrDefault(t => t.FullName!.Replace('+', '.') == name) ?? Type.GetType(name, throwOnError: true)!;
Type Resolve(JsonElement type)
{
    var kind = type.GetProperty("kind").GetString();
    if (kind == "nullable") { var inner = Resolve(type.GetProperty("type")); return inner.IsValueType ? typeof(Nullable<>).MakeGenericType(inner) : inner; }
    if (kind == "optional") return Nominal(type.GetProperty("name").GetString()! + "`1").MakeGenericType(Resolve(type.GetProperty("type")));
    if (kind == "array") return Resolve(type.GetProperty("type")).MakeArrayType();
    if (kind == "generic") { var args = type.GetProperty("arguments").EnumerateArray().Select(Resolve).ToArray(); return Nominal(type.GetProperty("name").GetString()! + "`" + args.Length).MakeGenericType(args); }
    return type.GetProperty("name").GetString() switch { "string" => typeof(string), "bool" => typeof(bool), "byte" => typeof(byte), "int" => typeof(int), "System.Text.Json.JsonElement" => typeof(JsonElement), var name => Nominal(name!) };
}
void Property(Type owner, JsonElement descriptor)
{
    var name = descriptor.GetProperty("name").GetString()!; var property = owner.GetProperty(name, BindingFlags.Public | BindingFlags.Instance | BindingFlags.DeclaredOnly) ?? owner.GetProperty(name);
    Check(property is not null, owner + "." + name);
    Check(property!.PropertyType == Resolve(descriptor.GetProperty("type")), "type " + owner + "." + name);
    Check(property.GetMethod?.IsPublic == true, "getter " + name);
    Check((property.SetMethod is null) == descriptor.GetProperty("readonly").GetBoolean(), "readonly " + name);
    if (descriptor.TryGetProperty("required", out var required)) Check(property.IsDefined(typeof(RequiredMemberAttribute)) == required.GetBoolean(), "required keyword " + name);
    var setter = descriptor.GetProperty("setter");
    if (setter.ValueKind != JsonValueKind.Null)
        Check(property.SetMethod!.ReturnParameter.GetRequiredCustomModifiers().Contains(typeof(IsExternalInit)) == (setter.GetString() == "init"), "init/set " + name);
}
using var snapshot = JsonDocument.Parse(File.ReadAllBytes(args[0]));
Check(snapshot.RootElement.GetProperty("findings").GetArrayLength() == 0, "complete descriptors");
foreach (var model in snapshot.RootElement.GetProperty("models").EnumerateArray())
{
    var role = model.GetProperty("role").GetString(); var name = model.GetProperty("name").GetString()!; var description = model.GetProperty("descriptor");
    if (role is "model" or "union-arm")
    {
        var type = Nominal(name); var kind = description.GetProperty("kind").GetString();
        if (kind == "record") { Check(type.IsSealed && type.GetConstructor(Type.EmptyTypes) is not null, "record constructor"); foreach (var property in description.GetProperty("fields").EnumerateArray()) Property(type, property); }
        if (kind == "union") Check(type.IsAbstract && type.GetConstructors(BindingFlags.Public | BindingFlags.Instance).Length == 0, "closed union");
        if (kind == "class") { Check(type.IsSealed, "sealed arm"); Property(type,description.GetProperty("fields")[0]); Check(type.GetConstructors().Single().GetParameters().Single().ParameterType == Resolve(description.GetProperty("constructor").GetProperty("parameters")[0].GetProperty("type")), "arm constructor"); }
        if (kind == "enum") foreach (var member in description.GetProperty("members").EnumerateArray()) Check((int)type.GetField(member.GetProperty("name").GetString()!)!.GetRawConstantValue()! == member.GetProperty("value").GetInt32(), "enum ordinal");
    }
    if (role == "credential-property") Property(typeof(Credentials),description);
    if (role == "codec-decode")
    {
        var method = name.Substring(name.LastIndexOf('.') + 1); var overloads = typeof(Codecs).GetMethods().Where(m => m.Name == method).ToArray();
        Check(overloads.Length == 2 && overloads.All(m => m.IsStatic && m.ReturnType == Resolve(description.GetProperty("returns"))), "decode overloads");
        foreach (var overload in description.GetProperty("overloads").EnumerateArray()) Check(overloads.Any(m => m.GetParameters().Single().ParameterType == Resolve(overload.GetProperty("parameters")[0].GetProperty("type"))), "decode parameter");
    }
    if (role == "codec-encode")
    {
        var method = typeof(Codecs).GetMethod(name.Substring(name.LastIndexOf('.') + 1))!;
        Check(method.IsStatic && method.ReturnType == typeof(byte[]) && method.GetParameters().Single().ParameterType == Resolve(description.GetProperty("parameters")[0].GetProperty("type")), "encode signature");
    }
}
foreach (var operation in snapshot.RootElement.GetProperty("operations").EnumerateArray())
{
    var descriptor=operation.GetProperty("descriptor"); var methodName=operation.GetProperty("symbols").GetProperty("method").GetString();
    var methods=typeof(Client).GetMethods().Where(m=>m.Name==methodName).ToArray();
    Check(methods.Length==descriptor.GetProperty("parameters").GetProperty("overloads").GetArrayLength(), "Task overload count");
    var inputType=Nominal(operation.GetProperty("symbols").GetProperty("input").GetString()!);
    foreach(var field in descriptor.GetProperty("constructor").GetProperty("inputDeclaration").GetProperty("fields").EnumerateArray()) Property(inputType,field);
    foreach(var overload in descriptor.GetProperty("parameters").GetProperty("overloads").EnumerateArray())
    {
        var parameters=overload.GetProperty("parameters").EnumerateArray().ToArray();
        var method=methods.Single(m=>m.GetParameters().Select(p=>p.ParameterType).SequenceEqual(parameters.Select(p=>Resolve(p.GetProperty("type")))));
        Check(method.ReturnType==Resolve(overload.GetProperty("returns")),"Task result signature");
        for(var i=0;i<parameters.Length;i++) Check(method.GetParameters()[i].Name==parameters[i].GetProperty("name").GetString() && method.GetParameters()[i].HasDefaultValue!=parameters[i].GetProperty("required").GetBoolean(),"native parameter name/default");
    }
    foreach(var response in descriptor.GetProperty("responses").EnumerateArray())
    {
        var type=Nominal(response.GetProperty("class").GetString()!); Check(type.IsSealed,"sealed response"); Property(type,response.GetProperty("fields")[0]);
        Check(type.GetConstructors().Length==0 && type.GetConstructors(BindingFlags.NonPublic|BindingFlags.Instance).Any(c=>c.IsAssembly),"internal result constructor");
    }
}
Check(assembly.GetType("NativeCases.KindAlias")==null,"aliases are erased, not invented classes");
Console.WriteLine($"C# metadata PASS: {checks} independent CLR checks; {System.Runtime.InteropServices.RuntimeInformation.FrameworkDescription}");
"#;
