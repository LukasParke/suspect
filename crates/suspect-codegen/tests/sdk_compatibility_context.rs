//! Compatibility uses the same effective schema and selected-operation contexts
//! as generation; ignored siblings and unrelated operations are not wire changes.
use serde_json::{Value, json};
use std::sync::Arc;
use suspect_codegen::compatibility::{self, Direction, Impact};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

fn load(value: Value) -> Arc<Contract> {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("api.json");
    std::fs::write(&path, value.to_string()).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(root.path()).build().unwrap());
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap())
}

fn supplied(entry: &str, documents: &[(&str, &str, Value)]) -> Arc<Contract> {
    let provider = Arc::new(
        DocumentProvider::new(documents.iter().map(|(requested, effective, value)| {
            ProvidedDocument::new(
                Uri::parse(requested).unwrap(),
                Uri::parse(effective).unwrap(),
                serde_json::to_vec_pretty(value).unwrap(),
            )
            .unwrap()
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
    Arc::new(Contract::from_workspace(&workspace, &Uri::parse(entry).unwrap()).unwrap())
}

fn at(uri: &str, value: Value) -> Arc<Contract> {
    supplied(uri, &[(uri, uri, value)])
}

fn document(version: &str, schema: Value) -> Value {
    json!({"openapi":version,"info":{"title":"Selected contexts","version":"1"},
    "servers":[{"url":"https://example.test"}],"security":[],
    "components":{"schemas":{"Scalar":{"type":"integer"}}},
    "paths":{"/items":{"post":{"operationId":"selected",
        "requestBody":{"required":true,"content":{"application/json":{"schema":schema}}},
        "responses":{"200":{"description":"Value","content":{"application/json":{"schema":schema}}}}
    }}}})
}

#[test]
fn ignored_oas30_reference_siblings_do_not_create_unknowns_or_schema_deltas() {
    let old = document(
        "3.0.4",
        json!({"$ref":"#/components/schemas/Scalar","minimum":0}),
    );
    let new = document(
        "3.0.4",
        json!({"$ref":"#/components/schemas/Scalar","minimum":"inert invalid sibling",
        "$dynamicRef":"missing.json#unused","$schema":false,
        "properties":{"ignored":{"$ref":"missing.json#also-unused"}}}),
    );
    let report = compatibility::compare(load(old), load(new), &["selected".into()], &[]).unwrap();
    assert!(report.is_proven_compatible(), "{report:#?}");
    assert!(
        report.wire.is_empty(),
        "ignored assertions cannot become migration work"
    );
}

#[test]
fn modern_reference_siblings_keep_real_narrowing_and_malformed_source_findings() {
    let old = document(
        "3.1.2",
        json!({"$ref":"#/components/schemas/Scalar","minimum":0}),
    );
    let new = document(
        "3.1.2",
        json!({"$ref":"#/components/schemas/Scalar","minimum":5}),
    );
    let report =
        compatibility::compare(load(old.clone()), load(new), &["selected".into()], &[]).unwrap();
    assert!(!report.is_proven_compatible());
    assert!(
        report
            .wire
            .iter()
            .any(|finding| finding.direction == Some(Direction::Request)
                && matches!(
                    finding.impact,
                    Impact::PotentiallyBreaking | Impact::Unknown
                ))
    );
    let malformed = document(
        "3.1.2",
        json!({"$ref":"#/components/schemas/Scalar","minimum":"invalid"}),
    );
    let report =
        compatibility::compare(load(old), load(malformed), &["selected".into()], &[]).unwrap();
    assert!(
        report
            .wire
            .iter()
            .any(|finding| finding.impact == Impact::Unknown)
    );
}

#[test]
fn a_malformed_unselected_operation_on_the_same_path_does_not_taint_the_selected_wire_contract() {
    let mut value = document("3.1.2", json!({"type":"integer"}));
    value["paths"]["/items"]["put"] = json!({"operationId":"unselected","responses":{"200":{"description":"Other",
        "content":{"application/json":{"schema":42}}}}});
    let contract = load(value.clone());
    assert!(
        contract
            .diagnostics()
            .iter()
            .any(|finding| finding.source.pointer().starts_with("/paths/~1items/put"))
    );
    let report =
        compatibility::compare(contract, load(value.clone()), &["selected".into()], &[]).unwrap();
    assert!(report.is_proven_compatible(), "{report:#?}");
    let all = compatibility::compare(load(value.clone()), load(value), &[], &[]).unwrap();
    assert!(
        !all.is_proven_compatible(),
        "attempting all operations must retain the malformed operation's finding"
    );
}

#[test]
fn streamed_item_schemas_follow_references_with_independent_request_and_response_variance() {
    let uri = "https://retrieval.test/api.json";
    let mut before = document("3.2.0", json!({"type":"integer"}));
    let stream = json!({"application/jsonl":{"itemSchema":{"$ref":"#/components/schemas/Scalar"}}});
    before["paths"]["/items"]["post"]["requestBody"]["content"] = stream.clone();
    before["paths"]["/items"]["post"]["responses"]["200"]["content"] = stream;
    before["components"]["schemas"]["Scalar"]["minimum"] = json!(0);
    let mut after = before.clone();
    after["components"]["schemas"]["Scalar"]["minimum"] = json!(5);
    let report =
        compatibility::compare(at(uri, before), at(uri, after), &["selected".into()], &[]).unwrap();
    for (direction, impact) in [
        (Direction::Request, Impact::PotentiallyBreaking),
        (Direction::Response, Impact::Compatible),
    ] {
        let finding = report
            .wire
            .iter()
            .find(|f| f.direction == Some(direction) && f.code == "wire-schema-changed")
            .unwrap_or_else(|| panic!("missing {direction:?} item-schema change: {report:#?}"));
        assert_eq!(finding.impact, impact, "{finding:#?}");
        assert!(finding.subject.contains("item"));
        assert!(
            finding
                .schema_deltas
                .iter()
                .any(|delta| delta.keyword == "minimum"
                    && delta.source_after.as_ref().unwrap().pointer
                        == "/components/schemas/Scalar/minimum")
        );
    }
}

#[test]
fn malformed_item_schemas_cannot_pass_an_unchanged_wire_only_comparison() {
    let mut value = document("3.2.0", json!({"type":"integer"}));
    value["paths"]["/items"]["post"]["responses"]["200"]["content"] =
        json!({"application/jsonl":{"itemSchema":42}});
    let contract = load(value);
    let report =
        compatibility::compare(contract.clone(), contract, &["selected".into()], &[]).unwrap();
    assert!(
        report
            .wire
            .iter()
            .any(|finding| finding.impact == Impact::Unknown
                && finding
                    .source_before
                    .as_ref()
                    .or(finding.source_after.as_ref())
                    .is_some_and(|source| source.pointer.ends_with("/itemSchema"))),
        "{report:#?}"
    );
}

#[test]
fn relative_server_identity_uses_effective_http_retrieval_and_the_implicit_default() {
    for declaration in [Some(json!([{"url":"/v1"}])), Some(json!([])), None] {
        let mut value = document("3.2.0", json!({"type":"integer"}));
        value["$self"] = json!("https://logical.test/stable-api");
        if let Some(declaration) = declaration {
            value["servers"] = declaration;
        } else {
            value.as_object_mut().unwrap().remove("servers");
        }
        let old = at("https://old.test/spec/api.json", value.clone());
        let new = at("https://new.test/spec/api.json", value);
        let report = compatibility::compare(old, new, &["selected".into()], &[]).unwrap();
        assert!(
            report.wire.iter().any(
                |f| f.code == "wire-servers-changed" && f.impact == Impact::PotentiallyBreaking
            ),
            "{report:#?}"
        );
    }
}

#[test]
fn logical_ids_and_requested_aliases_do_not_relocate_the_physical_server() {
    let requested = "https://requested.test/api.json";
    let effective = "https://retrieval.test/spec/api.json";
    let mut value = document("3.2.0", json!({"type":"integer"}));
    value["servers"] = json!([{"url":"/v1"}]);
    value["$self"] = json!("https://logical.test/old");
    let old = supplied(requested, &[(requested, effective, value.clone())]);
    value["$self"] = json!("https://logical.test/new");
    let new = at(effective, value);
    let report = compatibility::compare(old, new, &["selected".into()], &[]).unwrap();
    assert!(report.is_proven_compatible(), "{report:#?}");

    let value = document("3.2.0", json!({"type":"integer"}));
    let report = compatibility::compare(
        at("https://old.test/api.json", value.clone()),
        at("https://new.test/api.json", value),
        &["selected".into()],
        &[],
    )
    .unwrap();
    assert!(
        report.is_proven_compatible(),
        "an absolute server ignores retrieval relocation: {report:#?}"
    );
}

#[test]
fn canonical_resource_rebinding_follows_static_targets_with_original_physical_sources() {
    let entry = "https://retrieval.test/api.json";
    let mut value = document("3.2.0", json!({"$ref":"#/components/schemas/Use"}));
    value["components"]["schemas"]["Use"] =
        json!({"$id":"https://schemas.test/a/root","$ref":"leaf"});
    let first = json!({"$schema":"https://json-schema.org/draft/2020-12/schema",
        "$id":"https://schemas.test/a/leaf","type":"integer","minimum":0});
    let second = json!({"$schema":"https://json-schema.org/draft/2020-12/schema",
        "$id":"https://schemas.test/b/leaf","type":"integer","minimum":5});
    let load = |value: Value| {
        supplied(
            entry,
            &[
                (entry, entry, value),
                (
                    "https://retrieval.test/first.json",
                    "https://retrieval.test/first.json",
                    first.clone(),
                ),
                (
                    "https://retrieval.test/second.json",
                    "https://retrieval.test/second.json",
                    second.clone(),
                ),
            ],
        )
    };
    let old = load(value.clone());
    value["components"]["schemas"]["Use"]["$id"] = json!("https://schemas.test/b/root");
    let report = compatibility::compare(old, load(value), &["selected".into()], &[]).unwrap();
    assert!(
        report
            .wire
            .iter()
            .any(|finding| finding.direction == Some(Direction::Request)
                && finding.impact == Impact::PotentiallyBreaking),
        "{report:#?}"
    );
    let delta = report
        .wire
        .iter()
        .flat_map(|finding| &finding.schema_deltas)
        .find(|delta| delta.keyword == "minimum")
        .unwrap();
    assert_eq!(
        delta.source_before.as_ref().unwrap().document,
        "https://retrieval.test/first.json"
    );
    assert_eq!(
        delta.source_after.as_ref().unwrap().document,
        "https://retrieval.test/second.json"
    );
}

#[test]
fn positional_encoding_metadata_needs_a_proof_even_when_reference_text_is_unchanged() {
    let uri = "https://retrieval.test/api.json";
    let mut before = document("3.2.0", json!({"type":"string"}));
    before["paths"]["/items"]["post"]["requestBody"]["content"] = json!({"multipart/mixed":{
        "schema":{"type":"array","prefixItems":[{"type":"string"}],"items":false},
        "prefixEncoding":[{"headers":{"X-Trace":{"$ref":"#/components/headers/Trace"}}}]
    }});
    before["components"]["headers"] = json!({"Trace":{"schema":{"type":"string","maxLength":8}}});
    let mut after = before.clone();
    after["components"]["headers"]["Trace"]["schema"]["maxLength"] = json!(4);
    let report =
        compatibility::compare(at(uri, before), at(uri, after), &["selected".into()], &[]).unwrap();
    assert!(
        !report.is_proven_compatible(),
        "changed referenced part metadata needs a protocol-specific proof: {report:#?}"
    );
    assert!(
        report
            .wire
            .iter()
            .any(|finding| finding.direction == Some(Direction::Request)
                && finding.impact == Impact::Unknown),
        "{report:#?}"
    );
}

#[test]
fn unrecognized_item_schema_metadata_is_not_erased_from_older_openapi_versions() {
    let uri = "https://retrieval.test/api.json";
    let before = document("3.1.2", json!({"type":"integer"}));
    let mut after = before.clone();
    after["paths"]["/items"]["post"]["responses"]["200"]["content"]["application/json"]["itemSchema"] =
        json!({"type":"string"});
    let report =
        compatibility::compare(at(uri, before), at(uri, after), &["selected".into()], &[]).unwrap();
    assert!(
        report
            .wire
            .iter()
            .any(|finding| finding.impact == Impact::Unknown),
        "{report:#?}"
    );
}

#[test]
fn review_malformed_media_reference_chains_remain_unknown_while_valid_relocation_is_equal() {
    let uri = "https://retrieval.test/api.json";
    let mut value = document("3.2.0", json!({"type":"integer"}));
    value["paths"]["/items"]["post"]["responses"]["200"]["content"] =
        json!({"application/json":{"$ref":"#/components/mediaTypes/Alias"}});
    value["components"]["mediaTypes"] =
        json!({"Alias":{"$ref":"#/components/mediaTypes/Terminal"},"Terminal":42});
    let invalid = at(uri, value.clone());
    assert!(
        invalid
            .diagnostics()
            .iter()
            .any(|d| d.source.pointer() == "/components/mediaTypes/Terminal")
    );
    let report =
        compatibility::compare(invalid.clone(), invalid, &["selected".into()], &[]).unwrap();
    assert!(
        report
            .wire
            .iter()
            .any(|finding| finding.impact == Impact::Unknown
                && finding
                    .source_before
                    .as_ref()
                    .is_some_and(|s| s.pointer == "/components/mediaTypes/Terminal")),
        "{report:#?}"
    );

    value["components"]["mediaTypes"]["Terminal"] = json!({"schema":{"type":"string"}});
    let before = at(uri, value.clone());
    let unchanged =
        compatibility::compare(before.clone(), before.clone(), &["selected".into()], &[]).unwrap();
    assert!(unchanged.is_proven_compatible(), "{unchanged:#?}");
    value["components"]["mediaTypes"] = json!({"Alias":{"$ref":"#/components/mediaTypes/Moved"},
        "Moved":{"schema":{"type":"string"}}});
    let moved = compatibility::compare(before, at(uri, value), &["selected".into()], &[]).unwrap();
    assert!(moved.is_proven_compatible(), "{moved:#?}");
}

#[test]
fn review_unsupported_item_metadata_in_a_referenced_older_document_cannot_disappear() {
    let entry = "https://retrieval.test/api.json";
    let legacy = "https://retrieval.test/legacy.json";
    let mut value = document("3.2.0", json!({"type":"integer"}));
    value["paths"]["/items"]["post"]["responses"]["200"]["content"] = json!({"application/json":{"$ref":"legacy.json#/components/responses/R/content/application~1json"}});
    let mut target = json!({"openapi":"3.1.2","info":{"title":"Legacy media","version":"1"},"paths":{},
        "components":{"responses":{"R":{"description":"Result","content":{"application/json":{"schema":{"type":"integer"}}}}}}});
    let old = supplied(
        entry,
        &[
            (entry, entry, value.clone()),
            (legacy, legacy, target.clone()),
        ],
    );
    target["components"]["responses"]["R"]["content"]["application/json"]["itemSchema"] = json!(42);
    let new = supplied(entry, &[(entry, entry, value), (legacy, legacy, target)]);
    let report = compatibility::compare(old, new, &["selected".into()], &[]).unwrap();
    assert!(
        report
            .wire
            .iter()
            .any(|finding| finding.impact == Impact::Unknown
                && finding
                    .source_after
                    .as_ref()
                    .is_some_and(|s| s.document == legacy)),
        "{report:#?}"
    );
}

#[test]
fn review_an_empty_positional_encoding_declaration_still_requires_a_protocol_proof() {
    let mut value = document("3.2.0", json!({"type":"integer"}));
    value["paths"]["/items"]["post"]["requestBody"]["content"] = json!({"multipart/mixed":{
        "schema":{"type":"array","items":{"type":"string"}},"prefixEncoding":[]}});
    let contract = load(value.clone());
    let report =
        compatibility::compare(contract.clone(), contract, &["selected".into()], &[]).unwrap();
    assert!(
        report
            .wire
            .iter()
            .any(|finding| finding.code == "wire-media-metadata-unknown"
                && finding.direction == Some(Direction::Request)
                && finding.impact == Impact::Unknown),
        "{report:#?}"
    );

    value["paths"]["/items"]["post"]["requestBody"]["content"] = json!({"multipart/form-data":{
        "schema":{"type":"object","properties":{"name":{"type":"string"}},"additionalProperties":false}}});
    let contract = load(value);
    let ordinary =
        compatibility::compare(contract.clone(), contract, &["selected".into()], &[]).unwrap();
    assert!(ordinary.is_proven_compatible(), "{ordinary:#?}");
}

#[test]
fn review_schema_registration_does_not_hide_an_explicit_malformed_http_target() {
    let mut value = document("3.2.0", json!({"type":"integer"}));
    value["components"]["schemas"]["Bad"] = json!(42);
    value["paths"]["/items"]["post"]["responses"]["200"]["content"] =
        json!({"application/json":{"$ref":"#/components/schemas/Bad"}});
    let contract = load(value);
    let source = contract
        .schemas()
        .find(|schema| schema.id().pointer() == "/components/schemas/Bad")
        .unwrap()
        .id()
        .clone();
    assert!(contract.diagnostics().iter().any(|d| d.source == source));
    let report =
        compatibility::compare(contract.clone(), contract, &["selected".into()], &[]).unwrap();
    assert!(
        report
            .wire
            .iter()
            .any(|finding| finding.code == "wire-contract-diagnostic"
                && finding.impact == Impact::Unknown
                && finding
                    .source_before
                    .as_ref()
                    .is_some_and(|s| s.pointer == "/components/schemas/Bad")),
        "{report:#?}"
    );
}

#[test]
fn review_ignored_schema_siblings_stay_inert_when_the_same_reference_is_an_http_object() {
    let value = json!({"openapi":"3.0.4","info":{"title":"Dual-role reference","version":"1"},
        "servers":[{"url":"https://example.test"}],"security":[],
        "components":{"schemas":{"Base":{"description":"ok"},
            "Shared":{"$ref":"#/components/schemas/Base","$schema":false}}},
        "paths":{"/value":{"get":{"operationId":"selected","responses":{
            "204":{"$ref":"#/components/schemas/Shared"}}}}}});
    let contract = load(value);
    assert!(
        contract
            .diagnostics()
            .iter()
            .any(|d| d.code == "unsupported-schema-keyword"
                && d.source.pointer() == "/components/schemas/Shared")
    );
    let report =
        compatibility::compare(contract.clone(), contract, &["selected".into()], &[]).unwrap();
    assert!(report.is_proven_compatible(), "{report:#?}");
    assert!(report.wire.is_empty());
}
