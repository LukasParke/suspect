//! Filesystem guarantees at the shared artifact-writing boundary.

use std::path::Path;

use suspect_artifact::{Artifact, ArtifactBatch, Change};

fn prepare<'a>(
    root: &Path,
    files: &'a [(&str, &str)],
) -> Result<ArtifactBatch<'a>, suspect_artifact::ArtifactError> {
    ArtifactBatch::prepare(
        root,
        files.iter().map(|(path, content)| Artifact {
            path: Path::new(path),
            content: content.as_bytes(),
        }),
    )
}

#[test]
fn windows_aliases_and_device_names_are_not_artifact_targets() {
    let directory = tempfile::tempdir().unwrap();
    for path in [
        "file:stream",
        "trailing.",
        "trailing ",
        "CON",
        "aux.txt",
        "nul/data",
        "COM1.rs",
        "lpt9",
    ] {
        assert!(
            prepare(directory.path(), &[(path, "generated")]).is_err(),
            "accepted {path:?}"
        );
    }
}

#[test]
fn invalid_or_colliding_paths_are_rejected_before_creating_output() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("output");
    for paths in [
        vec!["valid.txt", "../outside.txt"],
        vec!["valid.txt", "/absolute.txt"],
        vec!["valid.txt", "C:/absolute.txt"],
        vec!["valid.txt", "nested\\file.txt"],
        vec!["valid.txt", ""],
        vec!["valid.txt", "."],
        vec!["client.txt", "./client.txt"],
        vec!["Client.txt", "client.txt"],
        vec!["models", "models/pet.txt"],
        vec!["models/pet.txt", "Models"],
        vec!["caf\u{e9}.txt", "cafe\u{301}.txt"],
        vec!["Σ.txt", "ς.txt"],
        vec!["ß.txt", "SS.txt"],
        vec!["ß.txt", "ẞ.txt"],
    ] {
        let files: Vec<_> = paths.iter().map(|path| (*path, "generated")).collect();
        assert!(prepare(&root, &files).is_err(), "accepted {paths:?}");
        assert!(!root.exists(), "created output for {paths:?}");
    }
}

#[test]
fn preparation_and_empty_commits_do_not_create_missing_directories() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("missing/output");
    let files = [("nested/client.txt", "generated")];
    let batch = prepare(&root, &files).unwrap();
    assert_eq!(batch.files()[0].change(), Change::Created);
    assert!(!directory.path().join("missing").exists());
    drop(batch);
    prepare(&root, &[]).unwrap().commit().unwrap();
    assert!(!directory.path().join("missing").exists());

    prepare(&root, &files).unwrap().commit().unwrap();
    assert_eq!(
        std::fs::read_to_string(root.join("nested/client.txt")).unwrap(),
        "generated"
    );
}

#[test]
fn conflicting_edits_after_preparation_preserve_the_entire_existing_batch() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("first.txt"), "old first").unwrap();
    std::fs::write(directory.path().join("last.txt"), "old last").unwrap();
    let files = [("first.txt", "new first"), ("last.txt", "new last")];
    let batch = prepare(directory.path(), &files).unwrap();
    std::fs::write(directory.path().join("last.txt"), "concurrent edit").unwrap();

    assert!(batch.commit().is_err());
    assert_eq!(
        std::fs::read_to_string(directory.path().join("first.txt")).unwrap(),
        "old first"
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("last.txt")).unwrap(),
        "concurrent edit"
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
}

#[test]
fn unchanged_files_keep_metadata_and_user_files_are_retained() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let path = root.join("nested/client.txt");
    let files = [("nested/client.txt", "generated")];
    prepare(root, &files).unwrap().commit().unwrap();
    std::fs::write(root.join("user.txt"), "user content").unwrap();
    std::fs::write(root.join("obsolete-generated.txt"), "retained artifact").unwrap();
    let timestamp = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
    std::fs::File::open(&path)
        .unwrap()
        .set_modified(timestamp)
        .unwrap();
    let before = std::fs::metadata(&path).unwrap();

    let batch = prepare(root, &files).unwrap();
    assert_eq!(batch.files()[0].change(), Change::Unchanged);
    batch.commit().unwrap();
    let after = std::fs::metadata(&path).unwrap();
    assert_eq!(before.modified().unwrap(), after.modified().unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(before.ino(), after.ino());
    }
    assert_eq!(
        std::fs::read_to_string(root.join("user.txt")).unwrap(),
        "user content"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("obsolete-generated.txt")).unwrap(),
        "retained artifact"
    );
    assert_eq!(std::fs::read_dir(root.join("nested")).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn replacements_are_atomic_per_file_and_keep_existing_permissions() {
    use std::io::Read;
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("client.txt");
    std::fs::write(&path, "old complete content").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    let mut previous_inode = std::fs::File::open(&path).unwrap();
    prepare(directory.path(), &[("client.txt", "new complete content")])
        .unwrap()
        .commit()
        .unwrap();

    let mut previous_content = String::new();
    previous_inode
        .read_to_string(&mut previous_content)
        .unwrap();
    assert_eq!(previous_content, "old complete content");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "new complete content"
    );
    assert_eq!(
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn symlinks_at_the_root_or_below_it_are_rejected_including_dangling_links() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let outside = directory.path().join("outside");
    let root = directory.path().join("output");
    std::fs::create_dir(&outside).unwrap();
    std::fs::create_dir(&root).unwrap();
    std::fs::write(outside.join("protected.txt"), "user content").unwrap();
    symlink(&outside, root.join("directory-link")).unwrap();
    symlink(outside.join("protected.txt"), root.join("file-link")).unwrap();
    symlink(outside.join("missing-file"), root.join("dangling-file")).unwrap();
    symlink(
        outside.join("missing-directory"),
        root.join("dangling-directory"),
    )
    .unwrap();
    symlink(&outside, directory.path().join("root-link")).unwrap();
    for target in [
        "directory-link/protected.txt",
        "file-link",
        "dangling-file",
        "dangling-directory/file.txt",
    ] {
        let files = [("valid.txt", "generated"), (target, "replacement")];
        assert!(prepare(&root, &files).is_err(), "accepted {target}");
        assert!(!root.join("valid.txt").exists());
    }
    assert!(
        prepare(
            &directory.path().join("root-link"),
            &[("file.txt", "generated")]
        )
        .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(outside.join("protected.txt")).unwrap(),
        "user content"
    );
    assert!(!outside.join("missing-file").exists());
    assert!(!outside.join("missing-directory").exists());
}

#[cfg(unix)]
#[test]
fn a_symlink_inserted_after_preparation_cannot_redirect_a_write() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("output");
    let outside = directory.path().join("outside");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(&outside).unwrap();
    let files = [("first.txt", "first"), ("nested/client.txt", "generated")];
    let batch = prepare(&root, &files).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("nested")).unwrap();

    assert!(batch.commit().is_err());
    assert!(!root.join("first.txt").exists());
    assert!(!outside.join("client.txt").exists());
}

#[cfg(unix)]
#[test]
fn replacing_the_output_root_with_a_symlink_aborts_the_batch() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("output");
    let outside = directory.path().join("outside");
    let moved = directory.path().join("moved");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(&outside).unwrap();
    let files = [("client.txt", "generated")];
    let batch = prepare(&root, &files).unwrap();
    std::fs::rename(&root, &moved).unwrap();
    std::os::unix::fs::symlink(&outside, &root).unwrap();

    assert!(batch.commit().is_err());
    assert!(!outside.join("client.txt").exists());
    assert!(!moved.join("client.txt").exists());
}
