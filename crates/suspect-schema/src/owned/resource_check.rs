//! V3 portable resource invariants. No loading and no raw-schema reinterpretation.
use super::*;
use iri_string::types::{UriAbsoluteStr, UriStr};
use suspect_ir::contract::resource_uri;

fn contains(parent: &ProgramSource, child: &ProgramSource) -> bool {
    parent.document == child.document
        && (parent.pointer == child.pointer
            || child
                .pointer
                .strip_prefix(&parent.pointer)
                .is_some_and(|suffix| suffix.starts_with('/')))
}

fn uri_key(value: &str, at: &ProgramSource) -> Result<String, ProgramCheckError> {
    if UriStr::new(value).is_err() {
        return Err(error(
            Some(at),
            "resource identifiers must be absolute RFC 3986 URIs",
        ));
    }
    let (document, fragment) =
        resource_uri::split_reference(value).map_err(|cause| error(Some(at), cause.to_string()))?;
    let document = resource_uri::resolve_document(document, "")
        .map_err(|cause| error(Some(at), cause.to_string()))?;
    let fragment = resource_uri::decode_fragment(fragment)
        .map_err(|cause| error(Some(at), cause.to_string()))?;
    Ok(if fragment.is_empty() {
        document
    } else {
        format!("{document}#{}", resource_uri::encode_fragment(&fragment))
    })
}

pub(super) fn anchor_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes
        .first()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(byte))
}

pub(super) fn check(program: &OwnedProgram) -> Result<(), ProgramCheckError> {
    if program.version != OwnedProgram::V3_VERSION {
        return if program.resource_context.is_none() {
            Ok(())
        } else {
            Err(error(None, "resource metadata requires the v3 profile"))
        };
    }
    let context = program
        .resource_context
        .as_ref()
        .ok_or_else(|| error(None, "v3 requires indexed resource context"))?;
    if context.node_scopes.len() != program.nodes.len() {
        return Err(error(
            None,
            "v3 node scopes must align one-to-one with program nodes",
        ));
    }
    let mut sources = BTreeSet::new();
    let mut aliases = BTreeMap::new();
    for (index, resource) in context.resources.iter().enumerate() {
        let at = &resource.source;
        source(at)?;
        if !sources.insert((&at.document, &at.pointer)) {
            return Err(error(Some(at), "duplicate physical resource identity"));
        }
        if !matches!(resource.kind, "schema" | "document" | "openApiDocument") {
            return Err(error(Some(at), "unknown compiled resource kind"));
        }
        let canonical = uri_key(&resource.canonical_uri, at)?;
        if UriAbsoluteStr::new(&resource.base_uri).is_err() {
            return Err(error(
                Some(at),
                "resource base URI must be absolute and fragment-free",
            ));
        }
        let base = uri_key(&resource.base_uri, at)?;
        if resource_uri::resolve_document(&resource.base_uri, &resource.canonical_uri)
            .map_err(|cause| error(Some(at), cause.to_string()))?
            != resource.base_uri
            || (resource.kind != "openApiDocument" && canonical != base)
        {
            return Err(error(
                Some(at),
                "resource canonical identifier and base are inconsistent",
            ));
        }
        if let Some(declaration) = &resource.declaration_source {
            source(declaration)?;
            let keyword = match resource.kind {
                "schema" => "$id",
                "openApiDocument" => "$self",
                _ => {
                    return Err(error(
                        Some(at),
                        "a fragment document has no identifier declaration",
                    ));
                }
            };
            if *declaration != child(at, keyword) {
                return Err(error(
                    Some(declaration),
                    "identifier declaration is not at its resource boundary",
                ));
            }
        } else if !at.pointer.is_empty() || canonical != uri_key(&at.document, at)? {
            return Err(error(
                Some(at),
                "an undeclared resource must retain its document retrieval identity",
            ));
        }
        distinct(
            resource.aliases.iter().map(String::as_str),
            at,
            "resource alias",
        )?;
        let mut names = BTreeSet::new();
        for alias in &resource.aliases {
            let key = uri_key(alias, at)?;
            if aliases
                .insert(key.clone(), index)
                .is_some_and(|previous| previous != index)
            {
                return Err(error(
                    Some(at),
                    "resource URI alias identifies multiple physical resources",
                ));
            }
            names.insert(key);
        }
        if !names.contains(&canonical) || !names.contains(&base) {
            return Err(error(
                Some(at),
                "resource aliases must include its canonical identifier and base",
            ));
        }
    }
    let mut used = BTreeSet::new();
    for (index, (resource_index, schema_root, address)) in context.node_scopes.iter().enumerate() {
        let node = &program.nodes[index];
        source(&node.source)?;
        source(schema_root)?;
        let resource = context.resources.get(*resource_index).ok_or_else(|| {
            error(
                Some(&node.source),
                "node resource index is outside the compiled registry",
            )
        })?;
        if !contains(&resource.source, schema_root)
            || !contains(schema_root, &node.source)
            || resource.kind == "schema" && *schema_root != resource.source
        {
            return Err(error(
                Some(&node.source),
                "node schema/resource roots do not contain its physical source",
            ));
        }
        let suffix = node
            .source
            .pointer
            .strip_prefix(&resource.source.pointer)
            .expect("checked physical containment");
        let expected = if suffix.is_empty() {
            resource.base_uri.clone()
        } else {
            format!(
                "{}#{}",
                resource.base_uri,
                resource_uri::encode_fragment(suffix)
            )
        };
        if *address != expected {
            return Err(error(
                Some(&node.source),
                "node canonical address does not match its resource-relative source pointer",
            ));
        }
        used.insert(*resource_index);
    }
    if used.len() != context.resources.len() {
        return Err(error(
            None,
            "compiled resource registry contains an unentered/unreferenced resource",
        ));
    }
    for (resource_index, resource) in context.resources.iter().enumerate() {
        distinct(
            resource
                .dynamic_anchors
                .iter()
                .map(|(name, _, _)| name.as_str()),
            &resource.source,
            "dynamic anchor name",
        )?;
        for (name, at, index) in &resource.dynamic_anchors {
            source(at)?;
            let node = target(program, *index, at)?;
            if !anchor_name(name)
                || *at != child(&node.source, "$dynamicAnchor")
                || context.node_scopes[*index].0 != resource_index
            {
                return Err(error(
                    Some(at),
                    "dynamic binding name, keyword source or target resource is inconsistent",
                ));
            }
        }
    }
    Ok(())
}
