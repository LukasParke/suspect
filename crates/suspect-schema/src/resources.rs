//! Lexical schema-resource indexing and static reference resolution.
//!
//! Instance data and unknown keyword values cannot declare resources. Dynamic
//! scope is a sequence of these resources, not a sequence of visited anchors.

use rustc_hash::FxHashMap;
use std::borrow::Cow;
use suspect_low::{NodeRef, Pointer, ValueKind};
mod uri;

use crate::CompileError;
use crate::compile::RefTarget;

#[derive(Default)]
pub(crate) struct Resource {
    pub base: String,
    pub anchors: FxHashMap<String, Pointer>,
    pub dynamic_anchors: FxHashMap<String, Pointer>,
}

#[derive(Default)]
pub(crate) struct Scan {
    pub resources: FxHashMap<Pointer, Resource>,
    pub base_ptrs: FxHashMap<String, Pointer>,
    schema_resources: FxHashMap<Pointer, Pointer>,
}

impl Scan {
    pub fn resource_for(&self, path: &Pointer) -> Pointer {
        let mut current = Some(path.clone());
        while let Some(path) = current {
            if let Some(resource) = self.schema_resources.get(&path) {
                return resource.clone();
            }
            current = path.parent();
        }
        Pointer::root()
    }

    pub fn base_for(&self, path: &Pointer) -> &str {
        &self.resources[&self.resource_for(path)].base
    }

    pub fn dynamic_anchor(&self, resource: &Pointer, name: &str) -> Option<&Pointer> {
        self.resources.get(resource)?.dynamic_anchors.get(name)
    }
}

struct Pending<'d> {
    node: NodeRef<'d>,
    path: Pointer,
    resource: Pointer,
    base: String,
    depth: usize,
}

fn invalid(message: impl Into<String>, node: NodeRef<'_>) -> CompileError {
    CompileError::Invalid {
        message: message.into(),
        at: node.byte_range(),
    }
}

pub(crate) fn string(node: NodeRef<'_>, keyword: &str) -> Result<String, CompileError> {
    if node.kind() != ValueKind::Str {
        return Err(invalid(format!("`{keyword}` must be a string"), node));
    }
    decoded_text(node)
        .map(Cow::into_owned)
        .ok_or_else(|| invalid(format!("`{keyword}` must be a valid decoded string"), node))
}

pub(crate) fn decoded_text(node: NodeRef<'_>) -> Option<Cow<'_, str>> {
    match node.try_decoded_scalar()? {
        Cow::Borrowed(bytes) => std::str::from_utf8(bytes).ok().map(Cow::Borrowed),
        Cow::Owned(bytes) => String::from_utf8(bytes).ok().map(Cow::Owned),
    }
}

pub(crate) fn scan_doc(
    root: NodeRef<'_>,
    retrieval_uri: &str,
    max_depth: usize,
) -> Result<Scan, CompileError> {
    let retrieval_uri = uri::resolve_document(retrieval_uri, "")
        .map_err(|error| invalid(format!("invalid schema retrieval URI: {error}"), root))?;
    let mut scan = Scan::default();
    scan.base_ptrs
        .insert(retrieval_uri.clone(), Pointer::root());
    let mut pending = vec![Pending {
        node: root,
        path: Pointer::root(),
        resource: Pointer::root(),
        base: retrieval_uri,
        depth: 0,
    }];
    while let Some(mut item) = pending.pop() {
        if item.depth > max_depth {
            return Err(CompileError::TooDeep { cap: max_depth });
        }
        if let Some(id) = item.node.get("$id") {
            let id_text = string(id, "$id")?;
            if id_text
                .split_once('#')
                .is_some_and(|(_, fragment)| !fragment.is_empty())
            {
                return Err(invalid("`$id` cannot contain a nonempty fragment", id));
            }
            item.base = join_uri(&item.base, &id_text)
                .ok_or_else(|| invalid(format!("unresolvable `$id` `{id_text}`"), id))?;
            item.resource = item.path.clone();
            if let Some(previous) = scan.base_ptrs.insert(item.base.clone(), item.path.clone())
                && previous != item.path
            {
                return Err(invalid(
                    format!("duplicate schema resource URI `{}`", item.base),
                    id,
                ));
            }
        }
        scan.schema_resources
            .insert(item.path.clone(), item.resource.clone());
        let resource = scan
            .resources
            .entry(item.resource.clone())
            .or_insert_with(|| Resource {
                base: item.base.clone(),
                ..Resource::default()
            });
        for keyword in ["$anchor", "$dynamicAnchor"] {
            if let Some(value) = item.node.get(keyword) {
                let name = string(value, keyword)?;
                let mut chars = name.chars();
                if !chars
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                    || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
                {
                    return Err(invalid(
                        format!("`{keyword}` must match [A-Za-z_][A-Za-z0-9_.-]*"),
                        value,
                    ));
                }
                if let Some(previous) = resource.anchors.insert(name.clone(), item.path.clone())
                    && previous != item.path
                {
                    return Err(invalid(
                        format!("duplicate schema anchor `{name}` in one resource"),
                        value,
                    ));
                }
                if keyword == "$dynamicAnchor" {
                    resource.dynamic_anchors.insert(name, item.path.clone());
                }
            }
        }

        let mut children = Vec::new();
        for entry in item.node.entries() {
            let keyword = string(entry.key_node, "schema keyword")?;
            let Some(value) = entry.value else { continue };
            let at = item.path.push(&keyword);
            match keyword.as_str() {
                "$defs" | "definitions" | "properties" | "patternProperties"
                | "dependentSchemas" | "dependencies" => {
                    for child in value.entries() {
                        let name = string(child.key_node, "schema name")?;
                        if let Some(node) = child.value
                            && matches!(node.kind(), ValueKind::Object | ValueKind::Bool)
                        {
                            children.push((node, at.push(&name)));
                        }
                    }
                }
                "allOf" | "anyOf" | "oneOf" | "prefixItems" => {
                    for (index, node) in value.items().into_iter().enumerate() {
                        children.push((node, at.push(&index.to_string())));
                    }
                }
                "items"
                | "contains"
                | "additionalProperties"
                | "propertyNames"
                | "unevaluatedProperties"
                | "unevaluatedItems"
                | "not"
                | "if"
                | "then"
                | "else" => children.push((value, at)),
                "contentSchema" => children.push((value, at)),
                _ => {}
            }
        }
        // Reversal preserves lexical document order despite the LIFO worklist.
        for (node, path) in children.into_iter().rev() {
            pending.push(Pending {
                node,
                path,
                resource: item.resource.clone(),
                base: item.base.clone(),
                depth: item.depth + 1,
            });
        }
    }
    Ok(scan)
}

pub(crate) fn resolve_ref_target(
    raw: &str,
    scan: &Scan,
    resource: &Pointer,
    at: NodeRef<'_>,
) -> Result<RefTarget, CompileError> {
    let (document, fragment) = uri::split_reference(raw)
        .map_err(|error| invalid(format!("invalid schema reference: {error}"), at))?;
    let target_resource = if document.is_empty() {
        resource.clone()
    } else {
        let uri = join_uri(&scan.resources[resource].base, document)
            .ok_or_else(|| invalid(format!("unresolvable `$ref` `{raw}`"), at))?;
        let Some(target) = scan.base_ptrs.get(&uri) else {
            return Ok(RefTarget::External);
        };
        target.clone()
    };
    let fragment = decode_fragment(fragment)
        .ok_or_else(|| invalid("invalid reference fragment encoding", at))?;
    if fragment.is_empty() {
        return Ok(RefTarget::Local(target_resource));
    }
    if fragment.starts_with('/') {
        let pointer = Pointer::parse(&fragment).map_err(|error| invalid(error.to_string(), at))?;
        return Ok(RefTarget::Local(target_resource.join(&pointer)));
    }
    scan.resources[&target_resource]
        .anchors
        .get(&fragment)
        .cloned()
        .map(RefTarget::Local)
        .ok_or_else(|| invalid(format!("unknown anchor `{fragment}`"), at))
}

pub(crate) fn dynamic_name(raw: &str, target: &RefTarget, scan: &Scan) -> Option<std::rc::Rc<str>> {
    let RefTarget::Local(path) = target else {
        return None;
    };
    let (_, fragment) = uri::split_reference(raw).ok()?;
    let name = decode_fragment(fragment)?;
    if name.is_empty() || name.starts_with('/') {
        return None;
    }
    (scan.dynamic_anchor(&scan.resource_for(path), &name) == Some(path)).then(|| name.into())
}

fn decode_fragment(fragment: &str) -> Option<String> {
    let mut result = Vec::with_capacity(fragment.len());
    let mut bytes = fragment.as_bytes().iter().copied();
    while let Some(byte) = bytes.next() {
        result.push(if byte == b'%' {
            let hi = char::from(bytes.next()?).to_digit(16)?;
            let lo = char::from(bytes.next()?).to_digit(16)?;
            u8::try_from(hi * 16 + lo).ok()?
        } else {
            byte
        });
    }
    String::from_utf8(result).ok()
}

fn join_uri(base: &str, reference: &str) -> Option<String> {
    uri::resolve_document(base, reference).ok()
}
