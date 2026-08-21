//! Tag-declaration check.

use rustc_hash::FxHashSet;
use suspect_oas::OpenApi;

use super::diag;
use crate::diagnostic::{Diagnostic, Severity};

/// `oas-tag-undeclared` (Warning): operation tags should be declared in the
/// root `tags` list.
pub(crate) fn check_declared_tags(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    let declared: FxHashSet<&str> =
        api.tags().into_iter().filter_map(|t| t.name()).collect();
    for op in api.operations() {
        let Some(tags_node) = op.node().get("tags") else { continue };
        for item in tags_node.items() {
            if let Some(name) = item.as_str()
                && !declared.contains(name) {
                    out.push(diag(
                        api,
                        "oas-tag-undeclared",
                        Severity::Warning,
                        item.byte_range(),
                        format!("tag `{name}` on {} operation is not declared in the root `tags`", op.method()),
                    ));
                }
        }
    }
}
