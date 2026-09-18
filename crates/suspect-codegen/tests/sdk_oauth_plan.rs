//! Generation-time OAuth lifecycle planning binds through the public, admitted
//! protocol plan, and the `sdk_defaults` OAuth section validates like the rest
//! of the policy configuration.
use std::sync::Arc;

use serde_json::json;
use suspect_codegen::{http_protocol, rust_http, sdk_defaults};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

fn contract_with_document(document: serde_json::Value) -> Arc<Contract> {
    let entry = Uri::parse("https://source.oauth.test/openapi.json").unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            serde_json::to_vec(&document).unwrap(),
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

fn plan_oauth(
    contract: &Arc<Contract>,
    defaults: Option<&sdk_defaults::SdkDefaults>,
) -> Result<http_protocol::OAuthPlan, Vec<suspect_codegen::sdk_defaults::HttpDiagnostic>> {
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let protocol = http_protocol::plan(contract, &selected, rust_http::native_capabilities_v3())
        .into_result()
        .unwrap();
    http_protocol::plan_oauth(contract, &protocol, defaults)
}

/// One oauth2 scheme with an authorization-code and a client-credentials flow.
/// `serde_json` map iteration is sorted, so flows arrive in key order.
fn oauth_document() -> serde_json::Value {
    json!({
        "openapi":"3.1.0", "info":{"title":"OAuth","version":"1"},
        "servers":[{"url":"https://api.oauth.test/v1"}],
        "security":[{"userOAuth":["read"]}],
        "components":{"securitySchemes":{"userOAuth":{
            "type":"oauth2",
            "flows":{
                "authorizationCode":{
                    "authorizationUrl":"https://auth.oauth.test/authorize",
                    "tokenUrl":"https://auth.oauth.test/token",
                    "refreshUrl":"https://auth.oauth.test/token-refresh",
                    "scopes":{"read":"Read access","write":"Write access"}
                },
                "clientCredentials":{
                    "tokenUrl":"https://auth.oauth.test/token",
                    "scopes":{"read":"Read access"}
                }
            }
        }}},
        "paths":{"/widgets":{"get":{"operationId":"listWidgets","responses":{"200":{"description":"Ok"}}}}}
    })
}

#[test]
fn declared_oauth2_flows_compile_with_urls_and_scopes() {
    let contract = contract_with_document(oauth_document());
    let plan = plan_oauth(&contract, None).unwrap();
    assert_eq!(plan.mode, http_protocol::OAuthMode::Auto);
    assert_eq!(plan.schemes.len(), 1);
    let scheme = &plan.schemes[0];
    assert_eq!(scheme.name, "userOAuth");
    assert_eq!(scheme.kind, http_protocol::OAuthSchemeKind::OAuth2);
    assert_eq!(scheme.flows.len(), 2);
    let code = &scheme.flows[0];
    assert_eq!(
        code.kind,
        http_protocol::OAuthFlowDescriptorKind::AuthorizationCode
    );
    assert_eq!(
        code.authorization_url.as_deref(),
        Some("https://auth.oauth.test/authorize")
    );
    assert_eq!(
        code.token_url.as_deref(),
        Some("https://auth.oauth.test/token")
    );
    assert_eq!(
        code.scopes.get("read").map(String::as_str),
        Some("Read access")
    );
    assert_eq!(
        code.scopes.get("write").map(String::as_str),
        Some("Write access")
    );
    assert!(!code.deprecated_flow);
    let service = &scheme.flows[1];
    assert_eq!(
        service.kind,
        http_protocol::OAuthFlowDescriptorKind::ClientCredentials
    );
    assert_eq!(
        service.token_url.as_deref(),
        Some("https://auth.oauth.test/token")
    );
    // Without configuration nothing is invented and the documented defaults hold.
    assert_eq!(scheme.client_id_env, None);
    assert_eq!(scheme.client_secret_env, None);
    assert_eq!(scheme.refresh_skew_seconds, 30);
    assert_eq!(scheme.revocation_endpoint, None);
    assert_eq!(scheme.introspection_endpoint, None);
    assert_eq!(scheme.discovery, None);
    assert_eq!(scheme.storage, http_protocol::OAuthStorage::Memory);
    assert_eq!(scheme.refresh, http_protocol::OAuthRefresh::OnDemand);
    // Token-endpoint flows without a configured secret are public clients.
    assert_eq!(code.client_auth, http_protocol::OAuthClientAuth::None);
}

#[test]
fn declared_refresh_urls_are_preserved() {
    let contract = contract_with_document(oauth_document());
    let plan = plan_oauth(&contract, None).unwrap();
    let flows = &plan.schemes[0].flows;
    assert_eq!(
        flows[0].refresh_url.as_deref(),
        Some("https://auth.oauth.test/token-refresh")
    );
    assert_eq!(flows[1].refresh_url, None);
}

#[test]
fn configuration_supplements_the_declaration() {
    let contract = contract_with_document(oauth_document());
    let defaults: sdk_defaults::SdkDefaults = serde_json::from_value(json!({
        "version":"v1",
        "oauth":{
            "mode":"auto","storage":"memory","refresh":"on-demand",
            "schemes":{"userOAuth":{
                "client_id_env":"EXAMPLE_CLIENT_ID",
                "client_secret_env":"EXAMPLE_CLIENT_SECRET",
                "refresh_skew_seconds":45,
                "revocation_endpoint":"https://auth.oauth.test/revoke",
                "introspection_endpoint":"https://auth.oauth.test/introspect",
                "discovery_url":"https://auth.oauth.test/.well-known/oauth-authorization-server"
            }}
        }
    }))
    .unwrap();
    let plan = plan_oauth(&contract, Some(&defaults)).unwrap();
    let scheme = &plan.schemes[0];
    assert_eq!(scheme.client_id_env.as_deref(), Some("EXAMPLE_CLIENT_ID"));
    assert_eq!(
        scheme.client_secret_env.as_deref(),
        Some("EXAMPLE_CLIENT_SECRET")
    );
    assert_eq!(scheme.refresh_skew_seconds, 45);
    assert_eq!(
        scheme.revocation_endpoint.as_deref(),
        Some("https://auth.oauth.test/revoke")
    );
    assert_eq!(
        scheme.introspection_endpoint.as_deref(),
        Some("https://auth.oauth.test/introspect")
    );
    // The declaration has no oauth2MetadataUrl, so the config supplies discovery.
    assert_eq!(
        scheme.discovery.as_deref(),
        Some("https://auth.oauth.test/.well-known/oauth-authorization-server")
    );
    // Token-endpoint flows derive the standard confidential default.
    assert_eq!(
        scheme.flows[0].client_auth,
        http_protocol::OAuthClientAuth::ClientSecretBasic
    );
    assert_eq!(
        scheme.flows[1].client_auth,
        http_protocol::OAuthClientAuth::ClientSecretBasic
    );
}

#[test]
fn off_shorthand_yields_an_empty_plan() {
    let contract = contract_with_document(oauth_document());
    let defaults: sdk_defaults::SdkDefaults =
        serde_json::from_value(json!({"version":"v1","oauth":"off"})).unwrap();
    let plan = plan_oauth(&contract, Some(&defaults)).unwrap();
    assert_eq!(plan.mode, http_protocol::OAuthMode::Off);
    assert!(plan.schemes.is_empty());

    // Off disables planning entirely, including configuration diagnostics.
    let defaults: sdk_defaults::SdkDefaults = serde_json::from_value(json!({
        "version":"v1","oauth":{"mode":"off","schemes":{"unused":{"client_id_env":"UNUSED_ID"}}}
    }))
    .unwrap();
    let plan = plan_oauth(&contract, Some(&defaults)).unwrap();
    assert!(plan.schemes.is_empty());
}

#[test]
fn open_id_connect_schemes_compile_with_their_discovery_url() {
    let document = json!({
        "openapi":"3.1.0", "info":{"title":"OIDC","version":"1"},
        "servers":[{"url":"https://api.oauth.test/v1"}],
        "security":[{"identity":[]}],
        "components":{"securitySchemes":{"identity":{
            "type":"openIdConnect",
            "openIdConnectUrl":"https://idp.oauth.test/.well-known/openid-configuration"
        }}},
        "paths":{"/widgets":{"get":{"operationId":"listWidgets","responses":{"200":{"description":"Ok"}}}}}
    });
    let contract = contract_with_document(document);
    let plan = plan_oauth(&contract, None).unwrap();
    assert_eq!(plan.schemes.len(), 1);
    let scheme = &plan.schemes[0];
    assert_eq!(scheme.kind, http_protocol::OAuthSchemeKind::OpenIdConnect);
    assert_eq!(
        scheme.discovery.as_deref(),
        Some("https://idp.oauth.test/.well-known/openid-configuration")
    );
    assert!(
        scheme.flows.is_empty(),
        "discovery defines flows at runtime"
    );
}

#[test]
fn a_configured_scheme_that_binds_no_used_source_scheme_is_an_error() {
    let contract = contract_with_document(oauth_document());
    let defaults: sdk_defaults::SdkDefaults = serde_json::from_value(json!({
        "version":"v1",
        "oauth":{"schemes":{"other":{"client_id_env":"OTHER_ID"}}}
    }))
    .unwrap();
    let error = plan_oauth(&contract, Some(&defaults)).unwrap_err();
    assert!(
        error.iter().any(|d| d.code == "sdk-oauth-config"),
        "{error:?}"
    );
}

#[test]
fn a_client_secret_on_a_public_implicit_only_declaration_is_an_error() {
    let document = json!({
        "openapi":"3.1.0", "info":{"title":"Legacy","version":"1"},
        "servers":[{"url":"https://api.oauth.test/v1"}],
        "security":[{"legacyOAuth":["read"]}],
        "components":{"securitySchemes":{"legacyOAuth":{
            "type":"oauth2",
            "flows":{"implicit":{
                "authorizationUrl":"https://auth.oauth.test/authorize",
                "scopes":{"read":"Read access"}
            }}
        }}},
        "paths":{"/widgets":{"get":{"operationId":"listWidgets","responses":{"200":{"description":"Ok"}}}}}
    });
    let contract = contract_with_document(document);
    let defaults: sdk_defaults::SdkDefaults = serde_json::from_value(json!({
        "version":"v1",
        "oauth":{"schemes":{"legacyOAuth":{"client_secret_env":"LEGACY_SECRET"}}}
    }))
    .unwrap();
    let error = plan_oauth(&contract, Some(&defaults)).unwrap_err();
    assert!(
        error.iter().any(|d| d.code == "sdk-oauth-config"),
        "{error:?}"
    );
    // Without the impossible secret the public descriptor still compiles.
    let defaults: sdk_defaults::SdkDefaults = serde_json::from_value(json!({
        "version":"v1",
        "oauth":{"schemes":{"legacyOAuth":{"client_id_env":"LEGACY_ID"}}}
    }))
    .unwrap();
    let plan = plan_oauth(&contract, Some(&defaults)).unwrap();
    assert_eq!(plan.schemes[0].client_secret_env, None);
}

#[test]
fn implicit_and_password_flows_are_represented_but_flagged_deprecated() {
    let document = json!({
        "openapi":"3.1.0", "info":{"title":"Legacy flows","version":"1"},
        "servers":[{"url":"https://api.oauth.test/v1"}],
        "security":[{"legacyOAuth":["read"]}],
        "components":{"securitySchemes":{"legacyOAuth":{
            "type":"oauth2",
            "flows":{
                "implicit":{
                    "authorizationUrl":"https://auth.oauth.test/authorize",
                    "scopes":{"read":"Read access"}
                },
                "password":{
                    "tokenUrl":"https://auth.oauth.test/token",
                    "scopes":{"read":"Read access"}
                }
            }
        }}},
        "paths":{"/widgets":{"get":{"operationId":"listWidgets","responses":{"200":{"description":"Ok"}}}}}
    });
    let contract = contract_with_document(document);
    let plan = plan_oauth(&contract, None).unwrap();
    let scheme = &plan.schemes[0];
    assert_eq!(scheme.flows.len(), 2);
    // serde_json map iteration is sorted, so the keys arrive alphabetically.
    let implicit = &scheme.flows[0];
    assert_eq!(
        implicit.kind,
        http_protocol::OAuthFlowDescriptorKind::Implicit
    );
    assert!(implicit.deprecated_flow);
    assert_eq!(
        implicit.authorization_url.as_deref(),
        Some("https://auth.oauth.test/authorize")
    );
    assert_eq!(implicit.token_url, None);
    assert_eq!(implicit.client_auth, http_protocol::OAuthClientAuth::None);
    let password = &scheme.flows[1];
    assert_eq!(
        password.kind,
        http_protocol::OAuthFlowDescriptorKind::Password
    );
    assert!(password.deprecated_flow);
    assert_eq!(
        password.token_url.as_deref(),
        Some("https://auth.oauth.test/token")
    );
}

#[test]
fn two_schemes_are_independent_entries() {
    let document = json!({
        "openapi":"3.1.0", "info":{"title":"Two schemes","version":"1"},
        "servers":[{"url":"https://api.oauth.test/v1"}],
        "components":{"securitySchemes":{
            "userOAuth":{
                "type":"oauth2",
                "flows":{"authorizationCode":{
                    "authorizationUrl":"https://auth.oauth.test/authorize",
                    "tokenUrl":"https://auth.oauth.test/token",
                    "scopes":{"read":"Read access"}
                }}
            },
            "serviceOAuth":{
                "type":"oauth2",
                "flows":{"clientCredentials":{
                    "tokenUrl":"https://auth.oauth.test/token",
                    "scopes":{"read":"Read access"}
                }}
            }
        }},
        "paths":{
            "/widgets":{"get":{"operationId":"listWidgets","security":[{"userOAuth":["read"]}],"responses":{"200":{"description":"Ok"}}}},
            "/jobs":{"post":{"operationId":"submitJob","security":[{"serviceOAuth":[]}],"responses":{"200":{"description":"Ok"}}}}
        }
    });
    let contract = contract_with_document(document);
    let defaults: sdk_defaults::SdkDefaults = serde_json::from_value(json!({
        "version":"v1",
        "oauth":{"schemes":{"serviceOAuth":{"client_id_env":"SERVICE_ID"}}}
    }))
    .unwrap();
    let plan = plan_oauth(&contract, Some(&defaults)).unwrap();
    assert_eq!(plan.schemes.len(), 2);
    // Entries are sorted by scheme name and configured independently.
    assert_eq!(plan.schemes[0].name, "serviceOAuth");
    assert_eq!(plan.schemes[0].client_id_env.as_deref(), Some("SERVICE_ID"));
    assert_eq!(plan.schemes[0].flows.len(), 1);
    assert_eq!(
        plan.schemes[0].flows[0].kind,
        http_protocol::OAuthFlowDescriptorKind::ClientCredentials
    );
    assert_eq!(plan.schemes[1].name, "userOAuth");
    assert_eq!(plan.schemes[1].client_id_env, None);
    assert_eq!(plan.schemes[1].flows.len(), 1);
}

#[test]
fn relative_declared_flow_urls_are_flow_errors() {
    let document = json!({
        "openapi":"3.1.0", "info":{"title":"Relative","version":"1"},
        "servers":[{"url":"https://api.oauth.test/v1"}],
        "security":[{"userOAuth":["read"]}],
        "components":{"securitySchemes":{"userOAuth":{
            "type":"oauth2",
            "flows":{"authorizationCode":{
                "authorizationUrl":"https://auth.oauth.test/authorize",
                "tokenUrl":"/oauth/token",
                "scopes":{"read":"Read access"}
            }}
        }}},
        "paths":{"/widgets":{"get":{"operationId":"listWidgets","responses":{"200":{"description":"Ok"}}}}}
    });
    let contract = contract_with_document(document);
    let error = plan_oauth(&contract, None).unwrap_err();
    assert!(
        error.iter().any(|d| d.code == "sdk-oauth-flow"),
        "{error:?}"
    );
}

#[test]
fn oauth_defaults_shorthand_and_expanded_normalize_identically() {
    let shorthand: sdk_defaults::SdkDefaults =
        serde_json::from_value(json!({"version":"v1","oauth":"off"})).unwrap();
    let expanded: sdk_defaults::SdkDefaults =
        serde_json::from_value(json!({"version":"v1","oauth":{"mode":"off"}})).unwrap();
    assert_eq!(shorthand, expanded);
    let auto: sdk_defaults::SdkDefaults =
        serde_json::from_value(json!({"version":"v1","oauth":"auto"})).unwrap();
    assert_eq!(auto, sdk_defaults::SdkDefaults::v1());
}

#[test]
fn oauth_defaults_reject_unknown_fields_and_out_of_policy_values() {
    // Unknown key on the section.
    assert!(
        serde_json::from_value::<sdk_defaults::SdkDefaults>(json!({
            "version":"v1","oauth":{"mode":"auto","bogus":true}
        }))
        .is_err()
    );
    // Unknown key on a scheme configuration.
    assert!(
        serde_json::from_value::<sdk_defaults::SdkDefaults>(json!({
            "version":"v1","oauth":{"schemes":{"x":{"token_endpoint":"https://auth.test/token"}}}
        }))
        .is_err()
    );
    // Environment variable names follow the shared portability rule.
    for variable in ["1BAD", "not-portable", "HAS SPACE", ""] {
        assert!(
            serde_json::from_value::<sdk_defaults::SdkDefaults>(json!({
                "version":"v1","oauth":{"schemes":{"x":{"client_id_env":variable}}}
            }))
            .is_err(),
            "{variable:?} must be refused"
        );
        assert!(
            serde_json::from_value::<sdk_defaults::SdkDefaults>(json!({
                "version":"v1","oauth":{"schemes":{"x":{"client_secret_env":variable}}}
            }))
            .is_err(),
            "{variable:?} must be refused"
        );
    }
    // Skew outside 0..=3600 is refused; the bounds are admitted.
    assert!(
        serde_json::from_value::<sdk_defaults::SdkDefaults>(json!({
            "version":"v1","oauth":{"schemes":{"x":{"refresh_skew_seconds":3601}}}
        }))
        .is_err()
    );
    // Plain http to a public host and relative endpoints are refused.
    assert!(
        serde_json::from_value::<sdk_defaults::SdkDefaults>(json!({
            "version":"v1","oauth":{"schemes":{"x":{"revocation_endpoint":"http://auth.test/revoke"}}}
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<sdk_defaults::SdkDefaults>(json!({
            "version":"v1","oauth":{"schemes":{"x":{"introspection_endpoint":"/introspect"}}}
        }))
        .is_err()
    );
    // Loopback http, https, and the skew bounds are all admitted.
    let ok: sdk_defaults::SdkDefaults = serde_json::from_value(json!({
        "version":"v1","oauth":{"schemes":{"x":{
            "revocation_endpoint":"http://127.0.0.1:9000/revoke",
            "introspection_endpoint":"http://localhost:9000/introspect",
            "discovery_url":"https://auth.test/.well-known/oauth-authorization-server",
            "refresh_skew_seconds":0
        }}}
    }))
    .unwrap();
    let scheme = ok.oauth.schemes.get("x").unwrap();
    assert_eq!(
        scheme.revocation_endpoint.as_deref(),
        Some("http://127.0.0.1:9000/revoke")
    );
    assert_eq!(scheme.refresh_skew_seconds, Some(0));
}

#[test]
fn sdk_defaults_descriptor_round_trips_the_oauth_section() {
    let defaults: sdk_defaults::SdkDefaults =
        serde_json::from_value(json!({"version":"v1","oauth":{"mode":"off"}})).unwrap();
    let descriptor = defaults.semantic_descriptor();
    let encoded = serde_json::to_value(&descriptor).unwrap();
    assert_eq!(encoded["oauth"]["mode"], json!("off"));
    assert_eq!(encoded["oauth"]["storage"], json!("memory"));
    assert_eq!(encoded["oauth"]["refresh"], json!("on-demand"));
    let decoded: sdk_defaults::SdkDefaultsDescriptor = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, descriptor);

    // A default section is omitted from the encoding entirely, so captures
    // recorded before the section existed still deserialize unchanged.
    let plain = sdk_defaults::SdkDefaults::v1().semantic_descriptor();
    let encoded = serde_json::to_value(&plain).unwrap();
    assert!(encoded.get("oauth").is_none(), "{encoded}");
    let recorded_before: sdk_defaults::SdkDefaultsDescriptor = serde_json::from_value(json!({
        "version":"v1","env_prefix":"OPENROUTER",
        "pagination":{"mode":"auto","page_size":25}
    }))
    .unwrap();
    assert_eq!(
        recorded_before.oauth,
        sdk_defaults::OAuthDefaults::default()
    );
}
