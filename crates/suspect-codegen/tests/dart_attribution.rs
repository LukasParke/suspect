//! ua/v1 attribution compiles into every generated Dart package: constant parts
//! at generation time, resolution and caller overrides in the native runtime.
#![cfg(feature = "dart-sdk")]
use std::sync::Arc;

use serde_json::json;
use suspect_codegen::backend::{self, Backend, GenerationOptions};
use suspect_codegen::dart_sdk::{self, DartConfig, PackageConfig};
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
        backend: Backend::DartHttp,
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
fn dart_packages_compile_the_constants_into_the_library() {
    let files = generate("openrouter_dart");
    let attribution = content(&files, "dart/lib/src/attribution.dart");
    assert!(
        attribution.contains("const userAgentSuspectVersion = "),
        "{attribution}"
    );
    assert!(attribution.contains("\"openrouter_dart\""), "{attribution}");
    assert!(attribution.contains("\"1.2.0\""), "{attribution}");
    assert!(attribution.contains("\"3.1.2\""), "{attribution}");
    let library = content(&files, "dart/lib/openrouter_dart.dart");
    assert!(
        library.contains("part 'src/attribution.dart';"),
        "{library}"
    );
    let runtime = content(&files, "dart/lib/src/transport.dart");
    assert!(runtime.contains("_resolveUserAgent"), "{runtime}");
    assert!(runtime.contains("suspect/"), "{runtime}");
    assert!(runtime.contains("applicationId"), "{runtime}");
    assert!(runtime.contains("'user-agent'"), "{runtime}");
    let client = content(&files, "dart/lib/src/client.dart");
    assert!(client.contains("String? userAgent"), "{client}");
    assert!(client.contains("String? applicationId"), "{client}");
    let manifest = content(&files, "dart/sdk-manifest.json");
    assert!(manifest.contains("\"attribution\""), "{manifest}");
    assert!(
        manifest.contains("\"template_version\": \"v1\""),
        "{manifest}"
    );
    assert!(manifest.contains("\"language\": \"dart\""), "{manifest}");
}

#[test]
fn dart_packages_without_a_descriptor_disable_the_attribution_header() {
    let contract = contract();
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = dart_sdk::plan_sdk(
        contract,
        &selected,
        DartConfig {
            package: PackageConfig {
                name: "openrouter_dart".into(),
                version: "1.2.0".into(),
            },
            ..Default::default()
        },
    )
    .unwrap();
    assert!(plan.attribution().is_none());
    let files = plan.render();
    let attribution = content(&files, "dart/lib/src/attribution.dart");
    assert!(
        attribution.contains("const userAgentSuspectVersion = '';"),
        "{attribution}"
    );
    let runtime = content(&files, "dart/lib/src/transport.dart");
    assert!(
        runtime.contains("if(userAgentSuspectVersion.isEmpty){return null;}"),
        "{runtime}"
    );
}
