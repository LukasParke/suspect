//! Focused physical document-base adoption. No earlier HTTP matrix is repeated.
use super::validation_tests::{Native, write};
use crate::http_protocol::Capability;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

fn directory(prefix: &str) -> PathBuf {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-csharp-document-servers");
    fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(fs::canonicalize(root).unwrap())
        .unwrap()
        .keep()
}
fn documents() -> Value {
    let mut entry = json!({"openapi":"3.2.0","$self":"https://logical.example/catalog/api.json#description","info":{"title":"Physical server bases","version":"1"},"paths":{}});
    for (path, item) in [
        ("default", "Default"),
        ("empty", "Empty"),
        ("declared", "Declared"),
        ("variable", "Variable"),
        ("absolute", "Absolute"),
        ("network", "Network"),
        ("oauth", "OAuth"),
        ("oidc", "Oidc"),
    ] {
        entry["paths"][format!("/{path}")] =
            json!({"$ref":format!("parts.json#/components/pathItems/{item}")});
    }
    entry["paths"]["/local"] = json!({"$ref":"local.json#/components/pathItems/Local"});
    let mut parts = json!({"openapi":"3.2.0","$self":"https://logical.example/catalog/parts.json#definition","info":{"title":"External operations","version":"1"},"paths":{},"components":{"pathItems":{},"securitySchemes":{
        "OAuth":{"type":"oauth2","oauth2MetadataUrl":"../oauth/metadata","flows":{"clientCredentials":{"tokenUrl":"./token","scopes":{"read":"Read access"}}}},
        "Oidc":{"type":"openIdConnect","openIdConnectUrl":"../openid/config"}
    }}});
    for (name, id, servers) in [
        ("Default", "defaultServer", Value::Null),
        ("Empty", "emptyServer", json!([])),
        (
            "Declared",
            "declaredServer",
            json!([{"url":"../Api%2Fv1/%2e%2E/{tenant}","variables":{"tenant":{"default":"North","enum":["North","South"]}}}]),
        ),
        (
            "Variable",
            "variableServer",
            json!([{"url":"{base}","variables":{"base":{"default":"http://absolute.example/v1"}}}]),
        ),
        (
            "Absolute",
            "absoluteServer",
            json!([{"url":"http://fixed.example/Keep%2f/%2e%2e/Case"}]),
        ),
        (
            "Network",
            "networkServer",
            json!([{"url":"//network.example/Top//Case"}]),
        ),
        ("OAuth", "oauthServer", json!([{"url":"../auth-api"}])),
        ("Oidc", "oidcServer", json!([{"url":"../oidc-api"}])),
    ] {
        let mut operation = json!({"operationId":id,"responses":{"204":{"description":"done"}}});
        if !servers.is_null() {
            operation["servers"] = servers;
        }
        if name == "OAuth" {
            operation["security"] = json!([{"OAuth":["read"]}]);
        }
        if name == "Oidc" {
            operation["security"] = json!([{"Oidc":[]}]);
        }
        parts["components"]["pathItems"][name] = json!({"get":operation});
    }
    json!({"entry":"http://requested.example/bootstrap/api.json","documents":[
        {"requested":"http://requested.example/bootstrap/api.json","effective":"http://entry.example/root/spec/api.json","value":entry},
        {"requested":"http://archive.example/download/parts.json","effective":"http://storage.example/releases/specs/parts.json","value":parts},
        {"requested":"file:///recorded/local/source.json","effective":"file:///recorded/local/source.json","value":{"openapi":"3.2.0","$self":"https://logical.example/catalog/local.json","info":{"title":"Local source","version":"1"},"paths":{},"components":{"pathItems":{"Local":{"get":{"operationId":"localServer","servers":[{"url":"../Local%2Fapi"}],"responses":{"204":{"description":"done"}}}}}}}}
    ]})
}
fn contract(fixture: &Value) -> Arc<Contract> {
    let provider = Arc::new(
        DocumentProvider::new(fixture["documents"].as_array().unwrap().iter().map(|doc| {
            ProvidedDocument::new(
                Uri::parse(doc["requested"].as_str().unwrap()).unwrap(),
                Uri::parse(doc["effective"].as_str().unwrap()).unwrap(),
                serde_json::to_vec(&doc["value"]).unwrap(),
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
    Arc::new(
        Contract::from_workspace(
            &workspace,
            &Uri::parse(fixture["entry"].as_str().unwrap()).unwrap(),
        )
        .unwrap(),
    )
}
fn staged_plan(contract: Arc<Contract>) -> super::SdkPlan {
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    super::plan_sdk(
        contract,
        &selected,
        super::SdkConfig {
            name: "Documents.Csharp".into(),
            version: "1.0.0".into(),
            namespace: "Documents.Csharp".into(),
        },
    )
    .unwrap()
}

#[test]
fn physical_server_metadata_and_schema_capability_fences() {
    let fixture = documents();
    let source = contract(&fixture);
    let plan = staged_plan(source.clone());
    assert!(plan.protocol().codec_roots().is_empty());
    assert!(plan.protocol().codec_schema_closure().is_empty());
    assert!(super::protocol::capabilities().supports(Capability::DocumentRelativeServers));
    assert!(
        plan.program().resource_context.is_none(),
        "HTTP logical metadata alone must not promote an empty schema closure"
    );
    let default = plan
        .operations()
        .iter()
        .find(|op| op.operation_id == "defaultServer")
        .unwrap();
    assert_eq!(
        default.wire.servers().candidates()[0]
            .document_base()
            .source()
            .document()
            .as_str(),
        "http://entry.example/root/spec/api.json"
    );
    let empty = plan
        .operations()
        .iter()
        .find(|op| op.operation_id == "emptyServer")
        .unwrap();
    assert_eq!(
        empty.wire.servers().candidates()[0]
            .document_base()
            .source()
            .document()
            .as_str(),
        "http://storage.example/releases/specs/parts.json"
    );
    let declared = plan
        .operations()
        .iter()
        .find(|op| op.operation_id == "declaredServer")
        .unwrap();
    let server = &declared.wire.servers().candidates()[0];
    assert_eq!(
        server
            .source()
            .unwrap()
            .terminal_resource()
            .unwrap()
            .base_uri(),
        "https://logical.example/catalog/parts.json"
    );
    assert_eq!(
        server.resolve_document_url(&Default::default()).unwrap(),
        "http://storage.example/releases/Api%2Fv1/%2e%2E/North"
    );
    for context in [
        declared.wire.source().use_site_resource().unwrap(),
        declared.wire.source().terminal_resource().unwrap(),
    ] {
        assert_eq!(
            Some(context.source().span()),
            source.source_span(context.source().source())
        );
        assert_eq!(
            Some(context.resource().span()),
            source.source_span(context.resource().source())
        );
    }
    for schema in [
        json!({"$id":"https://logical.example/schema","type":"string"}),
        json!({"$dynamicAnchor":"node","type":"object","properties":{"next":{"$dynamicRef":"#node"}}}),
    ] {
        let mut changed = fixture.clone();
        changed["documents"][1]["value"]["components"]["pathItems"]["Declared"]["get"]["responses"] =
            json!({"200":{"content":{"application/json":{"schema":schema}}}});
        let c = contract(&changed);
        let selected = c
            .operations()
            .filter(|op| op.operation_id() == Some("declaredServer"))
            .map(|op| op.source().clone())
            .collect::<Vec<_>>();
        let wire = crate::http_protocol::plan(
            &c,
            &selected,
            crate::http_protocol::Capabilities::for_adapter(
                "csharp-static-resource-fence",
                super::protocol::capabilities()
                    .enabled()
                    .iter()
                    .copied()
                    .filter(|c| {
                        !matches!(
                            c,
                            Capability::SchemaResources | Capability::DynamicSchemaReferences
                        )
                    }),
            ),
        );
        assert!(!wire.is_admitted());
        assert!(wire.diagnostics().iter().any(|d| matches!(
            d.capability(),
            Some(Capability::SchemaResources | Capability::DynamicSchemaReferences)
        ) && d.source().source().document().as_str()
            == "http://storage.example/releases/specs/parts.json"
            && !d.source().span().is_empty()
            && d.resource_context().is_some()));
    }
}

#[test]
#[ignore = "installed .NET 8/10 physical document-server bases, redirects, encoded paths and overrides"]
fn native_document_relative_servers() {
    let root = directory("native-");
    let fixture = documents();
    write(
        &root.join("source-documents.json"),
        serde_json::to_vec_pretty(&fixture).unwrap(),
    );
    let plan = staged_plan(contract(&fixture));
    let files = plan.render().unwrap();
    for (sdk, framework) in [("8.0.424", "net8.0"), ("10.0.400", "net10.0")] {
        let native = Native::new(&root, sdk, framework);
        crate::write_files(&files, &native.root).unwrap();
        native.checked(
            "csharp",
            "package-restore",
            &["restore", "--configfile", "../NuGet.Config"],
        );
        native.checked(
            "csharp",
            "package-pack",
            &["pack", "-c", "Release", "--no-restore", "-o", "../feed"],
        );
        native.project("consumer", Some("Documents.Csharp"), true);
        let mut consumer = include_str!("testdata/DocumentServerConsumer.cs").to_owned();
        for name in ["OAuth", "Oidc"] {
            let credential = plan
                .credential_bindings()
                .iter()
                .find(|c| c.requirement.name() == name)
                .unwrap();
            consumer = consumer.replace(&format!("__{name}__"), &credential.property_name);
        }
        write(&native.root.join("consumer/Program.cs"), consumer);
        native.checked(
            "consumer",
            "consumer-restore",
            &["restore", "--configfile", "../NuGet.Config"],
        );
        native.checked(
            "consumer",
            "consumer-run",
            &[
                "run",
                "-c",
                "Release",
                "--no-restore",
                "--",
                "../feed/Documents.Csharp.1.0.0.nupkg",
            ],
        );
        native.checked(
            "csharp/examples",
            "examples-restore",
            &["restore", "--configfile", "../../NuGet.Config"],
        );
        native.checked(
            "csharp/examples",
            "examples-run",
            &["run", "-c", "Release", "--no-restore"],
        );
        native.finish("physical-document-relative-servers");
    }
    println!("C# physical document-server evidence: {}", root.display());
}
