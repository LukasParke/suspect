//! Path-key and path-template checks.

use rustc_hash::FxHashSet;
use suspect_low::NodeRef;
use suspect_oas::{OpenApi, ParameterIn};

use super::{diag, template_vars};
use crate::diagnostic::{Diagnostic, Severity};

/// Keys of the `paths` object that are not path templates but part of the
/// Path Object itself (3.2 adds `summary`/`description` at this level).
const NON_PATH_KEYS: [&str; 2] = ["summary", "description"];

/// `oas-path-trailing-slash` (Warning) and `oas-empty-path-template` (Error).
pub(crate) fn check_path_keys(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    let Some(paths_node) = api.root().get("paths") else { return };
    for entry in paths_node.entries() {
        let Some(value) = entry.value else { continue };
        let key = entry.key;
        if key.starts_with("x-") || NON_PATH_KEYS.contains(&key) {
            continue;
        }
        if !key.starts_with('/') {
            out.push(diag(
                api,
                "oas-empty-path-template",
                Severity::Error,
                value.byte_range(),
                format!("path `{key}` must start with `/`"),
            ));
        } else if key.len() > 1 && key.ends_with('/') {
            out.push(diag(
                api,
                "oas-path-trailing-slash",
                Severity::Warning,
                value.byte_range(),
                format!("path `{key}` has a trailing slash"),
            ));
        }
    }
}

/// `oas-path-param-not-declared` (Error) and `oas-unused-path-param` (Warning).
pub(crate) fn check_path_templates<'s>(api: &OpenApi<'s>, out: &mut Vec<Diagnostic>) {
    let Some(paths) = api.paths() else { return };
    for (path, item) in paths.iter() {
        let vars: Vec<&str> = template_vars(&path);
        let r = item.resolved();

        let mut declared: Vec<(&str, NodeRef<'_>)> = Vec::new();
        let mut collect = |params: Vec<suspect_oas::Parameter<'s>>| {
            for p in params {
                let pr = p.resolved();
                if pr.location() == Some(ParameterIn::Path)
                    && let Some(name) = pr.name() {
                        declared.push((name, pr.node()));
                    }
            }
        };
        collect(r.parameters());
        for op in r.operations() {
            collect(op.parameters());
        }

        let item_range = item.node().byte_range();
        for var in &vars {
            if !declared.iter().any(|(name, _)| name == var) {
                out.push(diag(
                    api,
                    "oas-path-param-not-declared",
                    Severity::Error,
                    item_range.clone(),
                    format!("path `{path}` uses template variable `{{{var}}}` but no `in: path` parameter declares it"),
                ));
            }
        }
        let template: FxHashSet<&str> = vars.into_iter().collect();
        for (name, node) in &declared {
            if !template.contains(name) {
                out.push(diag(
                    api,
                    "oas-unused-path-param",
                    Severity::Warning,
                    node.byte_range(),
                    format!("path parameter `{name}` is declared but does not appear in the template of path `{path}`"),
                ));
            }
        }
    }
}
