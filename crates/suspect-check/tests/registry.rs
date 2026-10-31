//! Registry completeness: every code the checks emit over the fixture
//! corpus is registered, no code is registered twice, and every rule has a
//! summary.
//!
//! The corpus is every YAML/JSON file under `crates/*/tests/fixtures`
//! (including this crate's broken Swagger, Arazzo, Overlay and load-profile
//! documents),
//! `docs/demo`, `examples` and, when fetched, `corpus/`. SDK admission is
//! not run here: it passes through the open-ended `http-*` codes of the
//! shared HTTP admission layer, which are not tabled yet.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use suspect_check::Registry;
use suspect_check::registry::builtin_tables;
use suspect_diag::CheckId;
use suspect_low::{LowDoc, SpecFamily};
use suspect_source::{Source, Uri};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "node_modules") {
                continue;
            }
            collect(&path, out);
        } else if path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| matches!(e, "yaml" | "yml" | "json"))
        {
            out.push(path);
        }
    }
}

fn corpus() -> Vec<PathBuf> {
    let root = workspace_root();
    let mut files = Vec::new();
    if let Ok(crates) = std::fs::read_dir(root.join("crates")) {
        for entry in crates.flatten() {
            collect(&entry.path().join("tests/fixtures"), &mut files);
        }
    }
    for dir in ["docs/demo", "examples", "corpus"] {
        collect(&root.join(dir), &mut files);
    }
    files.sort();
    files
}

/// `(check, code)` -> first file that emitted it.
type Emitted = BTreeMap<(CheckId, String), PathBuf>;

fn record(emitted: &mut Emitted, check: CheckId, code: &str, path: &Path) {
    emitted
        .entry((check, code.to_owned()))
        .or_insert_with(|| path.to_path_buf());
}

fn run_checks(path: &Path, linter: &suspect_lint::Linter, emitted: &mut Emitted) {
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    let Ok(uri) = Uri::from_path(path) else {
        return;
    };
    let low = LowDoc::parse(uri, Source::from_vec(bytes));
    let family = low.sniff_family();
    for finding in linter.run(&low) {
        record(emitted, CheckId::Lint, &finding.code, path);
    }
    match family {
        SpecFamily::Oas30 | SpecFamily::Oas31 | SpecFamily::Oas32 => {
            let Some(dir) = path.parent() else { return };
            let Ok(workspace) = suspect_ref::WorkspaceBuilder::new().root(dir).build() else {
                return;
            };
            let session = suspect_oas::Session::new(Arc::new(workspace));
            let Some(entry) = path.to_str() else { return };
            if let Ok(diagnostics) = suspect_validate::validate_entry(&session, entry) {
                for d in diagnostics {
                    record(emitted, CheckId::Validate, d.code, path);
                }
            }
        }
        SpecFamily::Oas2 => {
            for d in suspect_validate::validate_swagger_low(&low) {
                record(emitted, CheckId::Swagger, d.code, path);
            }
        }
        SpecFamily::Arazzo10 => {
            let doc = suspect_arazzo::ArazzoDoc::new(&low);
            for d in suspect_arazzo::validate_arazzo(&doc) {
                record(emitted, CheckId::Arazzo, d.code, path);
            }
        }
        SpecFamily::Overlay10 => {
            if let Ok(doc) = suspect_overlay::OverlayDoc::parse(&low) {
                for d in suspect_overlay::validate_overlay(&doc) {
                    record(emitted, CheckId::Overlay, d.code, path);
                }
            }
        }
        SpecFamily::Load10 => {
            let (_, diagnostics) = suspect_test::load::read_profile_lenient(&low, path);
            for d in diagnostics {
                record(emitted, CheckId::LoadProfile, d.code, path);
            }
        }
        SpecFamily::Unknown => {}
    }
}

#[test]
fn every_emitted_code_is_registered() {
    let registry = Registry::builtin();
    let linter = suspect_lint::Linter::spectral_default();
    let files = corpus();
    assert!(files.len() > 20, "fixture corpus not found");
    let mut emitted = Emitted::new();
    for path in &files {
        run_checks(path, &linter, &mut emitted);
    }
    let mut checks: Vec<CheckId> = emitted.keys().map(|(check, _)| *check).collect();
    checks.dedup();
    // Every producer must emit something, or the test proves nothing.
    assert_eq!(
        checks,
        [
            CheckId::Validate,
            CheckId::Swagger,
            CheckId::Arazzo,
            CheckId::Overlay,
            CheckId::LoadProfile,
            CheckId::Lint,
        ],
        "corpus did not exercise every producer"
    );
    eprintln!(
        "{} distinct codes from {} files",
        emitted.len(),
        files.len()
    );
    let mut missing = Vec::new();
    for ((check, code), path) in &emitted {
        match registry.info(code) {
            None => missing.push(format!("{code} ({}, {})", check.name(), path.display())),
            Some(info) => {
                // Swagger findings come from the validate crate's second
                // battery; every other code is owned by the check that
                // emitted it.
                assert_eq!(
                    info.check, *check,
                    "{code} is registered under another check"
                );
            }
        }
    }
    assert!(
        missing.is_empty(),
        "codes emitted but not registered:\n  {}",
        missing.join("\n  ")
    );
}

#[test]
fn no_code_is_registered_twice() {
    let mut seen: BTreeMap<&str, CheckId> = BTreeMap::new();
    for rule in builtin_tables().into_iter().flatten() {
        if let Some(first) = seen.insert(rule.code, rule.check) {
            panic!(
                "{} registered by {} and {}",
                rule.code,
                first.name(),
                rule.check.name()
            );
        }
    }
    assert_eq!(seen.len(), Registry::builtin().len());
}

#[test]
fn every_rule_has_a_summary() {
    let registry = Registry::builtin();
    assert!(!registry.is_empty());
    for rule in registry.rules() {
        assert!(
            !rule.summary.trim().is_empty(),
            "{} has no summary",
            rule.code
        );
        assert!(
            rule.aliases.is_empty(),
            "{} has aliases before step 4",
            rule.code
        );
    }
}

#[test]
fn tables_cover_every_producer() {
    let registry = Registry::builtin();
    for check in [
        CheckId::Validate,
        CheckId::Swagger,
        CheckId::Lint,
        CheckId::Arazzo,
        CheckId::Overlay,
        CheckId::LoadProfile,
    ]
    .into_iter()
    .chain(cfg!(feature = "sdk").then_some(CheckId::Admission))
    {
        assert!(
            registry.rules().any(|r| r.check == check),
            "no rules for {}",
            check.name()
        );
    }
}
