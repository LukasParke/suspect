//! Immutable logical-resource metadata over original physical source identities.

mod index;

use std::collections::{BTreeMap, BTreeSet};

use suspect_ref::resource_uri;

use super::{Contract, SchemaContext, SchemaId, SourceId};

/// Physical source boundary of a document or an embedded schema resource.
/// Canonical URI names and retrieval aliases are separate metadata.
pub type ResourceId = SourceId;

/// The object establishing a URI/fragment resolution boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    /// An external HTTP fragment document without an OpenAPI root declaration.
    Document,
    /// An OpenAPI document, optionally identified by OAS 3.2 `$self`.
    OpenApiDocument,
    /// A JSON Schema document root or a schema containing modern `$id`.
    Schema,
}

/// Whether a schema's plain-name identifier also declares a dynamic extension point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorKind {
    Static,
    Dynamic,
}

/// One original `$anchor` or `$dynamicAnchor` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaAnchor {
    name: String,
    source: SourceId,
    target: SchemaId,
    resource: ResourceId,
    kind: AnchorKind,
}

impl SchemaAnchor {
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Source of the identifying keyword, rather than the containing schema.
    pub fn source(&self) -> &SourceId {
        &self.source
    }
    pub fn target(&self) -> &SchemaId {
        &self.target
    }
    pub fn resource(&self) -> &ResourceId {
        &self.resource
    }
    pub fn kind(&self) -> AnchorKind {
        self.kind
    }
}

/// A physical resource and its logical names. URI strings use generic RFC 3986
/// syntax, including opaque schemes, instead of the retrieval URL type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    source: ResourceId,
    kind: ResourceKind,
    canonical_uri: String,
    base_uri: String,
    declaration_source: Option<SourceId>,
    aliases: Vec<String>,
    anchors: Vec<SchemaAnchor>,
}

impl Resource {
    pub fn source(&self) -> &ResourceId {
        &self.source
    }
    pub fn kind(&self) -> ResourceKind {
        self.kind
    }
    pub fn canonical_uri(&self) -> &str {
        &self.canonical_uri
    }
    /// Fragment-free absolute base used for URI-reference resolution.
    pub fn base_uri(&self) -> &str {
        &self.base_uri
    }
    /// Actual `$self` / `$id` value source, absent for retrieval-derived bases.
    pub fn declaration_source(&self) -> Option<&SourceId> {
        self.declaration_source.as_ref()
    }
    /// Accepted canonical/retrieval/requested names, independent of fetch authority.
    pub fn aliases(&self) -> &[String] {
        &self.aliases
    }
    pub fn anchors(&self) -> &[SchemaAnchor] {
        &self.anchors
    }
}

/// Lexical URI scope of a structurally registered object/schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceScope {
    resource: ResourceId,
    schema_root: Option<SchemaId>,
    base_uri: String,
    base_source: Option<SourceId>,
    address: String,
}

impl ResourceScope {
    pub fn resource(&self) -> &ResourceId {
        &self.resource
    }
    /// Nearest `$id` resource root, or the root of an embedded id-less schema.
    /// This does not turn an OpenAPI/HTTP document into a Schema Object.
    pub fn schema_root(&self) -> Option<&SchemaId> {
        self.schema_root.as_ref()
    }
    pub fn base_uri(&self) -> &str {
        &self.base_uri
    }
    pub fn base_source(&self) -> Option<&SourceId> {
        self.base_source.as_ref()
    }
    /// Absolute URI plus resource-relative encoded pointer, without synthetic nodes.
    pub fn address(&self) -> &str {
        &self.address
    }
    pub(super) fn at_source(mut self, source: &SourceId) -> Self {
        if source != &self.resource {
            self.address = index::address(&self, source);
        }
        self
    }
}

/// Static starting point and possible resource bindings of one `$dynamicRef`.
/// A consumer must retain the dynamic distinction; candidates are not runtime
/// selections and their declaration order does not establish dynamic scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicReference {
    source: SourceId,
    initial_target: Option<SchemaId>,
    initial_resource: Option<ResourceId>,
    dynamic_anchor: Option<String>,
    candidates: Vec<SchemaAnchor>,
}

impl DynamicReference {
    /// Original `$dynamicRef` keyword source.
    pub fn source(&self) -> &SourceId {
        &self.source
    }
    pub fn initial_target(&self) -> Option<&SchemaId> {
        self.initial_target.as_ref()
    }
    pub fn initial_resource(&self) -> Option<&ResourceId> {
        self.initial_resource.as_ref()
    }
    /// Matching dynamic name only when the initial URI used `$dynamicAnchor`.
    /// Pointer/empty fragments and ordinary anchors have no dynamic override.
    pub fn dynamic_anchor(&self) -> Option<&str> {
        self.dynamic_anchor.as_deref()
    }
    pub fn candidates(&self) -> &[SchemaAnchor] {
        &self.candidates
    }
}

/// A readonly logical URI lookup failed without loading or fetching anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceResolutionError {
    code: &'static str,
    message: String,
}

impl ResourceResolutionError {
    pub fn code(&self) -> &'static str {
        self.code
    }
    pub fn message(&self) -> &str {
        &self.message
    }
    pub(super) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ResourceResolutionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}
impl std::error::Error for ResourceResolutionError {}

#[derive(Debug, Default)]
pub(super) struct ResourceIndex {
    documents: BTreeMap<suspect_source::Uri, Option<ResourceScope>>,
    resources: BTreeMap<ResourceId, Vec<Resource>>,
    scopes: BTreeMap<SourceId, Vec<Option<ResourceScope>>>,
    aliases: BTreeMap<String, BTreeSet<ResourceId>>,
    dynamics: BTreeMap<SchemaId, DynamicReference>,
}

impl Contract {
    /// All registered resource declarations. Conflicting physical contexts remain
    /// inspectable as separate variants and have blocking diagnostics.
    pub fn resources(&self) -> impl Iterator<Item = &Resource> {
        self.resources.resources.values().flatten()
    }
    /// A resource with one unambiguous physical/context identity.
    pub fn resource(&self, id: &ResourceId) -> Option<&Resource> {
        match self.resources.resources.get(id)?.as_slice() {
            [resource] => Some(resource),
            _ => None,
        }
    }
    /// Unique lexical URI scope. Malformed or conflicting contexts return None.
    pub fn resource_scope(&self, source: &SourceId) -> Option<&ResourceScope> {
        self.resources.scope(source)
    }
    /// Resolve against a registered source's lexical base using only this
    /// snapshot. This API has no document-loading or acquisition capability.
    pub fn resolve_resource_reference(
        &self,
        source: &SourceId,
        reference: &str,
    ) -> Result<SourceId, ResourceResolutionError> {
        let scope = self.resource_scope(source).ok_or_else(|| {
            ResourceResolutionError::new(
                "invalid-resource-scope",
                format!(
                    "no unambiguous resource scope at {}#{}",
                    source.document(),
                    source.pointer()
                ),
            )
        })?;
        let absolute =
            resource_uri::resolve_reference(scope.base_uri(), reference).map_err(|error| {
                ResourceResolutionError::new("invalid-reference-uri", error.to_string())
            })?;
        self.resources.lookup(self, &absolute)
    }
    /// Metadata for the `$dynamicRef` on this containing schema, if declared.
    pub fn dynamic_reference(&self, schema: &SchemaId) -> Option<&DynamicReference> {
        self.resources.dynamics.get(schema)
    }
}

pub(super) fn supports_resources(context: &SchemaContext) -> bool {
    supports_dialect(&context.dialect)
}

pub(super) fn supports_dialect(dialect: &super::SchemaDialect) -> bool {
    matches!(dialect, super::SchemaDialect::Uri(uri) if matches!(uri.strip_suffix('#').unwrap_or(uri),
        "https://spec.openapis.org/oas/3.1/dialect/base" | "https://json-schema.org/draft/2020-12/schema"))
}
