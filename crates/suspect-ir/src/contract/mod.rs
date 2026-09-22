//! Lossless contract documents, a source-addressed schema graph, and HTTP views.
//!
//! Raw JSON is owned once per reachable document. Schema views borrow that
//! data; graph edges retain references instead of expanding recursive trees.
//! This is a compiler input model, not a language-specific type lowering.

mod compile;
mod http;
mod http_compile;
mod http_validate;
mod method;
mod payload;
mod reader;
mod resources;
mod security;
mod shape;
mod walk;

pub use http::*;
pub use method::HttpMethod;
pub use payload::*;
pub use reader::ContractReader;
pub use resources::*;
pub use security::*;
/// Pure RFC 3986 operations shared with the lower reference layer. No I/O.
pub use suspect_ref::resource_uri;

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::Arc;

use serde_json::Value;
use suspect_low::Pointer;
use suspect_ref::Workspace;
use suspect_source::Uri;

/// Stable retrieval URI plus JSON Pointer. Formatting and byte offsets do
/// not affect identity; different retrieval URIs intentionally identify
/// different source values, even when their contents match.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceId {
    document: Uri,
    pointer: String,
}

/// Schema identity uses the same stable source address as other contract objects.
pub type SchemaId = SourceId;

impl SourceId {
    /// Identifies a source value at an already decoded JSON Pointer.
    #[must_use]
    pub fn new(document: Uri, pointer: Pointer) -> Self {
        Self {
            document,
            pointer: pointer.to_path(),
        }
    }

    /// Canonical retrieval URI of the source document.
    #[must_use]
    pub fn document(&self) -> &Uri {
        &self.document
    }

    /// RFC 6901 pointer, without a URI fragment prefix.
    #[must_use]
    pub fn pointer(&self) -> &str {
        &self.pointer
    }

    /// Address of one child field or array index.
    #[must_use]
    pub fn child(&self, token: &str) -> Self {
        let token = token.replace('~', "~0").replace('/', "~1");
        Self {
            document: self.document.clone(),
            pointer: format!("{}/{token}", self.pointer),
        }
    }
}

/// A schema's declared or inherited vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaDialect {
    /// The OpenAPI 3.0 Schema Object subset.
    OpenApi30,
    /// A declared/default dialect URI; unsupported URIs are retained and
    /// reported by diagnostics instead of silently assuming 2020-12.
    Uri(String),
}

/// One direct reference edge. The original value is retained in raw JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaReference {
    /// Source keyword, `$ref` or `$dynamicRef`.
    pub keyword: String,
    /// Direct target when the address resolved. A target may itself refer
    /// back to the source; this is a graph, not an expanded schema tree. For
    /// `$dynamicRef` this is only the initial target: consumers must also use
    /// `Contract::dynamic_reference` and preserve runtime dynamic scope.
    pub target: Option<SchemaId>,
}

/// An explicit contract compilation limitation or invalid source feature.
#[derive(Debug, Clone)]
pub struct ContractDiagnostic {
    /// Source location of the affected value or containing object.
    pub source: SourceId,
    /// Source byte range, separate from stable identity.
    pub at: Range<usize>,
    /// Stable machine-readable code.
    pub code: &'static str,
    /// Whether consumers must block semantic lowering or may preserve an
    /// unrecognized annotation with a warning.
    pub severity: ContractSeverity,
    /// Human-readable reason; raw source data remains available.
    pub message: String,
}

/// Contract diagnostic severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractSeverity {
    /// Semantics cannot be faithfully interpreted by this compiler yet.
    Error,
    /// Raw annotation retained without interpretation.
    Warning,
}

/// A source document could not be faithfully materialized or the entry was
/// not a supported OpenAPI version.
#[derive(Debug, Clone)]
pub struct ContractError(pub String);

impl std::fmt::Display for ContractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ContractError {}

#[derive(Debug)]
struct Document {
    raw: Value,
    spans: BTreeMap<String, Range<usize>>,
}

#[derive(Debug)]
struct SchemaNode {
    id: SchemaId,
    span: Range<usize>,
    dialect: SchemaDialect,
    dialect_source: Option<SourceId>,
    default_dialect_source: Option<SourceId>,
    children: Vec<SchemaId>,
    references: Vec<SchemaReference>,
}

/// One effective embedding context and the declarations that supplied it.
/// Conflicting contexts are retained even when compilation must refuse a schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaContext {
    version: String,
    version_source: SourceId,
    dialect: SchemaDialect,
    dialect_source: Option<SourceId>,
    default_dialect_source: Option<SourceId>,
}

impl SchemaContext {
    /// Declaring OpenAPI feature-set version.
    pub fn openapi_version(&self) -> &str {
        &self.version
    }
    /// Actual `openapi` declaration supplying the embedding version.
    pub fn version_source(&self) -> &SourceId {
        &self.version_source
    }
    /// Effective dialect after lexical `$schema` overrides.
    pub fn dialect(&self) -> &SchemaDialect {
        &self.dialect
    }
    /// Effective `$schema` or `jsonSchemaDialect` declaration, if explicit.
    pub fn dialect_source(&self) -> Option<&SourceId> {
        self.dialect_source.as_ref()
    }
    /// The actual embedding OpenAPI default declaration, if explicit.
    pub fn default_dialect_source(&self) -> Option<&SourceId> {
        self.default_dialect_source.as_ref()
    }
}

/// An immutable, owned snapshot independent of workspace/document lifetimes.
#[derive(Debug)]
pub struct Contract {
    entry: Uri,
    openapi_version: String,
    documents: BTreeMap<Uri, Document>,
    schemas: BTreeMap<SchemaId, SchemaNode>,
    roots: Vec<SchemaId>,
    diagnostics: Vec<ContractDiagnostic>,
    reference_targets: BTreeMap<SourceId, Option<SourceId>>,
    source_versions: BTreeMap<SourceId, String>,
    schema_contexts: BTreeMap<SourceId, Vec<SchemaContext>>,
    resources: resources::ResourceIndex,
    http: http::HttpIndex,
}

impl Contract {
    /// Every distinct effective context encountered for a schema source.
    /// Equal dialects may retain different provenance without being ambiguous.
    pub fn schema_contexts(&self, source: &SourceId) -> &[SchemaContext] {
        self.schema_contexts.get(source).map_or(&[], Vec::as_slice)
    }

    /// Whether structural parsing established this address as a Schema Object,
    /// including lexical ancestors of a selected fragment that are not roots.
    /// An external HTTP object does not become a schema merely by being a document.
    pub fn is_schema_position(&self, source: &SourceId) -> bool {
        self.schema_contexts.contains_key(source) || self.schemas.contains_key(source)
    }
    /// Compiles only the requested entry's semantic reference closure.
    /// Unrelated workspace documents and `$ref` keys in instance data do
    /// not expand the closure. Unsupported semantics remain explicit diagnostics.
    ///
    /// # Errors
    /// Unsupported entry family, source loading/syntax/decoding failures,
    /// or source values that cannot be faithfully represented as JSON.
    pub fn from_workspace(workspace: &Arc<Workspace>, entry: &Uri) -> Result<Self, ContractError> {
        Self::from_workspace_with_reader(workspace, entry, ContractReader::Lossless)
    }

    /// Compiles with an explicit [`ContractReader`].
    ///
    /// [`ContractReader::Lossless`] is the historical default. [`ContractReader::Fast`]
    /// materializes normalized document *values* independently through
    /// `suspect_syntax::fast::try_parse_fast` and the exact core-scalar rules.
    /// Both readers use the same structural graph traversal and reference decoder.
    /// Document loading and source spans still use the lossless workspace
    /// sidecar, so no parse-speed or span improvement is claimed. Unsupported
    /// fast syntax declines with an explicit error and is never silently
    /// substituted with CST values. Graph identity, diagnostics, and default
    /// output are identical for both readers on accepted inputs.
    ///
    /// # Errors
    /// Same as [`Contract::from_workspace`]; in `Fast` mode also reader
    /// declines and (with `SUSPECT_CONTRACT_READER_VERIFY=1`) reported
    /// fast/lossless value divergence.
    pub fn from_workspace_with_reader(
        workspace: &Arc<Workspace>,
        entry: &Uri,
        reader: ContractReader,
    ) -> Result<Self, ContractError> {
        compile::compile(workspace, entry, reader)
    }

    /// Requested canonical retrieval URI.
    #[must_use]
    pub fn entry(&self) -> &Uri {
        &self.entry
    }

    /// Exact OpenAPI version declared by the entry.
    #[must_use]
    pub fn openapi_version(&self) -> &str {
        &self.openapi_version
    }

    /// OpenAPI feature set of the declaring document, or the inherited context
    /// of a referenced fragment with no document-level `openapi` declaration.
    /// Conflicting fragment contexts are diagnosed rather than silently merged.
    #[must_use]
    pub fn openapi_version_at(&self, source: &SourceId) -> &str {
        if let Some(version) = self
            .document(source.document())
            .and_then(|doc| doc.get("openapi"))
            .and_then(Value::as_str)
            .filter(|version| {
                version.starts_with("3.0.")
                    || version.starts_with("3.1.")
                    || version.starts_with("3.2.")
            })
        {
            return version;
        }
        let mut current = source.clone();
        loop {
            if let Some(version) = self.source_versions.get(&current) {
                return version;
            }
            let Some((parent, _)) = current.pointer.rsplit_once('/') else {
                return &self.openapi_version;
            };
            current.pointer = parent.to_owned();
        }
    }

    /// Raw normalized document data, owned exactly once by the contract.
    #[must_use]
    pub fn document(&self, uri: &Uri) -> Option<&Value> {
        self.documents.get(uri).map(|d| &d.raw)
    }

    /// Raw data at any stable source address.
    #[must_use]
    pub fn source(&self, source: &SourceId) -> Option<&Value> {
        self.documents
            .get(&source.document)?
            .raw
            .pointer(&source.pointer)
    }

    /// One direct reference edge at a semantic HTTP or Schema Object position.
    /// The input names the containing object, not its `$ref` field. Intermediate
    /// reference objects are preserved; absent, invalid and unsupported edges
    /// return `None` and invalid/unsupported edges have located diagnostics.
    #[must_use]
    pub fn reference_target(&self, source: &SourceId) -> Option<&SourceId> {
        self.reference_targets.get(source).and_then(Option::as_ref)
    }

    /// Original byte range of an indexed source value, including transport
    /// metadata and annotations that are not schemas.
    #[must_use]
    pub fn source_span(&self, location: &SourceId) -> Option<Range<usize>> {
        self.documents
            .get(&location.document)?
            .spans
            .get(&location.pointer)
            .cloned()
    }

    /// Reachable documents, sorted by canonical retrieval URI.
    pub fn documents(&self) -> impl Iterator<Item = (&Uri, &Value)> {
        self.documents.iter().map(|(uri, doc)| (uri, &doc.raw))
    }

    /// Schema roots declared by OpenAPI components and transport objects.
    #[must_use]
    pub fn schema_roots(&self) -> &[SchemaId] {
        &self.roots
    }

    /// A schema view borrowing its raw data from the document table.
    #[must_use]
    pub fn schema(&self, id: &SchemaId) -> Option<Schema<'_>> {
        let node = self.schemas.get(id)?;
        let raw = self.documents.get(&id.document)?.raw.pointer(&id.pointer)?;
        Some(Schema { node, raw })
    }

    /// All indexed schemas, including inline and referenced schemas.
    pub fn schemas(&self) -> impl Iterator<Item = Schema<'_>> {
        self.schemas.keys().filter_map(|id| self.schema(id))
    }

    /// Source-linked limitations and invalid references. Consumers must
    /// handle these before claiming complete semantic support.
    #[must_use]
    pub fn diagnostics(&self) -> &[ContractDiagnostic] {
        &self.diagnostics
    }

    /// Whether unsupported or invalid semantics block faithful lowering.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == ContractSeverity::Error)
    }

    /// Stable, finite schema closure through containment and direct reference
    /// edges. Unknown starting IDs are omitted; recursive schemas are visited once.
    #[must_use]
    pub fn reachable_from(&self, roots: &[SchemaId]) -> Vec<SchemaId> {
        let mut pending = roots.to_vec();
        let mut seen = BTreeSet::new();
        while let Some(id) = pending.pop() {
            let Some(schema) = self.schemas.get(&id) else {
                continue;
            };
            if !seen.insert(id) {
                continue;
            }
            pending.extend(schema.children.iter().cloned());
            pending.extend(schema.references.iter().filter_map(|r| r.target.clone()));
        }
        seen.into_iter().collect()
    }

    /// Effective schema closure for validation/native planning. OpenAPI 3.0
    /// Reference Objects follow only their reference; raw sibling trees remain
    /// inspectable through `reachable_from` but have no semantic effect.
    /// Dynamic references add initial targets and candidate bindings only from
    /// resource scopes encountered in this closure. This is a finite planning
    /// superset; evaluation still chooses a binding using its dynamic stack.
    #[must_use]
    pub fn effective_schema_closure(&self, roots: &[SchemaId]) -> Vec<SchemaId> {
        let mut pending = roots.to_vec();
        let mut seen = BTreeSet::new();
        let mut resources = BTreeSet::new();
        let mut dynamic = BTreeSet::new();
        loop {
            while let Some(id) = pending.pop() {
                let Some(schema) = self.schema(&id) else {
                    continue;
                };
                if !seen.insert(id.clone()) {
                    continue;
                }
                if let Some(scope) = self.resource_scope(&id) {
                    resources.insert(scope.resource().clone());
                }
                if !schema.ignores_ref_siblings() {
                    pending.extend(schema.children().iter().cloned());
                    if self.dynamic_reference(&id).is_some() {
                        dynamic.insert(id.clone());
                    }
                }
                pending.extend(
                    schema
                        .references()
                        .iter()
                        .filter_map(|reference| reference.target.clone()),
                );
            }
            for id in &dynamic {
                for candidate in self
                    .dynamic_reference(id)
                    .into_iter()
                    .flat_map(|reference| reference.candidates())
                {
                    if resources.contains(candidate.resource())
                        && !seen.contains(candidate.target())
                    {
                        pending.push(candidate.target().clone());
                    }
                }
            }
            if pending.is_empty() {
                break;
            }
        }
        seen.into_iter().collect()
    }

    /// Whether a finding applies to an effective schema closure. Only known
    /// raw-keyword findings are ignored on a 3.0 reference. Structural/dialect
    /// ambiguity stays blocking, including for descendants of the affected node.
    #[must_use]
    pub fn schema_diagnostic_applies(
        &self,
        closure: &[SchemaId],
        diagnostic: &ContractDiagnostic,
    ) -> bool {
        if self
            .schema(&diagnostic.source)
            .is_some_and(|schema| schema.ignores_ref_siblings())
            && matches!(
                diagnostic.code,
                "invalid-schema"
                    | "unsupported-schema-dialect"
                    | "unsupported-schema-keyword"
                    | "unsupported-dynamic-reference"
                    | "unsupported-schema-resource"
                    | "unsupported-schema-vocabulary"
                    | "unknown-schema-keyword"
            )
        {
            return false;
        }
        let inherited = matches!(
            diagnostic.code,
            "AMBIGUOUS_OPENAPI_CONTEXT"
                | "unsupported-schema-resource"
                | "unsupported-schema-vocabulary"
                | "invalid-schema-resource"
                | "invalid-resource-identity"
                | "invalid-resource-uri"
        ) || diagnostic.code == "unsupported-dynamic-reference"
            && self.source(&diagnostic.source).is_some_and(|value| {
                value.get("$dynamicAnchor").is_some() || value.get("$recursiveAnchor").is_some()
            });
        closure.iter().any(|id| {
            id == &diagnostic.source
                || matches!(
                    diagnostic.code,
                    "invalid-openapi-self" | "invalid-resource-identity"
                ) && diagnostic.source.pointer() == "/$self"
                    && id.document() == diagnostic.source.document()
                || inherited
                    && id.document() == diagnostic.source.document()
                    && id
                        .pointer()
                        .strip_prefix(diagnostic.source.pointer())
                        .is_some_and(|tail| tail.starts_with('/'))
        })
    }
}

/// A lightweight view of indexed metadata and the original schema data.
#[derive(Debug, Clone, Copy)]
pub struct Schema<'a> {
    node: &'a SchemaNode,
    raw: &'a Value,
}

impl<'a> Schema<'a> {
    /// Stable source identity.
    #[must_use]
    pub fn id(&self) -> &'a SchemaId {
        &self.node.id
    }

    /// Entire original schema value; absent keys stay absent, null stays null,
    /// and unions, constraints, discriminator mappings, and annotations remain intact.
    #[must_use]
    pub fn raw(&self) -> &'a Value {
        self.raw
    }

    /// Original source byte range.
    #[must_use]
    pub fn span(&self) -> Range<usize> {
        self.node.span.clone()
    }

    /// Declared or inherited schema dialect.
    #[must_use]
    pub fn dialect(&self) -> &'a SchemaDialect {
        &self.node.dialect
    }

    /// Original declaration of the effective schema dialect, if explicit.
    pub fn dialect_source(&self) -> Option<&'a SourceId> {
        self.node.dialect_source.as_ref()
    }

    /// Original OpenAPI default declaration supplying this schema's context.
    pub fn default_dialect_source(&self) -> Option<&'a SourceId> {
        self.node.default_dialect_source.as_ref()
    }

    /// OpenAPI 3.0 Reference Objects do not apply siblings of `$ref`.
    #[must_use]
    pub fn ignores_ref_siblings(&self) -> bool {
        matches!(self.dialect(), SchemaDialect::OpenApi30) && self.raw().get("$ref").is_some()
    }

    /// Direct schema children under standard applicator keywords.
    #[must_use]
    pub fn children(&self) -> &'a [SchemaId] {
        &self.node.children
    }

    /// Direct reference edges, preserving recursive structure.
    #[must_use]
    pub fn references(&self) -> &'a [SchemaReference] {
        &self.node.references
    }
}
