//! Diagnostic production: syntax errors, semantic validation, linting, and
//! Arazzo checks, all mapped into tower-lsp `Diagnostic`s.

use std::sync::Arc;
use suspect_arazzo::{ArazzoDoc, validate_arazzo};
use suspect_low::{LowDoc, SpecFamily};
use suspect_oas::Session;
use suspect_ref::Workspace;
use suspect_source::LineIndex;
use suspect_validate::{self, validate_entry};
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString};

use crate::state::lsp_range;

/// `source` stamped on every diagnostic we publish.
pub const SOURCE: &str = "suspect";

/// `Diagnostic::source` for spectral-style lint findings; lets config gate the
/// recommended ruleset independently of syntax/validate batteries.
pub const SOURCE_LINT: &str = "suspect-lint";

/// Maps `suspect_validate::Severity` into LSP severity.
#[must_use]
pub fn map_validate_severity(s: suspect_validate::Severity) -> DiagnosticSeverity {
    match s {
        suspect_validate::Severity::Error => DiagnosticSeverity::ERROR,
        suspect_validate::Severity::Warning => DiagnosticSeverity::WARNING,
        suspect_validate::Severity::Info => DiagnosticSeverity::INFORMATION,
    }
}

/// Maps `suspect_lint::Severity` into LSP severity; `Off` findings are
/// dropped by the caller.
#[must_use]
pub fn map_lint_severity(s: suspect_lint::Severity) -> Option<DiagnosticSeverity> {
    match s {
        suspect_lint::Severity::Error => Some(DiagnosticSeverity::ERROR),
        suspect_lint::Severity::Warn => Some(DiagnosticSeverity::WARNING),
        suspect_lint::Severity::Info => Some(DiagnosticSeverity::INFORMATION),
        suspect_lint::Severity::Hint => Some(DiagnosticSeverity::HINT),
        suspect_lint::Severity::Off => None,
    }
}

/// Builds one [`Diagnostic`] with our [`SOURCE`], the given byte range
/// mapped through [`lsp_range`], severity, string code, and message.
fn make(
    bytes: &[u8],
    li: &LineIndex,
    range: std::ops::Range<usize>,
    severity: DiagnosticSeverity,
    code: &str,
    message: String,
) -> Diagnostic {
    Diagnostic {
        range: lsp_range(bytes, li, range),
        severity: Some(severity),
        code: Some(NumberOrString::String(code.to_owned())),
        code_description: None,
        source: Some(SOURCE.to_owned()),
        message,
        related_information: None,
        tags: None,
        data: None,
    }
}

/// Tree-sitter recovery errors from the parse.
#[must_use]
pub fn syntax_diagnostics(low: &LowDoc) -> Vec<Diagnostic> {
    let bytes = low.inner().bytes();
    let li = low.inner().line_index();
    low.syntax_errors()
        .iter()
        .map(|e| {
            make(
                bytes,
                li,
                e.range.clone(),
                DiagnosticSeverity::ERROR,
                "syntax",
                e.message.clone(),
            )
        })
        .collect()
}

/// Semantic validation for OpenAPI 3.x documents; other families yield
/// nothing. Workspace-load or model errors degrade to no diagnostics.
#[must_use]
pub fn validate_diagnostics(ws: &Arc<Workspace>, low: &LowDoc) -> Vec<Diagnostic> {
    if !matches!(
        low.sniff_family(),
        SpecFamily::Oas30 | SpecFamily::Oas31 | SpecFamily::Oas32
    ) {
        return Vec::new();
    }
    let session = Session::new(Arc::clone(ws));
    let Ok(diags) = validate_entry(&session, low.uri().as_str()) else {
        return Vec::new();
    };
    let bytes = low.inner().bytes();
    let li = low.inner().line_index();
    diags
        .into_iter()
        .map(|d| {
            make(
                bytes,
                li,
                d.range,
                map_validate_severity(d.severity),
                d.code,
                d.message,
            )
        })
        .collect()
}

/// Spectral-default lint findings; every family is linted.
#[must_use]
pub fn lint_diagnostics(low: &LowDoc) -> Vec<Diagnostic> {
    let linter = suspect_lint::Linter::spectral_default();
    let bytes = low.inner().bytes();
    let li = low.inner().line_index();
    linter
        .run(low)
        .into_iter()
        .filter_map(|f| {
            let severity = map_lint_severity(f.severity)?;
            let mut diag = make(bytes, li, f.range, severity, &f.code, f.message);
            diag.source = Some(SOURCE_LINT.to_owned());
            Some(diag)
        })
        .collect()
}

/// Arazzo 1.0 structural + cross-reference validation.
#[must_use]
pub fn arazzo_diagnostics(low: &LowDoc) -> Vec<Diagnostic> {
    if low.sniff_family() != SpecFamily::Arazzo10 {
        return Vec::new();
    }
    let doc = ArazzoDoc::new(low);
    let bytes = low.inner().bytes();
    let li = low.inner().line_index();
    validate_arazzo(&doc)
        .into_iter()
        .map(|d| {
            make(
                bytes,
                li,
                d.range,
                DiagnosticSeverity::ERROR,
                d.code,
                d.message,
            )
        })
        .collect()
}

/// Full battery for one document: syntax, semantic validation (OAS 3.x
/// only), lint (all families), and Arazzo checks.
#[must_use]
pub fn compute_diagnostics(
    ws: Option<&Arc<Workspace>>,
    low: &LowDoc,
    cfg: &crate::config_files::SuspectConfig,
) -> Vec<Diagnostic> {
    crate::config_files::apply_config(compute_diagnostics_raw(ws, low), cfg)
}

/// Unfiltered battery; [`compute_diagnostics`] applies user config on top.
#[must_use]
pub fn compute_diagnostics_raw(ws: Option<&Arc<Workspace>>, low: &LowDoc) -> Vec<Diagnostic> {
    let mut out = syntax_diagnostics(low);
    if let Some(ws) = ws {
        out.extend(validate_diagnostics(ws, low));
    }
    out.extend(lint_diagnostics(low));
    out.extend(arazzo_diagnostics(low));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use suspect_ref::WorkspaceBuilder;

    fn low_at(dir: &std::path::Path, name: &str, text: &str) -> LowDoc {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        let uri = suspect_source::Uri::from_path(&path).unwrap();
        LowDoc::parse(
            uri,
            suspect_source::Source::from_vec(text.as_bytes().to_vec()),
        )
    }

    #[test]
    fn syntax_errors_map_to_utf16_ranges() {
        // Unclosed quote: a YAML grammar error at a known position.
        let dir = std::env::temp_dir().join("suspect-lsp-diag-syntax");
        std::fs::create_dir_all(&dir).unwrap();
        let low = low_at(&dir, "bad.yaml", "a: \"unclosed\n");
        let diags = syntax_diagnostics(&low);
        assert!(!diags.is_empty());
        let d = &diags[0];
        assert_eq!(d.source.as_deref(), Some(SOURCE));
        assert_eq!(d.severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(d.code, Some(NumberOrString::String("syntax".to_owned())));
        assert_eq!((d.range.start.line, d.range.start.character), (0, 0));
        assert!(d.range.end.character >= 11, "end at the error site: {d:?}");
    }

    #[test]
    fn severity_mappings() {
        use suspect_lint::Severity as L;
        use suspect_validate::Severity as V;
        assert_eq!(map_validate_severity(V::Error), DiagnosticSeverity::ERROR);
        assert_eq!(
            map_validate_severity(V::Warning),
            DiagnosticSeverity::WARNING
        );
        assert_eq!(
            map_validate_severity(V::Info),
            DiagnosticSeverity::INFORMATION
        );
        assert_eq!(map_lint_severity(L::Error), Some(DiagnosticSeverity::ERROR));
        assert_eq!(
            map_lint_severity(L::Warn),
            Some(DiagnosticSeverity::WARNING)
        );
        assert_eq!(
            map_lint_severity(L::Info),
            Some(DiagnosticSeverity::INFORMATION)
        );
        assert_eq!(map_lint_severity(L::Hint), Some(DiagnosticSeverity::HINT));
        assert_eq!(map_lint_severity(L::Off), None);
    }

    #[test]
    fn oas_docs_get_validation_and_lint() {
        let dir = std::env::temp_dir().join("suspect-lsp-diag-oas");
        std::fs::create_dir_all(&dir).unwrap();
        let text = "openapi: 3.1.0\ninfo:\n  title: T\n  version: \"1\"\npaths: {}\n";
        let low = low_at(&dir, "api.yaml", text);
        let ws = WorkspaceBuilder::new().root(&dir).build().unwrap();
        ws.load_all("api.yaml").unwrap();
        let ws = Arc::new(ws);
        let diags = compute_diagnostics(Some(&ws), &low, &Default::default());
        // No syntax errors; everything else is well-formed.
        assert!(
            diags
                .iter()
                .all(|d| d.code != Some(NumberOrString::String("syntax".to_owned())))
        );
        assert!(
            diags
                .iter()
                .all(|d| matches!(d.source.as_deref(), Some(SOURCE) | Some(SOURCE_LINT)))
        );
        // Without a workspace only syntax + lint + arazzo run.
        let bare = compute_diagnostics(None, &low, &Default::default());
        assert!(bare.len() <= diags.len());
    }

    #[test]
    fn arazzo_duplicate_workflow_ids_flagged() {
        let dir = std::env::temp_dir().join("suspect-lsp-diag-arazzo");
        std::fs::create_dir_all(&dir).unwrap();
        let text = r#"
arazzo: 1.0.0
info:
  title: T
sourceDescriptions:
  - name: api
    url: openapi.yaml
workflows:
  - workflowId: w
    steps:
      - stepId: s1
  - workflowId: w
    steps:
      - stepId: s2
"#;
        let low = low_at(&dir, "flow.yaml", text);
        let diags = arazzo_diagnostics(&low);
        assert!(
            diags.iter().any(|d| {
                matches!(&d.code, Some(NumberOrString::String(c)) if c.starts_with("arazzo-"))
            }),
            "{diags:?}"
        );
    }

    #[test]
    fn non_arazzo_family_skips_arazzo_checks() {
        let dir = std::env::temp_dir().join("suspect-lsp-diag-skip");
        std::fs::create_dir_all(&dir).unwrap();
        let low = low_at(&dir, "oas.yaml", "openapi: 3.1.0\ninfo: {}\n");
        assert!(arazzo_diagnostics(&low).is_empty());
    }
}
