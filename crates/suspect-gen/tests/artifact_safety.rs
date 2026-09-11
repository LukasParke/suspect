//! Output safety through the public manifest-generation boundary.

use suspect_gen::{Manifest, MinijinjaEngine, OutputRule, TemplateEngine, render_manifest};

#[test]
fn retained_user_content_stays_protected_when_a_later_template_matches_it() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut engine = MinijinjaEngine::new();
    engine
        .add_template(
            "client",
            "generated\n// suspect:begin:user-code\n// default\n// suspect:end:user-code\n",
        )
        .unwrap();
    let manifest = Manifest {
        outputs: vec![OutputRule {
            template: "client".into(),
            target: "client.txt".into(),
        }],
    };
    let context = serde_json::json!({});
    render_manifest(&engine, &manifest, &context, root, false).unwrap();
    let path = root.join("client.txt");
    let edited = std::fs::read_to_string(&path)
        .unwrap()
        .replace("// default", "user_customization();");
    std::fs::write(&path, &edited).unwrap();
    render_manifest(&engine, &manifest, &context, root, false).unwrap();
    assert!(render_manifest(&engine, &Manifest::default(), &context, root, false).is_err());

    // A template update can make previously retained user code a default.
    // This changes neither the file nor the user's ownership of its content.
    engine.add_template("client", &edited).unwrap();
    render_manifest(&engine, &manifest, &context, root, false).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
    let preview = render_manifest(&engine, &Manifest::default(), &context, root, true).unwrap();
    assert!(
        preview.iter().any(|outcome| {
            outcome.reason == suspect_gen::WriteReason::OwnershipConflict
                && outcome
                    .conflict
                    .as_deref()
                    .is_some_and(|reason| reason.contains("retained user content"))
        }),
        "previous user-content protection must survive matching new template defaults"
    );
    assert!(render_manifest(&engine, &Manifest::default(), &context, root, false).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
}

#[test]
fn preserved_user_content_regenerates_but_is_never_removed_as_obsolete() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut engine = MinijinjaEngine::new();
    engine
        .add_template(
            "client",
            "version 1\n// suspect:begin:user-code\n// default\n// suspect:end:user-code\n",
        )
        .unwrap();
    let manifest = Manifest {
        outputs: vec![OutputRule {
            template: "client".into(),
            target: "client.txt".into(),
        }],
    };
    render_manifest(&engine, &manifest, &serde_json::json!({}), root, false).unwrap();
    let original = std::fs::read_to_string(root.join("client.txt")).unwrap();
    std::fs::write(
        root.join("client.txt"),
        original.replace("// default\n", "user();\r\n"),
    )
    .unwrap();
    engine
        .add_template(
            "client",
            "version 2\n// suspect:begin:user-code\n// default\n// suspect:end:user-code\n",
        )
        .unwrap();
    render_manifest(&engine, &manifest, &serde_json::json!({}), root, false).unwrap();
    let current = std::fs::read_to_string(root.join("client.txt")).unwrap();
    assert!(current.starts_with("version 2\n"));
    assert!(current.contains("user();\r\n"));
    let preview = render_manifest(
        &engine,
        &Manifest::default(),
        &serde_json::json!({}),
        root,
        true,
    )
    .unwrap();
    assert!(preview.iter().any(|outcome| {
        outcome.reason == suspect_gen::WriteReason::OwnershipConflict
            && outcome
                .conflict
                .as_deref()
                .is_some_and(|message| message.contains("retained user content"))
    }));
    assert!(
        render_manifest(
            &engine,
            &Manifest::default(),
            &serde_json::json!({}),
            root,
            false
        )
        .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(root.join("client.txt")).unwrap(),
        current
    );
    std::fs::write(
        root.join("client.txt"),
        current.replace("version 2", "edited outside region"),
    )
    .unwrap();
    assert!(render_manifest(&engine, &manifest, &serde_json::json!({}), root, false).is_err());
}

#[test]
fn obsolete_template_outputs_are_reported_in_read_only_checks_and_removed_on_regeneration() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let mut engine = MinijinjaEngine::new();
    engine.add_template("item", "generated").unwrap();
    let manifest = Manifest {
        outputs: vec![OutputRule {
            template: "item".into(),
            target: "item.txt".into(),
        }],
    };
    render_manifest(&engine, &manifest, &serde_json::json!({}), root, false).unwrap();
    let before = std::fs::read(root.join(suspect_artifact::OWNERSHIP_MANIFEST)).unwrap();
    let preview = render_manifest(
        &engine,
        &Manifest::default(),
        &serde_json::json!({}),
        root,
        true,
    )
    .unwrap();
    assert!(preview.iter().any(|outcome| {
        outcome.reason == suspect_gen::WriteReason::Removed
            && !outcome.wrote
            && outcome
                .diff
                .as_deref()
                .is_some_and(|diff| diff.contains("-generated"))
    }));
    assert!(root.join("item.txt").exists());
    assert_eq!(
        std::fs::read(root.join(suspect_artifact::OWNERSHIP_MANIFEST)).unwrap(),
        before
    );
    render_manifest(
        &engine,
        &Manifest::default(),
        &serde_json::json!({}),
        root,
        false,
    )
    .unwrap();
    assert!(!root.join("item.txt").exists());
}

#[test]
fn template_failures_do_not_leave_partially_generated_output() {
    let directory = tempfile::tempdir().unwrap();
    let mut engine = MinijinjaEngine::new();
    engine.add_template("valid", "generated content").unwrap();
    let manifest = Manifest {
        outputs: vec![
            OutputRule {
                template: "valid".into(),
                target: "first.txt".into(),
            },
            OutputRule {
                template: "missing".into(),
                target: "second.txt".into(),
            },
        ],
    };

    assert!(
        render_manifest(
            &engine,
            &manifest,
            &serde_json::json!({}),
            directory.path(),
            false,
        )
        .is_err()
    );
    assert!(!directory.path().join("first.txt").exists());
    assert!(!directory.path().join("second.txt").exists());
}

#[test]
fn unchanged_preserved_sections_do_not_rewrite_generated_files() {
    let directory = tempfile::tempdir().unwrap();
    let mut engine = MinijinjaEngine::new();
    engine
        .add_template(
            "client",
            "generated\n// suspect:begin:user-code\n// helper\n// suspect:end:user-code",
        )
        .unwrap();
    let manifest = Manifest {
        outputs: vec![OutputRule {
            template: "client".into(),
            target: "client.txt".into(),
        }],
    };
    let context = serde_json::json!({});
    render_manifest(&engine, &manifest, &context, directory.path(), false).unwrap();
    let path = directory.path().join("client.txt");
    let timestamp = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
    std::fs::File::open(&path)
        .unwrap()
        .set_modified(timestamp)
        .unwrap();
    let before = std::fs::metadata(&path).unwrap();

    let outputs = render_manifest(&engine, &manifest, &context, directory.path(), false).unwrap();
    assert!(!outputs[0].wrote);
    assert_eq!(
        std::fs::metadata(path).unwrap().modified().unwrap(),
        before.modified().unwrap()
    );
}

#[test]
fn preserved_user_sections_keep_original_line_endings() {
    let directory = tempfile::tempdir().unwrap();
    let mut engine = MinijinjaEngine::new();
    engine
        .add_template(
            "client",
            "generated\n// suspect:begin:user-code\n// default\n// suspect:end:user-code",
        )
        .unwrap();
    let manifest = Manifest {
        outputs: vec![OutputRule {
            template: "client".into(),
            target: "client.txt".into(),
        }],
    };
    let path = directory.path().join("client.txt");
    render_manifest(
        &engine,
        &manifest,
        &serde_json::json!({}),
        directory.path(),
        false,
    )
    .unwrap();
    std::fs::write(&path, "generated\n// suspect:begin:user-code\n// first helper\r\n// second helper\r\n// suspect:end:user-code").unwrap();

    let outcomes = render_manifest(
        &engine,
        &manifest,
        &serde_json::json!({}),
        directory.path(),
        false,
    )
    .unwrap();
    assert!(!outcomes[0].wrote);
    assert!(
        std::fs::read_to_string(path)
            .unwrap()
            .contains("// first helper\r\n// second helper\r\n")
    );
}

#[test]
fn malformed_new_user_sections_are_rejected_before_any_file_is_written() {
    let directory = tempfile::tempdir().unwrap();
    let mut engine = MinijinjaEngine::new();
    engine.add_template("valid", "generated").unwrap();
    engine
        .add_template("broken", "// suspect:begin:user-code\nunclosed")
        .unwrap();
    let manifest = Manifest {
        outputs: vec![
            OutputRule {
                template: "valid".into(),
                target: "first.txt".into(),
            },
            OutputRule {
                template: "broken".into(),
                target: "second.txt".into(),
            },
        ],
    };

    assert!(
        render_manifest(
            &engine,
            &manifest,
            &serde_json::json!({}),
            directory.path(),
            false
        )
        .is_err()
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}
