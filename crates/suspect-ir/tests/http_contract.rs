use std::sync::Arc;

use serde_json::json;
use suspect_ir::contract::{
    Contract, OAuthFlowKind, ParameterLocation, ParameterStyle, ResponseStatus, SecuritySchemeKind,
};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn compile(value: &serde_json::Value) -> Contract {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.json");
    std::fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(dir.path()).build().unwrap());
    Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap()
}

#[test]
fn bodies_responses_and_content_parameters_keep_every_media_header_and_encoding() {
    let contract = compile(&json!({
        "openapi":"3.1.0","info":{"title":"Upload","version":"1"},
        "paths":{"/files":{"post":{
            "operationId":"upload",
            "externalDocs":{"url":"https://docs.example.test/uploads","description":"Upload guide"},
            "parameters":[{"name":"filter","in":"query","content":{"application/json":{"schema":{"type":"object"},"example":{"kind":"image"}}}}],
            "requestBody":{"required":true,"description":"Upload a file","content":{
                "multipart/form-data":{"schema":{"type":"object","properties":{"file":{"type":"string","format":"binary"}}},
                    "encoding":{"file":{"contentType":"image/png, image/jpeg","headers":{"X-Checksum":{"required":true,"schema":{"type":"string"}}},"style":"form","explode":false,"allowReserved":true}}},
                "application/octet-stream":{"schema":{"type":"string","format":"binary"}}
            }},
            "responses":{
                "201":{"description":"Created","headers":{"Location":{"schema":{"type":"string","format":"uri"}}},"links":{"next":{"operationId":"getFile","parameters":{"id":"$response.body#/id"},"server":{"url":"https://files.example.test"}}},"content":{
                    "application/json":{"schema":{"type":"object","properties":{"id":{"type":"integer"}}},"examples":{"created":{"summary":"A file","value":{"id":9007199254740993_u64}}}},
                    "text/plain":{"schema":{"type":"string"}}
                }},
                "4XX":{"description":"Rejected","content":{"application/problem+json":{"schema":{"type":"object"}}}},
                "default":{"description":"Other","headers":{"X-Trace":{"content":{"application/json":{"schema":{"type":"string"}}}}}}
            }
        }}}
    }));
    assert!(!contract.has_errors(), "{:?}", contract.diagnostics());
    let operation = contract.operations().next().unwrap();
    assert_eq!(
        operation.external_docs().unwrap().url(),
        Some("https://docs.example.test/uploads")
    );
    assert_eq!(
        operation.external_docs().unwrap().description(),
        Some("Upload guide")
    );
    let parameter = operation.parameters().remove(0);
    assert!(parameter.schema().is_none());
    assert!(parameter.effective_style().is_none());
    assert_eq!(parameter.content()[0].name(), "application/json");
    assert_eq!(
        parameter.content()[0].example(),
        Some(&json!({"kind":"image"}))
    );
    let body = operation.request_body().unwrap();
    assert_eq!(body.required(), Some(true));
    assert_eq!(body.description(), Some("Upload a file"));
    assert_eq!(body.content().len(), 2);
    let multipart = body
        .content()
        .into_iter()
        .find(|m| m.name() == "multipart/form-data")
        .unwrap();
    assert_eq!(
        multipart.schema().unwrap().id().pointer(),
        "/paths/~1files/post/requestBody/content/multipart~1form-data/schema"
    );
    let encoding = multipart.encoding().remove(0);
    assert_eq!(encoding.name(), "file");
    assert_eq!(encoding.content_type(), Some("image/png, image/jpeg"));
    assert_eq!(encoding.style(), Some(ParameterStyle::Form));
    assert_eq!(encoding.explode(), Some(false));
    assert_eq!(encoding.allow_reserved(), Some(true));
    assert_eq!(encoding.headers()[0].name(), "X-Checksum");
    assert_eq!(encoding.headers()[0].required(), Some(true));
    let responses = operation.responses();
    assert_eq!(responses.len(), 3);
    let created = responses
        .iter()
        .find(|r| r.status() == Some(ResponseStatus::Exact(201)))
        .unwrap();
    assert_eq!(created.content().len(), 2);
    assert_eq!(created.headers()[0].name(), "Location");
    assert_eq!(created.links()[0].operation_id(), Some("getFile"));
    assert_eq!(
        created.links()[0].parameters().unwrap()["id"],
        "$response.body#/id"
    );
    assert_eq!(
        created.links()[0].server().unwrap().url(),
        Some("https://files.example.test")
    );
    assert_eq!(
        created.headers()[0].effective_style(),
        Some(ParameterStyle::Simple)
    );
    let json = created
        .content()
        .into_iter()
        .find(|m| m.name() == "application/json")
        .unwrap();
    let example = json.examples().remove(0);
    assert_eq!(example.name(), "created");
    assert_eq!(example.summary(), Some("A file"));
    assert_eq!(
        example.value().unwrap()["id"].to_string(),
        "9007199254740993"
    );
    assert!(
        responses
            .iter()
            .any(|r| r.status() == Some(ResponseStatus::Range(4)))
    );
    let fallback = responses
        .iter()
        .find(|r| r.status() == Some(ResponseStatus::Default))
        .unwrap();
    assert_eq!(fallback.status_key(), "default");
    assert_eq!(
        fallback.headers()[0].content()[0].name(),
        "application/json"
    );
}

#[test]
fn operation_metadata_preserves_inheritance_empty_overrides_and_parameter_identity() {
    let contract = compile(&json!({
        "openapi":"3.1.0","info":{"title":"HTTP","version":"1"},
        "servers":[{"url":"https://api.example.test/{region}","variables":{"region":{"default":"us","enum":["us","eu"]}}}],
        "security":[{"key":[]}],
        "components":{"securitySchemes":{"key":{"type":"apiKey","in":"header","name":"X-Key"}}},
        "paths":{
            "/pets/{id}":{
                "parameters":[
                    {"name":"id","in":"path","required":true,"schema":{"type":"string"}},
                    {"name":"filter","in":"query","schema":{"type":"string"}}
                ],
                "get":{"operationId":"inherited","responses":{"200":{"description":"OK"}}},
                "post":{"operationId":"overridden","servers":[],"security":[],
                    "parameters":[{"name":"filter","in":"query","style":"deepObject","explode":true,"allowReserved":true,"schema":{"type":"object","additionalProperties":{"type":"string"}}}],
                    "responses":{"204":{"description":"Done"}}
                }
            }
        }
    }));
    assert_eq!(contract.operations().count(), 2);
    let inherited = contract
        .operations()
        .find(|o| o.operation_id() == Some("inherited"))
        .unwrap();
    assert_eq!(inherited.path_template(), Some("/pets/{id}"));
    assert!(inherited.declared_servers().is_none());
    assert!(inherited.declared_security().is_none());
    assert_eq!(
        inherited.effective_servers()[0].url(),
        Some("https://api.example.test/{region}")
    );
    assert_eq!(
        inherited.effective_servers()[0].variables()[0].1.default(),
        Some("us")
    );
    assert_eq!(
        inherited.effective_security()[0].requirements()[0].name(),
        "key"
    );
    assert!(
        inherited
            .server_source()
            .unwrap()
            .pointer()
            .ends_with("/servers")
    );
    assert_eq!(inherited.parameters().len(), 2);

    let overridden = contract
        .operations()
        .find(|o| o.operation_id() == Some("overridden"))
        .unwrap();
    assert!(overridden.declared_servers().unwrap().is_empty());
    assert!(overridden.declared_security().unwrap().is_empty());
    assert!(overridden.effective_security().is_empty());
    assert_eq!(overridden.effective_servers().len(), 1);
    assert_eq!(overridden.effective_servers()[0].url(), Some("/"));
    assert!(overridden.effective_servers()[0].is_default());
    let parameters = overridden.parameters();
    assert_eq!(parameters.len(), 2);
    let filter = parameters
        .iter()
        .find(|p| p.name() == Some("filter"))
        .unwrap();
    assert_eq!(filter.location(), Some(ParameterLocation::Query));
    assert_eq!(filter.style(), Some(ParameterStyle::DeepObject));
    assert_eq!(filter.explode(), Some(true));
    assert_eq!(filter.allow_reserved(), Some(true));
    assert!(filter.source().pointer().contains("/post/parameters/"));
    assert!(filter.schema().unwrap().raw().is_object());
    assert!(!contract.has_errors(), "{:?}", contract.diagnostics());
}

#[test]
fn invalid_http_semantics_are_source_linked_and_never_replaced_with_defaults() {
    let contract = compile(&json!({
        "openapi":"3.1.0","info":{"title":"Invalid HTTP","version":"1"},
        "servers":[{"url":"https://api.example.test/{region}","variables":{"region":{"default":"moon","enum":["us","eu"]}}}],
        "components":{"securitySchemes":{"oauth":{"type":"oauth2","flows":{"implicit":{"authorizationUrl":"https://auth.example.test","scopes":{},"codeChallengeMethod":"S256"}}}}},
        "paths":{"/pets/{id}":{"post":{
            "externalDocs":{"url":"https://docs.example.test","title":"Unsupported title"},
            "parameters":[
                {"name":"id","in":"path","required":false,"schema":{"type":"string"}},
                {"name":"filter","in":"query","style":"magic","explode":"yes","schema":{"type":"string"},"content":{"application/json":{"schema":{"type":"string"}}}},
                {"name":"filter","in":"query","schema":{"type":"string"}},
                {"name":"bad-style","in":"query","style":"magic","schema":{"type":"string"}},
                {"name":"bad-explode","in":"query","style":"form","explode":"yes","schema":{"type":"string"}}
            ],
            "requestBody":{"content":[]},
            "responses":{"20X":{"description":"Invalid status"},"200":{"description":42}},
            "security":[{"missing":[]}]
        }}}
    }));
    assert!(contract.has_errors());
    for pointer in [
        "/servers/0/variables/region/default",
        "/components/securitySchemes/oauth/flows/implicit/codeChallengeMethod",
        "/paths/~1pets~1{id}/post/externalDocs/title",
        "/paths/~1pets~1{id}/post/parameters/0/required",
        "/paths/~1pets~1{id}/post/parameters/1/style",
        "/paths/~1pets~1{id}/post/parameters/1/explode",
        "/paths/~1pets~1{id}/post/parameters/1",
        "/paths/~1pets~1{id}/post/parameters/2",
        "/paths/~1pets~1{id}/post/requestBody/content",
        "/paths/~1pets~1{id}/post/responses/20X",
        "/paths/~1pets~1{id}/post/responses/200/description",
        "/paths/~1pets~1{id}/post/security/0/missing",
    ] {
        let diagnostic = contract
            .diagnostics()
            .iter()
            .find(|d| d.source.pointer() == pointer)
            .unwrap_or_else(|| {
                panic!(
                    "missing diagnostic for {pointer}: {:?}",
                    contract.diagnostics()
                )
            });
        assert_eq!(
            diagnostic.at,
            contract.source_span(&diagnostic.source).unwrap()
        );
        assert!(!diagnostic.at.is_empty());
    }
    let operation = contract.operations().next().unwrap();
    assert!(
        operation
            .responses()
            .iter()
            .any(|r| r.status_key() == "20X" && r.status().is_none())
    );
    let filter = operation
        .parameters()
        .into_iter()
        .find(|p| p.raw().get("style").is_some())
        .unwrap();
    assert_eq!(filter.style(), None);
    assert_eq!(filter.effective_style(), None);
    assert_eq!(filter.explode(), None);
    assert_eq!(filter.effective_explode(), None);
    let bad_style = operation
        .parameters()
        .into_iter()
        .find(|p| p.name() == Some("bad-style"))
        .unwrap();
    assert_eq!(bad_style.effective_style(), None);
    let bad_explode = operation
        .parameters()
        .into_iter()
        .find(|p| p.name() == Some("bad-explode"))
        .unwrap();
    assert_eq!(bad_explode.effective_explode(), None);
}

#[test]
fn server_and_security_overrides_are_independent() {
    let contract = compile(&json!({
        "openapi":"3.1.0","info":{"title":"Settings","version":"1"},
        "servers":[{"url":"https://root.example.test"}],"security":[{"key":[]}],
        "components":{"securitySchemes":{"key":{"type":"apiKey","in":"header","name":"X-Key"}}},
        "paths":{"/settings":{"servers":[{"url":"https://path.example.test"}],
            "get":{"servers":[],"responses":{"200":{"description":"OK"}}},
            "post":{"security":[],"responses":{"200":{"description":"OK"}}},
            "put":{"security":[{}],"responses":{"200":{"description":"OK"}}}
        }}
    }));
    assert!(!contract.has_errors(), "{:?}", contract.diagnostics());
    let get = contract
        .operations()
        .find(|o| o.method() == suspect_ir::Method::Get)
        .unwrap();
    assert_eq!(get.declared_servers().unwrap().len(), 0);
    assert!(get.declared_security().is_none());
    assert!(get.effective_servers()[0].is_default());
    assert_eq!(get.effective_security()[0].requirements()[0].name(), "key");
    let post = contract
        .operations()
        .find(|o| o.method() == suspect_ir::Method::Post)
        .unwrap();
    assert!(post.declared_servers().is_none());
    assert_eq!(post.declared_security().unwrap().len(), 0);
    assert_eq!(
        post.effective_servers()[0].url(),
        Some("https://path.example.test")
    );
    assert!(post.effective_security().is_empty());
    let put = contract
        .operations()
        .find(|o| o.method() == suspect_ir::Method::Put)
        .unwrap();
    assert_eq!(put.declared_security().unwrap().len(), 1);
    assert_eq!(put.effective_security().len(), 1);
    assert!(put.effective_security()[0].is_anonymous());
}

#[test]
fn reused_outgoing_operation_ids_report_mount_ambiguity_but_callback_reuse_keeps_context() {
    let contract = compile(&json!({
        "openapi":"3.1.0","info":{"title":"Reuse","version":"1"},
        "paths":{
            "/a":{"$ref":"#/components/pathItems/Shared"},
            "/b":{"$ref":"#/components/pathItems/Shared"},
            "/subscribe":{"post":{"operationId":"subscribe","responses":{"200":{"description":"OK"}},"callbacks":{"first":{"$ref":"#/components/callbacks/Event"},"second":{"$ref":"#/components/callbacks/Event"}}}}
        },
        "components":{
            "pathItems":{"Shared":{"get":{"operationId":"shared","responses":{"200":{"description":"OK"}}}}},
            "callbacks":{"Event":{"https://callback.example.test":{"post":{"operationId":"event","responses":{"200":{"description":"OK"}}}}}}
        }
    }));
    let diagnostics = contract.diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].code, "AMBIGUOUS_OPERATION_MOUNT");
    assert!(diagnostics[0].source.pointer().starts_with("/paths/"));
    assert_eq!(contract.operations().count(), 3);
    let mounts: Vec<_> = contract
        .operations()
        .filter(|o| o.operation_id() == Some("shared"))
        .collect();
    assert_eq!(mounts[0].source(), mounts[1].source());
    assert_ne!(mounts[0].path_item_source(), mounts[1].path_item_source());
    let callbacks: Vec<_> = contract.callbacks().collect();
    assert_eq!(callbacks.len(), 2);
    assert_eq!(callbacks[0].source(), callbacks[1].source());
    assert_eq!(
        callbacks[0].callback_parent(),
        callbacks[1].callback_parent()
    );
    assert_ne!(callbacks[0].callback_name(), callbacks[1].callback_name());
}

#[test]
fn callbacks_and_webhooks_remain_separate_from_outgoing_client_operations() {
    let contract = compile(&json!({
        "openapi":"3.1.0","info":{"title":"Events","version":"1"},
        "servers":[{"url":"https://api.example.test"}],
        "paths":{"/subscribe":{"post":{
            "operationId":"subscribe","servers":[{"url":"https://subscriptions.example.test"}],
            "requestBody":{"content":{"application/json":{"schema":{"type":"object","properties":{"callback":{"type":"string","format":"uri"}}}}}},
            "callbacks":{"onEvent":{"$ref":"#/components/callbacks/Event"}},
            "responses":{"202":{"description":"Subscribed"}}
        }}},
        "webhooks":{"notification":{"post":{"operationId":"receiveNotification","requestBody":{"content":{"application/json":{"schema":{"type":"object"}}}},"responses":{"204":{"description":"Accepted"}}}}},
        "components":{"callbacks":{"Event":{"{$request.body#/callback}":{"post":{
            "operationId":"onEvent","requestBody":{"content":{"application/json":{"schema":{"type":"object","properties":{"event":{"type":"string"}}}}}},
            "responses":{"204":{"description":"Received"}}
        }}}}}
    }));
    assert!(!contract.has_errors(), "{:?}", contract.diagnostics());
    assert_eq!(contract.operations().count(), 1);
    assert_eq!(contract.webhooks().count(), 1);
    assert_eq!(contract.callbacks().count(), 1);
    let outgoing = contract.operations().next().unwrap();
    let webhook = contract.webhooks().next().unwrap();
    let callback = contract.callbacks().next().unwrap();
    assert_eq!(webhook.webhook_name(), Some("notification"));
    assert!(webhook.path_template().is_none());
    assert_eq!(
        callback.callback_expression(),
        Some("{$request.body#/callback}")
    );
    assert_eq!(callback.callback_name(), Some("onEvent"));
    assert_eq!(callback.callback_parent(), Some(outgoing.source()));
    assert!(callback.path_template().is_none());
    assert_eq!(
        callback.effective_servers()[0].url(),
        Some("https://api.example.test")
    );
    assert_eq!(
        callback.request_body().unwrap().content()[0]
            .schema()
            .unwrap()
            .raw()["properties"]["event"]["type"],
        "string"
    );
}

#[test]
fn referenced_http_objects_keep_declarations_targets_and_versioned_sibling_rules() {
    for (version, expected_description) in [
        ("3.0.3", "Target description"),
        ("3.1.0", "Local description"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let entry = dir.path().join("api.json");
        let external = dir.path().join("http.json");
        std::fs::write(
            &entry,
            serde_json::to_vec(&json!({
                "openapi":version,"info":{"title":"Refs","version":"1"},
                "servers":[{"url":"https://root.example.test"}],
                "paths":{"/mounted":{"$ref":"http.json#/Route","summary":"Local path summary"}}
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(&external, serde_json::to_vec(&json!({
            "Route":{"servers":[],"parameters":[{"$ref":"#/Filter","name":"ignored","description":"Local description"}],
                "get":{"operationId":"mounted","responses":{"200":{"$ref":"#/Success","description":"Local response"}}}},
            "Filter":{"name":"filter","in":"query","description":"Target description","schema":{"type":"string"}},
            "Success":{"description":"Success description","content":{"application/json":{"schema":{"type":"integer"}}}}
        })).unwrap()).unwrap();
        let workspace = Arc::new(WorkspaceBuilder::new().root(dir.path()).build().unwrap());
        let contract =
            Contract::from_workspace(&workspace, &Uri::from_path(&entry).unwrap()).unwrap();
        assert!(!contract.has_errors(), "{:?}", contract.diagnostics());
        assert_eq!(contract.documents().count(), 2);
        let operation = contract.operations().next().unwrap();
        assert_eq!(operation.path_template(), Some("/mounted"));
        assert_eq!(
            operation.source().document(),
            &Uri::from_path(&external).unwrap()
        );
        assert_eq!(operation.path_item_source().document(), contract.entry());
        assert_eq!(operation.effective_servers()[0].url(), Some("/"));
        assert_eq!(
            operation.server_source().unwrap().pointer(),
            "/Route/servers"
        );
        let parameter = operation.parameters().remove(0);
        assert_eq!(parameter.source().pointer(), "/Route/parameters/0");
        assert_eq!(parameter.resolved_source().unwrap().pointer(), "/Filter");
        assert_eq!(parameter.name(), Some("filter"));
        assert_eq!(parameter.description(), Some(expected_description));
        assert_eq!(parameter.schema().unwrap().id().pointer(), "/Filter/schema");
        let response = operation.responses().remove(0);
        assert_eq!(
            response.content()[0].schema().unwrap().id().pointer(),
            "/Success/content/application~1json/schema"
        );
        assert_eq!(
            response.description(),
            Some(if version == "3.0.3" {
                "Success description"
            } else {
                "Local response"
            })
        );
    }
}

#[test]
fn security_schemes_and_oauth_flows_preserve_alternatives_scopes_and_document_sources() {
    let contract = compile(&json!({
        "openapi":"3.1.0","info":{"title":"Auth","version":"1"},
        "security":[{"key":[],"oauth":["files:read"]},{}],
        "paths":{"/files":{"get":{"responses":{"200":{"description":"OK"}}}}},
        "components":{"securitySchemes":{
            "key":{"type":"apiKey","in":"header","name":"X-Key"},
            "bearer":{"type":"http","scheme":"bearer","bearerFormat":"JWT"},
            "oauth":{"type":"oauth2","description":"OAuth login","flows":{
                "authorizationCode":{"authorizationUrl":"https://auth.example.test/authorize","tokenUrl":"https://auth.example.test/token","refreshUrl":"https://auth.example.test/refresh","scopes":{"files:read":"Read files","files:write":"Write files"}},
                "clientCredentials":{"tokenUrl":"https://auth.example.test/token","scopes":{}}
            }},
            "oidc":{"type":"openIdConnect","openIdConnectUrl":"https://auth.example.test/.well-known/openid-configuration"},
            "mtls":{"type":"mutualTLS"}
        }}
    }));
    assert!(!contract.has_errors(), "{:?}", contract.diagnostics());
    assert_eq!(contract.security_schemes().len(), 5);
    let operation = contract.operations().next().unwrap();
    let alternatives = operation.effective_security();
    assert_eq!(alternatives.len(), 2);
    assert_eq!(alternatives[0].requirements().len(), 2);
    assert!(alternatives[1].is_anonymous());
    let requirements = alternatives[0].requirements();
    let key = requirements
        .iter()
        .find(|r| r.name() == "key")
        .unwrap()
        .scheme()
        .unwrap();
    assert_eq!(key.kind(), Some(SecuritySchemeKind::ApiKey));
    assert_eq!(key.parameter_name(), Some("X-Key"));
    assert_eq!(key.parameter_location(), Some(ParameterLocation::Header));
    let oauth_use = requirements.iter().find(|r| r.name() == "oauth").unwrap();
    assert_eq!(oauth_use.scopes(), Some(vec!["files:read"]));
    let oauth = oauth_use.scheme().unwrap();
    assert_eq!(oauth.name(), "oauth");
    assert_eq!(oauth.kind(), Some(SecuritySchemeKind::OAuth2));
    assert_eq!(
        oauth.source().pointer(),
        "/components/securitySchemes/oauth"
    );
    assert_eq!(oauth.flows().len(), 2);
    let authorization = oauth
        .flows()
        .into_iter()
        .find(|f| f.kind() == Some(OAuthFlowKind::AuthorizationCode))
        .unwrap();
    assert_eq!(
        authorization.authorization_url(),
        Some("https://auth.example.test/authorize")
    );
    assert_eq!(
        authorization.token_url(),
        Some("https://auth.example.test/token")
    );
    assert_eq!(
        authorization.refresh_url(),
        Some("https://auth.example.test/refresh")
    );
    assert_eq!(authorization.scopes().unwrap()["files:read"], "Read files");
    assert_eq!(
        authorization.source().pointer(),
        "/components/securitySchemes/oauth/flows/authorizationCode"
    );
}

#[test]
fn reference_annotations_use_the_declaring_document_version() {
    let dir = tempfile::tempdir().unwrap();
    let entry = dir.path().join("api.json");
    std::fs::write(
        &entry,
        serde_json::to_vec(&json!({
            "openapi":"3.1.0","info":{"title":"Refs","version":"1"},
            "paths":{"/value":{"$ref":"legacy.json#/paths/~1value"}}
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(dir.path().join("legacy.json"), serde_json::to_vec(&json!({
        "openapi":"3.0.3","info":{"title":"Legacy","version":"1"},
        "paths":{"/value":{"get":{"parameters":[{"$ref":"#/components/parameters/Filter","description":"Ignored 3.0 sibling"}],"responses":{"200":{"description":"OK"}}}}},
        "components":{"parameters":{"Filter":{"name":"filter","in":"query","description":"Target description","schema":{"type":"string"}}}}
    })).unwrap()).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(dir.path()).build().unwrap());
    let contract = Contract::from_workspace(&workspace, &Uri::from_path(&entry).unwrap()).unwrap();
    assert!(!contract.has_errors(), "{:?}", contract.diagnostics());
    assert_eq!(
        contract.operations().next().unwrap().parameters()[0].description(),
        Some("Target description")
    );
}

#[test]
fn external_security_names_follow_their_own_documents_reference_closure() {
    let dir = tempfile::tempdir().unwrap();
    let entry = dir.path().join("api.json");
    std::fs::write(
        &entry,
        serde_json::to_vec(&json!({
            "openapi":"3.1.0","info":{"title":"Entry","version":"1"},
            "paths":{"/remote":{"$ref":"remote.json#/paths/~1remote"}},
            "components":{"securitySchemes":{"key":{"type":"http","scheme":"bearer"}}}
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(dir.path().join("remote.json"), serde_json::to_vec(&json!({
        "openapi":"3.1.0","info":{"title":"Remote","version":"1"},
        "paths":{"/remote":{"get":{"security":[{"key":[]}],"responses":{"200":{"description":"OK"}}}}},
        "components":{"securitySchemes":{"key":{"$ref":"auth.json#/Key"}}}
    })).unwrap()).unwrap();
    std::fs::write(
        dir.path().join("auth.json"),
        serde_json::to_vec(&json!({
            "Key":{"type":"apiKey","in":"header","name":"X-Remote-Key"}
        }))
        .unwrap(),
    )
    .unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().root(dir.path()).build().unwrap());
    let contract = Contract::from_workspace(&workspace, &Uri::from_path(&entry).unwrap()).unwrap();
    assert!(!contract.has_errors(), "{:?}", contract.diagnostics());
    assert_eq!(contract.documents().count(), 3);
    let scheme = contract.operations().next().unwrap().effective_security()[0].requirements()[0]
        .scheme()
        .unwrap();
    assert_eq!(scheme.kind(), Some(SecuritySchemeKind::ApiKey));
    assert_eq!(scheme.parameter_name(), Some("X-Remote-Key"));
    assert_eq!(
        scheme.resolved_source().unwrap().document(),
        &Uri::from_path(&dir.path().join("auth.json")).unwrap()
    );
}

#[test]
fn undefined_path_merges_and_reference_cycles_do_not_hide_indexed_additional_methods() {
    let contract = compile(&json!({
        "openapi":"3.2.0","info":{"title":"Unsupported","version":"1"},
        "paths":{
            "/conflict":{"$ref":"#/components/pathItems/Common","summary":"Local summary"},
            "/cycle":{"$ref":"#/components/pathItems/Cycle"},
            "/new-methods":{"query":{"responses":{"200":{"description":"OK"}}},"additionalOperations":{"COPY":{"responses":{"200":{"description":"OK"}}}}},
            "/recursive":{"get":{"responses":{"200":{"description":"OK"}},"callbacks":{"recursive":{"$ref":"#/components/callbacks/Recursive"}}}}
        },
        "components":{
            "pathItems":{"Common":{"summary":"Referenced summary","get":{"responses":{"200":{"description":"OK"}}}},"Cycle":{"$ref":"#/components/pathItems/Cycle"}},
            "callbacks":{"Recursive":{"https://callback.example.test":{"post":{"responses":{"200":{"description":"OK"}},"callbacks":{"again":{"$ref":"#/components/callbacks/Recursive"}}}}}}
        }
    }));
    assert!(contract.has_errors());
    for (code, pointer) in [
        ("AMBIGUOUS_PATH_ITEM_REFERENCE", "/paths/~1conflict/summary"),
        ("HTTP_REFERENCE_CYCLE", "/components/pathItems/Cycle/$ref"),
        (
            "RECURSIVE_CALLBACK",
            "/components/callbacks/Recursive/https:~1~1callback.example.test/post/callbacks/again",
        ),
    ] {
        assert!(
            contract
                .diagnostics()
                .iter()
                .any(|d| d.code == code && d.source.pointer() == pointer),
            "missing {code} at {pointer}: {:?}",
            contract.diagnostics()
        );
    }
    assert_eq!(contract.operations().count(), 4);
    let methods = contract
        .operations()
        .filter(|op| op.path_template() == Some("/new-methods"))
        .map(|op| op.method().as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(methods, std::collections::BTreeSet::from(["COPY", "QUERY"]));
    assert_eq!(contract.callbacks().count(), 1);
}

#[test]
#[ignore = "set SUSPECT_CONTRACT_SPEC and SUSPECT_CONTRACT_ORACLE to tracked OpenRouter YAML and independent JSON oracle"]
fn real_openrouter_preserves_all_103_http_operations_and_transport_schemas() {
    let path =
        std::path::PathBuf::from(std::env::var_os("SUSPECT_CONTRACT_SPEC").expect("corpus path"));
    let oracle = std::env::var_os("SUSPECT_CONTRACT_ORACLE").expect("independent JSON oracle");
    let expected: serde_json::Value =
        serde_json::from_slice(&std::fs::read(oracle).unwrap()).unwrap();
    let uri = Uri::from_path(&path).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    let started = std::time::Instant::now();
    let contract = Contract::from_workspace(&workspace, &uri).unwrap();
    assert!(!contract.has_errors(), "{:?}", contract.diagnostics());
    assert_eq!(contract.operations().count(), 103);
    let mut parameters = 0;
    let mut bodies = 0;
    let mut responses = 0;
    for (path, item) in expected["paths"].as_object().unwrap() {
        for method in [
            "get", "put", "post", "delete", "options", "head", "patch", "trace",
        ] {
            let Some(raw) = item.get(method) else {
                continue;
            };
            let operation = contract
                .operations()
                .find(|o| {
                    o.path_template() == Some(path)
                        && o.method().as_str().eq_ignore_ascii_case(method)
                })
                .unwrap();
            assert_eq!(operation.raw(), raw);
            assert_eq!(
                operation.operation_id(),
                raw.get("operationId").and_then(serde_json::Value::as_str)
            );
            assert_eq!(operation.source().document(), &uri);
            assert_eq!(
                operation.effective_servers()[0].raw(),
                Some(&expected["servers"][0])
            );
            let expected_security = raw
                .get("security")
                .unwrap_or(&expected["security"])
                .as_array()
                .unwrap();
            assert_eq!(
                operation
                    .effective_security()
                    .iter()
                    .map(|r| r.raw())
                    .collect::<Vec<_>>(),
                expected_security.iter().collect::<Vec<_>>()
            );
            let mut expected_parameters = item
                .get("parameters")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default();
            for parameter in raw
                .get("parameters")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
            {
                expected_parameters
                    .retain(|p| !(p["name"] == parameter["name"] && p["in"] == parameter["in"]));
                expected_parameters.push(parameter.clone());
            }
            assert_eq!(operation.parameters().len(), expected_parameters.len());
            for parameter in operation.parameters() {
                parameters += 1;
                let expected = expected_parameters
                    .iter()
                    .find(|p| {
                        p.get("name").and_then(serde_json::Value::as_str) == parameter.name()
                            && p["in"] == parameter.raw()["in"]
                    })
                    .unwrap();
                assert_eq!(parameter.raw(), expected);
                assert_eq!(parameter.schema().map(|s| s.raw()), expected.get("schema"));
                for media in parameter.content() {
                    assert_eq!(media.raw(), &expected["content"][media.name()]);
                    assert_eq!(media.schema().map(|s| s.raw()), media.raw().get("schema"));
                }
            }
            assert_eq!(
                operation.request_body().is_some(),
                raw.get("requestBody").is_some()
            );
            if let Some(body) = operation.request_body() {
                bodies += 1;
                assert_eq!(body.raw(), &raw["requestBody"]);
                assert_eq!(
                    body.content().len(),
                    raw["requestBody"]["content"].as_object().unwrap().len()
                );
                for media in body.content() {
                    assert_eq!(media.raw(), &raw["requestBody"]["content"][media.name()]);
                    assert_eq!(media.schema().map(|s| s.raw()), media.raw().get("schema"));
                }
            }
            assert_eq!(
                operation.responses().len(),
                raw["responses"].as_object().unwrap().len()
            );
            for response in operation.responses() {
                responses += 1;
                assert!(response.status().is_some());
                let expected = &raw["responses"][response.status_key()];
                assert_eq!(response.raw(), expected);
                assert_eq!(
                    response.content().len(),
                    expected
                        .get("content")
                        .and_then(serde_json::Value::as_object)
                        .map_or(0, |m| m.len())
                );
                for media in response.content() {
                    assert_eq!(media.raw(), &expected["content"][media.name()]);
                    assert_eq!(media.schema().map(|s| s.raw()), media.raw().get("schema"));
                }
                assert_eq!(
                    response.headers().len(),
                    expected
                        .get("headers")
                        .and_then(serde_json::Value::as_object)
                        .map_or(0, |m| m.len())
                );
                for header in response.headers() {
                    assert_eq!(header.raw(), &expected["headers"][header.name()]);
                    assert_eq!(header.schema().map(|s| s.raw()), header.raw().get("schema"));
                }
            }
        }
    }
    assert_eq!((parameters, bodies, responses), (191, 37, 590));
    eprintln!(
        "HTTP Contract {:?}: 103 operations, {parameters} parameters, {bodies} request bodies, {responses} responses, {} diagnostics",
        started.elapsed(),
        contract.diagnostics().len()
    );
}
