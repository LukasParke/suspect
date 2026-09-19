//! Conservative artifact comparison for controlled documentation edits.
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::collections::BTreeMap;
use suspect_codegen::OutFile;
use suspect_ir::contract::SourceId;

/// Remove genuine doc comments outside strings, preserving literal bytes.
/// Ambiguous JS regexp/interpolated-template syntax declines rather than
/// guessing. Unchanged files never need this normalization.
fn executable(path: &str, text: &str) -> Result<String> {
    let rust = path.ends_with(".rs");
    let bytes = text.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if rust && bytes[i] == b'r' {
            let mut quote = i + 1;
            while bytes.get(quote) == Some(&b'#') {
                quote += 1;
            }
            if bytes.get(quote) == Some(&b'"') {
                let closing = format!("\"{}", "#".repeat(quote - i - 1));
                let end = text[quote + 1..]
                    .find(&closing)
                    .context("unclosed raw string")?
                    + quote
                    + 1
                    + closing.len();
                out.push_str(&text[i..end]);
                i = end;
                continue;
            }
        }
        let rust_char = rust
            && bytes[i] == b'\''
            && (bytes.get(i + 1) == Some(&b'\\')
                || text
                    .get(i + 1..)
                    .and_then(|text| text.chars().next())
                    .is_some_and(|ch| bytes.get(i + 1 + ch.len_utf8()) == Some(&b'\'')));
        if bytes[i] == b'"' || (!rust && matches!(bytes[i], b'\'' | b'`')) || rust_char {
            let delimiter = bytes[i];
            let start = i;
            i += 1;
            let mut closed = false;
            while i < bytes.len() {
                if !rust && delimiter == b'`' && bytes[i..].starts_with(b"${") {
                    bail!("interpolated template requires a native syntax comparison")
                }
                if bytes[i] == b'\\' {
                    i += 2;
                    continue;
                }
                if bytes[i] == delimiter {
                    i += 1;
                    closed = true;
                    break;
                }
                i += 1;
            }
            ensure!(closed, "unclosed quoted literal");
            out.push_str(&text[start..i]);
            continue;
        }
        if bytes[i..].starts_with(b"//") {
            let end = text[i..]
                .find('\n')
                .map_or(bytes.len(), |offset| i + offset);
            let doc = rust && (bytes[i..].starts_with(b"///") || bytes[i..].starts_with(b"//!"));
            if !doc {
                out.push_str(&text[i..end]);
                out.push('\n');
            } else {
                out.push(' ');
            }
            i = end;
            continue;
        }
        if bytes[i..].starts_with(b"/*") {
            let doc = bytes[i..].starts_with(b"/**") || rust && bytes[i..].starts_with(b"/*!");
            let start = i;
            i += 2;
            let mut depth = 1;
            while i < bytes.len() && depth > 0 {
                if rust && bytes[i..].starts_with(b"/*") {
                    depth += 1;
                    i += 2;
                } else if bytes[i..].starts_with(b"*/") {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            ensure!(depth == 0, "unclosed comment");
            if doc {
                out.push(' ');
            } else {
                out.push_str(&text[start..i]);
            }
            continue;
        }
        if !rust && bytes[i] == b'/' {
            bail!("regexp/division syntax requires a native syntax comparison")
        }
        let ch = text[i..].chars().next().expect("character boundary");
        if ch.is_whitespace() {
            if !out.ends_with(' ') {
                out.push(' ');
            }
        } else {
            out.push(ch);
        }
        i += ch.len_utf8();
    }
    Ok(out.trim().to_owned())
}

pub fn executable_changes(before: &[OutFile], after: &[OutFile]) -> Vec<String> {
    let old: BTreeMap<_, _> = before
        .iter()
        .map(|file| (file.path.as_str(), file.content.as_str()))
        .collect();
    after
        .iter()
        .filter(|file| file.path.ends_with(".ts") || file.path.ends_with(".rs"))
        .filter(|file| {
            old.get(file.path.as_str())
                .is_some_and(|old| *old != file.content)
        })
        .filter(|file| {
            match (
                executable(&file.path, old[file.path.as_str()]),
                executable(&file.path, &file.content),
            ) {
                (Ok(old), Ok(new)) => old != new,
                _ => true,
            }
        })
        .map(|file| file.path.clone())
        .collect()
}

fn documentation_json(path: &str, text: &str, operation: &SourceId) -> Result<Value> {
    let mut value: Value = serde_json::from_str(text)?;
    if path.ends_with("/http-manifest.json") {
        for item in value["operations"]
            .as_array_mut()
            .context("operation manifest")?
        {
            if item["source"]["document"] == operation.document().as_str()
                && item["source"]["pointer"] == operation.pointer()
            {
                let object = item.as_object_mut().context("manifest operation")?;
                object.remove("descriptionText");
                object.remove("hasSourceDescription");
            }
        }
    } else if path.ends_with("/examples.json") {
        // A prose edit shifts following source byte offsets, not example meaning.
        for finding in value["diagnostics"]
            .as_array_mut()
            .context("example findings")?
        {
            finding.as_object_mut().context("finding")?.remove("range");
        }
    }
    Ok(value)
}

fn description(text: &str, operation: &SourceId) -> Result<String> {
    let manifest: Value = serde_json::from_str(text)?;
    let matching = manifest["operations"]
        .as_array()
        .context("operation manifest")?
        .iter()
        .filter(|item| {
            item["source"]["document"] == operation.document().as_str()
                && item["source"]["pointer"] == operation.pointer()
        })
        .collect::<Vec<_>>();
    ensure!(
        matching.len() == 1,
        "edited operation must have one manifest entry"
    );
    Ok(matching[0]["descriptionText"]
        .as_str()
        .context("source description text")?
        .into())
}

pub fn docs_only(before: &[OutFile], after: &[OutFile], operation: &SourceId) -> Result<()> {
    let old: BTreeMap<_, _> = before
        .iter()
        .map(|file| (file.path.as_str(), file.content.as_str()))
        .collect();
    let new: BTreeMap<_, _> = after
        .iter()
        .map(|file| (file.path.as_str(), file.content.as_str()))
        .collect();
    ensure!(
        old.keys().eq(new.keys()),
        "docs-only edit added/removed an artifact"
    );
    ensure!(
        executable_changes(before, after).is_empty(),
        "docs-only edit changed executable syntax or requires native syntax verification"
    );
    let mut manifests = 0;
    for (manifest, page) in [
        ("typescript/http-manifest.json", "typescript/operations.ts"),
        ("rust/http-manifest.json", "rust/README.md"),
    ] {
        if let Some(before) = old.get(manifest) {
            let after = description(new[manifest], operation)?;
            ensure!(
                description(before, operation)? != after,
                "source description did not propagate to {manifest}"
            );
            // The controlled benchmark probe is plain ASCII prose, so its exact
            // changed text must also reach the relevant native documentation
            // source (TypeDoc comments for TS, the package guide for Rust).
            ensure!(
                new.get(page).is_some_and(|text| text.contains(&after)),
                "changed source description did not reach {page}"
            );
            manifests += 1;
        }
    }
    ensure!(
        manifests > 0,
        "docs-only proof requires an operation manifest"
    );
    let mut documentation_changed = false;
    for (path, content) in new {
        if old[path] == content {
            continue;
        }
        match path {
            "typescript/http.md" | "rust/README.md" => documentation_changed = true,
            "typescript/http-manifest.json"
            | "rust/http-manifest.json"
            | "typescript/examples.json"
            | "rust/examples.json" => ensure!(
                documentation_json(path, old[path], operation)?
                    == documentation_json(path, content, operation)?,
                "docs-only edit changed semantic manifest data: {path}"
            ),
            path if path.ends_with(".rs") || path.ends_with(".ts") => {}
            _ => bail!("docs-only edit changed an unrelated artifact: {path}"),
        }
    }
    ensure!(
        documentation_changed,
        "description edit did not update its documentation"
    );
    Ok(())
}
