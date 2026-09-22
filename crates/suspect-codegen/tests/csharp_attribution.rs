//! ua/v1 attribution compiles into every generated C# package: constant parts
//! at generation time, language-version discovery in the native runtime.
#![cfg(feature = "csharp-sdk")]

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
fn csharp_packages_compile_the_attribution_constants_and_runtime() {
    let files = generate(Backend::CsharpHttp, "OpenRouter.CSharp");
    let attribution = content(&files, "csharp/src/Attribution.g.cs");
    assert!(
        attribution.contains("internal static class Attribution"),
        "{attribution}"
    );
    assert!(
        attribution.contains("internal const string SuspectVersion ="),
        "{attribution}"
    );
    assert!(
        attribution.contains("internal const string SdkName = \"OpenRouter.CSharp\";"),
        "{attribution}"
    );
    assert!(
        attribution.contains("internal const string SdkVersion = \"1.2.0\";"),
        "{attribution}"
    );
    assert!(
        attribution.contains("internal const string SpecVersion = \"3.1.2\";"),
        "{attribution}"
    );
    assert!(
        attribution.contains("internal const string Language = \"csharp\";"),
        "{attribution}"
    );
    let runtime = content(&files, "csharp/src/HttpRuntime.cs");
    assert!(runtime.contains("ResolveUserAgent"), "{runtime}");
    assert!(runtime.contains("\"suspect/\""), "{runtime}");
    assert!(runtime.contains("Attribution.SuspectVersion"), "{runtime}");
    assert!(
        runtime.contains("public string? UserAgent { get; init; }"),
        "{runtime}"
    );
    assert!(
        runtime.contains("public string? ApplicationId { get; init; }"),
        "{runtime}"
    );
    assert!(
        runtime.contains("TryAddWithoutValidation(\"User-Agent\""),
        "{runtime}"
    );
}

#[test]
fn csharp_unattributed_plans_emit_the_disabled_sentinel() {
    let contract = contract();
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = suspect_codegen::csharp_sdk::plan_sdk_with_options(
        contract,
        &selected,
        suspect_codegen::csharp_sdk::SdkConfig {
            name: "OpenRouter.CSharp".into(),
            version: "1.2.0".into(),
            namespace: "OpenRouter.CSharp".into(),
        },
        suspect_codegen::csharp_sdk::protocol::ProtocolOptions::default(),
    )
    .unwrap();
    let files = plan.render().unwrap();
    let attribution = content(&files, "csharp/src/Attribution.g.cs");
    assert!(
        attribution.contains("An empty suspect version disables the automatic attribution header"),
        "{attribution}"
    );
    assert!(
        attribution.contains("internal const string SuspectVersion = \"\";"),
        "{attribution}"
    );
    let runtime = content(&files, "csharp/src/HttpRuntime.cs");
    assert!(
        runtime.contains("Attribution.SuspectVersion.Length==0"),
        "{runtime}"
    );
}
