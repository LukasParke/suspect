//! Validation of suspect's own configuration documents against the
//! setting tables: `.suspect.yaml` ([`SETTINGS`]) and
//! `suspect.project.json` ([`MANIFEST`]).
//!
//! Everything keys off the document's file name, so a `.suspect.yaml`
//! belonging to some unrelated tool is never checked. The editor's hover
//! and completion for these files read the same lookups.

use suspect_low::{LowDoc, NodeRef, ValueKind};

use super::{Kind, MANIFEST, SETTINGS, Setting, TARGET_FIELDS, lookup_in};

/// The code of a schema problem (unknown key, wrong type, value outside an
/// enumeration, deprecated spelling).
pub const CODE: &str = "suspect.config";

/// The code of an ignored policy section in the manifest (option A).
pub const MANIFEST_POLICY_CODE: &str = "config-manifest-policy";

/// Which configuration file a document is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    /// `.suspect.yaml`
    Settings,
    /// `suspect.project.json`
    Project,
}

impl FileKind {
    /// The document's own name, for titles and messages.
    #[must_use]
    pub const fn display(self) -> &'static str {
        match self {
            Self::Settings => ".suspect.yaml",
            Self::Project => "suspect.project.json",
        }
    }

    /// The table that describes this file.
    #[must_use]
    pub const fn table(self) -> &'static [Setting] {
        match self {
            Self::Settings => SETTINGS,
            Self::Project => MANIFEST,
        }
    }
}

/// Recognises suspect's configuration files by file name. The settings
/// names are [`crate::CONFIG_NAMES`] itself, so a new name in the loader
/// cannot leave a file it reads unchecked.
#[must_use]
pub fn kind_of(file_name: &str) -> Option<FileKind> {
    if crate::CONFIG_NAMES.contains(&file_name) {
        return Some(FileKind::Settings);
    }
    match file_name {
        "suspect.project.json" => Some(FileKind::Project),
        _ => None,
    }
}

/// Which configuration file `low` is, by its path's file name.
#[must_use]
pub fn kind_for(low: &LowDoc) -> Option<FileKind> {
    let path = low.uri().as_path()?;
    kind_of(path.file_name()?.to_str()?)
}

/// The scope used for keys inside one `codegen` target, so they cannot
/// collide with the top-level `codegen` list.
pub const TARGET_SCOPE: &str = "codegen[]";

/// How many edits still count as a typo worth suggesting. Three covers
/// `min_severty` -> `min_severity` and `warn` -> `warning` without
/// offering a "did you mean" on every unknown key.
const SUGGEST_EDITS: usize = 3;

/// The row at a dot path (`codegen[].<key>` for target fields), by
/// canonical path or alias.
#[must_use]
pub fn field(kind: FileKind, path: &str) -> Option<&'static Setting> {
    if let Some(name) = path.strip_prefix(&format!("{TARGET_SCOPE}.")) {
        return TARGET_FIELDS.iter().find(|f| f.path == name);
    }
    lookup_in(kind.table(), path).map(|found| found.setting)
}

/// The rows valid directly inside `scope` (`""` is the document root).
#[must_use]
pub fn children(kind: FileKind, scope: &str) -> Vec<&'static Setting> {
    if scope.starts_with(TARGET_SCOPE) {
        return TARGET_FIELDS.iter().collect();
    }
    let prefix = if scope.is_empty() {
        String::new()
    } else {
        format!("{scope}.")
    };
    kind.table()
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
    children(kind, scope)
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

/// How a path reads in a message: `codegen[]` is noise in prose.
#[must_use]
pub fn display_path(path: &str) -> String {
    path.strip_prefix(&format!("{TARGET_SCOPE}."))
        .map_or_else(|| path.to_owned(), str::to_owned)
}

fn join(scope: &str, key: &str) -> String {
    if scope.is_empty() {
        key.to_owned()
    } else {
        format!("{scope}.{key}")
    }
}

/// A configuration problem, in byte offsets.
#[derive(Debug, Clone)]
pub struct Problem {
    /// [`CODE`] or [`MANIFEST_POLICY_CODE`].
    pub code: &'static str,
    /// Byte range to underline.
    pub range: std::ops::Range<usize>,
    /// Human-readable message.
    pub message: String,
    /// A closer key or value to suggest, when the problem is a typo.
    pub suggestion: Option<String>,
}

/// Checks a whole configuration document against its table.
///
/// Reports only what the schema knows to be wrong: an unknown key, a value
/// of the wrong kind, a value outside an enumeration, or a deprecated
/// spelling. It never reports a *missing* key: most keys are optional,
/// and a report about an absent key is noise in an editor.
#[must_use]
pub fn problems(kind: FileKind, low: &LowDoc) -> Vec<Problem> {
    let mut out = Vec::new();
    check_mapping(kind, &low.root(), "", &mut out);
    out
}

/// [`problems`] for a document recognised by [`kind_for`], plus a
/// [`MANIFEST_POLICY_CODE`] problem on each ignored policy section of a
/// manifest; other documents yield nothing.
#[must_use]
pub fn document_problems(low: &LowDoc) -> Vec<Problem> {
    let Some(kind) = kind_for(low) else {
        return Vec::new();
    };
    let mut out = problems(kind, low);
    if kind == FileKind::Project {
        out.extend(
            low.root()
                .entries()
                .into_iter()
                .filter(|e| matches!(e.key, "lint" | "validate" | "editor"))
                .map(|e| Problem {
                    code: MANIFEST_POLICY_CODE,
                    range: e.key_node.byte_range(),
                    message: crate::file::manifest_policy_warning(e.key),
                    suggestion: None,
                }),
        );
    }
    out
}

fn check_mapping(kind: FileKind, node: &NodeRef<'_>, scope: &str, out: &mut Vec<Problem>) {
    if node.kind() != ValueKind::Object {
        return;
    }
    let open_map = field(kind, scope).is_some_and(|f| matches!(f.kind, Kind::Map(_)));
    for entry in node.entries() {
        let path = join(scope, entry.key);
        let Some(row) = field(kind, &path) else {
            if open_map {
                // A user-chosen map accepts any key.
                continue;
            }
            out.push(Problem {
                code: CODE,
                range: entry.key_node.byte_range(),
                message: format!("`{}` is not a {} key.", entry.key, kind.display()),
                suggestion: closest(kind, scope, entry.key).map(str::to_owned),
            });
            continue;
        };
        let Some(value) = entry.value else { continue };
        if let Some(problem) = check_value(&path, row, &value, entry.key_node.byte_range()) {
            out.push(problem);
        }
        match row.kind {
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
    path: &str,
    row: &Setting,
    value: &NodeRef<'_>,
    key_range: std::ops::Range<usize>,
) -> Option<Problem> {
    let value_kind = value.kind();
    let shape_ok = match row.kind {
        Kind::Bool => matches!(value_kind, ValueKind::Bool | ValueKind::Null),
        Kind::Count | Kind::Port => matches!(value_kind, ValueKind::Int | ValueKind::Null),
        Kind::Str
        | Kind::Path
        | Kind::Url
        | Kind::Enum(_)
        | Kind::Severity
        | Kind::PolicySeverity => matches!(value_kind, ValueKind::Str | ValueKind::Null),
        Kind::List | Kind::EnumList(_) | Kind::Targets => {
            matches!(value_kind, ValueKind::Array | ValueKind::Null)
        }
        Kind::Section | Kind::Map(_) | Kind::Object => {
            matches!(value_kind, ValueKind::Object | ValueKind::Null)
        }
    };
    if !shape_ok {
        return Some(Problem {
            code: CODE,
            range: key_range,
            message: format!(
                "`{}` expects {} but found {found}.",
                display_path(path),
                row.kind.type_name(),
                found = describe(value_kind)
            ),
            suggestion: None,
        });
    }
    if row.values().is_empty() {
        return None;
    }
    // A non-string value already failed the shape check above, so there is
    // nothing to compare against an enumeration. A list of names checks
    // its first unknown item.
    let actual = match row.kind {
        Kind::EnumList(values) => value
            .items()
            .iter()
            .filter_map(NodeRef::as_str)
            .find(|item| !values.contains(item))?,
        _ => value.as_str()?,
    };
    if row.values().contains(&actual) {
        return None;
    }
    let suggestion = row
        .values()
        .iter()
        .copied()
        .min_by_key(|candidate| distance(candidate, actual))
        .filter(|candidate| distance(candidate, actual) <= SUGGEST_EDITS)
        .map(str::to_owned);
    Some(Problem {
        code: CODE,
        range: key_range,
        message: format!(
            "{actual} is not a valid {display} — expected one of: {allowed}.",
            display = display_path(path),
            // Plain list, no inner backticks: nesting them inside the
            // message's own quoting reads as one mangled word.
            allowed = row.values().join(", "),
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
        for name in crate::CONFIG_NAMES {
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
        assert_eq!(found[0].code, CODE);
    }

    #[test]
    fn an_unknown_name_in_a_name_list_is_reported() {
        let low = doc(
            ".suspect.yaml",
            "codegen:\n  compatibility_profiles: [colon-path-parameters-v1, colon-path-parameter-v1]\n",
        );
        let found = problems(FileKind::Settings, &low);
        assert_eq!(found.len(), 1, "{found:#?}");
        assert!(found[0].message.contains("colon-path-parameter-v1 is not"));
        assert_eq!(
            found[0].suggestion.as_deref(),
            Some("colon-path-parameters-v1")
        );
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
    fn a_misspelled_target_key_is_reported() {
        let text = "{\"codegen\": [{\"name\": \"go\", \"profilee\": \"go-http\"}]}";
        let low = doc("suspect.project.json", text);
        let found = problems(FileKind::Project, &low);
        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(found[0].suggestion.as_deref(), Some("profile"));
    }

    #[test]
    fn a_profile_name_is_not_reported_as_unknown() {
        let text = "{\"publish\": {\"profiles\": {\"public\": [\"a.overlay.yaml\"]}}}";
        let low = doc("suspect.project.json", text);
        assert!(problems(FileKind::Project, &low).is_empty());
    }

    #[test]
    fn manifest_policy_sections_are_deprecated_on_the_key() {
        let text = "{\"entry\": \"a.yaml\", \"lint\": {}, \"config\": \".suspect.yaml\"}";
        let low = doc("suspect.project.json", text);
        let found: Vec<_> = document_problems(&low)
            .into_iter()
            .filter(|p| p.code == MANIFEST_POLICY_CODE)
            .collect();
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0]
                .message
                .contains("`suspect config migrate` does both")
        );
        let settings = doc(".suspect.yaml", "lint: {}\n");
        assert!(document_problems(&settings).is_empty());
        assert!(document_problems(&doc("openapi.yaml", "lint: 1\n")).is_empty());
    }

    #[test]
    fn manifest_policy_mistakes_are_diagnosed() {
        // A bad severity value in the manifest is flagged exactly like one
        // in `.suspect.yaml` would be.
        let low = doc(
            "suspect.project.json",
            "{\"lint\": {\"min_severity\": \"shout\"}}",
        );
        let found = document_problems(&low);
        assert!(
            found.iter().any(|p| p.message.contains("shout")),
            "the bad value is named: {found:?}"
        );
    }
}
