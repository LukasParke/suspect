//! Dialect context must be explicit even at otherwise transparent reference nodes.

use serde_json::{Value, json};
use std::sync::Arc;
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_schema::{Config, OwnedCompiler};
use suspect_source::Uri;

fn load(documents: &[(&str, Value)]) -> Arc<Contract> {
    let provider = Arc::new(
        DocumentProvider::new(documents.iter().map(|(path, value)| {
            let uri = Uri::parse(&format!("https://example.test/{path}")).unwrap();
            ProvidedDocument::new(uri.clone(), uri, serde_json::to_vec(value).unwrap()).unwrap()
        }))
        .unwrap(),
    );
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    Arc::new(
        Contract::from_workspace(
            &workspace,
            &Uri::parse("https://example.test/api.json").unwrap(),
        )
        .unwrap(),
    )
}

fn api(version: &str, schemas: Value) -> Value {
    json!({"openapi":version,"info":{"title":"Context witness","version":"1"},"paths":{},"components":{"schemas":schemas}})
}

#[test]
fn ambiguous_reference_dialects_cannot_silently_drop_modern_sibling_assertions() {
    let contract = load(&[
        (
            "api.json",
            api(
                "3.1.0",
                json!({"Old":{"$ref":"old.json#/components/schemas/S"},"Modern":{"$ref":"bridge.json"}}),
            ),
        ),
        (
            "old.json",
            api("3.0.3", json!({"S":{"$ref":"shared.json"}})),
        ),
        ("bridge.json", json!({"$ref":"bridge2.json"})),
        ("bridge2.json", json!({"$ref":"shared.json"})),
        ("shared.json", json!({"$ref":"integer.json","minimum":5})),
        ("integer.json", json!({"type":"integer"})),
    ]);
    assert!(
        contract
            .diagnostics()
            .iter()
            .any(|finding| finding.code == "AMBIGUOUS_OPENAPI_CONTEXT")
    );
    let root = SourceId::new(contract.entry().clone(), Default::default())
        .child("components")
        .child("schemas")
        .child("Modern");
    let errors = OwnedCompiler::new(Config::default())
        .compile(contract, std::slice::from_ref(&root))
        .err()
        .expect(
            "ambiguous dialects must be refused instead of validating 1 against a lost minimum:5",
        );
    assert!(
        errors
            .iter()
            .any(|finding| finding.message.contains("AMBIGUOUS_OPENAPI_CONTEXT")),
        "{errors:?}"
    );
}

#[test]
fn referenced_fragment_uses_its_recorded_version_instead_of_the_entry_version() {
    let contract = load(&[
        (
            "api.json",
            api(
                "3.2.0",
                json!({"X":{"$ref":"old.json#/components/schemas/X"}}),
            ),
        ),
        (
            "old.json",
            api("3.1.0", json!({"X":{"$ref":"fragment.json"}})),
        ),
        (
            "fragment.json",
            json!({"type":"object","discriminator":{"propertyName":"tag","defaultMapping":"#/somewhere"}}),
        ),
    ]);
    let root = SourceId::new(
        Uri::parse("https://example.test/fragment.json").unwrap(),
        Default::default(),
    );
    assert_eq!(contract.openapi_version_at(&root), "3.1.0");
    let errors = OwnedCompiler::new(Config::default())
        .compile(contract, std::slice::from_ref(&root))
        .err()
        .expect("3.2 annotations cannot be silently applied to a 3.1 fragment");
    assert!(
        errors
            .iter()
            .any(|finding| finding.source.document() == root.document()
                && finding
                    .source
                    .pointer()
                    .ends_with("/discriminator/defaultMapping")),
        "{errors:?}"
    );
}

fn conflicting_defaults(json_short: bool, explicit: bool) -> Arc<Contract> {
    let mut json_api = api(
        "3.1.2",
        json!({"X":{"$ref":if json_short {"fragment.json"} else {"json-bridge.json"}}}),
    );
    json_api["jsonSchemaDialect"] = json!("https://json-schema.org/draft/2020-12/schema");
    let oas_api = api(
        "3.1.2",
        json!({"X":{"$ref":if json_short {"oas-bridge.json"} else {"fragment.json"}}}),
    );
    let mut fragment = json!({"type":"object","discriminator":false});
    if explicit {
        fragment["$schema"] = json!("https://json-schema.org/draft/2020-12/schema");
    }
    load(&[
        (
            "api.json",
            api(
                "3.1.2",
                json!({"Json":{"$ref":"json.json#/components/schemas/X"},"Oas":{"$ref":"oas.json#/components/schemas/X"}}),
            ),
        ),
        ("json.json", json_api),
        ("oas.json", oas_api),
        ("json-bridge.json", json!({"$ref":"fragment.json"})),
        ("oas-bridge.json", json!({"$ref":"fragment.json"})),
        ("fragment.json", fragment),
    ])
}

#[test]
fn same_version_conflicting_dialects_are_diagnosed_independently_of_discovery_order() {
    for json_short in [false, true] {
        let contract = conflicting_defaults(json_short, false);
        assert!(
            contract
                .diagnostics()
                .iter()
                .any(|finding| finding.code == "AMBIGUOUS_OPENAPI_CONTEXT"),
            "{:?}",
            contract.diagnostics()
        );
        let root = SourceId::new(
            Uri::parse("https://example.test/fragment.json").unwrap(),
            Default::default(),
        );
        let contexts = contract.schema_contexts(&root);
        assert!(
            contexts
                .iter()
                .any(|context| context.version_source().document().as_str()
                    == "https://example.test/json.json")
        );
        assert!(
            contexts
                .iter()
                .any(|context| context.version_source().document().as_str()
                    == "https://example.test/oas.json")
        );
        assert!(
            contexts.iter().any(
                |context| context
                    .dialect_source()
                    .is_some_and(|source| source.document().as_str()
                        == "https://example.test/json.json"
                        && source.pointer() == "/jsonSchemaDialect")
            )
        );
        let errors = OwnedCompiler::new(Config::default())
            .compile(contract, std::slice::from_ref(&root))
            .err()
            .expect("a shared schema cannot silently use the first of two dialects");
        assert!(
            errors
                .iter()
                .any(|finding| finding.message.contains("AMBIGUOUS_OPENAPI_CONTEXT"))
        );
    }
}

#[test]
fn an_explicit_fragment_dialect_overrides_both_inherited_defaults() {
    for json_short in [false, true] {
        let contract = conflicting_defaults(json_short, true);
        assert!(
            !contract
                .diagnostics()
                .iter()
                .any(|finding| finding.code == "AMBIGUOUS_OPENAPI_CONTEXT"),
            "{:?}",
            contract.diagnostics()
        );
        let root = SourceId::new(
            Uri::parse("https://example.test/fragment.json").unwrap(),
            Default::default(),
        );
        assert!(
            OwnedCompiler::new(Config::default())
                .compile(contract, &[root])
                .is_ok()
        );
    }
}

#[test]
fn a_30_fragment_does_not_inherit_the_unrelated_entry_dialect_declaration() {
    let mut entry = api(
        "3.2.0",
        json!({"X":{"$ref":"old.json#/components/schemas/X"}}),
    );
    entry["jsonSchemaDialect"] = json!("https://spec.openapis.org/oas/3.1/dialect/base");
    let contract = load(&[
        ("api.json", entry),
        (
            "old.json",
            api("3.0.4", json!({"X":{"$ref":"fragment.json"}})),
        ),
        ("fragment.json", json!({"type":"integer"})),
    ]);
    let root = SourceId::new(
        Uri::parse("https://example.test/fragment.json").unwrap(),
        Default::default(),
    );
    let result = OwnedCompiler::new(Config::default()).compile(contract, &[root]);
    assert!(result.is_ok(), "{:?}", result.err());
}

#[test]
fn unsupported_inherited_dialect_points_to_its_actual_declaration() {
    let mut entry = api(
        "3.2.0",
        json!({"X":{"$ref":"other.json#/components/schemas/X"}}),
    );
    entry["jsonSchemaDialect"] = json!("https://spec.openapis.org/oas/3.1/dialect/base");
    let mut other = api("3.1.2", json!({"X":{"$ref":"fragment.json"}}));
    other["jsonSchemaDialect"] = json!("https://example.test/custom-dialect");
    let contract = load(&[
        ("api.json", entry),
        ("other.json", other),
        ("fragment.json", json!({"type":"integer"})),
    ]);
    let root = SourceId::new(
        Uri::parse("https://example.test/fragment.json").unwrap(),
        Default::default(),
    );
    let errors = OwnedCompiler::new(Config::default())
        .compile(contract, &[root])
        .err()
        .unwrap();
    assert!(
        errors
            .iter()
            .any(
                |finding| finding.source.document().as_str() == "https://example.test/other.json"
                    && finding.source.pointer() == "/jsonSchemaDialect"
            ),
        "{errors:?}"
    );
}

#[test]
fn conflicting_defaults_propagate_through_reused_http_fragments() {
    for json_short in [false, true] {
        let operation = |reference: &str| json!({"post":{"requestBody":{"$ref":reference},"responses":{"204":{"description":"ok"}}}});
        let mut json_doc = api("3.1.2", json!({}));
        json_doc["jsonSchemaDialect"] = json!("https://json-schema.org/draft/2020-12/schema");
        json_doc["paths"] =
            json!({"/x":operation(if json_short {"body.json"} else {"json-body.json"})});
        let mut oas_doc = api("3.1.2", json!({}));
        oas_doc["paths"] =
            json!({"/x":operation(if json_short {"oas-body.json"} else {"body.json"})});
        let mut entry = api("3.1.2", json!({}));
        entry["paths"] =
            json!({"/json":{"$ref":"json.json#/paths/~1x"},"/oas":{"$ref":"oas.json#/paths/~1x"}});
        let contract = load(&[
            ("api.json", entry),
            ("json.json", json_doc),
            ("oas.json", oas_doc),
            ("json-body.json", json!({"$ref":"body.json"})),
            ("oas-body.json", json!({"$ref":"body.json"})),
            (
                "body.json",
                json!({"content":{"application/json":{"schema":{"type":"object","discriminator":false}}}}),
            ),
        ]);
        let root = SourceId::new(
            Uri::parse("https://example.test/body.json").unwrap(),
            Default::default(),
        )
        .child("content")
        .child("application/json")
        .child("schema");
        assert!(
            contract
                .diagnostics()
                .iter()
                .any(|finding| finding.code == "AMBIGUOUS_OPENAPI_CONTEXT"),
            "{:?}",
            contract.diagnostics()
        );
        assert!(contract.schema_contexts(&root).len() >= 2);
        assert!(
            OwnedCompiler::new(Config::default())
                .compile(contract, &[root])
                .is_err()
        );
    }
}

#[test]
fn splitting_a_request_body_does_not_invent_a_schema_resource_ancestor() {
    for split in [false, true] {
        let body = json!({"content":{"application/json":{"schema":{
            "$schema":"https://json-schema.org/draft/2020-12/schema","type":"integer"}}}});
        let mut entry = api("3.1.2", json!({}));
        entry["paths"] = json!({"/x":{"post":{"requestBody":if split {json!({"$ref":"body.json"})}else{body.clone()},
            "responses":{"204":{"description":"ok"}}}}});
        let contract = load(&[("api.json", entry), ("body.json", body)]);
        let root = contract
            .operations()
            .next()
            .unwrap()
            .request_body()
            .unwrap()
            .content()[0]
            .schema()
            .unwrap()
            .id()
            .clone();
        let result = OwnedCompiler::new(Config::default()).compile(contract, &[root]);
        assert!(result.is_ok(), "split={split}: {:?}", result.err());
    }
}
