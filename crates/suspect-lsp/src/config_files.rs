//! Configuration resolution for documents (client settings layered over
//! the files `suspect_config` discovers) plus pure-computation helpers for workspace file operations
//! (`workspace/willRenameFiles`, `didRenameFiles`, `willDeleteFiles`,
//! `willCreateFiles`).
//!
//! Nothing here touches a [`tower_lsp::Client`]: every function takes the
//! ref [`Workspace`] (and, where relevant, the request's file list) and
//! returns plain LSP values, so the backend handlers in `lib.rs` stay thin
//! lock-and-delegate shells.
//!
//! `$ref` rewriting follows the same approach as [`crate::rename`]: each
//! edge carries the raw scalar text plus its byte range, so only the path
//! portion of the string is replaced while the fragment and surrounding
//! quoting are preserved verbatim.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use suspect_config::{Resolved, Settings};

use suspect_ref::{ParsedRef, Workspace};
use suspect_source::Uri;
use tower_lsp::lsp_types::{
    CreateFilesParams, DeleteFilesParams, Diagnostic, DiagnosticSeverity, FileRename,
    NumberOrString, TextEdit, Url, WorkspaceEdit,
};

use crate::state::lsp_range;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// The client's settings layers and every configuration resolved under
/// them, one per discovered settings file.
///
/// Each document resolves from its own directory with
/// [`suspect_config::resolve_found`], the rule `suspect lint` uses, so a
/// monorepo with per-project `.suspect.yaml` files gets the same answer in
/// the editor as on the command line. Results are cached per discovered
/// file; the cache is replaced whenever client settings or a watched
/// configuration file change.
#[derive(Debug, Default)]
pub struct ResolvedCache {
    /// initializationOptions, then the pulled `suspect` section (wins).
    editor: Vec<serde_json::Value>,
    /// Documents without a file path resolve from the workspace root.
    root: Option<PathBuf>,
    /// Directory -> what discovery found there.
    dirs: Mutex<HashMap<PathBuf, CacheKey>>,
    /// Discovery result -> the resolution and its rejected values.
    resolved: Mutex<HashMap<CacheKey, Arc<Resolved>>>,
}

/// What discovery found, plus the start directory when nothing configures
/// it (editor paths then resolve against that directory).
type CacheKey = (suspect_config::Found, Option<PathBuf>);

impl ResolvedCache {
    /// A cache for these client layers, lowest first.
    #[must_use]
    pub fn new(editor: Vec<serde_json::Value>, root: Option<PathBuf>) -> Self {
        Self {
            editor,
            root,
            ..Self::default()
        }
    }

    /// The configuration for a document: discovered from its directory,
    /// else from the workspace root.
    #[must_use]
    pub fn for_uri(&self, uri: &Uri) -> Arc<Resolved> {
        let dir = uri
            .as_path()
            .and_then(|p| p.parent().map(Path::to_path_buf));
        self.for_dir(dir.as_deref().or(self.root.as_deref()))
    }

    /// The configuration for the workspace root (workspace-wide limits).
    #[must_use]
    pub fn workspace(&self) -> Arc<Resolved> {
        self.for_dir(self.root.as_deref())
    }

    /// The configuration discovered from `dir`; built-in defaults and the
    /// client layers when there is no directory.
    #[must_use]
    pub fn for_dir(&self, dir: Option<&Path>) -> Arc<Resolved> {
        let start = dir.map_or_else(
            || std::env::temp_dir().join("suspect-untitled"),
            Path::to_path_buf,
        );
        let key = {
            let mut dirs = self.dirs.lock().unwrap_or_else(|e| e.into_inner());
            dirs.entry(start.clone())
                .or_insert_with(|| {
                    let found = match dir {
                        Some(dir) => suspect_config::discover::discover(dir),
                        None => suspect_config::Found::default(),
                    };
                    let unconfigured = found.config.is_none() && found.manifest.is_none();
                    (found, unconfigured.then(|| start.clone()))
                })
                .clone()
        };
        let mut resolved = self.resolved.lock().unwrap_or_else(|e| e.into_inner());
        Arc::clone(resolved.entry(key.clone()).or_insert_with(|| {
            let editor: Vec<&serde_json::Value> = self.editor.iter().collect();
            let inputs = suspect_config::Inputs {
                editor: &editor,
                ..suspect_config::Inputs::default()
            };
            let start = key.1.clone().unwrap_or(start);
            Arc::new(
                suspect_config::resolve_found(&start, key.0.clone(), inputs)
                    .unwrap_or_else(|error| *error.resolved),
            )
        }))
    }

    /// Every rejected value in the resolutions made so far, one line each,
    /// deduplicated.
    #[must_use]
    pub fn problems(&self) -> Vec<String> {
        self.notices(|r| &r.errors)
    }

    /// Every deprecation and unknown key in the resolutions made so far.
    #[must_use]
    pub fn warnings(&self) -> Vec<String> {
        self.notices(|r| &r.warnings)
    }

    fn notices(&self, pick: impl Fn(&Resolved) -> &Vec<suspect_config::Notice>) -> Vec<String> {
        let resolved = self.resolved.lock().unwrap_or_else(|e| e.into_inner());
        let mut out: Vec<String> = resolved
            .values()
            .flat_map(|r| pick(r).iter())
            .map(ToString::to_string)
            .collect();
        out.sort();
        out.dedup();
        out
    }
}

/// The severity floor a document's settings name, as an LSP severity.
#[must_use]
pub fn floor(settings: &Settings) -> DiagnosticSeverity {
    use suspect_config::SeverityName;
    match settings.lint.floor() {
        SeverityName::Error => DiagnosticSeverity::ERROR,
        SeverityName::Warning => DiagnosticSeverity::WARNING,
        SeverityName::Info => DiagnosticSeverity::INFORMATION,
        SeverityName::Hint | SeverityName::Off => DiagnosticSeverity::HINT,
    }
}

/// The configured vendor extensions over the built-in registry.
#[must_use]
pub fn extensions(settings: &Settings) -> crate::extensions_config::ExtensionConfig {
    settings
        .extensions
        .as_ref()
        .map(crate::extensions_config::ExtensionConfig::from_value)
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// File operations
// ---------------------------------------------------------------------------

/// What `workspace/didRenameFiles` must do to internal caches after a
/// rename happened outside a `willRenameFiles` round-trip.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RenamePlan {
    /// Loaded documents whose URIs vanished; drop them from the open-doc
    /// cache and invalidate the ref workspace so the next query reloads
    /// under the new paths.
    pub dropped_docs: Vec<Uri>,
    /// Number of `$ref` edges across the workspace that pointed at renamed
    /// files and now resolve against the new locations.
    pub retargeted_edges: usize,
}

/// Rewrites the path portion of every external `$ref` that resolves to a
/// renamed file, returning a [`WorkspaceEdit`] to apply before the rename
/// lands.
///
/// For each rename whose old URI is loaded in the workspace (and whose new
/// name keeps a `.yaml`/`.yml`/`.json` extension), every edge in *all*
/// loaded documents pointing at it gets its `$ref` path part replaced by
/// the lexical path from the referencing document's directory to the new
/// location. The `#/pointer` fragment and the leading `./` style of the
/// original string are preserved; non-file targets and local refs are
/// untouched. Returns `None` when nothing references any renamed file.
#[must_use]
pub fn will_rename_files(ws: &Workspace, renames: &[FileRename]) -> Option<WorkspaceEdit> {
    // Old canonical URI → new filesystem path, filtered to spec files the
    // workspace actually knows about.
    let mut moved: HashMap<String, PathBuf> = HashMap::new();
    for r in renames {
        let Ok(old) = Uri::parse(r.old_uri.as_str()) else {
            continue;
        };
        if ws.get(&old).is_none() || !is_spec_extension(&r.new_uri) {
            continue;
        }
        let new_path = Url::parse(&r.new_uri)
            .ok()
            .and_then(|u| u.to_file_path().ok())
            .map(|p| normalize(&p));
        let Some(new_path) = new_path else {
            continue;
        };
        moved.insert(old.as_str().to_owned(), new_path);
    }
    if moved.is_empty() {
        return None;
    }

    // Byte-range edits per containing document URI.
    let mut per_doc: HashMap<Uri, Vec<(std::ops::Range<usize>, String)>> = HashMap::new();
    for uri in ws.uris() {
        let Some(handle) = ws.get(&uri) else { continue };
        let doc_dir = uri
            .as_path()
            .and_then(|p| p.parent().map(Path::to_path_buf));
        let bytes = handle.doc().inner().bytes();
        let mut edits: Vec<(std::ops::Range<usize>, String)> = Vec::new();
        for edge in handle.edges().iter() {
            let ParsedRef::External { uri: target, .. } = &edge.parsed else {
                continue;
            };
            let Some(new_path) = moved.get(target.as_str()) else {
                continue;
            };
            // Preserve everything from the last `#` on (fragment incl. its
            // hash); replace only the document part before it.
            let (doc_part, tail) = match edge.raw.rfind('#') {
                Some(i) => (&edge.raw[..i], &edge.raw[i..]),
                None => (&edge.raw[..], ""),
            };
            // Only rewrite file-relative references; absolute/remote
            // document parts cannot be expressed as sibling paths.
            if doc_part.contains(':') {
                continue;
            }
            let Some(dir) = &doc_dir else { continue };
            let mut rel = relative_to(dir, new_path);
            if doc_part.starts_with("./") && !rel.starts_with('.') {
                rel.insert_str(0, "./");
            }
            // Quoted scalars span their delimiters in `edge.at`; keep them.
            let (open, close) = scalar_quotes(bytes, &edge.at);
            let body = format!("{}{}", encode_uri_path(&rel), tail);
            edits.push((edge.at.clone(), format!("{open}{body}{close}")));
        }
        if !edits.is_empty() {
            per_doc.insert(uri, edits);
        }
    }
    build_edit(ws, per_doc)
}

/// Describes the cache invalidation `workspace/didRenameFiles` must apply.
///
/// Pure computation: lists loaded documents that need dropping and counts
/// the edges retargeted by the rename, without mutating anything.
#[must_use]
pub fn did_rename_plan(ws: &Workspace, renames: &[FileRename]) -> RenamePlan {
    let mut plan = RenamePlan::default();
    let olds: Vec<Uri> = renames
        .iter()
        .filter_map(|r| Uri::parse(r.old_uri.as_str()).ok())
        .collect();
    plan.dropped_docs = olds
        .iter()
        .filter(|u| ws.get(u).is_some())
        .cloned()
        .collect();
    if plan.dropped_docs.is_empty() {
        return plan;
    }
    for uri in ws.uris() {
        let Some(handle) = ws.get(&uri) else { continue };
        for edge in handle.edges().iter() {
            if let ParsedRef::External { uri: target, .. } = &edge.parsed
                && olds.iter().any(|o| o == target)
            {
                plan.retargeted_edges += 1;
            }
        }
    }
    plan
}

/// Warns about deletions that would strand incoming `$ref` edges.
///
/// Returns one WARNING diagnostic per doomed file that still has incoming
/// edges (code `suspect-ref-delete-incoming`), naming the count and up to
/// three sample referencing documents so the client can ask for
/// confirmation. Deletion should surface broken refs as deliberate
/// follow-up work — never auto-strip them. Returns `None` when no deleted
/// file is referenced anywhere.
#[must_use]
pub fn will_delete_files(ws: &Workspace, deletions: &DeleteFilesParams) -> Option<Vec<Diagnostic>> {
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    for f in &deletions.files {
        let Ok(target) = Uri::parse(f.uri.as_str()) else {
            continue;
        };
        let mut sources: Vec<String> = Vec::new();
        let mut count = 0usize;
        for uri in ws.uris() {
            let Some(handle) = ws.get(&uri) else { continue };
            for edge in handle.edges().iter() {
                if let ParsedRef::External { uri: t, .. } = &edge.parsed
                    && t == &target
                {
                    count += 1;
                    let name = std::path::Path::new(uri.as_str())
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| uri.to_string());
                    if !sources.contains(&name) {
                        sources.push(name);
                    }
                }
            }
        }
        if count > 0 {
            sources.truncate(3);
            let name = Path::new(f.uri.as_str())
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| f.uri.clone());
            diagnostics.push(Diagnostic {
                range: tower_lsp::lsp_types::Range::default(),
                severity: Some(DiagnosticSeverity::WARNING),
                code: Some(NumberOrString::String(
                    "suspect-ref-delete-incoming".to_owned(),
                )),
                code_description: None,
                source: Some("suspect".to_owned()),
                message: format!(
                    "deleting `{name}` strands {count} incoming $ref edge(s); referenced from: {}",
                    sources.join(", "),
                ),
                related_information: None,
                tags: None,
                data: None,
            });
        }
    }
    (!diagnostics.is_empty()).then_some(diagnostics)
}

/// Scaffolds minimal component skeletons for newly created fragment files.
///
/// A creation qualifies only when its path contains a conventional
/// `schemas/` or `components/` segment (and the file is not already
/// loaded); those get a minimal OpenAPI component skeleton inserted at
/// offset 0. Anything else returns `None` — there is no safe universal
/// scaffold for an arbitrary new YAML.
#[must_use]
pub fn will_create_files(ws: &Workspace, creations: &CreateFilesParams) -> Option<WorkspaceEdit> {
    const SCAFFOLD: &str = "components:\n  schemas:\n    NewComponent:\n      type: object\n";
    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    for f in &creations.files {
        let Ok(uri) = Uri::parse(f.uri.as_str()) else {
            continue;
        };
        if ws.get(&uri).is_some() {
            continue;
        }
        let component_dir = Path::new(f.uri.as_str())
            .components()
            .any(|c| c.as_os_str() == "schemas" || c.as_os_str() == "components");
        if !component_dir {
            continue;
        }
        let Ok(url) = Url::parse(f.uri.as_str()) else {
            continue;
        };
        changes.insert(
            url,
            vec![TextEdit {
                range: tower_lsp::lsp_types::Range::default(),
                new_text: SCAFFOLD.to_owned(),
            }],
        );
    }
    if changes.is_empty() {
        return None;
    }
    Some(WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    })
}

// ---------------------------------------------------------------------------
// Helpers shared by the file-operation functions
// ---------------------------------------------------------------------------

/// True for extensions the ref workspace tracks.
fn is_spec_extension(path: &str) -> bool {
    matches!(
        Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("yaml" | "yml" | "json")
    )
}

/// Lexically normalizes `.` and `..` segments away.
fn normalize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::with_capacity(path.as_os_str().len());
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Returns the quote delimiters surrounding a `$ref` scalar, if any.
/// `suspect-low` reports quoted scalar byte ranges including their
/// delimiters, so rewrites must keep them.
fn scalar_quotes(bytes: &[u8], range: &std::ops::Range<usize>) -> (&'static str, &'static str) {
    let first = bytes.get(range.start).copied();
    let last = range.end.checked_sub(1).and_then(|i| bytes.get(i).copied());
    match (first, last) {
        (Some(b'\''), Some(b'\'')) => ("'", "'"),
        (Some(b'"'), Some(b'"')) => ("\"", "\""),
        _ => ("", ""),
    }
}

/// Lexical path from directory `from` to file `to`, using `/` separators
/// and `..` escapes. Both sides should already be normalized.
fn relative_to(from: &Path, to: &Path) -> String {
    let from_comps: Vec<_> = from.components().collect();
    let to_comps: Vec<_> = to.components().collect();
    let common = from_comps
        .iter()
        .zip(to_comps.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let mut out = PathBuf::new();
    for _ in common..from_comps.len() {
        out.push("..");
    }
    for c in &to_comps[common..] {
        out.push(c);
    }
    out.to_string_lossy().into_owned()
}

/// Percent-encodes a relative path for use inside a URI reference:
/// reserved and non-ASCII bytes become `%XX`; `/` separators survive.
fn encode_uri_path(rel: &str) -> String {
    let mut out = String::with_capacity(rel.len());
    for byte in rel.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Converts accumulated byte-range edits into a [`WorkspaceEdit`] keyed by
/// LSP URL, sorting each document's edits back-to-front as clients expect.
fn build_edit(
    ws: &Workspace,
    per_doc: HashMap<Uri, Vec<(std::ops::Range<usize>, String)>>,
) -> Option<WorkspaceEdit> {
    if per_doc.is_empty() {
        return None;
    }
    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    for (uri, mut edits) in per_doc {
        let Ok(url) = Url::parse(uri.as_str()) else {
            continue;
        };
        edits.sort_by_key(|e| std::cmp::Reverse(e.0.start));
        let Some(handle) = ws.get(&uri) else {
            continue;
        };
        let inner = handle.doc().inner();
        let lsp_edits = edits
            .into_iter()
            .map(|(r, text)| TextEdit {
                range: lsp_range(inner.bytes(), inner.line_index(), r),
                new_text: text,
            })
            .collect();
        changes.insert(url, lsp_edits);
    }
    if changes.is_empty() {
        return None;
    }
    Some(WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    })
}

/// Applies the settings to a computed diagnostic battery:
///
/// * `lint.recommended = false` with no `lint.ruleset` drops every
///   `suspect-lint` finding, as `suspect lint` runs no rules then.
/// * `lint.rules.<id> = severity` remaps the severity of diagnostics whose
///   code matches `<id>`; `off` drops them. Markdown battery codes share
///   the same namespace, so one per-rule entry covers both.
///
/// Lint findings from the spectral battery carry their committed policy
/// already (a per-rule entry wins over `lint.design`); this pass is
/// idempotent for them and is the only pass the markdown battery gets.
#[must_use]
pub fn apply_config(diags: Vec<Diagnostic>, settings: &Settings) -> Vec<Diagnostic> {
    let lint_on = settings.lint.recommended || settings.lint.ruleset.is_some();
    let rules = &settings.lint.rules;
    diags
        .into_iter()
        .filter(|d| lint_on || d.source.as_deref() != Some(crate::diagnostics::SOURCE_LINT))
        .filter_map(|mut d| {
            if let Some(NumberOrString::String(code)) = &d.code
                && let Some(name) = rules.get(code)
            {
                match suspect_lint::Severity::from_policy(name) {
                    Some(suspect_lint::Severity::Off) => return None,
                    Some(severity) => {
                        d.severity = match severity {
                            suspect_lint::Severity::Error => Some(DiagnosticSeverity::ERROR),
                            suspect_lint::Severity::Warn => Some(DiagnosticSeverity::WARNING),
                            suspect_lint::Severity::Info => Some(DiagnosticSeverity::INFORMATION),
                            suspect_lint::Severity::Hint | suspect_lint::Severity::Off => {
                                Some(DiagnosticSeverity::HINT)
                            }
                        };
                    }
                    None => {}
                }
            }
            Some(d)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::offset_of_utf16;
    use suspect_ref::WorkspaceBuilder;
    use tower_lsp::lsp_types::{FileCreate, FileDelete};
    const MAIN: &str = r#"
openapi: 3.1.0
info:
  title: T
  version: "1"
paths:
  /pets:
    get:
      responses:
        '200':
          content:
            application/json:
              schema:
                $ref: './schemas/user.yaml#/User'
  /names:
    get:
      responses:
        '200':
          content:
            application/json:
              schema:
                $ref: 'schemas/user.yaml#/Name'
  /flows:
    get:
      responses:
        '200':
          content:
            application/json:
              schema:
                $ref: './docs/api.yaml#/Flow'
"#;

    const USER: &str = r#"
components:
  schemas:
    User:
      type: object
    Name:
      type: string
"#;

    const API: &str = r#"
arazzo: 1.0.0
info:
  name: api
flows:
  - flowId: Flow
components:
  schemas:
    Wrapped:
      $ref: '../schemas/user.yaml#/User'
"#;

    fn write(dir: &std::path::Path, name: &str, text: &str) {
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn workspace(dir: &std::path::Path) -> Workspace {
        write(dir, "main.yaml", MAIN);
        write(dir, "schemas/user.yaml", USER);
        write(dir, "docs/api.yaml", API);
        let ws = WorkspaceBuilder::new().root(dir).build().unwrap();
        ws.load_all("main.yaml").unwrap();
        ws
    }

    fn rename(dir: &std::path::Path, old: &str, new: &str) -> FileRename {
        FileRename {
            old_uri: Url::from_file_path(dir.join(old)).unwrap().to_string(),
            new_uri: Url::from_file_path(dir.join(new)).unwrap().to_string(),
        }
    }

    fn edit_text(edit: &WorkspaceEdit, suffix: &str) -> Vec<String> {
        edit.changes
            .as_ref()
            .unwrap()
            .iter()
            .filter(|(u, _)| u.path().ends_with(suffix))
            .flat_map(|(_, es)| es.iter().map(|e| e.new_text.clone()))
            .collect()
    }

    // ----- configuration -----

    #[test]
    fn documents_resolve_from_their_own_directory() {
        let dir = tempfile();
        write(&dir, ".suspect.yaml", "lint:\n  min_severity: warning\n");
        write(&dir, "api/.suspect.yaml", "lint:\n  min_severity: error\n");
        let client = serde_json::json!({"suspect": {"ref": {"maxDocs": 7}}});
        let cache = ResolvedCache::new(vec![client], Some(dir.clone()));
        let uri =
            |name: &str| Uri::parse(Url::from_file_path(dir.join(name)).unwrap().as_str()).unwrap();
        let top = cache.for_uri(&uri("openapi.yaml"));
        let api = cache.for_uri(&uri("api/openapi.yaml"));
        assert_eq!(top.settings.lint.min_severity.as_deref(), Some("warning"));
        assert_eq!(api.settings.lint.min_severity.as_deref(), Some("error"));
        assert_eq!(floor(&api.settings), DiagnosticSeverity::ERROR);
        assert_eq!(
            api.settings.refs.max_docs, 7,
            "client layers apply to every document"
        );
        assert!(
            Arc::ptr_eq(&top, &cache.workspace()),
            "cached per settings file"
        );
    }

    #[test]
    fn rejected_values_are_problems_and_the_rest_applies() {
        let dir = tempfile();
        write(
            &dir,
            ".suspect.yaml",
            "lint:\n  min_severity: fatal\n  rules:\n    info-contact: loud\n    info-license: warn\n",
        );
        let cache = ResolvedCache::new(Vec::new(), Some(dir.clone()));
        let resolved = cache.workspace();
        assert_eq!(floor(&resolved.settings), DiagnosticSeverity::HINT);
        assert_eq!(resolved.settings.lint.rules["info-license"], "warning");
        let problems = cache.problems();
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(
            problems[0].contains("`lint.min_severity`: unrecognized severity `fatal`"),
            "{problems:?}"
        );
        assert!(
            problems[1].contains("`lint.rules.info-contact`: unrecognized severity `loud`"),
            "{problems:?}"
        );
    }

    #[test]
    fn recommended_off_without_a_ruleset_drops_lint_findings() {
        let lint = Diagnostic {
            source: Some(crate::diagnostics::SOURCE_LINT.to_owned()),
            code: Some(NumberOrString::String("info-contact".to_owned())),
            ..Diagnostic::default()
        };
        let mut settings = Settings::default();
        assert_eq!(apply_config(vec![lint.clone()], &settings).len(), 1);
        settings.lint.recommended = false;
        assert!(apply_config(vec![lint.clone()], &settings).is_empty());
        settings.lint.ruleset = Some(PathBuf::from("rules.yaml"));
        assert_eq!(apply_config(vec![lint.clone()], &settings).len(), 1);
        settings
            .lint
            .rules
            .insert("info-contact".to_owned(), "off".to_owned());
        assert!(apply_config(vec![lint], &settings).is_empty());
    }

    // ----- will_rename_files -----

    #[test]
    fn rename_rewrites_relative_refs_from_both_sides() {
        let dir = tempfile();
        let ws = workspace(&dir);
        let edit = will_rename_files(
            &ws,
            &[rename(&dir, "schemas/user.yaml", "schemas/account.yaml")],
        )
        .expect("edits expected");
        let main = edit_text(&edit, "main.yaml");
        assert!(
            main.iter()
                .any(|t| t.contains("./schemas/account.yaml#/User"))
        );
        assert!(
            main.iter()
                .any(|t| t.contains("schemas/account.yaml#/Name"))
        );
        // untouched third ref stays out of the edit set
        assert!(!main.iter().any(|t| t.contains("api.yaml")));
        let api = edit_text(&edit, "api.yaml");
        assert_eq!(api, vec!["'../schemas/account.yaml#/User'".to_owned()]);
    }

    #[test]
    fn rename_subdir_to_sibling_moves_up_a_level() {
        let dir = tempfile();
        let ws = workspace(&dir);
        let edit =
            will_rename_files(&ws, &[rename(&dir, "docs/api.yaml", "api.yaml")]).expect("edits");
        let main = edit_text(&edit, "main.yaml");
        assert_eq!(main, vec!["'./api.yaml#/Flow'".to_owned()]);
    }

    #[test]
    fn rename_unknown_or_unreferenced_file_yields_none() {
        let dir = tempfile();
        let ws = workspace(&dir);
        // not loaded in the workspace
        assert!(will_rename_files(&ws, &[rename(&dir, "missing/user.yaml", "x.yaml")]).is_none());
        // loaded but nothing points at it
        write(&dir, "lonely.yaml", "components: {}\n");
        let ws2 = {
            let ws = WorkspaceBuilder::new().root(&dir).build().unwrap();
            ws.load_all("main.yaml").unwrap();
            ws
        };
        assert!(will_rename_files(&ws2, &[rename(&dir, "lonely.yaml", "lonely2.yaml")]).is_none());
    }

    #[test]
    fn rename_edits_span_exactly_the_ref_scalar() {
        let dir = tempfile();
        let ws = workspace(&dir);
        let edit = will_rename_files(
            &ws,
            &[rename(&dir, "schemas/user.yaml", "schemas/account.yaml")],
        )
        .unwrap();
        let (_, es) = edit
            .changes
            .as_ref()
            .unwrap()
            .iter()
            .find(|(u, _)| u.path().ends_with("main.yaml"))
            .unwrap();
        let main_uri =
            Uri::parse(Url::from_file_path(dir.join("main.yaml")).unwrap().as_str()).unwrap();
        let inner = ws.get(&main_uri).unwrap().doc().inner();
        let bytes = inner.bytes();
        let li = inner.line_index();
        // Map each LSP range back to bytes and check it covers precisely
        // the old `$ref` scalar text (quotes excluded).
        let mut spans: Vec<(usize, &TextEdit)> = es
            .iter()
            .map(|e| {
                let start = offset_of_utf16(bytes, li, e.range.start.line, e.range.start.character)
                    .unwrap();
                (start, e)
            })
            .collect();
        spans.sort_by_key(|(o, _)| *o);
        let old0 = b"'./schemas/user.yaml#/User'";
        assert_eq!(&bytes[spans[0].0..spans[0].0 + old0.len()], old0);
        assert_eq!(spans[0].1.new_text, "'./schemas/account.yaml#/User'");
        // The replacement covers exactly the quoted scalar, nothing more.
        assert_eq!(
            spans[0].1.range.end.character - spans[0].1.range.start.character,
            (old0.len() as u32)
        );
        assert_eq!(spans.len(), 2);
        let old1 = b"'schemas/user.yaml#/Name'";
        assert_eq!(&bytes[spans[1].0..spans[1].0 + old1.len()], old1);
        assert_eq!(spans[1].1.new_text, "'schemas/account.yaml#/Name'");
    }

    // ----- did_rename_plan -----

    #[test]
    fn plan_counts_dropped_docs_and_retargets() {
        let dir = tempfile();
        let ws = workspace(&dir);
        let plan = did_rename_plan(
            &ws,
            &[rename(&dir, "schemas/user.yaml", "schemas/account.yaml")],
        );
        assert_eq!(plan.dropped_docs.len(), 1);
        assert_eq!(plan.retargeted_edges, 3); // 2 in main + 1 in docs/api
        let none = did_rename_plan(&ws, &[rename(&dir, "nope.yaml", "nada.yaml")]);
        assert_eq!(none, RenamePlan::default());
    }

    // ----- will_delete_files -----

    #[test]
    fn delete_reports_incoming_refs() {
        let dir = tempfile();
        let ws = workspace(&dir);
        let params = DeleteFilesParams {
            files: vec![FileDelete {
                uri: Url::from_file_path(dir.join("schemas/user.yaml"))
                    .unwrap()
                    .to_string(),
            }],
        };
        let diags = will_delete_files(&ws, &params).expect("warning expected");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(
            diags[0].code,
            Some(NumberOrString::String(
                "suspect-ref-delete-incoming".to_owned()
            ))
        );
        assert!(diags[0].message.contains("3 incoming"));
        assert!(diags[0].message.contains("main.yaml"));
    }

    #[test]
    fn delete_unreferenced_file_is_silent() {
        let dir = tempfile();
        let ws = workspace(&dir);
        let params = DeleteFilesParams {
            files: vec![
                FileDelete {
                    uri: Url::from_file_path(dir.join("schemas/user.yaml"))
                        .unwrap()
                        .to_string(),
                },
                FileDelete {
                    uri: Url::from_file_path(dir.join("nothing.yaml"))
                        .unwrap()
                        .to_string(),
                },
            ],
        };
        // user.yaml has incoming refs even though nothing.yaml has none.
        let diags = will_delete_files(&ws, &params).unwrap();
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("user.yaml"));

        let empty = DeleteFilesParams {
            files: vec![FileDelete {
                uri: Url::from_file_path(dir.join("nothing.yaml"))
                    .unwrap()
                    .to_string(),
            }],
        };
        assert!(will_delete_files(&ws, &empty).is_none());
    }

    // ----- will_create_files -----

    #[test]
    fn create_in_components_dir_gets_skeleton() {
        let dir = tempfile();
        let ws = workspace(&dir);
        let params = CreateFilesParams {
            files: vec![FileCreate {
                uri: Url::from_file_path(dir.join("components/schemas/refund.yaml"))
                    .unwrap()
                    .to_string(),
            }],
        };
        let edit = will_create_files(&ws, &params).expect("scaffold expected");
        let (url, es) = edit.changes.as_ref().unwrap().iter().next().unwrap();
        assert!(url.path().ends_with("refund.yaml"));
        assert_eq!(es.len(), 1);
        assert!(es[0].new_text.starts_with("components:\n  schemas:\n"));
        assert!(es[0].new_text.contains("NewComponent:"));
    }

    #[test]
    fn create_outside_component_dirs_is_skipped() {
        let dir = tempfile();
        let ws = workspace(&dir);
        let params = CreateFilesParams {
            files: vec![
                FileCreate {
                    uri: Url::from_file_path(dir.join("notes.yaml"))
                        .unwrap()
                        .to_string(),
                },
                FileCreate {
                    uri: Url::from_file_path(dir.join("docs/readme.md"))
                        .unwrap()
                        .to_string(),
                },
            ],
        };
        assert!(will_create_files(&ws, &params).is_none());
    }

    fn tempfile() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "suspect-cfgfiles-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
