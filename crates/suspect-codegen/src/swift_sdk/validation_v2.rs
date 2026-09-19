//! Native evaluated-applicator vectors. Expected outcomes are literal normative
//! cases, never obtained by asking the Rust evaluator to validate the instance.
use super::*;
use crate::swift_sdk::validation_v2_support as support;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};
use suspect_schema::{Config, OwnedCompiler};

fn cases() -> Vec<Value> {
    let corpus: Value = serde_json::from_str(include_str!(
        "../../../suspect-schema/tests/fixtures/owned-applicators-v2.json"
    ))
    .unwrap();
    let mut cases = corpus["cases"].as_array().unwrap().clone();
    for (id, schema, instance, expected, source) in [
        (
            "if-does-not-seed-then",
            r#"{"if":{"properties":{"a":true}},"then":{"unevaluatedProperties":false}}"#,
            r#"{"a":1}"#,
            "Invalid",
            "/then/unevaluatedProperties",
        ),
        (
            "if-false-condition-does-not-leak",
            r#"{"if":{"properties":{"a":true},"required":["switch"]},"else":true,"unevaluatedProperties":false}"#,
            r#"{"a":1}"#,
            "Invalid",
            "/unevaluatedProperties",
        ),
        (
            "all-passing-anyof-marks-merge",
            r#"{"anyOf":[{"properties":{"a":true}},{"properties":{"b":true}}],"unevaluatedProperties":false}"#,
            r#"{"a":1,"b":2}"#,
            "Valid",
            "",
        ),
        (
            "ref-does-not-see-parent-marks",
            r##"{"$defs":{"T":{"unevaluatedProperties":false}},"properties":{"a":true},"$ref":"#/components/schemas/Root/$defs/T"}"##,
            r#"{"a":1}"#,
            "Invalid",
            "/$defs/T/unevaluatedProperties",
        ),
        (
            "dependency-schemas-are-isolated",
            r#"{"dependentSchemas":{"a":{"properties":{"a":true,"b":true}},"b":{"unevaluatedProperties":false}}}"#,
            r#"{"a":1,"b":2}"#,
            "Invalid",
            "/dependentSchemas/b/unevaluatedProperties",
        ),
        (
            "unicode-trigger-identity",
            r#"{"dependentRequired":{"é":["peer"]}}"#,
            r#"{"e\u0301":null}"#,
            "Valid",
            "",
        ),
        (
            "unicode-trigger-null-is-present",
            r#"{"dependentRequired":{"é":["peer"]}}"#,
            r#"{"\u00e9":null}"#,
            "Invalid",
            "/dependentRequired/é",
        ),
        (
            "unicode-name-scalars",
            r#"{"propertyNames":{"maxLength":1},"additionalProperties":true}"#,
            r#"{"é":null}"#,
            "Valid",
            "",
        ),
        (
            "unicode-name-is-not-normalized",
            r#"{"propertyNames":{"maxLength":1},"additionalProperties":true}"#,
            r#"{"e\u0301":null}"#,
            "Invalid",
            "/propertyNames/maxLength",
        ),
        (
            "contains-exact-equality",
            r#"{"contains":{"const":9007199254740993},"minContains":1,"maxContains":1,"unevaluatedItems":{"type":"string"}}"#,
            r#"[9007199254740993.000,"ok"]"#,
            "Valid",
            "",
        ),
        (
            "contains-no-float-rounding",
            r#"{"contains":{"const":9007199254740993},"maxContains":1,"unevaluatedItems":{"type":"string"}}"#,
            r#"[9007199254740992.0]"#,
            "Invalid",
            "/contains",
        ),
        (
            "contains-huge-cardinality",
            r#"{"contains":true,"maxContains":1e99999999999999999999999999}"#,
            r#"[1,2,3]"#,
            "Valid",
            "",
        ),
        (
            "contains-huge-minimum",
            r#"{"contains":true,"minContains":1e99999999999999999999999999}"#,
            r#"[1]"#,
            "Invalid",
            "/minContains",
        ),
        (
            "adjacent-patterns-only",
            r#"{"allOf":[{"patternProperties":{"^x":true}}],"additionalProperties":false}"#,
            r#"{"x":1}"#,
            "Invalid",
            "/additionalProperties",
        ),
        (
            "pattern-name-pointer-escaping",
            r#"{"patternProperties":{"^a/b~":{"type":"integer"}},"additionalProperties":false}"#,
            r#"{"a/b~x":"no"}"#,
            "Invalid",
            "/patternProperties/^a~1b~0/type",
        ),
        (
            "unevaluated-child-array-not-parent",
            r#"{"prefixItems":[{"prefixItems":[true],"unevaluatedItems":false}],"unevaluatedItems":false}"#,
            r#"[[1],2]"#,
            "Invalid",
            "/unevaluatedItems",
        ),
        (
            "contains-true-marks-every-index",
            r#"{"contains":true,"unevaluatedItems":false}"#,
            r#"[null,{},"x"]"#,
            "Valid",
            "",
        ),
        (
            "contains-zero-empty-array",
            r#"{"contains":false,"minContains":0,"unevaluatedItems":false}"#,
            "[]",
            "Valid",
            "",
        ),
        (
            "recursive-condition-is-failure",
            r##"{"if":{"$ref":"#/components/schemas/Root"},"then":true,"else":true}"##,
            "{}",
            "EvaluationFailure",
            "",
        ),
    ] {
        cases.push(json!({"id":id,"schemaJson":schema,"instanceJson":instance,"expected":expected,"sourceSuffix":source}));
    }
    cases.push(json!({"id":"zero-equality-condition","schemaJson":r#"{"if":{"const":"x"},"then":true,"else":true}"#,"instanceJson":r#""x""#,"expected":"EvaluationFailure","limits":{"maxEqualitySteps":0},"sourceSuffix":"/if/const"}));
    cases.push(json!({"id":"pattern-budget-not-hidden-by-anyof","schemaJson":r#"{"anyOf":[true,{"patternProperties":{"x+$":{"type":"integer"}}}]}"#,"instanceJson":format!("{{\"{}\":1}}", "x".repeat(4096)),"expected":"EvaluationFailure","limits":{"maxEvaluationSteps":64}}));
    cases
}

#[test]
fn checked_emission_declines_malformed_profiles_at_sources() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("api.json");
    std::fs::write(
        &input,
        json!({"openapi":"3.1.0","info":{"title":"admission","version":"1"},"paths":{},
        "components":{"schemas":{"Root":{"dependentRequired":{"a":["b"]}}}}})
        .to_string(),
    )
    .unwrap();
    let workspace = Arc::new(
        suspect_ref::WorkspaceBuilder::new()
            .root(root.path())
            .build()
            .unwrap(),
    );
    let contract = Arc::new(
        Contract::from_workspace(&workspace, &suspect_source::Uri::from_path(&input).unwrap())
            .unwrap(),
    );
    let mut program = OwnedCompiler::new(Config::default())
        .compile_v2(contract.clone(), contract.schema_roots())
        .unwrap()
        .program();
    assert!(emit(&contract, &program).is_ok());
    program.version = OwnedProgram::V1_VERSION;
    program.profile = OwnedProgram::V1_PROFILE;
    let findings = emit(&contract, &program).unwrap_err();
    assert_eq!(
        findings[0].source.pointer(),
        "/components/schemas/Root/dependentRequired"
    );
    assert!(findings[0].at.end > findings[0].at.start);
    program.version = OwnedProgram::V3_VERSION;
    program.profile = OwnedProgram::V3_PROFILE;
    let findings = emit(&contract, &program).unwrap_err();
    assert_eq!(findings[0].code, "swift-validation-program-invalid");
    assert_eq!(findings[0].source.pointer(), "/components/schemas/Root");
}

#[test]
#[ignore = "Swift v2 runtime oracle, scoped annotations and finite evaluation budgets on an installed toolchain"]
fn native_evaluated_applicator_vectors() {
    let root = support::root("runtime-v2-");
    let cases = cases();
    let mut groups: BTreeMap<String, Vec<(usize, &Value)>> = BTreeMap::new();
    for (index, case) in cases.iter().enumerate() {
        groups
            .entry(
                case.get("limits")
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "{}".into()),
            )
            .or_default()
            .push((index, case));
    }
    let mut module_names = Vec::new();
    let mut imports = String::new();
    let mut tests = String::from("\nfinal class ApplicatorVectors: XCTestCase {\n");
    for (group_index, (limits, entries)) in groups.into_iter().enumerate() {
        let module = format!("RuntimeV2Group{group_index}");
        module_names.push(module.clone());
        let mut schemas = serde_json::Map::new();
        for (index, case) in &entries {
            let schema = case["schemaJson"].as_str().unwrap().replace(
                "#/components/schemas/Root",
                &format!("#/components/schemas/Case{index}"),
            );
            schemas.insert(
                format!("Case{index}"),
                serde_json::from_str(&schema).unwrap(),
            );
        }
        let spec = root.join(format!("{module}.json"));
        std::fs::write(&spec, json!({"openapi":"3.1.0","info":{"title":"Swift v2 independent oracle","version":"1"},"paths":{},"components":{"schemas":schemas}}).to_string()).unwrap();
        let workspace = Arc::new(
            suspect_ref::WorkspaceBuilder::new()
                .root(&root)
                .build()
                .unwrap(),
        );
        let contract = Arc::new(
            Contract::from_workspace(&workspace, &suspect_source::Uri::from_path(&spec).unwrap())
                .unwrap(),
        );
        let mut config = Config {
            max_depth: 512,
            ..Default::default()
        };
        let limits: Value = serde_json::from_str(&limits).unwrap();
        if let Some(n) = limits["maxNumberBytes"].as_u64() {
            config.max_number_bytes = n as usize;
        }
        if let Some(n) = limits["maxEvaluationSteps"].as_u64() {
            config.max_evaluation_steps = n as usize;
        }
        if let Some(n) = limits["maxEqualitySteps"].as_u64() {
            config.max_equality_steps = n as usize;
        }
        let program = OwnedCompiler::new(config)
            .compile_v2(contract.clone(), contract.schema_roots())
            .unwrap()
            .program();
        program.check().unwrap();
        assert_eq!(program.version, OwnedProgram::V2_VERSION);
        let src = root.join("Sources").join(&module);
        std::fs::create_dir_all(&src).unwrap();
        for (name, text) in [
            ("Json.swift", include_str!("json.swift")),
            ("Number.swift", include_str!("number.swift")),
            ("Validation.swift", include_str!("validation.swift")),
        ] {
            std::fs::write(src.join(name), text).unwrap();
        }
        std::fs::write(
            src.join("Program.swift"),
            emit(&contract, &program).unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join(format!("{module}-program.json")),
            serde_json::to_string_pretty(&program).unwrap(),
        )
        .unwrap();
        let _ = writeln!(imports, "@testable import {module}");
        for (index, case) in entries {
            let target = program
                .roots
                .iter()
                .find(|r| r.source.pointer == format!("/components/schemas/Case{index}"))
                .unwrap()
                .target;
            let call = format!(
                "{module}.ValidationSession().check({target}, {module}.JsonValue.parse({}))",
                string(case["instanceJson"].as_str().unwrap())
            );
            let _ = writeln!(
                tests,
                "    func testCase{index}() throws {{ // {}",
                case["id"].as_str().unwrap()
            );
            if case["expected"] == "Valid" {
                let _ = writeln!(tests, "        XCTAssertNoThrow(try {call})");
            } else {
                let kind = if case["expected"] == "Invalid" {
                    "invalid"
                } else {
                    "evaluationFailure"
                };
                let _ = writeln!(
                    tests,
                    "        XCTAssertThrowsError(try {call}) {{ error in\n            guard let failure = error as? {module}.ValidationError else {{ return XCTFail(\"wrong failure type\") }}\n            XCTAssertEqual(failure.kind, .{kind})"
                );
                let source = case["source"]
                    .as_str()
                    .map(|s| {
                        s.replace(
                            "/components/schemas/Root",
                            &format!("/components/schemas/Case{index}"),
                        )
                    })
                    .or_else(|| {
                        case["sourceSuffix"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .map(|s| format!("/components/schemas/Case{index}{s}"))
                    });
                if let Some(source) = source {
                    let _ = writeln!(
                        tests,
                        "            XCTAssertEqual(failure.source.pointer, {})",
                        string(&source)
                    );
                }
                tests.push_str("        }\n");
            }
            tests.push_str("    }\n");
            if case["id"] == "recursive-ref-annotations-with-instance-progress" {
                let _ = writeln!(
                    tests,
                    "    func testNormalStackDepthBudget() throws {{\n        var value = {module}.JsonValue.object(.init())\n        for _ in 0..<1024 {{ var object = {module}.JsonObject<{module}.JsonValue>(); object[\"next\"] = value; value = .object(object) }}\n        let instance = value\n        let result = CapturedOutcome()\n        let done = DispatchSemaphore(value: 0)\n        let thread = Thread {{\n            defer {{ done.signal() }}\n            do {{ try {module}.ValidationSession().check({target}, instance); result.set(\"accepted\") }}\n            catch let error as {module}.ValidationError {{ result.set(error.kind.rawValue) }}\n            catch {{ result.set(\"wrong error\") }}\n        }}\n        thread.stackSize = 2 * 1024 * 1024\n        thread.start()\n        XCTAssertEqual(done.wait(timeout: .now() + 10), .success)\n        XCTAssertEqual(result.get(), \"evaluationFailure\")\n    }}"
                );
            }
        }
    }
    tests.push_str("}\n");
    std::fs::create_dir_all(root.join("Tests/Vectors")).unwrap();
    std::fs::write(
        root.join("Tests/Vectors/Vectors.swift"),
        format!("import Foundation\nimport XCTest\n{imports}\nprivate final class CapturedOutcome: @unchecked Sendable {{\n    private let lock = NSLock()\n    private var value = \"unfinished\"\n    func set(_ value: String) {{ lock.lock(); self.value = value; lock.unlock() }}\n    func get() -> String {{ lock.lock(); defer {{ lock.unlock() }}; return value }}\n}}\n{tests}"),
    )
    .unwrap();
    let mut targets = module_names
        .iter()
        .map(|m| {
            format!(
                ".target(name: {}, swiftSettings: [.swiftLanguageMode(.v6)])",
                string(m)
            )
        })
        .collect::<Vec<_>>();
    targets.push(format!(".testTarget(name: \"Vectors\", dependencies: [{}], swiftSettings: [.swiftLanguageMode(.v6)])",module_names.iter().map(|m|string(m)).collect::<Vec<_>>().join(", ")));
    std::fs::write(root.join("Package.swift"),format!("// swift-tools-version: 6.0\nimport PackageDescription\nlet package = Package(name: \"RuntimeV2\", platforms: [.macOS(.v13)], targets: [{}])\n",targets.join(", "))).unwrap();
    support::checked(
        support::swift("test")
            .arg("--package-path")
            .arg(&root)
            .arg("--scratch-path")
            .arg(root.join("build/runtime"))
            .args(["-Xswiftc", "-warnings-as-errors"]),
        &root,
    );
    println!(
        "Swift v2 scoped runtime: {} literal cases passed at {}",
        cases.len(),
        root.display()
    );
}
