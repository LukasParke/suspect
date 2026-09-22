//! ua/v1 attribution compiles into every generated Swift package: constant
//! parts at generation time, platform-version discovery in the native runtime.
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

fn generate(package_name: &str) -> Vec<suspect_codegen::OutFile> {
    let target = backend::TargetConfig {
        backend: Backend::SwiftHttp,
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
fn swift_packages_compile_the_attribution_constants() {
    let files = generate("OpenRouterSwift");
    let attribution = content(&files, "Attribution.swift");
    assert!(
        attribution.contains("enum Attribution: Sendable"),
        "{attribution}"
    );
    assert!(
        attribution.contains("static let suspectVersion ="),
        "{attribution}"
    );
    assert!(
        !attribution.contains("static let suspectVersion = \"\""),
        "{attribution}"
    );
    assert!(
        attribution.contains("static let sdkName = \"OpenRouterSwift\""),
        "{attribution}"
    );
    assert!(
        attribution.contains("static let sdkVersion = \"1.2.0\""),
        "{attribution}"
    );
    assert!(
        attribution.contains("static let specVersion = \"3.1.2\""),
        "{attribution}"
    );
    assert!(
        attribution.contains("static let language = \"swift\""),
        "{attribution}"
    );
}

#[test]
fn swift_runtime_resolves_the_automatic_user_agent() {
    let files = generate("OpenRouterSwift");
    let runtime = content(&files, "HTTPProtocol.swift");
    assert!(
        runtime.contains("static func resolveUserAgent"),
        "{runtime}"
    );
    assert!(
        runtime.contains("suspect/\\(Attribution.suspectVersion)"),
        "{runtime}"
    );
    assert!(
        runtime.contains(
            "(\\(Attribution.language)/\\(languageVersion); openapi/\\(Attribution.specVersion))"
        ),
        "{runtime}"
    );
    assert!(runtime.contains("static let languageVersion"), "{runtime}");
    assert!(runtime.contains("applicationIdentity"), "{runtime}");
    assert!(runtime.contains("resolveUserAgent(options)"), "{runtime}");
    assert!(
        runtime.contains("$0.name.lowercased() == \"user-agent\""),
        "{runtime}"
    );
}

#[test]
fn swift_client_options_carry_the_attribution_overrides() {
    let files = generate("OpenRouterSwift");
    let transport = content(&files, "HTTPTransport.swift");
    assert!(
        transport.contains("public var userAgent: String?"),
        "{transport}"
    );
    assert!(
        transport.contains("public var applicationId: String?"),
        "{transport}"
    );
    assert!(
        transport.contains("userAgent: String? = nil, applicationId: String? = nil"),
        "{transport}"
    );
}

#[test]
fn swift_packages_without_a_descriptor_emit_the_disabled_sentinel() {
    let contract = contract();
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = suspect_codegen::swift_sdk::plan_sdk(
        contract,
        &selected,
        suspect_codegen::swift_sdk::SwiftConfig::default(),
    )
    .unwrap();
    let files = suspect_codegen::swift_sdk::emit_sdk(
        &plan,
        &suspect_codegen::swift_sdk::PackageConfig {
            name: "OpenRouterSwift".into(),
            module_name: "OpenRouterSwift".into(),
            version: "1.2.0".into(),
        },
    )
    .unwrap();
    let attribution = content(&files, "Attribution.swift");
    assert!(
        attribution.contains("static let suspectVersion = \"\""),
        "{attribution}"
    );
    assert!(
        attribution.contains("static let sdkName = \"\""),
        "{attribution}"
    );
    assert!(
        attribution.contains("static let specVersion = \"\""),
        "{attribution}"
    );
}
