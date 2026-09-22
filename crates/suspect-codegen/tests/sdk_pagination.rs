//! Golden pagination detection binds through the public, admitted protocol plan.
use std::sync::Arc;

use serde_json::json;
use suspect_codegen::{http_protocol, rust_http, sdk_defaults};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

fn contract_with_paths(paths: serde_json::Value) -> Arc<Contract> {
    let entry = Uri::parse("https://source.pagination.test/openapi.json").unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            serde_json::to_vec(&json!({
                "openapi":"3.1.2", "info":{"title":"Pagination","version":"1"},
                "servers":[{"url":"https://api.pagination.test/v1"}],
                "paths": paths,
            }))
            .unwrap(),
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
    Arc::new(Contract::from_workspace(&workspace, &entry).unwrap())
}

fn plan_outcome(
    contract: &Arc<Contract>,
    defaults: Option<&sdk_defaults::SdkDefaults>,
) -> Result<http_protocol::PaginationOutcome, Vec<suspect_codegen::credential_env::HttpDiagnostic>>
{
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let protocol = http_protocol::plan(contract, &selected, rust_http::native_capabilities_v3())
        .into_result()
        .unwrap();
    http_protocol::plan_pagination(contract, &protocol, defaults)
}

fn limit_offset_paths() -> serde_json::Value {
    json!({
        "/widgets": {"get": {
            "operationId": "listWidgets",
            "parameters": [
                {"name":"limit","in":"query","schema":{"type":"integer"}},
                {"name":"offset","in":"query","schema":{"type":"integer"}}
            ],
            "responses": {"200": {"description":"Page","content":{"application/json":{"schema":{
                "type":"object","properties":{
                    "data":{"type":"array","items":{"type":"string"}},
                    "total":{"type":"integer"}
                }}}}}}}
        }
    })
}

#[test]
fn zero_config_limit_offset_is_detected_with_collection_evidence() {
    let contract = contract_with_paths(limit_offset_paths());
    let outcome = plan_outcome(&contract, Some(&sdk_defaults::SdkDefaults::v1())).unwrap();
    assert_eq!(outcome.paginated.len(), 1);
    let page = &outcome.paginated[0];
    assert_eq!(page.pattern, sdk_defaults::PaginationPattern::LimitOffset);
    assert_eq!(
        page.request[&sdk_defaults::PaginationRequestRole::Limit],
        "limit"
    );
    assert_eq!(
        page.request[&sdk_defaults::PaginationRequestRole::Offset],
        "offset"
    );
    assert_eq!(
        page.response[&sdk_defaults::PaginationResponseRole::Items],
        "/data"
    );
    assert_eq!(
        page.response[&sdk_defaults::PaginationResponseRole::Total],
        "/total"
    );
    assert_eq!(page.initial_offset, Some(0));
    assert_eq!(page.advance, sdk_defaults::PaginationAdvance::ItemsReturned);
}

#[test]
fn a_limit_parameter_alone_is_never_sufficient() {
    let paths = json!({
        "/widgets": {"get": {
            "operationId": "listWidgets",
            "parameters": [{"name":"limit","in":"query","schema":{"type":"integer"}}],
            "responses": {"200": {"description":"Page","content":{"application/json":{"schema":{
                "type":"object","properties":{"data":{"type":"array","items":{"type":"string"}}}
            }}}}}}
        }}
    );
    let contract = contract_with_paths(paths);
    let outcome = plan_outcome(&contract, Some(&sdk_defaults::SdkDefaults::v1())).unwrap();
    assert!(outcome.paginated.is_empty());
    assert_eq!(outcome.single_page.len(), 1);
    assert!(outcome.single_page[0].reason.contains("continuation rule"));
}

#[test]
fn cursor_pattern_requires_both_request_token_and_response_field() {
    let paths = json!({
        "/events": {"get": {
            "operationId": "listEvents",
            "parameters": [
                {"name":"cursor","in":"query","schema":{"type":"string"}},
                {"name":"pageSize","in":"query","schema":{"type":"integer"}}
            ],
            "responses": {"200": {"description":"Page","content":{"application/json":{"schema":{
                "type":"object","properties":{
                    "items":{"type":"array","items":{"type":"string"}},
                    "next_page_token":{"type":"string"}
                }}}}}}}
        }}
    );
    let contract = contract_with_paths(paths);
    let outcome = plan_outcome(&contract, Some(&sdk_defaults::SdkDefaults::v1())).unwrap();
    assert_eq!(outcome.paginated.len(), 1);
    let page = &outcome.paginated[0];
    assert_eq!(page.pattern, sdk_defaults::PaginationPattern::Cursor);
    assert_eq!(
        page.request[&sdk_defaults::PaginationRequestRole::Cursor],
        "cursor"
    );
    assert_eq!(
        page.request[&sdk_defaults::PaginationRequestRole::Limit],
        "pageSize"
    );
    assert_eq!(
        page.response[&sdk_defaults::PaginationResponseRole::NextCursor],
        "/next_page_token"
    );
    assert_eq!(
        page.response[&sdk_defaults::PaginationResponseRole::Items],
        "/items"
    );
}

#[test]
fn user_aliases_extend_the_builtin_vocabulary() {
    let paths = json!({
        "/rows": {"get": {
            "operationId": "listRows",
            "parameters": [
                {"name":"batch_size","in":"query","schema":{"type":"integer"}},
                {"name":"from_row","in":"query","schema":{"type":"integer"}}
            ],
            "responses": {"200": {"description":"Page","content":{"application/json":{"schema":{
                "type":"object","properties":{"records":{"type":"array","items":{"type":"string"}}}
            }}}}}}
        }}
    );
    let contract = contract_with_paths(paths);
    let without_aliases = plan_outcome(&contract, Some(&sdk_defaults::SdkDefaults::v1())).unwrap();
    assert!(without_aliases.paginated.is_empty());
    let configured = serde_json::from_value::<sdk_defaults::SdkDefaults>(json!({
        "version":"v1",
        "pagination":{"mode":"auto","aliases":{"limit":["batch_size"],"offset":["from_row"]}}
    }))
    .unwrap();
    let outcome = plan_outcome(&contract, Some(&configured)).unwrap();
    assert_eq!(outcome.paginated.len(), 1);
    let page = &outcome.paginated[0];
    assert_eq!(
        page.request[&sdk_defaults::PaginationRequestRole::Limit],
        "batch_size"
    );
    assert_eq!(
        page.request[&sdk_defaults::PaginationRequestRole::Offset],
        "from_row"
    );
}

#[test]
fn shorthand_and_expanded_configuration_normalize_identically() {
    let shorthand: sdk_defaults::SdkDefaults = serde_json::from_value(json!({
        "version":"v1","pagination":"auto"
    }))
    .unwrap();
    assert_eq!(
        shorthand.pagination.mode,
        sdk_defaults::PaginationMode::Auto
    );
    let expanded: sdk_defaults::SdkDefaults = serde_json::from_value(json!({
        "version":"v1","pagination":{"mode":"auto"}
    }))
    .unwrap();
    assert_eq!(shorthand, expanded);
    let off: sdk_defaults::SdkDefaults =
        serde_json::from_value(json!({"version":"v1","pagination":"off"})).unwrap();
    assert_eq!(off.pagination.mode, sdk_defaults::PaginationMode::Off);
}

#[test]
fn explicit_disable_and_full_manual_mapping_are_honored() {
    let paths = json!({
        "/invoices": {"get": {
            "operationId": "listInvoices",
            "parameters": [
                {"name":"maxRows","in":"query","schema":{"type":"integer"}},
                {"name":"fromRow","in":"query","schema":{"type":"integer"}}
            ],
            "responses": {"200": {"description":"Page","content":{"application/json":{"schema":{
                "type":"object","properties":{
                    "entries":{"type":"array","items":{"type":"string"}},
                    "paging":{"type":"object","properties":{"count":{"type":"integer"}}}
                }
            }}}}}}
        },
        "/audit": {"get": {"operationId": "listAuditEvents", "responses": {"200": {"description":"Page"}}}}
    });
    let contract = contract_with_paths(paths);
    let configured: sdk_defaults::SdkDefaults = serde_json::from_value(json!({
        "version":"v1",
        "pagination":{
            "mode":"auto",
            "operations":{
                "listInvoices":{
                    "pattern":"limit-offset",
                    "request":{"limit":"maxRows","offset":"fromRow"},
                    "response":{"items":"/entries","total":"/paging/count"},
                    "initial_offset":0,
                    "advance":"items-returned"
                },
                "listAuditEvents": false
            }
        }
    }))
    .unwrap();
    let outcome = plan_outcome(&contract, Some(&configured)).unwrap();
    assert_eq!(outcome.paginated.len(), 1);
    let page = &outcome.paginated[0];
    assert_eq!(page.operation, "listInvoices");
    assert_eq!(
        page.request[&sdk_defaults::PaginationRequestRole::Limit],
        "maxRows"
    );
    assert_eq!(
        page.request[&sdk_defaults::PaginationRequestRole::Offset],
        "fromRow"
    );
    assert_eq!(
        page.response[&sdk_defaults::PaginationResponseRole::Items],
        "/entries"
    );
    assert_eq!(
        page.response[&sdk_defaults::PaginationResponseRole::Total],
        "/paging/count"
    );
    let disabled = outcome
        .single_page
        .iter()
        .find(|reason| reason.operation == "listAuditEvents")
        .expect("disabled operation recorded");
    assert!(disabled.reason.contains("explicitly disabled"));
}

#[test]
fn an_override_pointing_outside_the_schema_is_a_generation_error() {
    let contract = contract_with_paths(limit_offset_paths());
    let configured: sdk_defaults::SdkDefaults = serde_json::from_value(json!({
        "version":"v1",
        "pagination":{"operations":{"listWidgets":{
            "pattern":"limit-offset",
            "request":{"limit":"limit","offset":"offset"},
            "response":{"items":"/entries"}
        }}}
    }))
    .unwrap();
    let error = plan_outcome(&contract, Some(&configured)).unwrap_err();
    assert!(
        error.iter().any(|d| d.code == "sdk-pagination-override"),
        "{error:?}"
    );
}

#[test]
fn competing_candidates_and_wrong_types_stay_single_page() {
    // Two limit-role candidates: ambiguous, so no inference.
    let ambiguous = json!({
        "/widgets": {"get": {
            "operationId": "listWidgets",
            "parameters": [
                {"name":"limit","in":"query","schema":{"type":"integer"}},
                {"name":"page_size","in":"query","schema":{"type":"integer"}},
                {"name":"offset","in":"query","schema":{"type":"integer"}}
            ],
            "responses": {"200": {"description":"Page","content":{"application/json":{"schema":{
                "type":"object","properties":{"data":{"type":"array","items":{"type":"string"}}}
            }}}}}}
        }}
    );
    let contract = contract_with_paths(ambiguous);
    let outcome = plan_outcome(&contract, Some(&sdk_defaults::SdkDefaults::v1())).unwrap();
    assert!(
        outcome.paginated.is_empty(),
        "competing candidates must not guess"
    );

    // A string-typed limit is not a usable size parameter.
    let wrong_types = json!({
        "/widgets": {"get": {
            "operationId": "listWidgets",
            "parameters": [
                {"name":"limit","in":"query","schema":{"type":"string"}},
                {"name":"offset","in":"query","schema":{"type":"integer"}}
            ],
            "responses": {"200": {"description":"Page","content":{"application/json":{"schema":{
                "type":"object","properties":{"data":{"type":"array","items":{"type":"string"}}}
            }}}}}}
        }}
    );
    let contract = contract_with_paths(wrong_types);
    let outcome = plan_outcome(&contract, Some(&sdk_defaults::SdkDefaults::v1())).unwrap();
    assert!(outcome.paginated.is_empty());
    assert_eq!(outcome.single_page.len(), 1);
}

#[test]
fn env_prefix_binds_one_unambiguous_string_credential_automatically() {
    let entry = Uri::parse("https://source.prefix.test/openapi.json").unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            serde_json::to_vec(&json!({
                "openapi":"3.1.0", "info":{"title":"Prefix","version":"1"},
                "servers":[{"url":"https://api.prefix.test/v1"}],
                "security":[{"apiKey":[]}],
                "components":{"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}}},
                "paths":{"/widgets":{"get":{"operationId":"listWidgets","responses":{"200":{"description":"Ok"}}}}}
            }))
            .unwrap(),
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
    let contract = Arc::new(Contract::from_workspace(&workspace, &entry).unwrap());
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let protocol = http_protocol::plan(&contract, &selected, rust_http::native_capabilities_v3())
        .into_result()
        .unwrap();
    let defaults: sdk_defaults::SdkDefaults = serde_json::from_value(json!({
        "version":"v1","env_prefix":"OPENROUTER"
    }))
    .unwrap();
    let planned = suspect_codegen::credential_env::plan_with_defaults(
        &contract,
        &protocol,
        None,
        Some(&defaults),
    )
    .unwrap()
    .expect("one unambiguous bearer scheme binds automatically");
    assert_eq!(planned.bindings().len(), 1);
    let binding = &planned.bindings()[0];
    assert_eq!(binding.name(), "apiKey");
    assert_eq!(binding.variable(), "OPENROUTER_API_KEY");

    // An explicit mapping remains the precise override and wins entirely.
    let explicit = suspect_codegen::credential_env::CredentialEnv::v1(
        [("apiKey".to_owned(), "CUSTOM_VAR".to_owned())]
            .into_iter()
            .collect(),
    );
    let overridden = suspect_codegen::credential_env::plan_with_defaults(
        &contract,
        &protocol,
        Some(&explicit),
        Some(&defaults),
    )
    .unwrap()
    .unwrap();
    assert_eq!(overridden.bindings()[0].variable(), "CUSTOM_VAR");

    // Two used string-credential schemes are ambiguous and bind nothing.
    let entry2 = Uri::parse("https://source.ambiguous.test/openapi.json").unwrap();
    let provider2 = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry2.clone(),
            entry2.clone(),
            serde_json::to_vec(&json!({
                "openapi":"3.1.0", "info":{"title":"Ambiguous","version":"1"},
                "servers":[{"url":"https://api.ambiguous.test/v1"}],
                "paths":{
                    "/widgets":{"get":{"operationId":"listWidgets","security":[{"primary":[]}],"responses":{"200":{"description":"Ok"}}}},
                    "/logs":{"get":{"operationId":"listLogs","security":[{"secondary":[]}],"responses":{"200":{"description":"Ok"}}}}
                },
                "components":{"securitySchemes":{
                    "primary":{"type":"http","scheme":"bearer"},
                    "secondary":{"type":"apiKey","in":"header","name":"X-Key"}
                }}
            }))
            .unwrap(),
        )
        .unwrap()])
        .unwrap(),
    );
    let workspace2 = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider2.logical_uris())
            .document_provider(provider2)
            .build()
            .unwrap(),
    );
    let ambiguous_contract = Arc::new(Contract::from_workspace(&workspace2, &entry2).unwrap());
    let selected2 = ambiguous_contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let ambiguous_protocol = http_protocol::plan(
        &ambiguous_contract,
        &selected2,
        rust_http::native_capabilities_v3(),
    )
    .into_result()
    .unwrap();
    let resolved = suspect_codegen::credential_env::plan_with_defaults(
        &ambiguous_contract,
        &ambiguous_protocol,
        None,
        Some(&defaults),
    )
    .unwrap();
    assert!(
        resolved.is_none(),
        "ambiguous sources produce no automatic binding"
    );
}

/// One limit/offset operation and one cursor operation in one document.
fn mixed_pattern_paths() -> serde_json::Value {
    json!({
        "/widgets": {"get": {
            "operationId": "listWidgets",
            "parameters": [
                {"name":"limit","in":"query","schema":{"type":"integer"}},
                {"name":"offset","in":"query","schema":{"type":"integer"}}
            ],
            "responses": {"200": {"description":"Page","content":{"application/json":{"schema":{
                "type":"object","properties":{
                    "data":{"type":"array","items":{"type":"string"}},
                    "total":{"type":"integer"}
                }}}}}}}
        },
        "/events": {"get": {
            "operationId": "listEvents",
            "parameters": [
                {"name":"cursor","in":"query","schema":{"type":"string"}},
                {"name":"pageSize","in":"query","schema":{"type":"integer"}}
            ],
            "responses": {"200": {"description":"Page","content":{"application/json":{"schema":{
                "type":"object","properties":{
                    "items":{"type":"array","items":{"type":"string"}},
                    "next_page_token":{"type":"string"}
                }}}}}}}
        }
    })
}

#[test]
fn configured_patterns_restrict_automatic_detection() {
    let contract = contract_with_paths(mixed_pattern_paths());
    let unrestricted = plan_outcome(&contract, Some(&sdk_defaults::SdkDefaults::v1())).unwrap();
    assert_eq!(unrestricted.paginated.len(), 2);

    let configured: sdk_defaults::SdkDefaults = serde_json::from_value(json!({
        "version":"v1",
        "pagination":{"mode":"auto","patterns":["limit-offset"]}
    }))
    .unwrap();
    let outcome = plan_outcome(&contract, Some(&configured)).unwrap();
    assert_eq!(outcome.paginated.len(), 1, "only the admitted pattern runs");
    assert_eq!(outcome.paginated[0].operation, "listWidgets");
    assert_eq!(
        outcome.paginated[0].pattern,
        sdk_defaults::PaginationPattern::LimitOffset
    );
    let restricted = outcome
        .single_page
        .iter()
        .find(|reason| reason.operation == "listEvents")
        .expect("the excluded match is explained");
    assert!(
        restricted.reason.contains("cursor")
            && restricted
                .reason
                .contains("sdk_defaults.pagination.patterns"),
        "the explanation names the restriction: {}",
        restricted.reason
    );

    // An empty subset admits no automatic detection at all; every operation
    // that would have matched is explained instead.
    let none: sdk_defaults::SdkDefaults = serde_json::from_value(json!({
        "version":"v1",
        "pagination":{"mode":"auto","patterns":[]}
    }))
    .unwrap();
    let outcome = plan_outcome(&contract, Some(&none)).unwrap();
    assert!(outcome.paginated.is_empty());
    assert_eq!(outcome.single_page.len(), 2);
    assert!(
        outcome
            .single_page
            .iter()
            .all(|reason| reason.reason.contains("sdk_defaults.pagination.patterns"))
    );
}

#[test]
fn per_operation_mappings_are_unaffected_by_the_patterns_subset() {
    let contract = contract_with_paths(mixed_pattern_paths());
    let configured: sdk_defaults::SdkDefaults = serde_json::from_value(json!({
        "version":"v1",
        "pagination":{
            "mode":"auto",
            "patterns":["limit-offset"],
            "operations":{"listEvents":{
                "pattern":"cursor",
                "request":{"cursor":"cursor","limit":"pageSize"},
                "response":{"items":"/items","next-cursor":"/next_page_token"}
            }}
        }
    }))
    .unwrap();
    let outcome = plan_outcome(&contract, Some(&configured)).unwrap();
    assert_eq!(
        outcome.paginated.len(),
        2,
        "an explicit mapping is never restricted by patterns"
    );
    let mapped = outcome
        .paginated
        .iter()
        .find(|page| page.operation == "listEvents")
        .expect("the manual mapping still paginates");
    assert_eq!(mapped.pattern, sdk_defaults::PaginationPattern::Cursor);
}

#[test]
fn page_size_becomes_the_first_page_limit_for_limit_bearing_patterns() {
    let contract = contract_with_paths(mixed_pattern_paths());
    let configured: sdk_defaults::SdkDefaults = serde_json::from_value(json!({
        "version":"v1",
        "pagination":{"mode":"auto","page_size":5}
    }))
    .unwrap();
    let outcome = plan_outcome(&contract, Some(&configured)).unwrap();
    assert_eq!(outcome.paginated.len(), 2);
    for page in &outcome.paginated {
        assert_eq!(page.initial_limit, Some(5), "{}", page.operation);
        // The fallback is a first-page convention only, and serializes as
        // such; the key is absent whenever no fallback is configured.
        let serialized = serde_json::to_value(page).unwrap();
        assert_eq!(serialized["initial_limit"], 5);
    }
    let plain = plan_outcome(&contract, Some(&sdk_defaults::SdkDefaults::v1())).unwrap();
    for page in &plain.paginated {
        assert_eq!(page.initial_limit, None);
        let serialized = serde_json::to_value(page).unwrap();
        assert!(
            serialized.get("initial_limit").is_none(),
            "no-policy serialization never carries the fallback key"
        );
    }
}

#[test]
fn a_declared_source_default_defers_the_page_size_fallback() {
    let paths = json!({
        "/widgets": {"get": {
            "operationId": "listWidgets",
            "parameters": [
                {"name":"limit","in":"query","schema":{"type":"integer","default":10}},
                {"name":"offset","in":"query","schema":{"type":"integer"}}
            ],
            "responses": {"200": {"description":"Page","content":{"application/json":{"schema":{
                "type":"object","properties":{
                    "data":{"type":"array","items":{"type":"string"}},
                    "total":{"type":"integer"}
                }}}}}}}
        }
    });
    let contract = contract_with_paths(paths);
    let configured: sdk_defaults::SdkDefaults = serde_json::from_value(json!({
        "version":"v1",
        "pagination":{"mode":"auto","page_size":5}
    }))
    .unwrap();
    let outcome = plan_outcome(&contract, Some(&configured)).unwrap();
    assert_eq!(outcome.paginated.len(), 1);
    assert_eq!(
        outcome.paginated[0].initial_limit, None,
        "a source default means the helper records no fallback"
    );
}

#[test]
fn page_size_violating_a_declared_maximum_is_a_generation_error() {
    let paths = json!({
        "/widgets": {"get": {
            "operationId": "listWidgets",
            "parameters": [
                {"name":"limit","in":"query","schema":{"type":"integer","maximum":10}},
                {"name":"offset","in":"query","schema":{"type":"integer"}}
            ],
            "responses": {"200": {"description":"Page","content":{"application/json":{"schema":{
                "type":"object","properties":{
                    "data":{"type":"array","items":{"type":"string"}},
                    "total":{"type":"integer"}
                }}}}}}}
        }
    });
    let contract = contract_with_paths(paths);
    let within: sdk_defaults::SdkDefaults = serde_json::from_value(json!({
        "version":"v1",
        "pagination":{"mode":"auto","page_size":10}
    }))
    .unwrap();
    let outcome = plan_outcome(&contract, Some(&within)).unwrap();
    assert_eq!(outcome.paginated[0].initial_limit, Some(10));

    let exceeding: sdk_defaults::SdkDefaults = serde_json::from_value(json!({
        "version":"v1",
        "pagination":{"mode":"auto","page_size":25}
    }))
    .unwrap();
    let error = plan_outcome(&contract, Some(&exceeding)).unwrap_err();
    assert!(
        error
            .iter()
            .any(|d| d.code == "sdk-pagination-page-size" && d.message.contains("maximum")),
        "{error:?}"
    );
}
