//! Source-backed resource and dynamic-reference oracles from JSON Schema Core
//! 2020-12 §§8–9, RFC 3986 §5.4, and OAS 3.2 Appendix F. Expected URI strings,
//! physical pointers and dynamic distinctions are independent literal witnesses.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde_json::{Value, json};
use suspect_ir::contract::{
    AnchorKind, Contract, ContractReader, ResourceKind, SchemaDialect, SourceId,
};
use suspect_low::Pointer;
use suspect_ref::{DocumentProvider, ProvidedDocument, Workspace, WorkspaceBuilder};
use suspect_source::Uri;

const ENTRY: &str = "https://retrieve.example/v1/api.json";

fn api(schemas: Value) -> Value {
    json!({"openapi":"3.2.0","info":{"title":"Resources","version":"1"},"paths":{},"components":{"schemas":schemas}})
}

fn source(uri: &str, pointer: &str) -> SourceId {
    SourceId::new(Uri::parse(uri).unwrap(), Pointer::parse(pointer).unwrap())
}

fn at(pointer: &str) -> SourceId {
    source(ENTRY, pointer)
}

fn workspace(files: &[(&str, &str, Value)]) -> Arc<Workspace> {
    let provider = Arc::new(
        DocumentProvider::new(files.iter().map(|(requested, effective, value)| {
            ProvidedDocument::new(
                Uri::parse(requested).unwrap(),
                Uri::parse(effective).unwrap(),
                serde_json::to_vec_pretty(value).unwrap(),
            )
            .unwrap()
        }))
        .unwrap(),
    );
    Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    )
}

fn both(files: &[(&str, &str, Value)], check: impl Fn(&Contract, &Workspace)) {
    let mut contracts = Vec::new();
    for reader in [ContractReader::Lossless, ContractReader::Fast] {
        let workspace = workspace(files);
        let contract =
            Contract::from_workspace_with_reader(&workspace, &Uri::parse(ENTRY).unwrap(), reader)
                .unwrap();
        check(&contract, &workspace);
        contracts.push(contract);
    }
    let left = &contracts[0];
    let right = &contracts[1];
    assert_eq!(
        left.documents().collect::<Vec<_>>(),
        right.documents().collect::<Vec<_>>()
    );
    assert_eq!(
        left.resources().collect::<Vec<_>>(),
        right.resources().collect::<Vec<_>>()
    );
    assert_eq!(left.schema_roots(), right.schema_roots());
    let graph = |contract: &Contract| {
        contract
            .schemas()
            .map(|schema| {
                (
                    schema.id().clone(),
                    (
                        schema.raw().clone(),
                        schema.references().to_vec(),
                        schema.children().to_vec(),
                        schema.dialect().clone(),
                        schema.dialect_source().cloned(),
                        schema.default_dialect_source().cloned(),
                        schema.span(),
                        contract.resource_scope(schema.id()).cloned(),
                        contract.dynamic_reference(schema.id()).cloned(),
                    ),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    assert_eq!(graph(left), graph(right));
    let diagnostics = |contract: &Contract| {
        contract
            .diagnostics()
            .iter()
            .map(|d| {
                (
                    d.source.clone(),
                    d.at.clone(),
                    d.code,
                    d.severity,
                    d.message.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(diagnostics(left), diagnostics(right));
}

fn clean(contract: &Contract) {
    assert!(!contract.has_errors(), "{:#?}", contract.diagnostics());
}

fn error(contract: &Contract, source: &SourceId, code: &str) {
    let diagnostic = contract
        .diagnostics()
        .iter()
        .find(|d| &d.source == source && d.code == code)
        .unwrap_or_else(|| {
            panic!(
                "missing {code} at {source:?}: {:#?}",
                contract.diagnostics()
            )
        });
    assert!(!diagnostic.at.is_empty(), "{diagnostic:?}");
    assert!(contract.source(&diagnostic.source).is_some());
}

#[test]
fn nested_ids_use_resource_local_pointers_anchors_empty_fragments_and_physical_ids() {
    both(
        &[(
            ENTRY,
            ENTRY,
            api(json!({
                "Root":{"$id":"HTTPS://SCHEMAS.example/a/root","$defs":{
                    "Child":{"$id":"../other/child#","$anchor":"same","type":"object","$defs":{"leaf":{"type":"string"}},"properties":{
                        "self":{"$ref":"#"},"leaf":{"$ref":"#%2F$defs%2Fleaf"}
                    }},
                    "Other":{"$id":"other","$anchor":"same","type":"integer"}
                },"allOf":[{"$ref":"../other/child#same"}]},
                "Physical":{"$ref":"#/components/schemas/Root/$defs/Child"}
            })),
        )],
        |contract, _| {
            clean(contract);
            let root = at("/components/schemas/Root");
            let child = at("/components/schemas/Root/$defs/Child");
            let other = at("/components/schemas/Root/$defs/Other");
            assert_eq!(
                contract.resource(&root).unwrap().canonical_uri(),
                "https://schemas.example/a/root"
            );
            assert_eq!(
                contract.resource(&child).unwrap().canonical_uri(),
                "https://schemas.example/other/child"
            );
            assert_eq!(
                contract.resource(&other).unwrap().canonical_uri(),
                "https://schemas.example/a/other"
            );
            assert_eq!(
                contract.resource(&child).unwrap().kind(),
                ResourceKind::Schema
            );
            assert_eq!(
                contract.resource(&child).unwrap().declaration_source(),
                Some(&child.child("$id"))
            );
            assert_eq!(
                contract.reference_target(&child.child("properties").child("self")),
                Some(&child)
            );
            assert_eq!(
                contract.reference_target(&child.child("properties").child("leaf")),
                Some(&child.child("$defs").child("leaf"))
            );
            assert_eq!(
                contract.reference_target(&root.child("allOf").child("0")),
                Some(&child)
            );
            assert_eq!(
                contract.reference_target(&at("/components/schemas/Physical")),
                Some(&child)
            );
            let scope = contract
                .resource_scope(&child.child("properties").child("self"))
                .unwrap();
            assert_eq!(scope.resource(), &child);
            assert_eq!(scope.schema_root(), Some(&child));
            assert_eq!(scope.base_uri(), "https://schemas.example/other/child");
            assert_eq!(scope.base_source(), Some(&child.child("$id")));
            assert_eq!(
                scope.address(),
                "https://schemas.example/other/child#/properties/self"
            );
            assert_eq!(
                contract.resolve_resource_reference(&child, ""),
                Ok(child.clone())
            );
            assert_eq!(
                contract.resolve_resource_reference(&child, "#"),
                Ok(child.clone())
            );
            assert_eq!(
                contract.resolve_resource_reference(&child, "#same"),
                Ok(child.clone())
            );
            assert_eq!(
                contract.resolve_resource_reference(&other, "#same"),
                Ok(other)
            );
            assert!(
                contract
                    .resolve_resource_reference(&child, "#/components/schemas/Root")
                    .is_err()
            );
            assert_eq!(child.document().as_str(), ENTRY);
            assert!(contract.reachable_from(&[root]).len() < 20);
        },
    );
}

#[test]
fn uri_resolution_keeps_opaque_schemes_encoded_path_data_and_queries_distinct() {
    for (base, declared, reference, expected) in [
        ("http://a/b/c/d;p?q", "../g", "../g#", "http://a/b/g"),
        ("http://a/b/c/d;p?q", "http:g", "http:g", "http:g"),
        (
            "urn:example:schema",
            "?v=2",
            "?v=2",
            "urn:example:schema?v=2",
        ),
        (
            "https://a.test/a/",
            "%2E%2E/x",
            "%2E%2E/x",
            "https://a.test/a/%2E%2E/x",
        ),
        (
            "https://a.test/a/",
            "target%2Fpart",
            "target%2Fpart",
            "https://a.test/a/target%2Fpart",
        ),
        (
            "https://a.test/a/",
            "target%23part",
            "target%23part",
            "https://a.test/a/target%23part",
        ),
        (
            "https://a.test/root",
            "?q=a%2Fb+z&x=%23?ok",
            "?q=a%2Fb+z&x=%23?ok",
            "https://a.test/root?q=a%2Fb+z&x=%23?ok",
        ),
        (
            "https://a.test/root",
            "HTTPS://User:Pass@EXAMPLE.test/Target?Key=Value",
            "https://User:Pass@example.test/Target?Key=Value",
            "https://User:Pass@example.test/Target?Key=Value",
        ),
    ] {
        both(
            &[(
                ENTRY,
                ENTRY,
                api(
                    json!({"Root":{"$id":base,"$defs":{"target":{"$id":declared,"type":"string"}},"$ref":reference}}),
                ),
            )],
            |contract, _| {
                clean(contract);
                let root = at("/components/schemas/Root");
                let target = root.child("$defs").child("target");
                assert_eq!(contract.reference_target(&root), Some(&target));
                assert_eq!(
                    contract.resource(&target).unwrap().canonical_uri(),
                    expected
                );
                assert_eq!(
                    contract.resource_scope(&target).unwrap().address(),
                    expected
                );
            },
        );
    }
}

#[test]
fn redirected_self_documents_resolve_canonical_dependencies_without_rewriting_sources() {
    let requested = "https://pins.example/latest-api.json";
    let physical_parts = "https://pins.example/releases/parts.json";
    let physical_model = "https://pins.example/releases/model.json";
    let files = [
        (
            requested,
            ENTRY,
            json!({"openapi":"3.2.0","$self":"../canonical/api.json","info":{"title":"Self","version":"1"},
            "paths":{"/thing":{"$ref":"parts.json#/components/pathItems/Thing"}}}),
        ),
        (
            "https://pins.example/latest-parts.json",
            physical_parts,
            json!({"openapi":"3.2.0","$self":"https://retrieve.example/canonical/parts.json","info":{"title":"Parts","version":"1"},
            "components":{"pathItems":{"Thing":{"query":{"requestBody":{"content":{"application/json":{"schema":{"$ref":"schemas/thing"}}}}}}}}}),
        ),
        (
            "https://pins.example/latest-model.json",
            physical_model,
            json!({"$id":"https://retrieve.example/canonical/schemas/thing","type":"string"}),
        ),
    ];
    both(&files, |contract, workspace| {
        clean(contract);
        assert_eq!(contract.entry().as_str(), ENTRY);
        assert_eq!(contract.documents().count(), 3);
        let document = contract.resource(&at("")).unwrap();
        assert_eq!(
            document.canonical_uri(),
            "https://retrieve.example/canonical/api.json"
        );
        assert_eq!(
            document.base_uri(),
            "https://retrieve.example/canonical/api.json"
        );
        assert_eq!(document.declaration_source(), Some(&at("/$self")));
        assert!(document.aliases().contains(&requested.to_owned()));
        assert!(document.aliases().contains(&ENTRY.to_owned()));
        assert!(contract.document(&Uri::parse(requested).unwrap()).is_none());
        let operation = contract.operations().next().unwrap();
        assert_eq!(
            operation.source(),
            &source(physical_parts, "/components/pathItems/Thing/query")
        );
        assert_eq!(operation.path_item_source(), &at("/paths/~1thing"));
        let schema = operation.request_body().unwrap().content()[0]
            .schema()
            .unwrap();
        assert_eq!(
            contract.reference_target(schema.id()),
            Some(&source(physical_model, ""))
        );
        assert_eq!(
            contract
                .resource(&source(physical_model, ""))
                .unwrap()
                .base_uri(),
            "https://retrieve.example/canonical/schemas/thing"
        );
        assert!(!workspace.uris().iter().any(|uri| {
            uri.as_str()
                .starts_with("https://retrieve.example/canonical/")
        }));
        assert!(workspace.failed_document_uris().is_empty());
    });
    for reader in [ContractReader::Lossless, ContractReader::Fast] {
        let workspace = workspace(&files);
        let contract = Contract::from_workspace_with_reader(
            &workspace,
            &Uri::parse(requested).unwrap(),
            reader,
        )
        .unwrap();
        clean(&contract);
        assert_eq!(contract.entry().as_str(), ENTRY);
    }
}

#[test]
fn relative_ids_in_redirected_dependencies_use_effective_retrieval_not_lookup_aliases() {
    let effective = "https://pins.example/v3/schema.json";
    both(
        &[
            (
                ENTRY,
                ENTRY,
                api(json!({"Use":{"$ref":"https://pins.example/latest.json"}})),
            ),
            (
                "https://pins.example/latest.json",
                effective,
                json!({"$id":"models/root","$ref":"../leaf.json"}),
            ),
            (
                "https://pins.example/v3/leaf.json",
                "https://pins.example/v3/leaf.json",
                json!({"type":"string"}),
            ),
            (
                "https://pins.example/leaf.json",
                "https://pins.example/leaf.json",
                json!({"type":"integer"}),
            ),
        ],
        |contract, _| {
            clean(contract);
            let root = source(effective, "");
            assert_eq!(
                contract.resource(&root).unwrap().canonical_uri(),
                "https://pins.example/v3/models/root"
            );
            assert_eq!(
                contract.reference_target(&root),
                Some(&source("https://pins.example/v3/leaf.json", ""))
            );
            assert_eq!(contract.documents().count(), 3);
        },
    );
}

#[test]
fn same_bytes_hosted_at_two_uris_keep_independent_resource_and_dependency_identity() {
    let value = json!({"$id":"model","$ref":"leaf.json"});
    both(
        &[
            (
                ENTRY,
                ENTRY,
                api(
                    json!({"A":{"$ref":"https://a.test/schema.json"},"B":{"$ref":"https://b.test/schema.json"}}),
                ),
            ),
            (
                "https://a.test/schema.json",
                "https://a.test/schema.json",
                value.clone(),
            ),
            (
                "https://b.test/schema.json",
                "https://b.test/schema.json",
                value,
            ),
            (
                "https://a.test/leaf.json",
                "https://a.test/leaf.json",
                json!({"type":"string"}),
            ),
            (
                "https://b.test/leaf.json",
                "https://b.test/leaf.json",
                json!({"type":"integer"}),
            ),
        ],
        |contract, _| {
            clean(contract);
            for (document, canonical, leaf, kind) in [
                (
                    "https://a.test/schema.json",
                    "https://a.test/model",
                    "https://a.test/leaf.json",
                    "string",
                ),
                (
                    "https://b.test/schema.json",
                    "https://b.test/model",
                    "https://b.test/leaf.json",
                    "integer",
                ),
            ] {
                let root = source(document, "");
                assert_eq!(contract.resource(&root).unwrap().canonical_uri(), canonical);
                assert_eq!(contract.reference_target(&root), Some(&source(leaf, "")));
                assert_eq!(
                    contract.schema(&source(leaf, "")).unwrap().raw()["type"],
                    kind
                );
            }
            assert_eq!(contract.documents().count(), 5);
        },
    );
}

#[test]
fn duplicate_canonical_ids_are_errors_even_with_identical_bytes_and_reversed_discovery() {
    for reverse in [false, true] {
        let (first, second) = if reverse { ("B", "A") } else { ("A", "B") };
        both(
            &[
                (
                    ENTRY,
                    ENTRY,
                    api(
                        json!({first:{"$ref":"https://pins.example/a.json"},second:{"$ref":"urn:shared"}}),
                    ),
                ),
                (
                    "https://pins.example/a.json",
                    "https://pins.example/a.json",
                    json!({"$id":"urn:shared","type":"string"}),
                ),
                (
                    "https://pins.example/b.json",
                    "https://pins.example/b.json",
                    json!({"$id":"urn:shared","type":"string"}),
                ),
            ],
            |contract, _| {
                assert!(contract.has_errors());
                for document in ["https://pins.example/a.json", "https://pins.example/b.json"] {
                    error(contract, &source(document, ""), "invalid-resource-identity");
                }
                let use_site = at("/components/schemas").child(second);
                assert!(contract.reference_target(&use_site).is_none());
                assert!(
                    contract
                        .resolve_resource_reference(&use_site, "urn:shared")
                        .is_err()
                );
                assert_eq!(
                    contract
                        .resources()
                        .filter(|r| r.canonical_uri() == "urn:shared")
                        .count(),
                    2
                );
            },
        );
    }
}

#[test]
fn a_canonical_id_cannot_silently_shadow_an_authorized_retrieval_document() {
    both(
        &[
            (
                ENTRY,
                ENTRY,
                api(
                    json!({"A":{"$ref":"https://pins.example/a.json"},"B":{"$ref":"https://pins.example/b.json"}}),
                ),
            ),
            (
                "https://pins.example/a.json",
                "https://pins.example/a.json",
                json!({"$id":"https://pins.example/b.json","type":"integer"}),
            ),
            (
                "https://pins.example/b.json",
                "https://pins.example/b.json",
                json!({"type":"string"}),
            ),
        ],
        |contract, _| {
            error(
                contract,
                &source("https://pins.example/a.json", ""),
                "invalid-resource-identity",
            );
            error(
                contract,
                &source("https://pins.example/b.json", ""),
                "invalid-resource-identity",
            );
            assert!(
                contract
                    .reference_target(&at("/components/schemas/B"))
                    .is_none()
            );
        },
    );
}

#[test]
fn anchors_are_resource_local_and_instance_identifier_words_do_not_participate() {
    both(
        &[(
            ENTRY,
            ENTRY,
            api(json!({
                "A":{"$id":"urn:a","$anchor":"node","type":"string"},
                "B":{"$id":"urn:b","$anchor":"node","type":"integer"},
                "UseA":{"$ref":"urn:a#n%6fde"},"UseB":{"$ref":"urn:b#node"},
                "Instances":{"examples":[{"$id":"urn:a","$anchor":"node","$dynamicAnchor":"node","$ref":"https://never-fetch.test/instance"}],
                    "default":{"$id":"urn:b","schema":{"$id":"urn:a"}},"custom":{"$id":"urn:a"}}
            })),
        )],
        |contract, workspace| {
            clean(contract);
            assert_eq!(
                contract.reference_target(&at("/components/schemas/UseA")),
                Some(&at("/components/schemas/A"))
            );
            assert_eq!(
                contract.reference_target(&at("/components/schemas/UseB")),
                Some(&at("/components/schemas/B"))
            );
            assert_eq!(
                contract
                    .resource(&at("/components/schemas/A"))
                    .unwrap()
                    .anchors()[0]
                    .source(),
                &at("/components/schemas/A/$anchor")
            );
            assert_eq!(
                contract
                    .resource(&at("/components/schemas/A"))
                    .unwrap()
                    .anchors()[0]
                    .kind(),
                AnchorKind::Static
            );
            assert!(
                !contract.is_schema_position(&at("/components/schemas/Instances/default/schema"))
            );
            assert!(workspace.failed_document_uris().is_empty());
        },
    );
    both(
        &[(
            ENTRY,
            ENTRY,
            api(
                json!({"Root":{"$id":"urn:dup","$anchor":"same","$dynamicAnchor":"same"},"Use":{"$ref":"urn:dup#same"}}),
            ),
        )],
        |contract, _| {
            error(
                contract,
                &at("/components/schemas/Root"),
                "invalid-schema-anchor",
            );
            assert!(
                contract
                    .reference_target(&at("/components/schemas/Use"))
                    .is_none()
            );
        },
    );
}

#[test]
fn dynamic_metadata_retains_initial_targets_resource_candidates_and_static_fallback_cases() {
    both(
        &[(
            ENTRY,
            ENTRY,
            api(json!({
                "Tree":{"$id":"https://schemas.test/tree","$anchor":"plain","$dynamicAnchor":"node","type":"object","properties":{"children":{"type":"array","items":{"$dynamicRef":"#node"}}}},
                "Strict":{"$id":"https://schemas.test/strict-tree","$dynamicAnchor":"node","$ref":"tree"},
                "Ordinary":{"$ref":"https://schemas.test/tree#node"},
                "Plain":{"$dynamicRef":"https://schemas.test/tree#plain"},
                "Pointer":{"$dynamicRef":"https://schemas.test/tree#/properties/children/items"},
                "Empty":{"$dynamicRef":"https://schemas.test/tree#"},
                "Encoded":{"$dynamicRef":"https://schemas.test/tree#n%6fde"}
            })),
        )],
        |contract, _| {
            clean(contract);
            let tree = at("/components/schemas/Tree");
            let strict = at("/components/schemas/Strict");
            let recursive = tree.child("properties").child("children").child("items");
            for site in [&recursive, &at("/components/schemas/Encoded")] {
                let reference = contract.dynamic_reference(site).unwrap();
                assert_eq!(reference.source(), &site.child("$dynamicRef"));
                assert_eq!(reference.initial_target(), Some(&tree));
                assert_eq!(reference.initial_resource(), Some(&tree));
                assert_eq!(reference.dynamic_anchor(), Some("node"));
                assert_eq!(
                    reference
                        .candidates()
                        .iter()
                        .map(|a| a.resource().clone())
                        .collect::<BTreeSet<_>>(),
                    BTreeSet::from([tree.clone(), strict.clone()])
                );
                assert!(contract.reference_target(site).is_none());
                let edge = contract
                    .schema(site)
                    .unwrap()
                    .references()
                    .iter()
                    .find(|r| r.keyword == "$dynamicRef")
                    .unwrap();
                assert_eq!(edge.target, Some(tree.clone()));
            }
            assert_eq!(contract.reference_target(&strict), Some(&tree));
            assert_eq!(
                contract.reference_target(&at("/components/schemas/Ordinary")),
                Some(&tree)
            );
            for (name, target) in [
                ("Plain", tree.clone()),
                ("Empty", tree.clone()),
                ("Pointer", recursive),
            ] {
                let reference = contract
                    .dynamic_reference(&at("/components/schemas").child(name))
                    .unwrap();
                assert_eq!(reference.initial_target(), Some(&target));
                assert!(reference.dynamic_anchor().is_none());
                assert!(reference.candidates().is_empty());
            }
        },
    );
}

#[test]
fn a_malformed_dynamic_edge_does_not_replace_or_clear_the_static_edge_on_the_same_schema() {
    both(
        &[(
            ENTRY,
            ENTRY,
            api(
                json!({"Target":{"type":"string"},"Use":{"$ref":"#/components/schemas/Target","$dynamicRef":null}}),
            ),
        )],
        |contract, _| {
            let use_site = at("/components/schemas/Use");
            error(contract, &use_site, "invalid-dynamic-reference");
            assert_eq!(
                contract.reference_target(&use_site),
                Some(&at("/components/schemas/Target"))
            );
            assert!(
                contract
                    .dynamic_reference(&use_site)
                    .unwrap()
                    .initial_target()
                    .is_none()
            );
            assert_eq!(contract.schema(&use_site).unwrap().references().len(), 2);
        },
    );
}

#[test]
fn invalid_identifiers_do_not_fall_back_to_a_parent_base_or_silently_repair_uri_syntax() {
    for value in [
        json!(null),
        json!(42),
        json!("#named"),
        json!(" leading"),
        json!("bad%"),
        json!("sub\\resource"),
        json!("café"),
        json!("https://example.test:port/a"),
    ] {
        both(
            &[(ENTRY, ENTRY, api(json!({"Root":{"$id":value,"$ref":"#"}})))],
            |contract, _| {
                let root = at("/components/schemas/Root");
                error(contract, &root, "invalid-schema-resource");
                assert!(contract.resource_scope(&root).is_none());
                assert!(contract.reference_target(&root).is_none());
                assert!(contract.resolve_resource_reference(&root, "#").is_err());
            },
        );
    }
    for value in [
        json!(null),
        json!(17),
        json!("leading space "),
        json!("https://bad.test/#bad%"),
    ] {
        let mut document = api(json!({"S":{"type":"string"}}));
        document["$self"] = value;
        both(&[(ENTRY, ENTRY, document)], |contract, _| {
            error(contract, &at("/$self"), "invalid-openapi-self");
            assert!(
                contract
                    .resource_scope(&at("/components/schemas/S"))
                    .is_none()
            );
        });
    }
}

#[test]
fn unknown_canonical_targets_do_not_authorize_loading_hidden_provider_or_filesystem_bytes() {
    for reader in [ContractReader::Lossless, ContractReader::Fast] {
        let hidden = "https://pins.example/hidden.json";
        let provider = Arc::new(
            DocumentProvider::new([
                ProvidedDocument::new(
                    Uri::parse(ENTRY).unwrap(),
                    Uri::parse(ENTRY).unwrap(),
                    serde_json::to_vec(&api(json!({"Use":{"$ref":"urn:hidden"}}))).unwrap(),
                )
                .unwrap(),
                ProvidedDocument::new(
                    Uri::parse(hidden).unwrap(),
                    Uri::parse(hidden).unwrap(),
                    br#"{"$id":"urn:hidden","type":"string"}"#.to_vec(),
                )
                .unwrap(),
            ])
            .unwrap(),
        );
        let workspace = Arc::new(
            WorkspaceBuilder::new()
                .allowed_documents([Uri::parse(ENTRY).unwrap()])
                .document_provider(provider)
                .build()
                .unwrap(),
        );
        assert_eq!(
            workspace.available_document_uris(),
            [Uri::parse(ENTRY).unwrap()]
        );
        assert_eq!(workspace.len(), 0);
        let contract =
            Contract::from_workspace_with_reader(&workspace, &Uri::parse(ENTRY).unwrap(), reader)
                .unwrap();
        error(
            &contract,
            &at("/components/schemas/Use"),
            "invalid-reference",
        );
        assert_eq!(workspace.uris(), [Uri::parse(ENTRY).unwrap()]);
        assert!(workspace.failed_document_uris().is_empty());
        assert!(
            contract
                .resolve_resource_reference(&at("/components/schemas/Use"), "urn:hidden")
                .is_err()
        );
        assert_eq!(workspace.len(), 1);
    }
    both(
        &[(
            ENTRY,
            ENTRY,
            api(json!({"Use":{"$id":"https://canonical.test/root","$ref":"unavailable"}})),
        )],
        |contract, workspace| {
            error(
                contract,
                &at("/components/schemas/Use"),
                "invalid-reference",
            );
            assert_eq!(workspace.uris(), [Uri::parse(ENTRY).unwrap()]);
            assert!(workspace.failed_document_uris().is_empty());
        },
    );
}

#[test]
fn embedded_schema_root_and_dialect_origins_remain_separate_from_the_uri_boundary() {
    let remote = "https://pins.example/remote.json";
    both(
        &[
            (
                ENTRY,
                ENTRY,
                api(json!({"Use":{"$ref":"https://schemas.test/nested#/properties/value"}})),
            ),
            (
                remote,
                remote,
                json!({"openapi":"3.1.2","jsonSchemaDialect":"https://spec.openapis.org/oas/3.1/dialect/base","info":{"title":"Dialect","version":"1"},
            "components":{"schemas":{"Root":{"$id":"https://schemas.test/root","$defs":{"Nested":{"$id":"nested","$schema":"https://json-schema.org/draft/2020-12/schema","properties":{"value":{"type":"string"}}}}}}}}),
            ),
        ],
        |contract, _| {
            clean(contract);
            let nested = source(remote, "/components/schemas/Root/$defs/Nested");
            let leaf = nested.child("properties").child("value");
            let schema = contract.schema(&leaf).unwrap();
            assert_eq!(
                schema.dialect(),
                &SchemaDialect::Uri("https://json-schema.org/draft/2020-12/schema".to_owned())
            );
            assert_eq!(schema.dialect_source(), Some(&nested.child("$schema")));
            assert_eq!(
                schema.default_dialect_source(),
                Some(&source(remote, "/jsonSchemaDialect"))
            );
            assert_eq!(
                contract.schema_contexts(&leaf)[0].version_source(),
                &source(remote, "/openapi")
            );
            let scope = contract.resource_scope(&leaf).unwrap();
            assert_eq!(scope.resource(), &nested);
            assert_eq!(scope.schema_root(), Some(&nested));
            assert_eq!(scope.base_source(), Some(&nested.child("$id")));
            assert_eq!(
                scope.address(),
                "https://schemas.test/nested#/properties/value"
            );
            assert!(contract.is_schema_position(&nested));
        },
    );
    both(
        &[(
            ENTRY,
            ENTRY,
            api(
                json!({"Inline":{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"string"}}),
            ),
        )],
        |contract, _| {
            clean(contract);
            let inline = at("/components/schemas/Inline");
            let scope = contract.resource_scope(&inline).unwrap();
            assert_eq!(scope.resource(), &at(""));
            assert_eq!(scope.schema_root(), Some(&inline));
            assert_eq!(
                scope.address(),
                "https://retrieve.example/v1/api.json#/components/schemas/Inline"
            );
            assert_eq!(
                contract.resource(scope.resource()).unwrap().kind(),
                ResourceKind::OpenApiDocument
            );
        },
    );
}

#[test]
fn legacy_reference_siblings_and_unsupported_dialects_do_not_gain_modern_resource_semantics() {
    let mut document = api(
        json!({"Target":{"type":"string"},"Use":{"$ref":"#/components/schemas/Target","$id":"urn:ignored","$dynamicAnchor":"node","$dynamicRef":"urn:ignored#node","properties":{"ignored":{"$ref":"https://never-fetch.test/ignored"}}}}),
    );
    document["openapi"] = json!("3.0.4");
    both(&[(ENTRY, ENTRY, document)], |contract, workspace| {
        let root = at("/components/schemas/Use");
        let target = at("/components/schemas/Target");
        assert!(contract.schema(&root).unwrap().ignores_ref_siblings());
        assert_eq!(contract.reference_target(&root), Some(&target));
        assert!(contract.dynamic_reference(&root).is_none());
        assert!(
            !contract
                .resources()
                .any(|resource| resource.canonical_uri() == "urn:ignored")
        );
        assert_eq!(
            contract.effective_schema_closure(std::slice::from_ref(&root)),
            vec![target, root]
        );
        assert!(workspace.failed_document_uris().is_empty());
        assert!(
            contract
                .schema(&at("/components/schemas/Use/properties/ignored"))
                .is_none()
        );
    });
    both(
        &[(
            ENTRY,
            ENTRY,
            api(
                json!({"Unknown":{"$schema":"https://example.test/new-dialect","$id":"urn:unknown","$ref":"urn:unknown"}}),
            ),
        )],
        |contract, workspace| {
            let root = at("/components/schemas/Unknown");
            error(contract, &root, "unsupported-schema-dialect");
            error(contract, &root, "unsupported-resource-dialect");
            assert!(contract.resource_scope(&root).is_none());
            assert!(
                !contract
                    .resources()
                    .any(|resource| resource.canonical_uri() == "urn:unknown")
            );
            assert!(workspace.failed_document_uris().is_empty());
        },
    );
}

#[test]
fn dynamic_candidates_outside_a_selected_fragment_are_indexed_with_their_dependencies() {
    let outer = "https://pins.example/outer.json";
    let generic = "https://pins.example/generic.json";
    let extra = "https://pins.example/extra.json";
    both(
        &[
            (
                ENTRY,
                ENTRY,
                api(
                    json!({"Use":{"$ref":"https://pins.example/outer.json#/$defs/start"},
            "Unrelated":{"$id":"urn:unrelated","$dynamicAnchor":"node","type":"boolean"}}),
                ),
            ),
            (
                outer,
                outer,
                json!({"$id":"urn:outer","$defs":{
                    "binding":{"$dynamicAnchor":"node","$ref":"urn:extra"},
                    "start":{"$ref":"urn:generic#/$defs/use"}
                }}),
            ),
            (
                generic,
                generic,
                json!({"$id":"urn:generic","$defs":{
                    "node":{"$dynamicAnchor":"node","type":"string"},
                    "use":{"$dynamicRef":"#node"}
                }}),
            ),
            (extra, extra, json!({"$id":"urn:extra","type":"integer"})),
        ],
        |contract, _| {
            clean(contract);
            let binding = source(outer, "/$defs/binding");
            let dynamic = source(generic, "/$defs/use");
            assert!(contract.schema(&binding).is_some());
            assert_eq!(
                contract.reference_target(&binding),
                Some(&source(extra, ""))
            );
            assert_eq!(
                contract
                    .dynamic_reference(&dynamic)
                    .unwrap()
                    .initial_target(),
                Some(&source(generic, "/$defs/node"))
            );
            assert!(
                contract
                    .dynamic_reference(&dynamic)
                    .unwrap()
                    .candidates()
                    .iter()
                    .any(|anchor| anchor.target() == &binding)
            );
            let closure = contract.effective_schema_closure(&[at("/components/schemas/Use")]);
            assert!(closure.contains(&binding));
            assert!(closure.contains(&source(extra, "")));
            assert!(!closure.contains(&at("/components/schemas/Unrelated")));
            assert!(
                !contract
                    .reachable_from(&[at("/components/schemas/Use")])
                    .contains(&binding)
            );
        },
    );
}

#[test]
fn yaml_and_json_resources_share_exact_source_addresses_and_decode_fragments_once() {
    let uri = Uri::parse("https://retrieve.example/v1/api.yaml").unwrap();
    let yaml = br##"openapi: 3.2.0
$self: ../canonical/api
info: {title: Resource pointers, version: '1'}
components:
  schemas:
    Root:
      $id: models/root
      $defs:
        Value:
          $id: child
          properties:
            "caf\u00E9/~% #":
              type: integer
              maximum: 18446744073709551617
          $ref: '#/properties/caf%C3%A9~1~0%25%20%23'
"##;
    let mut contracts = Vec::new();
    let mut json_bytes = None;
    for json in [false, true] {
        let bytes = if json {
            json_bytes.clone().unwrap()
        } else {
            yaml.to_vec()
        };
        for reader in [ContractReader::Lossless, ContractReader::Fast] {
            let provider = Arc::new(
                DocumentProvider::new([ProvidedDocument::new(
                    uri.clone(),
                    uri.clone(),
                    bytes.clone(),
                )
                .unwrap()])
                .unwrap(),
            );
            let workspace = Arc::new(
                WorkspaceBuilder::new()
                    .allowed_documents(provider.logical_uris())
                    .document_provider(provider)
                    .build()
                    .unwrap(),
            );
            let contract = Contract::from_workspace_with_reader(&workspace, &uri, reader).unwrap();
            clean(&contract);
            if json_bytes.is_none() {
                json_bytes = Some(serde_json::to_vec(contract.document(&uri).unwrap()).unwrap());
            }
            let root = SourceId::new(
                uri.clone(),
                Pointer::parse("/components/schemas/Root/$defs/Value").unwrap(),
            );
            let leaf = root.child("properties").child("café/~% #");
            assert_eq!(contract.reference_target(&root), Some(&leaf));
            assert_eq!(
                contract.resource_scope(&leaf).unwrap().address(),
                "https://retrieve.example/canonical/models/child#/properties/caf%C3%A9~1~0%25%20%23"
            );
            assert_eq!(
                contract.schema(&leaf).unwrap().raw()["maximum"].to_string(),
                "18446744073709551617"
            );
            assert_eq!(
                contract.resolve_resource_reference(&root, "#/properties/caf%C3%A9~1~0%25%20%23"),
                Ok(leaf.clone())
            );
            assert!(
                contract
                    .resolve_resource_reference(&root, "#/properties/caf%25C3%25A9~1~0%25%20%23")
                    .is_err()
            );
            contracts.push(contract);
        }
    }
    assert_eq!(contracts[0].schema_roots(), contracts[3].schema_roots());
    assert_eq!(
        contracts[0].resources().collect::<Vec<_>>(),
        contracts[3].resources().collect::<Vec<_>>()
    );
}

#[test]
fn self_fragment_identity_and_empty_id_fragments_preserve_the_fragment_free_base() {
    let mut document = api(json!({"S":{"$id":"model#","type":"string"}}));
    document["$self"] = json!("https://logical.test/api#document");
    both(&[(ENTRY, ENTRY, document)], |contract, _| {
        clean(contract);
        let resource = contract.resource(&at("")).unwrap();
        assert_eq!(
            resource.canonical_uri(),
            "https://logical.test/api#document"
        );
        assert_eq!(resource.base_uri(), "https://logical.test/api");
        assert_eq!(
            contract
                .resource(&at("/components/schemas/S"))
                .unwrap()
                .canonical_uri(),
            "https://logical.test/model"
        );
        assert_eq!(
            contract.resolve_resource_reference(&at(""), "#document"),
            Ok(at(""))
        );
        assert_eq!(
            contract.resolve_resource_reference(&at(""), "#/components/schemas/S"),
            Ok(at("/components/schemas/S"))
        );
    });
    both(
        &[(
            ENTRY,
            ENTRY,
            api(
                json!({"A":{"$id":"urn:empty#"},"B":{"$id":"urn:empty"},"Use":{"$ref":"urn:empty#"}}),
            ),
        )],
        |contract, _| {
            error(
                contract,
                &at("/components/schemas/A"),
                "invalid-resource-identity",
            );
            error(
                contract,
                &at("/components/schemas/B"),
                "invalid-resource-identity",
            );
            assert!(
                contract
                    .reference_target(&at("/components/schemas/Use"))
                    .is_none()
            );
        },
    );
}

#[test]
fn closed_providers_do_not_fall_back_to_existing_files_or_register_denied_lookup_aliases() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("outside.json");
    std::fs::write(&file, r#"{"$id":"urn:outside","type":"string"}"#).unwrap();
    let file_uri = Uri::from_path(&file).unwrap();
    both(
        &[(ENTRY, ENTRY, api(json!({"Use":{"$ref":file_uri.as_str()}})))],
        |contract, workspace| {
            error(
                contract,
                &at("/components/schemas/Use"),
                "invalid-reference",
            );
            assert!(workspace.get(&file_uri).is_none());
            assert!(workspace.failed_document_uris().is_empty());
            assert_eq!(workspace.len(), 1);
        },
    );
    let requested = "https://pins.example/denied-alias.json";
    for reader in [ContractReader::Lossless, ContractReader::Fast] {
        let provider = Arc::new(
            DocumentProvider::new([ProvidedDocument::new(
                Uri::parse(requested).unwrap(),
                Uri::parse(ENTRY).unwrap(),
                serde_json::to_vec(&api(json!({"S":{"type":"string"}}))).unwrap(),
            )
            .unwrap()])
            .unwrap(),
        );
        let workspace = Arc::new(
            WorkspaceBuilder::new()
                .allowed_documents([Uri::parse(ENTRY).unwrap()])
                .document_provider(provider)
                .build()
                .unwrap(),
        );
        let contract =
            Contract::from_workspace_with_reader(&workspace, &Uri::parse(ENTRY).unwrap(), reader)
                .unwrap();
        clean(&contract);
        assert_eq!(
            workspace.available_document_uris(),
            [Uri::parse(ENTRY).unwrap()]
        );
        assert!(
            !contract
                .resource(&at(""))
                .unwrap()
                .aliases()
                .contains(&requested.to_owned())
        );
        assert!(
            contract
                .resolve_resource_reference(&at("/components/schemas/S"), requested)
                .is_err()
        );
    }
}
