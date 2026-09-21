//! Feature-manifest integrity: the docs table, the registry, and the
//! acceptance evidence must agree.
//!
//! - Every claim must have its evidence file (and, when declared, the
//!   marker inside it).
//! - An unclaimed backend must NOT have the feature's conventional
//!   evidence file: support added without a claim fails here and forces
//!   the manifest update.
//! - `docs/SDK-FEATURES.md` must match the registry byte-for-byte.
//!
//! Regenerate the doc with `SUSPECT_REGEN_FEATURES=1` after intentional
//! registry changes.

use suspect_codegen::features::{BACKENDS, FEATURES, feature_matrix_markdown};

fn test_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests")
}

fn evidence_path(name: &str) -> std::path::PathBuf {
    test_dir().join(name)
}

#[test]
fn every_claim_has_its_evidence() {
    for (fi, feature) in FEATURES.iter().enumerate() {
        for (bi, backend) in BACKENDS.iter().enumerate() {
            let Some(evidence) = suspect_codegen::features::claim_for(fi, bi) else {
                continue;
            };
            let path = evidence_path(evidence);
            let content = std::fs::read_to_string(&path).unwrap_or_else(|e| {
                panic!(
                    "{backend} claims `{}` but evidence file {} is missing: {e}",
                    feature.id,
                    path.display()
                )
            });
            if let Some(marker) = feature.evidence_marker {
                assert!(
                    content.contains(marker),
                    "{backend} claims `{}` but its evidence file has no `{marker}` marker",
                    feature.id
                );
            }
        }
    }
}

#[test]
fn unclaimed_backends_have_no_conventional_evidence() {
    for (fi, feature) in FEATURES.iter().enumerate() {
        for (bi, backend) in BACKENDS.iter().enumerate() {
            if suspect_codegen::features::is_supported(fi, bi) {
                continue;
            }
            let conventional = evidence_path(&format!("{backend}{}", feature.evidence_suffix));
            assert!(
                !conventional.exists(),
                "{} has `{}` (evidence for `{}`) but the manifest does not claim the feature; \
                 add the claim to features.rs or remove the file",
                backend,
                conventional.display(),
                feature.id
            );
        }
    }
}

#[test]
fn docs_table_matches_the_registry() {
    let rendered = feature_matrix_markdown();
    let doc_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/SDK-FEATURES.md");
    if std::env::var_os("SUSPECT_REGEN_FEATURES").is_some() {
        std::fs::write(&doc_path, &rendered).expect("write feature matrix");
        return;
    }
    let committed = std::fs::read_to_string(&doc_path).unwrap_or_else(|e| {
        panic!("docs/SDK-FEATURES.md is missing ({e}); regenerate with SUSPECT_REGEN_FEATURES=1")
    });
    assert_eq!(
        committed, rendered,
        "docs/SDK-FEATURES.md drifted from the feature registry; regenerate with SUSPECT_REGEN_FEATURES=1"
    );
}
