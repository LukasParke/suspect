//! Pinned official equality cases through Contract and the owned public API.
//!
//! Fixture provenance and license are in `tests/conformance/README.md`.
//! The original schema/data values are embedded under an OpenAPI 3.1 component
//! to exercise the Contract boundary; unsupported or incomplete evaluation is
//! always a test failure, including cases whose expected validity is false.

use std::sync::Arc;

use serde_json::{Value, json};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_low::Pointer;
use suspect_ref::WorkspaceBuilder;
use suspect_schema::{Config, OwnedCompiler, OwnedOutcome};
use suspect_source::Uri;

fn check_fixture(name: &str, text: &str, expected_cases: usize) {
    let groups: Value = serde_json::from_str(text).expect("pinned fixture JSON");
    let mut cases = 0;
    for group in groups.as_array().expect("fixture groups") {
        let description = group["description"].as_str().expect("group description");
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("api.json");
        let document = json!({
            "openapi": "3.1.0",
            "info": {"title": "Official equality cases", "version": "1"},
            "components": {"schemas": {"Model": group["schema"]}},
        });
        std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        let workspace = Arc::new(
            WorkspaceBuilder::new()
                .root(directory.path())
                .build()
                .unwrap(),
        );
        let contract = Arc::new(
            Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap(),
        );
        let root = SchemaId::new(
            contract.entry().clone(),
            Pointer::parse("/components/schemas/Model").unwrap(),
        );
        let program = OwnedCompiler::new(Config::default())
            .compile(contract, std::slice::from_ref(&root))
            .unwrap_or_else(|errors| {
                panic!("{name} / {description}: fixture compilation failed: {errors:?}")
            });
        for case in group["tests"].as_array().expect("fixture cases") {
            cases += 1;
            let case_name = case["description"].as_str().expect("case description");
            let expected = case["valid"].as_bool().expect("expected validity");
            let actual = match program.validate(&root, &case["data"]) {
                OwnedOutcome::Valid => true,
                OwnedOutcome::Invalid(_) => false,
                OwnedOutcome::EvaluationFailure(finding) => {
                    panic!(
                        "{name} / {description} / {case_name}: incomplete evaluation: {finding:?}"
                    )
                }
            };
            assert_eq!(actual, expected, "{name} / {description} / {case_name}");
        }
    }
    assert_eq!(
        cases, expected_cases,
        "{name}: pinned case coverage changed"
    );
}

#[test]
fn official_enum_values_preserve_exact_owned_equality() {
    check_fixture(
        "enum",
        include_str!("../conformance/draft2020-12/enum.json"),
        53,
    );
}

#[test]
fn official_const_values_preserve_exact_owned_equality() {
    check_fixture(
        "const",
        include_str!("../conformance/draft2020-12/const.json"),
        54,
    );
}

#[test]
fn official_unique_items_preserve_exact_owned_equality() {
    check_fixture(
        "uniqueItems",
        include_str!("../conformance/draft2020-12/uniqueItems.json"),
        69,
    );
}
