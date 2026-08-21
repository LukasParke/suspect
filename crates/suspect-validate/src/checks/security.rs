//! Security-scheme checks.

use rustc_hash::FxHashSet;
use suspect_oas::OpenApi;

use super::diag;
use crate::diagnostic::{Diagnostic, Severity};

/// `oas-security-unknown-scheme` (Error): every scheme named in a security
/// requirement (root + operation level) must exist under
/// `components/securitySchemes`.
pub(crate) fn check_security_schemes(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    let known: FxHashSet<&str> = api
        .components()
        .map(|c| c.security_schemes().into_iter().map(|(name, _)| name).collect())
        .unwrap_or_default();

    let check_requirement = |req: &suspect_oas::SecurityRequirement<'_>, out: &mut Vec<Diagnostic>| {
        for (name, _) in req.requirements() {
            if !known.contains(name) {
                out.push(diag(
                    api,
                    "oas-security-unknown-scheme",
                    Severity::Error,
                    req.node().byte_range(),
                    format!("security requirement references unknown scheme `{name}`"),
                ));
            }
        }
    };

    for req in api.security() {
        check_requirement(&req, out);
    }
    for op in api.operations() {
        for req in op.security() {
            check_requirement(&req, out);
        }
    }
}
