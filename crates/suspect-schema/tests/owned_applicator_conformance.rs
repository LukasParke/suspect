//! Unmodified official cases through real standalone resource roots in Contract.
//! No pointer rewriting or source-evaluator-derived expectations.
use serde_json::{Value, json};
use std::sync::Arc;
use suspect_ir::contract::{Contract, SchemaId};
use suspect_ref::WorkspaceBuilder;
use suspect_schema::{Config, OwnedCompileErrorKind, OwnedCompiler, OwnedOutcome};
use suspect_source::Uri;

fn standalone(schema: &Value) -> (Arc<Contract>, SchemaId) {
    let directory = tempfile::tempdir().unwrap();
    let entry = directory.path().join("api.json");
    let path = directory.path().join("schema.json");
    std::fs::write(&path, schema.to_string()).unwrap();
    std::fs::write(&entry,json!({"openapi":"3.1.2","info":{"title":"Official applicators","version":"1"},"paths":{},"components":{"schemas":{"Entry":{"$ref":"schema.json"}}}}).to_string()).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(directory.path())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&entry).unwrap()).unwrap());
    (
        contract,
        SchemaId::new(Uri::from_path(&path).unwrap(), Default::default()),
    )
}

fn official(name: &str, text: &str, expected_cases: usize, expected_deferred: usize) {
    let groups: Vec<Value> = serde_json::from_str(text).unwrap();
    let mut executed = 0;
    let mut deferred = 0;
    for group in groups {
        let description = group["description"].as_str().unwrap();
        let (contract, root) = standalone(&group["schema"]);
        let result = OwnedCompiler::new(Config::default())
            .compile_v2(contract.clone(), std::slice::from_ref(&root));
        // Exactly these two named groups require the separately owned canonical
        // resource/dynamic-scope API. Verify the concrete refusal, not a pass.
        if matches!(
            description,
            "unevaluatedProperties with $dynamicRef" | "unevaluatedItems with $dynamicRef"
        ) {
            let errors = result
                .err()
                .expect("dynamic resource support must not be invented");
            assert!(
                errors
                    .iter()
                    .any(|error| error.kind == OwnedCompileErrorKind::Unsupported
                        && (error.message.contains("$id") || error.message.contains("dynamic"))),
                "{errors:?}"
            );
            deferred += group["tests"].as_array().unwrap().len();
            continue;
        }
        if description == "patternProperties with Unicode property escape" {
            let errors = result.err().expect(
                "Unicode property escapes remain outside the existing portable NFA profile",
            );
            assert!(
                errors
                    .iter()
                    .any(|error| error.kind == OwnedCompileErrorKind::Unsupported
                        && error.source.pointer() == r"/patternProperties/^\p{Letter}+$"
                        && error.message.contains("Unicode properties")
                        && error.span.is_some()),
                "{errors:?}"
            );
            deferred += group["tests"].as_array().unwrap().len();
            continue;
        }
        let schema = result.unwrap_or_else(|errors| panic!("{name}/{description}: {errors:?}"));
        schema.program().check().unwrap();
        for case in group["tests"].as_array().unwrap() {
            executed += 1;
            let actual = match schema.validate(&root, &case["data"]) {
                OwnedOutcome::Valid => true,
                OwnedOutcome::Invalid(_) => false,
                OwnedOutcome::EvaluationFailure(finding) => panic!(
                    "{name}/{description}/{}: incomplete {finding:?}",
                    case["description"]
                ),
            };
            assert_eq!(
                actual,
                case["valid"].as_bool().unwrap(),
                "{name}/{description}/{}",
                case["description"]
            );
        }
    }
    assert_eq!(
        (executed, deferred),
        (expected_cases, expected_deferred),
        "{name}: coverage changed"
    );
    eprintln!(
        "{name}: {executed} official cases executed, {deferred} explicit capability deferrals"
    );
}

macro_rules! cases {
    ($test:ident,$path:literal,$count:expr,$deferred:expr) => {
        #[test]
        fn $test() {
            official(stringify!($test), include_str!($path), $count, $deferred);
        }
    };
}
cases!(
    if_then_else,
    "fixtures/applicator-conformance/if-then-else.json",
    30,
    0
);
cases!(
    dependent_required,
    "fixtures/applicator-conformance/dependentRequired.json",
    20,
    0
);
cases!(
    dependent_schemas,
    "fixtures/applicator-conformance/dependentSchemas.json",
    20,
    0
);
cases!(
    pattern_properties,
    "fixtures/applicator-conformance/patternProperties.json",
    23,
    2
);
cases!(
    property_names,
    "fixtures/applicator-conformance/propertyNames.json",
    22,
    0
);
cases!(
    additional_properties,
    "fixtures/applicator-conformance/additionalProperties.json",
    21,
    0
);
cases!(contains, "conformance/draft2020-12/contains.json", 21, 0);
cases!(
    min_contains,
    "conformance/draft2020-12/minContains.json",
    28,
    0
);
cases!(
    max_contains,
    "conformance/draft2020-12/maxContains.json",
    14,
    0
);
cases!(
    unevaluated_properties,
    "conformance/draft2020-12/unevaluatedProperties.json",
    127,
    2
);
cases!(
    unevaluated_items,
    "conformance/draft2020-12/unevaluatedItems.json",
    69,
    2
);
