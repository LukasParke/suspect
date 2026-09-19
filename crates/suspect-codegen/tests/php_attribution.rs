//! ua/v1 attribution compiles into the generated PHP package: constant parts at
//! generation time, PHP version discovery in the native runtime, and explicit
//! caller override/suppression options on the immutable ClientOptions.
#![cfg(all(feature = "php-sdk", feature = "http-protocol"))]
use std::sync::Arc;

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
            serde_json::to_vec(&serde_json::json!({
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
        backend: Backend::PhpHttp,
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
fn php_packages_compile_the_constants_into_the_package() {
    let files = generate("openrouter/php-sdk");
    let config = content(&files, "php/src/RuntimeConfig.php");
    assert!(
        config.contains("ATTRIBUTION_SUSPECT_VERSION = "),
        "{config}"
    );
    // The Composer vendor/package slash is not an RFC 9110 token character, so
    // the package identity is the sanitized form inside the User-Agent grammar.
    assert!(
        config.contains("ATTRIBUTION_SDK_NAME = \"openrouter-php-sdk\";"),
        "{config}"
    );
    assert!(
        config.contains("ATTRIBUTION_SDK_VERSION = \"1.2.0\";"),
        "{config}"
    );
    assert!(
        config.contains("ATTRIBUTION_SPEC_VERSION = \"3.1.2\";"),
        "{config}"
    );
    assert!(
        config.contains("ATTRIBUTION_LANGUAGE = \"php\";"),
        "{config}"
    );
    assert!(!config.contains("openrouter/php-sdk"), "{config}");
}

#[test]
fn php_runtime_resolves_the_user_agent_with_caller_overrides() {
    let files = generate("openrouter/php-sdk");
    let protocol = content(&files, "php/src/Protocol.php");
    assert!(
        protocol.contains("userAgent(ClientOptions $options)"),
        "{protocol}"
    );
    assert!(protocol.contains("applicationIdentity"), "{protocol}");
    // The language version is discovered at client construction from PHP itself.
    assert!(protocol.contains("PHP_VERSION"), "{protocol}");
    assert!(protocol.contains("'suspect/'"), "{protocol}");
    assert!(protocol.contains("; openapi/"), "{protocol}");
    // Declared User-Agent header parameters win; the automatic value is only
    // set when the assembled request has no user-agent header yet.
    assert!(
        protocol.contains("!isset($headers['user-agent'])"),
        "{protocol}"
    );
    let http = content(&files, "php/src/Http.php");
    assert!(http.contains("public ?string $userAgent = null"), "{http}");
    assert!(
        http.contains("public ?string $applicationId = null"),
        "{http}"
    );
    assert!(http.contains("suppresses the header"), "{http}");
    let client = content(&files, "php/src/Client.php");
    assert!(client.contains("ClientOptions"), "{client}");
}

#[test]
fn php_packages_without_a_descriptor_keep_the_disabled_sentinel() {
    let contract = contract();
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = suspect_codegen::php_sdk::protocol::plan_sdk(
        contract,
        &selected,
        suspect_codegen::php_sdk::PhpConfig::default(),
        suspect_codegen::php_sdk::protocol::capabilities(),
    )
    .unwrap();
    let files = plan.render();
    let config = content(&files, "php/src/RuntimeConfig.php");
    assert!(
        config.contains("ATTRIBUTION_SUSPECT_VERSION = \"\";"),
        "{config}"
    );
    assert!(config.contains("ATTRIBUTION_SDK_NAME = \"\";"), "{config}");
    assert!(config.contains("ATTRIBUTION_LANGUAGE = \"\";"), "{config}");
}
