//! ua/v1 attribution compiles into every generated package: constant parts at
//! generation time, language-version discovery in the native runtime.
use std::sync::Arc;

use serde_json::json;
use suspect_codegen::backend::{self, Backend, GenerationOptions};
use suspect_ir::contract::Contract;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

const OPENAPI_VERSION: &str = "3.1.2";

fn contract() -> Arc<Contract> {
    let entry = Uri::parse("https://source.attribution.test/openapi.json").unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            serde_json::to_vec(&json!({
                "openapi": OPENAPI_VERSION, "info":{"title":"Attribution","version":"1"},
                "servers":[{"url":"https://api.attribution.test/v1"}],
                "paths":{
                    "/widgets":{"get":{"operationId":"listWidgets","responses":{"200":{"description":"Ok","content":{"application/json":{"schema":{"type":"array","items":{"type":"string"}}}}}}}}
                }
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

fn generate(backend: Backend, package_name: &str) -> Vec<suspect_codegen::OutFile> {
    let target = backend::TargetConfig {
        backend,
        package_name: package_name.into(),
        package_version: "1.2.0".into(),
        import_name: None,
    };
    let selected = {
        let contract = contract();
        contract
            .operations()
            .map(|operation| operation.source().clone())
            .collect::<Vec<_>>()
    };
    let contract = contract();
    backend::generate_with_options(contract, &selected, &target, &GenerationOptions::default())
        .unwrap()
}

fn content(files: &[suspect_codegen::OutFile], suffix: &str) -> String {
    files
        .iter()
        .find(|file| file.path.ends_with(suffix))
        .unwrap_or_else(|| panic!("no generated file ending in {suffix}"))
        .content
        .clone()
}

#[test]
fn typescript_packages_carry_the_attribution_descriptor_and_runtime() {
    let files = generate(Backend::TypescriptHttp, "openrouter-typescript");
    let operations = content(&files, "typescript/operations.ts");
    assert!(
        operations.contains("\"template_version\":\"v1\""),
        "{operations}"
    );
    assert!(operations.contains("\"suspect_version\""), "{operations}");
    assert!(
        operations.contains("\"sdk_name\":\"openrouter-typescript\""),
        "{operations}"
    );
    assert!(
        operations.contains("\"spec_version\":\"3.1.2\""),
        "{operations}"
    );
    assert!(
        operations.contains("\"language\":\"typescript\""),
        "{operations}"
    );
    let runtime = content(&files, "typescript/runtime.ts");
    assert!(runtime.contains("resolveUserAgent"), "{runtime}");
    assert!(runtime.contains("suspect/"), "{runtime}");
    assert!(runtime.contains("applicationId"), "{runtime}");
}

#[test]
fn python_packages_attach_attribution_to_both_clients() {
    let files = generate(Backend::PythonHttp, "openrouter-python");
    let client = content(&files, "src/openrouter_python/_client.py");
    assert!(client.contains("_ATTRIBUTION = {"), "{client}");
    assert!(client.contains("'suspect_version':"), "{client}");
    assert!(
        client.contains("'sdk_name': \"openrouter-python\""),
        "{client}"
    );
    assert!(client.contains("class Client(SyncClient):"), "{client}");
    assert!(client.contains("_ATTRIBUTION = _ATTRIBUTION"), "{client}");
    assert!(
        client.contains("class AsyncClient(AsyncClientBase):"),
        "{client}"
    );
    let runtime = content(&files, "src/openrouter_python/_runtime.py");
    assert!(runtime.contains("_resolve_user_agent"), "{runtime}");
    assert!(runtime.contains("platform.python_version()"), "{runtime}");
    assert!(runtime.contains("application_id"), "{runtime}");
}

#[test]
fn go_packages_compile_the_constants_into_the_package() {
    let files = generate(Backend::GoHttp, "github.com/openrouter/go-sdk");
    let attribution = content(&files, "go/attribution.go");
    assert!(
        attribution.contains("userAgentSuspectVersion ="),
        "{attribution}"
    );
    assert!(attribution.contains("userAgentSDKName"), "{attribution}");
    assert!(
        attribution.contains("userAgentSpecVersion"),
        "{attribution}"
    );
    let runtime = content(&files, "go/http_runtime.go");
    assert!(runtime.contains("resolveUserAgent"), "{runtime}");
    assert!(runtime.contains("UserAgent"), "{runtime}");
    assert!(runtime.contains("ApplicationID"), "{runtime}");
}

#[test]
fn rust_packages_embed_the_constants_module() {
    let files = generate(Backend::RustHttp, "openrouter-rust");
    let attribution = content(&files, "rust/src/attribution.rs");
    assert!(
        attribution.contains("ATTRIBUTION_SUSPECT_VERSION"),
        "{attribution}"
    );
    assert!(attribution.contains("openrouter-rust"), "{attribution}");
    let runtime = content(&files, "rust/src/http.rs");
    assert!(runtime.contains("resolve_user_agent"), "{runtime}");
    assert!(runtime.contains("user_agent"), "{runtime}");
    assert!(runtime.contains("application_id"), "{runtime}");
    let root = content(&files, "rust/src/lib.rs");
    assert!(root.contains("mod attribution;"), "{root}");
}
