//! Language-neutral source-selected HTTP admission and wire planning.
//!
//! This module owns the strict, source-located admission shared by every HTTP
//! backend: effective server/security/parameter/body/response declaration
//! checks, wire shape proofs (string path parameters, form-scalar queries),
//! wire path-placeholder matching and exact source identities. Per-language
//! native names and symbol collisions, resource limits, directional-codec
//! guards, codec planning, model symbols, docs and artifact emission stay in
//! each backend. Unsupported declarations produce source-linked diagnostics
//! before any artifact exists; nothing is inferred from operation names.

use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    ops::Range,
};
use suspect_ir::contract::{
    Contract, ParameterLocation, ParameterStyle, ResponseStatus, SchemaId, SecuritySchemeKind,
    SourceId,
};

/// Wire contract for exactly the selected operations, shared by all backends.
#[derive(Debug, Clone)]
pub(crate) struct HttpContract {
    pub operations: Vec<Operation>,
    pub roots: Vec<SchemaId>,
}

/// A planner rejection tied to the precise source construct.
#[derive(Debug, Clone)]
pub struct HttpDiagnostic {
    pub source: SourceId,
    pub at: Range<usize>,
    pub code: &'static str,
    pub message: String,
}

/// One admitted outgoing operation with its exact wire shape and sources.
#[derive(Debug, Clone)]
pub(crate) struct Operation {
    pub source: SourceId,
    pub operation_id: String,
    pub description: String,
    pub method: String,
    pub path: String,
    pub server: String,
    pub security_scheme_name: String,
    pub parameters: Vec<Parameter>,
    pub body: Option<Body>,
    pub responses: Vec<Response>,
}

/// One admitted parameter with its exact wire serialization.
#[derive(Debug, Clone)]
pub(crate) struct Parameter {
    pub source: SourceId,
    pub wire_name: String,
    pub location: ParameterLocation,
    pub schema: SchemaId,
}

/// One admitted request body.
#[derive(Debug, Clone)]
pub(crate) struct Body {
    /// Terminal media container, including through a Request Body reference.
    pub media_source: SourceId,
    pub required: bool,
    pub media_type: String,
    pub schema: SchemaId,
}

/// One admitted response.
#[derive(Debug, Clone)]
pub(crate) struct Response {
    pub source: SourceId,
    /// Terminal media container, including through a Response reference.
    pub media_source: SourceId,
    pub status: u16,
    pub media_type: String,
    pub schema: SchemaId,
}

/// Plan the wire contract for exactly the selected operation sources.
///
/// Every selected source must be an admitted outgoing path operation; each
/// admitted schema contributes to `roots` in admission order (sorted and
/// deduplicated). Wire path placeholders must exactly match the effective
/// path parameters.
///
/// # Errors
/// Unsupported or malformed declarations produce source-linked diagnostics
/// and no contract.
pub(crate) fn plan(
    contract: &Contract,
    selected: &[SourceId],
) -> Result<HttpContract, Vec<HttpDiagnostic>> {
    let mut errors = Vec::new();
    let wanted: BTreeSet<_> = selected.iter().cloned().collect();
    let available: BTreeMap<_, _> = contract
        .operations()
        .map(|o| (o.source().clone(), o))
        .collect();
    for source in &wanted {
        if !available.contains_key(source) {
            errors.push(diag(
                contract,
                source.clone(),
                "http-operation-not-found",
                "selected source is not an outgoing path operation",
            ));
        }
    }
    let mut operations = Vec::new();
    let mut roots = Vec::new();
    for source in &wanted {
        let Some(op) = available.get(source) else {
            continue;
        };
        // The indexed parameter view contains resolved entries, so a malformed
        // collection must be checked before an empty view can hide it.
        for collection in op.parameter_collections() {
            let Some(raw) = contract.source(&collection) else {
                continue;
            };
            let Some(entries) = raw.as_array() else {
                errors.push(diag(
                    contract,
                    collection,
                    "http-parameters-metadata-invalid",
                    "parameters must be an array",
                ));
                continue;
            };
            for (index, entry) in entries.iter().enumerate() {
                if !entry.is_object() {
                    errors.push(diag(
                        contract,
                        collection.child(&index.to_string()),
                        "http-parameters-metadata-invalid",
                        "parameter entries must be objects or references",
                    ));
                }
            }
        }
        let Some(operation_id) = op.operation_id().filter(|s| !s.is_empty()) else {
            errors.push(diag(
                contract,
                source.clone(),
                "http-operation-id",
                "a non-empty operationId is required",
            ));
            continue;
        };
        if !matches!(
            op.method().as_str(),
            "GET" | "POST" | "PATCH" | "PUT" | "DELETE"
        ) {
            errors.push(diag(
                contract,
                source.clone(),
                "http-method-unsupported",
                "this JSON HTTP response profile supports GET, POST, PATCH, PUT and DELETE",
            ));
            continue;
        }
        if let Some(server_source) = op.server_source() {
            match contract.source(server_source).and_then(Value::as_array) {
                Some(entries) => {
                    for (index, entry) in entries.iter().enumerate() {
                        let entry_source = server_source.child(&index.to_string());
                        let Some(object) = entry.as_object() else {
                            errors.push(diag(
                                contract,
                                entry_source,
                                "http-server-metadata-invalid",
                                "server entries must be objects",
                            ));
                            continue;
                        };
                        if object.get("url").is_some_and(|value| !value.is_string()) {
                            errors.push(diag(
                                contract,
                                entry_source.child("url"),
                                "http-server-metadata-invalid",
                                "server url must be a string",
                            ));
                        }
                        if object
                            .get("variables")
                            .is_some_and(|value| !value.is_object())
                        {
                            errors.push(diag(
                                contract,
                                entry_source.child("variables"),
                                "http-server-metadata-invalid",
                                "server variables must be an object",
                            ));
                        }
                    }
                }
                None => errors.push(diag(
                    contract,
                    server_source.clone(),
                    "http-server-metadata-invalid",
                    "effective servers must be an array",
                )),
            }
        }
        let servers = op.effective_servers();
        let server = if servers.len() == 1 && servers[0].variables().is_empty() {
            servers[0]
                .url()
                .filter(|u| valid_server(u))
                .map(str::to_owned)
        } else {
            None
        };
        let Some(server) = server else {
            errors.push(diag(
                contract,
                op.server_source()
                    .cloned()
                    .unwrap_or_else(|| source.clone()),
                "http-server-unsupported",
                "exactly one static absolute HTTPS server is supported",
            ));
            continue;
        };
        let security_source = op
            .security_source()
            .cloned()
            .unwrap_or_else(|| source.clone());
        // A missing declaration inherits nothing and is shaped correctly;
        // only an explicitly declared container can be malformed.
        if op.security_source().is_some()
            && let Some(raw) = contract.source(&security_source)
        {
            match raw.as_array() {
                Some(requirements) => {
                    for (requirement_index, requirement) in requirements.iter().enumerate() {
                        let requirement_source =
                            security_source.child(&requirement_index.to_string());
                        let Some(object) = requirement.as_object() else {
                            errors.push(diag(
                                contract,
                                requirement_source,
                                "http-security-metadata-invalid",
                                "security requirement entries must be objects",
                            ));
                            continue;
                        };
                        for (name, scopes) in object {
                            let scopes_source = requirement_source.child(name);
                            match scopes.as_array() {
                                Some(values) => {
                                    for (scope_index, scope) in values.iter().enumerate() {
                                        if !scope.is_string() {
                                            errors.push(diag(
                                                contract,
                                                scopes_source.child(&scope_index.to_string()),
                                                "http-security-scopes-invalid",
                                                "security scopes must be strings",
                                            ));
                                        }
                                    }
                                }
                                None => errors.push(diag(
                                    contract,
                                    scopes_source,
                                    "http-security-scopes-invalid",
                                    "security scheme scopes must be an array",
                                )),
                            }
                        }
                    }
                }
                None => errors.push(diag(
                    contract,
                    security_source.clone(),
                    "http-security-metadata-invalid",
                    "effective security must be an array",
                )),
            }
        }
        let requirements = op.effective_security();
        for requirement in &requirements {
            for security_use in requirement.requirements() {
                if let Some(scheme) = security_use.scheme() {
                    let definition = scheme
                        .resolved_source()
                        .unwrap_or_else(|| scheme.source().clone());
                    validate_security_scheme_metadata(contract, &definition, &mut errors);
                }
            }
        }
        let mut security_scheme_name = None;
        let security_ok = requirements.len() == 1 && !requirements[0].is_anonymous() && {
            let uses = requirements[0].requirements();
            let ok = uses.len() == 1
                && uses[0].scopes().is_some_and(|s| s.is_empty())
                && uses[0].scheme().is_some_and(|s| {
                    s.kind() == Some(SecuritySchemeKind::Http) && s.http_scheme() == Some("bearer")
                });
            if ok {
                security_scheme_name = Some(uses[0].name().to_owned());
            }
            ok
        };
        if !security_ok {
            errors.push(diag(contract, security_source.clone(), "http-security-unsupported", "the implemented profile requires one HTTP bearer scheme and no alternative or conjunctive schemes"));
            continue;
        }
        let mut parameters = Vec::new();
        // The index merges path-item and operation levels by overriding in
        // place, so any identity still present twice is a same-level
        // duplicate; a legal override no longer appears twice.
        let mut identities = HashSet::new();
        for p in op.parameters() {
            let parameter_source = p.resolved_source().unwrap_or_else(|| p.source().clone());
            if let Some(raw) = contract
                .source(&parameter_source)
                .and_then(Value::as_object)
            {
                for key in [
                    "required",
                    "explode",
                    "allowReserved",
                    "allowEmptyValue",
                    "deprecated",
                ] {
                    validate_boolean(contract, raw, &parameter_source, key, &mut errors);
                }
                for key in ["name", "in", "style"] {
                    if raw.get(key).is_some_and(|value| !value.is_string()) {
                        errors.push(diag(
                            contract,
                            parameter_source.child(key),
                            "http-parameter-metadata-invalid",
                            format!("parameter {key} must be a string"),
                        ));
                    }
                }
                if let Some(style) = raw.get("style").and_then(Value::as_str)
                    && !matches!(
                        style,
                        "simple"
                            | "form"
                            | "matrix"
                            | "label"
                            | "spaceDelimited"
                            | "pipeDelimited"
                            | "deepObject"
                    )
                {
                    errors.push(diag(
                        contract,
                        parameter_source.child("style"),
                        "http-parameter-metadata-invalid",
                        "parameter style is unknown",
                    ));
                }
            }
            let Some(name) = p.name() else {
                errors.push(diag(
                    contract,
                    p.source().clone(),
                    "http-parameter-name",
                    "parameter name is required",
                ));
                continue;
            };
            let Some(location) = p.location() else {
                errors.push(diag(
                    contract,
                    p.source().clone(),
                    "http-parameter-location",
                    "unknown parameter location",
                ));
                continue;
            };
            if !identities.insert((name.to_owned(), location)) {
                errors.push(diag(
                    contract,
                    p.source().clone(),
                    "http-parameter-duplicate",
                    "duplicate parameter (name, in) at the same declaration level; an operation parameter overrides its path-item counterpart instead of repeating it",
                ));
            }
            let required = p.required().unwrap_or(false);
            let supported = ((location == ParameterLocation::Path
                && required
                && p.effective_style() == Some(ParameterStyle::Simple)
                && p.effective_explode() == Some(false))
                || (location == ParameterLocation::Query
                    && p.effective_style() == Some(ParameterStyle::Form)))
                && p.effective_explode().is_some()
                && !p.allow_reserved().unwrap_or(false)
                && !contract
                    .source(&parameter_source)
                    .and_then(|raw| raw.get("allowEmptyValue"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                && !contract
                    .source(&parameter_source)
                    .is_some_and(|raw| raw.get("content").is_some());
            if !supported {
                errors.push(diag(contract, parameter_source.clone(), "http-parameter-serialization-unsupported", "supported: required simple/explode=false string paths; form scalar or scalar-array queries with either explode value; allowReserved and allowEmptyValue must be absent or false; content parameters are unsupported"));
                continue;
            }
            let Some(schema) = p.schema() else {
                let schema_source = parameter_source.child("schema");
                errors.push(diag(
                    contract,
                    schema_source,
                    if contract
                        .source(&parameter_source)
                        .and_then(|value| value.get("schema"))
                        .is_some()
                    {
                        "http-parameter-schema-invalid"
                    } else {
                        "http-parameter-schema"
                    },
                    "a valid parameter schema is required",
                ));
                continue;
            };
            if location == ParameterLocation::Path
                && !is_string_schema(contract, schema.id(), &mut BTreeSet::new())
            {
                errors.push(diag(
                    contract,
                    schema.id().clone(),
                    "http-path-type-unsupported",
                    "the implemented path serializer requires a string schema",
                ));
                continue;
            }
            if location == ParameterLocation::Query
                && query_shape(contract, schema.id(), &mut BTreeSet::new()).is_none()
            {
                errors.push(diag(contract, schema.id().clone(), "http-query-type-unsupported", "form queries require a non-null string, boolean, integer or number schema, or an array of those scalars; nullable, object and ambiguous representations have no implemented wire mapping"));
                continue;
            }
            roots.push(schema.id().clone());
            parameters.push(Parameter {
                source: p.source().clone(),
                wire_name: name.into(),
                location,
                schema: schema.id().clone(),
            });
        }
        let body = op.request_body().and_then(|b| {
            let body_source = b.resolved_source().unwrap_or_else(|| b.source().clone());
            // The media check below only proves an admitted shape; a declared
            // non-object (including a resolved reference target) is a
            // container defect at its terminal source.
            if contract
                .source(&body_source)
                .is_some_and(|raw| !raw.is_object())
            {
                errors.push(diag(
                    contract,
                    body_source.clone(),
                    "http-body-metadata-invalid",
                    "request body must be an object or reference",
                ));
                return None;
            }
            if let Some(raw) = contract.source(&body_source).and_then(Value::as_object) {
                validate_boolean(contract, raw, &body_source, "required", &mut errors);
                validate_content_metadata(
                    contract,
                    raw.get("content"),
                    &body_source.child("content"),
                    "request body",
                    &mut errors,
                );
            }
            let content = b.content();
            if content.len() != 1
                || content[0].name() != "application/json"
                || !content[0].encoding().is_empty()
            {
                errors.push(diag(
                    contract,
                    b.source().clone(),
                    "http-body-media-unsupported",
                    "request bodies require exactly application/json with no encoding map",
                ));
                return None;
            }
            let Some(schema) = content[0].schema() else {
                let schema_source = content[0].source().child("schema");
                errors.push(diag(
                    contract,
                    schema_source,
                    if content[0].raw().get("schema").is_some() {
                        "http-body-schema-invalid"
                    } else {
                        "http-body-schema"
                    },
                    "a valid JSON request body schema is required",
                ));
                return None;
            };
            roots.push(schema.id().clone());
            Some(Body {
                media_source: content[0].source().clone(),
                required: b.required().unwrap_or(false),
                media_type: "application/json".into(),
                schema: schema.id().clone(),
            })
        });
        validate_responses_metadata(contract, op.raw(), source, &mut errors);
        let mut responses = Vec::new();
        for r in op.responses() {
            let response_source = r.resolved_source().unwrap_or_else(|| r.source().clone());
            validate_response_metadata(contract, &response_source, &mut errors);
            let Some(ResponseStatus::Exact(status)) = r.status() else {
                errors.push(diag(
                    contract,
                    r.source().clone(),
                    "http-response-status-unsupported",
                    "only exact response status codes are implemented",
                ));
                continue;
            };
            if !r.headers().is_empty() || !r.links().is_empty() {
                errors.push(diag(
                    contract,
                    r.source().clone(),
                    "http-response-metadata-unsupported",
                    "typed response headers and links are not implemented",
                ));
                continue;
            }
            let content = r.content();
            if content.len() != 1 || content[0].name() != "application/json" {
                errors.push(diag(
                    contract,
                    r.source().clone(),
                    "http-response-media-unsupported",
                    "each response requires exactly application/json",
                ));
                continue;
            }
            let Some(schema) = content[0].schema() else {
                let schema_source = content[0].source().child("schema");
                errors.push(diag(
                    contract,
                    schema_source,
                    if content[0].raw().get("schema").is_some() {
                        "http-response-schema-invalid"
                    } else {
                        "http-response-schema"
                    },
                    "a valid JSON response schema is required",
                ));
                continue;
            };
            roots.push(schema.id().clone());
            responses.push(Response {
                source: r.source().clone(),
                media_source: content[0].source().clone(),
                status,
                media_type: "application/json".into(),
                schema: schema.id().clone(),
            });
        }
        if responses.is_empty() {
            errors.push(diag(
                contract,
                source.clone(),
                "http-responses",
                "at least one supported response is required",
            ));
        }
        operations.push(Operation {
            source: source.clone(),
            operation_id: operation_id.into(),
            description: op.description().unwrap_or("").to_owned(),
            method: op.method().as_str().into(),
            path: op
                .path_template()
                .expect("client operation has a path")
                .into(),
            server,
            security_scheme_name: security_scheme_name.expect("bearer requirement checked"),
            parameters,
            body,
            responses,
        });
    }
    roots.sort();
    roots.dedup();
    for op in &operations {
        let placeholders: BTreeSet<_> = path_placeholders(&op.path).into_iter().collect();
        let declared: BTreeSet<_> = op
            .parameters
            .iter()
            .filter(|p| p.location == ParameterLocation::Path)
            .map(|p| p.wire_name.as_str())
            .collect();
        if placeholders != declared {
            errors.push(diag(
                contract,
                op.source.clone(),
                "http-path-parameters",
                "path placeholders must exactly match effective path parameters",
            ));
        }
    }
    if errors.is_empty() {
        Ok(HttpContract { operations, roots })
    } else {
        Err(errors)
    }
}

fn valid_server(value: &str) -> bool {
    if value.contains(['\\', '?', '#'])
        || value.chars().any(char::is_whitespace)
        || value.chars().any(char::is_control)
    {
        return false;
    }
    let Ok(url) = url::Url::parse(value) else {
        return false;
    };
    url.scheme() == "https"
        && url.has_host()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
}
fn is_string_schema(contract: &Contract, id: &SchemaId, seen: &mut BTreeSet<SchemaId>) -> bool {
    if !seen.insert(id.clone()) {
        return false;
    }
    let Some(schema) = contract.schema(id) else {
        return false;
    };
    if schema.raw().get("type").is_some_and(|v| v == "string") {
        return true;
    }
    let refs = schema.references();
    refs.len() == 1
        && schema.raw().as_object().is_some_and(|o| {
            o.keys()
                .all(|k| k == "$ref" || k == "description" || k == "summary")
        })
        && refs[0]
            .target
            .as_ref()
            .is_some_and(|target| is_string_schema(contract, target, seen))
}
// Prove one wire shape, never infer one from a parameter name or extension.
fn query_shape(contract: &Contract, id: &SchemaId, seen: &mut BTreeSet<SchemaId>) -> Option<bool> {
    if !seen.insert(id.clone()) {
        return None;
    }
    let schema = contract.schema(id)?;
    let raw = schema.raw();
    if raw
        .get("nullable")
        .is_some_and(|v| v != &Value::Bool(false))
    {
        return None;
    }
    match raw.get("type").and_then(Value::as_str) {
        Some("string" | "boolean" | "integer" | "number") => Some(false),
        Some("array") => {
            if raw.get("prefixItems").is_some() {
                return None;
            }
            match query_shape(contract, &id.child("items"), seen) {
                Some(false) => Some(true),
                _ => None,
            }
        }
        _ => {
            let refs = schema.references();
            if refs.len() != 1
                || !raw
                    .as_object()?
                    .keys()
                    .all(|k| matches!(k.as_str(), "$ref" | "description" | "summary"))
            {
                return None;
            }
            query_shape(contract, refs[0].target.as_ref()?, seen)
        }
    }
}
fn path_placeholders(path: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut rest = path;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else { break };
        result.push(&after[..close]);
        rest = &after[close + 1..];
    }
    result
}

fn validate_boolean(
    contract: &Contract,
    object: &serde_json::Map<String, Value>,
    source: &SourceId,
    key: &'static str,
    errors: &mut Vec<HttpDiagnostic>,
) {
    if object.get(key).is_some_and(|value| !value.is_boolean()) {
        errors.push(diag(
            contract,
            source.child(key),
            "http-boolean-metadata-invalid",
            format!("{key} must be a boolean"),
        ));
    }
}
fn validate_security_scheme_metadata(
    contract: &Contract,
    source: &SourceId,
    errors: &mut Vec<HttpDiagnostic>,
) {
    if let Some(raw) = contract.source(source).and_then(Value::as_object) {
        for key in ["type", "scheme", "bearerFormat"] {
            if raw.get(key).is_some_and(|value| !value.is_string()) {
                errors.push(diag(
                    contract,
                    source.child(key),
                    "http-security-scheme-metadata-invalid",
                    format!("security scheme {key} must be a string"),
                ));
            }
        }
    }
}
fn validate_responses_metadata(
    contract: &Contract,
    operation: &Value,
    source: &SourceId,
    errors: &mut Vec<HttpDiagnostic>,
) {
    let responses_source = source.child("responses");
    let Some(responses) = operation.get("responses").and_then(Value::as_object) else {
        errors.push(diag(
            contract,
            responses_source,
            "http-responses-metadata-invalid",
            "responses must be an object",
        ));
        return;
    };
    for (name, response) in responses {
        if !name.starts_with("x-") && !response.is_object() {
            errors.push(diag(
                contract,
                responses_source.child(name),
                "http-response-metadata-invalid",
                "response entries must be Response Objects or references",
            ));
        }
    }
}
fn validate_response_metadata(
    contract: &Contract,
    source: &SourceId,
    errors: &mut Vec<HttpDiagnostic>,
) {
    let Some(response) = contract.source(source).and_then(Value::as_object) else {
        errors.push(diag(
            contract,
            source.clone(),
            "http-response-metadata-invalid",
            "response must resolve to an object",
        ));
        return;
    };
    if response
        .get("description")
        .is_some_and(|value| !value.is_string())
    {
        errors.push(diag(
            contract,
            source.child("description"),
            "http-response-metadata-invalid",
            "response description must be a string",
        ));
    }
    validate_content_metadata(
        contract,
        response.get("content"),
        &source.child("content"),
        "response",
        errors,
    );
    for key in ["headers", "links"] {
        let Some(value) = response.get(key) else {
            continue;
        };
        let field_source = source.child(key);
        let Some(entries) = value.as_object() else {
            errors.push(diag(
                contract,
                field_source,
                "http-response-metadata-invalid",
                format!("response {key} must be an object"),
            ));
            continue;
        };
        for (name, entry) in entries {
            if !entry.is_object() {
                errors.push(diag(
                    contract,
                    field_source.child(name),
                    "http-response-metadata-invalid",
                    format!("response {key} entries must be objects or references"),
                ));
            }
        }
    }
}
fn validate_content_metadata(
    contract: &Contract,
    value: Option<&Value>,
    source: &SourceId,
    owner: &str,
    errors: &mut Vec<HttpDiagnostic>,
) {
    let Some(value) = value else { return };
    let Some(entries) = value.as_object() else {
        errors.push(diag(
            contract,
            source.clone(),
            "http-content-metadata-invalid",
            format!("{owner} content must be an object"),
        ));
        return;
    };
    for (name, entry) in entries {
        let entry_source = source.child(name);
        let Some(media) = entry.as_object() else {
            errors.push(diag(
                contract,
                entry_source,
                "http-content-metadata-invalid",
                format!("{owner} media entries must be objects"),
            ));
            continue;
        };
        if let Some(schema) = media.get("schema")
            && !schema.is_object()
            && !schema.is_boolean()
        {
            errors.push(diag(
                contract,
                entry_source.child("schema"),
                "http-schema-metadata-invalid",
                format!("{owner} schema must be an object or boolean"),
            ));
        }
    }
}
fn diag(
    contract: &Contract,
    source: SourceId,
    code: &'static str,
    message: impl Into<String>,
) -> HttpDiagnostic {
    HttpDiagnostic {
        at: contract.source_span(&source).unwrap_or(0..0),
        source,
        code,
        message: message.into(),
    }
}
