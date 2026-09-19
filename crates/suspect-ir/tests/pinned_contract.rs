//! Logical redirect aliases must never become Contract reference bases.

use std::sync::Arc;

use serde_json::{Value, json};
use suspect_ir::contract::{Contract, ContractReader, SourceId};
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

fn fixture(reference: &str) -> (Arc<suspect_ref::Workspace>, Uri, Uri, Uri, Uri) {
    let entry_alias = Uri::parse("https://schemas.example/latest-api.json").unwrap();
    let entry = Uri::parse("https://schemas.example/v2/api.json").unwrap();
    let model_alias = Uri::parse("https://schemas.example/latest-model.json").unwrap();
    let model = Uri::parse("https://schemas.example/v3/model.json").unwrap();
    let leaf = Uri::parse("https://schemas.example/v3/leaf.json").unwrap();
    let decoy = Uri::parse("https://schemas.example/leaf.json").unwrap();
    let document = |requested: Uri, effective: Uri, value: Value| {
        ProvidedDocument::new(requested, effective, serde_json::to_vec(&value).unwrap()).unwrap()
    };
    let provider=Arc::new(DocumentProvider::new([
        document(entry_alias.clone(),entry.clone(),json!({"openapi":"3.1.2","info":{"title":"Redirected references","version":"1"},
            "paths":{},"components":{"schemas":{"Lookup":{"$ref":"/latest-model.json"},"Direct":{"$ref":"/v3/model.json"}}}})),
        document(model_alias,model.clone(),json!({"$ref":reference})),
        document(leaf.clone(),leaf.clone(),json!({"$anchor":"value","type":"string"})),
        document(decoy.clone(),decoy,json!({"$anchor":"value","type":"boolean"})),
    ]).unwrap());
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    (workspace, entry_alias, entry, model, leaf)
}

#[test]
fn redirected_dependencies_use_effective_bases_and_one_canonical_document() {
    for reference in ["leaf.json", "leaf.json#value"] {
        for reader in [ContractReader::Lossless, ContractReader::Fast] {
            let (workspace, _, entry, model, leaf) = fixture(reference);
            let contract =
                Contract::from_workspace_with_reader(&workspace, &entry, reader).unwrap();
            assert!(!contract.has_errors(), "{:?}", contract.diagnostics());
            let lookup = SourceId::new(entry.clone(), Default::default())
                .child("components")
                .child("schemas")
                .child("Lookup");
            let root = SourceId::new(model.clone(), Default::default());
            assert_eq!(
                contract.schema(&lookup).unwrap().references()[0]
                    .target
                    .as_ref(),
                Some(&root)
            );
            assert_eq!(
                contract.schema(&root).unwrap().references()[0]
                    .target
                    .as_ref()
                    .unwrap()
                    .document(),
                &leaf
            );
            assert_eq!(contract.documents().count(), 3);
            assert!(
                contract
                    .document(&Uri::parse("https://schemas.example/latest-model.json").unwrap())
                    .is_none()
            );
            assert_eq!(
                contract
                    .schema(&SourceId::new(leaf, Default::default()))
                    .unwrap()
                    .raw()["type"],
                "string"
            );
        }
    }
}

#[test]
fn a_requested_entry_alias_becomes_the_effective_contract_entry() {
    let (workspace, requested, entry, _, _) = fixture("leaf.json");
    let contract = Contract::from_workspace(&workspace, &requested).unwrap();
    assert_eq!(contract.entry(), &entry);
    assert!(contract.document(&requested).is_none());
}

#[test]
fn ignored_reference_siblings_do_not_acquire_malformed_documents() {
    for (version, schema_reference) in [("3.0.4", true), ("3.1.2", false)] {
        let entry = Uri::parse("https://schemas.example/api.json").unwrap();
        let good = Uri::parse("https://schemas.example/good.json").unwrap();
        let broken = Uri::parse("https://schemas.example/broken.json").unwrap();
        let source = if schema_reference {
            json!({"openapi":version,"info":{"title":"Ignored siblings","version":"1"},"paths":{},
                "components":{"schemas":{"S":{"$ref":"good.json","properties":{"ignored":{"$ref":"broken.json"}}}}}})
        } else {
            json!({"openapi":version,"info":{"title":"Ignored siblings","version":"1"},"paths":{"/x":{"get":{
                "operationId":"getX","responses":{"200":{"$ref":"good.json","content":{"application/json":{"schema":{"$ref":"broken.json"}}}}}}}}})
        };
        let target = if schema_reference {
            json!({"type":"string"})
        } else {
            json!({"description":"ok"})
        };
        let provider = Arc::new(
            DocumentProvider::new([
                ProvidedDocument::new(
                    entry.clone(),
                    entry.clone(),
                    serde_json::to_vec(&source).unwrap(),
                )
                .unwrap(),
                ProvidedDocument::new(good.clone(), good, serde_json::to_vec(&target).unwrap())
                    .unwrap(),
                ProvidedDocument::new(broken.clone(), broken.clone(), b"{".to_vec()).unwrap(),
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
        let contract = Contract::from_workspace(&workspace, &entry)
            .expect("ignored siblings cannot cause acquisition or parsing failures");
        assert_eq!(contract.documents().count(), 2);
        assert!(!workspace.uris().contains(&broken));
    }
}
