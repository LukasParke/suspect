#![cfg(feature = "java-sdk")]
//! ua/v1 attribution compiles into the generated Java package: constant parts
//! at generation time, JVM language-version discovery in the native runtime.
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
fn java_packages_embed_the_attribution_constants_and_runtime() {
    let files = generate(Backend::JavaHttp, "test.suspect:openrouter-java");
    let attribution = content(&files, "test/suspect/Attribution.java");
    assert!(
        attribution.contains("public static final String SUSPECT_VERSION"),
        "{attribution}"
    );
    assert!(
        attribution
            .contains("public static final String SDK_NAME = \"test.suspect-openrouter-java\""),
        "{attribution}"
    );
    assert!(
        attribution.contains("public static final String SDK_VERSION = \"1.2.0\""),
        "{attribution}"
    );
    assert!(
        attribution.contains("public static final String SPEC_VERSION = \"3.1.2\""),
        "{attribution}"
    );
    assert!(
        attribution.contains("public static final String LANGUAGE = \"java\""),
        "{attribution}"
    );
    let runtime = content(&files, "test/suspect/HttpRuntime.java");
    assert!(runtime.contains("resolveUserAgent"), "{runtime}");
    assert!(runtime.contains("suspect/"), "{runtime}");
    assert!(runtime.contains("userAgent"), "{runtime}");
    assert!(runtime.contains("applicationId"), "{runtime}");
    assert!(
        runtime.contains("System.getProperty(\"java.version\")"),
        "{runtime}"
    );
    assert!(
        runtime.contains("headers.putIfAbsent(\"User-Agent\",userAgent)"),
        "{runtime}"
    );
}

#[test]
fn java_packages_without_a_descriptor_emit_the_disabled_sentinel() {
    let contract = contract();
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = suspect_codegen::java_sdk::plan_sdk(
        contract,
        &selected,
        suspect_codegen::java_sdk::PackageConfig {
            package: "test.suspect.openrouter_java".into(),
            version: "1.2.0".into(),
            api_name: "Client".into(),
        },
        &[],
    )
    .unwrap();
    assert!(plan.attribution().is_none());
    let files = plan.render().unwrap();
    let attribution = content(&files, "test/suspect/openrouter_java/Attribution.java");
    assert!(
        attribution.contains("public static final String SUSPECT_VERSION = \"\""),
        "{attribution}"
    );
    assert!(
        attribution.contains("ua/v1 attribution is disabled for this package"),
        "{attribution}"
    );
    let runtime = content(&files, "test/suspect/openrouter_java/HttpRuntime.java");
    assert!(runtime.contains("resolveUserAgent"), "{runtime}");
}
