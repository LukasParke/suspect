#![cfg(feature = "ruby-sdk")]
//! Ruby through the same public generation, session and compatibility interfaces.

use std::{path::Path, sync::Arc};

use serde_json::{Value, json};
use suspect_codegen::{
    backend::{self, Backend, TargetConfig},
    compatibility::{self, PlanStatus},
    generation_session::{Session, SessionConfig},
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn api() -> Value {
    json!({"openapi":"3.1.0","info":{"title":"Things","version":"1"},
    "servers":[{"url":"https://api.example.test/v1"}],"security":[{"Bearer":[]}],
    "components":{"securitySchemes":{"Bearer":{"type":"http","scheme":"bearer"}},
        "schemas":{"Thing":{"type":"object","required":["name"],"additionalProperties":false,
            "properties":{"name":{"type":"string"},"note":{"type":["string","null"]}}}}},
    "paths":{"/things":{"post":{"operationId":"createThing",
        "requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Thing"}}}},
        "responses":{"201":{"description":"Created","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Thing"}}}}}
    }}}})
}

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::RubyHttp,
        package_name: "thing-sdk".into(),
        package_version: "1.0.0".into(),
        import_name: Some("ThingSDK".into()),
    }
}

fn load(path: &Path) -> Arc<Contract> {
    let workspace = Arc::new(WorkspaceBuilder::new().build().unwrap());
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap())
}

#[test]
fn canonical_generation_honors_ruby_package_and_namespace_identity() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(&path, api().to_string()).unwrap();
    let contract = load(&path);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let files = backend::generate(contract, &selected, &target()).unwrap();
    assert!(files.iter().all(|file| file.path.starts_with("ruby/")));
    assert!(
        files
            .iter()
            .any(|file| file.path == "ruby/lib/thing_sdk.rb")
    );
    assert!(
        files
            .iter()
            .any(|file| file.path == "ruby/thing-sdk.gemspec")
    );
    assert!(
        files
            .iter()
            .any(|file| file.path == "ruby/examples/quickstart.rb")
    );
    let plan = compatibility::snapshot(load(&path), &[], &[target()]).unwrap();
    let native = &plan.native[0];
    assert_eq!(native.status, PlanStatus::Planned, "{:?}", native.findings);
    assert!(native.findings.is_empty());
    assert_eq!(native.operations[0].symbols["client"], "ThingSDK::Client");
    assert_eq!(native.operations[0].symbols["method"], "create_thing");
    assert!(
        native
            .models
            .iter()
            .any(|model| model.name == "ThingSDK::Models::Thing" && model.role == "model")
    );
    assert!(
        native
            .runtime
            .fingerprinted_assets
            .contains(&"ruby_sdk/http.rb".to_owned())
    );
}

#[test]
fn ruby_sessions_reuse_the_contract_and_only_render_the_changed_target() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(&path, api().to_string()).unwrap();
    let mut config = SessionConfig {
        targets: vec![
            target(),
            TargetConfig {
                backend: Backend::TypescriptHttp,
                package_name: "thing-sdk".into(),
                package_version: "1.0.0".into(),
                import_name: None,
            },
        ],
        ..Default::default()
    };
    let mut session = Session::new(&path, config.clone()).unwrap();
    let cold = session.generate().unwrap();
    assert_eq!((cold.delta.compiles, cold.delta.renders), (1, 2));
    let warm = session.generate().unwrap();
    assert_eq!((warm.delta.compiles, warm.delta.renders), (0, 0));
    assert!(Arc::ptr_eq(&cold.files, &warm.files));
    config.targets[0].import_name = Some("RenamedSDK".into());
    session.set_config(config).unwrap();
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
        renamed
            .changed_paths
            .iter()
            .all(|path| path.starts_with("ruby/"))
    );
}

#[test]
fn ruby_native_compatibility_records_changed_constructor_requirements() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    let mut specification = api();
    std::fs::write(&path, specification.to_string()).unwrap();
    let before = load(&path);
    specification["components"]["schemas"]["Thing"]["required"] = json!(["name", "note"]);
    std::fs::write(&path, specification.to_string()).unwrap();
    let report = compatibility::compare(before, load(&path), &[], &[target()]).unwrap();
    assert!(!report.is_proven_compatible());
    let native = &report.native[0];
    assert_eq!(native.before.as_ref().unwrap().status, PlanStatus::Planned);
    assert_eq!(native.after.as_ref().unwrap().status, PlanStatus::Planned);
    assert!(!native.changes.is_empty());
    let model = native
        .after
        .as_ref()
        .unwrap()
        .models
        .iter()
        .find(|model| model.name == "ThingSDK::Models::Thing")
        .unwrap();
    let fields = model.descriptor.as_ref().unwrap()["fields"]
        .as_array()
        .unwrap();
    assert_eq!(
        fields.iter().find(|field| field["name"] == "note").unwrap()["required"],
        true
    );
}
