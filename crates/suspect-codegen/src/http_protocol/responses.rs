use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use suspect_ir::contract::SourceId;

use super::planner::Planner;
use super::*;

pub(super) fn plan(p: &mut Planner<'_>, operation: &SourceId) -> Vec<ResponsePlan> {
    let source = operation.child("responses");
    if p.contract.source(&source).is_none() && !p.is_30(operation) {
        p.require(Capability::UndeclaredResponses, operation);
        return Vec::new();
    }
    let Some(map) = p.object(&source, "Responses") else {
        return Vec::new();
    };
    let mut result = Vec::new();
    if map.keys().all(|key| key.starts_with("x-")) {
        p.error(
            &source,
            "http-responses-empty",
            "responses must contain at least one exact, range or default response",
        );
    }
    for (key, _) in map {
        let at = source.child(key);
        if key.starts_with("x-") {
            p.warn(
                &at,
                "http-extension-uninterpreted",
                "response extension is annotation, not a status declaration",
            );
            continue;
        }
        let Some(status) = status(key) else {
            p.error(
                &at,
                "http-response-status",
                "response keys must be exact 100–599, uppercase 1XX–5XX, or lowercase default",
            );
            continue;
        };
        match status {
            ResponseStatus::Range(_) => {
                p.require(Capability::RangeResponses, &at);
            }
            ResponseStatus::Default => {
                p.require(Capability::DefaultResponses, &at);
            }
            _ => {}
        }
        let Some(origin) = p.resolve(&at, false) else {
            continue;
        };
        let terminal = &origin.terminal.source;
        let Some(raw) = p.object(terminal, "Response") else {
            continue;
        };
        p.known(
            terminal,
            raw,
            &["description", "content", "headers", "links", "summary"],
        );
        if raw.contains_key("summary") && !p.is_32(terminal) {
            p.error(
                &terminal.child("summary"),
                "http-version-field",
                "Response.summary is not an OAS 3.0/3.1 field",
            );
        }
        // OAS 3.2 makes description optional. Retain absence separately from
        // the compatible empty display string, without inventing a source node.
        let required_description = !p.is_32(terminal);
        let base_description = p.string(terminal, "description", required_description);
        if required_description && base_description.is_none() {
            continue;
        }
        let declared_description = p.text(&origin, "description", false).or(base_description);
        let description_declared = declared_description.is_some();
        let description = declared_description.unwrap_or_else(|| Located {
            source: p.location(terminal),
            value: String::new(),
        });
        let summary = if p.is_32(terminal) {
            p.text(&origin, "summary", false)
        } else {
            None
        };
        let media = super::bodies::content(p, &terminal.child("content"), false, false);
        if !raw.contains_key("content")
            || raw
                .get("content")
                .and_then(Value::as_object)
                .is_some_and(|v| v.is_empty())
        {
            p.require(Capability::UndeclaredResponseBody, terminal);
        }
        let headers = headers(p, &terminal.child("headers"), false);
        let links = links(p, &terminal.child("links"));
        result.push(ResponsePlan {
            source: origin,
            status_key: key.clone(),
            status,
            description,
            description_declared,
            summary,
            media,
            headers,
            links,
            max_body_bytes: p.capabilities.limits.body,
        });
    }
    result
}

pub(super) fn headers(p: &mut Planner<'_>, source: &SourceId, part: bool) -> Vec<HeaderPlan> {
    let Some(value) = p.contract.source(source) else {
        return Vec::new();
    };
    let Some(map) = value.as_object() else {
        p.error(
            source,
            "http-headers-invalid",
            "headers must be a name to Header Object/reference map",
        );
        return Vec::new();
    };
    if !map.is_empty() {
        p.require(
            if part {
                Capability::PartEncodings
            } else {
                Capability::ResponseHeaders
            },
            source,
        );
    }
    let mut names = BTreeSet::new();
    let mut result = Vec::new();
    for (name, raw) in map {
        let at = source.child(name);
        if !names.insert(name.to_ascii_lowercase()) {
            p.error(
                &at,
                "http-header-duplicate",
                "header names are case-insensitive; duplicate wire header declaration",
            );
        }
        if !raw.is_object() {
            p.error(
                &at,
                "http-header-invalid",
                "header must be an object/reference",
            );
            continue;
        }
        if name.eq_ignore_ascii_case("Content-Type") {
            p.warn(&at, "http-header-ignored", "OpenAPI ignores Content-Type in response/encoding headers; media maps/contentType describe it");
            continue;
        }
        if let Some(header) = super::parameters::header(p, &at, name) {
            result.push(header);
        }
    }
    result
}

fn links(p: &mut Planner<'_>, source: &SourceId) -> Vec<LinkPlan> {
    let Some(value) = p.contract.source(source) else {
        return Vec::new();
    };
    let Some(map) = value.as_object() else {
        p.error(
            source,
            "http-links-invalid",
            "links must be a name to Link Object/reference map",
        );
        return Vec::new();
    };
    if !map.is_empty() {
        p.require(Capability::ResponseLinks, source);
    }
    let mut result = Vec::new();
    for name in map.keys() {
        let Some(origin) = p.resolve(&source.child(name), false) else {
            continue;
        };
        let terminal = &origin.terminal.source;
        let Some(raw) = p.object(terminal, "Link") else {
            continue;
        };
        p.known(
            terminal,
            raw,
            &[
                "operationId",
                "operationRef",
                "parameters",
                "requestBody",
                "description",
                "server",
            ],
        );
        let operation_id = p.string(terminal, "operationId", false);
        let operation_ref = p.string(terminal, "operationRef", false);
        if raw.contains_key("operationId") == raw.contains_key("operationRef") {
            p.error(
                terminal,
                "http-link-target",
                "Link requires exactly one of operationId and operationRef",
            );
            continue;
        }
        let target = if let Some(value) = operation_id {
            let found: Vec<_> = p
                .contract
                .operations()
                .filter(|op| op.operation_id() == Some(&value.value))
                .collect();
            if found.len() != 1 {
                p.error(&value.source.source, "http-link-operation-unresolved", "Link operationId must identify one unambiguous outgoing operation in the contract");
                continue;
            }
            LinkTarget::OperationId {
                operation: p.location(found[0].source()),
                value,
            }
        } else if let Some(value) = operation_ref {
            let Some(operation) = operation_ref_target(p, &value) else {
                continue;
            };
            LinkTarget::OperationRef { value, operation }
        } else {
            continue;
        };
        let mut parameters = BTreeMap::new();
        if let Some(value) = raw.get("parameters") {
            let at = terminal.child("parameters");
            if let Some(map) = value.as_object() {
                for (key, value) in map {
                    parameters.insert(
                        key.clone(),
                        Located {
                            source: p.location(&at.child(key)),
                            value: value.clone(),
                        },
                    );
                }
            } else {
                p.error(
                    &at,
                    "http-link-parameters",
                    "Link parameters must be a map of literal values/runtime expressions",
                );
            }
        }
        let request_body = raw.get("requestBody").map(|v| Located {
            source: p.location(&terminal.child("requestBody")),
            value: v.clone(),
        });
        let description = p.text(&origin, "description", false);
        let server = if raw.contains_key("server") {
            super::servers::server(p, &terminal.child("server"))
        } else {
            None
        };
        result.push(LinkPlan {
            source: origin,
            name: name.clone(),
            target,
            parameters,
            request_body,
            description,
            server,
        });
    }
    result
}

fn operation_ref_target(p: &mut Planner<'_>, value: &Located<String>) -> Option<SourceLocation> {
    // Read-only lookup in the canonical resource catalogue. Missing connection
    // discovery is explicit; a Link neither loads a document nor invokes it.
    let target = match p
        .contract
        .resolve_resource_reference(&value.source.source, &value.value)
    {
        Ok(target) => target,
        Err(error) => {
            p.unsupported(
                &value.source.source,
                "http-link-operation-unresolved",
                format!(
                    "operationRef {:?} has no known canonical target: {}: {}",
                    value.value,
                    error.code(),
                    error.message()
                ),
            );
            return None;
        }
    };
    let found: Vec<_> = p
        .contract
        .operations()
        .filter(|op| op.source() == &target)
        .collect();
    if found.len() != 1 {
        p.error(
            &value.source.source,
            "http-link-operation-unresolved",
            "operationRef must identify one indexed unambiguous outgoing operation",
        );
        return None;
    }
    Some(p.location(found[0].source()))
}

fn status(key: &str) -> Option<ResponseStatus> {
    if key == "default" {
        return Some(ResponseStatus::Default);
    }
    let bytes = key.as_bytes();
    if bytes.len() != 3 || !(b'1'..=b'5').contains(&bytes[0]) {
        return None;
    }
    if &bytes[1..] == b"XX" {
        return Some(ResponseStatus::Range(bytes[0] - b'0'));
    }
    if bytes.iter().all(u8::is_ascii_digit) {
        return key.parse().ok().map(ResponseStatus::Exact);
    }
    None
}
