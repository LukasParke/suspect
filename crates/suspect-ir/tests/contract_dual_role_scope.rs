//! A shared source can have HTTP and schema roles without changing its URI
//! scope. Schema-root metadata must not turn matching bases into ambiguity.

use std::sync::Arc;

use serde_json::{Value, json};
use suspect_ir::contract::{Contract, ContractReader, ResponseStatus, SchemaDialect, SourceId};
use suspect_low::Pointer;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

const ENTRY: &str = "https://example.test/spec/api.json";

fn id(pointer: &str) -> SourceId {
    SourceId::new(Uri::parse(ENTRY).unwrap(), Pointer::parse(pointer).unwrap())
}

fn load(value: &Value, reader: ContractReader) -> Contract {
    load_documents(
        &[(ENTRY, serde_json::to_vec(value).unwrap())],
        ENTRY,
        reader,
    )
}

fn load_documents(documents: &[(&str, Vec<u8>)], entry: &str, reader: ContractReader) -> Contract {
    let provider = Arc::new(
        DocumentProvider::new(documents.iter().map(|(uri, bytes)| {
            let uri = Uri::parse(uri).unwrap();
            ProvidedDocument::new(uri.clone(), uri, bytes.clone()).unwrap()
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
    Contract::from_workspace_with_reader(&workspace, &Uri::parse(entry).unwrap(), reader).unwrap()
}

fn fixture() -> Value {
    json!({"openapi":"3.0.4","info":{"title":"Dual-role reference","version":"1"},
        "servers":[{"url":"https://example.test"}],"security":[],
        "components":{"schemas":{"Base":{"description":"ok"},
            "Shared":{"$ref":"#/components/schemas/Base","$schema":false}}},
        "paths":{"/value":{"get":{"operationId":"selected","responses":{
            "204":{"$ref":"#/components/schemas/Shared"}}}}}})
}

fn assert_ignored_dual_role(reader: ContractReader) {
    let contract = load(&fixture(), reader);
    let shared = id("/components/schemas/Shared");
    let base = id("/components/schemas/Base");
    assert!(contract.schema(&shared).unwrap().ignores_ref_siblings());
    assert_eq!(
        contract.schema(&shared).unwrap().dialect(),
        &SchemaDialect::OpenApi30
    );
    assert_eq!(contract.schema_contexts(&shared).len(), 1);
    assert!(
        !contract
            .diagnostics()
            .iter()
            .any(|d| d.code == "invalid-reference" || d.code == "AMBIGUOUS_OPENAPI_CONTEXT"),
        "matching 3.0 reference-only roles must have a usable URI scope: {:?}",
        contract.diagnostics()
    );
    assert_eq!(contract.reference_target(&shared), Some(&base));
    let closure = contract.effective_schema_closure(std::slice::from_ref(&shared));
    assert_eq!(closure, vec![base.clone(), shared.clone()]);
    let ignored = contract
        .diagnostics()
        .iter()
        .find(|d| d.source == shared && d.code == "unsupported-schema-keyword")
        .unwrap();
    assert!(!contract.schema_diagnostic_applies(&closure, ignored));
    for source in [&shared, &base] {
        let scope = contract
            .resource_scope(source)
            .expect("schema and HTTP roles agree on the URI scope");
        assert_eq!(scope.resource(), &id(""));
        assert_eq!(scope.base_uri(), ENTRY);
        assert!(scope.base_source().is_none());
        assert_eq!(scope.address(), format!("{ENTRY}#{}", source.pointer()));
        assert_eq!(scope.schema_root(), Some(source));
    }
    let response = contract.operations().next().unwrap().responses().remove(0);
    assert_eq!(response.status(), Some(ResponseStatus::Exact(204)));
    assert_eq!(response.resolved_source(), Some(base));
    assert_eq!(response.description(), Some("ok"));
}

#[test]
fn ignored_schema_siblings_keep_dual_role_reference_scope_lossless() {
    assert_ignored_dual_role(ContractReader::Lossless);
}

#[test]
fn ignored_schema_siblings_keep_dual_role_reference_scope_fast() {
    assert_ignored_dual_role(ContractReader::Fast);
}

#[test]
fn removing_the_ignored_sibling_does_not_change_dual_role_uri_identity() {
    let mut value = fixture();
    value["components"]["schemas"]["Shared"]
        .as_object_mut()
        .unwrap()
        .remove("$schema");
    for reader in [ContractReader::Lossless, ContractReader::Fast] {
        let contract = load(&value, reader);
        assert!(!contract.has_errors(), "{:?}", contract.diagnostics());
        assert_eq!(
            contract.reference_target(&id("/components/schemas/Shared")),
            Some(&id("/components/schemas/Base"))
        );
        assert!(
            contract
                .resource_scope(&id("/components/schemas/Shared"))
                .is_some()
        );
    }
}

#[test]
fn schema_only_and_http_only_reference_roles_keep_their_own_metadata() {
    for reader in [ContractReader::Lossless, ContractReader::Fast] {
        let mut schema_only = fixture();
        schema_only["paths"] = json!({});
        let contract = load(&schema_only, reader);
        let shared = id("/components/schemas/Shared");
        assert_eq!(
            contract.resource_scope(&shared).unwrap().schema_root(),
            Some(&shared)
        );
        assert_eq!(
            contract.reference_target(&shared),
            Some(&id("/components/schemas/Base"))
        );
        assert!(
            !contract
                .diagnostics()
                .iter()
                .any(|d| d.code == "invalid-reference")
        );

        let mut http_only = fixture();
        http_only["components"] = json!({"responses":{"Base":{"description":"ok"},
            "Shared":{"$ref":"#/components/responses/Base","$schema":false}}});
        http_only["paths"]["/value"]["get"]["responses"]["204"] =
            json!({"$ref":"#/components/responses/Shared"});
        let contract = load(&http_only, reader);
        let shared = id("/components/responses/Shared");
        assert!(!contract.has_errors(), "{:?}", contract.diagnostics());
        assert!(contract.schema_contexts(&shared).is_empty());
        assert!(
            contract
                .resource_scope(&shared)
                .unwrap()
                .schema_root()
                .is_none()
        );
        assert_eq!(
            contract.reference_target(&shared),
            Some(&id("/components/responses/Base"))
        );
    }
}

#[test]
fn declaring_the_http_role_first_still_preserves_the_known_schema_root() {
    let mut value = fixture();
    value["components"] = json!({
        "responses":{"Base":{"description":"ok"},"Shared":{"$ref":"#/components/responses/Base","$schema":false}},
        "schemas":{"Use":{"$ref":"#/components/responses/Shared"}}
    });
    value["paths"]["/value"]["get"]["responses"]["204"] =
        json!({"$ref":"#/components/responses/Shared"});
    for reader in [ContractReader::Lossless, ContractReader::Fast] {
        let contract = load(&value, reader);
        let shared = id("/components/responses/Shared");
        let base = id("/components/responses/Base");
        assert!(
            !contract
                .diagnostics()
                .iter()
                .any(|d| d.code == "invalid-reference" || d.code == "AMBIGUOUS_OPENAPI_CONTEXT"),
            "{:?}",
            contract.diagnostics()
        );
        assert_eq!(contract.reference_target(&shared), Some(&base));
        assert_eq!(
            contract.resource_scope(&shared).unwrap().schema_root(),
            Some(&shared)
        );
        assert_eq!(
            contract.resource_scope(&base).unwrap().schema_root(),
            Some(&base)
        );
        assert_eq!(
            contract.operations().next().unwrap().responses()[0].resolved_source(),
            Some(base)
        );
    }
}

#[test]
fn equivalent_json_yaml_and_both_readers_have_the_same_dual_role_scope() {
    let uri = "https://example.test/spec/api.yaml";
    let yaml = br##"openapi: 3.0.4
info: {title: Dual-role reference, version: '1'}
servers:
  - url: https://example.test
security: []
components:
  schemas:
    Base:
      description: ok
    Shared:
      $ref: '#/components/schemas/Base'
      $schema: false
paths:
  /value:
    get:
      operationId: selected
      responses:
        '204':
          $ref: '#/components/schemas/Shared'
"##;
    let mut scopes = Vec::new();
    for bytes in [serde_json::to_vec(&fixture()).unwrap(), yaml.to_vec()] {
        for reader in [ContractReader::Lossless, ContractReader::Fast] {
            let contract = load_documents(&[(uri, bytes.clone())], uri, reader);
            let shared = SourceId::new(
                Uri::parse(uri).unwrap(),
                Pointer::parse("/components/schemas/Shared").unwrap(),
            );
            let base = SourceId::new(
                Uri::parse(uri).unwrap(),
                Pointer::parse("/components/schemas/Base").unwrap(),
            );
            assert!(
                !contract
                    .diagnostics()
                    .iter()
                    .any(|d| d.code == "invalid-reference"),
                "{:?}",
                contract.diagnostics()
            );
            assert_eq!(contract.reference_target(&shared), Some(&base));
            assert_eq!(contract.document(contract.entry()), Some(&fixture()));
            let scope = contract.resource_scope(&shared).unwrap();
            assert_eq!(scope.base_uri(), uri);
            assert_eq!(
                scope.address(),
                "https://example.test/spec/api.yaml#/components/schemas/Shared"
            );
            assert_eq!(scope.schema_root(), Some(&shared));
            scopes.push(scope.clone());
        }
    }
    assert!(scopes.windows(2).all(|pair| pair[0] == pair[1]));
}

#[test]
fn malformed_and_missing_references_remain_real_applicable_findings_in_both_roles() {
    for (reference, reason) in [
        (Value::Null, "must be a URI-reference string"),
        (json!(17), "must be a URI-reference string"),
        (json!("#/components/schemas/Bad%"), "invalid RFC 3986"),
        (json!("#/components/schemas/Missing"), "does not exist"),
    ] {
        let mut value = fixture();
        value["components"]["schemas"]["Shared"]["$ref"] = reference;
        for reader in [ContractReader::Lossless, ContractReader::Fast] {
            let contract = load(&value, reader);
            let shared = id("/components/schemas/Shared");
            let diagnostic = contract
                .diagnostics()
                .iter()
                .find(|d| d.code == "invalid-reference" && d.source == shared)
                .unwrap();
            assert!(diagnostic.message.contains(reason), "{diagnostic:?}");
            assert_eq!(
                diagnostic.at,
                contract.source_span(&shared.child("$ref")).unwrap()
            );
            assert!(contract.schema_diagnostic_applies(std::slice::from_ref(&shared), diagnostic));
            assert!(contract.reference_target(&shared).is_none());
            assert!(
                contract.resource_scope(&shared).is_some(),
                "malformed reference values do not make an otherwise matching URI base ambiguous"
            );
        }
    }
}

#[test]
fn an_invalid_modern_schema_scope_is_not_replaced_by_a_valid_http_scope() {
    let mut value = fixture();
    value["openapi"] = json!("3.1.2");
    for reader in [ContractReader::Lossless, ContractReader::Fast] {
        let contract = load(&value, reader);
        let shared = id("/components/schemas/Shared");
        assert!(!contract.schema(&shared).unwrap().ignores_ref_siblings());
        let diagnostic = contract
            .diagnostics()
            .iter()
            .find(|d| d.source == shared && d.code == "unsupported-schema-dialect")
            .unwrap();
        assert!(contract.schema_diagnostic_applies(std::slice::from_ref(&shared), diagnostic));
        assert!(contract.resource_scope(&shared).is_none());
        assert!(contract.has_errors());
    }
}

#[test]
fn actual_resource_boundaries_and_base_uris_still_make_dual_role_scopes_ambiguous() {
    for identifier in ["https://logical.test/schema", ENTRY] {
        let mut value = fixture();
        value["openapi"] = json!("3.2.0");
        value["components"]["schemas"]["Shared"] =
            json!({"$id":identifier,"$ref":format!("{ENTRY}#/components/schemas/Base")});
        for reader in [ContractReader::Lossless, ContractReader::Fast] {
            let contract = load(&value, reader);
            let shared = id("/components/schemas/Shared");
            if identifier == ENTRY {
                // The URI collision blocks the HTTP mount itself: the
                // document name already identifies two physical resources.
                assert!(
                    contract
                        .diagnostics()
                        .iter()
                        .any(|d| d.code == "invalid-resource-identity"),
                    "equal base URI text cannot hide different resource identities"
                );
                assert_eq!(
                    contract
                        .resources()
                        .filter(|resource| resource.canonical_uri() == ENTRY)
                        .count(),
                    2
                );
                assert!(
                    contract
                        .reference_target(&id("/paths/~1value/get/responses/204"))
                        .is_none()
                );
            } else {
                assert!(
                    contract.resource_scope(&shared).is_none(),
                    "an active schema $id disagrees with the HTTP Reference Object's document scope"
                );
                let diagnostic = contract
                    .diagnostics()
                    .iter()
                    .find(|d| d.source == shared && d.code == "invalid-reference")
                    .unwrap();
                assert!(contract.schema_diagnostic_applies(&[shared], diagnostic));
            }
        }
    }
}

#[test]
fn true_cross_document_dialect_ambiguity_is_retained_with_both_origins() {
    let make = |version, reference| json!({"openapi":version,"info":{"title":"Context","version":"1"},"paths":{},"components":{"schemas":{"Use":{"$ref":reference}}}});
    let documents = [
        (ENTRY, json!({"openapi":"3.2.0","info":{"title":"Context","version":"1"},"paths":{},"components":{"schemas":{
            "Old":{"$ref":"old.json#/components/schemas/Use"},"Modern":{"$ref":"modern.json#/components/schemas/Use"}
        }}})),
        ("https://example.test/spec/old.json", make("3.0.4", "shared.json")),
        ("https://example.test/spec/modern.json", make("3.2.0", "shared.json")),
        ("https://example.test/spec/shared.json", json!({"$ref":"leaf.json","minimum":5})),
        ("https://example.test/spec/leaf.json", json!({"type":"integer"})),
    ].into_iter().map(|(uri, value)| (uri, serde_json::to_vec(&value).unwrap())).collect::<Vec<_>>();
    for reader in [ContractReader::Lossless, ContractReader::Fast] {
        let contract = load_documents(&documents, ENTRY, reader);
        let shared = SourceId::new(
            Uri::parse("https://example.test/spec/shared.json").unwrap(),
            Pointer::root(),
        );
        let contexts = contract.schema_contexts(&shared);
        for version in ["3.0.4", "3.2.0"] {
            assert!(
                contexts
                    .iter()
                    .any(|context| context.openapi_version() == version)
            );
        }
        let diagnostic = contract
            .diagnostics()
            .iter()
            .find(|d| d.source == shared && d.code == "AMBIGUOUS_OPENAPI_CONTEXT")
            .unwrap();
        assert!(contract.schema_diagnostic_applies(&[shared], diagnostic));
    }
}
