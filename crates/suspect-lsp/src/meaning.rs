//! The per-node semantic model: what a cursor position *means*.
//!
//! Every editor feature used to answer "what is this position" with its
//! own walk over the tree — hover re-derived it, completion re-derived it,
//! code actions re-derived it, inlay hints re-derived it. That is why two
//! features could disagree, and why adding a feature meant writing another
//! heuristic instead of a query.
//!
//! This module answers the question once, per position, from the syntax
//! tree: which OpenAPI object contains the cursor, which logical path it
//! sits at, which dialect governs it, whether it is inside a Schema
//! Object, and — for a `$ref` — what it resolves to. Features become
//! queries against the model, and the model is directly testable.
//!
//! ```text
//! position ──▶ Model::at(offset) ──▶ Meaning { kind, pointer, dialect,
//!                                        in_schema, ref_target }
//! ```

use std::collections::BTreeMap;
use std::ops::Range;

use suspect_low::{LowDoc, NodeRef, Pointer};
use suspect_syntax::SNode;
use suspect_syntax::SyntaxKind;

/// Which OpenAPI (or Arazzo, or Overlay) object contains a position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    /// The document root.
    Root,
    /// `info`.
    Info,
    /// `servers[]`.
    Server,
    /// The `paths` map.
    Paths,
    /// One `paths./…` path item.
    PathItem,
    /// One operation under a path item.
    Operation,
    /// `operation.parameters[]`.
    Parameter,
    /// `operation.requestBody`.
    RequestBody,
    /// `operation.responses`.
    Responses,
    /// One response under `responses`.
    Response,
    /// One media type under a request or response body.
    MediaType,
    /// A JSON Schema position (any dialect).
    Schema,
    /// `components.schemas` and friends.
    Components,
    /// `webhooks` (OAS 3.1+).
    Webhooks,
    /// `securitySchemes[]`.
    SecurityScheme,
    /// An Arazzo workflow.
    Workflow,
    /// An Arazzo step.
    Step,
    /// An Overlay action.
    OverlayAction,
    /// A key, value, or anything not otherwise classified.
    Other,
}

impl ObjectKind {
    /// A human label for hover and diagnostics.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Root => "document",
            Self::Info => "Info Object",
            Self::Server => "Server Object",
            Self::Paths => "paths",
            Self::PathItem => "Path Item Object",
            Self::Operation => "Operation",
            Self::Parameter => "Parameter Object",
            Self::RequestBody => "Request Body Object",
            Self::Responses => "responses",
            Self::Response => "Response Object",
            Self::MediaType => "Media Type Object",
            Self::Schema => "Schema Object",
            Self::Components => "components",
            Self::Webhooks => "webhooks",
            Self::SecurityScheme => "Security Scheme Object",
            Self::Workflow => "Workflow",
            Self::Step => "Step Object",
            Self::OverlayAction => "Action Object",
            Self::Other => "value",
        }
    }
}

/// The specification family governing a position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// `openapi: 3.0.x`.
    Oas30,
    /// `openapi: 3.1.x`.
    Oas31,
    /// `openapi: 3.2.x`.
    Oas32,
    /// An Arazzo description.
    Arazzo,
    /// An Overlay document.
    Overlay,
    /// Not one of the above.
    Unknown,
}

impl Dialect {
    /// Reads the dialect from the document root.
    #[must_use]
    pub fn of(low: &LowDoc) -> Self {
        let root = low.root();
        let text = |key: &str| root.get(key).and_then(|n| n.as_str()).map(str::to_owned);
        if let Some(version) = text("openapi") {
            return if version.starts_with("3.0") {
                Self::Oas30
            } else if version.starts_with("3.1") {
                Self::Oas31
            } else if version.starts_with("3.2") {
                Self::Oas32
            } else {
                Self::Unknown
            };
        }
        match low.sniff_family() {
            suspect_low::SpecFamily::Arazzo10 => Self::Arazzo,
            suspect_low::SpecFamily::Overlay10 => Self::Overlay,
            _ => Self::Unknown,
        }
    }

    /// A human label.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Oas30 => "OpenAPI 3.0",
            Self::Oas31 => "OpenAPI 3.1",
            Self::Oas32 => "OpenAPI 3.2",
            Self::Arazzo => "Arazzo",
            Self::Overlay => "Overlay",
            Self::Unknown => "unknown",
        }
    }
}

/// A resolved `$ref` target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefTarget {
    /// The target document's URI.
    pub document: suspect_source::Uri,
    /// The pointer inside that document.
    pub pointer: Pointer,
}

/// What a position means.
#[derive(Debug, Clone, PartialEq)]
pub struct Meaning {
    /// The innermost object containing the position.
    pub kind: ObjectKind,
    /// The logical path of that object from the document root.
    pub pointer: Pointer,
    /// The dialect governing the position.
    pub dialect: Dialect,
    /// Whether the position is inside a Schema Object.
    pub in_schema: bool,
    /// The `$ref` this position is inside, when it is inside one.
    pub ref_target: Option<RefTarget>,
    /// The byte range of the innermost node.
    pub range: Range<usize>,
    /// The ancestor chain, outermost first, as `(kind, name)` pairs — the
    /// trail a hover or a diagnostic can narrate.
    pub trail: Vec<(ObjectKind, String)>,
}

impl Meaning {
    /// Whether the position sits inside the named object kind.
    #[must_use]
    pub fn within(&self, kind: ObjectKind) -> bool {
        self.trail.iter().any(|(found, _)| *found == kind)
    }
}

/// A per-document semantic model.
///
/// Cheap to build for a single position; build once per cursor move and
/// query repeatedly, or cache per document generation.
pub struct Model<'d> {
    low: &'d LowDoc,
    dialect: Dialect,
}

impl<'d> Model<'d> {
    /// Wraps a document.
    #[must_use]
    pub fn new(low: &'d LowDoc) -> Self {
        Self {
            dialect: Dialect::of(low),
            low,
        }
    }

    /// The document's dialect.
    #[must_use]
    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    /// What the position at `offset` means.
    #[must_use]
    pub fn at(&self, offset: usize) -> Option<Meaning> {
        let node = node_at(self.low, offset)?;
        let range = node.byte_range();
        // The logical pointer is composed from the syntax ancestors, which
        // is pointer-chasing: deriving it by descending through the
        // low-level tree allocates a vector of entries per level, and the
        // latency gate measured that as the dominant cost of a cursor
        // position.
        let mut keys: Vec<&'d str> = Vec::new();
        let mut cursor = Some(node);
        while let Some(current) = cursor {
            if let Some(key) = key_of_pair(&current) {
                keys.push(key);
            }
            cursor = current.parent();
        }
        keys.reverse();
        let mut pointer = Pointer::root();
        for key in &keys {
            pointer = pointer.push(key);
        }
        let (kind, in_schema) = classify_pointer(&pointer, self.dialect);
        // The trail is derived from the same pointer as `kind` and
        // `in_schema`, so the narration can never disagree with the
        // classification — the disagreement this module exists to remove.
        let trail = trail_from_pointer(&pointer, self.dialect);
        Some(Meaning {
            kind,
            pointer,
            dialect: self.dialect,
            in_schema,
            ref_target: self.ref_target(offset),
            range,
            trail,
        })
    }

    /// The `$ref` target the position sits inside, resolved against the
    /// workspace when one is supplied.
    #[must_use]
    pub fn ref_target(&self, offset: usize) -> Option<RefTarget> {
        let value = crate::navigation::ref_value_node(self.low, offset)?;
        let text = String::from_utf8_lossy(value.scalar_bytes()).into_owned();
        parse_ref(&text)
    }

    /// The nearest enclosing object node, whatever its position: a cursor
    /// on a property key is inside the object that declares it.
    #[must_use]
    pub fn object_ancestor(&self, offset: usize) -> Option<NodeRef<'d>> {
        let mut cursor = node_at(self.low, offset);
        while let Some(current) = cursor {
            if current.kind() == suspect_syntax::SyntaxKind::Mapping {
                return Some(NodeRef::new(current));
            }
            cursor = current.parent();
        }
        None
    }

    /// The nearest enclosing schema node, for completions and hovers that
    /// need the schema a position is being written inside.
    #[must_use]
    pub fn enclosing_schema(&self, offset: usize) -> Option<NodeRef<'d>> {
        let mut cursor = node_at(self.low, offset);
        while let Some(current) = cursor {
            if classify(&current).is_some_and(|(kind, _)| kind == ObjectKind::Schema) {
                return Some(NodeRef::new(current));
            }
            cursor = current.parent();
        }
        None
    }
}

/// Splits a `$ref` into a document and pointer.
#[must_use]
pub fn parse_ref(text: &str) -> Option<RefTarget> {
    let (document, pointer) = match text.split_once('#') {
        Some((document, pointer)) => (document, pointer),
        None => (text, ""),
    };
    let pointer = suspect_low::Pointer::parse(pointer).ok()?;
    Some(RefTarget {
        document: suspect_source::Uri::parse(if document.is_empty() {
            "mem://self"
        } else {
            document
        })
        .ok()?,
        pointer,
    })
}

/// The deepest node containing `offset`.
///
/// Reuses the server's cursor probe rather than re-deriving it: two
/// implementations of "which node is the cursor in" is exactly the
/// disagreement this module exists to remove.
fn node_at(low: &LowDoc, offset: usize) -> Option<SNode<'_>> {
    crate::navigation::node_at(low, offset)
}

/// Classifies one syntax node, with the key that led into it.
///
/// Allocation-free: this runs for every ancestor of every cursor
/// position, and a keystroke pays it every time. The key borrows from the
/// syntax node rather than being copied.
fn classify<'d>(node: &SNode<'d>) -> Option<(ObjectKind, &'d str)> {
    if node.kind() == SyntaxKind::Pair {
        let key = key_of(node)?;
        return classify_key(key).map(|kind| (kind, key));
    }
    // A mapping or sequence: classify by what its parent key says.
    let parent = node.parent()?;
    if parent.kind() == SyntaxKind::Pair {
        let key = key_of(&parent)?;
        return classify_key(key).map(|kind| (kind, key));
    }
    None
}

/// The mapping key of the pair a node is inside, when the node is a
/// mapping or sequence (the thing a pointer segment names).
fn key_of_pair<'d>(node: &SNode<'d>) -> Option<&'d str> {
    let parent = node.parent()?;
    if parent.kind() == SyntaxKind::Pair {
        return key_of(&parent);
    }
    // A sequence item is addressed by index.
    if parent.kind() == SyntaxKind::Sequence {
        let index = parent
            .children()
            .filter(|child| child.raw().is_named())
            .position(|child| child.byte_range() == node.byte_range())?;
        return Some(index.to_string().leak());
    }
    None
}

/// The mapping key of a `Pair`, borrowed.
fn key_of<'d>(pair: &SNode<'d>) -> Option<&'d str> {
    let key = pair.child_by_field("key")?;
    std::str::from_utf8(key.scalar_bytes()).ok()
}

/// The trail a position sits on, derived from its logical path: the kind
/// each segment introduces, with the segment's own name.
///
/// One source of truth with [`classify_pointer`]: a feature can read
/// `kind`, `in_schema` and `trail` without any of them disagreeing.
#[must_use]
pub fn trail_from_pointer(pointer: &Pointer, dialect: Dialect) -> Vec<(ObjectKind, String)> {
    let mut out: Vec<(ObjectKind, String)> = Vec::new();
    let mut walked = Pointer::root();
    for token in pointer.tokens() {
        walked = walked.push(token);
        let (kind, _) = classify_pointer(&walked, dialect);
        out.push((kind, token.to_string()));
    }
    out
}

/// The keys whose *value* is a Schema Object, wherever they appear.
const SCHEMA_KEYS: &[&str] = &[
    "schema",
    "items",
    "allOf",
    "anyOf",
    "oneOf",
    "not",
    "additionalProperties",
    "contains",
    "propertyNames",
    "unevaluatedProperties",
    "unevaluatedItems",
    "if",
    "then",
    "else",
    "$defs",
    "definitions",
    "dependentSchemas",
    "patternProperties",
    "properties",
    "prefixItems",
];

/// The HTTP methods that make a path item an operation.
const METHODS: &[&str] = &[
    "get", "put", "post", "delete", "options", "head", "patch", "trace", "query",
];

/// What a logical path means.
///
/// Positions the keys alone cannot express are visible here:
/// `components/schemas/Pet` and every entry under `properties` are schema
/// positions however they are named. Schema-ness is decided by order — a
/// schema keyword after the operation wins, one before it loses — so an
/// operation's parameter schemas are schemas while its `responses` are not.
#[must_use]
pub fn classify_pointer(pointer: &Pointer, dialect: Dialect) -> (ObjectKind, bool) {
    let tokens: Vec<String> = pointer
        .tokens()
        .iter()
        .map(|token| token.to_string())
        .collect();
    let mut schema_at: Option<usize> = None;
    let mut operation_at: Option<usize> = None;
    let mut kind = ObjectKind::Other;

    for (index, token) in tokens.iter().enumerate() {
        let opens_schema = SCHEMA_KEYS.contains(&token.as_str())
            || (token == "schemas" && tokens.first().is_some_and(|first| first == "components"));
        if opens_schema {
            schema_at = Some(index);
        }
        if tokens.first().is_some_and(|first| first == "paths")
            && index >= 2
            && METHODS.contains(&token.as_str())
        {
            operation_at = Some(index);
        }
        match token.as_str() {
            "paths" => kind = ObjectKind::Paths,
            "servers" => kind = ObjectKind::Server,
            "webhooks" => kind = ObjectKind::Webhooks,
            "info" => kind = ObjectKind::Info,
            "components" => kind = ObjectKind::Components,
            "requestBody" => kind = ObjectKind::RequestBody,
            "responses" => kind = ObjectKind::Responses,
            "securitySchemes" => kind = ObjectKind::SecurityScheme,
            "content" => kind = ObjectKind::MediaType,
            "workflows" | "workflowId" => kind = ObjectKind::Workflow,
            "steps" | "stepId" => kind = ObjectKind::Step,
            "actions" => kind = ObjectKind::OverlayAction,
            "target" if dialect == Dialect::Overlay => kind = ObjectKind::OverlayAction,
            _ => {}
        }
    }

    let in_schema = schema_at > operation_at;
    if in_schema {
        return (ObjectKind::Schema, true);
    }
    if operation_at.is_some() {
        return (ObjectKind::Operation, false);
    }
    // Inside `paths` but above a method: a path item. Inside `responses`
    // but below it: a response.
    if let Some(index) = tokens.iter().rposition(|token| token == "paths")
        && (index + 1 == tokens.len() - 1
            || (index + 2 == tokens.len() && !METHODS.contains(&tokens[index + 1].as_str())))
    {
        return (ObjectKind::PathItem, false);
    }
    if let Some(index) = tokens.iter().rposition(|token| token == "responses")
        && index + 1 < tokens.len()
    {
        return (ObjectKind::Response, false);
    }
    if tokens.is_empty() {
        return (ObjectKind::Root, false);
    }
    // A parameter is the entry under `parameters` outside a schema.
    for (index, token) in tokens.iter().enumerate() {
        if token == "parameters"
            && !tokens
                .first()
                .is_some_and(|first| first == "paths" || first == "components")
        {
            kind = ObjectKind::Parameter;
            let _ = index;
        }
    }
    (kind, false)
}

/// Maps a key name to the object kind it introduces.
fn classify_key(key: &str) -> Option<ObjectKind> {
    Some(match key {
        "info" => ObjectKind::Info,
        "servers" | "server" => ObjectKind::Server,
        "paths" => ObjectKind::Paths,
        "webhooks" => ObjectKind::Webhooks,
        "components" => ObjectKind::Components,
        "securitySchemes" => ObjectKind::SecurityScheme,
        "parameters" => ObjectKind::Parameter,
        "requestBody" => ObjectKind::RequestBody,
        "responses" => ObjectKind::Responses,
        "schema"
        | "items"
        | "properties"
        | "allOf"
        | "anyOf"
        | "oneOf"
        | "not"
        | "additionalProperties"
        | "prefixItems"
        | "contains"
        | "if"
        | "then"
        | "else"
        | "propertyNames"
        | "unevaluatedProperties"
        | "unevaluatedItems"
        | "$defs"
        | "definitions"
        | "dependentSchemas"
        | "patternProperties" => ObjectKind::Schema,
        "workflows" | "workflowId" => ObjectKind::Workflow,
        "steps" | "stepId" => ObjectKind::Step,
        "actions" | "target" => ObjectKind::OverlayAction,
        "content" => ObjectKind::MediaType,
        _ => return None,
    })
}

/// A reverse index: which nodes reference which, across a workspace.
///
/// Built once per workspace and reused by change-impact analysis,
/// "find references" on a definition, and rename.
#[derive(Debug, Clone, Default)]
pub struct Index {
    /// Target pointer (as `uri#pointer`) → the source positions that
    /// reference it.
    inbound: BTreeMap<String, Vec<Reference>>,
    /// Every `$ref` in the workspace, in document order.
    references: Vec<Reference>,
}

/// One `$ref` occurrence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// The document holding the reference.
    pub document: String,
    /// Where the reference's `$ref` value sits.
    pub range: Range<usize>,
    /// The pointer of the node carrying the reference.
    pub source: Pointer,
    /// The raw `$ref` text.
    pub text: String,
}

impl Index {
    /// Indexes every `$ref` in a workspace.
    #[must_use]
    pub fn build(ws: &suspect_ref::Workspace) -> Self {
        let mut index = Index::default();
        for uri in ws.uris() {
            let Some(handle) = ws.get(&uri) else {
                continue;
            };
            index.index_document(uri.as_str(), handle.doc());
        }
        index
    }

    /// Indexes one document's references.
    pub fn index_document(&mut self, document: &str, low: &LowDoc) {
        let mut pending = vec![low.root()];
        while let Some(node) = pending.pop() {
            if node.kind() == suspect_low::ValueKind::Object {
                for entry in node.entries() {
                    if entry.key == "$ref"
                        && let Some(value) = entry.value
                    {
                        let text = String::from_utf8_lossy(value.scalar_bytes()).into_owned();
                        let reference = Reference {
                            document: document.to_owned(),
                            range: value.byte_range(),
                            source: node.path_from_root(),
                            text: text.clone(),
                        };
                        if let Some(target) = parse_ref(&text) {
                            // A local reference resolves inside the referring
                            // document; only a relative path names another.
                            let target_document = if text.starts_with('#') {
                                document
                            } else {
                                target.document.as_str()
                            };
                            let key = format!("{target_document}#{}", target.pointer.to_path());
                            self.inbound.entry(key).or_default().push(reference.clone());
                        }
                        self.references.push(reference);
                    }
                    if let Some(child) = entry.value {
                        pending.push(child);
                    }
                }
            } else if node.kind() == suspect_low::ValueKind::Array {
                pending.extend(node.items());
            }
        }
    }

    /// Every reference in the workspace, in discovery order.
    #[must_use]
    pub fn references(&self) -> &[Reference] {
        &self.references
    }

    /// The references pointing at `uri#pointer`.
    #[must_use]
    pub fn inbound(&self, uri: &str, pointer: &Pointer) -> &[Reference] {
        self.inbound
            .get(&format!("{uri}#{}", pointer.to_path()))
            .map_or(&[], Vec::as_slice)
    }

    /// The references pointing at a composite `uri#pointer` key, as the
    /// impact analysis walks the graph.
    #[must_use]
    pub fn references_to(&self, key: &str) -> &[Reference] {
        self.inbound.get(key).map_or(&[], Vec::as_slice)
    }
}

#[cfg(test)]
#[path = "meaning_tests.rs"]
mod meaning_tests;
