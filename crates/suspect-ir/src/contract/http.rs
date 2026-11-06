//! Typed HTTP metadata views over the contract's original source documents.

use std::collections::HashSet;

use serde_json::Value;

use super::{Contract, HttpMethod, Schema, SourceId};

/// Supported parameter locations. Unknown values remain raw and are diagnosed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParameterLocation {
    /// `in: query` — one serialized value per parameter in the query string.
    Query,
    /// The entire query string, serialized through `content` (OpenAPI 3.2).
    Querystring,
    /// `in: header`.
    Header,
    /// `in: path` — substitutes a `{name}` template segment.
    Path,
    /// `in: cookie`.
    Cookie,
}

impl ParameterLocation {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "query" => Some(Self::Query),
            "querystring" => Some(Self::Querystring),
            "header" => Some(Self::Header),
            "path" => Some(Self::Path),
            "cookie" => Some(Self::Cookie),
            _ => None,
        }
    }
}

/// OpenAPI parameter serialization styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterStyle {
    /// `form` — the default for query and cookie parameters.
    Form,
    /// Cookie header serialization introduced by OpenAPI 3.2.
    Cookie,
    /// `simple` — the default for path and header parameters.
    Simple,
    /// `matrix` — `;name=value` path prefixes.
    Matrix,
    /// `label` — `.value` path prefixes.
    Label,
    /// `spaceDelimited` — space-joined arrays in the query string.
    SpaceDelimited,
    /// `pipeDelimited` — pipe-joined arrays in the query string.
    PipeDelimited,
    /// `deepObject` — `name[key]=value` objects in the query string.
    DeepObject,
}

impl ParameterStyle {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "form" => Some(Self::Form),
            "cookie" => Some(Self::Cookie),
            "simple" => Some(Self::Simple),
            "matrix" => Some(Self::Matrix),
            "label" => Some(Self::Label),
            "spaceDelimited" => Some(Self::SpaceDelimited),
            "pipeDelimited" => Some(Self::PipeDelimited),
            "deepObject" => Some(Self::DeepObject),
            _ => None,
        }
    }
}

/// The direction/context of an operation declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperationKind {
    /// An operation under `paths`, called by API clients.
    Client,
    /// An operation under `webhooks`, initiated by the provider.
    Webhook,
    /// An operation nested in another operation's `callbacks` map.
    Callback,
}

#[derive(Debug, Default)]
pub(super) struct HttpIndex {
    pub operations: Vec<OperationNode>,
}

#[derive(Debug)]
pub(super) struct OperationNode {
    pub source: SourceId,
    pub method: String,
    pub kind: OperationKind,
    pub route: String,
    pub path_item: SourceId,
    pub callback: Option<(SourceId, String)>,
    pub parameters: Vec<SourceId>,
    pub servers: Option<SourceId>,
    pub security: Option<SourceId>,
}

/// A raw object with reference-aware field access. Source IDs always retain
/// the original declaration; fields may come from a referenced target.
#[derive(Debug, Clone)]
pub(super) struct Object<'a> {
    pub contract: &'a Contract,
    pub source: SourceId,
    pub path_item: bool,
}

impl<'a> Object<'a> {
    pub fn new(contract: &'a Contract, source: SourceId) -> Self {
        Self {
            contract,
            source,
            path_item: false,
        }
    }
    pub fn raw(&self) -> &'a Value {
        self.contract
            .source(&self.source)
            .expect("indexed HTTP source exists")
    }
    pub fn field(&self, name: &str) -> Option<(SourceId, &'a Value)> {
        let mut current = self.source.clone();
        let mut seen = HashSet::new();
        loop {
            if !seen.insert(current.clone()) {
                return None;
            }
            let raw = self.contract.source(&current)?;
            let has_ref = self.contract.reference_targets.contains_key(&current);
            let annotation = !self
                .contract
                .openapi_version_at(&current)
                .starts_with("3.0.")
                && matches!(name, "summary" | "description");
            if (!has_ref || self.path_item || annotation)
                && let Some(value) = raw.get(name)
            {
                return Some((current.child(name), value));
            }
            if !has_ref {
                return None;
            }
            current = self.contract.reference_targets.get(&current)?.clone()?;
        }
    }
    pub fn string(&self, name: &str) -> Option<&'a str> {
        self.field(name)?.1.as_str()
    }
    pub fn boolean(&self, name: &str) -> Option<bool> {
        self.field(name)?.1.as_bool()
    }
    pub fn field_32(&self, name: &str) -> Option<(SourceId, &'a Value)> {
        let field = self.field(name)?;
        self.contract
            .openapi_version_at(&field.0)
            .starts_with("3.2.")
            .then_some(field)
    }
    pub fn style(&self) -> Option<ParameterStyle> {
        let (source, value) = self.field("style")?;
        let style = ParameterStyle::parse(value.as_str()?)?;
        (style != ParameterStyle::Cookie
            || self
                .contract
                .openapi_version_at(&source)
                .starts_with("3.2."))
        .then_some(style)
    }
    pub fn encoding_style(&self) -> Option<ParameterStyle> {
        if self.field("style").is_some() {
            self.style()
        } else if self.field("explode").is_some() || self.field("allowReserved").is_some() {
            Some(ParameterStyle::Form)
        } else {
            // With no explicit RFC6570 serialization fields, the Encoding
            // Object uses contentType. There is no active form-style default.
            None
        }
    }
    pub fn schema(&self) -> Option<Schema<'a>> {
        self.contract.schema(&self.field("schema")?.0)
    }
    pub fn resolved_source(&self) -> Option<SourceId> {
        let mut current = self.source.clone();
        let mut seen = HashSet::new();
        loop {
            if !seen.insert(current.clone()) {
                return None;
            }
            self.contract.source(&current)?;
            if !self.contract.reference_targets.contains_key(&current) {
                return Some(current);
            }
            current = self.contract.reference_targets.get(&current)?.clone()?;
        }
    }
    pub fn named(&self, key: &str) -> Vec<(&'a str, Self)> {
        let Some((source, value)) = self.field(key) else {
            return Vec::new();
        };
        value
            .as_object()
            .map(|map| {
                map.keys()
                    .map(|name| (name.as_str(), Self::new(self.contract, source.child(name))))
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn operations(&self) -> Vec<(HttpMethod<'a>, SourceId, &'a Value)> {
        let mut operations = Vec::new();
        for (field, method) in super::method::FIXED_METHODS {
            let value = if field == "query" {
                self.field_32(field)
            } else {
                self.field(field)
            };
            if let Some((source, raw)) = value {
                operations.push((HttpMethod::parse(method).unwrap(), source, raw));
            }
        }
        if let Some((source, value)) = self.field_32("additionalOperations")
            && let Some(map) = value.as_object()
        {
            for (token, raw) in map {
                if let Some(method) = super::method::additional_method(token) {
                    operations.push((method, source.child(token), raw));
                }
            }
        }
        operations
    }
}

impl Contract {
    /// Outgoing client operations under `paths`; callbacks and webhooks have
    /// separate collections and cannot silently become SDK client methods.
    pub fn operations(&self) -> impl Iterator<Item = Operation<'_>> {
        self.http
            .operations
            .iter()
            .filter(|n| n.kind == OperationKind::Client)
            .map(|node| Operation {
                contract: self,
                node,
            })
    }
    /// Operations describing provider-initiated webhooks.
    pub fn webhooks(&self) -> impl Iterator<Item = Operation<'_>> {
        self.http
            .operations
            .iter()
            .filter(|n| n.kind == OperationKind::Webhook)
            .map(|node| Operation {
                contract: self,
                node,
            })
    }
    /// Callback operations nested under client/webhook/callback operations.
    pub fn callbacks(&self) -> impl Iterator<Item = Operation<'_>> {
        self.http
            .operations
            .iter()
            .filter(|n| n.kind == OperationKind::Callback)
            .map(|node| Operation {
                contract: self,
                node,
            })
    }
}

/// A typed operation with source-preserving effective HTTP settings.
#[derive(Debug, Clone, Copy)]
pub struct Operation<'a> {
    contract: &'a Contract,
    node: &'a OperationNode,
}

impl<'a> Operation<'a> {
    pub(super) fn object(&self) -> Object<'a> {
        Object::new(self.contract, self.node.source.clone())
    }
    /// Source of the operation object itself.
    pub fn source(&self) -> &'a SourceId {
        &self.node.source
    }
    /// The operation object as written.
    pub fn raw(&self) -> &'a Value {
        self.contract
            .source(&self.node.source)
            .expect("operation exists")
    }
    /// HTTP method, including OpenAPI 3.2 `query` and `additionalOperations`.
    pub fn method(&self) -> HttpMethod<'a> {
        HttpMethod::parse(&self.node.method).expect("indexed methods are valid HTTP tokens")
    }
    /// Whether this is a client, webhook, or callback operation.
    pub fn kind(&self) -> OperationKind {
        self.node.kind
    }
    /// Path template (`/pets/{id}`); `Some` only for client operations.
    pub fn path_template(&self) -> Option<&'a str> {
        (self.node.kind == OperationKind::Client).then_some(&self.node.route)
    }
    /// Key under `webhooks`; `Some` only for webhook operations.
    pub fn webhook_name(&self) -> Option<&'a str> {
        (self.node.kind == OperationKind::Webhook).then_some(&self.node.route)
    }
    /// Runtime expression keying the callback path item; `Some` only for callbacks.
    pub fn callback_expression(&self) -> Option<&'a str> {
        (self.node.kind == OperationKind::Callback).then_some(&self.node.route)
    }
    /// Source of the path-item declaration, including a reference mount point.
    pub fn path_item_source(&self) -> &'a SourceId {
        &self.node.path_item
    }
    /// Operation containing this callback; distinct from the callback operation.
    pub fn callback_parent(&self) -> Option<&'a SourceId> {
        self.node.callback.as_ref().map(|(source, _)| source)
    }
    /// Key under the parent operation's `callbacks` map; `Some` only for callbacks.
    pub fn callback_name(&self) -> Option<&'a str> {
        self.node.callback.as_ref().map(|(_, name)| name.as_str())
    }
    /// Declared `operationId`, if any.
    pub fn operation_id(&self) -> Option<&'a str> {
        self.object().string("operationId")
    }
    /// Declared `summary`, if any.
    pub fn summary(&self) -> Option<&'a str> {
        self.object().string("summary")
    }
    /// Declared `description`, if any.
    pub fn description(&self) -> Option<&'a str> {
        self.object().string("description")
    }
    /// Declared `deprecated` flag; `None` when absent.
    pub fn deprecated(&self) -> Option<bool> {
        self.object().boolean("deprecated")
    }
    /// External documentation declared for this operation.
    pub fn external_docs(&self) -> Option<ExternalDocumentation<'a>> {
        Some(ExternalDocumentation {
            object: Object::new(self.contract, self.object().field("externalDocs")?.0),
        })
    }
    /// String entries of the operation's own `tags` array; non-strings are skipped.
    pub fn tags(&self) -> Vec<&'a str> {
        self.raw()
            .get("tags")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    }
    /// Path-item parameters with operation-level `(name, in)` overrides applied.
    /// A querystring parameter keeps its name for override identity even though
    /// the name does not participate in serialization.
    pub fn parameters(&self) -> Vec<Parameter<'a>> {
        self.node
            .parameters
            .iter()
            .map(|source| Parameter {
                object: Object::new(self.contract, source.clone()),
            })
            .collect()
    }
    /// Original path-item and operation parameter collection locations, before
    /// entry filtering/overrides. Canonical reference layering preserves the
    /// terminal source even when a malformed collection yields no parameters.
    pub fn parameter_collections(&self) -> Vec<SourceId> {
        let path_item = Object {
            contract: self.contract,
            source: self.node.path_item.clone(),
            path_item: true,
        };
        [
            path_item.field("parameters"),
            self.object().field("parameters"),
        ]
        .into_iter()
        .flatten()
        .map(|(source, _)| source)
        .collect()
    }
    /// Servers declared directly on the operation; `None` when it has no `servers`
    /// field (inherited servers come from [`Operation::effective_servers`]).
    pub fn declared_servers(&self) -> Option<Vec<Server<'a>>> {
        self.raw().get("servers").map(|_| {
            servers(
                self.contract,
                Some(self.node.source.child("servers")),
                false,
            )
        })
    }
    /// Security declared directly on the operation; `None` when it has no
    /// `security` field (inherited security comes from [`Operation::effective_security`]).
    pub fn declared_security(&self) -> Option<Vec<SecurityRequirement<'a>>> {
        self.raw()
            .get("security")
            .map(|_| security(self.contract, Some(self.node.source.child("security"))))
    }
    /// The operation/path-item/root array providing effective servers. An
    /// explicit empty array retains provenance and selects the OAS default `/`.
    pub fn server_source(&self) -> Option<&'a SourceId> {
        self.node.servers.as_ref()
    }
    /// The operation or root array providing effective security, if any.
    pub fn security_source(&self) -> Option<&'a SourceId> {
        self.node.security.as_ref()
    }
    /// Servers that apply to the operation after operation → path item → root
    /// inheritance, falling back to the OAS default `/` server.
    pub fn effective_servers(&self) -> Vec<Server<'a>> {
        servers(self.contract, self.node.servers.clone(), true)
    }
    /// Array entries are alternatives (OR); each requirement object's scheme
    /// members apply together (AND). Empty arrays disable inherited security.
    pub fn effective_security(&self) -> Vec<SecurityRequirement<'a>> {
        security(self.contract, self.node.security.clone())
    }
}

/// An operation/path-item parameter, including content-based serialization.
#[derive(Debug, Clone)]
pub struct Parameter<'a> {
    pub(super) object: Object<'a>,
}

impl<'a> Parameter<'a> {
    /// Source of the parameter declaration (the `$ref` site when referenced).
    pub fn source(&self) -> &SourceId {
        &self.object.source
    }
    /// Terminal reference target; the declaration remains available via source/raw.
    pub fn resolved_source(&self) -> Option<SourceId> {
        self.object.resolved_source()
    }
    /// The declaration as written, which may be a `$ref` object.
    pub fn raw(&self) -> &'a Value {
        self.object.raw()
    }
    /// Parameter `name`, following references.
    pub fn name(&self) -> Option<&'a str> {
        self.object.string("name")
    }
    /// Parameter `in`; `querystring` is only recognized in OpenAPI 3.2 documents.
    pub fn location(&self) -> Option<ParameterLocation> {
        let (source, value) = self.object.field("in")?;
        let location = ParameterLocation::parse(value.as_str()?)?;
        (location != ParameterLocation::Querystring
            || self
                .object
                .contract
                .openapi_version_at(&source)
                .starts_with("3.2."))
        .then_some(location)
    }
    /// Declared `description`, if any.
    pub fn description(&self) -> Option<&'a str> {
        self.object.string("description")
    }
    /// Declared `required` flag; `None` when absent.
    pub fn required(&self) -> Option<bool> {
        self.object.boolean("required")
    }
    /// Declared `deprecated` flag; `None` when absent.
    pub fn deprecated(&self) -> Option<bool> {
        self.object.boolean("deprecated")
    }
    /// Declared `allowReserved` flag; `None` when absent.
    pub fn allow_reserved(&self) -> Option<bool> {
        self.object.boolean("allowReserved")
    }
    /// Declared `allowEmptyValue` flag; `None` when absent.
    pub fn allow_empty_value(&self) -> Option<bool> {
        self.object.boolean("allowEmptyValue")
    }
    /// Declared `style`; `cookie` is only recognized in OpenAPI 3.2 documents.
    pub fn style(&self) -> Option<ParameterStyle> {
        self.object.style()
    }
    /// Declared `explode` flag; `None` when absent.
    pub fn explode(&self) -> Option<bool> {
        self.object.boolean("explode")
    }
    /// Style after applying the location default (`form` for query/cookie,
    /// `simple` for path/header). `None` for `content` and `querystring` parameters,
    /// which serialize through a media type instead.
    pub fn effective_style(&self) -> Option<ParameterStyle> {
        if self.object.field("content").is_some()
            || self.location() == Some(ParameterLocation::Querystring)
        {
            return None;
        }
        if self.object.field("style").is_some() {
            return self.style();
        }
        match self.location()? {
            ParameterLocation::Query | ParameterLocation::Cookie => Some(ParameterStyle::Form),
            ParameterLocation::Path | ParameterLocation::Header => Some(ParameterStyle::Simple),
            ParameterLocation::Querystring => None,
        }
    }
    /// Explode after applying the style default (`true` for `form`/`cookie`).
    /// `None` for `content` and `querystring` parameters.
    pub fn effective_explode(&self) -> Option<bool> {
        if self.object.field("content").is_some()
            || self.location() == Some(ParameterLocation::Querystring)
        {
            return None;
        }
        if self.object.field("explode").is_some() {
            self.explode()
        } else {
            self.effective_style()
                .map(|style| matches!(style, ParameterStyle::Form | ParameterStyle::Cookie))
        }
    }
    /// The parameter's `schema`, following references; `None` for `content` parameters.
    pub fn schema(&self) -> Option<Schema<'a>> {
        self.object.schema()
    }
}

/// One effective or declared server. Defaults have no invented source node.
#[derive(Debug, Clone)]
pub struct Server<'a> {
    pub(super) object: Option<Object<'a>>,
}
impl<'a> Server<'a> {
    /// Source of the server object; `None` for the synthesized default server.
    pub fn source(&self) -> Option<&SourceId> {
        self.object.as_ref().map(|o| &o.source)
    }
    /// The server object as written; `None` for the synthesized default server.
    pub fn raw(&self) -> Option<&'a Value> {
        self.object.as_ref().map(Object::raw)
    }
    /// True for the synthesized OAS default (`url: /`) used when no servers apply.
    pub fn is_default(&self) -> bool {
        self.object.is_none()
    }
    /// Server URL template; the synthesized default yields `/`.
    pub fn url(&self) -> Option<&'a str> {
        self.object.as_ref().map_or(Some("/"), |o| o.string("url"))
    }
    /// Declared `description`, if any.
    pub fn description(&self) -> Option<&'a str> {
        self.object.as_ref()?.string("description")
    }
    /// Optional server identifier introduced by OpenAPI 3.2.
    pub fn name(&self) -> Option<&'a str> {
        self.object.as_ref()?.field_32("name")?.1.as_str()
    }
    /// Declared `variables`, in document order.
    pub fn variables(&self) -> Vec<(&'a str, ServerVariable<'a>)> {
        let Some((source, value)) = self.object.as_ref().and_then(|o| o.field("variables")) else {
            return Vec::new();
        };
        value
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(name, _)| {
                        (
                            name.as_str(),
                            ServerVariable {
                                object: Object::new(
                                    self.object.as_ref().unwrap().contract,
                                    source.child(name),
                                ),
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// A server URL substitution variable, retaining default/enum/prose.
#[derive(Debug, Clone)]
pub struct ServerVariable<'a> {
    object: Object<'a>,
}
impl<'a> ServerVariable<'a> {
    /// Source of the server variable object.
    pub fn source(&self) -> &SourceId {
        &self.object.source
    }
    /// The server variable object as written.
    pub fn raw(&self) -> &'a Value {
        self.object.raw()
    }
    /// Declared `default` substitution value.
    pub fn default(&self) -> Option<&'a str> {
        self.object.string("default")
    }
    /// Declared `description`, if any.
    pub fn description(&self) -> Option<&'a str> {
        self.object.string("description")
    }
    /// String entries of the declared `enum`; `None` when `enum` is absent.
    pub fn values(&self) -> Option<Vec<&'a str>> {
        self.object
            .field("enum")?
            .1
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).collect())
    }
}

/// One alternative in a security array; its member schemes are conjunctive.
#[derive(Debug, Clone)]
pub struct SecurityRequirement<'a> {
    object: Object<'a>,
}
impl<'a> SecurityRequirement<'a> {
    /// Source of the requirement object within its security array.
    pub fn source(&self) -> &SourceId {
        &self.object.source
    }
    /// The requirement object as written.
    pub fn raw(&self) -> &'a Value {
        self.object.raw()
    }
    /// True for `{}` — the alternative that permits unauthenticated access.
    pub fn is_anonymous(&self) -> bool {
        self.raw().as_object().is_some_and(|o| o.is_empty())
    }
    /// The schemes in this requirement, all of which apply together.
    pub fn requirements(&self) -> Vec<SecurityUse<'a>> {
        self.raw()
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(name, scopes)| SecurityUse {
                        contract: self.object.contract,
                        name,
                        scopes,
                        source: self.object.source.child(name),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// A scheme name and scopes inside one conjunctive requirement.
#[derive(Debug, Clone)]
pub struct SecurityUse<'a> {
    pub(super) contract: &'a Contract,
    name: &'a str,
    scopes: &'a Value,
    source: SourceId,
}
impl<'a> SecurityUse<'a> {
    /// Source of this scheme's entry inside the requirement object.
    pub fn source(&self) -> &SourceId {
        &self.source
    }
    /// Security scheme name as written in the requirement.
    pub fn name(&self) -> &'a str {
        self.name
    }
    /// Required scope names; `None` when the value is not an array.
    pub fn scopes(&self) -> Option<Vec<&'a str>> {
        self.scopes
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).collect())
    }
}

/// A source-addressed link to external documentation.
#[derive(Debug, Clone)]
pub struct ExternalDocumentation<'a> {
    object: Object<'a>,
}

impl<'a> ExternalDocumentation<'a> {
    /// Source of the External Documentation object.
    pub fn source(&self) -> &SourceId {
        &self.object.source
    }
    /// The External Documentation object as written.
    pub fn raw(&self) -> &'a Value {
        self.object.raw()
    }
    /// Declared `url`.
    pub fn url(&self) -> Option<&'a str> {
        self.object.string("url")
    }
    /// Declared `description`, if any.
    pub fn description(&self) -> Option<&'a str> {
        self.object.string("description")
    }
}

fn servers(contract: &Contract, source: Option<SourceId>, default: bool) -> Vec<Server<'_>> {
    match source {
        None if default => vec![Server { object: None }],
        Some(source) => match contract.source(&source).and_then(Value::as_array) {
            Some(values) if values.is_empty() && default => vec![Server { object: None }],
            Some(values) => values
                .iter()
                .enumerate()
                .map(|(i, _)| Server {
                    object: Some(Object::new(contract, source.child(&i.to_string()))),
                })
                .collect(),
            None => Vec::new(),
        },
        None => Vec::new(),
    }
}

fn security(contract: &Contract, source: Option<SourceId>) -> Vec<SecurityRequirement<'_>> {
    let Some(source) = source else {
        return Vec::new();
    };
    contract
        .source(&source)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .enumerate()
                .map(|(i, _)| SecurityRequirement {
                    object: Object::new(contract, source.child(&i.to_string())),
                })
                .collect()
        })
        .unwrap_or_default()
}
