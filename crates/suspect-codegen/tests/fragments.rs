//! Admission fragments: one focused, self-describing OpenAPI document per
//! tricky construct, executed through the public admission review.
//!
//! Each fragment file states its invariant in a leading `# INVARIANT:`
//! comment; this test pins that the admission verdict matches it. Add a
//! fragment (kebab-case, with the invariant stated) whenever a new edge
//! case needs coverage; see `tests/fragments/README.md`.

use std::path::Path;
use std::sync::Arc;

use suspect_codegen::admission::{self, FindingKind};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn fragment_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fragments")
}

fn review(name: &str) -> (admission::AdmissionReport, Vec<String>) {
    let path = fragment_dir().join(name);
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    let contract = Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap())
        .unwrap_or_else(|e| panic!("{name} must compile into a contract: {e:?}"));
    // Every fragment must state its invariant.
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("# INVARIANT:"),
        "{name} must carry a `# INVARIANT:` header comment"
    );
    let codes: Vec<String> = report_codes(&admission::review(&contract));
    (admission::review(&contract), codes)
}

fn report_codes(report: &admission::AdmissionReport) -> Vec<String> {
    report.findings.iter().map(|f| f.code.to_owned()).collect()
}

fn has(codes: &[String], code: &str) -> bool {
    codes.iter().any(|c| c == code)
}

#[test]
fn fragments_pin_their_admission_verdicts() {
    for entry in std::fs::read_dir(fragment_dir()).expect("fragments directory") {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.ends_with(".yaml") {
            continue;
        }
        let (report, codes) = review(&name);
        let admissible = report.is_admissible();
        match name.as_str() {
            // Clean fragments: the construct must admit without refusal.
            "self-recursive-additional-properties-map.yaml"
            | "oneof-const-int32.yaml"
            | "nullable-datetime-allof.yaml" => {
                assert!(
                    admissible,
                    "{name} must admit; findings {:?}",
                    report.findings
                );
                assert!(
                    !has(&codes, "http-form-extra") && !has(&codes, "http-wire-shape-unknown"),
                    "{name}: {codes:?}"
                );
            }
            // Naming advice: admissible, but the collision must be flagged.
            "method-name-fold-collision.yaml" | "model-name-fold-collision.yaml" => {
                assert!(admissible, "{name} must admit; {:?}", report.findings);
                let expected = if name.starts_with("method-") {
                    "naming-method-collision"
                } else {
                    "naming-model-collision"
                };
                assert!(
                    has(&codes, expected),
                    "{name} must flag {expected}: {codes:?}"
                );
                assert!(
                    report
                        .findings
                        .iter()
                        .all(|f| f.kind == FindingKind::Advice),
                    "{name}: collisions are advice, not refusals: {:?}",
                    report.findings
                );
            }
            // Refusals.
            "duplicate-operation-id.yaml" => {
                assert!(
                    !admissible && has(&codes, "DUPLICATE_OPERATION_ID"),
                    "{name}: {codes:?}"
                );
            }
            "missing-operation-id.yaml" => {
                assert!(
                    !admissible && has(&codes, "http-operation-id"),
                    "{name}: {codes:?}"
                );
            }
            "webhook-on-openapi30.yaml" => {
                assert!(
                    !admissible && has(&codes, "sdk-incoming-version"),
                    "{name}: {codes:?}"
                );
            }
            other => {
                panic!("fragment {other} has no expected verdict; add a match arm in this test")
            }
        }
    }
}

#[test]
fn every_fragment_states_an_invariant() {
    for entry in std::fs::read_dir(fragment_dir()).expect("fragments directory") {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.ends_with(".yaml") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("# INVARIANT:"),
            "{name} must state its invariant in a `# INVARIANT:` comment"
        );
        assert!(
            text.lines().count() >= 10,
            "{name} is too small to be a focused document"
        );
    }
}

#[test]
fn readme_lists_the_convention() {
    let readme = std::fs::read_to_string(fragment_dir().join("README.md")).unwrap();
    assert!(readme.contains("# INVARIANT:"));
    assert!(readme.contains("kebab-case"));
}
