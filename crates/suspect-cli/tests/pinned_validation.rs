//! Explicit resolution admission must not fall back to unrestricted filesystem reads.

use std::path::Path;
use std::process::Command;

fn validate(spec: &Path, allowlist: &Path) -> (i32, serde_json::Value) {
    let output = Command::new(env!("CARGO_BIN_EXE_suspect"))
        .arg("validate")
        .arg(spec)
        .arg("--reference-allowlist")
        .arg(allowlist)
        .args(["--format", "json"])
        .output()
        .unwrap();
    (
        output.status.code().unwrap(),
        serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr))),
    )
}

#[test]
fn allowlist_membership_controls_entry_and_reference_loading() {
    let directory = tempfile::tempdir().unwrap();
    let spec = directory.path().join("api.json");
    let shared = directory.path().join("shared.json");
    let allowlist = directory.path().join("allowed.json");
    std::fs::write(
        &spec,
        r##"{"openapi":"3.1.0","info":{"title":"Pinned sources","version":"1"},"paths":{},"components":{"schemas":{"Model":{"$ref":"shared.json"}}}}"##,
    )
    .unwrap();
    std::fs::write(&shared, r#"{"type":"string"}"#).unwrap();

    std::fs::write(&allowlist, serde_json::to_vec(&[&spec]).unwrap()).unwrap();
    let (code, findings) = validate(&spec, &allowlist);
    assert_eq!(code, 1);
    let denied = findings
        .as_array()
        .unwrap()
        .iter()
        .find(|finding| finding["code"] == "ref-outside-allowlist")
        .expect("a valid but unpinned external document must be refused");
    assert_eq!(denied["file"], spec.to_str().unwrap());
    let range: std::ops::Range<usize> = serde_json::from_value(denied["range"].clone()).unwrap();
    let source = std::fs::read_to_string(&spec).unwrap();
    assert_eq!(&source[range], "\"shared.json\"");

    std::fs::write(&allowlist, serde_json::to_vec(&[&spec, &shared]).unwrap()).unwrap();
    let (code, findings) = validate(&spec, &allowlist);
    assert_eq!(code, 0, "{findings}");
    assert_eq!(findings, serde_json::json!([]));

    // Some(empty) is denial, never the None/unrestricted default.
    std::fs::write(&allowlist, "[]").unwrap();
    let (code, findings) = validate(&spec, &allowlist);
    assert_eq!(code, 1);
    assert_eq!(findings[0]["code"], "validation-input");
}
