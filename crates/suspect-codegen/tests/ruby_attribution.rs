//! ua/v1 attribution compiles into every generated Ruby gem: constant parts at
//! generation time, language-version discovery in the native runtime.
#![cfg(feature = "ruby-sdk")]

use std::sync::Arc;

use serde_json::json;
use suspect_codegen::backend::{self, Backend, GenerationOptions};
use suspect_codegen::ruby_sdk::{PackageConfig, RubyConfig};
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
        backend: Backend::RubyHttp,
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
fn ruby_packages_compile_the_attribution_constants_into_the_gem() {
    let files = generate("openrouter-ruby");
    let attribution = content(&files, "lib/openrouter_ruby/attribution.rb");
    assert!(attribution.contains("ATTRIBUTION = {"), "{attribution}");
    assert!(attribution.contains("suspect_version:"), "{attribution}");
    assert!(attribution.contains("\"openrouter-ruby\""), "{attribution}");
    assert!(
        attribution.contains("sdk_version: \"1.2.0\""),
        "{attribution}"
    );
    assert!(
        attribution.contains("spec_version: \"3.1.2\""),
        "{attribution}"
    );
    assert!(attribution.contains("language: \"ruby\""), "{attribution}");
    let entry = content(&files, "lib/openrouter_ruby.rb");
    assert!(
        entry.contains("require_relative \"openrouter_ruby/attribution\""),
        "{entry}"
    );
    let runtime = content(&files, "lib/openrouter_ruby/http.rb");
    assert!(runtime.contains("resolve_user_agent"), "{runtime}");
    assert!(runtime.contains("suspect/"), "{runtime}");
    assert!(
        runtime.contains("' (ruby/' + ::RUBY_VERSION + '; openapi/'"),
        "{runtime}"
    );
    assert!(runtime.contains("application_identity?"), "{runtime}");
}

#[test]
fn ruby_clients_expose_the_user_agent_and_application_id_options() {
    let files = generate("openrouter-ruby");
    let runtime = content(&files, "lib/openrouter_ruby/http.rb");
    assert!(
        runtime.contains("user_agent: UNSET, application_id: nil"),
        "{runtime}"
    );
    assert!(runtime.contains("@user_agent"), "{runtime}");
    assert!(runtime.contains("@application_id"), "{runtime}");
    assert!(runtime.contains("name.casecmp?('user-agent')"), "{runtime}");
    let rbs = content(&files, "sig/openrouter_ruby.rbs");
    assert!(rbs.contains("?user_agent: String | nil | Unset"), "{rbs}");
    assert!(rbs.contains("?application_id: String?"), "{rbs}");
}

#[test]
fn ruby_packages_without_attribution_emit_the_disabled_sentinel() {
    let contract = contract();
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = suspect_codegen::ruby_sdk::plan_sdk(contract, &selected, RubyConfig::default())
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let files = suspect_codegen::ruby_sdk::emit_sdk(
        &plan,
        &PackageConfig {
            name: "openrouter-ruby".into(),
            version: "1.2.0".into(),
            require_name: "openrouter_ruby".into(),
            namespace: "OpenrouterRuby".into(),
        },
    )
    .unwrap();
    let attribution = content(&files, "lib/openrouter_ruby/attribution.rb");
    assert!(
        attribution.contains("An empty suspect version disables the automatic attribution header"),
        "{attribution}"
    );
    assert!(
        attribution.contains("suspect_version: \"\""),
        "{attribution}"
    );
}
