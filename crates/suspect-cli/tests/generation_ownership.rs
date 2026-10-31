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
            "template",
            "api.yaml",
            "--manifest",
            "gen.toml",
            "--output",
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
    let first = generate(root, &["--check", "--diff"]);
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
    assert_eq!(
        generate(root, &["--check", "--diff"]).status.code(),
        Some(0)
    );
    std::fs::write(root.join("output/user.txt"), "user file").unwrap();
    input(root, "new.txt");
    let changed = generate(root, &["--check", "--diff"]);
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
        generate(root, &["--owner", "public-sdk", "--check", "--diff"])
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

#[test]
fn a_yaml_manifest_renders_what_the_toml_one_does() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    input(root, "item.txt");
    std::fs::write(
        root.join("gen.yaml"),
        "output:\n  - template: item.j2\n    target: item.txt\n",
    )
    .unwrap();
    let run = |manifest: &str, out: &str| {
        Command::new(env!("CARGO_BIN_EXE_suspect"))
            .current_dir(root)
            .args([
                "gen",
                "template",
                "api.yaml",
                "--manifest",
                manifest,
                "--output",
                out,
            ])
            .output()
            .unwrap()
    };
    for (manifest, out) in [("gen.toml", "from-toml"), ("gen.yaml", "from-yaml")] {
        let output = run(manifest, out);
        assert!(
            output.status.success(),
            "{manifest}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(
        std::fs::read_to_string(root.join("from-yaml/item.txt")).unwrap(),
        std::fs::read_to_string(root.join("from-toml/item.txt")).unwrap()
    );
    std::fs::write(
        root.join("gen.yaml"),
        "output:\n  - template: item.j2\n    targte: item.txt\n",
    )
    .unwrap();
    let typo = run("gen.yaml", "typo");
    assert!(!typo.status.success());
    let stderr = String::from_utf8_lossy(&typo.stderr);
    assert!(
        stderr.contains("gen.yaml:3: unknown output rule key `targte`"),
        "{stderr}"
    );
}
