//! Shape checks for HTTP declarations whose typed accessors otherwise filter or default malformed values.

use std::collections::BTreeSet;

use suspect_low::{NodeRef, ValueKind};
use suspect_oas::{OasVersion, OpenApi};
use suspect_ref::Resolution;

use super::diag_at;
use crate::{Diagnostic, Severity};

struct Findings<'a> {
    out: &'a mut Vec<Diagnostic>,
    seen: BTreeSet<(String, usize, usize, &'static str)>,
}
impl Findings<'_> {
    fn error(&mut self, node: NodeRef<'_>, code: &'static str, message: impl Into<String>) {
        let range = node.byte_range();
        let doc = node.syntax().doc().uri().to_string();
        if self.seen.insert((doc, range.start, range.end, code)) {
            self.out
                .push(diag_at(node, code, Severity::Error, range, message));
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Path,
    Operation,
    Callback,
    Parameter,
    Header,
    Body,
    Response,
    Media,
    Link,
    Server,
    Variable,
}
impl Kind {
    fn code(self) -> &'static str {
        match self {
            Self::Path => "oas-path-item-invalid",
            Self::Operation => "oas-operation-invalid",
            Self::Callback => "oas-callback-invalid",
            Self::Parameter => "oas-parameter-invalid",
            Self::Header => "oas-header-invalid",
            Self::Body => "oas-request-body-invalid",
            Self::Response => "oas-response-invalid",
            Self::Media => "oas-media-type-invalid",
            Self::Link => "oas-link-invalid",
            Self::Server => "oas-server-invalid",
            Self::Variable => "oas-server-variable-invalid",
        }
    }
}

pub(crate) fn check_http_declarations(api: &OpenApi<'_>, out: &mut Vec<Diagnostic>) {
    let mut findings = Findings {
        out,
        seen: BTreeSet::new(),
    };
    let root = api.root();
    let mut pending = Vec::new();
    check_security(root, api, &mut findings);
    array(root, "servers", Kind::Server, &mut pending, &mut findings);
    map(root, "paths", Kind::Path, true, &mut pending, &mut findings);
    if api.version().is_31_plus() {
        map(
            root,
            "webhooks",
            Kind::Path,
            false,
            &mut pending,
            &mut findings,
        );
    }
    if let Some(components) = field_at(root, "components") {
        if components.kind() != ValueKind::Object {
            findings.error(
                components,
                "oas-http-map-invalid",
                "components must be an object",
            );
        }
        for (key, kind) in [
            ("parameters", Kind::Parameter),
            ("headers", Kind::Header),
            ("requestBodies", Kind::Body),
            ("responses", Kind::Response),
            ("callbacks", Kind::Callback),
            ("links", Kind::Link),
        ] {
            map(components, key, kind, false, &mut pending, &mut findings);
        }
        if api.version().is_31_plus() {
            map(
                components,
                "pathItems",
                Kind::Path,
                false,
                &mut pending,
                &mut findings,
            );
        }
        if api.version() == OasVersion::V32 {
            map(
                components,
                "mediaTypes",
                Kind::Media,
                false,
                &mut pending,
                &mut findings,
            );
        }
    }
    // Iterative traversal: reference/callback cycles cannot consume the call stack.
    // Each source/context is visited once; reference chains use the workspace cap.
    let mut visited = BTreeSet::new();
    while let Some((node, kind)) = pending.pop() {
        let range = node.byte_range();
        if !visited.insert((
            node.syntax().doc().uri().to_string(),
            range.start,
            range.end,
            kind,
        )) {
            continue;
        }
        if node.kind() != ValueKind::Object {
            findings.error(node, kind.code(), "HTTP declaration must be an object");
            continue;
        }
        let version = api
            .session()
            .workspace()
            .get(node.syntax().doc().uri())
            .and_then(|document| OasVersion::sniff(document.doc()))
            .unwrap_or(api.version());
        let reference_allowed = !matches!(kind, Kind::Operation | Kind::Server | Kind::Variable)
            && (kind != Kind::Media || version == OasVersion::V32);
        if reference_allowed && let Some(reference) = field_at(node, "$ref") {
            let resolved = resolve(api, reference);
            let target = if kind == Kind::Path && resolved.is_some() {
                // Admit the bounded chain first, then preserve fields on every
                // Path Item rather than skipping straight to its terminal object.
                api.session()
                    .reference_target(reference)
                    .ok()
                    .and_then(|target| {
                        api.session()
                            .workspace()
                            .get_by_id(target.doc)?
                            .node_at_pointer(&target.pointer)
                            .ok()
                    })
            } else {
                resolved
            };
            if let Some(target) = target {
                pending.push((target, kind));
            }
            // Path Item $ref is a field, not a Reference Object: siblings apply too.
            if kind != Kind::Path {
                continue;
            }
        }
        match kind {
            Kind::Path => {
                if let Some(security) = field_at(node, "security") {
                    findings.error(security, "oas-path-item-security-invalid", "security is not a Path Item field; declare it on the document or operation");
                }
                array(
                    node,
                    "parameters",
                    Kind::Parameter,
                    &mut pending,
                    &mut findings,
                );
                array(node, "servers", Kind::Server, &mut pending, &mut findings);
                for method in [
                    "get", "put", "post", "delete", "options", "head", "patch", "trace",
                ] {
                    if let Some(operation) = field_at(node, method) {
                        pending.push((operation, Kind::Operation));
                    }
                }
                if version == OasVersion::V32 {
                    if let Some(operation) = field_at(node, "query") {
                        pending.push((operation, Kind::Operation));
                    }
                    map(
                        node,
                        "additionalOperations",
                        Kind::Operation,
                        false,
                        &mut pending,
                        &mut findings,
                    );
                }
            }
            Kind::Operation => {
                check_security(node, api, &mut findings);
                check_boolean(node, "deprecated", "operation", &mut findings);
                array(
                    node,
                    "parameters",
                    Kind::Parameter,
                    &mut pending,
                    &mut findings,
                );
                array(node, "servers", Kind::Server, &mut pending, &mut findings);
                if let Some(body) = field_at(node, "requestBody") {
                    pending.push((body, Kind::Body));
                }
                if let Some(responses) = field_at(node, "responses") {
                    check_responses(responses, &mut findings);
                    entries(responses, Kind::Response, true, &mut pending);
                }
                map(
                    node,
                    "callbacks",
                    Kind::Callback,
                    false,
                    &mut pending,
                    &mut findings,
                );
            }
            Kind::Callback => entries(node, Kind::Path, true, &mut pending),
            Kind::Parameter | Kind::Header => {
                for key in [
                    "required",
                    "explode",
                    "allowReserved",
                    "allowEmptyValue",
                    "deprecated",
                ] {
                    check_boolean(node, key, "parameter/header", &mut findings);
                }
                map(
                    node,
                    "content",
                    Kind::Media,
                    false,
                    &mut pending,
                    &mut findings,
                );
            }
            Kind::Body => {
                check_boolean(node, "required", "request body", &mut findings);
                if field_at(node, "content").is_none() {
                    findings.error(
                        node,
                        "oas-request-body-content-missing",
                        "request body requires a content object",
                    );
                }
                map(
                    node,
                    "content",
                    Kind::Media,
                    false,
                    &mut pending,
                    &mut findings,
                );
            }
            Kind::Response => {
                check_response(node, &mut findings);
                for (key, child) in [
                    ("content", Kind::Media),
                    ("headers", Kind::Header),
                    ("links", Kind::Link),
                ] {
                    if let Some(value) = field_at(node, key) {
                        entries(value, child, false, &mut pending);
                    }
                }
            }
            Kind::Server => {
                required_string(node, "url", "oas-server-url-invalid", &mut findings);
                map(
                    node,
                    "variables",
                    Kind::Variable,
                    false,
                    &mut pending,
                    &mut findings,
                );
            }
            Kind::Variable => {
                required_string(
                    node,
                    "default",
                    "oas-server-variable-default-invalid",
                    &mut findings,
                );
                if let Some(values) = field_at(node, "enum") {
                    if values.kind() != ValueKind::Array
                        || (version.is_31_plus() && values.items().is_empty())
                    {
                        findings.error(values, "oas-server-variable-enum-invalid", "server variable enum must be an array of strings, nonempty in OpenAPI 3.1+");
                    } else {
                        for value in values.items() {
                            if value.kind() != ValueKind::Str {
                                findings.error(
                                    value,
                                    "oas-server-variable-enum-invalid",
                                    "server variable enum member must be a string",
                                );
                            }
                        }
                    }
                }
            }
            Kind::Link => {
                if let Some(server) = field_at(node, "server") {
                    pending.push((server, Kind::Server));
                }
            }
            Kind::Media => {}
        }
    }
}

fn resolve<'s>(api: &OpenApi<'s>, reference: NodeRef<'_>) -> Option<NodeRef<'s>> {
    match api.session().resolve_reference(reference).ok()? {
        Resolution::Node(node) => Some(node),
        Resolution::WholeDoc(doc) => Some(api.session().workspace().get_by_id(doc)?.doc().root()),
        Resolution::Cycle { .. } => None, // The shared reference check reports resolution failures.
    }
}

// A present YAML implicit null has no value node. Preserve its key as the
// diagnostic location instead of silently treating the declaration as absent.
fn field_at<'s>(node: NodeRef<'s>, key: &str) -> Option<NodeRef<'s>> {
    node.entries()
        .into_iter()
        .find(|entry| {
            entry
                .key_node
                .try_decoded_scalar()
                .is_some_and(|name| name.as_ref() == key.as_bytes())
        })
        .map(|entry| entry.value.unwrap_or(entry.key_node))
}

fn is_extension(key: NodeRef<'_>) -> bool {
    key.try_decoded_scalar()
        .is_some_and(|name| name.starts_with(b"x-"))
}

fn entries<'s>(
    node: NodeRef<'s>,
    kind: Kind,
    extensions: bool,
    pending: &mut Vec<(NodeRef<'s>, Kind)>,
) {
    for entry in node.entries() {
        if !extensions || !is_extension(entry.key_node) {
            pending.push((entry.value.unwrap_or(entry.key_node), kind));
        }
    }
}

fn map<'s>(
    node: NodeRef<'s>,
    key: &str,
    kind: Kind,
    extensions: bool,
    pending: &mut Vec<(NodeRef<'s>, Kind)>,
    findings: &mut Findings<'_>,
) {
    if let Some(value) = field_at(node, key) {
        if value.kind() != ValueKind::Object {
            findings.error(
                value,
                "oas-http-map-invalid",
                format!("`{key}` must be an object"),
            );
        } else {
            entries(value, kind, extensions, pending);
        }
    }
}

fn array<'s>(
    node: NodeRef<'s>,
    key: &str,
    kind: Kind,
    pending: &mut Vec<(NodeRef<'s>, Kind)>,
    findings: &mut Findings<'_>,
) {
    if let Some(value) = field_at(node, key) {
        if value.kind() != ValueKind::Array {
            findings.error(
                value,
                "oas-http-array-invalid",
                format!("`{key}` must be an array"),
            );
        } else {
            pending.extend(value.items().into_iter().map(|item| (item, kind)));
        }
    }
}

fn required_string(node: NodeRef<'_>, key: &str, code: &'static str, findings: &mut Findings<'_>) {
    if !node
        .get(key)
        .is_some_and(|value| value.kind() == ValueKind::Str)
    {
        findings.error(
            field_at(node, key).unwrap_or(node),
            code,
            format!("`{key}` is required and must be a string"),
        );
    }
}

fn check_security(owner: NodeRef<'_>, api: &OpenApi<'_>, findings: &mut Findings<'_>) {
    let Some(security) = field_at(owner, "security") else {
        return;
    };
    if security.kind() != ValueKind::Array {
        findings.error(
            security,
            "oas-security-invalid",
            "security must be an array of requirement objects",
        );
        return;
    }
    for node in security.items() {
        if node.kind() != ValueKind::Object {
            findings.error(
                node,
                "oas-security-requirement-invalid",
                "security requirement must be an object",
            );
            continue;
        }
        for entry in node.entries() {
            let value = entry.value.unwrap_or(entry.key_node);
            if value.kind() != ValueKind::Array {
                findings.error(
                    value,
                    "oas-security-scopes-invalid",
                    "security requirement scopes must be an array of strings",
                );
                continue;
            }
            for item in value.items() {
                if item.kind() != ValueKind::Str {
                    findings.error(
                        item,
                        "oas-security-scope-invalid",
                        "security requirement scope must be a string",
                    );
                }
            }
            // 3.1+ permits role names for non-OAuth schemes. In 3.0 the array
            // MUST be empty. Scheme naming/resolution policy remains separate.
            let (version, document) = api
                .session()
                .workspace()
                .get(owner.syntax().doc().uri())
                .and_then(|document| {
                    OasVersion::sniff(document.doc())
                        .map(|version| (version, document.doc().root()))
                })
                .unwrap_or((api.version(), api.root()));
            if version == OasVersion::V30 && !value.items().is_empty() {
                let Some(name) = entry.key_node.try_decoded_scalar() else {
                    continue;
                };
                let Ok(name) = std::str::from_utf8(&name) else {
                    continue;
                };
                let scheme = document
                    .get("components")
                    .and_then(|c| c.get("securitySchemes"))
                    .and_then(|schemes| schemes.get(name));
                let scheme = scheme.and_then(|scheme| match scheme.get("$ref") {
                    Some(reference) => resolve(api, reference),
                    None => Some(scheme),
                });
                if scheme
                    .and_then(|scheme| scheme.get("type"))
                    .and_then(|value| value.as_str())
                    .is_some_and(|kind| matches!(kind, "apiKey" | "http"))
                {
                    findings.error(value, "oas-security-scopes-nonempty", "OpenAPI 3.0 non-OAuth security requirements must have an empty scope array");
                }
            }
        }
    }
}

fn check_boolean(node: NodeRef<'_>, key: &str, owner: &str, findings: &mut Findings<'_>) {
    if let Some(value) = field_at(node, key)
        && value.kind() != ValueKind::Bool
    {
        findings.error(
            value,
            "oas-http-boolean-invalid",
            format!("{owner} `{key}` must be a boolean"),
        );
    }
}
fn check_responses(node: NodeRef<'_>, findings: &mut Findings<'_>) {
    if node.kind() != ValueKind::Object {
        findings.error(node, "oas-responses-invalid", "responses must be an object");
        return;
    }
    for entry in node.entries() {
        if !is_extension(entry.key_node) {
            match entry.value {
                Some(value) if value.kind() == ValueKind::Object => {}
                Some(value) => findings.error(
                    value,
                    "oas-response-invalid",
                    "response entry must be an object or reference",
                ),
                None => findings.error(
                    entry.key_node,
                    "oas-response-invalid",
                    "response entry must be an object or reference",
                ),
            }
        }
    }
}
fn check_response(node: NodeRef<'_>, findings: &mut Findings<'_>) {
    for key in ["content", "headers", "links"] {
        let Some(map) = field_at(node, key) else {
            continue;
        };
        if map.kind() != ValueKind::Object {
            findings.error(
                map,
                "oas-response-map-invalid",
                format!("response `{key}` must be an object"),
            );
            continue;
        }
        for entry in map.entries() {
            match entry.value {
                Some(value) if value.kind() == ValueKind::Object => {}
                Some(value) => findings.error(
                    value,
                    "oas-response-entry-invalid",
                    format!("response `{key}` entry must be an object or reference"),
                ),
                None => findings.error(
                    entry.key_node,
                    "oas-response-entry-invalid",
                    format!("response `{key}` entry must be an object or reference"),
                ),
            }
        }
    }
}
