//! Configuration plumbing (`workspace/configuration`, initialization
//! options) plus pure-computation helpers for workspace file operations
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

use std::sync::LazyLock;

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

/// Lint section: ruleset selection and per-rule severity overrides.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LintCfg {
    /// Rule id → severity name (`error`, `warn`, `info`, `off`) overriding
    /// the severity a rule would otherwise produce.
    pub rules: HashMap<String, String>,
    /// Whether the recommended (spectral-default) ruleset is enabled.
    /// `None` means "not configured"; [`SuspectConfig::lint_recommended`]
    /// applies the default.
    pub recommended: Option<bool>,
}

/// Ref-workspace section: loading limits mapped onto `WorkspaceBuilder`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RefCfg {
    /// Maximum number of documents the ref workspace loads before giving
    /// up (`suspect.ref.maxDocs`). `None` means unconfigured.
    pub max_docs: Option<usize>,
}

/// Inlay-hint section: which hints are shown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InlayCfg {
    /// Whether `$ref` target inlay hints are shown
    /// (`suspect.inlayHints.refTargets`). `None` means unconfigured.
    pub ref_targets: Option<bool>,
}

/// Formatting section: output style knobs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FmtCfg {
    /// Indent width in spaces used by canonical formatting
    /// (`suspect.formatting.indent`). `None` means unconfigured.
    pub indent: Option<u8>,
}

/// The `suspect.*` configuration tree as sent by clients and accepted via
/// initialization options.
///
/// Parsing is manual over `serde_json::Value` rather than derived so the
/// crate needs no direct `serde` dependency; unknown keys and sections are
/// ignored, making forward-compatible with future fields.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SuspectConfig {
    /// Lint configuration (`suspect.lint`).
    pub lint: Option<LintCfg>,
    /// Ref-workspace configuration (`suspect.ref`, also accepted as
    /// `suspect.refs`).
    pub refs: Option<RefCfg>,
    /// Inlay-hint configuration (`suspect.inlayHints`).
    pub inlay_hints: Option<InlayCfg>,
    /// Formatting configuration (`suspect.formatting`).
    pub formatting: Option<FmtCfg>,
}

impl SuspectConfig {
    /// Effective maximum number of loaded documents (default 500).
    #[must_use]
    pub fn max_docs(&self) -> usize {
        self.refs.and_then(|r| r.max_docs).unwrap_or(500)
    }

    /// Effective `$ref`-target inlay hint toggle (default on).
    #[must_use]
    pub fn inlay_ref_targets(&self) -> bool {
        self.inlay_hints.and_then(|i| i.ref_targets).unwrap_or(true)
    }

    /// Effective formatting indent width in spaces (default 2).
    #[must_use]
    pub fn format_indent(&self) -> u8 {
        self.formatting.and_then(|f| f.indent).unwrap_or(2)
    }

    /// Effective recommended-ruleset toggle (default on).
    #[must_use]
    pub fn lint_recommended(&self) -> bool {
        self.lint
            .as_ref()
            .and_then(|l| l.recommended)
            .unwrap_or(true)
    }

    /// Per-rule severity overrides; empty when unconfigured.
    #[must_use]
    pub fn lint_rules(&self) -> &HashMap<String, String> {
        static EMPTY: LazyLock<HashMap<String, String>> = LazyLock::new(HashMap::new);
        self.lint.as_ref().map(|l| &l.rules).unwrap_or(&EMPTY)
    }

    /// Field-wise merge of two partial configs: set fields of `overlay`
    /// win, unset fields keep `base`'s values. Sections absent from both
    /// stay absent.
    #[must_use]
    fn overlay(self, overlay: &SuspectConfig) -> SuspectConfig {
        let lint = match (self.lint, &overlay.lint) {
            (Some(mut base), Some(o)) => {
                base.rules = if o.rules.is_empty() {
                    base.rules
                } else {
                    o.rules.clone()
                };
                base.recommended = o.recommended.or(base.recommended);
                Some(base)
            }
            (base, Some(o)) => base.or_else(|| Some(o.clone())),
            (base, None) => base,
        };
        let refs = match (self.refs, overlay.refs) {
            (Some(base), Some(o)) => Some(RefCfg {
                max_docs: o.max_docs.or(base.max_docs),
            }),
            (base, Some(o)) => base.or(Some(o)),
            (base, None) => base,
        };
        let inlay_hints = match (self.inlay_hints, overlay.inlay_hints) {
            (Some(base), Some(o)) => Some(InlayCfg {
                ref_targets: o.ref_targets.or(base.ref_targets),
            }),
            (base, Some(o)) => base.or(Some(o)),
            (base, None) => base,
        };
        let formatting = match (self.formatting, overlay.formatting) {
            (Some(base), Some(o)) => Some(FmtCfg {
                indent: o.indent.or(base.indent),
            }),
            (base, Some(o)) => base.or(Some(o)),
            (base, None) => base,
        };
        SuspectConfig {
            lint,
            refs,
            inlay_hints,
            formatting,
        }
    }
}

/// Parses a `suspect` config object out of a raw JSON value.
///
/// Accepts either the section itself or a wrapper object containing it
/// under `suspect` (the shape `initialization_options` typically uses).
/// Returns `None` when neither shape is present or the value is not an
/// object.
#[must_use]
pub fn parse_config(value: &serde_json::Value) -> Option<SuspectConfig> {
    let obj = value.get("suspect").unwrap_or(value).as_object()?;
    let lint = obj.get("lint").map(|v| LintCfg {
        rules: v
            .get("rules")
            .and_then(|r| r.as_object())
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                    .collect()
            })
            .unwrap_or_default(),
        recommended: v.get("recommended").and_then(serde_json::Value::as_bool),
    });
    // The spec spells the section `ref`; accept `refs` as an alias.
    let refs_json = obj.get("ref").or_else(|| obj.get("refs"));
    let refs = refs_json.map(|v| RefCfg {
        max_docs: v
            .get("maxDocs")
            .or_else(|| v.get("max_docs"))
            .and_then(serde_json::Value::as_u64)
            .and_then(|n| usize::try_from(n).ok()),
    });
    let inlay_hints = obj.get("inlayHints").map(|v| InlayCfg {
        ref_targets: v
            .get("refTargets")
            .or_else(|| v.get("ref_targets"))
            .and_then(serde_json::Value::as_bool),
    });
    let formatting = obj.get("formatting").map(|v| FmtCfg {
        indent: v
            .get("indent")
            .and_then(serde_json::Value::as_u64)
            .and_then(|n| u8::try_from(n).ok()),
    });
    Some(SuspectConfig {
        lint,
        refs,
        inlay_hints,
        formatting,
    })
}

/// Merges the three configuration sources into one effective config.
///
/// Precedence (highest wins): the client-provided `client_section`
/// (from `workspace/configuration` / `didChangeConfiguration`), then
/// `initialization_options` parsed via [`parse_config`], then `base`.
/// Merging happens per leaf field, so a client that only sets
/// `lint.rules` keeps every other configured value intact.
#[must_use]
pub fn merge(
    initialization_options: Option<serde_json::Value>,
    client_section: Option<SuspectConfig>,
    base: SuspectConfig,
) -> SuspectConfig {
    let mut merged = base;
    if let Some(init) = initialization_options.as_ref().and_then(parse_config) {
        merged = merged.overlay(&init);
    }
    if let Some(client) = client_section.as_ref() {
        merged = merged.overlay(client);
    }
    merged
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

/// Applies [`SuspectConfig`] to a computed diagnostic battery:
///
/// * `lint.recommended = false` drops every `suspect-lint` finding.
/// * `lint.rules.<id> = "error"|"warning"|"information"|"hint"` remaps the
///   severity of diagnostics whose code matches `<id>`.
#[must_use]
pub fn apply_config(diags: Vec<Diagnostic>, cfg: &SuspectConfig) -> Vec<Diagnostic> {
    let lint_on = cfg.lint_recommended();
    let rules = cfg
        .lint
        .as_ref()
        .map(|l| l.rules.clone())
        .unwrap_or_default();
    diags
        .into_iter()
        .filter(|d| lint_on || d.source.as_deref() != Some(crate::diagnostics::SOURCE_LINT))
        .map(|mut d| {
            if let Some(NumberOrString::String(code)) = &d.code
                && let Some(sev) = rules.get(code)
            {
                d.severity = match sev.as_str() {
                    "error" => Some(DiagnosticSeverity::ERROR),
                    "warning" => Some(DiagnosticSeverity::WARNING),
                    "information" => Some(DiagnosticSeverity::INFORMATION),
                    "hint" => Some(DiagnosticSeverity::HINT),
                    _ => d.severity,
                };
            }
            d
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
    fn parse_reads_direct_and_wrapped_sections() {
        let direct = serde_json::json!({
            "lint": {"recommended": false, "rules": {"info-contact": "warn"}},
            "ref": {"maxDocs": 50},
            "inlayHints": {"refTargets": false},
            "formatting": {"indent": 4}
        });
        let cfg = parse_config(&direct).unwrap();
        assert_eq!(cfg.max_docs(), 50);
        assert!(!cfg.inlay_ref_targets());
        assert_eq!(cfg.format_indent(), 4);
        assert!(!cfg.lint_recommended());
        assert_eq!(cfg.lint_rules()["info-contact"], "warn");

        let wrapped = serde_json::json!({"suspect": {"refs": {"maxDocs": 7}}});
        assert_eq!(parse_config(&wrapped).unwrap().max_docs(), 7);

        assert!(parse_config(&serde_json::json!("nope")).is_none());
        assert!(parse_config(&serde_json::Value::Null).is_none());
    }

    #[test]
    fn merge_precedence_client_over_init_over_base() {
        let base = SuspectConfig {
            refs: Some(RefCfg {
                max_docs: Some(500),
            }),
            inlay_hints: Some(InlayCfg {
                ref_targets: Some(true),
            }),
            lint: Some(LintCfg {
                rules: HashMap::from([("old-rule".into(), "off".into())]),
                recommended: Some(true),
            }),
            formatting: Some(FmtCfg { indent: Some(2) }),
        };
        let init = serde_json::json!({
            "ref": {"maxDocs": 100},
            "inlayHints": {"refTargets": false}
        });
        let client = parse_config(&serde_json::json!({
            "lint": {"rules": {"new-rule": "warn"}}
        }))
        .unwrap();

        let merged = merge(Some(init), Some(client), base);
        // client wins over init and base
        assert_eq!(merged.lint_rules()["new-rule"], "warn");
        // init fills what client leaves unset
        assert_eq!(merged.max_docs(), 100);
        assert!(!merged.inlay_ref_targets());
        // base survives where upper layers are silent
        assert_eq!(merged.format_indent(), 2);
        assert!(merged.lint_recommended());
        // base rules kept when client adds others? No: leaf-level override
        assert_eq!(merged.lint_rules().get("old-rule"), None);
    }

    #[test]
    fn merge_with_nothing_set_keeps_base_defaults() {
        let merged = merge(None, None, SuspectConfig::default());
        assert_eq!(merged.max_docs(), 500);
        assert!(merged.inlay_ref_targets());
        assert_eq!(merged.format_indent(), 2);
        assert!(merged.lint_recommended());
        assert!(merged.lint_rules().is_empty());
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
