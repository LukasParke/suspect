//! Ownership behavior through public preflight, regeneration and drift checks.

use std::path::Path;
use suspect_artifact::{
    Adoption, Artifact, ArtifactBatch, OWNERSHIP_MANIFEST, OwnedBatch, OwnershipChangeKind,
};

fn owned<'a>(
    root: &Path,
    owner: &str,
    files: &'a [(&str, &str)],
    adoption: Adoption,
) -> OwnedBatch<'a> {
    ArtifactBatch::prepare(
        root,
        files.iter().map(|(path, content)| Artifact {
            path: Path::new(path),
            content: content.as_bytes(),
        }),
    )
    .unwrap()
    .with_ownership(owner, adoption)
    .unwrap()
}

#[test]
fn recorded_user_content_protection_survives_identical_regeneration_without_region_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let files = [("client.txt", "generated\nuser_customization();")];
    let mut initial = ArtifactBatch::prepare(
        root,
        files.iter().map(|(path, content)| Artifact {
            path: Path::new(path),
            content: content.as_bytes(),
        }),
    )
    .unwrap();
    initial.files_mut()[0].set_managed_content("user-regions-v1", b"", b"generated", true);
    initial
        .with_ownership("sdk", Adoption::Refuse)
        .unwrap()
        .commit()
        .unwrap();
    assert!(owned(root, "sdk", &[], Adoption::Refuse).commit().is_err());

    // The same owner can regenerate identical bytes through a different
    // frontend. Omitting current region metadata cannot release user content.
    owned(root, "sdk", &files, Adoption::Refuse)
        .commit()
        .unwrap();
    let obsolete = owned(root, "sdk", &[], Adoption::Refuse);
    assert!(
        obsolete
            .report()
            .conflicts()
            .any(|change| change.path == Path::new("client.txt"))
    );
    assert!(obsolete.commit().is_err());
    assert_eq!(
        std::fs::read_to_string(root.join("client.txt")).unwrap(),
        files[0].1
    );

    // Deliberate removal plus omission retires the record. A later explicit
    // adoption starts new ownership, rather than inheriting retired history.
    std::fs::remove_file(root.join("client.txt")).unwrap();
    owned(root, "sdk", &[], Adoption::Refuse).commit().unwrap();
    std::fs::write(root.join("client.txt"), files[0].1).unwrap();
    owned(root, "sdk", &files, Adoption::Identical)
        .commit()
        .unwrap();
    owned(root, "sdk", &[], Adoption::Refuse).commit().unwrap();
    assert!(!root.join("client.txt").exists());
}

#[test]
fn regeneration_detects_missing_changed_and_obsolete_owned_outputs() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let original = [
        ("keep.txt", "same"),
        ("change.txt", "old"),
        ("missing.txt", "restore"),
        ("obsolete.txt", "remove"),
    ];
    assert!(
        !owned(root, "sdk", &original, Adoption::Refuse)
            .report()
            .is_current()
    );
    owned(root, "sdk", &original, Adoption::Refuse)
        .commit()
        .unwrap();
    std::fs::write(root.join("user.txt"), "keep user file").unwrap();
    std::fs::remove_file(root.join("missing.txt")).unwrap();
    let next = [
        ("keep.txt", "same"),
        ("change.txt", "new"),
        ("missing.txt", "restore"),
    ];
    let batch = owned(root, "sdk", &next, Adoption::Refuse);
    assert!(
        batch
            .report()
            .changes
            .iter()
            .any(|change| change.path == Path::new("obsolete.txt")
                && change.kind == OwnershipChangeKind::Obsolete)
    );
    assert!(
        batch
            .report()
            .changes
            .iter()
            .any(|change| change.path == Path::new("missing.txt")
                && change.kind == OwnershipChangeKind::Created)
    );
    assert!(!batch.report().is_current());
    batch.commit().unwrap();
    assert!(!root.join("obsolete.txt").exists());
    assert_eq!(
        std::fs::read_to_string(root.join("user.txt")).unwrap(),
        "keep user file"
    );
    assert!(
        owned(root, "sdk", &next, Adoption::Refuse)
            .report()
            .is_current()
    );
}

#[test]
fn edited_owned_and_unowned_files_conflict_before_any_writes_or_deletions() {
    for edited in ["active.txt", "obsolete.txt", "unowned.txt"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        owned(
            root,
            "sdk",
            &[("active.txt", "old"), ("obsolete.txt", "old obsolete")],
            Adoption::Refuse,
        )
        .commit()
        .unwrap();
        std::fs::write(root.join(edited), "user edit").unwrap();
        let manifest = std::fs::read(root.join(OWNERSHIP_MANIFEST)).unwrap();
        let next = [("active.txt", "new"), ("unowned.txt", "generated")];
        let batch = owned(root, "sdk", &next, Adoption::Refuse);
        assert!(
            batch
                .report()
                .conflicts()
                .any(|change| change.path == Path::new(edited))
        );
        assert!(batch.commit().is_err());
        assert_eq!(
            std::fs::read_to_string(root.join(edited)).unwrap(),
            "user edit"
        );
        assert!(root.join("obsolete.txt").exists());
        assert_eq!(
            std::fs::read(root.join(OWNERSHIP_MANIFEST)).unwrap(),
            manifest
        );
    }
}

#[test]
fn unowned_file_adoption_is_explicit_and_byte_identical_only() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    std::fs::write(root.join("old.txt"), "same").unwrap();
    let files = [("old.txt", "same")];
    assert!(
        owned(root, "sdk", &files, Adoption::Refuse)
            .commit()
            .is_err()
    );
    assert!(!root.join(OWNERSHIP_MANIFEST).exists());
    assert!(
        owned(
            root,
            "sdk",
            &[("old.txt", "different")],
            Adoption::Identical
        )
        .commit()
        .is_err()
    );
    owned(root, "sdk", &files, Adoption::Identical)
        .commit()
        .unwrap();
    owned(root, "sdk", &[], Adoption::Refuse).commit().unwrap();
    assert!(!root.join("old.txt").exists());
}

#[test]
fn manifest_and_outputs_keep_inodes_and_mtimes_when_nothing_changes() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let files = [("generated.txt", "same")];
    owned(root, "sdk", &files, Adoption::Refuse)
        .commit()
        .unwrap();
    let paths = [root.join("generated.txt"), root.join(OWNERSHIP_MANIFEST)];
    let time = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
    for path in &paths {
        std::fs::File::open(path)
            .unwrap()
            .set_modified(time)
            .unwrap();
    }
    let before: Vec<_> = paths
        .iter()
        .map(|path| std::fs::metadata(path).unwrap())
        .collect();
    owned(root, "sdk", &files, Adoption::Refuse)
        .commit()
        .unwrap();
    for (path, before) in paths.iter().zip(before) {
        let after = std::fs::metadata(path).unwrap();
        assert_eq!(before.modified().unwrap(), after.modified().unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(before.ino(), after.ino());
        }
    }
}

#[test]
fn owners_are_isolated_and_stale_manifests_reconcile_only_known_identical_output() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    owned(root, "sdk", &[("sdk.txt", "v1")], Adoption::Refuse)
        .commit()
        .unwrap();
    let previous = std::fs::read(root.join(OWNERSHIP_MANIFEST)).unwrap();
    owned(root, "sdk", &[("sdk.txt", "v2")], Adoption::Refuse)
        .commit()
        .unwrap();
    // Simulate interruption after replacing a previously owned output but before manifest publication.
    std::fs::write(root.join(OWNERSHIP_MANIFEST), previous).unwrap();
    let repaired = owned(root, "sdk", &[("sdk.txt", "v2")], Adoption::Refuse);
    assert!(!repaired.report().is_current());
    repaired.commit().unwrap();
    owned(root, "docs", &[("docs.txt", "docs")], Adoption::Refuse)
        .commit()
        .unwrap();
    assert!(
        owned(root, "docs", &[("sdk.txt", "v2")], Adoption::Identical)
            .commit()
            .is_err()
    );
    owned(root, "sdk", &[], Adoption::Refuse).commit().unwrap();
    assert_eq!(
        std::fs::read_to_string(root.join("docs.txt")).unwrap(),
        "docs"
    );
    assert!(!root.join("sdk.txt").exists());
    // Interrupted creation is unrecorded and is never silently adopted.
    std::fs::write(root.join("new.txt"), "new").unwrap();
    assert!(
        owned(root, "sdk", &[("new.txt", "new")], Adoption::Refuse)
            .commit()
            .is_err()
    );
}

#[test]
fn malformed_future_or_unsafe_manifests_block_all_output_changes() {
    for corruption in [
        "syntax",
        "version",
        "duplicate",
        "traversal",
        "reserved",
        "digest",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        owned(root, "sdk", &[("owned.txt", "old")], Adoption::Refuse)
            .commit()
            .unwrap();
        let path = root.join(OWNERSHIP_MANIFEST);
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        match corruption {
            "version" => value["version"] = serde_json::json!(999),
            "duplicate" => {
                let duplicate = value["owners"][0]["files"][0].clone();
                value["owners"][0]["files"]
                    .as_array_mut()
                    .unwrap()
                    .push(duplicate);
            }
            "traversal" => {
                value["owners"][0]["files"][0]["path"] = serde_json::json!("../protected.txt")
            }
            "reserved" => {
                value["owners"][0]["files"][0]["path"] = serde_json::json!(OWNERSHIP_MANIFEST)
            }
            "digest" => {
                value["owners"][0]["files"][0]["fingerprint"]["sha256"] =
                    serde_json::json!("not-a-digest")
            }
            _ => {}
        }
        let corrupt = if corruption == "syntax" {
            b"{truncated".to_vec()
        } else {
            serde_json::to_vec(&value).unwrap()
        };
        std::fs::write(&path, &corrupt).unwrap();
        let files = [("owned.txt", "new"), ("created.txt", "new")];
        let batch = ArtifactBatch::prepare(
            root,
            files.iter().map(|(path, content)| Artifact {
                path: Path::new(path),
                content: content.as_bytes(),
            }),
        )
        .unwrap();
        assert!(
            batch.with_ownership("sdk", Adoption::Identical).is_err(),
            "{corruption}"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("owned.txt")).unwrap(),
            "old"
        );
        assert!(!root.join("created.txt").exists());
        assert_eq!(std::fs::read(path).unwrap(), corrupt);
    }
}

#[test]
fn reserved_and_recorded_case_collisions_do_not_create_output() {
    for target in [
        ".SUSPECT-ARTIFACTS.JSON",
        ".suspect-artifacts.json/child",
        "OWNED.txt",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        owned(root, "sdk", &[("owned.txt", "old")], Adoption::Refuse)
            .commit()
            .unwrap();
        let files = [(target, "new")];
        let result = ArtifactBatch::prepare(
            root,
            files.iter().map(|(path, content)| Artifact {
                path: Path::new(path),
                content: content.as_bytes(),
            }),
        )
        .and_then(|batch| batch.with_ownership("sdk", Adoption::Refuse));
        assert!(result.is_err(), "{target}");
        assert_eq!(
            std::fs::read_to_string(root.join("owned.txt")).unwrap(),
            "old"
        );
    }
}

#[test]
fn concurrent_manifest_or_obsolete_edits_abort_before_replacing_output() {
    for manifest_edit in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        owned(
            root,
            "sdk",
            &[("active.txt", "old"), ("obsolete.txt", "old obsolete")],
            Adoption::Refuse,
        )
        .commit()
        .unwrap();
        let desired = [("active.txt", "new")];
        let batch = owned(root, "sdk", &desired, Adoption::Refuse);
        if manifest_edit {
            owned(root, "docs", &[("docs.txt", "docs")], Adoption::Refuse)
                .commit()
                .unwrap();
        } else {
            std::fs::write(root.join("obsolete.txt"), "user edit").unwrap();
        }
        assert!(batch.commit().is_err());
        assert_eq!(
            std::fs::read_to_string(root.join("active.txt")).unwrap(),
            "old"
        );
        assert!(root.join("obsolete.txt").exists());
    }
}

#[cfg(unix)]
#[test]
fn manifest_and_obsolete_symlinks_are_never_followed_or_deleted() {
    for target in [OWNERSHIP_MANIFEST, "obsolete.txt"] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("output");
        owned(&root, "sdk", &[("obsolete.txt", "old")], Adoption::Refuse)
            .commit()
            .unwrap();
        let outside = directory.path().join("outside");
        let bytes = std::fs::read(root.join(target)).unwrap();
        std::fs::write(&outside, &bytes).unwrap();
        std::fs::remove_file(root.join(target)).unwrap();
        std::os::unix::fs::symlink(&outside, root.join(target)).unwrap();
        let files = [("new.txt", "new")];
        let batch = ArtifactBatch::prepare(
            &root,
            files.iter().map(|(path, content)| Artifact {
                path: Path::new(path),
                content: content.as_bytes(),
            }),
        )
        .unwrap();
        assert!(batch.with_ownership("sdk", Adoption::Refuse).is_err());
        assert_eq!(std::fs::read(outside).unwrap(), bytes);
        assert!(!root.join("new.txt").exists());
        assert!(
            std::fs::symlink_metadata(root.join(target))
                .unwrap()
                .is_symlink()
        );
    }
}
