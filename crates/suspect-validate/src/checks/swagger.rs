//! Swagger 2.0 validation battery.
//!
//! The typed [`suspect_oas::OpenApi`] model is OAS 3.x-specific, so the
//! Swagger battery walks the low tree directly and produces the same
//! [`Diagnostic`] shape as the 3.x checks. Coverage focuses on the
//! structural surface an author actually edits: info, operations,
//! parameters, definitions, responses, security, and path templates.

use rustc_hash::FxHashSet;
use suspect_low::{LowDoc, NodeRef, ValueKind};
use suspect_source::Uri;

use super::template_vars;
use crate::diagnostic::{Diagnostic, Severity};

/// Runs the full Swagger 2.0 battery over `low`, sorted by
/// `(range, code)` like the 3.x battery.
pub(crate) fn run(low: &LowDoc) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let doc = low.uri().clone();
    let root = low.root().resolved();

    let components: std::collections::BTreeMap<String, serde_json::Value> = root
        .get("definitions")
        .map(|defs| {
            defs.entries()
                .iter()
                .filter_map(|e| {
                    e.value.map(|v| {
                        let json = suspect_overlay::Value::from_node(v).to_json();
                        (
                            e.key.to_owned(),
                            serde_json::from_str::<serde_json::Value>(&json)
                                .unwrap_or(serde_json::Value::Bool(true)),
                        )
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    out.extend(super::schema_instances::check_swagger_definition_instances(
        low,
        &components,
    ));
    check_info(&root, &doc, &mut out);
    check_operations(&root, &doc, &mut out);
    check_security(&root, &doc, &mut out);
    check_definitions(&root, &doc, &mut out);
    check_path_templates(&root, &doc, &mut out);

    out.sort_by(|a, b| {
        (a.range.start, a.range.end, a.code).cmp(&(b.range.start, b.range.end, b.code))
    });
    out
}

fn diag(
    doc: &Uri,
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
        doc: doc.clone(),
        summary: crate::guidance::summary(code),
        how_to_fix: crate::guidance::how_to_fix(code),
    }
}

/// `info` must exist and carry `title` + `version`.
fn check_info(root: &NodeRef<'_>, doc: &Uri, out: &mut Vec<Diagnostic>) {
    let Some(info) = root.get("info") else {
        out.push(diag(
            doc,
            "swagger-info-required",
            Severity::Error,
            root.byte_range(),
            "Swagger 2.0 requires an `info` object",
        ));
        return;
    };
    for field in ["title", "version"] {
        if info.get(field).is_none() {
            out.push(diag(
                doc,
                "swagger-info-field-required",
                Severity::Error,
                info.byte_range(),
                format!("`info.{field}` is required"),
            ));
        }
    }
}

const METHODS: [&str; 8] = [
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

/// Operations must define `responses`; parameters must carry `name`, `in`,
/// and a schema for body parameters; responses need descriptions.
fn check_operations(root: &NodeRef<'_>, doc: &Uri, out: &mut Vec<Diagnostic>) {
    let Some(paths) = root.get("paths") else {
        return;
    };
    for path_entry in paths.entries() {
        let Some(path_item) = path_entry.value else {
            continue;
        };
        // Path-item level parameters count toward template declarations.
        let mut declared = FxHashSet::default();
        collect_path_params(&path_item, &mut declared);
        for method in METHODS {
            let Some(op) = path_item.get(method) else {
                continue;
            };
            let Some(responses) = op.get("responses") else {
                out.push(diag(
                    doc,
                    "swagger-operation-missing-responses",
                    Severity::Error,
                    op.byte_range(),
                    format!(
                        "{} {} must define `responses`",
                        method.to_ascii_uppercase(),
                        path_entry.key
                    ),
                ));
                continue;
            };
            if responses.kind() == ValueKind::Object {
                for entry in responses.entries() {
                    if entry.value.is_some_and(|r| r.get("description").is_none()) {
                        out.push(diag(
                            doc,
                            "swagger-response-missing-description",
                            Severity::Error,
                            entry.key_node.byte_range(),
                            format!("response `{}` must include `description`", entry.key),
                        ));
                    }
                }
            }
            // Operation-level parameters join the path-item ones.
            let mut op_declared = declared.clone();
            collect_path_params(&op, &mut op_declared);
            check_parameters(&op, doc, &mut op_declared, path_entry.key, out);
        }
        // Path-item level parameter objects are checked too.
        let mut item_declared = FxHashSet::default();
        collect_path_params(&path_item, &mut item_declared);
        if let Some(params) = path_item.get("parameters")
            && params.kind() == ValueKind::Array
        {
            for param in params.items() {
                check_parameter_object(&param, doc, &mut item_declared, path_entry.key, out);
            }
        }
    }
}

/// Records `in: path` parameter names declared on a node's `parameters`.
fn collect_path_params(node: &NodeRef<'_>, declared: &mut FxHashSet<String>) {
    let Some(params) = node.get("parameters") else {
        return;
    };
    if params.kind() != ValueKind::Array {
        return;
    }
    for param in params.items() {
        let is_path = param
            .get("in")
            .and_then(|i| i.as_str())
            .is_some_and(|i| i == "path");
        if is_path && let Some(name) = param.get("name").and_then(|n| n.as_str()) {
            declared.insert(name.to_owned());
        }
    }
}

/// Validates one operation's `parameters` array.
fn check_parameters(
    op: &NodeRef<'_>,
    doc: &Uri,
    declared: &mut FxHashSet<String>,
    path_key: &str,
    out: &mut Vec<Diagnostic>,
) {
    let Some(params) = op.get("parameters") else {
        return;
    };
    if params.kind() != ValueKind::Array {
        return;
    }
    for param in params.items() {
        check_parameter_object(&param, doc, declared, path_key, out);
    }
}

/// Validates one parameter object: `name`, `in`, body schema, and — when
/// `in: path` — participation in the path template.
fn check_parameter_object(
    param: &NodeRef<'_>,
    doc: &Uri,
    declared: &mut FxHashSet<String>,
    path_key: &str,
    out: &mut Vec<Diagnostic>,
) {
    if param.get("$ref").is_some() {
        return;
    }
    let name = param.get("name").and_then(|n| n.as_str()).map(String::from);
    if name.is_none() {
        out.push(diag(
            doc,
            "swagger-parameter-missing-name",
            Severity::Error,
            param.byte_range(),
            "parameter must declare `name`",
        ));
    }
    let location = param.get("in").and_then(|i| i.as_str()).map(String::from);
    let Some(location) = location else {
        out.push(diag(
            doc,
            "swagger-parameter-missing-in",
            Severity::Error,
            param.byte_range(),
            "parameter must declare `in`",
        ));
        return;
    };
    if location == "body" && param.get("schema").is_none() {
        out.push(diag(
            doc,
            "swagger-body-param-schema",
            Severity::Error,
            param.byte_range(),
            "body parameter must declare `schema`",
        ));
    }
    if location == "path"
        && let Some(name) = name.as_ref()
        && !template_vars(path_key).iter().any(|v| *v == name)
    {
        out.push(diag(
            doc,
            "swagger-parameter-not-in-template",
            Severity::Error,
            param.byte_range(),
            format!("path parameter `{name}` does not appear in the path template `{path_key}`"),
        ));
    }
    if location == "path"
        && let Some(name) = name
    {
        declared.insert(name);
    }
}

/// Security requirements must name schemes declared under
/// `securityDefinitions`.
fn check_security(root: &NodeRef<'_>, doc: &Uri, out: &mut Vec<Diagnostic>) {
    let Some(security) = root.get("security") else {
        return;
    };
    if security.kind() != ValueKind::Array {
        return;
    }
    let known: FxHashSet<String> = root
        .get("securityDefinitions")
        .map(|defs| {
            defs.entries()
                .iter()
                .map(|e| e.key.to_owned())
                .collect::<FxHashSet<_>>()
        })
        .unwrap_or_default();
    for requirement in security.items() {
        for entry in requirement.resolved().entries() {
            if !known.contains(entry.key) {
                out.push(diag(
                    doc,
                    "swagger-security-undefined",
                    Severity::Error,
                    entry.key_node.byte_range(),
                    format!(
                        "security scheme `{}` is not declared in `securityDefinitions`",
                        entry.key
                    ),
                ));
            }
        }
    }
}

/// Definitions must look like schemas: a `type`, a `$ref`, or a
/// composition of sub-schemas.
fn check_definitions(root: &NodeRef<'_>, doc: &Uri, out: &mut Vec<Diagnostic>) {
    let Some(defs) = root.get("definitions") else {
        return;
    };
    if defs.kind() != ValueKind::Object {
        return;
    }
    for entry in defs.entries() {
        let Some(def) = entry.value else {
            continue;
        };
        let shaped = [
            "type",
            "$ref",
            "allOf",
            "anyOf",
            "oneOf",
            "properties",
            "items",
        ]
        .iter()
        .any(|k| def.get(k).is_some());
        if !shaped {
            out.push(diag(
                doc,
                "swagger-definition-shape",
                Severity::Error,
                def.byte_range(),
                format!(
                    "definition `{}` must declare `type`, a `$ref`, or a composition",
                    entry.key
                ),
            ));
        }
    }
}

/// `{var}` templates in path keys must be declared as `in: path`
/// parameters on the path item or its operations.
fn check_path_templates(root: &NodeRef<'_>, doc: &Uri, out: &mut Vec<Diagnostic>) {
    let Some(paths) = root.get("paths") else {
        return;
    };
    for path_entry in paths.entries() {
        let vars = template_vars(path_entry.key);
        if vars.is_empty() {
            continue;
        }
        let Some(path_item) = path_entry.value else {
            continue;
        };
        let mut declared = FxHashSet::default();
        collect_path_params(&path_item, &mut declared);
        for method in METHODS {
            if let Some(op) = path_item.get(method) {
                collect_path_params(&op, &mut declared);
            }
        }
        for var in vars {
            if !declared.contains(var) {
                out.push(diag(
                    doc,
                    "swagger-path-param-undeclared",
                    Severity::Error,
                    path_entry.key_node.byte_range(),
                    format!(
                        "path template variable `{var}` is not declared as an `in: path` parameter"
                    ),
                ));
            }
        }
    }
}
