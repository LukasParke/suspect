//! Credential compatibility is a native API/auth surface, not physical source
//! identity. These checks use the public Contract -> native capture -> compare seam.
#![cfg(feature = "ruby-sdk")]

use std::sync::Arc;

use serde_json::{Value, json};
use suspect_codegen::{
    backend::{Backend, TargetConfig},
    compatibility::{self, CompatibilityReport, Direction, Impact, PlanStatus},
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

const BEFORE: &str = "https://physical-before.ruby.test/openapi.json";
const AFTER: &str = "https://physical-after.ruby.test/openapi.json";

fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::RubyHttp,
        package_name: "ruby-credential-comparison".into(),
        package_version: "0.1.0".into(),
        import_name: Some("RubyCredentialComparison".into()),
    }
}

fn api() -> Value {
    json!({
        "openapi":"3.1.2", "info":{"title":"Credential relocation","version":"1"},
        "servers":[{"url":"https://service.ruby.test/v1"}],
        "security":[{"apiKey":[]}],
        "components":{"securitySchemes":{"apiKey":{"type":"http","scheme":"bearer"}}},
        "paths":{"/credits":{"get":{"operationId":"getCredits","responses":{"200":{"description":"OK"}}}}}
    })
}

fn load(uri: &str, document: Value) -> Arc<Contract> {
    let uri = Uri::parse(uri).unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            uri.clone(),
            uri.clone(),
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
    let contract = Arc::new(Contract::from_workspace(&workspace, &uri).unwrap());
    assert!(workspace.failed_document_uris().is_empty());
    contract
}

fn compare(before: Value, after: Value) -> CompatibilityReport {
    compatibility::compare(load(BEFORE, before), load(AFTER, after), &[], &[target()]).unwrap()
}

fn rich_auth() -> Value {
    let mut document = api();
    document["openapi"] = json!("3.2.0");
    document["components"]["securitySchemes"] = json!({
        "apiKey":{"type":"http","scheme":"bearer","bearerFormat":"JWT"},
        "basic":{"type":"http","scheme":"basic"},
        "header":{"type":"apiKey","in":"header","name":"X-API-Key"},
        "query":{"type":"apiKey","in":"query","name":"source"},
        "cookie":{"type":"apiKey","in":"cookie","name":"session"},
        "oauth":{"type":"oauth2","oauth2MetadataUrl":"./.well-known/authorization-server","flows":{
            "authorizationCode":{"authorizationUrl":"../authorize","tokenUrl":"./token","refreshUrl":"../refresh","scopes":{"read":"Read","source":"Source","value":"Value","description":"Description","span":"Span"}},
            "deviceAuthorization":{"deviceAuthorizationUrl":"../device","tokenUrl":"./token","scopes":{"read":"Read"}}
        }},
        "oidc":{"type":"openIdConnect","openIdConnectUrl":"../.well-known/openid-configuration"}
    });
    document["security"] = json!([{"apiKey":["operator"]},{"basic":[]},{"header":[]},{"query":[]},{"cookie":[]},{"oauth":["read"]},{"oidc":["openid"]},{}]);
    document
}

fn credential(report: &CompatibilityReport, after: bool) -> &Value {
    let native = &report.native[0];
    let snapshot = if after {
        native.after.as_ref()
    } else {
        native.before.as_ref()
    }
    .unwrap();
    assert_eq!(
        snapshot.status,
        PlanStatus::Planned,
        "{:?}",
        snapshot.findings
    );
    &snapshot.operations[0].descriptor["credential"]
}

fn requirement<'a>(value: &'a Value, name: &str) -> &'a Value {
    value["alternatives"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|alternative| alternative["requirements"].as_array().unwrap())
        .find(|requirement| requirement["name"] == name)
        .unwrap()
}

fn changed(before: Value, after: Value, label: &str) -> CompatibilityReport {
    let report = compare(before, after);
    assert_ne!(
        credential(&report, false),
        credential(&report, true),
        "{label}"
    );
    let changes = report.native[0]
        .changes
        .iter()
        .filter(|c| c.code == "native-credentials-changed")
        .collect::<Vec<_>>();
    assert_eq!(changes.len(), 1, "{label}: {}", report.migration_notes());
    assert_eq!(changes[0].impact, Impact::PotentiallyBreaking);
    assert_eq!(changes[0].source_before.as_ref().unwrap().document, BEFORE);
    assert_eq!(changes[0].source_after.as_ref().unwrap().document, AFTER);
    report
}

#[test]
fn source_relocation_alone_preserves_native_credential_equality_and_locations() {
    let report = compare(api(), api());
    let native = &report.native[0];
    let before = native.before.as_ref().unwrap();
    let after = native.after.as_ref().unwrap();
    assert_eq!(before.status, PlanStatus::Planned, "{:?}", before.findings);
    assert_eq!(after.status, PlanStatus::Planned, "{:?}", after.findings);
    assert_eq!(before.operations[0].source.document, BEFORE);
    assert_eq!(after.operations[0].source.document, AFTER);
    assert!(
        before.operations[0].source.span.is_some() && after.operations[0].source.span.is_some()
    );
    assert_eq!(
        before.operations[0].descriptor["credential"], after.operations[0].descriptor["credential"],
        "physical relocation leaked into the native credential descriptor"
    );
    assert!(native.changes.is_empty(), "{}", report.migration_notes());
    assert!(
        report.is_proven_compatible(),
        "{}",
        report.migration_notes()
    );
}

#[test]
fn relocated_oauth_oidc_and_api_keys_keep_typed_values_and_url_bases() {
    let report = compare(rich_auth(), rich_auth());
    let before = credential(&report, false);
    assert_eq!(before, credential(&report, true));
    assert!(
        report.native[0].changes.is_empty(),
        "{}",
        report.migration_notes()
    );
    assert!(
        report.is_proven_compatible(),
        "{}",
        report.migration_notes()
    );
    let oauth = &requirement(before, "oauth")["credential"];
    assert_eq!(oauth["url_base"], "effective-server");
    assert_eq!(oauth["metadata_url"], "./.well-known/authorization-server");
    let flow = oauth["flows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["kind"] == "authorizationCode")
        .unwrap();
    assert_eq!(flow["url_base"], "effective-server");
    assert_eq!(flow["authorization_url"], "../authorize");
    assert_eq!(flow["token_url"], "./token");
    assert_eq!(flow["refresh_url"], "../refresh");
    assert_eq!(
        flow["scopes"],
        json!([{"name":"description","value":"Description"},{"name":"read","value":"Read"},{"name":"source","value":"Source"},{"name":"span","value":"Span"},{"name":"value","value":"Value"}])
    );
    assert_eq!(requirement(before, "query")["credential"]["name"], "source");
    assert_eq!(
        requirement(before, "oidc")["credential"]["url_base"],
        "effective-server"
    );
    assert_eq!(
        requirement(before, "oidc")["credential"]["discovery_url"],
        "../.well-known/openid-configuration"
    );
    // A literal source/value map must not be mistaken for a Located wrapper.
    let mut literal = rich_auth();
    literal["components"]["securitySchemes"]["oauth"]["flows"]["authorizationCode"]["scopes"] =
        json!({"source":"Source scope","value":"Value scope"});
    literal["security"] = json!([{"oauth":[]}]);
    let report = compare(literal.clone(), literal.clone());
    let scope_entries =
        &requirement(credential(&report, false), "oauth")["credential"]["flows"][0]["scopes"];
    assert_eq!(
        scope_entries,
        &json!([{"name":"source","value":"Source scope"},{"name":"value","value":"Value scope"}])
    );
    let mut after = literal.clone();
    after["components"]["securitySchemes"]["oauth"]["flows"]["authorizationCode"]["scopes"]["source"] =
        json!("Updated source scope");
    changed(literal, after, "literal scope description");
    let mut fields = rich_auth();
    fields["security"] = json!([{"oauth":[]}]);
    fields["components"]["securitySchemes"]["oauth"]["flows"]["authorizationCode"]["scopes"] =
        json!({"name":"Name scope","type":"Type scope","wire":"Wire scope"});
    let mut after = fields.clone();
    after["components"]["securitySchemes"]["oauth"]["flows"]["authorizationCode"]["scopes"]
        .as_object_mut()
        .unwrap()
        .remove("wire");
    changed(
        fields,
        after,
        "literal scope name resembling descriptor metadata",
    );
}

#[test]
fn credential_names_types_attachments_alternatives_and_permissions_still_change() {
    let baseline = rich_auth();
    for (pointer, value) in [
        ("/components/securitySchemes/apiKey/scheme", json!("basic")),
        (
            "/components/securitySchemes/apiKey/bearerFormat",
            json!("opaque"),
        ),
        (
            "/components/securitySchemes/header/name",
            json!("X-Other-Key"),
        ),
        ("/components/securitySchemes/header/in", json!("query")),
        ("/components/securitySchemes/query/in", json!("cookie")),
        ("/security/0/apiKey", json!(["auditor"])),
        ("/security/5/oauth", json!(["read", "source"])),
        ("/security/6/oidc", json!(["openid", "email"])),
    ] {
        let mut after = baseline.clone();
        *after.pointer_mut(pointer).unwrap() = value;
        changed(baseline.clone(), after, pointer);
    }
    let mut renamed = baseline.clone();
    let definition = renamed["components"]["securitySchemes"]
        .as_object_mut()
        .unwrap()
        .remove("apiKey")
        .unwrap();
    renamed["components"]["securitySchemes"]["renamed"] = definition;
    renamed["security"][0] = json!({"renamed":["operator"]});
    changed(baseline.clone(), renamed, "credential caller name");
    let mut reordered = baseline.clone();
    reordered["security"].as_array_mut().unwrap().swap(0, 1);
    let report = changed(baseline.clone(), reordered, "public alternative indices");
    assert!(
        report.wire.is_empty(),
        "wire-equivalent OR order: {}",
        report.migration_notes()
    );
    let mut conjunction = baseline.clone();
    conjunction["security"][0]["header"] = json!([]);
    changed(baseline, conjunction, "conjunctive attachments");
    let mut no_auth = api();
    no_auth["security"] = json!([]);
    let mut undeclared = no_auth.clone();
    undeclared.as_object_mut().unwrap().remove("security");
    changed(no_auth, undeclared, "disabled versus undeclared security");
}

#[test]
fn oauth_flows_endpoints_and_oidc_discovery_changes_survive_capture() {
    let baseline = rich_auth();
    for (pointer, value) in [
        (
            "/components/securitySchemes/oauth/oauth2MetadataUrl",
            json!("../other-metadata"),
        ),
        (
            "/components/securitySchemes/oauth/flows/authorizationCode/authorizationUrl",
            json!("https://auth.ruby.test/authorize"),
        ),
        (
            "/components/securitySchemes/oauth/flows/authorizationCode/tokenUrl",
            json!("../new-token"),
        ),
        (
            "/components/securitySchemes/oauth/flows/authorizationCode/refreshUrl",
            json!("../new-refresh"),
        ),
        (
            "/components/securitySchemes/oauth/flows/deviceAuthorization/deviceAuthorizationUrl",
            json!("../other-device"),
        ),
        (
            "/components/securitySchemes/oauth/flows/authorizationCode/scopes",
            json!({"read":"Read","new-scope":"New scope"}),
        ),
        (
            "/components/securitySchemes/oauth/flows",
            json!({"clientCredentials":{"tokenUrl":"./token","scopes":{"read":"Read"}}}),
        ),
        (
            "/components/securitySchemes/oidc/openIdConnectUrl",
            json!("https://identity.ruby.test/.well-known/openid-configuration"),
        ),
    ] {
        let mut after = baseline.clone();
        *after.pointer_mut(pointer).unwrap() = value;
        changed(baseline.clone(), after, pointer);
    }
}

#[test]
fn effective_server_base_changes_remain_real_wire_changes() {
    let mut baseline = rich_auth();
    baseline["security"] = json!([{"oauth":["read"]}]);
    let mut after = baseline.clone();
    after["servers"][0]["url"] = json!("https://different-service.ruby.test/v2/");
    let report = compare(baseline.clone(), after);
    assert!(report.wire.iter().any(|c| c.code == "wire-servers-changed"));
    assert!(!report.is_proven_compatible());
    // A relative server's effective physical base is observable behavior. The
    // common wire record, rather than a scheme's unrelated ResourceContext,
    // retains the actual resolved HTTP endpoints and their physical locations.
    baseline["servers"][0]["url"] = json!("./service/");
    let report = compare(baseline.clone(), baseline);
    let change = report
        .wire
        .iter()
        .find(|c| c.code == "wire-servers-changed")
        .unwrap();
    assert_eq!(
        change.before.as_ref().unwrap()[0]["relativeResolution"]["resolved"],
        "https://physical-before.ruby.test/service/"
    );
    assert_eq!(
        change.after.as_ref().unwrap()[0]["relativeResolution"]["resolved"],
        "https://physical-after.ruby.test/service/"
    );
    assert_eq!(change.source_before.as_ref().unwrap().document, BEFORE);
    assert_eq!(change.source_after.as_ref().unwrap().document, AFTER);
    assert!(!report.is_proven_compatible());
}

#[test]
fn request_name_tightening_remains_wire_change_without_credential_noise() {
    let mut baseline = api();
    baseline["paths"] = json!({"/keys":{"post":{"operationId":"createKey","requestBody":{"required":true,"content":{"application/json":{"schema":{"type":"object","required":["name"],"properties":{"name":{"type":"string","minLength":1}}}}}},"responses":{"200":{"description":"OK"}}}}});
    let mut after = baseline.clone();
    after["paths"]["/keys"]["post"]["requestBody"]["content"]["application/json"]["schema"]["properties"]
        ["name"]["minLength"] = json!(8);
    let report = compare(baseline, after);
    assert_eq!(credential(&report, false), credential(&report, true));
    assert!(
        report.native[0].changes.is_empty(),
        "{}",
        report.migration_notes()
    );
    assert!(
        report
            .wire
            .iter()
            .any(|change| change.direction == Some(Direction::Request)
                && change.impact == Impact::PotentiallyBreaking
                && change
                    .schema_deltas
                    .iter()
                    .any(|delta| delta.keyword == "minLength"
                        && delta.before == Some(json!(1))
                        && delta.after == Some(json!(8)))),
        "{}",
        report.migration_notes()
    );
    assert!(!report.is_proven_compatible());
}
