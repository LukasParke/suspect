//! Public admission witnesses for data the indexed HTTP views omit or
//! override: referenced path-item parameter collections, duplicate effective
//! parameters versus legal overrides, absent security declarations and
//! malformed request-body containers. Every case runs through both the Rust
//! and TypeScript planning seams with independently computed expectations.

use serde_json::{Value, json};
use std::sync::Arc;
use suspect_codegen::rust_http::{HttpConfig, HttpDiagnostic, plan_http as plan_rust};
use suspect_codegen::typescript::http::{
    HttpConfig as TypescriptConfig, plan_http as plan_typescript,
};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

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

fn selected(contract: &Arc<Contract>) -> Vec<SourceId> {
    contract
        .operations()
        .map(|op| op.source().clone())
        .collect()
}

fn admitted(files: &[(&str, Value)]) {
    let contract = workspace(files);
    let selected = selected(&contract);
    assert!(
        plan_rust(contract.clone(), &selected, HttpConfig::default()).is_ok(),
        "rust profile rejected the valid witness"
    );
    assert!(
        plan_typescript(contract, &selected, TypescriptConfig::expanded()).is_ok(),
        "typescript profile rejected the valid witness"
    );
}

fn rejected(files: &[(&str, Value)]) -> (Vec<HttpDiagnostic>, Vec<HttpDiagnostic>) {
    let contract = workspace(files);
    let selected = selected(&contract);
    let rust = plan_rust(contract.clone(), &selected, HttpConfig::default()).unwrap_err();
    let ts = plan_typescript(contract, &selected, TypescriptConfig::expanded()).unwrap_err();
    (rust, ts)
}

fn expect((rust, ts): &(Vec<HttpDiagnostic>, Vec<HttpDiagnostic>), code: &str, pointer: &str) {
    for (found, label) in [(rust, "rust"), (ts, "typescript")] {
        assert!(
            found
                .iter()
                .any(|d| d.code == code && d.source.pointer() == pointer && !d.at.is_empty()),
            "{label}: {found:?}"
        );
    }
}

/// A profile-admittable operation whose only parameter arrives through
/// inheritance, so the inherited collection is load-bearing.
fn base() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {"title": "Admission", "version": "1"},
        "servers": [{"url": "https://example.test"}],
        "security": [{"key": []}],
        "components": {
            "securitySchemes": {"key": {"type": "http", "scheme": "bearer"}},
            "schemas": {
                "Credit": {"type": "object", "required": ["amount"], "properties": {"amount": {"type": "number"}}}
            }
        },
        "paths": {}
    })
}

fn get_operation(schema: Value) -> Value {
    json!({
        "operationId": "getCredits",
        "responses": {
            "200": {"description": "ok", "content": {"application/json": {"schema": schema}}}
        }
    })
}

#[test]
fn referenced_path_item_parameters_cannot_disappear_from_either_http_profile() {
    // Valid witness: the inherited query parameter is admitted through an
    // external document reference and no false finding is produced for it.
    let shared = json!({
        "components": {"pathItems": {"Credits": {
            "parameters": [{"name": "limit", "in": "query", "schema": {"type": "integer"}}],
            "get": get_operation(json!({"type": "object"}))
        }}}
    });
    let mut valid = base();
    valid["paths"]["/credits"] = json!({"$ref": "shared.json#/components/pathItems/Credits"});
    admitted(&[("api.json", valid), ("shared.json", shared)]);

    // Invalid witness: a malformed inherited collection vanishes from the
    // indexed view, so admission uses the canonical collection source. The path
    // template has no placeholder, so nothing else would report this.
    let shared = json!({
        "components": {"pathItems": {"Credits": {
            "parameters": 42,
            "get": get_operation(json!({"type": "object"}))
        }}}
    });
    let mut invalid = base();
    invalid["paths"]["/credits"] = json!({"$ref": "shared.json#/components/pathItems/Credits"});
    let errors = rejected(&[("api.json", invalid), ("shared.json", shared)]);
    expect(
        &errors,
        "http-parameters-invalid",
        "/components/pathItems/Credits/parameters",
    );
    for errors in [&errors.0, &errors.1] {
        assert!(errors.iter().any(|error| {
            error.code == "http-parameters-invalid"
                && error
                    .source
                    .document()
                    .to_string()
                    .ends_with("/shared.json")
        }));
    }

    // A malformed entry of an inherited collection is reported at the
    // actual terminal parameter entry.
    let mut local = base();
    local["paths"]["/credits"] = json!({"$ref": "#/components/pathItems/Credits"});
    local["components"]["pathItems"]["Credits"] = json!({
        "parameters": [null],
        "get": get_operation(json!({"$ref": "#/components/schemas/Credit"}))
    });
    let errors = rejected(&[("api.json", local)]);
    expect(
        &errors,
        "http-metadata-object",
        "/components/pathItems/Credits/parameters/0",
    );
}

#[test]
fn duplicate_parameters_are_rejected_while_legal_overrides_admit() {
    // Valid witness: the operation parameter overrides the path-item one and
    // exactly one effective parameter remains.
    let mut valid = base();
    valid["paths"]["/credits"] = json!({
        "parameters": [{"name": "limit", "in": "query", "schema": {"type": "integer"}}],
        "get": {
            "operationId": "getCredits",
            "parameters": [{"name": "limit", "in": "query", "schema": {"type": "string"}}],
            "responses": {
                "200": {"description": "ok", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Credit"}}}}
            }
        }
    });
    admitted(&[("api.json", valid)]);

    let get = get_operation(json!({"$ref": "#/components/schemas/Credit"}));

    // Invalid witness: two same-level operation parameters would both reach
    // the wire with the same name.
    let mut operation = base();
    operation["paths"]["/credits"]["get"] = get.clone();
    operation["paths"]["/credits"]["get"]["parameters"] = json!([
        {"name": "limit", "in": "query", "schema": {"type": "integer"}},
        {"name": "limit", "in": "query", "schema": {"type": "string"}}
    ]);
    let errors = rejected(&[("api.json", operation)]);
    expect(
        &errors,
        "http-parameter-duplicate",
        "/paths/~1credits/get/parameters/1",
    );

    // Invalid witness: two same-level path-item parameters survive the
    // override merge as a repeated identity.
    let mut path_item = base();
    path_item["paths"]["/credits"] = json!({
        "parameters": [
            {"name": "limit", "in": "query", "schema": {"type": "integer"}},
            {"name": "limit", "in": "query", "schema": {"type": "string"}}
        ],
        "get": get
    });
    let errors = rejected(&[("api.json", path_item)]);
    expect(
        &errors,
        "http-parameter-duplicate",
        "/paths/~1credits/parameters/1",
    );
}

#[test]
fn absent_security_is_not_reported_as_a_malformed_container() {
    // No level declares security: canonical expanded adapters admit anonymous
    // calls. A real malformed container must still be rejected at its source.
    let mut spec = base();
    spec["paths"]["/credits"]["get"] =
        get_operation(json!({"$ref": "#/components/schemas/Credit"}));
    let object = spec.as_object_mut().unwrap();
    object.remove("security");
    admitted(&[("api.json", spec.clone())]);
    spec["security"] = json!({});
    expect(
        &rejected(&[("api.json", spec)]),
        "http-security-invalid",
        "/security",
    );
}

#[test]
fn malformed_request_body_containers_locate_their_terminal_source() {
    // Valid witness: an admitted inline request body.
    let mut valid = base();
    valid["paths"]["/credits"]["post"] = json!({
        "operationId": "createCredit",
        "requestBody": {
            "required": true,
            "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Credit"}}}
        },
        "responses": {
            "200": {"description": "ok", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Credit"}}}}
        }
    });
    admitted(&[("api.json", valid)]);

    // Invalid witness: a declared non-object container is a container defect,
    // not a media-type mismatch.
    let mut direct = base();
    direct["paths"]["/credits"]["post"] = json!({
        "operationId": "createCredit",
        "requestBody": [],
        "responses": {
            "200": {"description": "ok", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Credit"}}}}
        }
    });
    let errors = rejected(&[("api.json", direct)]);
    expect(
        &errors,
        "http-metadata-object",
        "/paths/~1credits/post/requestBody",
    );

    // Invalid witness: a reference resolving to a non-object is located at
    // the terminal source, not at the reference mount point.
    let mut referenced = base();
    referenced["paths"]["/credits"]["post"] = json!({
        "operationId": "createCredit",
        "requestBody": {"$ref": "#/components/requestBodies/Broken"},
        "responses": {
            "200": {"description": "ok", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Credit"}}}}
        }
    });
    referenced["components"]["requestBodies"] = json!({"Broken": []});
    let errors = rejected(&[("api.json", referenced)]);
    expect(
        &errors,
        "http-metadata-object",
        "/components/requestBodies/Broken",
    );
}
