//! Ownership and read-only drift through the real generator CLI.

use std::{
    path::Path,
    process::{Command, Output},
};

fn generate(root: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_suspect"))
        .current_dir(root)
        .args([
            "gen",
            "api.yaml",
            "--manifest",
            "gen.toml",
            "--out",
            "output",
        ])
        .args(extra)
        .output()
        .unwrap()
}

fn input(root: &Path, target: &str) {
    std::fs::write(
        root.join("api.yaml"),
        "openapi: 3.1.0\ninfo: {title: Ownership, version: '1'}\npaths: {}\n",
    )
    .unwrap();
    std::fs::write(root.join("item.j2"), "generated").unwrap();
    std::fs::write(
        root.join("gen.toml"),
        format!("[[output]]\ntemplate = 'item.j2'\ntarget = '{target}'\n"),
    )
    .unwrap();
}

#[test]
fn diff_is_read_only_and_reports_obsolete_owned_outputs() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    input(root, "old.txt");
    let first = generate(root, &["--diff"]);
    assert_eq!(
        first.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(!root.join("output").exists());
    let written = generate(root, &[]);
    assert!(
        written.status.success(),
        "{}",
        String::from_utf8_lossy(&written.stderr)
    );
    assert_eq!(generate(root, &["--diff"]).status.code(), Some(0));
    std::fs::write(root.join("output/user.txt"), "user file").unwrap();
    input(root, "new.txt");
    let changed = generate(root, &["--diff"]);
    assert_eq!(changed.status.code(), Some(1));
    assert!(root.join("output/old.txt").exists());
    assert!(!root.join("output/new.txt").exists());
    assert!(String::from_utf8_lossy(&changed.stdout).contains("-generated"));
    assert!(generate(root, &[]).status.success());
    assert!(!root.join("output/old.txt").exists());
    assert!(root.join("output/new.txt").exists());
    assert_eq!(
        std::fs::read_to_string(root.join("output/user.txt")).unwrap(),
        "user file"
    );
}

#[test]
fn adoption_requires_an_explicit_flag_and_never_transfers_another_owners_file() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    input(root, "old.txt");
    std::fs::create_dir(root.join("output")).unwrap();
    std::fs::write(root.join("output/old.txt"), "generated").unwrap();
    let refused = generate(root, &[]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("unowned file"));
    assert!(!root.join("output/.suspect-artifacts.json").exists());
    assert!(
        generate(root, &["--owner", "public-sdk", "--adopt-identical"])
            .status
            .success()
    );
    assert!(
        generate(root, &["--owner", "public-sdk", "--diff"])
            .status
            .success()
    );
    let foreign = generate(root, &["--owner", "other-sdk", "--adopt-identical"]);
    assert!(!foreign.status.success());
    assert!(String::from_utf8_lossy(&foreign.stderr).contains("owned by `public-sdk`"));
    assert_eq!(
        std::fs::read_to_string(root.join("output/old.txt")).unwrap(),
        "generated"
    );
}
