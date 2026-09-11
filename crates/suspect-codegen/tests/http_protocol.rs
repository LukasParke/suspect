//! Public source Contract -> immutable wire plan seam. Expectations live in a
//! literal, independently sourced normative fixture, not generated snapshots.

#![cfg(feature = "http-protocol")]

use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};
use suspect_codegen::http_protocol::{self as http, Capabilities, Capability, plan};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::{DocumentProvider, ProvidedDocument, Workspace, WorkspaceBuilder};
use suspect_source::Uri;

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/http-protocol-v1.json")).unwrap()
}

fn workspace(files: &[(&str, Value)]) -> Arc<Contract> {
    let directory = tempfile::tempdir().unwrap();
    for (name, value) in files {
        std::fs::write(directory.path().join(name), value.to_string()).unwrap();
    }
    let entry = directory.path().join("api.json");
    let workspace = WorkspaceBuilder::new()
        .root(directory.path())
        .build()
        .unwrap();
    Arc::new(
        Contract::from_workspace(&Arc::new(workspace), &Uri::from_path(&entry).unwrap()).unwrap(),
    )
}

fn provided_contract(witness: &Value) -> (Arc<Contract>, Arc<Workspace>) {
    let provider =
        Arc::new(
            DocumentProvider::new(witness["documents"].as_array().unwrap().iter().map(
                |document| {
                    ProvidedDocument::new(
                        Uri::parse(document["requested"].as_str().unwrap()).unwrap(),
                        Uri::parse(document["effective"].as_str().unwrap()).unwrap(),
                        serde_json::to_vec(&document["value"]).unwrap(),
                    )
                    .unwrap()
                },
            ))
            .unwrap(),
        );
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    let contract = Arc::new(
        Contract::from_workspace(
            &workspace,
            &Uri::parse(witness["entry"].as_str().unwrap()).unwrap(),
        )
        .unwrap(),
    );
    (contract, workspace)
}

fn selected(contract: &Contract) -> Vec<SourceId> {
    contract
        .operations()
        .map(|op| op.source().clone())
        .collect()
}

fn normative_capabilities() -> Capabilities {
    // This identifies the test interpreter, not a native language adapter.
    Capabilities::for_adapter("normative-fixtures-v1", Capability::ALL.iter().copied())
}

fn ordinary() -> Value {
    json!({"openapi":"3.1.2","info":{"title":"Protocol witness","version":"1"},"paths":{"/items":{"get":{"operationId":"getItems","responses":{"200":{"description":"ok"}}}}}})
}

fn assert_diagnostic(protocol: &http::ProtocolPlan, code: &str, pointer: &str) {
    assert!(!protocol.is_admitted());
    assert!(
        protocol.operations().is_empty(),
        "failed plans cannot emit a partial selected SDK"
    );
    assert!(protocol.codec_roots().is_empty());
    assert!(
        protocol.diagnostics().iter().any(|d| d.code() == code
            && d.source().source().pointer() == pointer
            && !d.source().span().is_empty()),
        "missing {code} at {pointer}: {:#?}",
        protocol.diagnostics()
    );
}

// Apply a literal fixture mutation; expected behavior is never derived here.
fn replace(document: &mut Value, pointer: &str, value: Value) {
    let (parent, key) = pointer.rsplit_once('/').unwrap();
    document
        .pointer_mut(parent)
        .unwrap()
        .as_object_mut()
        .unwrap()
        .insert(key.replace("~1", "/").replace("~0", "~"), value);
}

#[test]
fn status_and_media_selection_obey_normative_precedence_and_actual_status() {
    let witnesses = fixture();
    let witness = &witnesses["responseSelection"];
    let contract = workspace(&[("api.json", witness["document"].clone())]);
    let plan = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(plan.is_admitted(), "{:#?}", plan.diagnostics());
    let operation = &plan.operations()[0];
    for case in witness["cases"].as_array().unwrap() {
        let matched = operation
            .match_response(
                case["status"].as_u64().unwrap() as u16,
                case["contentType"].as_str(),
            )
            .unwrap();
        assert_eq!(
            matched.response().status_key(),
            case["response"].as_str().unwrap()
        );
        assert_eq!(
            matched.media().map(|m| m.media_type().declared()),
            case["media"].as_str()
        );
        assert_eq!(matched.is_success(), case["success"].as_bool().unwrap());
    }
    assert_eq!(operation.servers().candidates()[0].template(), "/");
    assert!(operation.servers().candidates()[0].is_default());
}

#[test]
fn planned_parameter_serializations_match_literal_openapi_vectors() {
    let fixture = fixture();
    for witness in fixture["parameterCases"].as_array().unwrap() {
        let path = if witness["parameter"]["in"] == "path" {
            "/items/{color}"
        } else {
            "/items"
        };
        let document = json!({
            "openapi": witness["version"].as_str().unwrap_or("3.1.2"),
            "info": {"title": "Normative parameter serialization", "version": "1"},
            "paths": {path: {"get": {
                "parameters": [witness["parameter"].clone()],
                "responses": {"200": {"description": "ok"}}
            }}}
        });
        let contract = workspace(&[("api.json", document)]);
        let protocol = plan(&contract, &selected(&contract), normative_capabilities());
        assert!(
            protocol.is_admitted(),
            "{witness:#}\n{:#?}",
            protocol.diagnostics()
        );
        let parameter = &protocol.operations()[0].parameters()[0];
        assert_eq!(
            parameter.serialize(&witness["value"]).unwrap().value(),
            witness["wire"].as_str().unwrap(),
            "{witness:#}"
        );
        assert!(contract.schema(parameter.codec().schema().id()).is_some());
        assert!(!parameter.source().terminal().span().is_empty());
    }
}

#[test]
fn malformed_ordinary_declarations_are_not_lost_by_filtered_views() {
    let fixture = fixture();
    for witness in fixture["invalidMetadata"].as_array().unwrap() {
        let mut document = ordinary();
        replace(
            &mut document,
            witness["pointer"].as_str().unwrap(),
            witness["value"].clone(),
        );
        let contract = workspace(&[("api.json", document)]);
        let protocol = plan(&contract, &selected(&contract), normative_capabilities());
        assert_diagnostic(
            &protocol,
            witness["code"].as_str().unwrap(),
            witness["at"].as_str().unwrap(),
        );
    }
}

#[test]
fn strict_baseline_is_an_explicit_atomic_projection_and_capabilities_do_not_leak() {
    let fixture = fixture();
    let contract = workspace(&[("api.json", fixture["baseline"].clone())]);
    let protocol = plan(&contract, &selected(&contract), Capabilities::default());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    assert_eq!(protocol.operations()[0].method(), http::Method::Get);
    assert_eq!(protocol.codec_roots().len(), 4);
    let contract = workspace(&[("api.json", ordinary())]);
    let protocol = plan(&contract, &selected(&contract), Capabilities::default());
    for capability in [
        Capability::RelativeServers,
        Capability::AnonymousSecurity,
        Capability::UndeclaredResponseBody,
    ] {
        assert!(
            protocol
                .diagnostics()
                .iter()
                .any(|d| d.capability() == Some(capability))
        );
    }
    assert!(protocol.operations().is_empty());
    assert!(protocol.codec_roots().is_empty());
    let all = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(all.is_admitted(), "{:#?}", all.diagnostics());
    assert_eq!(all.version(), 1);
    // Serialization is public structured data with versioned semantics.
    let descriptor = serde_json::to_value(&all).unwrap();
    assert_eq!(descriptor["operations"][0]["method"], "GET");
    assert_eq!(
        descriptor["operations"][0]["security"]["kind"],
        "undeclared"
    );
}

#[test]
fn effective_servers_preserve_defaults_variables_overrides_and_relative_resolution() {
    let fixture = fixture();
    let witness = &fixture["serverCases"];
    let mut document = ordinary();
    document["servers"] = witness["servers"].clone();
    let contract = workspace(&[("api.json", document.clone())]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    let servers = protocol.operations()[0].servers().candidates();
    assert_eq!(
        servers[0].expand(&BTreeMap::new()).unwrap(),
        witness["defaults"].as_str().unwrap()
    );
    let overrides: BTreeMap<String, String> =
        serde_json::from_value(witness["overrides"].clone()).unwrap();
    assert_eq!(
        servers[0].expand(&overrides).unwrap(),
        witness["overridden"].as_str().unwrap()
    );
    assert_eq!(
        servers[1]
            .resolve_url(witness["documentUrl"].as_str().unwrap(), &BTreeMap::new())
            .unwrap(),
        witness["relativeResolved"].as_str().unwrap()
    );
    let invalid: BTreeMap<String, String> =
        serde_json::from_value(witness["invalidOverride"].clone()).unwrap();
    assert_eq!(
        servers[0].expand(&invalid).unwrap_err().code(),
        "http-server-override-enum"
    );
    assert!(
        servers[0]
            .expand(&BTreeMap::from([("typo".into(), "value".into())]))
            .is_err()
    );
    assert_eq!(
        servers[0].variables()[0]
            .default()
            .source()
            .source()
            .pointer(),
        "/servers/0/variables/basePath/default"
    );
    document["paths"]["/items"]["servers"] = json!([{"url":"https://path.example.test"}]);
    document["paths"]["/items"]["get"]["servers"] = json!([]);
    let contract = workspace(&[("api.json", document)]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    let servers = protocol.operations()[0].servers();
    assert_eq!(
        servers.source().source().pointer(),
        "/paths/~1items/get/servers"
    );
    assert!(servers.candidates()[0].source().is_none());
    assert_eq!(
        servers.candidates()[0]
            .default_from()
            .unwrap()
            .source()
            .pointer(),
        "/paths/~1items/get/servers"
    );
    assert_eq!(servers.candidates()[0].template(), "/");
}

#[test]
fn security_alternatives_are_or_members_are_and_and_scopes_are_not_roles() {
    let fixture = fixture();
    let witness = &fixture["securityCases"];
    let mut document = ordinary();
    document["security"] = witness["security"].clone();
    document["components"] = json!({"securitySchemes":witness["schemes"].clone()});
    let contract = workspace(&[("api.json", document.clone())]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    let security = protocol.operations()[0].security();
    let alternatives = security.alternatives();
    assert_eq!(alternatives.len(), 5);
    assert!(alternatives[0].is_anonymous());
    assert_eq!(alternatives[1].requirements().len(), 2);
    let bearer = alternatives[1]
        .requirements()
        .iter()
        .find(|r| r.name() == "token")
        .unwrap();
    assert!(matches!(
        bearer.credential(),
        http::CredentialHook::Bearer { .. }
    ));
    let http::Permissions::Roles(roles) = bearer.permissions() else {
        panic!("bearer names are roles")
    };
    assert_eq!(roles[0].value(), "reader");
    assert_eq!(roles[0].source().source().pointer(), "/security/1/token/0");
    let oauth = &alternatives[2].requirements()[0];
    let http::Permissions::Scopes(scopes) = oauth.permissions() else {
        panic!("OAuth names are scopes")
    };
    assert_eq!(scopes[0].value(), "read:items");
    let http::CredentialHook::OAuth2 { flows, .. } = oauth.credential() else {
        panic!("OAuth credential hook")
    };
    assert_eq!(flows[0].kind(), http::OAuthFlowKind::AuthorizationCode);
    assert_eq!(
        flows[0].token_url().unwrap().value(),
        "https://auth.example.test/token"
    );
    assert_eq!(flows[0].scopes()["read:items"].value(), "Read items");
    assert!(matches!(
        alternatives[3].requirements()[0].credential(),
        http::CredentialHook::OpenIdConnect { .. }
    ));
    assert!(matches!(
        alternatives[4].requirements()[0].credential(),
        http::CredentialHook::Basic
    ));
    document["paths"]["/items"]["get"]["security"] = json!([]);
    let contract = workspace(&[("api.json", document)]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    assert!(matches!(
        protocol.operations()[0].security(),
        http::SecurityPlan::NoAuth { .. }
    ));
}

#[test]
fn multipart_parts_use_only_real_per_part_codecs_and_bound_file_bytes_separately() {
    let fixture = fixture();
    let contract = workspace(&[("api.json", fixture["multipart"].clone())]);
    let protocol = plan(
        &contract,
        &selected(&contract),
        normative_capabilities().with_limits(http::ByteLimits::new(4096, 100, 20)),
    );
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    let body = protocol.operations()[0].body().unwrap();
    assert!(body.required());
    let http::Representation::Multipart {
        multipart:
            http::MultipartPlan::Named {
                rules,
                parts,
                additional,
            },
    } = body.media()[0].representation()
    else {
        panic!("named multipart")
    };
    assert_eq!(
        rules
            .required()
            .iter()
            .map(|v| v.value().as_str())
            .collect::<Vec<_>>(),
        ["file", "metadata"]
    );
    assert!(matches!(additional, http::AdditionalParts::Forbidden));
    let file = parts.iter().find(|p| p.name() == Some("file")).unwrap();
    assert!(file.required());
    let http::PartRepresentation::Binary { bytes } = file.representation() else {
        panic!("unencoded file bytes")
    };
    assert_eq!(bytes.max_bytes(), 100);
    assert_eq!(
        file.content_types()
            .iter()
            .map(|m| m.declared())
            .collect::<Vec<_>>(),
        ["image/png", "image/jpeg"]
    );
    assert!(file.headers()[0].required());
    let labels = parts.iter().find(|p| p.name() == Some("labels")).unwrap();
    assert_eq!(
        labels.multiplicity(),
        http::PartMultiplicity::RepeatedArrayItems
    );
    assert_eq!(*labels.min_items().unwrap().value(), 1);
    assert_eq!(*labels.max_items().unwrap().value(), 3);
    let roots = protocol
        .codec_roots()
        .iter()
        .map(|id| id.pointer())
        .collect::<Vec<_>>();
    let expected = fixture["multipartCodecRoots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(roots, expected);
    assert!(!protocol.codec_roots().contains(rules.schema().id()));
    assert!(!protocol.codec_roots().contains(file.schema().id()));
    for root in protocol.codec_roots() {
        assert!(contract.schema(root).is_some());
    }
}

#[test]
fn form_content_encoding_and_explicit_style_encoding_are_distinct() {
    let fixture = fixture();
    let contract = workspace(&[("api.json", fixture["form"].clone())]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    let http::Representation::Form { form } =
        protocol.operations()[0].body().unwrap().media()[0].representation()
    else {
        panic!("form")
    };
    let id = form
        .fields()
        .iter()
        .find(|p| p.name() == Some("id"))
        .unwrap();
    assert!(matches!(
        id.representation(),
        http::PartRepresentation::Text {
            outer_encoding: http::PercentEncoding::FormUrlEncoded,
            ..
        }
    ));
    let address = form
        .fields()
        .iter()
        .find(|p| p.name() == Some("address"))
        .unwrap();
    assert!(matches!(
        address.representation(),
        http::PartRepresentation::Json {
            outer_encoding: http::PercentEncoding::FormUrlEncoded,
            ..
        }
    ));
    let codes = form
        .fields()
        .iter()
        .find(|p| p.name() == Some("codes"))
        .unwrap();
    assert_eq!(codes.multiplicity(), http::PartMultiplicity::One);
    assert!(
        codes.content_types().is_empty(),
        "explicit style/explode ignores contentType"
    );
    assert!(matches!(
        codes.representation(),
        http::PartRepresentation::Style {
            serialization: http::ParameterSerialization::Style {
                style: http::Style::Form,
                explode: false,
                ..
            },
            ..
        }
    ));
    let expected = fixture["formCodecRoots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        protocol
            .codec_roots()
            .iter()
            .map(|id| id.pointer())
            .collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn oas32_streams_bind_parsed_item_schemas_without_inferring_data_json_or_sentinels() {
    let fixture = fixture();
    let contract = workspace(&[("api.json", fixture["stream"].clone())]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    let operation = &protocol.operations()[0];
    let matched = operation
        .match_response(200, Some("text/event-stream"))
        .unwrap();
    let http::Representation::Stream { stream } = matched.media().unwrap().representation() else {
        panic!("SSE")
    };
    assert_eq!(stream.framing(), http::StreamFraming::ServerSentEvents);
    let schema = contract.schema(stream.item_codec().schema().id()).unwrap();
    assert_eq!(schema.raw()["properties"]["data"]["type"], "string");
    assert_eq!(schema.raw()["properties"]["retry"]["type"], "integer");
    let expected = fixture["streamCodecRoots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        protocol
            .codec_roots()
            .iter()
            .map(|id| id.pointer())
            .collect::<Vec<_>>(),
        expected
    );
    // A 3.1 whole-content schema and a vendor sentinel do not opt in to any
    // item/data interpretation, even when all standard capabilities are set.
    let mut legacy = ordinary();
    legacy["paths"]["/items"]["get"]["responses"]["200"]["content"] = json!({"text/event-stream":{"schema":{"type":"object"},"x-speakeasy-sse-sentinel":"[DONE]"}});
    let contract = workspace(&[("api.json", legacy)]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert_diagnostic(
        &protocol,
        "http-stream-item-schema-required",
        "/paths/~1items/get/responses/200/content/text~1event-stream/schema",
    );
    assert!(
        protocol
            .diagnostics()
            .iter()
            .any(|d| d.code() == "http-extension-uninterpreted")
    );
}

#[test]
fn reference_chains_keep_use_sites_terminal_spans_and_indexed_header_roots() {
    let fixture = fixture();
    let files = fixture["sourceReferences"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, document)| (name.as_str(), document.clone()))
        .collect::<Vec<_>>();
    let contract = workspace(&files);
    let selected = contract
        .operations()
        .filter(|op| op.operation_id() == Some("getItem"))
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let protocol = plan(&contract, &selected, normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    assert_eq!(
        protocol.operations().len(),
        1,
        "a Link must not implicitly select getState"
    );
    let operation = &protocol.operations()[0];
    assert_eq!(
        operation.source().use_site().source().pointer(),
        "/paths/~1items~1{id}"
    );
    assert_eq!(
        operation.source().terminal().source().pointer(),
        "/components/pathItems/Item/get"
    );
    assert_eq!(operation.path(), "/items/{id}");
    assert_eq!(operation.parameters().len(), 2);
    let limit = operation
        .parameters()
        .iter()
        .find(|p| p.name() == "limit")
        .unwrap();
    assert_eq!(
        contract.schema(limit.codec().schema().id()).unwrap().raw()["type"],
        "string"
    );
    assert_eq!(
        limit.source().terminal().source().pointer(),
        "/components/pathItems/Item/get/parameters/0"
    );
    let response = &operation.responses()[0];
    assert_eq!(
        response.source().use_site().source().pointer(),
        "/components/pathItems/Item/get/responses/200"
    );
    assert_eq!(
        response.source().terminal().source().pointer(),
        "/components/responses/Item~1Result~0"
    );
    assert_eq!(response.source().references().len(), 2);
    assert_eq!(response.description().value(), "Use-site description");
    let header = &response.headers()[0];
    assert!(header.required());
    assert_eq!(
        header.source().use_site().source().pointer(),
        "/components/responses/Item~1Result~0/headers/X-Rate"
    );
    assert!(
        header
            .source()
            .terminal()
            .source()
            .document()
            .as_str()
            .ends_with("/headers.json")
    );
    assert_eq!(header.source().terminal().source().pointer(), "/Rate");
    assert_eq!(header.codec().schema().id().pointer(), "/Rate/schema");
    assert!(
        protocol
            .codec_roots()
            .contains(header.codec().schema().id())
    );
    let link = &response.links()[0];
    assert_eq!(
        link.source().terminal().source().pointer(),
        "/components/links/State"
    );
    assert_eq!(link.parameters()["id"].value(), "$response.body#/id");
    assert_eq!(
        link.request_body().unwrap().value()["$ref"],
        "literal-instance-data"
    );
    assert!(
        !protocol
            .codec_roots()
            .iter()
            .any(|id| id.pointer().contains("/links/"))
    );
    for location in [
        operation.source().use_site(),
        operation.source().terminal(),
        response.source().use_site(),
        response.source().terminal(),
        header.source().use_site(),
        header.source().terminal(),
        link.source().use_site(),
        link.source().terminal(),
    ] {
        assert_eq!(
            Some(location.span()),
            contract.source_span(location.source())
        );
        assert!(contract.source(location.source()).is_some());
    }
}

#[test]
fn duplicate_overridden_parameters_and_malformed_reference_targets_are_located() {
    let mut document = ordinary();
    document["paths"]["/items"]["parameters"] = json!([
        {"name":"limit","in":"query","schema":{"type":"integer"}},
        {"name":"limit","in":"query","schema":{"type":"string"}}
    ]);
    document["paths"]["/items"]["get"]["parameters"] =
        json!([{ "name":"limit","in":"query","schema":{"type":"string"} }]);
    let contract = workspace(&[("api.json", document)]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert_diagnostic(
        &protocol,
        "http-parameter-duplicate",
        "/paths/~1items/parameters/1",
    );
    let mut document = ordinary();
    document["paths"]["/items"]["get"]["requestBody"] = json!({"$ref":"broken.json#/Body"});
    let contract = workspace(&[("api.json", document), ("broken.json", json!({"Body":[]}))]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert_diagnostic(&protocol, "http-metadata-object", "/Body");
    assert!(protocol.diagnostics().iter().any(|d| {
        d.code() == "http-metadata-object"
            && d.source()
                .source()
                .document()
                .as_str()
                .ends_with("/broken.json")
    }));
    assert!(protocol.diagnostics().iter().any(|d| {
        d.code() == "http-metadata-object"
            && d.related()
                .iter()
                .any(|at| at.source().pointer() == "/paths/~1items/get/requestBody")
    }));
}

#[test]
fn response_matching_uses_parameters_requires_real_media_and_never_falls_back_across_status() {
    let mut document = ordinary();
    document["paths"]["/items"]["get"]["responses"] = json!({
        "200":{"description":"exact","content":{
            "application/json":{"schema":{}},
            "application/json;profile=\"https://example.test/v1\"":{"schema":{}},
            "application/json;profile=\"https://example.test/v2\"":{"schema":{}}
        }},
        "2XX":{"description":"class","content":{"text/plain":{"schema":{"type":"string"}}}},
        "default":{"description":"fallback"}
    });
    let contract = workspace(&[("api.json", document)]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    let operation = &protocol.operations()[0];
    let matched = operation
        .match_response(
            200,
            Some("application/json; profile=\"https://example.test/v2\"; charset=UTF-8"),
        )
        .unwrap();
    assert_eq!(
        matched.media().unwrap().media_type().declared(),
        "application/json;profile=\"https://example.test/v2\""
    );
    assert!(matches!(
        operation.match_response(200, Some("text/plain")),
        Err(http::ResponseMatchError::UndeclaredMediaType(_))
    ));
    assert_eq!(
        operation.match_response(200, None).unwrap_err(),
        http::ResponseMatchError::MissingContentType
    );
    assert!(matches!(
        operation.match_response(200, Some("application/json; charset")),
        Err(http::ResponseMatchError::InvalidContentType(_))
    ));
    assert!(
        operation
            .match_response(207, Some("text/plain"))
            .unwrap()
            .is_success()
    );
    assert_eq!(
        operation
            .match_response(207, Some("text/plain; charset=iso-8859-1"))
            .unwrap_err(),
        http::ResponseMatchError::UnsupportedCharset("iso-8859-1".into())
    );
    assert!(!operation.match_response(503, None).unwrap().is_success());
    // Default-only success must remain a success; default is a matching rule.
    let mut document = ordinary();
    document["paths"]["/items"]["get"]["responses"] = json!({"default":{"description":"any"}});
    let contract = workspace(&[("api.json", document)]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(
        protocol.operations()[0]
            .match_response(201, None)
            .unwrap()
            .is_success()
    );
}

#[test]
fn overlapping_media_declarations_and_undefined_style_combinations_are_refused() {
    let mut document = ordinary();
    document["paths"]["/items"]["get"]["responses"]["200"]["content"] = json!({
        "application/json;profile=a":{"schema":{}},
        "application/json;charset=utf-8":{"schema":{}}
    });
    let contract = workspace(&[("api.json", document)]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert_diagnostic(
        &protocol,
        "http-media-type-ambiguous",
        "/paths/~1items/get/responses/200/content/application~1json;profile=a",
    );
    for parameter in [
        json!({"name":"p","in":"query","style":"deepObject","schema":{"type":"object","additionalProperties":{"type":"string"}}}),
        json!({"name":"p","in":"query","style":"pipeDelimited","explode":true,"schema":{"type":"array","items":{"type":"string"}}}),
        json!({"name":"p","in":"cookie","schema":{"type":"array","items":{"type":"string"}}}),
    ] {
        let mut document = ordinary();
        document["paths"]["/items"]["get"]["parameters"] = json!([parameter]);
        let contract = workspace(&[("api.json", document)]);
        let protocol = plan(&contract, &selected(&contract), normative_capabilities());
        assert!(!protocol.is_admitted());
        assert!(
            protocol
                .diagnostics()
                .iter()
                .any(|d| d.code() == "http-parameter-combination-undefined"
                    && !d.source().span().is_empty()),
            "{:#?}",
            protocol.diagnostics()
        );
    }
    let mut document = ordinary();
    document["paths"]["/items"]["get"]["parameters"] = json!([{"name":"p","in":"query","style":"deepObject","explode":true,"schema":{"type":"object","properties":{"nested":{"type":"object"}}}}]);
    let contract = workspace(&[("api.json", document)]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert_diagnostic(
        &protocol,
        "http-scalar-shape-unsupported",
        "/paths/~1items/get/parameters/0/schema/properties/nested",
    );
}

#[test]
fn head_and_content_forbidden_statuses_retain_metadata_without_unused_body_codecs() {
    let mut document = ordinary();
    document["paths"]["/items"] = json!({"head":{"responses":{
        "200":{"description":"header-only","headers":{"X-Count":{"required":true,"schema":{"type":"integer"}}},"content":{"application/json":{"schema":{"type":"object"}}}}
    }},"get":{"responses":{"204":{"description":"empty","content":{"application/json":{"schema":{"type":"object"}}}}}}});
    let contract = workspace(&[("api.json", document)]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    assert_eq!(
        protocol
            .codec_roots()
            .iter()
            .map(|id| id.pointer())
            .collect::<Vec<_>>(),
        ["/paths/~1items/head/responses/200/headers/X-Count/schema"]
    );
    for operation in protocol.operations() {
        let matched = operation
            .match_response(
                if operation.method() == http::Method::Head {
                    200
                } else {
                    204
                },
                None,
            )
            .unwrap();
        assert_eq!(
            matched.body_disposition(),
            http::ResponseBodyDisposition::ForbiddenByHttp
        );
        assert!(matched.media().is_none());
        assert_eq!(
            matched.response().media().len(),
            1,
            "declared content metadata remains available"
        );
    }
}

#[test]
fn finite_positional_multipart_binds_prefix_and_item_encoding_to_actual_schemas() {
    let fixture = fixture();
    let contract = workspace(&[("api.json", fixture["positionalMultipart"].clone())]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    let body = protocol.operations()[0].body().unwrap();
    let http::Representation::Multipart {
        multipart:
            http::MultipartPlan::Positional {
                schema,
                prefix,
                items,
                min_items,
                max_items,
            },
    } = body.media()[0].representation()
    else {
        panic!("positional multipart")
    };
    assert_eq!(*min_items.as_ref().unwrap().value(), 1);
    assert_eq!(*max_items.as_ref().unwrap().value(), 3);
    assert_eq!(prefix.len(), 1);
    assert!(matches!(
        prefix[0].representation(),
        http::PartRepresentation::Text { .. }
    ));
    let http::AdditionalParts::Allowed(item) = items else {
        panic!("itemEncoding applies to every remaining part")
    };
    assert!(matches!(
        item.representation(),
        http::PartRepresentation::Json { .. }
    ));
    assert_eq!(
        item.encoding_source()
            .unwrap()
            .terminal()
            .source()
            .pointer(),
        "/paths/~1parts/post/requestBody/content/multipart~1mixed/itemEncoding"
    );
    assert!(!protocol.codec_roots().contains(schema.id()));
    let expected = fixture["positionalCodecRoots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        protocol
            .codec_roots()
            .iter()
            .map(|id| id.pointer())
            .collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn binary_schema_policies_never_construct_json_roots_and_legacy_markers_are_opt_in() {
    let mut document = ordinary();
    document["openapi"] = json!("3.2.0");
    document["paths"]["/items"]["get"]["responses"]["200"]["content"] = json!({"application/octet-stream":{"schema":{"maxLength":serde_json::from_str::<Value>("5e1").unwrap()}}});
    let contract = workspace(&[("api.json", document)]);
    let protocol = plan(
        &contract,
        &selected(&contract),
        normative_capabilities().with_limits(http::ByteLimits::new(100, 100, 100)),
    );
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    let http::Representation::Binary { schema, bytes } =
        protocol.operations()[0].responses()[0].media()[0].representation()
    else {
        panic!("bytes")
    };
    assert!(schema.is_some());
    assert_eq!(bytes.max_bytes(), 50);
    assert_eq!(*bytes.declared_max_bytes().unwrap().value(), 50);
    assert!(protocol.codec_roots().is_empty());

    let mut legacy = ordinary();
    legacy["paths"]["/items"]["get"]["responses"]["200"]["content"] =
        json!({"application/octet-stream":{"schema":{"type":"string","format":"binary"}}});
    let contract = workspace(&[("api.json", legacy.clone())]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert_diagnostic(
        &protocol,
        "http-binary-legacy-marker",
        "/paths/~1items/get/responses/200/content/application~1octet-stream/schema/format",
    );
    let protocol = plan(
        &contract,
        &selected(&contract),
        normative_capabilities().with_profile(http::CompatibilityProfile::LegacyBinaryStringV1),
    );
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    assert!(protocol.codec_roots().is_empty());
    assert!(
        protocol
            .diagnostics()
            .iter()
            .any(|d| d.code() == "http-compatibility-profile")
    );
    legacy["openapi"] = json!("3.0.4");
    let contract = workspace(&[("api.json", legacy)]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    assert!(protocol.codec_roots().is_empty());
    assert!(
        !protocol
            .diagnostics()
            .iter()
            .any(|d| d.code() == "http-compatibility-profile")
    );
}

#[test]
fn body_structure_and_reference_assertions_cannot_be_dropped_before_part_planning() {
    let fixture = fixture();
    let mut document = fixture["multipart"].clone();
    document["components"] = json!({"schemas":{"Upload":document["paths"]["/upload"]["post"]["requestBody"]["content"]["multipart/form-data"]["schema"].clone()}});
    document["paths"]["/upload"]["post"]["requestBody"]["content"]["multipart/form-data"]["schema"] =
        json!({"$ref":"#/components/schemas/Upload","required":["file"]});
    let contract = workspace(&[("api.json", document)]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert_diagnostic(
        &protocol,
        "http-wire-reference-siblings",
        "/paths/~1upload/post/requestBody/content/multipart~1form-data/schema/required",
    );

    let mut document = fixture["multipart"].clone();
    document["paths"]["/upload"]["post"]["requestBody"]["content"]["multipart/form-data"]["schema"]
        ["additionalProperties"] = json!(true);
    let contract = workspace(&[("api.json", document)]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert_diagnostic(
        &protocol,
        "http-form-untyped-extras",
        "/paths/~1upload/post/requestBody/content/multipart~1form-data/schema/additionalProperties",
    );

    let mut document = fixture["multipart"].clone();
    document["paths"]["/upload"]["post"]["requestBody"]["content"]["multipart/form-data"]["encoding"]
        ["typo"] = json!({"contentType":"application/json"});
    let contract = workspace(&[("api.json", document)]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert_diagnostic(
        &protocol,
        "http-encoding-property-unknown",
        "/paths/~1upload/post/requestBody/content/multipart~1form-data/encoding/typo",
    );
}

#[test]
fn all_indexed_standard_methods_and_api_key_locations_are_preserved() {
    let mut document = ordinary();
    document["paths"] = json!({"/methods":{}});
    for method in [
        "get", "put", "post", "delete", "options", "head", "patch", "trace",
    ] {
        document["paths"]["/methods"][method] =
            json!({"operationId":method,"responses":{"200":{"description":"ok"}}});
    }
    let contract = workspace(&[("api.json", document)]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    assert_eq!(
        protocol
            .operations()
            .iter()
            .map(|op| op.method().as_str())
            .collect::<Vec<_>>(),
        [
            "DELETE", "GET", "HEAD", "OPTIONS", "PATCH", "POST", "PUT", "TRACE"
        ]
    );
    for (location, expected) in [
        ("header", http::ParameterLocation::Header),
        ("query", http::ParameterLocation::Query),
        ("cookie", http::ParameterLocation::Cookie),
    ] {
        let mut document = ordinary();
        document["security"] = json!([{"apiKey":[]}]);
        document["components"] = json!({"securitySchemes":{"apiKey":{"type":"apiKey","in":location,"name":"credential"}}});
        let contract = workspace(&[("api.json", document)]);
        let protocol = plan(&contract, &selected(&contract), normative_capabilities());
        assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
        let requirement = &protocol.operations()[0].security().alternatives()[0].requirements()[0];
        let http::CredentialHook::ApiKey { location, name } = requirement.credential() else {
            panic!("API key")
        };
        assert_eq!(*location, expected);
        assert_eq!(name.value(), "credential");
    }
}

#[test]
fn wire_guards_refuse_reserved_delimiter_injection_and_nested_values() {
    let mut document = ordinary();
    document["paths"]["/items"]["get"]["parameters"] = json!([
        {"name":"q","in":"query","allowReserved":true,"schema":{"type":"string"}},
        {"name":"X-Test","in":"header","schema":{"type":"string"}},
        {"name":"map","in":"query","schema":{"type":"object"}},
        {"name":"list","in":"query","style":"spaceDelimited","schema":{"type":"array","items":{"type":"string"}}}
    ]);
    let contract = workspace(&[("api.json", document)]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    let parameters = protocol.operations()[0].parameters();
    assert_eq!(
        parameters[0]
            .serialize(&json!("one&admin=true"))
            .unwrap_err()
            .code(),
        "http-reserved-value-escaping"
    );
    assert_eq!(
        parameters[1]
            .serialize(&json!("ok\r\nInjected: value"))
            .unwrap_err()
            .code(),
        "http-wire-control"
    );
    assert_eq!(
        parameters[2]
            .serialize(&json!({"nested":{"x":1}}))
            .unwrap_err()
            .code(),
        "http-wire-value"
    );
    assert_eq!(
        parameters[3]
            .serialize(&json!(["two words"]))
            .unwrap_err()
            .code(),
        "http-delimiter-escaping"
    );
}

#[test]
fn request_media_choices_cannot_bypass_the_most_specific_schema() {
    let mut document = ordinary();
    document["paths"]["/items"] = json!({"post":{"requestBody":{"content":{
        "application/json":{"schema":{"type":"object","required":["name"],"properties":{"name":{"type":"string"}}}},
        "application/*":{},"*/*":{}
    }},"responses":{"204":{"description":"accepted"}}}});
    let contract = workspace(&[("api.json", document)]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    let body = protocol.operations()[0].body().unwrap();
    let selected = body.match_media("Application/JSON; charset=UTF-8").unwrap();
    assert_eq!(selected.media_type().declared(), "application/json");
    assert!(matches!(
        selected.representation(),
        http::Representation::Json { codec: Some(_) }
    ));
    assert_eq!(
        body.match_media("application/pdf")
            .unwrap()
            .media_type()
            .declared(),
        "application/*"
    );
    assert_eq!(
        body.match_media("image/png")
            .unwrap()
            .media_type()
            .declared(),
        "*/*"
    );
    assert!(matches!(
        body.match_media("*/*"),
        Err(http::MediaMatchError::InvalidContentType(_))
    ));
}

#[test]
fn oas32_custom_methods_preserve_tokens_sources_and_http_case_semantics() {
    let fixture = fixture();
    let witness = &fixture["oas32Methods"];
    let contract = workspace(&[("api.json", witness["document"].clone())]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    for (operation, expected) in protocol
        .operations()
        .iter()
        .zip(witness["expected"].as_array().unwrap())
    {
        assert_eq!(
            operation.method().as_str(),
            expected["token"].as_str().unwrap()
        );
        assert_eq!(
            serde_json::to_value(operation.method()).unwrap(),
            expected["token"]
        );
        assert_eq!(
            operation.source().use_site().source().pointer(),
            expected["source"].as_str().unwrap()
        );
        assert_eq!(
            operation.source().terminal().source().pointer(),
            expected["source"].as_str().unwrap()
        );
    }
    assert_eq!(protocol.operations().len(), 7);
    let lower_head = protocol
        .operations()
        .iter()
        .find(|op| op.method().as_str() == "head")
        .unwrap();
    assert_ne!(lower_head.method(), http::Method::Head);
    assert_eq!(
        lower_head
            .match_response(200, Some("application/json"))
            .unwrap()
            .body_disposition(),
        http::ResponseBodyDisposition::Declared
    );
    assert!(protocol.codec_roots().iter().any(|id| id.pointer() == "/paths/~1methods/additionalOperations/head/responses/200/content/application~1json/schema"));
    let fixed_get = protocol
        .operations()
        .iter()
        .find(|op| op.method().as_str() == "GET")
        .unwrap();
    assert!(fixed_get.method() == http::Method::Get);
    assert!(http::Method::Get == fixed_get.method());
    assert!(fixed_get.method() != http::Method::Post);
    assert!(http::Method::Post != fixed_get.method());
    assert!(lower_head.method() == http::Method::Custom("head".to_owned()));
    assert!(http::Method::Custom("head".to_owned()) == lower_head.method());
    assert!(http::Method::Custom("HEAD".to_owned()) != lower_head.method());
    assert!(matches!(fixed_get.method(), http::Method::Get));
}

#[test]
fn oas32_querystring_content_has_no_name_prefix_and_encodes_its_media_once() {
    let fixture = fixture();
    for witness in fixture["querystringCases"].as_array().unwrap() {
        let mut document = ordinary();
        document["openapi"] = json!("3.2.0");
        document["paths"]["/items"]["get"]["parameters"] = json!([witness["parameter"].clone()]);
        let contract = workspace(&[("api.json", document)]);
        let protocol = plan(&contract, &selected(&contract), normative_capabilities());
        assert!(
            protocol.is_admitted(),
            "{witness:#}\n{:#?}",
            protocol.diagnostics()
        );
        let parameter = &protocol.operations()[0].parameters()[0];
        assert_eq!(
            parameter.serialize(&witness["value"]).unwrap().value(),
            witness["wire"].as_str().unwrap()
        );
        assert_eq!(
            protocol.codec_roots().len(),
            witness["rootCount"].as_u64().unwrap() as usize
        );
        for root in protocol.codec_roots() {
            assert!(contract.schema(root).is_some());
        }
    }
}

#[test]
fn oas32_media_references_stream_items_and_positional_headers_keep_actual_indexed_roots() {
    let fixture = fixture();
    let files = fixture["oas32MediaReferences"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, value)| (name.as_str(), value.clone()))
        .collect::<Vec<_>>();
    let contract = workspace(&files);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    let operation = &protocol.operations()[0];
    assert!(operation.responses()[0].declared_description().is_none());
    assert_eq!(
        operation.responses()[0].summary().unwrap().value(),
        "Sequential representations"
    );
    let media = operation
        .match_response(200, Some("text/event-stream"))
        .unwrap()
        .media()
        .unwrap();
    assert_eq!(
        media.source().use_site().source().pointer(),
        "/paths/~1transfer/post/responses/200/content/text~1event-stream"
    );
    assert_eq!(media.source().terminal().source().pointer(), "/Sse");
    assert_eq!(media.source().references().len(), 2);
    let http::Representation::Stream { stream } = media.representation() else {
        panic!("SSE item stream")
    };
    assert_eq!(
        stream.item_codec().schema().id().pointer(),
        "/Sse/itemSchema"
    );
    assert_eq!(
        stream
            .item_codec()
            .schema()
            .source()
            .terminal()
            .source()
            .pointer(),
        "/$defs/Event"
    );
    assert_eq!(
        contract
            .schema(stream.item_codec().schema().id())
            .unwrap()
            .raw()["maxProperties"],
        4,
        "reference assertions belong to the indexed use-site codec"
    );
    let indexed = contract.operations().next().unwrap().responses()[0].content();
    let indexed_sse = indexed
        .iter()
        .find(|m| m.name() == "text/event-stream")
        .unwrap();
    assert_eq!(
        indexed_sse.item_schema().unwrap().id(),
        stream.item_codec().schema().id()
    );
    assert_eq!(
        indexed_sse.schema_roots(),
        [stream.item_codec().schema().id().clone()]
    );
    assert_eq!(
        Some(media.source().terminal().source().clone()),
        indexed_sse.resolved_source()
    );

    let body = operation.body().unwrap();
    let http::Representation::Multipart {
        multipart:
            http::MultipartPlan::Positional {
                schema,
                prefix,
                items,
                ..
            },
    } = body.media()[0].representation()
    else {
        panic!("positional parts")
    };
    assert!(!protocol.codec_roots().contains(schema.id()));
    assert!(prefix[0].headers()[0].required());
    assert_eq!(
        prefix[0].headers()[0]
            .source()
            .terminal()
            .source()
            .pointer(),
        "/Prefix"
    );
    let http::AdditionalParts::Allowed(item) = items else {
        panic!("remaining byte items")
    };
    assert!(matches!(
        item.representation(),
        http::PartRepresentation::Binary { .. }
    ));
    assert!(item.headers()[0].required());
    assert!(!protocol.codec_roots().contains(item.schema().id()));
    let expected = fixture["oas32MediaRoots"].as_array().unwrap();
    assert_eq!(protocol.codec_roots().len(), expected.len());
    for (actual, expected) in protocol.codec_roots().iter().zip(expected) {
        assert!(
            actual
                .document()
                .as_str()
                .ends_with(&format!("/{}", expected["document"].as_str().unwrap()))
        );
        assert_eq!(actual.pointer(), expected["pointer"].as_str().unwrap());
        assert!(contract.schema(actual).is_some());
        assert!(
            contract
                .source_span(actual)
                .is_some_and(|span| !span.is_empty())
        );
    }
}

#[test]
fn oas32_querystring_override_identity_and_mutual_exclusion_are_source_checked() {
    let fixture = fixture();
    let document = fixture["querystringOverride"].clone();
    let contract = workspace(&[("api.json", document.clone())]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    assert_eq!(protocol.operations()[0].parameters().len(), 1);
    let query = &protocol.operations()[0].parameters()[0];
    assert_eq!(query.location(), http::ParameterLocation::Querystring);
    assert_eq!(
        query.source().use_site().source().pointer(),
        "/paths/~1search/get/parameters/0"
    );
    assert_eq!(
        query.source().terminal().source().pointer(),
        "/components/parameters/Override"
    );
    assert_eq!(
        query.serialize(&json!({"a":1})).unwrap().value(),
        "%7B%22a%22%3A1%7D"
    );
    assert_eq!(
        protocol
            .codec_roots()
            .iter()
            .map(|id| id.pointer())
            .collect::<Vec<_>>(),
        ["/components/parameters/Override/content/application~1json/schema"]
    );
    assert!(query.content_media().is_some());

    let mut different = document.clone();
    different["components"]["parameters"]["Override"]["name"] = json!("differentName");
    let contract = workspace(&[("api.json", different)]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert_diagnostic(
        &protocol,
        "http-querystring-duplicate",
        "/paths/~1search/get/parameters/0",
    );

    let mut mixed = document.clone();
    mixed["paths"]["/search"]["parameters"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"limit","in":"query","schema":{"type":"integer"}}));
    let contract = workspace(&[("api.json", mixed)]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert_diagnostic(
        &protocol,
        "http-querystring-query-conflict",
        "/paths/~1search/get/parameters/0",
    );

    for (field, value, code) in [
        (
            "schema",
            json!({"type":"string"}),
            "http-querystring-field-forbidden",
        ),
        ("style", json!("form"), "http-querystring-field-forbidden"),
        ("explode", json!(false), "http-querystring-field-forbidden"),
        (
            "allowReserved",
            json!(false),
            "http-querystring-field-forbidden",
        ),
        (
            "allowEmptyValue",
            json!(false),
            "http-parameter-field-location",
        ),
    ] {
        let mut invalid = document.clone();
        invalid["components"]["parameters"]["Override"][field] = value;
        let contract = workspace(&[("api.json", invalid)]);
        let protocol = plan(&contract, &selected(&contract), normative_capabilities());
        assert_diagnostic(
            &protocol,
            code,
            &format!("/components/parameters/Override/{field}"),
        );
    }
    let mut malformed_inherited = document;
    malformed_inherited["components"]["parameters"]["Inherited"]["content"]["text/plain"] =
        Value::Null;
    let contract = workspace(&[("api.json", malformed_inherited)]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert_diagnostic(
        &protocol,
        "http-metadata-declaration",
        "/components/parameters/Inherited/content/text~1plain",
    );
}

#[test]
fn oas32_capabilities_isolate_custom_methods_and_whole_query_content() {
    let fixture = fixture();
    let contract = workspace(&[("api.json", fixture["oas32Methods"]["document"].clone())]);
    let limited = Capabilities::for_adapter(
        "fixed-method-only",
        Capability::ALL
            .iter()
            .copied()
            .filter(|c| *c != Capability::CustomMethods),
    );
    let protocol = plan(&contract, &selected(&contract), limited.clone());
    assert!(
        protocol
            .diagnostics()
            .iter()
            .any(|d| d.capability() == Some(Capability::CustomMethods))
    );
    assert!(protocol.operations().is_empty());
    let fixed = contract
        .operations()
        .filter(|op| op.method().as_str() == "GET")
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    assert!(
        plan(&contract, &fixed, limited).is_admitted(),
        "unselected valid custom operations do not require adapter opt-in"
    );

    let mut document = ordinary();
    document["openapi"] = json!("3.2.0");
    document["paths"]["/items"]["get"]["parameters"] =
        json!([fixture["querystringCases"][2]["parameter"].clone()]);
    let contract = workspace(&[("api.json", document)]);
    for disabled in [
        Capability::QuerystringParameters,
        Capability::QuerystringForm,
    ] {
        let limited = Capabilities::for_adapter(
            "no-querystring-adoption",
            Capability::ALL.iter().copied().filter(|c| *c != disabled),
        );
        let protocol = plan(&contract, &selected(&contract), limited);
        assert!(
            protocol
                .diagnostics()
                .iter()
                .any(|d| d.capability() == Some(disabled))
        );
        assert!(protocol.operations().is_empty());
    }
}

#[test]
fn oas32_additional_method_maps_do_not_hide_invalid_or_fixed_tokens() {
    for (additional, code, at) in [
        (
            json!({"GET":{"responses":{"200":{}}}}),
            "http-additional-method-fixed",
            "/paths/~1items/additionalOperations/GET",
        ),
        (
            json!({"HEAD":{"responses":{"200":{}}}}),
            "http-additional-method-fixed",
            "/paths/~1items/additionalOperations/HEAD",
        ),
        (
            json!({"bad method":{"responses":{"200":{}}}}),
            "http-method-token",
            "/paths/~1items/additionalOperations/bad method",
        ),
        (
            json!({"COPY":null}),
            "http-operation-object",
            "/paths/~1items/additionalOperations/COPY",
        ),
        (
            json!([]),
            "http-additional-methods-map",
            "/paths/~1items/additionalOperations",
        ),
    ] {
        let mut document = ordinary();
        document["openapi"] = json!("3.2.0");
        document["paths"]["/items"]["additionalOperations"] = additional;
        let contract = workspace(&[("api.json", document)]);
        let protocol = plan(&contract, &selected(&contract), normative_capabilities());
        assert_diagnostic(&protocol, code, at);
    }
}

#[test]
fn oas32_examples_and_auth_metadata_preserve_field_identity_without_extra_schema_roots() {
    let fixture = fixture();
    let files = fixture["oas32Metadata"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, value)| (name.as_str(), value.clone()))
        .collect::<Vec<_>>();
    let contract = workspace(&files);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    assert_eq!(
        contract.documents().count(),
        3,
        "instance $ref values cannot fetch a document"
    );
    let operation = &protocol.operations()[0];
    let requirement = &operation.security().alternatives()[0].requirements()[0];
    assert!(requirement.deprecated());
    assert_eq!(
        requirement
            .deprecated_source()
            .unwrap()
            .source()
            .source()
            .pointer(),
        "/Device/deprecated"
    );
    let http::CredentialHook::OAuth2 {
        flows,
        metadata_url,
    } = requirement.credential()
    else {
        panic!("OAuth credential metadata")
    };
    assert_eq!(
        metadata_url.as_ref().unwrap().value(),
        "https://auth.example.test/.well-known/oauth-authorization-server"
    );
    assert_eq!(flows[0].kind(), http::OAuthFlowKind::DeviceAuthorization);
    assert_eq!(
        flows[0]
            .device_authorization_url()
            .unwrap()
            .source()
            .source()
            .pointer(),
        "/Device/flows/deviceAuthorization/deviceAuthorizationUrl"
    );
    assert_eq!(
        flows[0].scopes()["read:items"].value(),
        "Read the item collection"
    );
    assert!(operation.responses()[0].declared_description().is_none());
    assert_eq!(operation.responses()[0].description().value(), "");
    assert_eq!(
        operation.responses()[0]
            .description()
            .source()
            .source()
            .pointer(),
        "/paths/~1items/get/responses/200"
    );
    let examples = operation.responses()[0].media()[0].examples();
    let semantic = examples
        .named()
        .iter()
        .find(|e| e.name() == "semantic")
        .unwrap();
    assert_eq!(semantic.source().terminal().source().pointer(), "/Semantic");
    assert_eq!(
        semantic.description().unwrap().value(),
        "Use-site explanation"
    );
    assert_eq!(
        semantic.data_value().unwrap().value()["$ref"],
        "must-not-load.json#/value"
    );
    assert_eq!(
        semantic
            .serialized_value()
            .unwrap()
            .source()
            .source()
            .pointer(),
        "/Semantic/serializedValue"
    );
    let external = examples
        .named()
        .iter()
        .find(|e| e.name() == "external")
        .unwrap();
    assert_eq!(external.data_value().unwrap().value(), &Value::Null);
    assert!(external.value().is_none());
    assert!(external.serialized_value().is_none());
    assert_eq!(
        external.external_value().unwrap().value(),
        "https://example.test/serialized.json"
    );
    assert_eq!(
        protocol
            .codec_roots()
            .iter()
            .map(|id| id.pointer())
            .collect::<Vec<_>>(),
        ["/paths/~1items/get/responses/200/content/application~1json/schema"]
    );
}

#[test]
fn oas32_metadata_invalid_values_are_located_at_their_terminal_declarations() {
    let fixture = fixture();
    for (document, pointer, value, code) in [
        (
            "examples.json",
            "/Semantic/value",
            json!({}),
            "http-example-exclusive",
        ),
        (
            "examples.json",
            "/Semantic/serializedValue",
            json!(3),
            "http-metadata-string",
        ),
        (
            "examples.json",
            "/Semantic/externalValue",
            json!("https://example.test/other"),
            "http-example-exclusive",
        ),
        (
            "auth.json",
            "/Device/deprecated",
            json!("true"),
            "http-metadata-boolean",
        ),
        (
            "auth.json",
            "/Device/flows/deviceAuthorization/deviceAuthorizationUrl",
            json!(false),
            "http-metadata-string",
        ),
    ] {
        let mut inputs = fixture["oas32Metadata"].clone();
        replace(&mut inputs[document], pointer, value);
        let files = inputs
            .as_object()
            .unwrap()
            .iter()
            .map(|(name, value)| (name.as_str(), value.clone()))
            .collect::<Vec<_>>();
        let contract = workspace(&files);
        let protocol = plan(&contract, &selected(&contract), normative_capabilities());
        let diagnostic_at = if pointer == "/Semantic/value" {
            "/Semantic/dataValue"
        } else if pointer == "/Semantic/externalValue" {
            "/Semantic/serializedValue"
        } else {
            pointer
        };
        assert_diagnostic(&protocol, code, diagnostic_at);
    }
}

#[test]
fn oas32_fragment_version_context_comes_from_the_canonical_referring_document() {
    let entry = json!({"openapi":"3.1.2","info":{"title":"Mixed document feature sets","version":"1"},"paths":{"/items":{"$ref":"v32.json#/components/pathItems/Items"}}});
    let v32 = json!({"openapi":"3.2.0","info":{"title":"Versioned reference owner","version":"1"},"components":{"pathItems":{"Items":{"get":{"parameters":[{"$ref":"fragment.json#/Query"}],"responses":{"200":{}}}}}}});
    let fragment = json!({"Query":{"name":"whole","in":"querystring","content":{"text/plain":{"schema":{"type":"string"}}}}});
    let contract = workspace(&[
        ("api.json", entry.clone()),
        ("v32.json", v32.clone()),
        ("fragment.json", fragment.clone()),
    ]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    let parameter = &protocol.operations()[0].parameters()[0];
    assert_eq!(parameter.source().terminal().source().pointer(), "/Query");
    assert_eq!(
        contract.openapi_version_at(parameter.source().terminal().source()),
        "3.2.0"
    );
    assert_eq!(
        parameter.serialize(&json!("q=snow")).unwrap().value(),
        "q%3Dsnow"
    );
    let mut ambiguous = entry;
    ambiguous["paths"]["/legacy"] = json!({"get":{"parameters":[{"$ref":"fragment.json#/Query"}],"responses":{"200":{"description":"legacy"}}}});
    let contract = workspace(&[
        ("api.json", ambiguous),
        ("v32.json", v32),
        ("fragment.json", fragment),
    ]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert_diagnostic(&protocol, "AMBIGUOUS_OPENAPI_CONTEXT", "/Query");
}

#[test]
fn omitted_responses_have_an_explicit_capability_and_no_invented_default() {
    for version in ["3.1.2", "3.2.0"] {
        let mut document = ordinary();
        document["openapi"] = json!(version);
        document["paths"]["/items"]["get"]
            .as_object_mut()
            .unwrap()
            .remove("responses");
        let contract = workspace(&[("api.json", document)]);
        let protocol = plan(&contract, &selected(&contract), normative_capabilities());
        assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
        assert!(protocol.operations()[0].responses().is_empty());
        assert!(protocol.codec_roots().is_empty());
        assert_eq!(
            protocol.operations()[0]
                .match_response(200, None)
                .unwrap_err(),
            http::ResponseMatchError::UndeclaredStatus(200)
        );
        let limited = Capabilities::for_adapter(
            "declared-responses-only",
            Capability::ALL
                .iter()
                .copied()
                .filter(|c| *c != Capability::UndeclaredResponses),
        );
        let protocol = plan(&contract, &selected(&contract), limited);
        assert!(
            protocol
                .diagnostics()
                .iter()
                .any(|d| d.capability() == Some(Capability::UndeclaredResponses))
        );
        assert!(!protocol.is_admitted());
    }
}

#[test]
fn all_capabilities_still_refuse_unimplemented_stream_aggregate_and_tunnel_semantics() {
    let fixture = fixture();
    let mut inputs = fixture["oas32MediaReferences"].clone();
    inputs["streams.json"]["Sse"]["schema"] = json!({"type":"array"});
    let files = inputs
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, value)| (name.as_str(), value.clone()))
        .collect::<Vec<_>>();
    let contract = workspace(&files);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert_diagnostic(&protocol, "http-stream-aggregate-schema", "/Sse/schema");
    let mut inputs = fixture["oas32MediaReferences"].clone();
    inputs["api.json"]["components"]["mediaTypes"]["Parts"]["itemEncoding"]["encoding"] =
        json!({"nested":{"contentType":"application/json"}});
    let files = inputs
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, value)| (name.as_str(), value.clone()))
        .collect::<Vec<_>>();
    let contract = workspace(&files);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert_diagnostic(
        &protocol,
        "http-nested-part-encoding",
        "/components/mediaTypes/Parts/itemEncoding/encoding",
    );
    let mut document = ordinary();
    document["openapi"] = json!("3.2.0");
    document["paths"]["/items"]["additionalOperations"] =
        json!({"CONNECT":{"responses":{"200":{}}}});
    let contract = workspace(&[("api.json", document)]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert_diagnostic(
        &protocol,
        "http-connect-tunnel-unsupported",
        "/paths/~1items/additionalOperations/CONNECT",
    );
}

#[test]
fn canonical_http_and_media_references_keep_effective_physical_sources() {
    let fixture = fixture();
    let witness = &fixture["canonicalResources"];
    let (contract, workspace) = provided_contract(witness);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    assert_eq!(
        contract.entry().as_str(),
        witness["serverBase"].as_str().unwrap()
    );
    let operation = &protocol.operations()[0];
    assert_eq!(
        operation.source().use_site().source().document().as_str(),
        witness["serverBase"].as_str().unwrap()
    );
    assert_eq!(
        operation.source().terminal().source().document().as_str(),
        witness["codecRootDocument"].as_str().unwrap()
    );
    assert_eq!(protocol.codec_roots().len(), 1);
    assert_eq!(
        protocol.codec_roots()[0].document().as_str(),
        witness["codecRootDocument"].as_str().unwrap()
    );
    assert_eq!(
        protocol.codec_roots()[0].pointer(),
        witness["codecRootPointer"].as_str().unwrap()
    );
    let http::Representation::Json { codec: Some(codec) } =
        operation.responses()[0].media()[0].representation()
    else {
        panic!("JSON response codec")
    };
    assert_eq!(
        codec
            .schema()
            .source()
            .terminal()
            .source()
            .document()
            .as_str(),
        witness["schemaTargetDocument"].as_str().unwrap()
    );
    assert_eq!(
        codec.schema().source().terminal().source().pointer(),
        witness["schemaTargetPointer"].as_str().unwrap()
    );
    assert_eq!(contract.documents().count(), 3);
    assert!(workspace.failed_document_uris().is_empty());
    assert!(
        !workspace
            .uris()
            .iter()
            .any(|uri| uri.as_str().starts_with("https://logical.example/"))
    );
}

#[test]
fn server_document_bases_are_physical_and_resource_addresses_are_separate() {
    let fixture = fixture();
    let witness = &fixture["canonicalResources"];
    let (contract, _) = provided_contract(witness);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    let operation = &protocol.operations()[0];
    let server = &operation.servers().candidates()[0];
    assert_eq!(server.url_base(), http::ApiUrlBase::ServerDocument);
    assert_eq!(
        server.document_base().source().document().as_str(),
        witness["serverBase"].as_str().unwrap()
    );
    assert_eq!(
        server.resolve_document_url(&BTreeMap::new()).unwrap(),
        witness["serverUrl"].as_str().unwrap()
    );
    assert_eq!(
        server
            .resolve_url("https://runtime.example/ui/root.json", &BTreeMap::new())
            .unwrap(),
        "https://runtime.example/service"
    );
    let server_context = server.source().unwrap().terminal_resource().unwrap();
    assert_eq!(
        server_context.canonical_uri(),
        "https://logical.example/catalog/api.json#definition"
    );
    assert_eq!(
        server_context.base_uri(),
        "https://logical.example/catalog/api.json"
    );
    assert_eq!(
        server_context.base_source().unwrap().source().pointer(),
        "/$self"
    );
    assert!(
        server_context
            .aliases()
            .iter()
            .any(|uri| uri == witness["entry"].as_str().unwrap())
    );
    assert_eq!(
        server_context.resource().source().document().as_str(),
        witness["serverBase"].as_str().unwrap()
    );
    let source = operation.source();
    assert_eq!(
        source.terminal_resource().unwrap().base_uri(),
        witness["referenceBase"].as_str().unwrap()
    );
    assert_eq!(
        source.references().len(),
        source.reference_resources().len()
    );
    for context in [
        source.use_site_resource().unwrap(),
        source.terminal_resource().unwrap(),
        server_context,
    ] {
        assert_eq!(
            Some(context.source().span()),
            contract.source_span(context.source().source())
        );
        assert_eq!(
            Some(context.resource().span()),
            contract.source_span(context.resource().source())
        );
    }
    let native_without_document_bases = Capabilities::for_adapter(
        "relative-only-legacy",
        Capability::ALL
            .iter()
            .copied()
            .filter(|c| *c != Capability::DocumentRelativeServers),
    );
    let refused = plan(
        &contract,
        &selected(&contract),
        native_without_document_bases,
    );
    assert!(
        refused
            .diagnostics()
            .iter()
            .any(|d| d.capability() == Some(Capability::DocumentRelativeServers))
    );
    assert!(refused.operations().is_empty());

    let mut encoded = witness.clone();
    encoded["documents"][0]["value"]["servers"][0]["url"] = json!("%2e%2e/Api%2Fv1");
    let (contract, _) = provided_contract(&encoded);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    assert_eq!(
        protocol.operations()[0].servers().candidates()[0]
            .resolve_document_url(&BTreeMap::new())
            .unwrap(),
        "https://cdn.example/specs/releases/%2e%2e/Api%2Fv1"
    );
}

#[test]
fn default_and_explicit_empty_servers_retain_the_correct_declaring_document() {
    let fixture = fixture();
    let mut witness = fixture["canonicalResources"].clone();
    witness["documents"][0]["value"]
        .as_object_mut()
        .unwrap()
        .remove("servers");
    let (contract, _) = provided_contract(&witness);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    let server = &protocol.operations()[0].servers().candidates()[0];
    assert!(server.is_default());
    assert_eq!(
        server.resolve_document_url(&BTreeMap::new()).unwrap(),
        "https://cdn.example/"
    );
    witness["documents"][1]["value"]["components"]["pathItems"]["Items"]["get"]["servers"] =
        json!([]);
    let (contract, _) = provided_contract(&witness);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    let server = &protocol.operations()[0].servers().candidates()[0];
    assert_eq!(
        server.default_from().unwrap().source().pointer(),
        "/components/pathItems/Items/get/servers"
    );
    assert_eq!(
        server.resolve_document_url(&BTreeMap::new()).unwrap(),
        "https://storage.example/"
    );
}

#[test]
fn logical_operation_links_use_reference_bases_but_link_servers_use_physical_documents() {
    let fixture = fixture();
    let mut witness = fixture["canonicalResources"].clone();
    witness["documents"][0]["value"]["paths"]["/other"] =
        json!({"get":{"operationId":"getOther","responses":{"200":{}}}});
    witness["documents"][1]["value"]["components"]["responses"]["Ok"]["links"] = json!({"other":{
        "operationRef":"api.json#/paths/~1other/get","server":{"url":"./linked"}
    }});
    let (contract, workspace) = provided_contract(&witness);
    let selected = contract
        .operations()
        .filter(|op| op.operation_id() == Some("getItems"))
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let protocol = plan(&contract, &selected, normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    assert_eq!(
        protocol.operations().len(),
        1,
        "a logical link does not select/invoke another operation"
    );
    let link = &protocol.operations()[0].responses()[0].links()[0];
    let http::LinkTarget::OperationRef { value, operation } = link.target() else {
        panic!("operation URI link")
    };
    assert_eq!(value.value(), "api.json#/paths/~1other/get");
    assert_eq!(
        operation.source().document().as_str(),
        "https://cdn.example/specs/releases/api.json"
    );
    assert_eq!(operation.source().pointer(), "/paths/~1other/get");
    assert_eq!(
        link.source().terminal_resource().unwrap().base_uri(),
        "https://logical.example/catalog/parts.json"
    );
    assert_eq!(
        link.server()
            .unwrap()
            .resolve_document_url(&BTreeMap::new())
            .unwrap(),
        "https://storage.example/artifacts/linked"
    );
    assert!(workspace.failed_document_uris().is_empty());
}

#[test]
fn unresolved_logical_references_keep_their_physical_source_and_logical_base() {
    let fixture = fixture();
    let mut witness = fixture["canonicalResources"].clone();
    witness["documents"][1]["value"]["components"]["mediaTypes"]["Result"]["schema"]["$ref"] =
        json!("models/missing#value");
    let (contract, workspace) = provided_contract(&witness);
    let loaded = workspace.uris();
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert_diagnostic(
        &protocol,
        "http-schema-reference",
        "/components/mediaTypes/Result/schema/$ref",
    );
    let finding = protocol
        .diagnostics()
        .iter()
        .find(|d| d.code() == "http-schema-reference")
        .unwrap();
    assert_eq!(
        finding.source().source().document().as_str(),
        "https://storage.example/artifacts/parts.json"
    );
    assert_eq!(
        Some(finding.source().span()),
        contract.source_span(finding.source().source())
    );
    assert_eq!(
        finding.resource_context().unwrap().base_uri(),
        "https://logical.example/catalog/parts.json"
    );
    assert!(finding.message().contains("models/missing#value"));
    assert!(
        finding
            .message()
            .contains("https://logical.example/catalog/parts.json")
    );
    assert_eq!(
        workspace.uris(),
        loaded,
        "protocol planning must not acquire a logical target"
    );
    assert!(workspace.failed_document_uris().is_empty());
}

#[test]
fn missing_canonical_examples_remain_warnings_but_missing_wire_refs_are_errors() {
    let fixture = fixture();
    let mut witness = fixture["canonicalResources"].clone();
    witness["documents"][1]["value"]["components"]["mediaTypes"]["Result"]["examples"] = json!({
        "declared":{"$ref":"missing-examples.json#/Example"},
        "external":{"externalValue":"examples/value.json"}
    });
    let (contract, workspace) = provided_contract(&witness);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    let finding = protocol
        .diagnostics()
        .iter()
        .find(|d| d.code() == "http-reference-unresolved")
        .unwrap();
    assert_eq!(finding.severity(), http::Severity::Warning);
    assert_eq!(finding.kind(), http::DiagnosticKind::Annotation);
    assert_eq!(
        finding.source().source().pointer(),
        "/components/mediaTypes/Result/examples/declared/$ref"
    );
    assert_eq!(
        finding.resource_context().unwrap().base_uri(),
        "https://logical.example/catalog/parts.json"
    );
    assert!(workspace.failed_document_uris().is_empty());
    let external = protocol.operations()[0].responses()[0].media()[0]
        .examples()
        .named()
        .iter()
        .find(|example| example.name() == "external")
        .unwrap();
    assert_eq!(
        external
            .source()
            .terminal_resource()
            .unwrap()
            .resolve_reference(external.external_value().unwrap().value())
            .unwrap(),
        "https://logical.example/catalog/examples/value.json"
    );
    witness["documents"][1]["value"]["components"]["responses"]["Ok"]["content"]["application/json"] =
        json!({"$ref":"missing-media.json#/Response"});
    let (contract, _) = provided_contract(&witness);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert_diagnostic(
        &protocol,
        "http-reference-unresolved",
        "/components/responses/Ok/content/application~1json/$ref",
    );
    assert!(
        protocol
            .diagnostics()
            .iter()
            .any(|d| d.code() == "http-reference-unresolved"
                && d.severity() == http::Severity::Error)
    );
}

#[test]
fn resource_codec_capabilities_preserve_actual_roots_and_dynamic_candidate_closure() {
    let fixture = fixture();
    let contract = workspace(&[("api.json", fixture["dynamicResourceCodec"].clone())]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    assert_eq!(
        protocol
            .codec_roots()
            .iter()
            .map(|id| id.pointer())
            .collect::<Vec<_>>(),
        ["/paths/~1tree/get/responses/200/content/application~1json/schema"]
    );
    assert_eq!(
        protocol.codec_schema_closure(),
        contract.effective_schema_closure(protocol.codec_roots())
    );
    assert!(
        protocol
            .codec_schema_closure()
            .iter()
            .any(|id| id.pointer() == "/components/schemas/Extended"),
        "resource entered through a nested schema must retain its dynamic candidate binding"
    );
    let dynamic_source = protocol
        .codec_schema_closure()
        .iter()
        .find(|id| id.pointer() == "/components/schemas/Base/properties/next")
        .unwrap();
    let dynamic = contract.dynamic_reference(dynamic_source).unwrap();
    assert_eq!(
        dynamic.initial_target().unwrap().pointer(),
        "/components/schemas/Base"
    );
    assert_eq!(dynamic.dynamic_anchor(), Some("item"));
    assert!(
        dynamic
            .candidates()
            .iter()
            .any(|candidate| candidate.target().pointer() == "/components/schemas/Extended")
    );
    assert_eq!(
        contract.schema(dynamic_source).unwrap().references()[0].keyword,
        "$dynamicRef"
    );
    for capability in [
        Capability::SchemaResources,
        Capability::DynamicSchemaReferences,
    ] {
        let limited = Capabilities::for_adapter(
            "frozen-codec-profile",
            Capability::ALL.iter().copied().filter(|c| *c != capability),
        );
        let refused = plan(&contract, &selected(&contract), limited);
        assert!(
            refused
                .diagnostics()
                .iter()
                .any(|d| d.capability() == Some(capability))
        );
        assert!(refused.codec_roots().is_empty());
        assert!(refused.codec_schema_closure().is_empty());
    }
}

#[test]
fn forbidden_response_bodies_do_not_create_resource_or_dynamic_codec_requirements() {
    let fixture = fixture();
    for (method, status) in [
        ("head", "200"),
        ("get", "204"),
        ("get", "205"),
        ("get", "304"),
    ] {
        let mut document = fixture["dynamicResourceCodec"].clone();
        let response = document["paths"]["/tree"]["get"]["responses"]["200"].clone();
        document["paths"]["/tree"] = json!({method:{"responses":{status:response}}});
        let contract = workspace(&[("api.json", document)]);
        let capabilities = Capabilities::for_adapter(
            "bodyless-profile",
            Capability::ALL.iter().copied().filter(|c| {
                !matches!(
                    c,
                    Capability::SchemaResources | Capability::DynamicSchemaReferences
                )
            }),
        );
        let protocol = plan(&contract, &selected(&contract), capabilities);
        assert!(
            protocol.is_admitted(),
            "{method} {status}: {:#?}",
            protocol.diagnostics()
        );
        assert!(protocol.codec_roots().is_empty());
        assert!(protocol.codec_schema_closure().is_empty());
        assert_eq!(
            protocol.operations()[0]
                .match_response(status.parse().unwrap(), None)
                .unwrap()
                .body_disposition(),
            http::ResponseBodyDisposition::ForbiddenByHttp
        );
    }
}

#[test]
fn canonical_binary_schema_metadata_is_not_a_json_codec_input() {
    let document = json!({
        "openapi":"3.2.0","info":{"title":"Resource-backed byte binding","version":"1"},
        "paths":{"/bytes":{"get":{"responses":{"200":{"content":{"application/octet-stream":{"schema":{"$ref":"https://schemas.example.test/file"}}}}}}}},
        "components":{"schemas":{"File":{"$id":"https://schemas.example.test/file","maxLength":16}}}
    });
    let contract = workspace(&[("api.json", document)]);
    let capabilities = Capabilities::for_adapter(
        "bounded-byte-profile",
        Capability::ALL.iter().copied().filter(|c| {
            !matches!(
                c,
                Capability::SchemaResources | Capability::DynamicSchemaReferences
            )
        }),
    );
    let protocol = plan(&contract, &selected(&contract), capabilities);
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    assert!(protocol.codec_roots().is_empty());
    assert!(protocol.codec_schema_closure().is_empty());
    let http::Representation::Binary {
        schema: Some(schema),
        bytes,
    } = protocol.operations()[0].responses()[0].media()[0].representation()
    else {
        panic!("bounded byte representation")
    };
    assert_eq!(bytes.max_bytes(), 16);
    assert_eq!(
        schema.source().terminal_resource().unwrap().canonical_uri(),
        "https://schemas.example.test/file"
    );
    assert_eq!(
        schema.source().terminal().source().pointer(),
        "/components/schemas/File"
    );
}

#[test]
fn canonical_reference_bases_do_not_rebase_oauth_metadata_urls_or_acquire_credentials() {
    let fixture = fixture();
    let mut witness = fixture["canonicalResources"].clone();
    witness["documents"][0]["value"]["security"] = json!([{"oauth":[]}]);
    witness["documents"][0]["value"]["components"] = json!({"securitySchemes":{"oauth":{
        "type":"oauth2","oauth2MetadataUrl":"./.well-known/authorization-server",
        "flows":{"clientCredentials":{"tokenUrl":"./token","scopes":{}}}
    }}});
    let (contract, workspace) = provided_contract(&witness);
    let loaded = workspace.uris();
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    let requirement = &protocol.operations()[0].security().alternatives()[0].requirements()[0];
    assert_eq!(
        requirement.scheme().terminal_resource().unwrap().base_uri(),
        "https://logical.example/catalog/api.json"
    );
    assert_eq!(
        requirement.credential().url_base(),
        Some(http::ApiUrlBase::EffectiveServer)
    );
    let http::CredentialHook::OAuth2 {
        flows,
        metadata_url,
    } = requirement.credential()
    else {
        panic!("credential hook")
    };
    assert_eq!(
        metadata_url.as_ref().unwrap().value(),
        "./.well-known/authorization-server"
    );
    assert_eq!(flows[0].token_url().unwrap().value(), "./token");
    assert_eq!(flows[0].url_base(), http::ApiUrlBase::EffectiveServer);
    assert_eq!(workspace.uris(), loaded);
    assert!(workspace.failed_document_uris().is_empty());
}

#[test]
fn malformed_canonical_identity_is_located_without_a_retrieval_base_fallback() {
    let witness = json!({"entry":"https://physical.example/api.json","documents":[{
        "requested":"https://physical.example/api.json","effective":"https://physical.example/api.json",
        "value":{"openapi":"3.2.0","$self":17,"info":{"title":"Malformed identity","version":"1"},
            "paths":{"/x":{"get":{"responses":{"200":{}}}}}}
    }]});
    let (contract, _) = provided_contract(&witness);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert_diagnostic(&protocol, "invalid-openapi-self", "/$self");
    let finding = protocol
        .diagnostics()
        .iter()
        .find(|d| d.code() == "invalid-openapi-self")
        .unwrap();
    assert_eq!(
        finding.source().source().document().as_str(),
        "https://physical.example/api.json"
    );
    assert!(finding.resource_context().is_none());
}

#[test]
fn ignored_multipart_style_fields_preserve_explicit_and_default_content_plans() {
    let fixture = fixture();
    let witness = &fixture["ignoredMultipartEncoding"];
    let contract = workspace(&[("api.json", witness["document"].clone())]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert!(protocol.is_admitted(), "{:#?}", protocol.diagnostics());
    let operation = &protocol.operations()[0];
    for media in [
        operation.body().unwrap().media()[0].representation(),
        operation.responses()[0].media()[0].representation(),
    ] {
        let http::Representation::Multipart {
            multipart: http::MultipartPlan::Positional { prefix, items, .. },
        } = media
        else {
            panic!("positional multipart/mixed")
        };
        assert_eq!(prefix.len(), 4);
        for (part, expected) in prefix
            .iter()
            .zip(witness["prefixMedia"].as_array().unwrap())
        {
            assert_eq!(
                part.content_types()
                    .iter()
                    .map(|media| media.declared())
                    .collect::<Vec<_>>(),
                [expected.as_str().unwrap()]
            );
            assert_eq!(part.multiplicity(), http::PartMultiplicity::One);
            assert!(part.encoding_source().is_some());
        }
        assert!(matches!(
            prefix[0].representation(),
            http::PartRepresentation::Json {
                outer_encoding: http::PercentEncoding::None,
                ..
            }
        ));
        assert!(prefix[0].headers()[0].required());
        assert!(matches!(
            prefix[1].representation(),
            http::PartRepresentation::Text {
                scalar: http::ScalarType::Integer,
                outer_encoding: http::PercentEncoding::None,
                ..
            }
        ));
        let http::PartRepresentation::Json { codec, .. } = prefix[2].representation() else {
            panic!("the whole array part uses its JSON codec")
        };
        assert_eq!(
            codec.schema().id().pointer(),
            "/components/mediaTypes/Mixed/schema/prefixItems/2"
        );
        assert!(matches!(
            prefix[3].representation(),
            http::PartRepresentation::Binary { .. }
        ));
        let http::AdditionalParts::Allowed(item) = items else {
            panic!("itemEncoding content")
        };
        assert_eq!(
            item.content_types()[0].declared(),
            witness["itemMedia"].as_str().unwrap()
        );
        assert!(matches!(
            item.representation(),
            http::PartRepresentation::Json { .. }
        ));
        assert_eq!(
            item.encoding_source()
                .unwrap()
                .terminal()
                .source()
                .pointer(),
            "/components/mediaTypes/Mixed/itemEncoding"
        );
    }
    let expected = witness["codecRoots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        protocol
            .codec_roots()
            .iter()
            .map(|id| id.pointer())
            .collect::<Vec<_>>(),
        expected
    );
    assert!(
        !protocol
            .diagnostics()
            .iter()
            .any(|d| d.code() == "http-encoding-content-type-ignored")
    );
    for pointer in [
        "/components/mediaTypes/Mixed/prefixEncoding/0/style",
        "/components/mediaTypes/Mixed/prefixEncoding/0/explode",
        "/components/mediaTypes/Mixed/prefixEncoding/1/style",
        "/components/mediaTypes/Mixed/prefixEncoding/2/explode",
        "/components/mediaTypes/Mixed/prefixEncoding/3/allowReserved",
        "/components/mediaTypes/Mixed/itemEncoding/style",
        "/components/mediaTypes/Mixed/itemEncoding/explode",
        "/components/mediaTypes/Mixed/itemEncoding/allowReserved",
    ] {
        assert!(
            protocol
                .diagnostics()
                .iter()
                .any(|d| d.code() == "http-encoding-style-ignored"
                    && d.source().source().pointer() == pointer
                    && d.severity() == http::Severity::Warning
                    && !d.source().span().is_empty()),
            "missing located ignored-field warning at {pointer}"
        );
    }
}

#[test]
fn ignored_multipart_style_cannot_bypass_the_actual_content_codec_or_metadata_validation() {
    let fixture = fixture();
    let original = &fixture["ignoredMultipartEncoding"]["document"];
    let media = "/components/mediaTypes/Mixed";
    for (field, value, schema, code, pointer) in [
        (
            "contentType",
            json!("text/plain"),
            Some(json!({"type":"object","properties":{"x":{"type":"string"}}})),
            "http-scalar-shape-unsupported",
            "/components/mediaTypes/Mixed/schema/prefixItems/0",
        ),
        (
            "contentType",
            json!("application/json, text/plain"),
            None,
            "http-part-mixed-representations",
            "/components/mediaTypes/Mixed/prefixEncoding/0",
        ),
        (
            "contentType",
            json!("application/octet-stream"),
            None,
            "http-binary-schema-type",
            "/components/mediaTypes/Mixed/schema/prefixItems/0/type",
        ),
        (
            "contentType",
            json!("not a media type"),
            None,
            "http-media-type-invalid",
            "/components/mediaTypes/Mixed/prefixEncoding/0/contentType",
        ),
        (
            "explode",
            json!("false"),
            None,
            "http-metadata-boolean",
            "/components/mediaTypes/Mixed/prefixEncoding/0/explode",
        ),
        (
            "style",
            json!(false),
            None,
            "http-metadata-string",
            "/components/mediaTypes/Mixed/prefixEncoding/0/style",
        ),
    ] {
        let mut document = original.clone();
        document
            .pointer_mut(&format!("{media}/prefixEncoding/0"))
            .unwrap()[field] = value;
        if let Some(schema) = schema {
            document
                .pointer_mut(&format!("{media}/schema/prefixItems"))
                .unwrap()[0] = schema;
        }
        let contract = workspace(&[("api.json", document)]);
        let protocol = plan(&contract, &selected(&contract), normative_capabilities());
        assert_diagnostic(&protocol, code, pointer);
    }
}

#[test]
fn active_form_styles_keep_their_oas31_whole_property_and_oas32_item_semantics() {
    for (version, expected_multiplicity, expected_suffix) in [
        (
            "3.1.2",
            http::PartMultiplicity::One,
            "/schema/properties/values",
        ),
        (
            "3.2.0",
            http::PartMultiplicity::RepeatedArrayItems,
            "/schema/properties/values/items",
        ),
    ] {
        for media_type in ["multipart/form-data", "application/x-www-form-urlencoded"] {
            let document = json!({
                "openapi":version,"info":{"title":"Active form encoding","version":"1"},
                "paths":{"/form":{"post":{"requestBody":{"content":{media_type:{
                    "schema":{"type":"object","additionalProperties":false,"properties":{"values":{"type":"array","items":{"type":"string"}}}},
                    "encoding":{"values":{"explode":false,"contentType":"application/json"}}
                }}},"responses":{"204":{"description":"accepted"}}}}}
            });
            let contract = workspace(&[("api.json", document)]);
            let protocol = plan(&contract, &selected(&contract), normative_capabilities());
            assert!(
                protocol.is_admitted(),
                "{version} {media_type}: {:#?}",
                protocol.diagnostics()
            );
            let part = match protocol.operations()[0].body().unwrap().media()[0].representation() {
                http::Representation::Multipart {
                    multipart: http::MultipartPlan::Named { parts, .. },
                } => &parts[0],
                http::Representation::Form { form } => &form.fields()[0],
                _ => panic!("named form representation"),
            };
            assert_eq!(part.multiplicity(), expected_multiplicity);
            assert!(
                part.content_types().is_empty(),
                "active RFC6570 fields ignore contentType"
            );
            let http::PartRepresentation::Style {
                codec,
                serialization:
                    http::ParameterSerialization::Style {
                        style,
                        explode,
                        shape,
                        percent_encoding,
                    },
            } = part.representation()
            else {
                panic!("explicit explode selects active form style")
            };
            assert_eq!(*style, http::Style::Form);
            assert!(!explode);
            assert!(codec.schema().id().pointer().ends_with(expected_suffix));
            assert_eq!(
                *percent_encoding,
                if media_type == "multipart/form-data" {
                    http::PercentEncoding::None
                } else {
                    http::PercentEncoding::UriComponent
                }
            );
            if version == "3.1.2" {
                assert!(matches!(shape, http::WireShape::Array { .. }));
            } else {
                assert!(matches!(shape, http::WireShape::Scalar { .. }));
            }
            assert!(
                protocol
                    .diagnostics()
                    .iter()
                    .any(|d| d.code() == "http-encoding-content-type-ignored")
            );
            assert!(
                !protocol
                    .diagnostics()
                    .iter()
                    .any(|d| d.code() == "http-encoding-style-ignored")
            );
        }
    }
}

#[test]
fn older_oas_versions_do_not_gain_invented_positional_or_named_mixed_encodings() {
    let mut document = json!({
        "openapi":"3.1.2","info":{"title":"No inferred mixed part correspondence","version":"1"},
        "paths":{"/parts":{"post":{"requestBody":{"content":{"multipart/mixed":{
            "schema":{"type":"object","additionalProperties":false,"properties":{"value":{"type":"string"}}},
            "encoding":{"value":{"contentType":"application/json","style":"form"}}
        }}},"responses":{"204":{"description":"accepted"}}}}}
    });
    let contract = workspace(&[("api.json", document.clone())]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert_diagnostic(
        &protocol,
        "http-multipart-encoding-required",
        "/paths/~1parts/post/requestBody/content/multipart~1mixed",
    );
    document["paths"]["/parts"]["post"]["requestBody"]["content"]["multipart/mixed"] = json!({
        "schema":{"type":"array","items":{"type":"string"}},
        "itemEncoding":{"contentType":"application/json","explode":false}
    });
    let contract = workspace(&[("api.json", document)]);
    let protocol = plan(&contract, &selected(&contract), normative_capabilities());
    assert_diagnostic(
        &protocol,
        "http-version-field",
        "/paths/~1parts/post/requestBody/content/multipart~1mixed/itemEncoding",
    );
}
