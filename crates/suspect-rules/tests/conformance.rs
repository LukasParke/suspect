//! Cross-language conformance: the Rust mirrors must agree with the shared
//! fixture corpus that the TS mirrors also run (`bun test` in
//! `rules-runtime`).

use serde_json::Value;
use suspect_rules::mirrors::{
    Casing, casing, defined, enum_values, is_date_time, length_between, matches, truthy,
};

#[test]
fn conformance_with_ts_mirrors() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../rules-runtime/conformance/cases.json");
    let raw =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let cases: Vec<Value> = serde_json::from_str(&raw).expect("valid cases json");

    assert!(!cases.is_empty(), "fixture corpus must not be empty");
    for case in &cases {
        let name = case["fn"].as_str().expect("fn name");
        let args: Vec<Value> = case["args"].as_array().expect("args array").clone();
        let expected = case["expected"].clone();
        let actual = match name {
            "casing" => {
                let s = args[0].as_str().expect("string arg");
                let style = Casing::parse(args[1].as_str().expect("style arg"))
                    .unwrap_or_else(|| panic!("unknown style {}", args[1]));
                Value::Bool(casing(s, style))
            }
            "defined" => Value::Bool(defined(&args[0])),
            "truthy" => Value::Bool(truthy(&args[0])),
            "lengthBetween" => Value::Bool(length_between(
                &args[0],
                args[1].as_u64().expect("min") as usize,
                args[2].as_u64().expect("max") as usize,
            )),
            "matches" => Value::Bool(matches(&args[0], args[1].as_str().expect("pattern"))),
            "isDateTime" => Value::Bool(is_date_time(&args[0])),
            "enumValues" => enum_values(&args[0]).map_or(Value::Null, |e| {
                serde_json::to_value(e).expect("serializable enum")
            }),
            other => panic!("unknown conformance fn: {other}"),
        };
        assert_eq!(
            actual, expected,
            "conformance mismatch on {name}({args:?}): rust={actual}, expected={expected}"
        );
    }
}
