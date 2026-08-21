//! Webhook-version and license checks.

use suspect_oas::{OasVersion, OpenApi};

use super::diag;
use crate::diagnostic::{Diagnostic, Severity};

/// `oas-webhook-unsupported-version` (Error): `webhooks` on a 3.0 document.
pub(crate) fn check_webhook_version(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    if api.version() != OasVersion::V30 {
        return;
    }
    if let Some(webhooks) = api.root().get("webhooks") {
        out.push(diag(
            api,
            "oas-webhook-unsupported-version",
            Severity::Error,
            webhooks.byte_range(),
            "`webhooks` is not supported in OpenAPI 3.0 documents",
        ));
    }
}
