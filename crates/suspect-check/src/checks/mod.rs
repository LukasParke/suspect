//! The builtin checks, one function per producer, each mapping its own
//! finding type onto [`suspect_diag::Finding`].

pub mod extensions;

use std::path::PathBuf;
use std::sync::Arc;

use suspect_diag::{CheckId, Finding, Severity};
use suspect_low::{LowDoc, SpecFamily};
use suspect_ref::Workspace;

use crate::Policy;

/// The code a YAML/JSON syntax error carries, in the CLI and the editor
/// (the editor's old `syntax` is an alias).
pub const SYNTAX_CODE: &str = "syntax-error";

/// What a check pass runs against.
#[derive(Clone, Copy)]
pub struct Context<'a> {
    /// The loaded workspace; OAS 3.x validation and load-profile workflow
    /// checks need it and are skipped without one.
    pub workspace: Option<&'a Arc<Workspace>>,
    /// The policy the findings will be filtered by; checks read only the
    /// inputs that change what they detect (ruleset, strict format,
    /// extension schemas, which checks run).
    pub policy: &'a Policy,
}

/// Every builtin check for one document, before policy. Sync and CPU-only,
/// so the editor can run it on a blocking thread.
///
/// Checks route by [`LowDoc::sniff_family`]. A document with syntax errors
/// still gets every check, as the editor has always run them. Only
/// findings of the checks in `policy.checks` are returned (validation
/// runs for `ref` alone, but its non-`$ref` findings are dropped).
///
/// Validation, lint and admission are independent and each costs a large
/// share of the pass on a big OpenAPI document, so they run side by side
/// on their own threads; findings keep the check order below either way.
#[must_use]
pub fn check_document(cx: &Context<'_>, doc: &LowDoc) -> Vec<Finding> {
    let policy = cx.policy;
    let on = |check: CheckId| policy.checks.contains(check);
    let validated = || match cx.workspace {
        Some(ws) if on(CheckId::Validate) || on(CheckId::Ref) => {
            validate(ws, doc, policy.strict_format)
        }
        _ => Vec::new(),
    };
    let linted = || {
        if on(CheckId::Lint) && policy.lint_enabled() {
            lint(doc, &policy.linter)
        } else {
            Vec::new()
        }
    };
    let admitted = || {
        #[cfg(feature = "sdk")]
        if policy.sdk
            && on(CheckId::Admission)
            && let Some(ws) = cx.workspace
        {
            return admission(ws, doc);
        }
        Vec::new()
    };
    // Validation and admission only do work on an OpenAPI 3.x document in
    // a workspace; elsewhere the threads would cost more than they save.
    let parallel = cx.workspace.is_some()
        && matches!(
            doc.sniff_family(),
            SpecFamily::Oas30 | SpecFamily::Oas31 | SpecFamily::Oas32
        );
    let (validated, linted, admitted) = if parallel {
        // Plain threads, not `rayon::join`: a pool worker waiting on a
        // join steals unrelated jobs (another document's pass), so one
        // request's latency would include another's lint.
        std::thread::scope(|scope| {
            let validated = scope.spawn(validated);
            let admitted = scope.spawn(admitted);
            let linted = linted();
            let join = |handle: std::thread::ScopedJoinHandle<'_, Vec<Finding>>| {
                handle
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            };
            (join(validated), linted, join(admitted))
        })
    } else {
        (validated(), linted(), admitted())
    };

    let mut out = Vec::new();
    if on(CheckId::Syntax) {
        out.extend(syntax(doc));
    }
    if on(CheckId::Config) {
        out.extend(config(doc));
    }
    out.extend(validated);
    out.extend(linted);
    if on(CheckId::Swagger) {
        out.extend(swagger(doc));
    }
    if on(CheckId::Arazzo) {
        out.extend(arazzo(doc));
    }
    if on(CheckId::Overlay) {
        out.extend(overlay(doc));
    }
    if on(CheckId::Markdown) && policy.lint_enabled() {
        out.extend(markdown(doc));
    }
    if on(CheckId::Extension) {
        let root = cx.workspace.and_then(|ws| ws.root_path());
        out.extend(extensions::check(doc, &policy.extensions, root));
    }
    if on(CheckId::LoadProfile) {
        out.extend(load_profile(cx.workspace, doc));
    }
    out.extend(admitted);
    // Validate reports `$ref` codes too; `--only` keeps the checks it names.
    out.retain(|f| on(f.check));
    out
}

/// SDK generation admission for an OpenAPI 3.x document: the refusals,
/// advice and contract limitations `suspect gen admission` reports. Findings
/// located in another document are reported at this document's level;
/// a document the contract compiler cannot open yields nothing.
#[cfg(feature = "sdk")]
#[must_use]
pub fn admission(ws: &Arc<Workspace>, low: &LowDoc) -> Vec<Finding> {
    use suspect_codegen::admission::FindingKind;
    if !matches!(
        low.sniff_family(),
        SpecFamily::Oas30 | SpecFamily::Oas31 | SpecFamily::Oas32
    ) {
        return Vec::new();
    }
    let uri = low.uri();
    let Ok(contract) = suspect_ir::contract::Contract::from_workspace(ws, uri) else {
        return Vec::new();
    };
    // Shared admission refuses once per operation; a refusal of a shared
    // value (the servers list, for instance) repeats verbatim at one spot.
    let mut seen = std::collections::HashSet::new();
    suspect_codegen::admission::review(&contract)
        .findings
        .into_iter()
        .filter(|f| seen.insert((f.code, f.at.clone(), f.message.clone())))
        .map(|f| {
            let severity = match f.kind {
                FindingKind::Refusal => Severity::Error,
                FindingKind::Contract => Severity::Warning,
                FindingKind::Advice => Severity::Info,
            };
            let here = f.source.as_ref().is_none_or(|s| s.document() == uri);
            let mut finding =
                Finding::new(CheckId::Admission, f.code, severity, uri.clone(), f.message);
            if here {
                finding.range = Some(f.at);
                finding.pointer = f.source.map(|s| s.pointer().to_string().into_boxed_str());
            }
            finding
        })
        .collect()
}

/// Tree-sitter recovery errors from the parse. A `.json` document that
/// parsed without them must also be strict JSON: format sniffing may have
/// read it as YAML, which accepts `"\x41"`, so its first strict-parse
/// error is reported.
#[must_use]
pub fn syntax(low: &LowDoc) -> Vec<Finding> {
    if low.syntax_errors().is_empty()
        && let Some(finding) = strict_json(low)
    {
        return vec![finding];
    }
    low.syntax_errors()
        .iter()
        .map(|e| {
            Finding::new(
                CheckId::Syntax,
                SYNTAX_CODE,
                Severity::Error,
                low.uri().clone(),
                e.message.clone(),
            )
            .with_range(e.range.clone())
        })
        .collect()
}

/// The strict JSON parse error of a `.json` document, if any.
fn strict_json(low: &LowDoc) -> Option<Finding> {
    let json = low
        .uri()
        .as_str()
        .rsplit('.')
        .next()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"));
    if !json {
        return None;
    }
    let bytes = low.inner().bytes();
    let error = serde_json::from_slice::<serde::de::IgnoredAny>(bytes).err()?;
    let line = u32::try_from(error.line().saturating_sub(1)).unwrap_or(u32::MAX);
    let offset = low
        .inner()
        .line_index()
        .line_range(bytes, line)
        .map_or(0, |range| range.start + error.column().saturating_sub(1))
        .min(bytes.len());
    Some(
        Finding::new(
            CheckId::Syntax,
            SYNTAX_CODE,
            Severity::Error,
            low.uri().clone(),
            error.to_string(),
        )
        .with_range(offset..offset),
    )
}

/// `.suspect.yaml` and `suspect.project.json` against their schema; other
/// documents yield nothing.
#[must_use]
pub fn config(low: &LowDoc) -> Vec<Finding> {
    suspect_config::schema::document_problems(low)
        .into_iter()
        .map(|problem| {
            let mut message = problem.message;
            if let Some(suggestion) = &problem.suggestion {
                message.push_str(&format!(" Did you mean `{suggestion}`?"));
            }
            Finding::new(
                CheckId::Config,
                problem.code,
                Severity::Warning,
                low.uri().clone(),
                message,
            )
            .with_range(problem.range)
        })
        .collect()
}

const fn validate_severity(s: suspect_validate::Severity) -> Severity {
    match s {
        suspect_validate::Severity::Error => Severity::Error,
        suspect_validate::Severity::Warning => Severity::Warning,
        suspect_validate::Severity::Info => Severity::Info,
    }
}

/// The check a validate code belongs to: `$ref` codes are [`CheckId::Ref`].
fn validate_check(code: &str) -> CheckId {
    crate::registry::builtin()
        .info(code)
        .map_or(CheckId::Validate, |info| info.check)
}

/// Semantic validation for OpenAPI 3.x documents (including `$ref`
/// checks); other families yield nothing, and so does a document the
/// typed model cannot open.
#[must_use]
pub fn validate(ws: &Arc<Workspace>, low: &LowDoc, strict_format: bool) -> Vec<Finding> {
    if !matches!(
        low.sniff_family(),
        SpecFamily::Oas30 | SpecFamily::Oas31 | SpecFamily::Oas32
    ) {
        return Vec::new();
    }
    let session = suspect_oas::Session::new(Arc::clone(ws));
    let Ok(api) = session.open(low.uri().as_str()) else {
        return Vec::new();
    };
    let options = suspect_validate::ValidationOptions {
        format_assertion: strict_format,
    };
    suspect_validate::validate_openapi_with(&api, &options)
        .into_iter()
        .map(|d| {
            Finding::new(
                validate_check(d.code),
                d.code,
                validate_severity(d.severity),
                d.doc,
                d.message,
            )
            .with_range(d.range)
        })
        .collect()
}

/// Spectral-style lint findings, at the severity the ruleset gives them.
#[must_use]
pub fn lint(low: &LowDoc, linter: &suspect_lint::Linter) -> Vec<Finding> {
    linter
        .run(low)
        .into_iter()
        .filter_map(|f| {
            let severity = f.severity.to_diag()?;
            let mut finding = Finding::new(
                CheckId::Lint,
                f.code.into_string(),
                severity,
                low.uri().clone(),
                f.message,
            )
            .with_range(f.range);
            finding.pointer = Some(f.path.to_string().into_boxed_str());
            Some(finding)
        })
        .collect()
}

/// Swagger 2.0 semantic validation over the parse tree.
#[must_use]
pub fn swagger(low: &LowDoc) -> Vec<Finding> {
    if low.sniff_family() != SpecFamily::Oas2 {
        return Vec::new();
    }
    suspect_validate::validate_swagger_low(low)
        .into_iter()
        .map(|d| {
            Finding::new(
                CheckId::Swagger,
                d.code,
                validate_severity(d.severity),
                low.uri().clone(),
                d.message,
            )
            .with_range(d.range)
        })
        .collect()
}

/// Arazzo 1.0 structural and cross-reference validation.
#[must_use]
pub fn arazzo(low: &LowDoc) -> Vec<Finding> {
    if low.sniff_family() != SpecFamily::Arazzo10 {
        return Vec::new();
    }
    let doc = suspect_arazzo::ArazzoDoc::new(low);
    suspect_arazzo::validate_arazzo(&doc)
        .into_iter()
        .map(|d| {
            Finding::new(
                CheckId::Arazzo,
                d.code,
                Severity::Error,
                low.uri().clone(),
                d.message,
            )
            .with_range(d.range)
        })
        .collect()
}

/// Overlay structure (`validate_overlay`). An overlay that does not parse
/// as one (no `actions`, a bad target) yields nothing here; lint reports
/// its shape.
#[must_use]
pub fn overlay(low: &LowDoc) -> Vec<Finding> {
    if low.sniff_family() != SpecFamily::Overlay10 {
        return Vec::new();
    }
    let Ok(doc) = suspect_overlay::OverlayDoc::parse(low) else {
        return Vec::new();
    };
    let registry = crate::registry::builtin();
    suspect_overlay::validate_overlay(&doc)
        .into_iter()
        .map(|d| {
            let severity = registry
                .info(d.code)
                .map_or(Severity::Warning, |r| r.default);
            Finding::new(
                CheckId::Overlay,
                d.code,
                severity,
                low.uri().clone(),
                d.message,
            )
            .with_range(d.range)
        })
        .collect()
}

/// markdownlint-compatible rules over `description`/`summary` values, at
/// hint severity.
#[must_use]
pub fn markdown(low: &LowDoc) -> Vec<Finding> {
    suspect_lint::markdown::fields(low)
        .into_iter()
        .flat_map(|field| suspect_lint::markdown::lint_findings(&field))
        .map(|f| {
            Finding::new(
                CheckId::Markdown,
                f.code,
                Severity::Hint,
                low.uri().clone(),
                f.message,
            )
            .with_range(f.range)
        })
        .collect()
}

/// Load profile (`suspectLoad: "1.0"`) structure; workflow ids are checked
/// against the Arazzo document when the workspace has it loaded.
#[must_use]
pub fn load_profile(ws: Option<&Arc<Workspace>>, low: &LowDoc) -> Vec<Finding> {
    if low.sniff_family() != SpecFamily::Load10 {
        return Vec::new();
    }
    let path = low
        .uri()
        .as_path()
        .unwrap_or_else(|| PathBuf::from("profile.load.yaml"));
    let (profile, mut diags) = suspect_test::load::read_profile_lenient(low, &path);
    if let (Some(ws), Some(arazzo)) = (ws, &profile.arazzo)
        && let Ok(uri) = suspect_source::Uri::from_path(arazzo)
        && let Some(handle) = ws.get(&uri)
        && let Ok(plan) = suspect_test::compile_plan(handle.doc(), ws)
    {
        diags.extend(suspect_test::load::profile::check_against_plan(
            &profile, &plan,
        ));
    }
    diags
        .into_iter()
        .map(|d| {
            let severity = if d.error {
                Severity::Error
            } else {
                Severity::Warning
            };
            Finding::new(
                CheckId::LoadProfile,
                d.code,
                severity,
                low.uri().clone(),
                d.message,
            )
            .with_range(d.range)
        })
        .collect()
}
