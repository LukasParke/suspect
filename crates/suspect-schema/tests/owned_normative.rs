//! Run the complete pinned static-keyword files as an independent oracle.
//! These are the original JSON Schema Test Suite fixtures, not generated from
//! the owned compiler. Revision/license: tests/conformance/README.md.

use std::sync::Arc;

use serde_json::{Value, json};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_low::Pointer;
use suspect_ref::WorkspaceBuilder;
use suspect_schema::{Config, OwnedCompiler, OwnedOutcome};
use suspect_source::Uri;

const FILES: &[(&str, &str)] = &[
    ("type", include_str!("conformance/draft2020-12/type.json")),
    (
        "minimum",
        include_str!("conformance/draft2020-12/minimum.json"),
    ),
    (
        "maximum",
        include_str!("conformance/draft2020-12/maximum.json"),
    ),
    (
        "exclusiveMinimum",
        include_str!("conformance/draft2020-12/exclusiveMinimum.json"),
    ),
    (
        "exclusiveMaximum",
        include_str!("conformance/draft2020-12/exclusiveMaximum.json"),
    ),
    (
        "multipleOf",
        include_str!("conformance/draft2020-12/multipleOf.json"),
    ),
    (
        "minLength",
        include_str!("conformance/draft2020-12/minLength.json"),
    ),
    (
        "maxLength",
        include_str!("conformance/draft2020-12/maxLength.json"),
    ),
    (
        "minItems",
        include_str!("conformance/draft2020-12/minItems.json"),
    ),
    (
        "maxItems",
        include_str!("conformance/draft2020-12/maxItems.json"),
    ),
    (
        "minProperties",
        include_str!("conformance/draft2020-12/minProperties.json"),
    ),
    (
        "maxProperties",
        include_str!("conformance/draft2020-12/maxProperties.json"),
    ),
    ("enum", include_str!("conformance/draft2020-12/enum.json")),
    ("const", include_str!("conformance/draft2020-12/const.json")),
    (
        "uniqueItems",
        include_str!("conformance/draft2020-12/uniqueItems.json"),
    ),
];

fn run(version: &str, explicit_json_schema: bool) {
    let mut cases = 0;
    for (file, text) in FILES {
        let groups: Vec<Value> = serde_json::from_str(text).unwrap();
        for group in groups {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("api.json");
            let mut document = json!({
                "openapi":version,"info":{"title":"Normative vectors","version":"1"},"paths":{},
                "components":{"schemas":{"Model":group["schema"]}}
            });
            if explicit_json_schema {
                document["jsonSchemaDialect"] =
                    json!("https://json-schema.org/draft/2020-12/schema");
            }
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
            let validator = OwnedCompiler::new(Config::default())
                .compile(contract, std::slice::from_ref(&root))
                .unwrap_or_else(|errors| {
                    panic!("{version}/{file}/{}: {errors:?}", group["description"])
                });
            validator.program().check().unwrap();
            for case in group["tests"].as_array().unwrap() {
                cases += 1;
                let actual = match validator.validate(&root, &case["data"]) {
                    OwnedOutcome::Valid => true,
                    OwnedOutcome::Invalid(_) => false,
                    OwnedOutcome::EvaluationFailure(finding) => {
                        panic!("{version}/{file}/{}: {finding:?}", case["description"])
                    }
                };
                assert_eq!(
                    actual,
                    case["valid"].as_bool().unwrap(),
                    "{version}/{file}/{}/{}",
                    group["description"],
                    case["description"]
                );
            }
        }
    }
    eprintln!(
        "{version}, explicit JSON Schema dialect={explicit_json_schema}: {cases} normative cases across {} unmodified files",
        FILES.len()
    );
}

#[test]
fn oas31_standard_dialect_runs_all_pinned_static_cases() {
    run("3.1.2", false);
}

#[test]
fn oas32_standard_dialect_runs_all_pinned_static_cases() {
    run("3.2.0", false);
}

#[test]
fn json_schema_202012_explicit_dialect_runs_all_pinned_static_cases() {
    run("3.1.2", true);
}
