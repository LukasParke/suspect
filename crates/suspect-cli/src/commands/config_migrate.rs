//! `suspect config migrate [MANIFEST]`: move a manifest's ignored
//! `lint`, `validate` and `editor` sections into `.suspect.yaml`.
//!
//! The settings file discovery finds from the manifest's directory gets
//! the keys it does not already carry (its own values win, as they do in
//! the current release), and the manifest gets `"config"` pointing at it.
//! Both files are edited as text: comments and key order survive.

use std::path::{Path, PathBuf};

use suspect_low::{LowDoc, NodeRef, ValueKind};

/// One setting moved out of the manifest: its `.suspect.yaml` path and
/// the value as YAML text.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Moved {
    path: Vec<String>,
    yaml: String,
}

/// What a migration did, for the report.
#[derive(Debug, Default)]
pub struct Report {
    /// The settings file written.
    pub config: PathBuf,
    /// Whether it was created.
    pub created: bool,
    /// One line per key: moved, kept or dropped.
    pub lines: Vec<String>,
}

/// Runs the migration and prints what it did.
///
/// # Errors
/// An unreadable manifest, or a settings file the keys cannot be inserted
/// into (flow-style YAML); nothing is written then.
pub fn migrate(manifest: &Path) -> anyhow::Result<i32> {
    let report = migrate_files(manifest)?;
    let shown = suspect_config::layer::shown(&report.config);
    if report.lines.is_empty() {
        println!("{}: nothing to migrate", manifest.display());
        return Ok(0);
    }
    println!(
        "{} {}",
        if report.created { "created" } else { "updated" },
        shown.display()
    );
    for line in &report.lines {
        println!("  {line}");
    }
    println!("updated {}", manifest.display());
    Ok(0)
}

/// The migration, returning what it did.
///
/// # Errors
/// As [`migrate`].
pub fn migrate_files(manifest: &Path) -> anyhow::Result<Report> {
    let text = std::fs::read_to_string(manifest)
        .map_err(|e| anyhow::anyhow!("{}: {e}", manifest.display()))?;
    let doc = parse(manifest, &text)?;
    let dir = manifest
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let dir = suspect_config::discover::start_dir(&dir);
    let root = doc.root();
    if root.kind() != ValueKind::Object {
        anyhow::bail!("{}: the manifest is not a JSON object", manifest.display());
    }
    let mut lines = Vec::new();
    let moved = policy(&root, &mut lines);
    let sections: Vec<&str> = ["lint", "validate", "editor"]
        .into_iter()
        .filter(|k| root.get(k).is_some())
        .collect();
    if sections.is_empty() {
        return Ok(Report {
            config: dir.join(".suspect.yaml"),
            ..Report::default()
        });
    }

    // Where the keys go: the settings file discovery finds (ignoring this
    // manifest's own `config`, which is what is being added).
    let found = crate::settings::settings_file_above(&dir);
    let (target, existing, extends) = match found {
        Some(file) if file.parent() == Some(dir.as_path()) => {
            let text = std::fs::read_to_string(&file)?;
            (file, Some(text), None)
        }
        Some(shared) => (dir.join(".suspect.yaml"), None, Some(shared)),
        None => (dir.join(".suspect.yaml"), None, None),
    };

    let new_text = match &existing {
        Some(text) => {
            let doc = parse(&target, text)?;
            let mut out = text.clone();
            let mut inserts: Vec<(usize, String)> = Vec::new();
            let mut refused = Vec::new();
            for item in &moved {
                let key = item.path.join(".");
                if present(&doc.root(), &item.path) {
                    lines.push(format!("{key}: kept the {} value", file_name(&target)));
                    continue;
                }
                match insertion(&doc, text, &item.path, &item.yaml) {
                    Some(insert) => {
                        lines.push(format!("{key}: moved ({})", item.yaml));
                        inserts.push(insert);
                    }
                    None => refused.push(key),
                }
            }
            if !refused.is_empty() {
                anyhow::bail!(
                    "{}: cannot insert under a flow-style section; add these keys by hand: {}",
                    target.display(),
                    refused.join(", ")
                );
            }
            // Later offsets first, so earlier ones stay valid; equal
            // offsets keep their order.
            // At one offset (a nested section ending where its parent does),
            // the deeper keys go first.
            inserts.sort_by_key(|(at, chunk)| {
                std::cmp::Reverse((*at, chunk.len() - chunk.trim_start().len()))
            });
            let mut grouped: Vec<(usize, String)> = Vec::new();
            for (at, chunk) in inserts {
                match grouped.last_mut() {
                    Some((last, text)) if *last == at => text.push_str(&chunk),
                    _ => grouped.push((at, chunk)),
                }
            }
            if !out.ends_with('\n') && !out.is_empty() {
                out.push('\n');
            }
            for (at, chunk) in grouped {
                let at = at.min(out.len());
                out.insert_str(at, &chunk);
            }
            out
        }
        None => {
            let mut out = String::from(
                "# suspect settings: read by every suspect command, CI and the editor.\n",
            );
            if let Some(shared) = &extends {
                let relative = relative_to(shared, &dir);
                out.push_str(&format!("extends: {}\n", relative.display()));
                lines.push(format!("extends: {}", relative.display()));
            }
            out.push_str(&render(&moved));
            for item in &moved {
                lines.push(format!("{}: moved ({})", item.path.join("."), item.yaml));
            }
            out
        }
    };

    let relative = relative_to(&target, &dir);
    let manifest_text = rewrite_manifest(&text, &doc, &sections, &relative.display().to_string())?;
    std::fs::write(&target, new_text)?;
    std::fs::write(manifest, manifest_text)?;
    for section in &sections {
        lines.push(format!("removed `{section}` from the manifest"));
    }
    lines.push(format!("\"config\": \"{}\"", relative.display()));
    Ok(Report {
        config: target,
        created: existing.is_none(),
        lines,
    })
}

fn parse(path: &Path, text: &str) -> anyhow::Result<LowDoc> {
    let uri = suspect_source::Uri::from_path(path)?;
    let doc = LowDoc::parse(
        uri,
        suspect_source::Source::from_vec(text.as_bytes().to_vec()),
    );
    if let Some(error) = doc.syntax_errors().first() {
        anyhow::bail!("{}: {}", path.display(), error.message);
    }
    Ok(doc)
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// The manifest's policy as `.suspect.yaml` paths, in manifest order.
/// Editor keys with no settings home are reported as dropped.
fn policy(root: &NodeRef<'_>, lines: &mut Vec<String>) -> Vec<Moved> {
    let mut out = Vec::new();
    for section in ["lint", "validate"] {
        if let Some(node) = root.get(section) {
            flatten(&node, vec![section.to_owned()], &mut out);
        }
    }
    if let Some(editor) = root.get("editor") {
        for entry in editor.entries() {
            let Some(value) = entry.value else { continue };
            let prefix: Vec<String> = match entry.key {
                "inlay_hints" | "inlayHints" => vec!["editor".into(), "inlay_hints".into()],
                "ref" | "refs" => vec!["ref".into()],
                "formatting" => vec!["format".into()],
                other => {
                    lines.push(format!(
                        "editor.{other}: dropped (no .suspect.yaml setting)"
                    ));
                    continue;
                }
            };
            flatten(&value, prefix, &mut out);
        }
    }
    // `max_docs` and `sort_keys` already carry their settings spelling;
    // the camelCase client spellings map onto it.
    for item in &mut out {
        if let Some(last) = item.path.last_mut() {
            *last = suspect_config::schema::snake_case(last);
        }
        if item.path == ["editor", "inlay_hints", "ref_targets"] {
            item.path = vec!["editor".into(), "inlay_hints".into(), "refs".into()];
        }
    }
    out
}

fn flatten(node: &NodeRef<'_>, path: Vec<String>, out: &mut Vec<Moved>) {
    if node.kind() == ValueKind::Object {
        let entries = node.entries();
        if entries.is_empty() {
            // `rules: {}` is worth nothing in the settings file.
            return;
        }
        for entry in entries {
            let mut next = path.clone();
            next.push(entry.key.to_owned());
            if let Some(value) = entry.value {
                flatten(&value, next, out);
            }
        }
        return;
    }
    if node.kind() == ValueKind::Null {
        return;
    }
    out.push(Moved {
        path,
        yaml: yaml_scalar(node),
    });
}

/// A scalar or list as YAML text: plain when that reads back as the same
/// string, JSON-quoted otherwise.
fn yaml_scalar(node: &NodeRef<'_>) -> String {
    match node.kind() {
        ValueKind::Str => {
            let s = node.as_str().unwrap_or_default();
            plain_or_quoted(s)
        }
        ValueKind::Array => {
            let items: Vec<String> = node.items().iter().map(yaml_scalar).collect();
            format!("[{}]", items.join(", "))
        }
        _ => String::from_utf8_lossy(node.raw_text()).trim().to_owned(),
    }
}

fn plain_or_quoted(s: &str) -> String {
    let plain = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
        && !matches!(
            s.to_ascii_lowercase().as_str(),
            "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "~"
        )
        && s.parse::<f64>().is_err();
    // `off` is a severity name here, and YAML 1.2 reads it as a string;
    // quoting it anyway keeps YAML 1.1 readers from turning it into false.
    if plain {
        s.to_owned()
    } else {
        serde_json::to_string(s).unwrap_or_else(|_| s.to_owned())
    }
}

fn key_text(key: &str) -> String {
    let plain = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | '$'));
    if plain {
        key.to_owned()
    } else {
        serde_json::to_string(key).unwrap_or_else(|_| key.to_owned())
    }
}

/// A fresh YAML document for `moved`, nested by path.
fn render(moved: &[Moved]) -> String {
    let mut out = String::new();
    let mut open: Vec<String> = Vec::new();
    for item in moved {
        let common = open
            .iter()
            .zip(&item.path)
            .take_while(|(a, b)| a == b)
            .count()
            .min(item.path.len() - 1);
        open.truncate(common);
        for (depth, part) in item.path.iter().enumerate().skip(common) {
            out.push_str(&"  ".repeat(depth));
            out.push_str(&key_text(part));
            out.push(':');
            if depth + 1 == item.path.len() {
                out.push(' ');
                out.push_str(&item.yaml);
            } else {
                open.push(part.clone());
            }
            out.push('\n');
        }
    }
    out
}

/// Whether `path` already has a value in the settings file.
fn present(root: &NodeRef<'_>, path: &[String]) -> bool {
    let key = path.join(".");
    let mut spellings = vec![key.clone()];
    if let Some(found) = suspect_config::schema::lookup(&key) {
        spellings.push(found.setting.path.to_owned());
    }
    spellings.iter().any(|spelling| {
        let mut node = Some(*root);
        for part in spelling.split('.') {
            node = node.and_then(|n| n.get(part));
        }
        // A map key (`lint.rules.<id>`) may itself contain no dots, but the
        // canonical spelling could; fall back to walking the original path.
        node.is_some()
    }) || {
        let mut node = Some(*root);
        for part in path {
            node = node.and_then(|n| n.get(part));
        }
        node.is_some()
    }
}

/// Where to insert `path: yaml` into the existing file: under the deepest
/// block-style section that exists, at the end of that section, indented
/// like its keys. `None` when that section is flow style.
fn insertion(doc: &LowDoc, text: &str, path: &[String], yaml: &str) -> Option<(usize, String)> {
    let root = doc.root();
    let mut depth = 0;
    let mut node = root;
    while depth + 1 < path.len() {
        match node.get(&path[depth]) {
            Some(child) if child.kind() == ValueKind::Object => {
                node = child;
                depth += 1;
            }
            Some(child) if child.kind() == ValueKind::Null => {
                // `lint:` with nothing under it: treat as an empty block.
                return None;
            }
            Some(_) => return None,
            None => break,
        }
    }
    let rest = &path[depth..];
    let (at, indent) = if depth == 0 {
        (text.len(), 0)
    } else {
        let range = node.byte_range();
        if text[range.clone()].trim_start().starts_with('{') {
            return None;
        }
        let indent = node
            .entries()
            .first()
            .map_or(depth * 2, |e| e.key_node.line_col().1 as usize);
        // After the section's last line.
        let end = text[range.end..]
            .find('\n')
            .map_or(text.len(), |i| range.end + i + 1);
        (end, indent)
    };
    let mut chunk = String::new();
    for (i, part) in rest.iter().enumerate() {
        chunk.push_str(&" ".repeat(indent + i * 2));
        chunk.push_str(&key_text(part));
        chunk.push(':');
        if i + 1 == rest.len() {
            chunk.push(' ');
            chunk.push_str(yaml);
        }
        chunk.push('\n');
    }
    Some((at, chunk))
}

/// The manifest text without `sections`, with `"config"` as its first key
/// (replacing an existing one). Entries are kept as written, joined with
/// the separator the file already uses between its first two keys.
fn rewrite_manifest(
    text: &str,
    doc: &LowDoc,
    sections: &[&str],
    config: &str,
) -> anyhow::Result<String> {
    let entries = doc.root().entries();
    let span = |e: &suspect_low::Entry<'_>| {
        let start = e.key_node.byte_range().start;
        let end = e
            .value
            .map_or(e.key_node.byte_range().end, |v| v.byte_range().end);
        (start, end)
    };
    let (Some(first), Some(last)) = (entries.first(), entries.last()) else {
        anyhow::bail!("the manifest has no keys");
    };
    let (head_end, _) = span(first);
    let (_, tail_start) = span(last);
    let separator = match entries.get(1) {
        Some(second) => text[span(first).1..span(second).0].to_owned(),
        None => ", ".to_owned(),
    };
    let mut kept = vec![format!("\"config\": {}", serde_json::to_string(config)?)];
    for entry in &entries {
        if sections.contains(&entry.key) || entry.key == "config" {
            continue;
        }
        let (start, end) = span(entry);
        kept.push(text[start..end].to_owned());
    }
    Ok(format!(
        "{}{}{}",
        &text[..head_end],
        kept.join(&separator),
        &text[tail_start..]
    ))
}

/// `path` relative to `dir`, lexically (`../.suspect.yaml`).
fn relative_to(path: &Path, dir: &Path) -> PathBuf {
    let path: Vec<_> = path.components().collect();
    let dir: Vec<_> = dir.components().collect();
    let common = path.iter().zip(&dir).take_while(|(a, b)| a == b).count();
    let mut out = PathBuf::new();
    for _ in common..dir.len() {
        out.push("..");
    }
    for part in &path[common..] {
        out.push(part.as_os_str());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_render_as_yaml_that_reads_back_the_same() {
        assert_eq!(plain_or_quoted("warning"), "warning");
        assert_eq!(plain_or_quoted("off"), "\"off\"");
        assert_eq!(
            plain_or_quoted("rules/my rules.yaml"),
            "\"rules/my rules.yaml\""
        );
        assert_eq!(plain_or_quoted("1.0"), "\"1.0\"");
    }

    #[test]
    fn relative_paths_climb_out_of_the_directory() {
        assert_eq!(
            relative_to(Path::new("/r/.suspect.yaml"), Path::new("/r/svc")),
            PathBuf::from("../.suspect.yaml")
        );
        assert_eq!(
            relative_to(Path::new("/r/svc/.suspect.yaml"), Path::new("/r/svc")),
            PathBuf::from(".suspect.yaml")
        );
    }
}
