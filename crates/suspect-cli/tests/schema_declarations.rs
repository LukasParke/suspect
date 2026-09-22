//! Declaration validation on the actual OpenRouter contracts through the CLI.

use std::path::PathBuf;
use std::process::Command;

use serde_json::{Value, json};

fn upstream(relative: &str) -> String {
    let root = std::env::var_os("OPENROUTER_WEB_ROOT")
        .map(PathBuf::from)
        .expect("set OPENROUTER_WEB_ROOT to the openrouter-web source checkout");
    std::fs::read_to_string(root.join(relative)).expect("read tracked OpenRouter specification")
}

fn validate(name: &str, source: &str) -> (i32, Vec<Value>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    std::fs::write(&path, source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_suspect"))
        .args(["validate", "--format", "json"])
        .arg(&path)
        .output()
        .unwrap();
    let findings: Vec<Value> = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)));
    assert!(findings.iter().all(|f| f["file"] == path.to_str().unwrap()));
    (output.status.code().unwrap(), findings)
}

#[test]
#[ignore = "requires the OpenRouter source checkout; run the dedicated acceptance gate"]
fn openrouter_chat_message_cardinality_mutations_are_located_errors() {
    let source = upstream("projects/docs/openapi/openapi.yaml");
    let (status, findings) = validate("openrouter.yaml", &source);
    // This tracked 3.1 snapshot contains a real upstream 3.0-style bound.
    // Record it explicitly; acceptance must never hide it or patch the input.
    let upstream_bound = source.find("          exclusiveMinimum: true\n").unwrap();
    let upstream_line = source[..upstream_bound]
        .bytes()
        .filter(|b| *b == b'\n')
        .count()
        + 1;
    let errors: Vec<_> = findings
        .iter()
        .filter(|f| f["severity"] == "Error")
        .collect();
    assert_eq!(
        status, 1,
        "tracked upstream declaration defect: {findings:?}"
    );
    assert_eq!(
        errors.len(),
        1,
        "only the recorded upstream bound defect: {findings:?}"
    );
    assert_eq!(errors[0]["code"], "oas-schema-invalid-keyword");
    assert_eq!(errors[0]["line"], upstream_line);
    assert!(
        errors[0]["message"]
            .as_str()
            .unwrap()
            .contains("exclusiveMinimum")
    );
    let chat = source.find("    ChatRequest:\n").unwrap();
    let messages = chat + source[chat..].find("        messages:\n").unwrap();
    let cardinality = messages + source[messages..].find("          minItems: 1\n").unwrap();
    let value = cardinality + "          minItems: ".len();
    let line = source[..value].bytes().filter(|b| *b == b'\n').count() + 1;
    for spelling in ["-1", "false"] {
        let mut malformed = source.clone();
        malformed.replace_range(value..value + 1, spelling);
        let (status, findings) = validate("openrouter.yaml", &malformed);
        assert_eq!(status, 1, "minItems {spelling}: {findings:?}");
        let matching: Vec<_> = findings
            .iter()
            .filter(|f| {
                f["code"] == "oas-schema-invalid-keyword"
                    && f["message"].as_str().unwrap().contains("minItems")
            })
            .collect();
        assert_eq!(matching.len(), 1, "{findings:?}");
        assert_eq!(
            findings.iter().filter(|f| f["severity"] == "Error").count(),
            2,
            "upstream defect plus mutation: {findings:?}"
        );
        assert_eq!(matching[0]["line"], line);
        assert_eq!(matching[0]["col"], "          minItems: ".len() + 1);
        assert!(
            matching[0]["message"]
                .as_str()
                .unwrap()
                .contains("minItems")
        );
    }
}

#[test]
#[ignore = "requires the OpenRouter source checkout; run the dedicated acceptance gate"]
fn provider_monitor_conditions_preserve_keyword_named_properties_and_check_actual_schemas() {
    let source = upstream("projects/docs/assets/provider-monitor-schema-v2.openapi.json");
    let (status, findings) = validate("provider.json", &source);
    assert_eq!(status, 0, "valid provider contract: {findings:?}");
    assert!(
        findings.is_empty(),
        "valid conditions and properties named type/properties/items: {findings:?}"
    );
    for (pointer, value, code, spelling) in [
        (
            "/components/schemas/ParameterDescriptor/allOf/0/if/required",
            json!([7]),
            "oas-schema-invalid-keyword",
            "7",
        ),
        (
            "/components/schemas/ParameterDescriptor/allOf/0/then/properties/items",
            Value::Null,
            "oas-schema-invalid-kind",
            "null",
        ),
    ] {
        let mut malformed: Value = serde_json::from_str(&source).unwrap();
        *malformed
            .pointer_mut(pointer)
            .expect("provider condition schema remains present") = value;
        let malformed = serde_json::to_string_pretty(&malformed).unwrap();
        let (status, findings) = validate("provider.json", &malformed);
        assert_eq!(status, 1, "{pointer}: {findings:?}");
        assert_eq!(
            findings.len(),
            1,
            "one finding per mutated source value: {findings:?}"
        );
        assert_eq!(findings[0]["code"], code);
        let line = findings[0]["line"].as_u64().unwrap() as usize;
        let col = findings[0]["col"].as_u64().unwrap() as usize;
        assert!(
            malformed.lines().nth(line - 1).unwrap()[col - 1..].starts_with(spelling),
            "diagnostic points to mutated value: {findings:?}"
        );
    }
}
