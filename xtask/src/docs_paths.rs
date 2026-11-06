//! Guards for documentation paths that code reads or cites: a renamed or
//! moved doc must fail `cargo test`, not a manual run or an `#[ignore]`
//! native gate.

/// Directories under `docs/` that hold repository docs (P6 layout). Generated
/// SDK packages emit their own `docs/reference.md`, `docs/guide.md`, ...;
/// those never sit under one of these, so they are not matched.
const DOC_DIRS: &[&str] = &[
    "guide",
    "reference",
    "sdk",
    "design",
    "internal",
    "proposals",
];

fn is_path_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'/')
}

/// Every `docs/<UPPER>.md` or `docs/<dir>/<path>.md` in `text`: the
/// `docs/(?:[A-Z][A-Za-z0-9_-]*\.md|(?:guide|...)/[A-Za-z0-9_./-]+\.md)`
/// pattern, written out to keep xtask free of a regex dependency.
fn doc_paths(text: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(offset) = text[from..].find("docs/") {
        let start = from + offset;
        let rest = start + "docs/".len();
        from = rest;
        let tail = &text[rest..];
        // docs/<UPPER>.md: one segment, no dots before `.md`.
        if tail.as_bytes().first().is_some_and(u8::is_ascii_uppercase) {
            let len = tail
                .bytes()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
                .count();
            if tail[len..].starts_with(".md") {
                found.push(&text[start..rest + len + 3]);
            }
            continue;
        }
        // docs/<dir>/<path>.md: the longest run of path characters, cut at
        // its last `.md` (what a greedy regex matches).
        for dir in DOC_DIRS {
            let Some(path) = tail.strip_prefix(dir).and_then(|t| t.strip_prefix('/')) else {
                continue;
            };
            let run = path.bytes().take_while(|c| is_path_char(*c)).count();
            if let Some(end) = path[..run].rfind(".md")
                && end > 0
            {
                let end = rest + dir.len() + 1 + end + 3;
                found.push(&text[start..end]);
            }
            break;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scanner_matches_the_documented_pattern() {
        let text = "see docs/SDK-FULL-EXIT.md, docs/sdk/kotlin/README.md#x and \
                    docs/reference/cli.md; not docs/reference.md, docs/guide.md, \
                    docs/lower.md or docs/SDK-X.mdx";
        assert_eq!(
            doc_paths(text),
            vec![
                "docs/SDK-FULL-EXIT.md",
                "docs/sdk/kotlin/README.md",
                "docs/reference/cli.md",
                "docs/SDK-X.md",
            ]
        );
    }

    #[test]
    fn every_docs_path_named_in_code_exists() {
        let root = crate::repo_root();
        // This file's own scanner samples are not references.
        let files = crate::tracked_files(&root, |path| {
            !path.ends_with(".md") && path != "xtask/src/docs_paths.rs"
        })
        .unwrap();
        let mut missing = Vec::new();
        let mut checked = 0;
        for file in files {
            let Ok(text) = std::fs::read_to_string(root.join(&file)) else {
                continue; // binary or deleted in the working tree
            };
            for path in doc_paths(&text) {
                checked += 1;
                if !root.join(path).is_file() {
                    missing.push(format!("{file}: {path}"));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "doc paths named in code do not exist:\n{}",
            missing.join("\n")
        );
        assert!(checked > 0, "the scan found no doc paths at all");
    }

    #[test]
    fn repository_passes_sdk_full_syntax_check() {
        let root = crate::repo_root().canonicalize().unwrap();
        let result = crate::sdk_full::syntax_check(&root).unwrap();
        assert_eq!(
            result["readyToBuild"], true,
            "sdk-full --check-syntax is not ready to build (a doc or suite path it names is missing):\n{result:#}"
        );
    }
}
