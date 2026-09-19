//! Unmodified official JSON Schema vectors through the public validator API.

use suspect_low::LowDoc;
use suspect_schema::{Compiler, Config, SchemaErrorKind};
use suspect_source::{Source, Uri};
use suspect_syntax::Format;

fn check_fixture(name: &str, text: &str) {
    let doc = LowDoc::with_format(
        Uri::parse(&format!("memory://conformance/{name}.json")).unwrap(),
        Source::from_vec(text.as_bytes().to_vec()),
        Format::Json,
    );
    assert!(doc.syntax_errors().is_empty(), "{name}: fixture syntax");
    let mut failures = Vec::new();
    let mut cases = 0;
    for group in doc.root().items() {
        let description = group.get("description").unwrap();
        let description = description.as_str().unwrap();
        let schema = Compiler::new(Config::default()).compile(group.get("schema").unwrap());
        let schema = match schema {
            Ok(schema) => schema,
            Err(error) => {
                failures.push(format!("{description}: compilation failed: {error}"));
                continue;
            }
        };
        for test in group.get("tests").unwrap().items() {
            cases += 1;
            let expected = test.get("valid").unwrap().as_bool().unwrap();
            let errors = schema.validate(test.get("data").unwrap());
            if errors
                .iter()
                .any(|error| error.kind == SchemaErrorKind::Evaluation)
            {
                failures.push(format!(
                    "{description} / {}: evaluation did not complete: {errors:?}",
                    test.get("description").unwrap().as_str().unwrap()
                ));
            } else if errors.is_empty() != expected {
                failures.push(format!(
                    "{description} / {}: expected valid={expected}, errors={errors:?}",
                    test.get("description").unwrap().as_str().unwrap()
                ));
            }
        }
    }
    assert!(cases > 0, "{name}: no cases executed");
    assert!(failures.is_empty(), "{name}: {}", failures.join("\n"));
}

#[test]
#[should_panic(expected = "evaluation did not complete")]
fn incomplete_evaluation_cannot_pass_an_expected_invalid_case() {
    check_fixture(
        "incomplete-evaluation",
        r#"[{
            "description": "an unavailable resource cannot establish invalidity",
            "schema": {"$ref": "https://unavailable.test/schema.json"},
            "tests": [{"description": "unresolved", "data": null, "valid": false}]
        }]"#,
    );
}

macro_rules! suite {
    ($test:ident, $file:literal) => {
        #[test]
        fn $test() {
            check_fixture(
                $file,
                include_str!(concat!("conformance/draft2020-12/", $file, ".json")),
            );
        }
    };
}

suite!(draft202012_min_items, "minItems");
suite!(draft202012_max_items, "maxItems");
suite!(draft202012_min_properties, "minProperties");
suite!(draft202012_max_properties, "maxProperties");
suite!(draft202012_contains, "contains");
suite!(draft202012_min_contains, "minContains");
suite!(draft202012_max_contains, "maxContains");

suite!(draft202012_maximum, "maximum");
suite!(draft202012_minimum, "minimum");
suite!(draft202012_exclusive_maximum, "exclusiveMaximum");
suite!(draft202012_exclusive_minimum, "exclusiveMinimum");
suite!(draft202012_multiple_of, "multipleOf");
suite!(draft202012_enum, "enum");
suite!(draft202012_const, "const");
suite!(draft202012_unique_items, "uniqueItems");
suite!(draft202012_type, "type");

suite!(draft202012_min_length, "minLength");
suite!(draft202012_max_length, "maxLength");
suite!(draft202012_unevaluated_properties, "unevaluatedProperties");
suite!(draft202012_unevaluated_items, "unevaluatedItems");
