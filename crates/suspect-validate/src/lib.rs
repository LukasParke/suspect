#![deny(missing_docs)]
//! suspect-validate: semantic validation for OpenAPI 3.x documents.
//!
//! Runs a fixed battery of checks over a [`suspect_oas::OpenApi`] typed view
//! and returns [`Diagnostic`]s with byte ranges into the source document.
//! Output is deterministic: sorted by `(doc, range, code)`.

mod checks;
mod diagnostic;

use suspect_oas::{ModelError, OpenApi, Session};

pub use diagnostic::{Diagnostic, Severity};
/// Validates one loaded OpenAPI document.
///
/// Runs the full check battery over `api` — operation, parameter, path,
/// response, security, server, tag, schema, example, webhook, and info
/// checks — and returns every finding. The result is sorted by
/// `(doc, range, code)` so output is stable across runs.
///
/// Schema and reference diagnostics use the offending node's source document,
/// including external documents. Other check groups currently anchor diagnostics
/// to `api`'s source document. Reference-shaped instance data is not traversed.
#[must_use]
pub fn validate_openapi(api: &OpenApi<'_>) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    checks::run_all(api, &mut out);
    finish(out)
}

/// Validates every loaded OpenAPI 3.x document in the session's workspace.
///
/// Documents that do not sniff as OpenAPI 3.x (shared schema files, other
/// families) are skipped silently.
///
/// # Errors
/// Propagates [`ModelError`] if any eligible document fails to load as an
/// OpenAPI model.
pub fn validate_workspace(session: &Session) -> Result<Vec<Diagnostic>, ModelError> {
    let ws = session.workspace();
    let mut out = Vec::new();
    for uri in ws.uris() {
        let family = ws.get(&uri).map(|h| h.doc().sniff_family());
        if matches!(
            family,
            Some(
                suspect_low::SpecFamily::Oas30
                    | suspect_low::SpecFamily::Oas31
                    | suspect_low::SpecFamily::Oas32
            )
        ) {
            let api = session.open(uri.as_str())?;
            out.extend(validate_openapi(&api));
        }
    }
    Ok(finish(out))
}

/// Validates one entry and its reachable contract positions. Reference-shaped
/// example/default data cannot cause document loading. Invalid contract
/// references produce located findings, including missing external documents.
///
/// # Errors
/// Propagates [`ModelError`] if `entry` fails to open as an OpenAPI model.
pub fn validate_entry(session: &Session, entry: &str) -> Result<Vec<Diagnostic>, ModelError> {
    let api = session.open(entry)?;
    Ok(validate_openapi(&api))
}

fn finish(mut out: Vec<Diagnostic>) -> Vec<Diagnostic> {
    out.sort_by(|a, b| {
        (&a.doc, a.range.start, a.range.end, a.code).cmp(&(
            &b.doc,
            b.range.start,
            b.range.end,
            b.code,
        ))
    });
    out
}
