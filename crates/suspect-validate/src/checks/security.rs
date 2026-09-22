//! Security-scheme checks.

use rustc_hash::FxHashSet;
use suspect_oas::OpenApi;

use super::diag_at;
use crate::diagnostic::{Diagnostic, Severity};

/// `oas-security-unknown-scheme` (Error): every scheme named in a security
/// requirement (root + operation level) must exist under
/// `components/securitySchemes`.
pub(crate) fn check_security_schemes(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    let known: FxHashSet<Vec<u8>> = api
        .root()
        .get("components")
        .and_then(|components| components.get("securitySchemes"))
        .map(|schemes| {
            schemes
                .entries()
                .into_iter()
                .filter_map(|entry| {
                    entry
                        .key_node
                        .try_decoded_scalar()
                        .map(|name| name.into_owned())
                })
                .collect()
        })
        .unwrap_or_default();

    let check_requirement = |req: &suspect_oas::SecurityRequirement<'_>,
                             out: &mut Vec<Diagnostic>| {
        for entry in req.node().entries() {
            let Some(name) = entry.key_node.try_decoded_scalar() else {
                continue;
            };
            if !known.contains(name.as_ref()) {
                let name = String::from_utf8_lossy(&name);
                out.push(diag_at(
                    req.node(),
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
