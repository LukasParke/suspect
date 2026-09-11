//! Canonical generation determinism and ownership-aware artifact writes.
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use suspect_codegen::{
    OutFile,
    backend::{Backend, TargetConfig},
    matches_disk, write_files,
};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn load(path: &Path) -> Arc<Contract> {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap())
}
fn targets() -> Vec<TargetConfig> {
    [
        (Backend::TypescriptHttp, "@fixture/artifacts"),
        (Backend::RustHttp, "artifact-sdk"),
        (Backend::PythonHttp, "artifact-sdk"),
        (Backend::GoHttp, "example.com/artifact-sdk"),
        (Backend::SwiftHttp, "ArtifactSDK"),
    ]
    .into_iter()
    .map(|(backend, name)| TargetConfig {
        backend,
        package_name: name.into(),
        package_version: "0.0.0".into(),
        import_name: None,
    })
    .collect()
}
fn generate(contract: Arc<Contract>, selected: &[SourceId]) -> Vec<OutFile> {
    let mut files = Vec::new();
    for target in targets() {
        files.extend(
            suspect_codegen::backend::generate(contract.clone(), selected, &target).unwrap(),
        );
    }
    files
}

#[test]
fn selection_order_and_duplicate_roots_do_not_change_shared_native_artifacts() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/m2/canonical.openapi.yaml");
    let contract = load(&path);
    let mut selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let first = generate(contract.clone(), &selected);
    selected.reverse();
    selected.push(selected[0].clone());
    assert_eq!(first, generate(contract, &selected));
}

#[test]
fn native_generation_keeps_recursive_references_and_rejects_unresolved_roots() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(&path,serde_json::json!({"openapi":"3.1.0","info":{"title":"References","version":"1"},"servers":[{"url":"https://example.test"}],"security":[{"key":[]}],"components":{"securitySchemes":{"key":{"type":"http","scheme":"bearer"}},"schemas":{"Node":{"type":"object","properties":{"child":{"$ref":"#/components/schemas/Node"}}}}},"paths":{"/nodes":{"get":{"operationId":"getNode","responses":{"200":{"description":"Node","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Node"}}}}}}},"/missing":{"get":{"operationId":"getMissing","responses":{"200":{"description":"Missing","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Missing"}}}}}}}}}).to_string()).unwrap();
    let contract = load(&path);
    let node = contract
        .operations()
        .find(|operation| operation.operation_id() == Some("getNode"))
        .unwrap()
        .source()
        .clone();
    assert!(!generate(contract.clone(), &[node]).is_empty());
    let missing = contract
        .operations()
        .find(|operation| operation.operation_id() == Some("getMissing"))
        .unwrap()
        .source()
        .clone();
    for target in targets() {
        let errors = suspect_codegen::backend::generate(
            contract.clone(),
            std::slice::from_ref(&missing),
            &target,
        )
        .unwrap_err();
        assert!(
            errors.iter().any(|finding| finding
                .source
                .as_ref()
                .is_some_and(|source| source.pointer().starts_with("/paths/~1missing"))),
            "{errors:?}"
        );
    }
}

#[test]
fn generated_ownership_detects_obsolete_files_and_preserves_edited_or_unowned_output() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    write_files(
        &[
            OutFile {
                path: "keep.ts".into(),
                content: "old".into(),
            },
            OutFile {
                path: "obsolete.ts".into(),
                content: "obsolete".into(),
            },
        ],
        root,
    )
    .unwrap();
    let next = [OutFile {
        path: "keep.ts".into(),
        content: "new".into(),
    }];
    assert!(!matches_disk(&next, root));
    std::fs::write(root.join("notes.txt"), "user notes").unwrap();
    write_files(&next, root).unwrap();
    assert!(!root.join("obsolete.ts").exists());
    assert!(matches_disk(&next, root));
    std::fs::write(root.join("keep.ts"), "user edit").unwrap();
    assert!(!matches_disk(&next, root));
    assert!(
        write_files(&next, root)
            .unwrap_err()
            .contains("user changes")
    );
    assert_eq!(
        std::fs::read_to_string(root.join("keep.ts")).unwrap(),
        "user edit"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("notes.txt")).unwrap(),
        "user notes"
    );
}

#[test]
fn invalid_paths_are_rejected_before_any_generated_file_is_written() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("output");
    std::fs::create_dir(&root).unwrap();
    let protected = directory.path().join("protected.txt");
    std::fs::write(&protected, "user content").unwrap();
    let files = [
        OutFile {
            path: "generated.txt".into(),
            content: "generated content".into(),
        },
        OutFile {
            path: "../protected.txt".into(),
            content: "replacement".into(),
        },
    ];
    assert!(write_files(&files, &root).is_err());
    assert!(!root.join("generated.txt").exists());
    assert_eq!(std::fs::read_to_string(protected).unwrap(), "user content");
}

#[test]
fn identical_generation_keeps_file_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let files = [OutFile {
        path: "types.txt".into(),
        content: "generated content".into(),
    }];
    write_files(&files, directory.path()).unwrap();
    let target = directory.path().join("types.txt");
    std::fs::File::open(&target)
        .unwrap()
        .set_modified(std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000))
        .unwrap();
    let before = std::fs::metadata(&target).unwrap();
    write_files(&files, directory.path()).unwrap();
    let after = std::fs::metadata(&target).unwrap();
    assert_eq!(after.modified().unwrap(), before.modified().unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(after.ino(), before.ino());
    }
    assert!(matches_disk(&files, directory.path()));
}

#[cfg(unix)]
#[test]
fn symlink_targets_cannot_write_or_satisfy_drift_checks() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("output");
    std::fs::create_dir(&root).unwrap();
    let protected = directory.path().join("protected.txt");
    std::fs::write(&protected, "user content").unwrap();
    std::os::unix::fs::symlink(&protected, root.join("generated.txt")).unwrap();
    let files = [OutFile {
        path: "generated.txt".into(),
        content: "user content".into(),
    }];
    assert!(!matches_disk(&files, &root));
    assert!(write_files(&files, &root).is_err());
    assert_eq!(std::fs::read_to_string(protected).unwrap(), "user content");
}

#[test]
#[ignore = "requires SUSPECT_OPENROUTER_SPEC pointing to the tracked public YAML"]
fn openrouter_generation_is_byte_and_metadata_stable() {
    let source = PathBuf::from(
        std::env::var_os("SUSPECT_OPENROUTER_SPEC").expect("set SUSPECT_OPENROUTER_SPEC"),
    );
    let directory = tempfile::tempdir().unwrap();
    let wanted = [
        "getCredits",
        "createKeys",
        "updateKeys",
        "listContainerFiles",
        "getContainerFile",
    ];
    let contract = load(&source);
    let selected = contract
        .operations()
        .filter(|operation| wanted.contains(&operation.operation_id().unwrap_or_default()))
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    assert_eq!(selected.len(), 5);
    let first = generate(contract, &selected);
    write_files(&first, directory.path()).unwrap();
    let mut paths = first
        .iter()
        .map(|file| directory.path().join(&file.path))
        .collect::<Vec<_>>();
    paths.push(directory.path().join(suspect_artifact::OWNERSHIP_MANIFEST));
    let timestamp = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000);
    for path in &paths {
        std::fs::File::open(path)
            .unwrap()
            .set_modified(timestamp)
            .unwrap();
    }
    let before = paths
        .iter()
        .map(|path| {
            (
                std::fs::read(path).unwrap(),
                std::fs::metadata(path).unwrap().modified().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    let second = generate(load(&source), &selected);
    assert_eq!(first, second);
    assert!(matches_disk(&second, directory.path()));
    write_files(&second, directory.path()).unwrap();
    let after = paths
        .iter()
        .map(|path| {
            (
                std::fs::read(path).unwrap(),
                std::fs::metadata(path).unwrap().modified().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(before, after);
}
