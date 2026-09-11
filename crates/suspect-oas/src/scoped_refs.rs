//! OpenAPI reference addressing. Generic reference scanning remains available
//! in suspect-ref; schema anchors here are indexed only at schema positions.

use std::collections::{HashMap, HashSet};

use suspect_low::{NodeRef, Pointer, ValueKind};
use suspect_ref::{
    DocId, ParsedRef, RefError, ReferenceTarget, Resolution, Step, WorkspaceError, parse_ref,
};

use crate::{CycleGuard, OasVersion, OpenApi, SchemaView, Session};

#[derive(Default)]
pub(crate) struct ReferenceRegistry {
    documents: HashMap<DocId, SchemaIndex>,
}

#[derive(Default)]
struct SchemaIndex {
    identifiers: bool,
    seen: HashSet<(usize, usize, usize)>,
    anchors: HashMap<String, Vec<Pointer>>,
    ids: HashMap<Pointer, String>,
}

impl Session {
    /// Resolve one OpenAPI/schema reference using anchors declared at schema
    /// positions. Instance data in examples/defaults/const/enum cannot supply
    /// or shadow an anchor. The returned address preserves direct graph edges.
    ///
    /// # Errors
    /// Malformed refs, missing/ambiguous schema anchors, missing targets, or
    /// denied external documents. Unsupported contexts never use generic anchors.
    pub fn reference_target(&self, value: NodeRef<'_>) -> Result<ReferenceTarget, RefError> {
        let uri = value.syntax().doc().uri();
        let document = self
            .workspace()
            .get(uri)
            .ok_or_else(|| RefError::MissingDoc {
                uri: uri.to_string(),
            })?;
        let input = document.reference_input(value)?;
        self.ensure_schema_index(input.doc)?;
        let mut base = uri.clone();
        {
            let registry = self
                .reference_registry
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let index = &registry.documents[&input.doc];
            let tokens = input.containing.tokens();
            for len in 0..=tokens.len() {
                let pointer = Pointer::from_tokens(tokens[..len].to_vec());
                if let Some(id) = index.ids.get(&pointer) {
                    base = base.join(id).map_err(|error| RefError::InvalidRef {
                        raw: input.raw.clone(),
                        reason: format!("cannot join schema $id {id:?}: {error}"),
                    })?;
                }
            }
        }
        let parsed = match parse_ref(&base, &input.raw)? {
            ParsedRef::Local(pointer) if base != *uri => ParsedRef::External { uri: base, pointer },
            ParsedRef::PlainName(name) if base != *uri => {
                ParsedRef::ExternalAnchor { uri: base, name }
            }
            parsed => parsed,
        };
        match parsed {
            ParsedRef::PlainName(name) => self.anchor_target(input.doc, &name, &input.raw),
            ParsedRef::ExternalAnchor { uri, name } => {
                let target = self
                    .workspace()
                    .open(uri.as_str())
                    .map_err(workspace_error)?;
                self.anchor_target(target.id(), &name, &input.raw)
            }
            ParsedRef::Local(pointer) => {
                document.node_at_pointer(&pointer)?;
                Ok(ReferenceTarget {
                    doc: input.doc,
                    pointer,
                })
            }
            ParsedRef::External { uri, pointer } => {
                let target = self
                    .workspace()
                    .open(uri.as_str())
                    .map_err(workspace_error)?;
                target.node_at_pointer(&pointer)?;
                Ok(ReferenceTarget {
                    doc: target.id(),
                    pointer,
                })
            }
        }
    }

    fn anchor_target(
        &self,
        doc: DocId,
        name: &str,
        raw: &str,
    ) -> Result<ReferenceTarget, RefError> {
        self.ensure_schema_index(doc)?;
        let registry = self
            .reference_registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let targets = registry
            .documents
            .get(&doc)
            .and_then(|index| index.anchors.get(name));
        let pointer = match targets.map(Vec::as_slice) {
            Some([pointer]) => pointer.clone(),
            Some(_) => {
                return Err(RefError::InvalidRef {
                    raw: raw.to_owned(),
                    reason: format!("schema anchor #{name} is ambiguous in the target document"),
                });
            }
            None => {
                return Err(RefError::InvalidRef {
                    raw: raw.to_owned(),
                    reason: format!("#{name} matches no anchor declared at a schema position"),
                });
            }
        };
        Ok(ReferenceTarget { doc, pointer })
    }

    fn ensure_schema_index(&self, doc: DocId) -> Result<(), RefError> {
        let mut registry = self
            .reference_registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if registry.documents.contains_key(&doc) {
            return Ok(());
        }
        let document = self
            .workspace()
            .get_by_id(doc)
            .ok_or_else(|| RefError::MissingDoc {
                uri: format!("doc #{doc}"),
            })?;
        let version = OasVersion::sniff(document.doc());
        let roots = match version {
            Some(version) => {
                OpenApi::new(self, version, doc, document.doc().root()).declared_schema_roots()
            }
            None => vec![SchemaView::new(self, document.doc().root())],
        };
        let mut index = SchemaIndex {
            identifiers: version != Some(OasVersion::V30),
            ..SchemaIndex::default()
        };
        index.add(roots);
        registry.documents.insert(doc, index);
        Ok(())
    }

    pub(crate) fn register_schema(&self, node: NodeRef<'_>) {
        let Some(document) = self.workspace().get(node.syntax().doc().uri()) else {
            return;
        };
        if self.ensure_schema_index(document.id()).is_err() {
            return;
        }
        self.reference_registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .documents
            .get_mut(&document.id())
            .expect("schema index exists")
            .add(vec![SchemaView::new(self, node)]);
    }

    pub(crate) fn schema_ancestors<'s>(&'s self, node: NodeRef<'s>) -> Vec<NodeRef<'s>> {
        self.register_schema(node);
        let Some(document) = self.workspace().get(node.syntax().doc().uri()) else {
            return Vec::new();
        };
        let registry = self
            .reference_registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(index) = registry.documents.get(&document.id()) else {
            return Vec::new();
        };
        let mut current = Some(*node.syntax());
        let mut ancestors = Vec::new();
        while let Some(syntax) = current {
            let range = syntax.byte_range();
            if index
                .seen
                .contains(&(range.start, range.end, syntax.raw().id()))
            {
                ancestors.push(NodeRef::new(syntax));
            }
            current = syntax.parent();
        }
        ancestors.reverse();
        ancestors
    }

    pub(crate) fn resolve_scoped<'s>(
        &'s self,
        value: NodeRef<'_>,
        schema: bool,
    ) -> Result<NodeRef<'s>, CycleGuard> {
        match self
            .resolve_reference_context(value, schema)
            .map_err(|_| CycleGuard)?
        {
            Resolution::Node(node) => Ok(node),
            Resolution::WholeDoc(doc) => self
                .workspace()
                .get_by_id(doc)
                .map(|document| document.doc().root())
                .ok_or(CycleGuard),
            Resolution::Cycle { .. } => Err(CycleGuard),
        }
    }

    /// Follow a reference chain through the same schema-scoped addresses as
    /// `reference_target`, retaining detailed errors and explicit cycle reports.
    /// Generic reference/JSON callers can continue using suspect-ref directly.
    ///
    /// # Errors
    /// Invalid/ambiguous references, inaccessible targets, or the workspace's
    /// configured reference-chain depth limit.
    pub fn resolve_reference<'s>(&'s self, value: NodeRef<'_>) -> Result<Resolution<'s>, RefError> {
        self.resolve_reference_context(value, false)
    }

    fn resolve_reference_context<'s>(
        &'s self,
        value: NodeRef<'_>,
        schema: bool,
    ) -> Result<Resolution<'s>, RefError> {
        let mut target = self.reference_target(value)?;
        let mut seen = HashSet::new();
        let mut steps = Vec::new();
        loop {
            if !seen.insert((target.doc, target.pointer.clone())) {
                return Ok(Resolution::Cycle {
                    path: steps.into_boxed_slice(),
                });
            }
            if steps.len() > self.workspace().reference_depth_cap() {
                return Err(RefError::TooDeep {
                    cap: self.workspace().reference_depth_cap(),
                });
            }
            let node = self
                .workspace()
                .get_by_id(target.doc)
                .ok_or_else(|| RefError::MissingDoc {
                    uri: format!("doc #{}", target.doc),
                })?
                .node_at_pointer(&target.pointer)?;
            steps.push(Step {
                doc: target.doc,
                at: node.byte_range(),
            });
            if schema {
                self.register_schema(node);
            }
            match node.get("$ref") {
                Some(reference) => target = self.reference_target(reference)?,
                None if target.pointer.is_root() => return Ok(Resolution::WholeDoc(target.doc)),
                None => return Ok(Resolution::Node(node)),
            }
        }
    }
}

impl SchemaIndex {
    fn add(&mut self, mut pending: Vec<SchemaView<'_>>) {
        while let Some(schema) = pending.pop() {
            if schema.is_missing_value() {
                continue;
            }
            let node = schema.node();
            let range = node.byte_range();
            if !self
                .seen
                .insert((range.start, range.end, node.syntax().raw().id()))
            {
                continue;
            }
            if self.identifiers
                && let Some(anchor) = node.get("$anchor")
                && anchor.kind() == ValueKind::Str
                && let Some(bytes) = anchor.try_decoded_scalar()
                && let Ok(name) = std::str::from_utf8(&bytes)
            {
                let pointer = node.path_from_root();
                let targets = self.anchors.entry(name.to_owned()).or_default();
                if !targets.contains(&pointer) {
                    targets.push(pointer);
                }
            }
            if self.identifiers
                && let Some(id) = node.get("$id")
                && id.kind() == ValueKind::Str
                && let Some(bytes) = id.try_decoded_scalar()
                && let Ok(value) = std::str::from_utf8(&bytes)
            {
                self.ids.insert(node.path_from_root(), value.to_owned());
            }
            if self.identifiers || node.get("$ref").is_none() {
                pending.extend(schema.subschemas());
            }
        }
    }
}

fn workspace_error(error: WorkspaceError) -> RefError {
    match error {
        WorkspaceError::Ref(error) => error,
        WorkspaceError::Io(error) => RefError::Io(error),
        other => RefError::InvalidRef {
            raw: String::new(),
            reason: other.to_string(),
        },
    }
}
