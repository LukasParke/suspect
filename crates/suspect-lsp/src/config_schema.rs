//! Knowledge about suspect's own configuration files, so that editing them
//! in an editor is as good as editing an OpenAPI document.
//!
//! Two files get first-class treatment: `.suspect.yaml` (the settings the
//! CLI, CI and the language server all read) and `suspect.project.json`
//! (the project manifest). The tables are `suspect_config::schema`'s, the
//! same rows the CLI parses with and the JSON Schemas are generated from,
//! so hover, completion and validation here cannot disagree with them.
//!
//! Everything keys off the document's *file name*, so a `.suspect.yaml`
//! belonging to some unrelated tool never gets this treatment.

use suspect_low::{LowDoc, NodeRef, ValueKind};
use suspect_syntax::SyntaxKind;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, Diagnostic, DiagnosticSeverity, NumberOrString,
};

use crate::navigation::node_at;

/// Which configuration file a document is, if it is one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// `.suspect.yaml`
    Settings,
    /// `suspect.project.json`
    Project,
}

impl FileKind {
    /// The document's own name, for titles and messages.
    const fn display(self) -> &'static str {
        match self {
            Self::Settings => ".suspect.yaml",
            Self::Project => "suspect.project.json",
        }
    }
}

/// Recognises suspect's configuration files by file name.
///
/// The settings names come from [`suspect_config::CONFIG_NAMES`] itself
/// rather than a copy, so a new name in the loader cannot leave the editor
/// validating a file the toolchain ignores — or skipping one it reads.
#[must_use]
pub fn kind_of(file_name: &str) -> Option<FileKind> {
    if suspect_config::CONFIG_NAMES.contains(&file_name) {
        return Some(FileKind::Settings);
    }
    match file_name {
        "suspect.project.json" => Some(FileKind::Project),
        _ => None,
    }
}

pub use suspect_config::schema::{Kind, Setting as Field};
use suspect_config::schema::{MANIFEST, SETTINGS, TARGET_FIELDS};

/// The scope suffix used for a key inside a `codegen` target, so it cannot
/// collide with the top-level `codegen` list.
const TARGET_SCOPE: &str = "codegen[]";

/// How many edits still count as a typo worth suggesting. Three covers
/// `min_severty` → `min_severity` and `warn` → `warning` without offering a
/// "did you mean" on every unknown key.
const SUGGEST_EDITS: usize = 3;

const fn table(kind: FileKind) -> &'static [Field] {
    match kind {
        FileKind::Settings => SETTINGS,
        FileKind::Project => MANIFEST,
    }
}

/// The field at a dot path, if the schema has one.
#[must_use]
pub fn lookup(kind: FileKind, path: &str) -> Option<&'static Field> {
    if let Some(name) = path.strip_prefix(&format!("{TARGET_SCOPE}.")) {
        return TARGET_FIELDS.iter().find(|f| f.path == name);
    }
    suspect_config::schema::lookup_in(table(kind), path).map(|found| found.setting)
}

/// The fields valid directly inside `scope` (`""` is the document root).
#[must_use]
pub fn children(kind: FileKind, scope: &str) -> Vec<&'static Field> {
    if let Some(name) = scope.strip_prefix(TARGET_SCOPE) {
        let _ = name;
        return TARGET_FIELDS.iter().collect();
    }
    immediate(table(kind), scope)
}

/// The fields valid inside one `codegen` target, as paths from the root.
#[must_use]
pub fn target_children(scope: &str) -> Vec<&'static Field> {
    let _ = scope;
    TARGET_FIELDS.iter().collect()
}

fn immediate(table: &'static [Field], scope: &str) -> Vec<&'static Field> {
    let prefix = if scope.is_empty() {
        String::new()
    } else {
        format!("{scope}.")
    };
    table
        .iter()
        .filter(|f| {
            f.path.len() > prefix.len()
                && f.path.starts_with(&prefix)
                && !f.path[prefix.len()..].contains('.')
        })
        .collect()
}

/// The closest known key name to `name` within `scope`, for "did you mean".
#[must_use]
pub fn closest(kind: FileKind, scope: &str, name: &str) -> Option<&'static str> {
    let candidates = children(kind, scope);
    candidates
        .iter()
        .filter_map(|f| f.path.rsplit('.').next())
        .min_by_key(|candidate| distance(candidate, name))
        .filter(|candidate| distance(candidate, name) <= SUGGEST_EDITS)
}

/// Plain Levenshtein distance; these are short configuration keys.
fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut next = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            next.push((row[j] + cost).min(row[j + 1] + 1).min(next[j] + 1));
        }
        row = next;
    }
    row[b.len()]
}

/// A configuration problem, in byte offsets so the caller converts them.
#[derive(Debug, Clone)]
pub struct Problem {
    /// Byte range to underline.
    pub range: std::ops::Range<usize>,
    /// Human-readable message.
    pub message: String,
    /// A closer key to suggest, when the problem is a typo.
    pub suggestion: Option<String>,
}

/// Checks a whole configuration document.
///
/// Reports only what the schema knows to be wrong: an unknown key, a value
/// of the wrong kind, or a value outside an enumeration. It never reports a
/// *missing* key — most keys are optional, and a report about an absent key
/// is noise in an editor.
#[must_use]
pub fn problems(kind: FileKind, low: &LowDoc) -> Vec<Problem> {
    let mut out = Vec::new();
    check_mapping(kind, &low.root(), "", &mut out);
    out
}

fn check_mapping(kind: FileKind, node: &NodeRef<'_>, scope: &str, out: &mut Vec<Problem>) {
    if node.kind() != ValueKind::Object {
        return;
    }
    let open_map = lookup(kind, scope).is_some_and(|f| matches!(f.kind, Kind::Map(_)));
    for entry in node.entries() {
        let path = join(scope, entry.key);
        let Some(field) = lookup(kind, &path) else {
            if open_map {
                // A user-chosen map accepts any key.
                continue;
            }
            out.push(Problem {
                range: entry.key_node.byte_range(),
                message: format!("`{}` is not a {} key.", entry.key, kind.display()),
                suggestion: closest(kind, scope, entry.key).map(str::to_owned),
            });
            continue;
        };
        if let Some(found) = suspect_config::schema::lookup_in(table(kind), &path)
            && let Some(alias) = found.alias
        {
            out.push(Problem {
                range: entry.key_node.byte_range(),
                message: format!("`{alias}` is deprecated; use `{}`.", field.path),
                suggestion: None,
            });
        }
        let Some(value) = entry.value else { continue };
        if let Some(problem) = check_value(kind, &path, field, &value, entry.key_node.byte_range())
        {
            out.push(problem);
        }
        match field.kind {
            Kind::Section => check_mapping(kind, &value, &path, out),
            Kind::Targets => {
                for target in value.items() {
                    check_mapping(kind, &target, TARGET_SCOPE, out);
                }
            }
            _ => {}
        }
    }
}

fn check_value(
    kind: FileKind,
    path: &str,
    field: &Field,
    value: &NodeRef<'_>,
    key_range: std::ops::Range<usize>,
) -> Option<Problem> {
    let value_kind = value.kind();
    let shape_ok = match field.kind {
        Kind::Bool => matches!(value_kind, ValueKind::Bool | ValueKind::Null),
        Kind::Count | Kind::Port => matches!(value_kind, ValueKind::Int | ValueKind::Null),
        Kind::Str
        | Kind::Path
        | Kind::Url
        | Kind::Enum(_)
        | Kind::Severity
        | Kind::PolicySeverity => matches!(value_kind, ValueKind::Str | ValueKind::Null),
        Kind::List | Kind::Targets => matches!(value_kind, ValueKind::Array | ValueKind::Null),
        Kind::Section | Kind::Map(_) | Kind::Object => {
            matches!(value_kind, ValueKind::Object | ValueKind::Null)
        }
    };
    if !shape_ok {
        return Some(Problem {
            range: key_range,
            message: format!(
                "`{}` expects {} but found {found}.",
                display_path(kind, path),
                field.kind.type_name(),
                found = describe(value_kind)
            ),
            suggestion: None,
        });
    }
    if field.values().is_empty() {
        return None;
    }
    // A non-string value already failed the shape check above, so there is
    // nothing to compare against an enumeration.
    let actual = value.as_str()?;
    if field.values().contains(&actual) {
        return None;
    }
    let suggestion = field
        .values()
        .iter()
        .copied()
        .min_by_key(|candidate| distance(candidate, actual))
        .filter(|candidate| distance(candidate, actual) <= SUGGEST_EDITS)
        .map(str::to_owned);
    Some(Problem {
        range: key_range,
        message: format!(
            "{actual} is not a valid {display} — expected one of: {allowed}.",
            display = display_path(kind, path),
            // Plain list, no inner backticks: nesting them inside the
            // message's own quoting reads as one mangled word.
            allowed = field.values().join(", "),
        ),
        suggestion,
    })
}

const fn describe(kind: ValueKind) -> &'static str {
    match kind {
        ValueKind::Bool => "a boolean",
        ValueKind::Int => "an integer",
        ValueKind::Float => "a number",
        ValueKind::Str => "a string",
        ValueKind::Object => "a section",
        ValueKind::Array => "a list",
        ValueKind::Null => "nothing",
    }
}

/// How a path reads in a message: `codegen[]` is noise in prose.
fn display_path(kind: FileKind, path: &str) -> String {
    let _ = kind;
    path.strip_prefix(&format!("{TARGET_SCOPE}."))
        .map_or_else(|| path.to_owned(), str::to_owned)
}

/// A mapping key's text, without the quotes a JSON document puts around it.
fn key_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text
        .strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .or_else(|| text.strip_prefix('\'').and_then(|t| t.strip_suffix('\'')))
        .unwrap_or(&text);
    trimmed.to_owned()
}

fn join(scope: &str, key: &str) -> String {
    if scope.is_empty() {
        key.to_owned()
    } else {
        format!("{scope}.{key}")
    }
}

/// The dot path of the configuration key under `offset`, and its byte range.
///
/// Recognises the key of the pair the cursor is in, whether the cursor is on
/// the key text or on the value beside it — the same generosity
/// [`crate::navigation::ref_value_node`] applies to `$ref`.
#[must_use]
pub fn key_path_at(low: &LowDoc, offset: usize) -> Option<(String, std::ops::Range<usize>)> {
    let mut cur = node_at(low, offset)?;
    loop {
        if cur.kind() == SyntaxKind::Pair {
            // Walking up from the smallest containing node, the first pair
            // reached is the innermost one — so its key is the answer
            // whether the cursor is on the key text or on the value beside
            // it. Descending into the value first loses the key entirely.
            let key = cur.child_by_field("key")?;
            let range = key.byte_range();
            let name = key_text(key.scalar_bytes());
            return Some((join(&scope_of(low, range.start)?, &name), range));
        }
        cur = cur.parent()?;
    }
}

/// Like [`key_path_at`], but only when the cursor is inside the pair's
/// *value*.
///
/// Enumeration values apply to values, not keys, and a cursor on the blank
/// line after `min_severity: warning` is still inside that pair's line span
/// without being anywhere near its value. Offering severities there would be
/// right about the document and wrong about the user's intent, which is to
/// add the next key.
#[must_use]
pub fn value_key_path_at(low: &LowDoc, offset: usize) -> Option<(String, std::ops::Range<usize>)> {
    let node = node_at(low, offset)?;
    let mut cur = node;
    loop {
        if cur.kind() == SyntaxKind::Pair {
            let key = cur.child_by_field("key")?;
            let range = key.byte_range();
            let inside_value = cur.child_by_field("value").is_some_and(|value| {
                let vr = value.byte_range();
                vr.start <= offset && offset < vr.end.max(vr.start + 1)
            });
            if inside_value {
                let name = key_text(key.scalar_bytes());
                return Some((join(&scope_of(low, range.start)?, &name), range));
            }
        }
        cur = cur.parent()?;
    }
}

/// The dot path of the mapping (or list-of-targets) a byte sits in.
fn scope_of(low: &LowDoc, offset: usize) -> Option<String> {
    let mut cur = node_at(low, offset)?;
    loop {
        if cur.kind() == SyntaxKind::Mapping {
            let node = NodeRef::new(cur);
            if node.kind() == ValueKind::Object {
                let mut tokens: Vec<String> = node
                    .path_from_root()
                    .tokens()
                    .iter()
                    .map(|t| t.to_string())
                    .collect();
                // A mapping inside a sequence is one `codegen` target, so
                // mark the list it belongs to; otherwise its keys would be
                // looked up against the wrong scope.
                let in_target_list = cur
                    .parent()
                    .filter(|p| p.kind() == SyntaxKind::Sequence)
                    .and_then(|s| s.parent())
                    .is_some_and(|p| p.kind() == SyntaxKind::Pair);
                if in_target_list && !tokens.is_empty() {
                    tokens[0] = format!("{}[]", tokens[0]);
                }
                return Some(tokens.join("."));
            }
        }
        cur = cur.parent()?;
    }
}

/// Hover text for the configuration key under `offset`, if there is one.
#[must_use]
pub fn hover(kind: FileKind, low: &LowDoc, offset: usize) -> Option<String> {
    let (path, _) = key_path_at(low, offset)?;
    let field = lookup(kind, &path).or_else(|| closest_field(kind, &path))?;
    Some(hover_markdown(kind, field))
}

/// The hover body for a field.
///
/// The same card frame as the OpenAPI hover surfaces: a `### \`key\``
/// heading, an italic subtitle carrying the qualified path, prose, and
/// a `| Field | Value |` table for the type/default/allowed facts.
#[must_use]
pub fn hover_markdown(kind: FileKind, field: &Field) -> String {
    let name = if field.path.is_empty() {
        // The root field describes the whole file; its key *is* the file.
        kind.display()
    } else {
        field.path.rsplit('.').next().unwrap_or(field.path)
    };
    let mut out = format!("### `{name}`");
    if !field.path.is_empty() {
        out.push_str(&format!("\n\n*`{}`*", display_path(kind, field.path)));
    }
    out.push_str(&format!("\n\n{}", field.doc));
    out.push_str("\n\n| Field | Value |\n|---|---|");
    out.push_str(&format!("\n| Type | {} |", field.kind.type_name()));
    if let Some(default) = field.default {
        out.push_str(&format!("\n| Default | `{default}` |"));
    }
    if !field.values().is_empty() {
        out.push_str(&format!(
            "\n| One of | {} |",
            field
                .values()
                .iter()
                .map(|v| format!("`{v}`"))
                .collect::<Vec<_>>()
                .join(" · ")
        ));
    }
    out.push_str(&format!("\n\n---\n\n*{}*", kind.display()));
    out
}

/// A field whose path is the longest known prefix of `path`, so hovering a
/// section header still says something useful.
fn closest_field(kind: FileKind, path: &str) -> Option<&'static Field> {
    let mut best: Option<&'static Field> = None;
    for field in table(kind) {
        if field.path.is_empty() {
            continue;
        }
        if path == field.path
            || path.starts_with(&format!("{}.", field.path))
                && best.is_none_or(|b| b.path.len() < field.path.len())
        {
            best = Some(field);
        }
    }
    best
}

/// Completions for a configuration document: the permitted values when the
/// cursor is on an enum value, otherwise the keys valid in the enclosing
/// scope that are not already present.
#[must_use]
pub fn completions(kind: FileKind, low: &LowDoc, offset: usize) -> Vec<CompletionItem> {
    if let Some((path, _)) = value_key_path_at(low, offset)
        && let Some(field) = lookup(kind, &path)
        && !field.values().is_empty()
    {
        return field
            .values()
            .iter()
            .map(|value| CompletionItem {
                label: (*value).to_owned(),
                detail: Some(format!("{} — {}", field.doc, display_path(kind, &path))),
                kind: Some(CompletionItemKind::VALUE),
                sort_text: Some(format!("0{value}")),
                ..CompletionItem::default()
            })
            .collect();
    }
    let scope = scope_of(low, offset).unwrap_or_default();
    let present: Vec<String> = node_at(low, offset)
        .and_then(|n| {
            let mut cur = n;
            loop {
                if cur.kind() == SyntaxKind::Mapping
                    && NodeRef::new(cur).kind() == ValueKind::Object
                {
                    return Some(NodeRef::new(cur));
                }
                cur = cur.parent()?;
            }
        })
        .map(|node| node.entries().iter().map(|e| e.key.to_owned()).collect())
        .unwrap_or_default();
    let candidates = if scope == TARGET_SCOPE {
        target_children(&scope)
    } else {
        children(kind, &scope)
    };
    candidates
        .into_iter()
        .filter(|f| !f.path.is_empty())
        .filter(|f| {
            let name = f.path.rsplit('.').next().unwrap_or(f.path);
            !present.iter().any(|p| p == name)
        })
        .map(|f| CompletionItem {
            label: f.path.rsplit('.').next().unwrap_or(f.path).to_owned(),
            detail: Some(f.doc.to_owned()),
            kind: Some(CompletionItemKind::PROPERTY),
            sort_text: Some(format!("1{}", f.path)),
            ..CompletionItem::default()
        })
        .collect()
}

/// The schema's problems as LSP diagnostics, ready to merge into a pull
/// report.
#[must_use]
pub fn diagnostics(low: &LowDoc) -> Vec<Diagnostic> {
    let Some(kind) = kind_for(low) else {
        return Vec::new();
    };
    let inner = low.inner();
    let (bytes, li) = (inner.bytes(), inner.line_index());
    // Policy sections in the manifest are deprecated (option A): said on
    // the key, with the same text the CLI prints.
    let deprecated: Vec<Diagnostic> = match kind {
        FileKind::Project => low
            .root()
            .entries()
            .into_iter()
            .filter(|e| matches!(e.key, "lint" | "validate" | "editor"))
            .map(|e| Diagnostic {
                range: crate::state::lsp_range(bytes, li, e.key_node.byte_range()),
                severity: Some(DiagnosticSeverity::WARNING),
                code: Some(NumberOrString::String("config-manifest-policy".to_owned())),
                source: Some("suspect".to_owned()),
                message: suspect_config::file::manifest_policy_warning(e.key),
                ..Diagnostic::default()
            })
            .collect(),
        FileKind::Settings => Vec::new(),
    };
    problems(kind, low)
        .into_iter()
        .map(|problem| {
            let mut message = problem.message;
            if let Some(suggestion) = &problem.suggestion {
                message.push_str(&format!(" Did you mean `{suggestion}`?"));
            }
            Diagnostic {
                range: crate::state::lsp_range(bytes, li, problem.range),
                severity: Some(DiagnosticSeverity::WARNING),
                code: Some(NumberOrString::String("suspect.config".to_owned())),
                source: Some("suspect".to_owned()),
                message,
                ..Diagnostic::default()
            }
        })
        .chain(deprecated)
        .collect()
}

/// Which configuration flavour a document is, if any.
#[must_use]
pub fn kind_for(low: &LowDoc) -> Option<FileKind> {
    let path = low.uri().as_path()?;
    kind_of(path.file_name()?.to_str()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use suspect_source::{Source, Uri};

    fn doc(name: &str, text: &str) -> LowDoc {
        LowDoc::parse(
            Uri::parse(&format!("file:///w/{name}")).unwrap(),
            Source::from_vec(text.as_bytes().to_vec()),
        )
    }

    fn offset(text: &str, needle: &str) -> usize {
        text.find(needle).expect("needle")
    }

    const SETTINGS_YAML: &str = "\
lint:
  min_severity: warning
  ruleset: ./rules.yaml
validate:
  strict_format: true
format:
  json: false
  yaml: true
docs:
  style: sveltekit
codegen:
  package_version: 1.1.1
";

    #[test]
    fn recognises_only_suspects_own_files() {
        for name in suspect_config::CONFIG_NAMES {
            assert_eq!(kind_of(name), Some(FileKind::Settings), "{name}");
        }
        assert_eq!(kind_of("suspect.project.json"), Some(FileKind::Project));
        assert_eq!(kind_of("openapi.yaml"), None);
        assert_eq!(kind_of(".github/workflows/ci.yaml"), None);
    }

    #[test]
    fn every_key_in_the_real_settings_file_is_known() {
        let low = doc(".suspect.yaml", SETTINGS_YAML);
        let problems = problems(FileKind::Settings, &low);
        assert!(problems.is_empty(), "{problems:#?}");
    }

    #[test]
    fn every_key_in_the_real_manifest_is_known() {
        // The manifest the Plex repository actually ships.
        let text = r#"{
  "version": 1,
  "name": "plex-api-spec",
  "entry": "plex-api-spec.yaml",
  "publish": {"output": ".suspect/spec.yaml", "profiles": {"cloud": ["p.yaml"]}},
  "contract": {"output": ".suspect/contract"},
  "lint": {"min_severity": "warning"},
  "docs": {"style": "sveltekit", "output": ".suspect/docs"},
  "codegen": [{"name": "typescript", "profile": "typescript-http",
    "package_name": "@p/p", "package_version": "1.1.1", "out": "out",
    "operation_id": ["a", "b"]}],
  "tests": {"arazzo": ["w.yaml"], "base_url": "http://localhost:32400"}
}"#;
        let low = doc("suspect.project.json", text);
        let found = problems(FileKind::Project, &low);
        assert!(found.is_empty(), "{found:#?}");
    }

    #[test]
    fn an_unknown_key_is_reported_with_a_suggestion() {
        let low = doc(
            ".suspect.yaml",
            "lint:\n  min_severity: warning\n  min_severty: error\n",
        );
        let found = problems(FileKind::Settings, &low);
        assert_eq!(found.len(), 1, "{found:#?}");
        assert!(found[0].message.contains("`min_severty`"), "{found:#?}");
        assert_eq!(found[0].suggestion.as_deref(), Some("min_severity"));
    }

    #[test]
    fn an_out_of_range_enumeration_is_reported() {
        let low = doc(".suspect.yaml", "lint:\n  min_severity: warn\n");
        let found = problems(FileKind::Settings, &low);
        assert_eq!(found.len(), 1, "{found:#?}");
        assert!(
            found[0].message.contains("warn is not a valid"),
            "{found:#?}"
        );
        assert!(
            found[0].message.contains("expected one of: error, warning"),
            "{found:#?}"
        );
        assert_eq!(found[0].suggestion.as_deref(), Some("warning"));
    }

    #[test]
    fn a_value_of_the_wrong_type_is_reported() {
        let low = doc(".suspect.yaml", "validate:\n  strict_format: sometimes\n");
        let found = problems(FileKind::Settings, &low);
        assert_eq!(found.len(), 1, "{found:#?}");
        assert!(found[0].message.contains("expects boolean"), "{found:#?}");
    }

    #[test]
    fn hover_describes_a_key() {
        let low = doc(".suspect.yaml", SETTINGS_YAML);
        let md = hover(
            FileKind::Settings,
            &low,
            offset(SETTINGS_YAML, "min_severity"),
        )
        .expect("hover");
        assert!(md.starts_with("### `min_severity`"), "{md}");
        assert!(md.contains("*`lint.min_severity`*"), "{md}");
        assert!(md.contains("Minimum severity"), "{md}");
        assert!(md.contains("| Type | string |"), "{md}");
        assert!(
            md.contains("| One of | `error` · `warning` · `info` · `hint` |"),
            "{md}"
        );
        assert!(md.ends_with("\n\n---\n\n*.suspect.yaml*"), "{md}");
    }

    #[test]
    fn hover_works_from_the_value_side_too() {
        let low = doc(".suspect.yaml", SETTINGS_YAML);
        let md =
            hover(FileKind::Settings, &low, offset(SETTINGS_YAML, "sveltekit")).expect("hover");
        assert!(md.contains("Output style"), "{md}");
    }

    #[test]
    fn hover_describes_a_manifest_key() {
        let text = "{\n  \"entry\": \"a.yaml\"\n}\n";
        let low = doc("suspect.project.json", text);
        let md = hover(FileKind::Project, &low, offset(text, "entry")).expect("hover");
        assert!(md.contains("Entry OpenAPI document"), "{md}");
        assert!(md.contains("suspect.project.json"), "{md}");
    }

    #[test]
    fn manifest_policy_keys_hover_and_complete() {
        // The committed policy sections read like the rest of the manifest:
        // hover names what the key does, completion offers what is missing.
        let text = "{\"entry\": \"a.yaml\", \"lint\": {\"min_severity\": \"warning\"}, \
                    \"validate\": {\"strict_format\": true}, \"editor\": {\"inlay_hints\": {}}}";
        let low = doc("suspect.project.json", text);
        let md = hover(FileKind::Project, &low, offset(text, "min_severity")).expect("lint hover");
        assert!(
            md.contains("Deprecated") && md.contains(".suspect.yaml"),
            "{md}"
        );
        let md =
            hover(FileKind::Project, &low, offset(text, "strict_format")).expect("validate hover");
        assert!(md.contains("format"), "{md}");
        let md =
            hover(FileKind::Project, &low, offset(text, "editor")).expect("editor section hover");
        assert!(md.contains("Deprecated"), "{md}");

        // Completion inside the lint section offers the policy keys.
        let items = completions(FileKind::Project, &low, offset(text, "min_severity"));
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        for expected in ["ruleset", "rules", "recommended"] {
            assert!(labels.contains(&expected), "missing {expected}: {labels:?}");
        }
    }

    #[test]
    fn manifest_policy_sections_are_deprecated_on_the_key() {
        let text = "{\"entry\": \"a.yaml\", \"lint\": {}, \"config\": \".suspect.yaml\"}";
        let low = doc("suspect.project.json", text);
        let found: Vec<_> = diagnostics(&low)
            .into_iter()
            .filter(|d| d.code == Some(NumberOrString::String("config-manifest-policy".into())))
            .collect();
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0]
                .message
                .contains("`suspect config migrate` does both")
        );
        let settings = doc(".suspect.yaml", "lint: {}\n");
        assert!(diagnostics(&settings).is_empty());
    }

    #[test]
    fn manifest_policy_mistakes_are_diagnosed() {
        // A bad severity value in the manifest is flagged exactly like one
        // in `.suspect.yaml` would be.
        let text = "{\"lint\": {\"min_severity\": \"shout\"}}";
        let low = doc("suspect.project.json", text);
        let findings = diagnostics(&low);
        assert!(
            findings.iter().any(|f| f.message.contains("shout")),
            "the bad value is named: {findings:?}"
        );
    }

    #[test]
    fn completion_offers_the_keys_still_missing_from_a_section() {
        let low = doc(".suspect.yaml", "lint:\n  min_severity: warning\n");
        let items = completions(
            FileKind::Settings,
            &low,
            offset("lint:\n  min_severity", "min"),
        );
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["ruleset", "design", "rules", "recommended"],
            "{labels:?}"
        );
    }

    #[test]
    fn completion_offers_every_root_key_when_empty() {
        let low = doc(".suspect.yaml", "");
        let items = completions(FileKind::Settings, &low, 0);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        for expected in ["lint", "validate", "format", "docs", "codegen"] {
            assert!(
                labels.contains(&expected),
                "{expected} missing from {labels:?}"
            );
        }
    }

    #[test]
    fn completion_offers_the_permitted_values_of_an_enum() {
        let low = doc(".suspect.yaml", "lint:\n  min_severity: warn\n");
        let items = completions(
            FileKind::Settings,
            &low,
            offset("lint:\n  min_severity: warn", "warn"),
        );
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["error", "warning", "info", "hint"],
            "{labels:?}"
        );
    }

    #[test]
    fn completion_offers_target_keys_inside_a_codegen_entry() {
        let text = "{\"codegen\": [{\"name\": \"go\", \"profilee\": \"go-http\"}]}";
        let low = doc("suspect.project.json", text);
        let found = problems(FileKind::Project, &low);
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].suggestion.as_deref(), Some("profile"));

        let items = completions(FileKind::Project, &low, offset(text, "go-http"));
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"package_name"), "{labels:?}");
        assert!(!labels.contains(&"name"), "already present: {labels:?}");
    }

    #[test]
    fn a_profile_name_is_not_reported_as_unknown() {
        let text = "{\"publish\": {\"profiles\": {\"public\": [\"a.overlay.yaml\"]}}}";
        let low = doc("suspect.project.json", text);
        assert!(problems(FileKind::Project, &low).is_empty());
    }
}
