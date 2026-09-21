//! Operation-level checks: operationIds, responses, deprecation.

use rustc_hash::FxHashMap;
use suspect_oas::OpenApi;

use super::{diag, range_of};
use crate::diagnostic::{Diagnostic, Severity};

/// `oas-operation-missing-operationId` (Warning) and
/// `oas-duplicate-operation-id` (Error) across paths + webhooks.
pub(crate) fn check_operation_ids(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    let mut seen: FxHashMap<&str, std::ops::Range<usize>> = FxHashMap::default();
    for op in api.operations() {
        match op.operation_id() {
            None => {
                out.push(diag(
                    api,
                    "oas-operation-missing-operationId",
                    Severity::Warning,
                    op.node().byte_range(),
                    format!("{} operation is missing an operationId", op.method()),
                ));
            }
            Some(id) => {
                let range = range_of(op.node().get("operationId"), op.node());
                if seen.contains_key(id) {
                    out.push(diag(
                        api,
                        "oas-duplicate-operation-id",
                        Severity::Error,
                        range,
                        format!("duplicate operationId `{id}`"),
                    ));
                } else {
                    seen.insert(id, range);
                }
            }
        }
    }
}

/// `oas-operation-missing-responses` (Error).
pub(crate) fn check_missing_responses(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    for op in api.operations() {
        if op.responses().is_none() {
            out.push(diag(
                api,
                "oas-operation-missing-responses",
                Severity::Error,
                op.node().byte_range(),
                format!(
                    "{} operation is missing the required `responses` field",
                    op.method()
                ),
            ));
        }
    }
}

/// `oas-deprecated-operation` (Info).
pub(crate) fn check_deprecated(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    for op in api.operations() {
        if op.deprecated() {
            let id = op.operation_id().unwrap_or("<unnamed>");
            out.push(diag(
                api,
                "oas-deprecated-operation",
                Severity::Info,
                op.node().byte_range(),
                format!("operation `{id}` ({}) is deprecated", op.method()),
            ));
        }
    }
}

/// `oas-operation-no-error-response` (Info): the operation's `responses`
/// carry no `default` and no 4XX/5XX entry, so generated client error
/// handling has nothing to bind to.
pub(crate) fn check_no_error_response(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    for op in api.operations() {
        let Some(responses) = op.responses() else {
            continue; // missing `responses` is a separate, harder failure
        };
        let has_error = responses.iter().into_iter().any(|(key, _)| {
            key == "default"
                || key
                    .parse::<u16>()
                    .is_ok_and(|status| (400..=599).contains(&status))
        });
        if !has_error {
            let id = op.operation_id().unwrap_or("<unnamed>");
            out.push(diag(
                api,
                "oas-operation-no-error-response",
                Severity::Info,
                op.node().byte_range(),
                format!(
                    "operation `{id}` ({}) declares no `default` or 4XX/5XX response",
                    op.method()
                ),
            ));
        }
    }
}
