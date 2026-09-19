//! Locate schema roots using OpenAPI object positions, never arbitrary data.

use rustc_hash::FxHashSet;
use suspect_low::NodeRef;

use crate::{OpenApi, SchemaView};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Root,
    Components,
    PathItem,
    Operation,
    Parameter,
    Response,
    Body,
    Media,
    Header,
    Encoding,
    Callback,
    ReferenceLeaf,
    Schema,
    MissingSchema,
}

struct Objects<'s> {
    schemas: Vec<SchemaView<'s>>,
    references: Vec<NodeRef<'s>>,
}

impl<'s> OpenApi<'s> {
    /// Schema roots declared in components and transport objects, including
    /// parameters, bodies, responses, headers, callbacks, and webhooks.
    ///
    /// Follows OpenAPI Reference Objects while retaining source identity and
    /// terminating cycles. Nested schema applicators are exposed separately
    /// by [`SchemaView::subschemas`]. Examples, defaults, and extensions are
    /// instance/annotation data and cannot introduce schema roots.
    #[must_use]
    pub fn schema_roots(&self) -> Vec<SchemaView<'s>> {
        self.walk_objects(true).schemas
    }

    pub(crate) fn declared_schema_roots(&self) -> Vec<SchemaView<'s>> {
        self.walk_objects(false).schemas
    }

    /// Objects carrying `$ref` in OpenAPI Reference Object or Schema Object
    /// positions, including malformed or absent values. Arbitrary instance
    /// data in examples/defaults/extensions does not introduce references.
    ///
    /// Returns the containing object so callers can report the key's location
    /// even when its value is absent. Schemas and references are visited once
    /// per source document/range; recursive contracts terminate.
    #[must_use]
    pub fn reference_objects(&self) -> Vec<NodeRef<'s>> {
        let Objects {
            schemas,
            mut references,
        } = self.walk_objects(true);
        let mut pending = schemas;
        let mut visited = FxHashSet::default();
        while let Some(schema) = pending.pop() {
            let node = schema.node();
            let range = node.byte_range();
            if !visited.insert((node.syntax().doc().uri().clone(), range.start, range.end)) {
                continue;
            }
            if has_ref(node) {
                references.push(node);
                if let Ok(Some(target)) = schema.reference_target() {
                    pending.push(target);
                }
                if !self.version.is_31_plus() {
                    continue;
                }
            }
            pending.extend(schema.subschemas());
        }
        references
    }

    fn walk_objects(&self, follow_references: bool) -> Objects<'s> {
        let mut pending = vec![(Kind::Root, self.root())];
        let mut visited = FxHashSet::default();
        let mut schemas = Vec::new();
        let mut references = Vec::new();
        while let Some((kind, node)) = pending.pop() {
            let range = node.byte_range();
            if !visited.insert((
                kind,
                node.syntax().doc().uri().clone(),
                range.start,
                range.end,
            )) {
                continue;
            }
            if matches!(kind, Kind::Schema) {
                schemas.push(SchemaView::new(self.session, node));
                continue;
            }
            if matches!(kind, Kind::MissingSchema) {
                schemas.push(SchemaView::missing(self.session, node));
                continue;
            }
            let version = self
                .session
                .workspace()
                .get(node.syntax().doc().uri())
                .and_then(|document| crate::OasVersion::sniff(document.doc()))
                .unwrap_or(self.version);
            if (matches!(
                kind,
                Kind::PathItem
                    | Kind::Parameter
                    | Kind::Response
                    | Kind::Body
                    | Kind::Header
                    | Kind::Callback
                    | Kind::ReferenceLeaf
            ) || (kind == Kind::Media && version == crate::OasVersion::V32))
                && has_ref(node)
            {
                references.push(node);
                if follow_references
                    && let Some(reference) = node.get("$ref")
                    && let Ok(target) = self.session.resolve_target(reference)
                {
                    pending.push((kind, target));
                }
                // Non-schema Reference Object siblings cannot declare
                // schemas. Path Items have their own $ref field semantics.
                if kind != Kind::PathItem {
                    continue;
                }
            }
            match kind {
                Kind::Root => {
                    self.security_schemes(&mut pending, node);
                    field(&mut pending, node, "components", Kind::Components);
                    map(&mut pending, node, "paths", Kind::PathItem, true);
                    if version.is_31_plus() {
                        map(&mut pending, node, "webhooks", Kind::PathItem, false);
                    }
                }
                Kind::Components => {
                    map(&mut pending, node, "schemas", Kind::Schema, false);
                    map(&mut pending, node, "parameters", Kind::Parameter, false);
                    map(&mut pending, node, "responses", Kind::Response, false);
                    map(&mut pending, node, "requestBodies", Kind::Body, false);
                    map(&mut pending, node, "headers", Kind::Header, false);
                    map(&mut pending, node, "callbacks", Kind::Callback, false);
                    for section in ["examples", "links", "securitySchemes"] {
                        map(&mut pending, node, section, Kind::ReferenceLeaf, false);
                    }
                    if version.is_31_plus() {
                        map(&mut pending, node, "pathItems", Kind::PathItem, false);
                    }
                    if version == crate::OasVersion::V32 {
                        map(&mut pending, node, "mediaTypes", Kind::Media, false);
                    }
                }
                Kind::PathItem => {
                    array(&mut pending, node, "parameters", Kind::Parameter);
                    for method in crate::paths::PATH_METHODS {
                        field(&mut pending, node, method, Kind::Operation);
                    }
                    if version == crate::OasVersion::V32 {
                        field(&mut pending, node, "query", Kind::Operation);
                        map(
                            &mut pending,
                            node,
                            "additionalOperations",
                            Kind::Operation,
                            false,
                        );
                    }
                }
                Kind::Operation => {
                    self.security_schemes(&mut pending, node);
                    array(&mut pending, node, "parameters", Kind::Parameter);
                    field(&mut pending, node, "requestBody", Kind::Body);
                    map(&mut pending, node, "responses", Kind::Response, true);
                    map(&mut pending, node, "callbacks", Kind::Callback, false);
                }
                Kind::Parameter | Kind::Header => {
                    field(&mut pending, node, "schema", Kind::Schema);
                    map(&mut pending, node, "content", Kind::Media, false);
                    map(&mut pending, node, "examples", Kind::ReferenceLeaf, false);
                }
                Kind::Response => {
                    map(&mut pending, node, "headers", Kind::Header, false);
                    map(&mut pending, node, "content", Kind::Media, false);
                    map(&mut pending, node, "links", Kind::ReferenceLeaf, false);
                }
                Kind::Body => map(&mut pending, node, "content", Kind::Media, false),
                Kind::Media => {
                    field(&mut pending, node, "schema", Kind::Schema);
                    map(&mut pending, node, "encoding", Kind::Encoding, false);
                    map(&mut pending, node, "examples", Kind::ReferenceLeaf, false);
                }
                Kind::Encoding => map(&mut pending, node, "headers", Kind::Header, false),
                Kind::Callback => {
                    for entry in node.entries() {
                        let Some(key) = entry.key_node.try_decoded_scalar() else {
                            continue;
                        };
                        if !key.starts_with(b"x-")
                            && key.as_ref() != b"$ref"
                            && let Some(value) = entry.value
                        {
                            pending.push((Kind::PathItem, value));
                        }
                    }
                }
                Kind::Schema | Kind::MissingSchema => {
                    unreachable!("schema handled before traversal")
                }
                Kind::ReferenceLeaf => {}
            }
        }
        Objects {
            schemas,
            references,
        }
    }

    // Security requirement keys are implicit references resolved in the source
    // document. A referenced operation can therefore introduce a security
    // scheme whose explicit $ref is outside the entry document's components.
    fn security_schemes(&self, pending: &mut Vec<(Kind, NodeRef<'s>)>, node: NodeRef<'s>) {
        let Some(requirements) = node.get("security") else {
            return;
        };
        let Some(document) = self.session.workspace().get(node.syntax().doc().uri()) else {
            return;
        };
        let Some(schemes) = document
            .doc()
            .root()
            .get("components")
            .and_then(|components| components.get("securitySchemes"))
        else {
            return;
        };
        for requirement in requirements.items() {
            for entry in requirement.entries() {
                let Some(name) = entry.key_node.try_decoded_scalar() else {
                    continue;
                };
                let Ok(name) = std::str::from_utf8(&name) else {
                    continue;
                };
                if let Some(scheme) = schemes.get(name) {
                    pending.push((Kind::ReferenceLeaf, scheme));
                }
            }
        }
    }
}

fn has_ref(node: NodeRef<'_>) -> bool {
    node.entries().iter().any(|entry| {
        entry
            .key_node
            .try_decoded_scalar()
            .is_some_and(|key| key.as_ref() == b"$ref")
    })
}

fn field<'s>(pending: &mut Vec<(Kind, NodeRef<'s>)>, node: NodeRef<'s>, key: &str, kind: Kind) {
    if let Some(value) = node.get(key) {
        pending.push((kind, value));
    } else if kind == Kind::Schema
        && let Some(entry) = node.entries().into_iter().find(|entry| {
            entry
                .key_node
                .try_decoded_scalar()
                .is_some_and(|text| text.as_ref() == key.as_bytes())
        })
    {
        pending.push((Kind::MissingSchema, entry.key_node));
    }
}

fn map<'s>(
    pending: &mut Vec<(Kind, NodeRef<'s>)>,
    node: NodeRef<'s>,
    key: &str,
    kind: Kind,
    extensions: bool,
) {
    if let Some(value) = node.get(key) {
        for entry in value.entries() {
            let extension = entry
                .key_node
                .try_decoded_scalar()
                .is_some_and(|key| key.starts_with(b"x-"));
            if !(extensions && extension) {
                if let Some(value) = entry.value {
                    pending.push((kind, value));
                } else if kind == Kind::Schema {
                    pending.push((Kind::MissingSchema, entry.key_node));
                }
            }
        }
    }
}

fn array<'s>(pending: &mut Vec<(Kind, NodeRef<'s>)>, node: NodeRef<'s>, key: &str, kind: Kind) {
    if let Some(value) = node.get(key) {
        pending.extend(value.items().into_iter().map(|value| (kind, value)));
    }
}
