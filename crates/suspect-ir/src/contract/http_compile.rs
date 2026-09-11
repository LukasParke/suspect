use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::http::{HttpIndex, Object, OperationKind, OperationNode, Parameter};
use super::{Contract, ContractDiagnostic, ContractSeverity, SourceId};

struct PathTask {
    source: SourceId,
    kind: OperationKind,
    route: String,
    callback: Option<(SourceId, String)>,
    ancestors: Vec<SourceId>,
}

pub(super) fn index(contract: &Contract) -> (HttpIndex, Vec<ContractDiagnostic>) {
    let mut out = HttpIndex::default();
    let mut diagnostics = super::http_validate::validate(contract);
    let root = SourceId::new(contract.entry().clone(), suspect_low::Pointer::root());
    let servers = contract
        .source(&root)
        .and_then(|v| v.get("servers"))
        .map(|_| root.child("servers"));
    let security = contract
        .source(&root)
        .and_then(|v| v.get("security"))
        .map(|_| root.child("security"));
    let mut pending = Vec::new();
    for (key, kind) in [
        ("paths", OperationKind::Client),
        ("webhooks", OperationKind::Webhook),
    ] {
        if key == "webhooks" && contract.openapi_version().starts_with("3.0.") {
            continue;
        }
        if let Some(paths) = contract.source(&root.child(key)).and_then(Value::as_object) {
            for path in paths
                .keys()
                .filter(|p| key != "paths" || !p.starts_with("x-"))
            {
                if kind == OperationKind::Client && !path.starts_with('/') {
                    diagnostics.push(error(
                        contract,
                        root.child(key).child(path),
                        "INVALID_PATH_TEMPLATE",
                        "paths keys must begin with /",
                    ));
                }
                pending.push(PathTask {
                    source: root.child(key).child(path),
                    kind,
                    route: path.clone(),
                    callback: None,
                    ancestors: Vec::new(),
                });
            }
        }
    }
    let mut operation_ids = HashMap::<String, SourceId>::new();
    let mut operation_mounts = HashMap::<(SourceId, OperationKind), SourceId>::new();
    while let Some(task) = pending.pop() {
        let item = Object {
            contract,
            source: task.source.clone(),
            path_item: true,
        };
        let shared = parameter_sources(&item);
        let path_servers = item
            .field("servers")
            .map(|(source, _)| source)
            .or_else(|| servers.clone());
        for (method, source, raw) in item.operations() {
            if !raw.is_object() {
                continue;
            }
            let operation = Object::new(contract, source.clone());
            let declared = parameter_sources(&operation);
            let mut parameters = shared.clone();
            let mut by_identity = HashMap::new();
            for (i, source) in parameters.iter().enumerate() {
                let p = Parameter {
                    object: Object::new(contract, source.clone()),
                };
                if let Some(identity) = p.name().zip(p.location()) {
                    by_identity.insert((identity.0.to_owned(), identity.1), i);
                }
            }
            let mut declared_identities = HashSet::new();
            for source in declared {
                let p = Parameter {
                    object: Object::new(contract, source.clone()),
                };
                let identity = p
                    .name()
                    .zip(p.location())
                    .map(|(name, location)| (name.to_owned(), location));
                if let Some(identity) = identity
                    && declared_identities.insert(identity.clone())
                    && let Some(&i) = by_identity.get(&identity)
                {
                    parameters[i] = source;
                } else {
                    parameters.push(source);
                }
            }
            diagnostics.extend(super::http_validate::parameter_set(contract, &parameters));
            if let Some(id) = operation.string("operationId") {
                let previous = operation_ids
                    .entry(id.to_owned())
                    .or_insert_with(|| source.clone());
                if *previous != source {
                    diagnostics.push(error(
                        contract,
                        source.child("operationId"),
                        "DUPLICATE_OPERATION_ID",
                        format!(
                            "operationId {id:?} is also declared at {}{}",
                            previous.document(),
                            previous.pointer()
                        ),
                    ));
                }
                if task.kind != OperationKind::Callback {
                    let previous_mount = operation_mounts
                        .entry((source.clone(), task.kind))
                        .or_insert_with(|| task.source.clone());
                    if *previous_mount != task.source {
                        diagnostics.push(error(contract, task.source.clone(), "AMBIGUOUS_OPERATION_MOUNT", format!("operationId {id:?} is reused at distinct path-item mounts; its declaration is also mounted at {}{}", previous_mount.document(), previous_mount.pointer())));
                    }
                }
            }
            for (name, callback) in operation.named("callbacks").into_iter() {
                let Some(callback_source) = callback.resolved_source() else {
                    continue;
                };
                let Some(expressions) =
                    contract.source(&callback_source).and_then(Value::as_object)
                else {
                    continue;
                };
                for expression in expressions.keys().filter(|key| !key.starts_with("x-")) {
                    let path_source = callback_source.child(expression);
                    if path_source == task.source || task.ancestors.contains(&path_source) {
                        diagnostics.push(error(contract, callback.source.clone(), "RECURSIVE_CALLBACK", "recursive callback declarations cannot be expanded into a finite operation collection"));
                        continue;
                    }
                    let mut ancestors = task.ancestors.clone();
                    ancestors.push(task.source.clone());
                    pending.push(PathTask {
                        source: path_source,
                        kind: OperationKind::Callback,
                        route: expression.clone(),
                        callback: Some((source.clone(), name.to_owned())),
                        ancestors,
                    });
                }
            }
            out.operations.push(OperationNode {
                source,
                method: method.as_str().to_owned(),
                kind: task.kind,
                route: task.route.clone(),
                path_item: task.source.clone(),
                callback: task.callback.clone(),
                parameters,
                servers: operation
                    .field("servers")
                    .map(|(source, _)| source)
                    .or_else(|| path_servers.clone()),
                security: operation
                    .field("security")
                    .map(|(source, _)| source)
                    .or_else(|| security.clone()),
            });
        }
    }
    out.operations.sort_by(|a, b| {
        a.path_item
            .cmp(&b.path_item)
            .then_with(|| a.method.as_str().cmp(b.method.as_str()))
    });
    (out, diagnostics)
}

fn parameter_sources(object: &Object<'_>) -> Vec<SourceId> {
    object
        .field("parameters")
        .and_then(|(source, value)| {
            value.as_array().map(|array| {
                array
                    .iter()
                    .enumerate()
                    .map(|(i, _)| source.child(&i.to_string()))
                    .collect()
            })
        })
        .unwrap_or_default()
}

fn error(
    contract: &Contract,
    source: SourceId,
    code: &'static str,
    message: impl Into<String>,
) -> ContractDiagnostic {
    let at = contract.source_span(&source).unwrap_or_default();
    ContractDiagnostic {
        source,
        at,
        code,
        severity: ContractSeverity::Error,
        message: message.into(),
    }
}
