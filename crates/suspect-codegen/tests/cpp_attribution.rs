//! ua/v1 attribution compiles into every generated C++ package: constant parts
//! at generation time, resolution and caller controls in the native runtime.
#![cfg(feature = "cpp-sdk")]

use std::sync::Arc;

use serde_json::json;
use suspect_codegen::backend::{self, Backend, GenerationOptions};
use suspect_codegen::cpp_sdk::{SdkConfig, plan_sdk};
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

/// `Backend::CppHttp` requires an ASCII C++ identifier package name and a
/// MAJOR.MINOR.PATCH version; the namespace defaults to the package name.
fn generate(package_name: &str) -> Vec<suspect_codegen::OutFile> {
    let target = backend::TargetConfig {
        backend: Backend::CppHttp,
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
fn cpp_packages_compile_the_attribution_constants_into_the_package() {
    let files = generate("openrouter_cpp");
    let attribution = content(&files, "cpp/include/openrouter_cpp/attribution.hpp");
    assert!(
        attribution.contains("inline constexpr std::string_view attribution_suspect_version"),
        "{attribution}"
    );
    assert!(
        attribution.contains("inline constexpr std::string_view attribution_sdk_name"),
        "{attribution}"
    );
    assert!(
        attribution.contains("inline constexpr std::string_view attribution_spec_version"),
        "{attribution}"
    );
    assert!(attribution.contains("openrouter_cpp"), "{attribution}");
    assert!(attribution.contains("1.2.0"), "{attribution}");
    assert!(attribution.contains(OPENAPI_VERSION), "{attribution}");
    let runtime = content(&files, "cpp/src/wire.cpp");
    assert!(runtime.contains("resolve_user_agent"), "{runtime}");
    assert!(runtime.contains("attribution_suspect_version"), "{runtime}");
    assert!(runtime.contains("\"/unknown; openapi/\""), "{runtime}");
    assert!(runtime.contains("attribution_language"), "{runtime}");
    assert!(runtime.contains("User-Agent"), "{runtime}");
    assert!(runtime.contains("application_id"), "{runtime}");
    let options = content(&files, "cpp/include/openrouter_cpp/http.hpp");
    assert!(options.contains("user_agent"), "{options}");
    assert!(options.contains("application_id"), "{options}");
    let adapter = content(&files, "cpp/src/http.cpp");
    assert!(
        adapter.contains("result.user_agent=client.user_agent"),
        "{adapter}"
    );
    assert!(
        adapter.contains("result.application_id=client.application_id"),
        "{adapter}"
    );
}

#[test]
fn cpp_packages_without_attribution_emit_the_disabled_sentinel() {
    let contract = contract();
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = plan_sdk(contract, &selected, SdkConfig::default()).unwrap();
    let files = plan.render().unwrap();
    let attribution = content(&files, "cpp/include/generated_sdk/attribution.hpp");
    assert!(
        attribution.contains("An empty suspect version disables the automatic attribution header"),
        "{attribution}"
    );
    assert!(
        attribution.contains(r#"attribution_suspect_version = "";"#),
        "{attribution}"
    );
    let runtime = content(&files, "cpp/src/wire.cpp");
    assert!(
        runtime.contains("attribution_suspect_version.empty()"),
        "{runtime}"
    );
}
