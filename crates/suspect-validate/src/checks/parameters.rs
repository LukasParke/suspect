//! Parameter-level checks: required fields, path-param `required`, header
//! duplication.

use rustc_hash::FxHashMap;
use suspect_oas::{OpenApi, Parameter, ParameterIn};

use super::diag;
use crate::diagnostic::{Diagnostic, Severity};

/// Every parameter view reachable from the document: path-item level,
/// operation level (paths and webhooks), and `components/parameters`.
pub(crate) fn all_parameters<'s>(api: &OpenApi<'s>) -> Vec<Parameter<'s>> {
    let mut out = Vec::new();
    if let Some(paths) = api.paths() {
        for (_, item) in paths.iter() {
            out.extend(item.parameters());
            for op in item.operations() {
                out.extend(op.parameters());
            }
        }
    }
    if let Some(webhooks) = api.webhooks() {
        for (_, item) in webhooks.iter() {
            out.extend(item.parameters());
            for op in item.operations() {
                out.extend(op.parameters());
            }
        }
    }
    if let Some(components) = api.components() {
        out.extend(components.parameters().into_iter().map(|(_, p)| p));
    }
    out
}

/// `oas-parameter-missing-name` / `oas-parameter-missing-in` (Error).
pub(crate) fn check_parameter_fields(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    for p in all_parameters(api) {
        let r = p.resolved();
        if r.name().is_none() {
            out.push(diag(
                api,
                "oas-parameter-missing-name",
                Severity::Error,
                r.node().byte_range(),
                "parameter is missing the required `name` field",
            ));
        }
        if r.location().is_none() {
            out.push(diag(
                api,
                "oas-parameter-missing-in",
                Severity::Error,
                r.node().byte_range(),
                "parameter is missing the required `in` field",
            ));
        }
    }
}

/// `oas-parameter-required-missing` (Error): `in: path` parameters must
/// declare `required: true`; an explicit `required: false` is a violation.
pub(crate) fn check_required_path_params(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    for p in all_parameters(api) {
        let r = p.resolved();
        if r.location() == Some(ParameterIn::Path)
            && r.node().get("required").and_then(|n| n.as_bool()) == Some(false)
        {
            let name = r.name().unwrap_or("<unnamed>");
            out.push(diag(
                api,
                "oas-parameter-required-missing",
                Severity::Error,
                r.node().byte_range(),
                format!("path parameter `{name}` must have `required: true`"),
            ));
        }
    }
}

/// `oas-duplicate-header-param` (Error): same header name declared twice on
/// one operation (path-item + operation parameters merged).
pub(crate) fn check_duplicate_header_params(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    // The path-item/operation view types are unnameable from outside
    // suspect-oas, so flatten them into plain data first.
    struct Item<'s> {
        params: Vec<Parameter<'s>>,
        ops: Vec<(&'static str, Vec<Parameter<'s>>)>,
    }

    let mut items: Vec<Item<'_>> = Vec::new();
    if let Some(paths) = api.paths() {
        for (_, item) in paths.iter() {
            items.push(Item {
                params: item.parameters(),
                ops: item.operations().into_iter().map(|op| (op.method(), op.parameters())).collect(),
            });
        }
    }
    if let Some(webhooks) = api.webhooks() {
        for (_, item) in webhooks.iter() {
            items.push(Item {
                params: item.parameters(),
                ops: item.operations().into_iter().map(|op| (op.method(), op.parameters())).collect(),
            });
        }
    }

    for item in items {
        for (method, op_params) in &item.ops {
            let mut first: FxHashMap<&str, ()> = FxHashMap::default();
            let merged = item.params.iter().copied().chain(op_params.iter().copied());
            for p in merged {
                let r = p.resolved();
                if r.location() != Some(ParameterIn::Header) {
                    continue;
                }
                let Some(name) = r.name() else { continue };
                if first.contains_key(name) {
                    out.push(diag(
                        api,
                        "oas-duplicate-header-param",
                        Severity::Error,
                        r.node().byte_range(),
                        format!("header parameter `{name}` declared more than once for {method} operation"),
                    ));
                } else {
                    first.insert(name, ());
                }
            }
        }
    }
}
