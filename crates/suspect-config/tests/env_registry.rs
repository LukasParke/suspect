//! Every `SUSPECT_*` variable the shipped binary reads is declared: as the
//! `env` column of a setting, or in `DEBUG_VARS`. A new product variable
//! must be added to the table, where `--explain`, the schema and the docs
//! pick it up.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use suspect_config::schema;

/// Source files that are test harnesses or generator output, not product
/// code: their variables gate toolchain tests or belong to generated SDKs.
const HARNESSES: &[&str] = &[
    // `#[ignore]` toolchain gates' binary overrides.
    "suspect-codegen/src/toolchain.rs",
    "suspect-codegen/src/swift_sdk/protocol_next_harness.rs",
    "suspect-codegen/src/swift_sdk/validation.rs",
    "suspect-codegen/src/swift_sdk/validation_v2_support.rs",
    "suspect-codegen/src/swift_sdk/validation_v3_support.rs",
    // A Python script the Go generator emits.
    "suspect-codegen/src/go_http/docs.rs",
    // The latency bench module.
    "suspect-lsp/src/latency.rs",
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn is_test_file(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name == "tests.rs"
        || name.ends_with("_tests.rs")
        || name.starts_with("tests_")
        || path.components().any(|c| c.as_os_str() == "tests")
}

/// The file's text with `#[cfg(test)]` items removed, by brace depth.
fn product_code(text: &str) -> String {
    let mut out = String::new();
    let mut pending = false;
    let mut depth: i64 = 0;
    let mut skipping = false;
    for line in text.lines() {
        if skipping {
            depth += line.matches('{').count() as i64 - line.matches('}').count() as i64;
            if depth <= 0 {
                skipping = false;
            }
            continue;
        }
        if line.trim_start().starts_with("#[cfg(test)]") {
            pending = true;
            continue;
        }
        if pending {
            let opens = line.matches('{').count() as i64 - line.matches('}').count() as i64;
            if line.contains('{') {
                pending = false;
                if opens > 0 {
                    depth = opens;
                    skipping = true;
                }
                continue;
            }
            if line.trim_end().ends_with(';') {
                // `#[cfg(test)] mod tests;` or a single-line item.
                pending = false;
                continue;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn literals(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while let Some(at) = text[i..].find("\"SUSPECT_") {
        let start = i + at + 1;
        let mut end = start;
        while end < bytes.len()
            && (bytes[end].is_ascii_uppercase()
                || bytes[end].is_ascii_digit()
                || bytes[end] == b'_')
        {
            end += 1;
        }
        found.insert(text[start..end].to_owned());
        i = end;
    }
    found
}

#[test]
fn every_product_variable_is_declared() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let declared: BTreeSet<&str> = schema::env_vars()
        .map(|(name, _)| name)
        .chain(schema::DEBUG_VARS.iter().map(|d| d.name))
        .collect();
    let mut undeclared = Vec::new();
    let mut seen = BTreeSet::new();
    let Ok(entries) = std::fs::read_dir(&crates) else {
        panic!("no crates directory at {}", crates.display());
    };
    for krate in entries.flatten() {
        let src = krate.path().join("src");
        let mut files = Vec::new();
        rust_files(&src, &mut files);
        for file in files {
            let relative = file.strip_prefix(&crates).unwrap_or(&file);
            let relative = relative.to_string_lossy().replace('\\', "/");
            if is_test_file(&file) || HARNESSES.contains(&relative.as_str()) {
                continue;
            }
            let text = std::fs::read_to_string(&file).unwrap_or_default();
            for name in literals(&product_code(&text)) {
                seen.insert(name.clone());
                if !declared.contains(name.as_str()) {
                    undeclared.push(format!("{relative}: {name}"));
                }
            }
        }
    }
    assert!(
        undeclared.is_empty(),
        "undeclared SUSPECT_* variables (add them to schema::SETTINGS `env` or DEBUG_VARS):\n{}",
        undeclared.join("\n")
    );
    // The scan is not vacuous: the registered names are found where read.
    for name in [
        "SUSPECT_PROFILE",
        "SUSPECT_GW_PROFILE",
        "SUSPECT_RULES_DEBUG",
    ] {
        assert!(
            seen.contains(name),
            "{name} not found by the scan: {seen:?}"
        );
    }
}

#[test]
fn cfg_test_items_are_skipped() {
    let code = "let a = \"SUSPECT_REAL\";\n#[cfg(test)]\nmod tests {\n    fn f() { let b = \"SUSPECT_TEST_ONLY\"; }\n}\nlet c = 1;\n";
    let found = literals(&product_code(code));
    assert_eq!(found.into_iter().collect::<Vec<_>>(), ["SUSPECT_REAL"]);
}
