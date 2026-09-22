use std::{path::Path, sync::Arc};
use suspect_oas::Session;
use suspect_ref::WorkspaceBuilder;
use suspect_validate::validate_entry;

fn validate(dir: &Path, name: &str, source: &str) -> Vec<suspect_validate::Diagnostic> {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(name), source).unwrap();
    let session = Session::new(Arc::new(WorkspaceBuilder::new().root(dir).build().unwrap()));
    validate_entry(&session, name).unwrap()
}
fn directory(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("suspect-http-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn malformed_http_declarations_are_located_without_filtering_or_defaults() {
    let dir = directory("malformed");
    let source = r#"{"openapi":"3.1.0","info":{"title":"x","version":"1"},"security":[{"oauth":["ok",42]}],"paths":{"/x":{"post":{"parameters":[{"name":"id","in":"query","required":"yes","explode":0,"allowReserved":null,"allowEmptyValue":[],"deprecated":{}}],"requestBody":{"required":"false","content":{}},"responses":{"200":{"description":"ok","content":[],"headers":{"x":42},"links":false},"201":42,"x-note":42}}}},"components":{"securitySchemes":{"oauth":{"type":"oauth2","flows":{}}}}}"#;
    let findings = validate(&dir, "api.json", source);
    for (code, text) in [
        ("oas-security-scope-invalid", "42"),
        ("oas-http-boolean-invalid", "\"yes\""),
        ("oas-http-boolean-invalid", "0"),
        ("oas-http-boolean-invalid", "null"),
        ("oas-http-boolean-invalid", "[]"),
        ("oas-http-boolean-invalid", "{}"),
        ("oas-http-boolean-invalid", "\"false\""),
        ("oas-response-map-invalid", "[]"),
        ("oas-response-entry-invalid", "42"),
        ("oas-response-map-invalid", "false"),
        ("oas-response-invalid", "42"),
    ] {
        assert!(
            findings
                .iter()
                .any(|finding| finding.code == code && &source[finding.range.clone()] == text),
            "{code} {text}: {findings:?}"
        );
    }
    assert!(
        !findings.iter().any(|finding| matches!(
            finding.code,
            "oas-response-invalid" | "oas-response-map-invalid" | "oas-response-entry-invalid"
        ) && finding.range.start == source.rfind("42").unwrap()),
        "x-* response extension was interpreted as a response"
    );
}

#[test]
fn valid_security_alternatives_http_shapes_and_extensions_remain_valid() {
    let dir = directory("valid");
    let source = r#"{"openapi":"3.1.0","info":{"title":"x","version":"1"},"security":[{}, {"oauth":["read","write"],"api":[]}],"paths":{"/x":{"get":{"parameters":[{"name":"q","in":"query","required":false,"explode":true,"allowReserved":true,"allowEmptyValue":false,"deprecated":false}],"responses":{"200":{"description":"ok","content":{"application/json":{},"text/plain":{}},"headers":{"X-Rate":{"schema":{"type":"integer"}}},"links":{"next":{"operationId":"next"}}},"x-extra":{"anything":42}}}}},"components":{"securitySchemes":{"oauth":{"type":"oauth2","flows":{}},"api":{"type":"apiKey","name":"x","in":"header"}}}}"#;
    let findings = validate(&dir, "api.json", source);
    assert!(
        !findings.iter().any(|finding| matches!(
            finding.code,
            "oas-http-boolean-invalid"
                | "oas-responses-invalid"
                | "oas-response-invalid"
                | "oas-response-map-invalid"
                | "oas-response-entry-invalid"
                | "oas-security-scopes-invalid"
                | "oas-security-scope-invalid"
        )),
        "{findings:?}"
    );
}

#[test]
fn response_extensions_are_not_responses_but_real_responses_still_need_descriptions() {
    let dir = directory("response-description");
    let source = r#"{"openapi":"3.1.0","info":{"title":"x","version":"1"},"paths":{"/x":{"get":{"responses":{"200":{},"x-note":{"anything":42}}}}}}"#;
    let findings = validate(&dir, "api.json", source);
    let missing = findings
        .iter()
        .filter(|finding| finding.code == "oas-response-missing-description")
        .collect::<Vec<_>>();
    assert_eq!(missing.len(), 1, "{findings:?}");
    assert_eq!(&source[missing[0].range.clone()], "{}");
}

#[test]
fn x_prefixed_component_response_is_validated_at_its_referenced_source() {
    let dir = directory("x-component-response");
    let external = r#"{"Bad":{"content":[]}}"#;
    std::fs::write(dir.join("responses.json"), external).unwrap();
    let source = r##"{"openapi":"3.1.0","info":{"title":"x","version":"1"},"paths":{},"components":{"responses":{"x-error":{"$ref":"./responses.json#/Bad"}}}}"##;
    let findings = validate(&dir, "api.json", source);

    for code in [
        "oas-response-missing-description",
        "oas-response-map-invalid",
    ] {
        let finding = findings
            .iter()
            .find(|finding| finding.code == code)
            .unwrap_or_else(|| panic!("missing {code}: {findings:?}"));
        assert!(finding.doc.to_string().ends_with("/responses.json"));
        assert_eq!(
            &external[finding.range.clone()],
            if code == "oas-response-map-invalid" {
                "[]"
            } else {
                r#"{"content":[]}"#
            }
        );
    }
}

#[test]
fn external_request_body_reports_the_target_document() {
    let dir = directory("external");
    std::fs::write(
        dir.join("body.json"),
        r#"{"Body":{"required":"false","content":{}}}"#,
    )
    .unwrap();
    let source = r##"{"openapi":"3.1.0","info":{"title":"x","version":"1"},"paths":{"/x":{"post":{"requestBody":{"$ref":"./body.json#/Body"},"responses":{"204":{"description":"ok"}}}}}}"##;
    let findings = validate(&dir, "api.json", source);
    let finding = findings
        .iter()
        .find(|finding| finding.code == "oas-http-boolean-invalid")
        .unwrap();
    assert!(finding.doc.to_string().ends_with("/body.json"));
    let external = std::fs::read_to_string(dir.join("body.json")).unwrap();
    assert_eq!(&external[finding.range.clone()], "\"false\"");
}

#[test]
#[ignore = "requires OPENROUTER_WEB_ROOT tracked public spec"]
fn tracked_openrouter_mutations_are_caught_at_exact_values() {
    let root = std::env::var_os("OPENROUTER_WEB_ROOT").expect("OPENROUTER_WEB_ROOT");
    let original =
        std::fs::read_to_string(Path::new(&root).join("projects/docs/openapi/openapi.yaml"))
            .unwrap();
    let mut source = original.replacen(
        "\nsecurity:\n  - apiKey: []",
        "\nsecurity:\n  - apiKey: [42]",
        1,
    );
    let operation = source.find("operationId: 'createKeys'").unwrap();
    let required = source[operation..].find("        required: true").unwrap() + operation;
    source.replace_range(
        required..required + "        required: true".len(),
        "        required: 'false'",
    );
    let dir = directory("openrouter");
    let findings = validate(&dir, "openapi.yaml", &source);
    assert!(
        findings
            .iter()
            .any(|finding| finding.code == "oas-security-scope-invalid"
                && &source[finding.range.clone()] == "42")
    );
    assert!(
        findings
            .iter()
            .any(|finding| finding.code == "oas-http-boolean-invalid"
                && &source[finding.range.clone()] == "'false'")
    );
}

fn assert_at(
    dir: &Path,
    findings: &[suspect_validate::Diagnostic],
    file: &str,
    pointer: &str,
    code: &str,
) {
    let workspace = WorkspaceBuilder::new().root(dir).build().unwrap();
    let document = workspace.open(file).unwrap();
    let node = document
        .node_at_pointer(&suspect_low::Pointer::parse(pointer).unwrap())
        .unwrap();
    let matches = findings
        .iter()
        .filter(|finding| {
            finding.code == code
                && finding.doc == *document.doc().uri()
                && finding.range == node.byte_range()
        })
        .count();
    assert_eq!(matches, 1, "{file}{pointer} {code}: {findings:?}");
}

#[test]
fn security_containers_apply_to_documents_and_operations_not_path_items() {
    for version in ["3.0.4", "3.1.2", "3.2.0"] {
        let dir = directory(&format!("security-containers-{version}"));
        let source = r#"{"openapi":"VERSION","info":{"title":"x","version":"1"},"security":{},"paths":{"/x":{"security":[],"get":{"security":[false,{"oauth":null},{"oauth":[7]}],"responses":{"204":{"description":"ok"}}}},"/y":{"post":{"security":null,"responses":{"204":{"description":"ok"}}}}},"components":{"securitySchemes":{"oauth":{"type":"oauth2","flows":{}}}}}"#.replace("VERSION", version);
        let findings = validate(&dir, "api.json", &source);
        for (pointer, code) in [
            ("/security", "oas-security-invalid"),
            ("/paths/~1x/security", "oas-path-item-security-invalid"),
            (
                "/paths/~1x/get/security/0",
                "oas-security-requirement-invalid",
            ),
            (
                "/paths/~1x/get/security/1/oauth",
                "oas-security-scopes-invalid",
            ),
            (
                "/paths/~1x/get/security/2/oauth/0",
                "oas-security-scope-invalid",
            ),
            ("/paths/~1y/post/security", "oas-security-invalid"),
        ] {
            assert_at(&dir, &findings, "api.json", pointer, code);
        }
    }
}

#[test]
fn non_oauth_roles_are_versioned_and_scheme_references_are_resolved() {
    for (version, invalid) in [("3.0.4", true), ("3.1.2", false), ("3.2.0", false)] {
        let dir = directory(&format!("security-roles-{version}"));
        std::fs::write(
            dir.join("scheme.json"),
            r#"{"type":"http","scheme":"bearer"}"#,
        )
        .unwrap();
        let source = r#"{"openapi":"VERSION","info":{"title":"x","version":"1"},"paths":{},"security":[{"auth":["admin"]}],"components":{"securitySchemes":{"auth":{"$ref":"./scheme.json"}}}}"#.replace("VERSION", version);
        let findings = validate(&dir, "api.json", &source);
        if invalid {
            assert_at(
                &dir,
                &findings,
                "api.json",
                "/security/0/auth",
                "oas-security-scopes-nonempty",
            );
        } else {
            assert!(
                !findings
                    .iter()
                    .any(|finding| finding.code.starts_with("oas-security-")),
                "{findings:?}"
            );
        }
    }
}

#[test]
fn referenced_callback_cycles_keep_target_identity_and_path_siblings() {
    let dir = directory("callback-cycle");
    let callback = r##"{"event":{"post":{"security":42,"parameters":{},"requestBody":{"$ref":"./body.json"},"responses":[],"callbacks":{"again":{"$ref":"./callback.json"}}}}}"##;
    std::fs::write(dir.join("callback.json"), callback).unwrap();
    std::fs::write(
        dir.join("body.json"),
        r#"{"content":null,"required":"true"}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("path.json"),
        r#"{"get":{"security":[false],"responses":{"204":{"description":"ok"}}}}"#,
    )
    .unwrap();
    let source = r##"{"openapi":"3.1.0","info":{"title":"x","version":"1"},"paths":{"/x":{"$ref":"./path.json","post":{"security":false,"responses":{"204":{"description":"ok"}}}}},"components":{"callbacks":{"x-event":{"$ref":"./callback.json"}}}}"##;
    let findings = validate(&dir, "api.json", source);
    for (file, pointer, code) in [
        (
            "callback.json",
            "/event/post/security",
            "oas-security-invalid",
        ),
        (
            "callback.json",
            "/event/post/parameters",
            "oas-http-array-invalid",
        ),
        (
            "callback.json",
            "/event/post/responses",
            "oas-responses-invalid",
        ),
        ("body.json", "/content", "oas-http-map-invalid"),
        ("body.json", "/required", "oas-http-boolean-invalid"),
        (
            "path.json",
            "/get/security/0",
            "oas-security-requirement-invalid",
        ),
        (
            "api.json",
            "/paths/~1x/post/security",
            "oas-security-invalid",
        ),
    ] {
        assert_at(&dir, &findings, file, pointer, code);
    }
}

#[test]
fn declaration_containers_and_referenced_header_targets_are_not_normalized() {
    let dir = directory("declaration-containers");
    std::fs::write(dir.join("header.json"), "false").unwrap();
    let source = r#"{"openapi":"3.1.0","info":{"title":"x","version":"1"},"servers":[null,{"url":42,"variables":{"region":{"default":false,"enum":["east",9]}}}],"paths":{"/x":{"servers":{},"parameters":[null],"post":{"servers":[false],"requestBody":{},"callbacks":[],"responses":{"200":{"description":"ok","headers":{"x-rate":{"$ref":"./header.json"}},"content":{"application/json":false}}}}}},"components":{"parameters":[],"requestBodies":{"Bad":false}}}"#;
    let findings = validate(&dir, "api.json", source);
    for (pointer, code) in [
        ("/servers/0", "oas-server-invalid"),
        ("/servers/1/url", "oas-server-url-invalid"),
        (
            "/servers/1/variables/region/default",
            "oas-server-variable-default-invalid",
        ),
        (
            "/servers/1/variables/region/enum/1",
            "oas-server-variable-enum-invalid",
        ),
        ("/paths/~1x/servers", "oas-http-array-invalid"),
        ("/paths/~1x/parameters/0", "oas-parameter-invalid"),
        ("/paths/~1x/post/servers/0", "oas-server-invalid"),
        (
            "/paths/~1x/post/requestBody",
            "oas-request-body-content-missing",
        ),
        ("/paths/~1x/post/callbacks", "oas-http-map-invalid"),
        (
            "/paths/~1x/post/responses/200/content/application~1json",
            "oas-response-entry-invalid",
        ),
        ("/components/parameters", "oas-http-map-invalid"),
        ("/components/requestBodies/Bad", "oas-request-body-invalid"),
    ] {
        assert_at(&dir, &findings, "api.json", pointer, code);
    }
    assert_at(&dir, &findings, "header.json", "", "oas-header-invalid");
}

#[test]
fn additional_operations_webhooks_and_unused_path_items_are_checked_in_supported_versions() {
    let dir = directory("oas32-operation-positions");
    let source = r#"{"openapi":"3.2.0","info":{"title":"x","version":"1"},"paths":{"/x":{"query":{"security":{}},"additionalOperations":{"COPY":{"security":[null]}}}},"webhooks":{"x-event":{"post":{"security":false}}},"components":{"pathItems":{"x-unused":{"get":{"security":[{"auth":[false]}]}}}}}"#;
    let findings = validate(&dir, "api.json", source);
    for (pointer, code) in [
        ("/paths/~1x/query/security", "oas-security-invalid"),
        (
            "/paths/~1x/additionalOperations/COPY/security/0",
            "oas-security-requirement-invalid",
        ),
        ("/webhooks/x-event/post/security", "oas-security-invalid"),
        (
            "/components/pathItems/x-unused/get/security/0/auth/0",
            "oas-security-scope-invalid",
        ),
    ] {
        assert_at(&dir, &findings, "api.json", pointer, code);
    }
}

#[test]
fn security_extensions_examples_and_optional_operation_names_are_not_generation_policy() {
    let dir = directory("declaration-not-generation-policy");
    let source = r#"{"openapi":"3.1.0","info":{"title":"x","version":"1"},"security":[{}],"paths":{"/x":{"x-metadata":{"security":false},"get":{"security":[],"callbacks":{"x-event":{"x-metadata":{"security":false},"event":{"post":{"security":[{}],"responses":{"204":{"description":"ok"}}}}}},"responses":{"200":{"description":"ok","content":{"application/json":{"example":{"security":42,"servers":false}}}}}}}}}"#;
    let findings = validate(&dir, "api.json", source);
    assert!(
        !findings
            .iter()
            .any(|finding| finding.code.starts_with("oas-security-")
                || finding.code.starts_with("oas-http-")
                || finding.code == "oas-path-item-security-invalid"),
        "{findings:?}"
    );
}

#[test]
#[ignore = "requires OPENROUTER_WEB_ROOT tracked public spec"]
fn tracked_openrouter_outer_security_and_callback_mutations_preserve_existing_defects() {
    let root = std::env::var_os("OPENROUTER_WEB_ROOT").expect("OPENROUTER_WEB_ROOT");
    let original =
        std::fs::read_to_string(Path::new(&root).join("projects/docs/openapi/openapi.yaml"))
            .unwrap();
    assert!(original.contains("\nsecurity:\n  - apiKey: []"));
    assert!(original.contains("operationId: 'createKeys'"));
    let source = original.replacen("\nsecurity:\n  - apiKey: []", "\nsecurity: false", 1)
        .replacen("operationId: 'createKeys'", "operationId: 'createKeys'\n      callbacks:\n        receipt:\n          '{$request.body#/url}':\n            post:\n              security: [false]\n              responses:\n                '204':\n                  description: Received", 1);
    let dir = directory("openrouter-outer-callback");
    let findings = validate(&dir, "openapi.yaml", &source);
    assert_at(
        &dir,
        &findings,
        "openapi.yaml",
        "/security",
        "oas-security-invalid",
    );
    assert_at(
        &dir,
        &findings,
        "openapi.yaml",
        "/paths/~1keys/post/callbacks/receipt/{$request.body#~1url}/post/security/0",
        "oas-security-requirement-invalid",
    );
    assert!(
        findings
            .iter()
            .any(|finding| finding.code == "oas-schema-invalid-keyword"
                && finding.message.contains("exclusiveMinimum")
                && &source[finding.range.clone()] == "true"),
        "known public numeric declaration defect must remain visible"
    );
}

#[test]
fn empty_server_enum_is_recommended_against_in_30_but_invalid_in_31() {
    for (version, invalid) in [("3.0.4", false), ("3.1.2", true), ("3.2.0", true)] {
        let dir = directory(&format!("server-enum-{version}"));
        let source = r#"{"openapi":"VERSION","info":{"title":"x","version":"1"},"paths":{},"servers":[{"url":"https://example.com/{region}","variables":{"region":{"default":"east","enum":[]}}}]}"#.replace("VERSION", version);
        let findings = validate(&dir, "api.json", &source);
        if invalid {
            assert_at(
                &dir,
                &findings,
                "api.json",
                "/servers/0/variables/region/enum",
                "oas-server-variable-enum-invalid",
            );
        } else {
            assert!(
                !findings
                    .iter()
                    .any(|finding| finding.code == "oas-server-variable-enum-invalid"),
                "{findings:?}"
            );
        }
    }
}

#[test]
fn implicit_yaml_nulls_report_the_same_shape_codes_as_explicit_nulls() {
    for (index, (declaration, key, code)) in [
        ("security: VALUE\n", "security", "oas-security-invalid"),
        ("servers: VALUE\n", "servers", "oas-http-array-invalid"),
        ("components: VALUE\n", "components", "oas-http-map-invalid"),
        ("paths:\n  /x:\n    parameters: VALUE\n", "parameters", "oas-http-array-invalid"),
        ("paths:\n  /x:\n    post:\n      requestBody: VALUE\n", "requestBody", "oas-request-body-invalid"),
        ("paths:\n  /x:\n    post:\n      responses: VALUE\n", "responses", "oas-responses-invalid"),
        ("components:\n  callbacks: VALUE\n", "callbacks", "oas-http-map-invalid"),
        ("components:\n  requestBodies:\n    Body:\n      content: VALUE\n", "content", "oas-http-map-invalid"),
        ("components:\n  requestBodies:\n    Body:\n      content: {}\n      required: VALUE\n", "required", "oas-http-boolean-invalid"),
        ("components:\n  responses:\n    Response:\n      description: ok\n      headers: VALUE\n", "headers", "oas-response-map-invalid"),
        ("servers:\n  - url: VALUE\n", "url", "oas-server-url-invalid"),
        ("servers:\n  - url: https://example.com/{region}\n    variables:\n      region:\n        default: VALUE\n", "default", "oas-server-variable-default-invalid"),
        ("security:\n  - auth: VALUE\n", "auth", "oas-security-scopes-invalid"),
    ].into_iter().enumerate() {
        for value in ["", "null"] {
            let dir = directory(&format!("yaml-null-{index}-{}", value.len()));
            let source = format!("openapi: 3.1.0\ninfo: {{title: x, version: '1'}}\n{}", declaration.replace("VALUE", value));
            let findings = validate(&dir, "api.yaml", &source);
            let start = if value.is_empty() { source.find(&format!("{key}:")).unwrap() } else { source.find("null").unwrap() };
            let end = start + if value.is_empty() { key.len() } else { value.len() };
            assert_eq!(findings.iter().filter(|finding| finding.code == code && finding.range == (start..end)).count(), 1, "{source}: {findings:?}");
        }
    }
}

#[test]
fn escaped_extension_and_security_names_have_decoded_semantics() {
    let dir = directory("escaped-http-names");
    let source = r#"{"openapi":"3.0.4","info":{"title":"x","version":"1"},"paths":{"/x":{"get":{"responses":{"200":{"description":"ok"},"x\u002dnote":42}}}},"security":[{"a\u0075th":["admin"]}],"components":{"securitySchemes":{"auth":{"type":"http","scheme":"bearer"}},"callbacks":{"cb":{"x\u002dnote":42}}}}"#;
    let findings = validate(&dir, "api.json", source);
    assert_at(
        &dir,
        &findings,
        "api.json",
        "/security/0/auth",
        "oas-security-scopes-nonempty",
    );
    assert!(
        !findings.iter().any(|finding| matches!(
            finding.code,
            "oas-path-item-invalid"
                | "oas-security-unknown-scheme"
                | "oas-response-invalid"
                | "oas-response-missing-description"
        )),
        "{findings:?}"
    );
    let source = "openapi: 3.1.0\ninfo: {title: x, version: '1'}\npaths: {}\ncomponents:\n  callbacks:\n    cb:\n      \"x\\u002dnote\": {$ref: './must-not-load.json'}\n";
    let findings = validate(&dir, "api.yaml", source);
    assert!(
        !findings
            .iter()
            .any(|finding| matches!(finding.code, "unresolved-ref" | "oas-path-item-invalid")),
        "{findings:?}"
    );
}

#[test]
fn new_operation_and_media_positions_report_reference_failures_at_source() {
    let dir = directory("new-reference-positions");
    let source = r##"{"openapi":"3.2.0","info":{"title":"x","version":"1"},"paths":{"/x":{"query":{"requestBody":{"$ref":"#/missingQuery"}},"additionalOperations":{"COPY":{"parameters":[{"$ref":"#/missingParameter"}],"callbacks":{"x-event":{"event":{"post":{"requestBody":{"$ref":"#/missingCallback"}}}}}}}}},"webhooks":{"x-event":{"post":{"requestBody":{"$ref":"#/missingWebhook"}}}},"components":{"mediaTypes":{"Bad":{"$ref":"#/missingMedia"}},"callbacks":{"cb":{"$ref":null}}}}"##;
    let findings = validate(&dir, "api.json", source);
    for pointer in [
        "/paths/~1x/query/requestBody/$ref",
        "/paths/~1x/additionalOperations/COPY/parameters/0/$ref",
        "/paths/~1x/additionalOperations/COPY/callbacks/x-event/event/post/requestBody/$ref",
        "/webhooks/x-event/post/requestBody/$ref",
        "/components/mediaTypes/Bad/$ref",
    ] {
        assert_at(&dir, &findings, "api.json", pointer, "unresolved-ref");
    }
    assert_at(
        &dir,
        &findings,
        "api.json",
        "/components/callbacks/cb/$ref",
        "invalid-ref",
    );
}

#[test]
fn references_outside_the_source_manifest_have_a_distinct_located_code() {
    let dir = directory("manifest-reference");
    let source = r#"{"openapi":"3.2.0","info":{"title":"x","version":"1"},"paths":{},"components":{"mediaTypes":{"External":{"$ref":"./outside.json"}}}}"#;
    std::fs::write(dir.join("api.json"), source).unwrap();
    std::fs::write(dir.join("outside.json"), r#"{"schema":{"type":"string"}}"#).unwrap();
    let workspace = WorkspaceBuilder::new().root(&dir).build().unwrap();
    let entry_uri = workspace.open("api.json").unwrap().uri().clone();
    let session = Session::new(Arc::new(
        WorkspaceBuilder::new()
            .root(&dir)
            .allowed_documents([entry_uri])
            .build()
            .unwrap(),
    ));
    let findings = validate_entry(&session, "api.json").unwrap();
    assert_at(
        &dir,
        &findings,
        "api.json",
        "/components/mediaTypes/External/$ref",
        "ref-outside-allowlist",
    );
}
