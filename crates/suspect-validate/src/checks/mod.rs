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
use rayon::prelude::*;

/// One validation check group.
type CheckFn = fn(&OpenApi<'_>, &mut Vec<Diagnostic>);

/// Every check group, in canonical output order. Groups are independent
/// (read-only over the same tree), so they execute in parallel buckets and
/// merge in this order to keep diagnostics deterministic.
#[rustfmt::skip]
fn check_groups() -> Vec<(&'static str, CheckFn)> {
    vec![
        ("operations::operation_ids",          operations::check_operation_ids),
        ("operations::missing_responses",      operations::check_missing_responses),
        ("operations::deprecated",             operations::check_deprecated),
        ("parameters::fields",                 parameters::check_parameter_fields),
        ("parameters::required_path_params",   parameters::check_required_path_params),
        ("parameters::duplicate_header_params",parameters::check_duplicate_header_params),
        ("paths::keys",                        paths::check_path_keys),
        ("paths::templates",                   paths::check_path_templates),
        ("responses::descriptions",            responses::check_response_descriptions),
        ("security::schemes",                  security::check_security_schemes),
        ("servers::variables",                 servers::check_server_variables),
        ("tags::declared",                     tags::check_declared_tags),
        ("schemas",                            schemas::check_schemas),
        ("examples",                           examples::check_example_types),
        ("webhooks",                           webhooks::check_webhook_version),
        ("info::license",                      info::check_license),
    ]
}

/// Runs every check group against `api`, appending findings to `out` in
/// module order; the caller sorts the accumulated diagnostics.
///
/// Groups are executed in parallel buckets when the rayon pool has more
/// than one thread; with `SUSPECT_PROFILE=1` the run degrades to a
/// sequential per-check timed pass instead (profiling wants serial truth).
pub(crate) fn run_all(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    let groups = check_groups();
    let profile = std::env::var_os("SUSPECT_PROFILE").is_some();

    if profile {
        for (name, check) in &groups {
            let t = std::time::Instant::now();
            check(api, out);
            eprintln!(
                "[suspect-validate profile] {:>9.2} ms  {}",
                t.elapsed().as_secs_f64() * 1000.0,
                name
            );
        }
        return;
    }

    // Parallel buckets: round-robin groups across `buckets` workers so each
    // worker appends into its own vec (no locking); merge in group order.
    let buckets = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(groups.len());
    let lists: Vec<Vec<Diagnostic>> = if buckets > 1 {
        let groups_arc = &groups;
        let api_ref = &api;
        (0..buckets)
            .into_par_iter()
            .map(|b| {
                let mut out_b = Vec::new();
                for (idx, (_name, check)) in groups_arc.iter().enumerate() {
                    if idx % buckets == b {
                        check(api_ref, &mut out_b);
                    }
                }
                out_b
            })
            .collect()
    } else {
        let mut all = Vec::new();
        for (_name, check) in &groups {
            check(api, &mut all);
        }
        vec![all]
    };

    for list in lists {
        out.extend(list);
    }
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
        let Some(close_rel) = rest[open + 1..].find('}') else {
            break;
        };
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
    matches!(
        s,
        "null" | "boolean" | "object" | "array" | "number" | "integer" | "string"
    )
}
