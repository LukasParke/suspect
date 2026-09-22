use std::{path::Path, sync::Arc};
use suspect_codegen::{
    backend::{Backend, TargetConfig},
    generation_session::{Session, SessionConfig},
};
fn config() -> SessionConfig {
    SessionConfig {
        targets: vec![
            TargetConfig {
                backend: Backend::TypescriptHttp,
                package_name: "test-sdk".into(),
                package_version: "0.0.0".into(),
                import_name: None,
            },
            TargetConfig {
                backend: Backend::RustHttp,
                package_name: "test-sdk".into(),
                package_version: "0.0.0".into(),
                import_name: None,
            },
            TargetConfig {
                backend: Backend::PythonHttp,
                package_name: "test-sdk".into(),
                package_version: "0.0.0".into(),
                import_name: Some("test_sdk".into()),
            },
            TargetConfig {
                backend: Backend::GoHttp,
                package_name: "example.com/test-sdk".into(),
                package_version: "0.0.0".into(),
                import_name: None,
            },
            TargetConfig {
                backend: Backend::SwiftHttp,
                package_name: "TestSdk".into(),
                package_version: "0.0.0".into(),
                import_name: None,
            },
        ],
        ..Default::default()
    }
}
fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m2");
    for name in ["split.openapi.yaml", "split-resources.yaml"] {
        std::fs::copy(source.join(name), dir.path().join(name)).unwrap();
    }
    let entry = dir.path().join("split.openapi.yaml");
    (dir, entry)
}
#[test]
fn one_snapshot_drives_all_backends_and_unchanged_runs_reuse_everything() {
    let (dir, entry) = fixture();
    let mut session = Session::new(&entry, config()).unwrap();
    let first = session.generate().unwrap();
    assert_eq!(first.delta.compiles, 1);
    assert_eq!(first.delta.renders, 5);
    for path in [
        "typescript/package.json",
        "rust/Cargo.toml",
        "python/pyproject.toml",
        "go/go.mod",
        "swift/Package.swift",
    ] {
        assert!(first.files.iter().any(|file| file.path == path), "{path}");
    }
    let next = session.generate().unwrap();
    assert_eq!(next.delta.compiles, 0);
    assert_eq!(next.delta.renders, 0);
    assert_eq!(next.delta.cache_hits, 1);
    assert!(next.changed_paths.is_empty());
    assert!(Arc::ptr_eq(&first.contract, &next.contract));
    assert!(Arc::ptr_eq(&first.files, &next.files));
    let output = dir.path().join("output");
    session.write(&next, &output).unwrap();
    let before = std::fs::metadata(output.join("typescript/package.json"))
        .unwrap()
        .modified()
        .unwrap();
    let fresh = session.generate().unwrap();
    session.write(&fresh, &output).unwrap();
    assert_eq!(
        before,
        std::fs::metadata(output.join("typescript/package.json"))
            .unwrap()
            .modified()
            .unwrap()
    );
}
#[test]
fn external_changes_invalidate_and_reverts_reuse_the_original_owned_snapshot() {
    let (dir, entry) = fixture();
    let external = dir.path().join("split-resources.yaml");
    let original = std::fs::read_to_string(&external).unwrap();
    let mut session = Session::new(&entry, config()).unwrap();
    let first = session.generate().unwrap();
    std::fs::write(&external, original.replace("minLength: 1", "minLength: 2")).unwrap();
    let changed = session.generate().unwrap();
    assert_eq!(changed.delta.compiles, 1);
    assert!(!changed.changed_paths.is_empty());
    assert!(!Arc::ptr_eq(&first.contract, &changed.contract));
    std::fs::write(&external, original).unwrap();
    let reverted = session.generate().unwrap();
    assert_eq!(reverted.delta.compiles, 0);
    assert_eq!(reverted.delta.cache_hits, 1);
    assert!(Arc::ptr_eq(&first.contract, &reverted.contract));
    assert_eq!(reverted.revision, first.revision);
    assert_ne!(reverted.revision, changed.revision);
    std::fs::remove_file(&external).unwrap();
    assert!(session.generate().is_err());
}

#[test]
fn finite_cache_eviction_recompiles_without_changing_output_ownership() {
    let (dir, entry) = fixture();
    let external = dir.path().join("split-resources.yaml");
    let original = std::fs::read_to_string(&external).unwrap();
    let mut settings = config();
    settings.cache_entries = 1;
    let mut session = Session::new(&entry, settings).unwrap();
    let first = session.generate().unwrap();
    std::fs::write(&external, original.replace("minLength: 1", "minLength: 2")).unwrap();
    session.generate().unwrap();
    std::fs::write(&external, original).unwrap();
    let evicted = session.generate().unwrap();
    assert_eq!(evicted.delta.compiles, 1);
    assert_eq!(evicted.files, first.files);
    assert!(!Arc::ptr_eq(&first.contract, &evicted.contract));
    let mut settings = config();
    settings.cache_bytes = 1;
    session.set_config(settings).unwrap();
    let uncached = session.generate().unwrap();
    let next = session.generate().unwrap();
    assert_eq!(next.delta.compiles, 1);
    assert_eq!(next.files, uncached.files);
    assert!(next.changed_paths.is_empty());
}

#[test]
fn removing_a_target_deletes_only_its_owned_unedited_artifacts() {
    let (dir, entry) = fixture();
    let mut session = Session::new(&entry, config()).unwrap();
    let output = dir.path().join("output");
    let before = session.generate().unwrap();
    session.write(&before, &output).unwrap();
    let mut settings = config();
    settings
        .targets
        .retain(|target| target.backend != Backend::GoHttp);
    session.set_config(settings).unwrap();
    let after = session.generate().unwrap();
    assert_eq!(after.delta.compiles, 0);
    assert_eq!(after.delta.renders, 0);
    assert!(after.changed_paths.contains(&"go/go.mod".into()));
    session.write(&after, &output).unwrap();
    assert!(!output.join("go/go.mod").exists());
    assert!(output.join("python/pyproject.toml").exists());
}

#[test]
fn missing_and_unresolved_example_dependencies_recover_without_touching_the_entry() {
    let root = tempfile::tempdir().unwrap();
    let entry = root.path().join("api.json");
    let example = root.path().join("example.json");
    let source = serde_json::json!({"openapi":"3.1.0","info":{"title":"Missing example","version":"1"},"servers":[{"url":"https://example.test"}],"security":[{"key":[]}],"components":{"securitySchemes":{"key":{"type":"http","scheme":"bearer"}}},"paths":{"/value":{"get":{"operationId":"getValue","responses":{"200":{"description":"Value","content":{"application/json":{"schema":{"type":"string"},"examples":{"external":{"$ref":"example.json#/entry"}}}}}}}}}});
    std::fs::write(&entry, source.to_string()).unwrap();
    let mut settings = config();
    settings
        .targets
        .retain(|target| target.backend == Backend::PythonHttp);
    let mut session = Session::new(&entry, settings).unwrap();
    let values =
        |output: &suspect_codegen::generation_session::SessionOutput| -> serde_json::Value {
            serde_json::from_str(
                &output
                    .files
                    .iter()
                    .find(|file| file.path == "python/src/test_sdk/examples.json")
                    .unwrap()
                    .content,
            )
            .unwrap()
        };
    let absent = session.generate().unwrap();
    assert_eq!(
        values(&absent)["operations"][0]["entries"][0]["origin"],
        "synthesized"
    );
    assert_eq!(session.generate().unwrap().delta.compiles, 0);
    std::fs::write(&example, r#"{"entry":{"value":"declared-value"}}"#).unwrap();
    let created = session.generate().unwrap();
    assert_eq!(created.delta.compiles, 1);
    assert!(
        created
            .new_documents
            .contains(&example.display().to_string())
    );
    assert_eq!(
        values(&created)["operations"][0]["entries"][0]["value"],
        "declared-value"
    );
    assert_ne!(absent.revision, created.revision);
    std::fs::remove_file(&example).unwrap();
    let reverted = session.generate().unwrap();
    assert_eq!(reverted.delta.compiles, 0);
    assert_eq!(reverted.revision, absent.revision);
    std::fs::write(
        &example,
        r#"{"other":{"value":"not at the referenced pointer"}}"#,
    )
    .unwrap();
    let missing_pointer = session.generate().unwrap();
    assert_eq!(missing_pointer.delta.compiles, 1);
    assert_eq!(
        values(&missing_pointer)["operations"][0]["entries"][0]["origin"],
        "synthesized"
    );
    assert_eq!(session.generate().unwrap().delta.compiles, 0);
    std::fs::write(&example, r#"{"entry":{"value":"recovered-pointer"}}"#).unwrap();
    let recovered = session.generate().unwrap();
    assert_eq!(recovered.delta.compiles, 1);
    assert_eq!(
        values(&recovered)["operations"][0]["entries"][0]["value"],
        "recovered-pointer"
    );
}

#[test]
fn reference_shaped_example_data_does_not_create_a_cache_dependency() {
    let root = tempfile::tempdir().unwrap();
    let entry = root.path().join("api.json");
    std::fs::write(&entry,serde_json::json!({"openapi":"3.1.0","info":{"title":"Literal example","version":"1"},"servers":[{"url":"https://example.test"}],"security":[{"key":[]}],"components":{"securitySchemes":{"key":{"type":"http","scheme":"bearer"}}},"paths":{"/value":{"get":{"operationId":"getValue","responses":{"200":{"description":"Value","content":{"application/json":{"schema":{"type":"object","properties":{"$ref":{"type":"string"}}},"example":{"$ref":"literal.json"}}}}}}}}}).to_string()).unwrap();
    let mut settings = config();
    settings
        .targets
        .retain(|target| target.backend == Backend::PythonHttp);
    let mut session = Session::new(&entry, settings).unwrap();
    let before = session.generate().unwrap();
    std::fs::write(
        root.path().join("literal.json"),
        "not an acquired reference",
    )
    .unwrap();
    let after = session.generate().unwrap();
    assert_eq!(after.delta.compiles, 0);
    assert_eq!(after.delta.renders, 0);
    assert_eq!(after.revision, before.revision);
    assert!(after.new_documents.is_empty());
}

#[cfg(unix)]
#[test]
fn entry_symlink_preserves_lexical_relative_reference_base() {
    let (dir, entry) = fixture();
    let alias = dir.path().join("alias");
    std::fs::create_dir(&alias).unwrap();
    std::os::unix::fs::symlink(&entry, alias.join("api.yaml")).unwrap();
    let external = alias.join("split-resources.yaml");
    std::fs::copy(dir.path().join("split-resources.yaml"), &external).unwrap();
    let mut session = Session::new(alias.join("api.yaml"), config()).unwrap();
    let first = session.generate().unwrap();
    assert!(
        first
            .contract
            .documents()
            .any(|(uri, _)| uri.as_path().as_ref() == Some(&external))
    );
    let physical = dir.path().join("split-resources.yaml");
    let original = std::fs::read_to_string(&physical).unwrap();
    std::fs::write(physical, original.replace("minLength: 1", "minLength: 9")).unwrap();
    assert_eq!(session.generate().unwrap().delta.compiles, 0);
    std::fs::write(external, original.replace("minLength: 1", "minLength: 2")).unwrap();
    assert_eq!(session.generate().unwrap().delta.compiles, 1);
}
#[test]
fn package_config_replans_without_recompiling_and_user_edits_remain_conflicts() {
    let (dir, entry) = fixture();
    let mut session = Session::new(&entry, config()).unwrap();
    let first = session.generate().unwrap();
    let mut settings = config();
    settings.targets[0].package_name = "other-sdk".into();
    session.set_config(settings).unwrap();
    let next = session.generate().unwrap();
    assert_eq!(next.delta.compiles, 0);
    assert_eq!(next.delta.renders, 1);
    assert_eq!(next.delta.cache_hits, 4);
    assert!(Arc::ptr_eq(&first.contract, &next.contract));
    let output = dir.path().join("output");
    session.write(&next, &output).unwrap();
    std::fs::write(output.join("typescript/package.json"), "user content").unwrap();
    assert!(session.write(&next, &output).is_err());
    assert_eq!(
        std::fs::read_to_string(output.join("typescript/package.json")).unwrap(),
        "user content"
    );
}
