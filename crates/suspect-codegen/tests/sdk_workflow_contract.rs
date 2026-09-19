#[path = "../examples/sdk_milestone_bench/compare.rs"]
mod compare;
use serde_json::json;
use suspect_codegen::OutFile;
use suspect_ir::contract::SourceId;
use suspect_source::Uri;

fn file(path: &str, content: &str) -> OutFile {
    OutFile {
        path: path.into(),
        content: content.into(),
    }
}
fn source() -> SourceId {
    SourceId::new(
        Uri::parse("file:///fixture/api.json").unwrap(),
        Default::default(),
    )
    .child("operation")
}

#[test]
fn quoted_multiline_data_cannot_be_erased_as_a_doc_comment() {
    for (path, old, new) in [
        (
            "typescript/operations.ts",
            "const text = `start\n * old\nend`;",
            "const text = `start\n * new\nend`;",
        ),
        (
            "rust/src/operations/call.rs",
            "const TEXT: &str = r##\"start\n/// old\nend\"##;",
            "const TEXT: &str = r##\"start\n/// new\nend\"##;",
        ),
        (
            "typescript/operations.ts",
            "const text = \"/** old */\";",
            "const text = \"/** new */\";",
        ),
    ] {
        assert_eq!(
            compare::executable_changes(&[file(path, old)], &[file(path, new)]),
            [path]
        );
    }
    assert!(
        compare::executable_changes(
            &[file(
                "rust/a.rs",
                "/// Old\npub fn f<'a>(s: &'a str) -> &'a str { s }"
            )],
            &[file(
                "rust/a.rs",
                "/// New\npub fn f<'a>(s: &'a str) -> &'a str { s }"
            )]
        )
        .is_empty()
    );
}

#[test]
fn docs_only_checks_manifest_semantics_and_unrelated_files() {
    let operation = source();
    let manifest=json!({"operations":[{"source":{"document":operation.document().as_str(),"pointer":operation.pointer()},"method":"GET","descriptionText":"old","hasSourceDescription":true}]}).to_string();
    let before = vec![
        file("typescript/http.md", "old"),
        file("typescript/http-manifest.json", &manifest),
        file("typescript/package.json", r#"{"name":"sdk"}"#),
        file(
            "typescript/operations.ts",
            "/** old */\nexport const value=1;",
        ),
    ];
    let mut after = before.clone();
    after[0].content = "new".into();
    after[1].content = manifest.replace("old", "new");
    after[3].content = "/** new */\nexport const value=1;".into();
    compare::docs_only(&before, &after, &operation).unwrap();
    let mut unrelated = after.clone();
    unrelated[1] = before[1].clone();
    assert!(
        compare::docs_only(&before, &unrelated, &operation).is_err(),
        "changing unrelated prose cannot prove description propagation"
    );
    unrelated[1] = after[1].clone();
    unrelated[3] = before[3].clone();
    assert!(
        compare::docs_only(&before, &unrelated, &operation).is_err(),
        "manifest text must reach the actual native documentation source"
    );
    after[1].content = after[1].content.replace("GET", "POST");
    assert!(compare::docs_only(&before, &after, &operation).is_err());
    after[1].content = manifest.replace("old", "new");
    after[2].content = r#"{"name":"different"}"#.into();
    assert!(compare::docs_only(&before, &after, &operation).is_err());
    after[2] = before[2].clone();
    after.push(file("typescript/unrelated.md", "changed"));
    assert!(compare::docs_only(&before, &after, &operation).is_err());
}
