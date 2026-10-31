//! Diagnostics: the `suspect_check` pass for one document, mapped into
//! tower-lsp `Diagnostic`s.
//!
//! Every path that shows findings (push, document and workspace pull, code
//! actions, the latency probe) goes through [`compute_diagnostics`], so
//! they cannot disagree about what a document's problems are.

use std::sync::Arc;
use suspect_check::{Context, Policy, Registry};
use suspect_diag::{CheckId, Finding, Severity};
use suspect_low::LowDoc;
use suspect_ref::Workspace;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString};

use crate::state::lsp_range;

/// `source` stamped on every diagnostic we publish.
pub const SOURCE: &str = "suspect";

/// `Diagnostic::source` for lint-style findings (spectral rules, markdown,
/// TypeScript rules); lets clients tell the rule batteries from the
/// syntax/validate checks.
pub const SOURCE_LINT: &str = "suspect-lint";

/// Maps a [`Severity`] into LSP severity.
#[must_use]
pub const fn lsp_severity(s: Severity) -> DiagnosticSeverity {
    match s {
        Severity::Error => DiagnosticSeverity::ERROR,
        Severity::Warning => DiagnosticSeverity::WARNING,
        Severity::Info => DiagnosticSeverity::INFORMATION,
        Severity::Hint => DiagnosticSeverity::HINT,
    }
}

/// One finding as an LSP diagnostic, its byte range mapped through
/// [`lsp_range`] against `low` (a document-level finding sits at the
/// start). A code with fix guidance in the registry carries it in `data`,
/// so clients can render or act on it without re-parsing the message.
#[must_use]
pub fn to_lsp(finding: &Finding, low: &LowDoc, registry: &Registry) -> Diagnostic {
    let bytes = low.inner().bytes();
    let li = low.inner().line_index();
    let source = match finding.check {
        CheckId::Lint | CheckId::Markdown | CheckId::Rules => SOURCE_LINT,
        _ => SOURCE,
    };
    let data = registry
        .info(&finding.code)
        .filter(|info| !info.how_to_fix.is_empty())
        .map(|info| {
            serde_json::json!({
                "suspect": finding.check.name(),
                "code": finding.code,
                "summary": info.summary,
                "how_to_fix": info.how_to_fix,
            })
        });
    Diagnostic {
        range: lsp_range(bytes, li, finding.range.clone().unwrap_or(0..0)),
        severity: Some(lsp_severity(finding.severity)),
        code: Some(NumberOrString::String(finding.code.to_string())),
        code_description: None,
        source: Some(source.to_owned()),
        message: finding.message.clone(),
        related_information: None,
        tags: None,
        data,
    }
}

/// Every check for one document under `policy`, after policy: what the
/// editor shows.
#[must_use]
pub fn findings(ws: Option<&Arc<Workspace>>, low: &LowDoc, policy: &Policy) -> Vec<Finding> {
    let cx = Context {
        workspace: ws,
        policy,
    };
    suspect_check::apply_policy(suspect_check::check_document(&cx, low), low, policy)
}

/// The diagnostics for one document under its settings: syntax, the
/// configuration-file schema, OAS 3.x and Swagger 2.0 validation, lint,
/// Arazzo, Overlay, markdown, vendor-extension schemas and load profiles,
/// with `lint.rules`, `lint.design`, `lint.recommended`, suppressions and
/// the severity floor applied. SDK findings follow `codegen:` in
/// `settings` alone; [`compute_diagnostics_for`] also reads the manifest.
#[must_use]
pub fn compute_diagnostics(
    ws: Option<&Arc<Workspace>>,
    low: &LowDoc,
    settings: &suspect_config::Settings,
) -> Vec<Diagnostic> {
    diagnostics_under(ws, low, &Policy::from_settings(settings), Vec::new())
}

/// The diagnostics for one document under its resolved configuration:
/// [`compute_diagnostics`], with SDK findings and admission on when the
/// configuration or the project manifest configures codegen.
#[must_use]
pub fn compute_diagnostics_for(
    ws: Option<&Arc<Workspace>>,
    low: &LowDoc,
    resolved: &suspect_config::Resolved,
) -> Vec<Diagnostic> {
    diagnostics_under(ws, low, &Policy::from_resolved(resolved), Vec::new())
}

/// [`compute_diagnostics_for`] with `extra` findings (the TS rules' for
/// this run) merged before policy. `registry` replaces the builtin one
/// when the running rules extend it with their ids.
#[must_use]
pub fn compute_diagnostics_with(
    ws: Option<&Arc<Workspace>>,
    low: &LowDoc,
    resolved: &suspect_config::Resolved,
    extra: Vec<Finding>,
    registry: Option<Arc<Registry>>,
) -> Vec<Diagnostic> {
    let mut policy = Policy::from_resolved(resolved);
    if let Some(registry) = registry {
        policy.registry = registry;
    }
    diagnostics_under(ws, low, &policy, extra)
}

/// Every check for one document plus `extra` findings, under `policy`,
/// as LSP diagnostics.
#[must_use]
pub fn diagnostics_under(
    ws: Option<&Arc<Workspace>>,
    low: &LowDoc,
    policy: &Policy,
    extra: Vec<Finding>,
) -> Vec<Diagnostic> {
    let cx = Context {
        workspace: ws,
        policy,
    };
    let mut raw = suspect_check::check_document(&cx, low);
    raw.extend(extra);
    suspect_check::apply_policy(raw, low, policy)
        .iter()
        .map(|f| to_lsp(f, low, &policy.registry))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn low(name: &str, text: &str) -> LowDoc {
        LowDoc::parse(
            suspect_source::Uri::parse(&format!("file:///w/{name}")).unwrap(),
            suspect_source::Source::from_vec(text.as_bytes().to_vec()),
        )
    }

    #[test]
    fn syntax_errors_map_to_utf16_ranges() {
        // Unclosed quote: a YAML grammar error at a known position.
        let low = low("bad.yaml", "a: \"unclosed\n");
        let diags = compute_diagnostics(None, &low, &Default::default());
        let d = diags
            .iter()
            .find(|d| d.code == Some(NumberOrString::String("syntax-error".to_owned())))
            .expect("a syntax diagnostic");
        assert_eq!(d.source.as_deref(), Some(SOURCE));
        assert_eq!(d.severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!((d.range.start.line, d.range.start.character), (0, 0));
        assert!(d.range.end.character >= 11, "end at the error site: {d:?}");
    }

    #[test]
    fn severity_mapping() {
        assert_eq!(lsp_severity(Severity::Error), DiagnosticSeverity::ERROR);
        assert_eq!(lsp_severity(Severity::Warning), DiagnosticSeverity::WARNING);
        assert_eq!(
            lsp_severity(Severity::Info),
            DiagnosticSeverity::INFORMATION
        );
        assert_eq!(lsp_severity(Severity::Hint), DiagnosticSeverity::HINT);
    }

    #[test]
    fn lint_and_markdown_findings_carry_the_lint_source() {
        let text = "openapi: 3.1.0\ninfo:\n  title: T\n  version: '1'\n  description: See https://example.com\npaths: {}\n";
        let diags = compute_diagnostics(None, &low("api.yaml", text), &Default::default());
        let source_of = |code: &str| {
            diags
                .iter()
                .find(|d| d.code == Some(NumberOrString::String(code.to_owned())))
                .and_then(|d| d.source.clone())
        };
        assert_eq!(source_of("info-contact").as_deref(), Some(SOURCE_LINT));
        assert_eq!(source_of("md-bare-url").as_deref(), Some(SOURCE_LINT));
    }

    #[test]
    fn the_floor_and_overrides_apply() {
        let text = "openapi: 3.1.0\ninfo:\n  title: T\n  version: '1'\npaths: {}\n";
        let low = low("api.yaml", text);
        let mut settings = suspect_config::Settings::default();
        settings.lint.min_severity = Some("warning".to_owned());
        settings
            .lint
            .rules
            .insert("info-license".to_owned(), "off".to_owned());
        let diags = compute_diagnostics(None, &low, &settings);
        assert!(!diags.is_empty());
        for d in &diags {
            assert!(
                d.severity <= Some(DiagnosticSeverity::WARNING),
                "below the floor: {d:?}"
            );
            assert_ne!(
                d.code,
                Some(NumberOrString::String("info-license".to_owned()))
            );
        }
    }

    #[test]
    fn admission_refusals_show_when_codegen_is_configured() {
        let dir = tempfile::tempdir().unwrap();
        let text = "openapi: 3.1.0\ninfo:\n  title: T\n  version: '1'\npaths:\n  /a:\n    get:\n      responses:\n        '200':\n          description: ok\n";
        let path = dir.path().join("api.yaml");
        std::fs::write(&path, text).unwrap();
        let ws = suspect_ref::WorkspaceBuilder::new()
            .root(dir.path())
            .build()
            .unwrap();
        ws.load_all("api.yaml").unwrap();
        let ws = Arc::new(ws);
        let low = LowDoc::parse(
            suspect_source::Uri::from_path(&path).unwrap(),
            suspect_source::Source::from_vec(text.as_bytes().to_vec()),
        );
        let codes = |dir: &std::path::Path| -> Vec<String> {
            let resolved = suspect_config::resolve(dir).unwrap();
            compute_diagnostics_for(Some(&ws), &low, &resolved)
                .into_iter()
                .filter_map(|d| match d.code {
                    Some(NumberOrString::String(code)) => Some(code),
                    _ => None,
                })
                .collect()
        };
        let without = codes(dir.path());
        assert!(!without.iter().any(|c| c == "http-operation-id"));
        assert!(!without.iter().any(|c| c == "sdk-operation-missing-id"));
        std::fs::write(
            dir.path().join(".suspect.yaml"),
            "codegen:\n  profile: typescript-http\n",
        )
        .unwrap();
        let with = codes(dir.path());
        assert!(with.iter().any(|c| c == "http-operation-id"), "{with:?}");
        assert!(with.iter().any(|c| c == "sdk-operation-missing-id"));
    }

    #[test]
    fn suppressions_apply_in_the_editor() {
        let text = "openapi: 3.1.0\ninfo:\n  title: T\n  version: '1'\npaths: {}\n";
        let has = |text: &str| {
            compute_diagnostics(None, &low("api.yaml", text), &Default::default())
                .iter()
                .any(|d| d.code == Some(NumberOrString::String("info-contact".to_owned())))
        };
        assert!(has(text));
        assert!(!has(&format!(
            "# suspect-disable-file info-contact\n{text}"
        )));
        assert!(!has(&text.replace(
            "info:\n",
            "# suspect-disable-next-line info-contact\ninfo:\n"
        )));
    }

    #[test]
    fn fix_guidance_rides_in_data() {
        let finding = Finding::new(
            CheckId::Validate,
            "oas-duplicate-operation-id",
            Severity::Error,
            suspect_source::Uri::parse("file:///w/api.yaml").unwrap(),
            "dup",
        );
        let low = low("api.yaml", "openapi: 3.1.0\n");
        let d = to_lsp(&finding, &low, suspect_check::registry::builtin());
        let data = d.data.expect("guidance");
        assert_eq!(data["suspect"], "validate");
        assert!(!data["how_to_fix"].as_str().unwrap().is_empty());
    }
}
