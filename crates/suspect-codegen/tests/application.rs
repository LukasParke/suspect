//! Source-addressed operation selector: exact operationId, exact
//! "METHOD /path", ambiguity refusal, and missing-operation refusal.
#![cfg(feature = "http-protocol")]

use std::{path::Path, sync::Arc};

use suspect_codegen::application::{Diagnostic, select_operation};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn contract() -> Arc<Contract> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/application-v1");
    let workspace = Arc::new(WorkspaceBuilder::new().root(&root).build().unwrap());
    Arc::new(
        Contract::from_workspace(
            &workspace,
            &Uri::from_path(&root.join("openapi.json")).unwrap(),
        )
        .unwrap(),
    )
}

#[test]
fn exact_operation_id_resolves() {
    let contract = contract();
    let op = select_operation(&contract, "/selector", "getWidget").unwrap();
    assert_eq!(op.operation_id(), Some("getWidget"));
    assert_eq!(op.path_template(), Some("/widgets/{id}"));
}

#[test]
fn exact_method_and_path_resolves() {
    let contract = contract();
    let op = select_operation(&contract, "/selector", "GET /widgets/{id}").unwrap();
    assert_eq!(op.operation_id(), Some("getWidget"));
}

#[test]
fn method_path_selector_is_case_sensitive_and_exact() {
    let contract = contract();
    assert!(select_operation(&contract, "/selector", "get /widgets/{id}").is_err());
    assert!(select_operation(&contract, "/selector", "GET /widgets").is_err());
}

#[test]
fn ambiguous_operation_id_is_refused() {
    let contract = contract();
    let error = select_operation(&contract, "/selector", "dup").unwrap_err();
    assert_eq!(error.code, "application-operation-ambiguous");
    assert_eq!(error.mapping_pointer, "/selector");
    assert!(error.message.contains("found 2"));
}

#[test]
fn missing_operation_id_is_refused() {
    let contract = contract();
    let error = select_operation(&contract, "/selector", "missing").unwrap_err();
    assert_eq!(error.code, "application-operation-missing");
    assert_eq!(error.mapping_pointer, "/selector");
}

#[test]
fn missing_method_path_is_refused() {
    let contract = contract();
    let error = select_operation(&contract, "/selector", "POST /nowhere").unwrap_err();
    assert_eq!(error.code, "application-operation-missing");
}

#[test]
fn diagnostic_constructor_locates_source_and_span() {
    let contract = contract();
    let source = contract.operations().next().unwrap().source().clone();
    let diagnostic = Diagnostic::new(
        &contract,
        source.clone(),
        "/selector",
        "application-test",
        "message",
    );
    assert_eq!(diagnostic.source, source);
    assert_eq!(diagnostic.mapping_pointer, "/selector");
    assert_eq!(diagnostic.code, "application-test");
    assert_eq!(diagnostic.message, "message");
    assert!(diagnostic.at.end > diagnostic.at.start);
}
