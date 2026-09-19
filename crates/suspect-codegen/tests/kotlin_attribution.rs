//! ua/v1 attribution compiles into the generated Kotlin package: constant parts
//! at generation time, JVM language-version discovery in the native runtime.
#![cfg(feature = "kotlin-sdk")]
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
fn kotlin_packages_embed_the_attribution_constants_and_runtime() {
    let files = generate(Backend::KotlinHttp, "test.suspect:openrouter-kotlin");
    let attribution = content(&files, "openrouter_kotlin/Attribution.kt");
    assert!(
        attribution.contains("ATTRIBUTION_SUSPECT_VERSION: String ="),
        "{attribution}"
    );
    assert!(
        attribution.contains("ATTRIBUTION_SDK_NAME: String = \"test.suspect-openrouter-kotlin\""),
        "{attribution}"
    );
    assert!(
        attribution.contains("ATTRIBUTION_SDK_VERSION: String = \"1.2.0\""),
        "{attribution}"
    );
    assert!(
        attribution.contains("ATTRIBUTION_SPEC_VERSION: String = \"3.1.2\""),
        "{attribution}"
    );
    let runtime = content(&files, "openrouter_kotlin/Protocol.kt");
    assert!(runtime.contains("resolveUserAgent"), "{runtime}");
    assert!(runtime.contains("suspect/"), "{runtime}");
    assert!(runtime.contains("applicationId"), "{runtime}");
    assert!(
        runtime.contains("System.getProperty(\"java.version\") ?: \"unknown\""),
        "{runtime}"
    );
    let options = content(&files, "openrouter_kotlin/Http.kt");
    assert!(options.contains("userAgent: String? = null"), "{options}");
    assert!(
        options.contains("applicationId: String? = null"),
        "{options}"
    );
}

#[test]
fn kotlin_packages_without_a_descriptor_emit_the_disabled_sentinel() {
    let selected = {
        let contract = contract();
        contract
            .operations()
            .map(|operation| operation.source().clone())
            .collect::<Vec<_>>()
    };
    let contract = contract();
    let config = suspect_codegen::kotlin_sdk::SdkConfig {
        group_id: "test.suspect".into(),
        artifact_id: "openrouter-kotlin".into(),
        version: "1.2.0".into(),
        package_name: "test.suspect.openrouter_kotlin".into(),
        ..Default::default()
    };
    let plan = suspect_codegen::kotlin_sdk::plan_sdk(contract, &selected, config).unwrap();
    let files = plan.render().unwrap();
    let attribution = content(&files, "openrouter_kotlin/Attribution.kt");
    assert!(
        attribution.contains("ATTRIBUTION_SUSPECT_VERSION: String = \"\""),
        "{attribution}"
    );
    assert!(
        attribution
            .contains("// An empty suspect version disables the automatic attribution header."),
        "{attribution}"
    );
    let runtime = content(&files, "openrouter_kotlin/Protocol.kt");
    assert!(runtime.contains("resolveUserAgent"), "{runtime}");
}
