//! Response-description checks.

use suspect_oas::OpenApi;

use super::diag;
use crate::diagnostic::{Diagnostic, Severity};

/// `oas-response-missing-description` (Error): every response, resolved,
/// needs a description. Covers operation responses (paths + webhooks) and
/// `components/responses`.
pub(crate) fn check_response_descriptions(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    for op in api.operations() {
        let Some(responses) = op.responses() else {
            continue;
        };
        for (status, response) in responses.iter() {
            let r = response.resolved();
            if r.description().is_none() {
                out.push(diag(
                    api,
                    "oas-response-missing-description",
                    Severity::Error,
                    r.node().byte_range(),
                    format!(
                        "response `{status}` on {} operation is missing a description",
                        op.method()
                    ),
                ));
            }
        }
    }
    let Some(components) = api.components() else {
        return;
    };
    for (name, response) in components.responses() {
        let r = response.resolved();
        if r.description().is_none() {
            out.push(diag(
                api,
                "oas-response-missing-description",
                Severity::Error,
                r.node().byte_range(),
                format!("component response `{name}` is missing a description"),
            ));
        }
    }
}
