//! Compile the immutable Contract resource registry, without acquiring or guessing.
use super::compile::invalid;
use super::*;
use suspect_ir::contract::{AnchorKind, ResourceKind};

pub(super) fn context(
    contract: &Contract,
    closure: &[SchemaId],
    indices: &BTreeMap<SchemaId, usize>,
) -> Result<ProgramResourceContext, OwnedCompileError> {
    let mut ids = BTreeSet::new();
    for id in closure {
        let scope = contract.resource_scope(id).ok_or_else(|| {
            invalid(
                contract,
                id,
                "schema has no unambiguous indexed resource scope",
            )
        })?;
        ids.insert(scope.resource().clone());
        let resolved = contract
            .resolve_resource_reference(id, scope.address())
            .map_err(|cause| {
                invalid(
                    contract,
                    id,
                    &format!("canonical schema address is not uniquely resolvable: {cause}"),
                )
            })?;
        if resolved != *id {
            return Err(invalid(
                contract,
                id,
                "canonical schema address does not identify its physical source",
            ));
        }
    }
    let resource_indices: BTreeMap<_, _> = ids
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, id)| (id, index))
        .collect();
    let mut resources = Vec::new();
    for id in ids {
        let resource = contract.resource(&id).ok_or_else(|| {
            invalid(
                contract,
                &id,
                "resource is missing or has conflicting indexed contexts",
            )
        })?;
        let mut dynamic_anchors = Vec::new();
        let mut names = BTreeSet::new();
        for anchor in resource
            .anchors()
            .iter()
            .filter(|anchor| anchor.kind() == AnchorKind::Dynamic)
        {
            if let Some(&target) = indices.get(anchor.target()) {
                if !names.insert(anchor.name()) {
                    return Err(invalid(
                        contract,
                        anchor.source(),
                        "dynamic anchor is ambiguous within its resource",
                    ));
                }
                dynamic_anchors.push((anchor.name().to_owned(), anchor.source().into(), target));
            }
        }
        dynamic_anchors.sort_by(|left, right| left.0.cmp(&right.0));
        resources.push(ProgramResource {
            source: resource.source().into(),
            kind: match resource.kind() {
                ResourceKind::Document => "document",
                ResourceKind::OpenApiDocument => "openApiDocument",
                ResourceKind::Schema => "schema",
            },
            canonical_uri: resource.canonical_uri().to_owned(),
            base_uri: resource.base_uri().to_owned(),
            aliases: resource.aliases().to_vec(),
            declaration_source: resource.declaration_source().map(Into::into),
            dynamic_anchors,
        });
    }
    let node_scopes = closure
        .iter()
        .map(|id| {
            let scope = contract.resource_scope(id).expect("admitted scope");
            let root = scope.schema_root().ok_or_else(|| {
                invalid(
                    contract,
                    id,
                    "indexed schema scope has no original schema-root source",
                )
            })?;
            Ok((
                resource_indices[scope.resource()],
                root.into(),
                scope.address().to_owned(),
            ))
        })
        .collect::<Result<_, OwnedCompileError>>()?;
    Ok(ProgramResourceContext {
        resources,
        node_scopes,
    })
}

pub(super) fn dynamic(
    contract: &Contract,
    id: &SchemaId,
    indices: &BTreeMap<SchemaId, usize>,
    context: &ProgramResourceContext,
) -> Result<Kind, OwnedCompileError> {
    let source = id.child("$dynamicRef");
    let raw = contract.source(&source).expect("declared dynamicRef");
    let text = super::dialect::uri_reference(contract, &source, raw)?;
    let reference = contract.dynamic_reference(id).ok_or_else(|| {
        invalid(
            contract,
            &source,
            "dynamic reference has no indexed metadata",
        )
    })?;
    let target = reference.initial_target().ok_or_else(|| {
        invalid(
            contract,
            &source,
            "dynamic reference has no resolved initial target",
        )
    })?;
    let resolved = contract
        .resolve_resource_reference(id, text)
        .map_err(|cause| invalid(contract, &source, &cause.to_string()))?;
    if resolved != *target {
        return Err(invalid(
            contract,
            &source,
            "dynamic initial target disagrees with indexed URI resolution",
        ));
    }
    let target_index = *indices.get(target).ok_or_else(|| {
        invalid(
            contract,
            &source,
            "dynamic initial target is outside the selected indexed closure",
        )
    })?;
    let resource = reference
        .initial_resource()
        .ok_or_else(|| invalid(contract, &source, "dynamic initial target has no resource"))?;
    let initial_resource = context
        .resources
        .iter()
        .position(|entry| entry.source == ProgramSource::from(resource))
        .ok_or_else(|| {
            invalid(
                contract,
                &source,
                "dynamic initial resource is outside the compiled registry",
            )
        })?;
    for binding in reference.candidates() {
        if let Some(resource) = context
            .resources
            .iter()
            .find(|entry| entry.source == ProgramSource::from(binding.resource()))
        {
            let index = indices.get(binding.target()).ok_or_else(|| {
                invalid(
                    contract,
                    binding.source(),
                    "entered-resource dynamic candidate is missing from the indexed closure",
                )
            })?;
            if !resource.dynamic_anchors.iter().any(|(name, at, target)| {
                name == binding.name()
                    && *at == ProgramSource::from(binding.source())
                    && target == index
            }) {
                return Err(invalid(
                    contract,
                    binding.source(),
                    "dynamic candidate is missing from its compiled resource bindings",
                ));
            }
        }
    }
    Ok(Kind::DynamicReference {
        target: target_index,
        initial_resource,
        anchor: reference.dynamic_anchor().map(str::to_owned),
    })
}
