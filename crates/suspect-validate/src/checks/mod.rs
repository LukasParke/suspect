//! Check battery: one module per check group, all driven by [`run_all`].
//!
//! Groups and their modules:
//!
//! - [`operations`] — operationIds, required `responses`, deprecation notices
//! - [`parameters`] — parameter `name`/`in` fields, path-param `required`,
//!   header duplication
//! - [`paths`] — path-key shape and path-template/parameter correspondence
//! - [`responses`] — response descriptions
//! - [`security`] — security requirements referencing declared schemes
//! - [`servers`] — server-URL template variables
//! - [`tags`] — operation tags declared in the root `tags` list
//! - [`schemas`] — schema `type` values and discriminators
//! - [`examples`] — media-type examples vs. schema type sets
//! - [`webhooks`] — `webhooks` availability per OpenAPI version
//! - [`info`] — license identification fields

mod examples;
mod info;
mod operations;
mod parameters;
mod paths;
mod responses;
mod schemas;
mod security;
mod servers;
mod tags;
mod webhooks;

use suspect_low::NodeRef;
use suspect_oas::OpenApi;

use crate::diagnostic::{Diagnostic, Severity};

/// Runs every check group against `api`, appending findings to `out` in
/// module order; the caller sorts the accumulated diagnostics.
pub(crate) fn run_all(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    operations::check_operation_ids(api, out);
    operations::check_missing_responses(api, out);
    operations::check_deprecated(api, out);
    parameters::check_parameter_fields(api, out);
    parameters::check_required_path_params(api, out);
    parameters::check_duplicate_header_params(api, out);
    paths::check_path_keys(api, out);
    paths::check_path_templates(api, out);
    responses::check_response_descriptions(api, out);
    security::check_security_schemes(api, out);
    servers::check_server_variables(api, out);
    tags::check_declared_tags(api, out);
    schemas::check_schemas(api, out);
    examples::check_example_types(api, out);
    webhooks::check_webhook_version(api, out);
    info::check_license(api, out);
}

/// Builds a diagnostic anchored to `api`'s document.
pub(crate) fn diag(
    api: &OpenApi<'_>,
    code: &'static str,
    severity: Severity,
    range: std::ops::Range<usize>,
    message: impl Into<String>,
) -> Diagnostic {
    Diagnostic {
        code,
        severity,
        message: message.into(),
        range,
        doc: api.root().syntax().doc().uri().clone(),
    }
}

/// `{name}` template variables in a path or server-URL string, in order,
/// duplicates removed.
pub(crate) fn template_vars(s: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    let mut rest = s;
    while let Some(open) = rest.find('{') {
        let Some(close_rel) = rest[open + 1..].find('}') else { break };
        let name = &rest[open + 1..open + 1 + close_rel];
        if !name.is_empty() && !out.contains(&name) {
            out.push(name);
        }
        rest = &rest[open + 1 + close_rel + 1..];
    }
    out
}

/// Byte range of `node`, or of `fallback` when the key is absent.
pub(crate) fn range_of(node: Option<NodeRef<'_>>, fallback: NodeRef<'_>) -> std::ops::Range<usize> {
    node.map_or_else(|| fallback.byte_range(), |n| n.byte_range())
}

/// True when `s` is one of the JSON-Schema primitive type names.
fn is_valid_type(s: &str) -> bool {
    matches!(s, "null" | "boolean" | "object" | "array" | "number" | "integer" | "string")
}
