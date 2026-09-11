//! Pinned bytes are tested at the public Workspace consumer seam.

use std::sync::Arc;

use suspect_ir::contract::{
    AnchorKind, Contract, ContractReader, ContractSeverity, ResourceKind, SourceId,
};
use suspect_low::Pointer;
use suspect_ref::{
    DocumentProvider, ProvidedDocument, ProviderError, RefError, Resolution, WorkspaceBuilder,
    WorkspaceError,
};
use suspect_source::Uri;

fn uri(text: &str) -> Uri {
    Uri::parse(text).unwrap()
}

#[test]
fn conflicting_alias_bytes_or_reference_bases_are_rejected_before_compilation() {
    let alias = uri("https://spec.example.test/latest.json");
    let final_uri = uri("https://spec.example.test/v1/root.json");
    for (second_uri, bytes) in [
        (final_uri.clone(), b"[]".to_vec()),
        (
            uri("https://spec.example.test/v2/root.json"),
            b"{}".to_vec(),
        ),
    ] {
        let error = DocumentProvider::new([
            ProvidedDocument::new(alias.clone(), final_uri.clone(), b"{}".to_vec()).unwrap(),
            ProvidedDocument::new(alias.clone(), second_uri, bytes).unwrap(),
        ])
        .unwrap_err();
        assert!(matches!(error, ProviderError::ConflictingIdentity { .. }));
    }
}

#[test]
fn a_closed_provider_never_falls_back_to_an_existing_file() {
    mod_fixture(|path| {
        let entry = Uri::from_path(path).unwrap();
        let workspace = WorkspaceBuilder::new()
            .allowed_documents([entry.clone()])
            .document_provider(Arc::new(DocumentProvider::new([]).unwrap()))
            .build()
            .unwrap();
        assert!(matches!(
            workspace.open(entry.as_str()),
            Err(WorkspaceError::Ref(RefError::MissingDoc { .. }))
        ));
        assert_eq!(workspace.failed_document_uris(), vec![entry]);
    });
}

fn mod_fixture(test: impl FnOnce(&std::path::Path)) {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-acquire-fixtures");
    std::fs::create_dir_all(&root).unwrap();
    let directory = tempfile::tempdir_in(root).unwrap();
    let path = directory.path().join("original.json");
    std::fs::write(&path, b"{}").unwrap();
    test(&path);
}

#[test]
fn entry_and_effective_alias_allowlists_are_checked_before_loading() {
    let requested = uri("https://spec.example.test/latest.json");
    let effective = uri("https://spec.example.test/v1/root.json");
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            requested.clone(),
            effective.clone(),
            b"{}".to_vec(),
        )
        .unwrap()])
        .unwrap(),
    );
    for allowed in [vec![], vec![requested.clone()], vec![effective.clone()]] {
        let workspace = WorkspaceBuilder::new()
            .allowed_documents(allowed)
            .document_provider(provider.clone())
            .build()
            .unwrap();
        assert!(matches!(
            workspace.open(requested.as_str()),
            Err(WorkspaceError::Ref(RefError::OutsideAllowlist { .. }))
        ));
        assert_eq!(workspace.len(), 0);
    }
    let nonexistent = uri("file:///nonexistent/pinned-entry.json");
    let workspace = WorkspaceBuilder::new()
        .allowed_documents([])
        .build()
        .unwrap();
    assert!(matches!(
        workspace.open(nonexistent.as_str()),
        Err(WorkspaceError::Ref(RefError::OutsideAllowlist { .. }))
    ));
}

#[test]
fn lazy_reference_loads_obey_document_and_byte_limits_and_record_failed_targets() {
    let entry = uri("https://spec.example.test/root.json");
    let target = uri("https://spec.example.test/leaf.json");
    let provider = Arc::new(
        DocumentProvider::new([
            ProvidedDocument::new(
                entry.clone(),
                entry.clone(),
                br##"{"use":{"$ref":"leaf.json#/Value"}}"##.to_vec(),
            )
            .unwrap(),
            ProvidedDocument::new(
                target.clone(),
                target.clone(),
                br#"{"Value":{"type":"string"}}"#.to_vec(),
            )
            .unwrap(),
        ])
        .unwrap(),
    );
    let workspace = WorkspaceBuilder::new()
        .document_provider(provider.clone())
        .max_docs(1)
        .build()
        .unwrap();
    let root = workspace.open(entry.as_str()).unwrap();
    assert!(matches!(
        root.resolve_edge(0),
        Err(RefError::TooManyDocs { max: 1 })
    ));
    assert_eq!(workspace.failed_document_uris(), vec![target]);
    assert_eq!(workspace.len(), 1);
    let workspace = WorkspaceBuilder::new()
        .document_provider(provider)
        .max_doc_size(2)
        .build()
        .unwrap();
    assert!(matches!(
        workspace.open(entry.as_str()),
        Err(WorkspaceError::Ref(RefError::TooLarge { limit: 2, .. }))
    ));
    assert_eq!(workspace.len(), 0);
}

#[test]
fn unprovided_allowed_refs_are_missing_and_unlisted_refs_are_denied_without_guessing() {
    let entry = uri("https://spec.example.test/root.json");
    let target = uri("https://spec.example.test/missing.json");
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            br##"{"use":{"$ref":"missing.json#/Value"}}"##.to_vec(),
        )
        .unwrap()])
        .unwrap(),
    );
    for allowed in [vec![entry.clone()], vec![entry.clone(), target.clone()]] {
        let admits_target = allowed.contains(&target);
        let workspace = WorkspaceBuilder::new()
            .document_provider(provider.clone())
            .allowed_documents(allowed)
            .build()
            .unwrap();
        let error = workspace
            .open(entry.as_str())
            .unwrap()
            .resolve_edge(0)
            .unwrap_err();
        if admits_target {
            assert!(matches!(error, RefError::MissingDoc { .. }));
        } else {
            assert!(matches!(error, RefError::OutsideAllowlist { .. }));
        }
        assert_eq!(workspace.failed_document_uris(), vec![target.clone()]);
    }
}

fn pinned_contract_document() -> serde_json::Value {
    serde_json::json!({
        "openapi":"3.1.0","info":{"title":"Pins","version":"1"},"paths":{},
        "components":{"schemas":{
            "Use":{"$ref":"schema.json#Pet"},
            "Dynamic":{"$dynamicRef":"#node"},
            "Node":{"$dynamicAnchor":"node","type":"integer"},
            "Resource":{"$id":"https://identifiers.example.test/identifier-only","type":"string"}
        }}
    })
}

fn pinned_contract(
    document: &serde_json::Value,
    reader: ContractReader,
) -> (Contract, Arc<suspect_ref::Workspace>, Uri, Uri) {
    let entry = uri("https://spec.example.test/openapi.json");
    let schema = uri("https://spec.example.test/schema.json");
    let provider = Arc::new(
        DocumentProvider::new([
            ProvidedDocument::new(
                entry.clone(),
                entry.clone(),
                serde_json::to_vec(document).unwrap(),
            )
            .unwrap(),
            ProvidedDocument::new(
                schema.clone(),
                schema.clone(),
                br#"{"$defs":{"Pet":{"$anchor":"Pet","type":"string"}}}"#.to_vec(),
            )
            .unwrap(),
        ])
        .unwrap(),
    );
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    let contract = Contract::from_workspace_with_reader(&workspace, &entry, reader).unwrap();
    (contract, workspace, entry, schema)
}

#[test]
fn pinned_contract_preserves_static_anchors_physical_sources_and_resource_metadata() {
    for reader in [ContractReader::Lossless, ContractReader::Fast] {
        let (contract, workspace, entry, schema) =
            pinned_contract(&pinned_contract_document(), reader);
        assert!(
            contract.diagnostics().is_empty(),
            "{:?}",
            contract.diagnostics()
        );
        let root = SourceId::new(entry.clone(), Pointer::root());
        let components = root.child("components").child("schemas");
        let pet = SourceId::new(schema.clone(), Pointer::parse("/$defs/Pet").unwrap());
        assert_eq!(
            contract.reference_target(&components.child("Use")),
            Some(&pet)
        );
        assert_eq!(contract.schema(&pet).unwrap().raw()["type"], "string");
        assert_eq!(contract.schema(&pet).unwrap().id().document(), &schema);

        let node = components.child("Node");
        let dynamic_source = components.child("Dynamic");
        let dynamic = contract.dynamic_reference(&dynamic_source).unwrap();
        assert_eq!(dynamic.source(), &dynamic_source.child("$dynamicRef"));
        assert_eq!(dynamic.initial_target(), Some(&node));
        assert_eq!(dynamic.initial_resource(), Some(&root));
        assert_eq!(dynamic.dynamic_anchor(), Some("node"));
        assert_eq!(dynamic.candidates().len(), 1);
        assert_eq!(dynamic.candidates()[0].target(), &node);
        assert_eq!(
            dynamic.candidates()[0].source(),
            &node.child("$dynamicAnchor")
        );
        assert_eq!(dynamic.candidates()[0].kind(), AnchorKind::Dynamic);
        assert!(
            contract.reference_target(&dynamic_source).is_none(),
            "a dynamic initial target must not become an ordinary static edge"
        );

        let resource_source = components.child("Resource");
        let resource = contract.resource(&resource_source).unwrap();
        assert_eq!(resource.source(), &resource_source);
        assert_eq!(resource.source().document(), &entry);
        assert_eq!(resource.kind(), ResourceKind::Schema);
        assert_eq!(
            resource.canonical_uri(),
            "https://identifiers.example.test/identifier-only"
        );
        assert_eq!(
            resource.declaration_source(),
            Some(&resource_source.child("$id"))
        );
        assert_eq!(
            contract
                .resource_scope(&resource_source)
                .unwrap()
                .base_uri(),
            resource.canonical_uri()
        );
        assert_eq!(
            contract.resolve_resource_reference(&components.child("Use"), resource.canonical_uri()),
            Ok(resource_source)
        );
        assert!(
            !contract
                .source_span(resource.declaration_source().unwrap())
                .unwrap()
                .is_empty()
        );

        assert_eq!(contract.documents().count(), 2);
        assert_eq!(workspace.uris(), vec![entry, schema]);
        assert!(workspace.get(&uri(resource.canonical_uri())).is_none());
        assert!(
            workspace
                .document_metadata(&uri(resource.canonical_uri()))
                .is_none()
        );
        assert!(
            workspace.failed_document_uris().is_empty(),
            "$id declaration and in-memory identifier lookup must not request retrieval"
        );
    }
}

#[test]
fn pinned_contract_reports_genuine_invalid_references_and_resource_declarations() {
    for (name, keyword, value, code) in [
        (
            "Use",
            "$ref",
            serde_json::json!("schema.json#Missing"),
            "invalid-reference",
        ),
        (
            "Dynamic",
            "$dynamicRef",
            serde_json::json!("#missing"),
            "invalid-dynamic-reference",
        ),
        (
            "Dynamic",
            "$dynamicRef",
            serde_json::Value::Null,
            "invalid-dynamic-reference",
        ),
        (
            "Resource",
            "$id",
            serde_json::json!(17),
            "invalid-schema-resource",
        ),
        (
            "Resource",
            "$id",
            serde_json::json!("https://identifiers.example.test/id#nonempty"),
            "invalid-schema-resource",
        ),
    ] {
        let mut document = pinned_contract_document();
        document["components"]["schemas"][name][keyword] = value.clone();
        for reader in [ContractReader::Lossless, ContractReader::Fast] {
            let (contract, workspace, entry, schema) = pinned_contract(&document, reader);
            let source = SourceId::new(entry.clone(), Pointer::root())
                .child("components")
                .child("schemas")
                .child(name);
            let diagnostic = contract
                .diagnostics()
                .iter()
                .find(|d| d.source == source && d.code == code)
                .unwrap_or_else(|| {
                    panic!("missing {code} at {source:?}: {:?}", contract.diagnostics())
                });
            assert_eq!(
                contract.diagnostics().len(),
                1,
                "{:?}",
                contract.diagnostics()
            );
            assert_eq!(diagnostic.severity, ContractSeverity::Error);
            assert_eq!(
                diagnostic.at,
                contract.source_span(&source.child(keyword)).unwrap()
            );
            assert!(!diagnostic.at.is_empty());
            assert_eq!(contract.source(&source.child(keyword)), Some(&value));
            match name {
                "Use" => assert!(contract.reference_target(&source).is_none()),
                "Dynamic" => {
                    let dynamic = contract.dynamic_reference(&source).unwrap();
                    assert!(dynamic.initial_target().is_none());
                    assert!(dynamic.dynamic_anchor().is_none());
                    assert!(dynamic.candidates().is_empty());
                }
                "Resource" => assert!(contract.resource_scope(&source).is_none()),
                _ => unreachable!(),
            }
            assert_eq!(contract.documents().count(), 2);
            assert_eq!(workspace.uris(), vec![entry, schema]);
            assert!(
                workspace.failed_document_uris().is_empty(),
                "invalid declarations must be diagnosed without acquiring identifier URIs"
            );
        }
    }
}

#[test]
fn requested_alias_and_effective_uri_share_a_document_and_its_remote_reference_base() {
    let requested = uri("https://spec.example.test/latest.json");
    let effective = uri("https://spec.example.test/v1/root.json");
    let dependency = uri("https://spec.example.test/v1/leaf.json");
    let provider = Arc::new(
        DocumentProvider::new([
            ProvidedDocument::new(
                requested.clone(),
                effective.clone(),
                br##"{"use":{"$ref":"leaf.json#/Value"}}"##.to_vec(),
            )
            .unwrap(),
            ProvidedDocument::new(
                dependency.clone(),
                dependency.clone(),
                br#"{"Value":{"type":"string"}}"#.to_vec(),
            )
            .unwrap(),
        ])
        .unwrap(),
    );
    let workspace = WorkspaceBuilder::new()
        .allowed_documents(provider.logical_uris())
        .document_provider(provider)
        .build()
        .unwrap();
    let root = workspace.open(requested.as_str()).unwrap();
    assert_eq!(root.uri(), &effective);
    assert_eq!(root.id(), workspace.open(effective.as_str()).unwrap().id());
    match root.resolve_edge(0).unwrap() {
        Resolution::Node(node) => {
            assert_eq!(node.get("type").unwrap().as_str(), Some("string"));
            assert_eq!(node.syntax().doc().uri(), &dependency);
        }
        other => panic!("expected the remote-relative leaf, got {other:?}"),
    }
    assert_eq!(workspace.len(), 2, "aliases do not consume document slots");
    assert_eq!(workspace.uris(), vec![dependency, effective]);
}
