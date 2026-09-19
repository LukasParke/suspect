//! Checked OwnedProgram -> immutable Swift instructions. No raw-schema walk.
use std::fmt::Write as _;

use serde_json::Value;
use suspect_ir::contract::{Contract, SchemaId, SourceId};
use suspect_schema::{
    OwnedProgram, PatternState, ProgramInstruction, ProgramResourceContext, ProgramSource,
};

use super::{HttpDiagnostic, diagnostic};

pub(super) fn admit(
    contract: &Contract,
    program: &OwnedProgram,
) -> Result<(), Vec<HttpDiagnostic>> {
    let locate = |at| location(contract, at);
    if !matches!(
        (program.version, program.profile),
        (OwnedProgram::V1_VERSION, OwnedProgram::V1_PROFILE)
            | (OwnedProgram::V2_VERSION, OwnedProgram::V2_PROFILE)
            | (OwnedProgram::V3_VERSION, OwnedProgram::V3_PROFILE)
    ) {
        let at = program
            .nodes
            .iter()
            .flat_map(|n| &n.checks)
            .find(|c| matches!(c.instruction, ProgramInstruction::DynamicRef { .. }))
            .map(|c| &c.source)
            .or_else(|| program.roots.first().map(|r| &r.source));
        return Err(vec![diagnostic(
            contract,
            locate(at),
            "swift-validation-profile-unsupported",
            "Swift requires an exact checked v1, v2 static-applicator, or v3 resource/dynamic profile pair",
        )]);
    }
    if let Err(error) = program.check() {
        return Err(vec![diagnostic(
            contract,
            locate(
                error
                    .source
                    .as_ref()
                    .or_else(|| program.roots.first().map(|root| &root.source)),
            ),
            "swift-validation-program-invalid",
            error.message,
        )]);
    }
    let limits = &program.limits;
    if limits.max_depth > 512
        || limits.max_number_bytes > 65_536
        || [
            limits.max_errors,
            limits.max_equality_steps,
            limits.max_evaluation_steps,
        ]
        .into_iter()
        .any(|v| v > i32::MAX as usize)
    {
        return Err(vec![diagnostic(
            contract,
            locate(program.roots.first().map(|r| &r.source)),
            "swift-validation-resource-policy",
            "Swift validation limits require depth <= 512, number bytes <= 65536, and counters <= Int32.max",
        )]);
    }
    Ok(())
}

fn location(contract: &Contract, at: Option<&ProgramSource>) -> SourceId {
    at.and_then(|at| {
        contract
            .schemas()
            .find(|schema| schema.id().document().as_str() == at.document)
            .map(|schema| {
                at.pointer.split('/').skip(1).fold(
                    SourceId::new(schema.id().document().clone(), Default::default()),
                    |source, token| source.child(&token.replace("~1", "/").replace("~0", "~")),
                )
            })
    })
    .unwrap_or_else(|| SourceId::new(contract.entry().clone(), Default::default()))
}

pub(super) fn index(program: &OwnedProgram, id: &SchemaId) -> usize {
    program
        .nodes
        .iter()
        .position(|node| {
            node.source.document == id.document().as_str() && node.source.pointer == id.pointer()
        })
        .expect("planned codec source in OwnedProgram")
}

/// A Swift literal, not a JSON literal: Swift's escapes and interpolation are
/// different. Every string from the contract passes through this boundary.
pub(super) fn string(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() || matches!(ch, '\u{2028}' | '\u{2029}') => {
                let _ = write!(out, "\\u{{{:x}}}", ch as u32);
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

pub(super) fn source(source: &ProgramSource) -> String {
    format!(
        "SourceLocation(document: {}, pointer: {})",
        string(&source.document),
        string(&source.pointer)
    )
}

fn list<T>(values: &[T], mut render: impl FnMut(&T) -> String) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(&mut render)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

pub(super) fn json(value: &Value) -> String {
    match value {
        Value::Null => ".null".into(),
        Value::Bool(b) => format!(".bool({b})"),
        Value::Number(n) => format!(".number(JsonNumber(trusted: {}))", string(&n.to_string())),
        Value::String(s) => format!(".string({})", string(s)),
        Value::Array(xs) => format!(".array({})", list(xs, json)),
        Value::Object(map) => format!(
            ".object(JsonObject(trusted: [{}]))",
            map.iter()
                .map(|(k, v)| format!("({}, {})", string(k), json(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

pub(super) fn emit(
    contract: &Contract,
    program: &OwnedProgram,
) -> Result<String, Vec<HttpDiagnostic>> {
    admit(contract, program)?;
    let mut out = String::from(
        "// Generated from a checked OwnedProgram; source schemas are not interpreted here.\nimport Foundation\n\nenum ValidationProgram {\n",
    );
    let l = &program.limits;
    let _ = writeln!(
        out,
        "    static let version = {}\n    static let profile = {}\n    static let collectAnnotations = {}",
        string(program.version),
        string(program.profile),
        program.version != OwnedProgram::V1_VERSION
    );
    let _ = writeln!(
        out,
        "    static let maxDepth = {}\n    static let maxErrors = {}\n    static let maxNumberBytes = {}\n    static let maxEqualitySteps = {}\n    static let maxEvaluationSteps = {}",
        l.max_depth, l.max_errors, l.max_number_bytes, l.max_equality_steps, l.max_evaluation_steps
    );
    let _ = writeln!(
        out,
        "    static let nodes: [ValidationNode] = [{}]",
        (0..program.nodes.len())
            .map(|i| format!("node{i}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    emit_resources(&mut out, program.resource_context.as_ref());
    for (i, node) in program.nodes.iter().enumerate() {
        let _ = writeln!(
            out,
            "    static let node{i} = ValidationNode(source: {}, checks: [",
            source(&node.source)
        );
        for check in &node.checks {
            let encoded = instruction(&check.instruction).map_err(|message| {
                vec![diagnostic(
                    contract,
                    location(contract, Some(&check.source)),
                    "swift-validation-instruction-unsupported",
                    message,
                )]
            })?;
            let _ = writeln!(
                out,
                "        ValidationCheck(source: {}, instruction: {}),",
                source(&check.source),
                encoded
            );
        }
        out.push_str("    ])\n");
    }
    out.push_str("}\n");
    Ok(out)
}

fn emit_resources(out: &mut String, context: Option<&ProgramResourceContext>) {
    let Some(context) = context else {
        out.push_str("    static let resourceContext: ValidationResourceContext? = nil\n");
        return;
    };
    let _ = writeln!(
        out,
        "    static let resourceContext: ValidationResourceContext? = ValidationResourceContext(resources: [{}], nodeScopes: [{}])",
        (0..context.resources.len())
            .map(|i| format!("resource{i}"))
            .collect::<Vec<_>>()
            .join(", "),
        (0..context.node_scopes.len())
            .map(|i| format!("scope{i}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    for (index, resource) in context.resources.iter().enumerate() {
        let _ = writeln!(
            out,
            "    static let resource{index} = ValidationResource(source: {}, kind: {}, canonicalURI: {}, baseURI: {}, aliases: {}, declarationSource: {}, dynamicAnchors: {})",
            source(&resource.source),
            string(resource.kind),
            string(&resource.canonical_uri),
            string(&resource.base_uri),
            list(&resource.aliases, |a| string(a)),
            resource
                .declaration_source
                .as_ref()
                .map(source)
                .unwrap_or_else(|| "nil".into()),
            list(&resource.dynamic_anchors, |(name, at, target)| format!(
                "({}, {}, {target})",
                string(name),
                source(at)
            ))
        );
    }
    for (index, (resource, root, address)) in context.node_scopes.iter().enumerate() {
        let _ = writeln!(
            out,
            "    static let scope{index} = ValidationNodeScope(resource: {resource}, schemaRoot: {}, canonicalAddress: {})",
            source(root),
            string(address)
        );
    }
}

fn instruction(op: &ProgramInstruction) -> Result<String, &'static str> {
    use ProgramInstruction as I;
    Ok(match op {
        I::DynamicRef {
            target,
            initial_resource,
            anchor,
        } => format!(
            ".dynamicReference(target: {target}, initialResource: {initial_resource}, anchor: {})",
            anchor
                .as_deref()
                .map(string)
                .unwrap_or_else(|| "nil".into())
        ),
        I::Always { value } => format!(".always({value})"),
        I::Type { types } => format!(
            ".type({})",
            list(types, |t| string(
                serde_json::to_value(t).unwrap().as_str().unwrap()
            ))
        ),
        I::Ref { target } => format!(".reference({target})"),
        I::Properties { properties } => format!(
            ".properties([{}])",
            properties
                .iter()
                .map(|p| format!("({}, {})", string(&p.name), p.target))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        I::AdditionalProperties { declared, target } => format!(
            ".additionalProperties(declared: {}, target: {target})",
            list(declared, |s| string(s))
        ),
        I::Required { names } => format!(".required({})", list(names, |s| string(s))),
        I::Items { target, start } => format!(".items(target: {target}, start: {start})"),
        I::PrefixItems { targets } => {
            format!(".prefixItems({})", list(targets, ToString::to_string))
        }
        I::AllOf { targets } => format!(".allOf({})", list(targets, ToString::to_string)),
        I::AnyOf { targets } => format!(".anyOf({})", list(targets, ToString::to_string)),
        I::OneOf { targets } => format!(".oneOf({})", list(targets, ToString::to_string)),
        I::Not { target } => format!(".not({target})"),
        I::Bound {
            value,
            maximum,
            exclusive,
        } => format!(
            ".bound(value: {}, maximum: {maximum}, exclusive: {exclusive})",
            string(value)
        ),
        I::MultipleOf { value } => format!(".multipleOf({})", string(value)),
        I::Count {
            value,
            maximum,
            target,
        } => format!(
            ".count(value: {}, maximum: {maximum}, target: {})",
            string(value),
            string(serde_json::to_value(target).unwrap().as_str().unwrap())
        ),
        I::Enum { values } => format!(".enumeration({})", list(values, json)),
        I::Const { value } => format!(".constant({})", json(value)),
        I::UniqueItems => ".uniqueItems".into(),
        I::Pattern { program } => {
            format!(".pattern({})", pattern(program))
        }
        I::If {
            condition,
            then_target,
            else_target,
        } => format!(
            ".conditional(condition: {condition}, then: {}, otherwise: {})",
            then_target.map_or_else(|| "nil".into(), |v| v.to_string()),
            else_target.map_or_else(|| "nil".into(), |v| v.to_string())
        ),
        I::DependentRequired { dependencies } => format!(
            ".dependentRequired({})",
            list(dependencies, |(name, names)| format!(
                "({}, {})",
                string(name),
                list(names, |n| string(n))
            ))
        ),
        I::DependentSchemas { dependencies } => format!(
            ".dependentSchemas({})",
            list(dependencies, |p| format!(
                "({}, {})",
                string(&p.name),
                p.target
            ))
        ),
        I::Contains {
            target,
            minimum,
            maximum,
        } => format!(
            ".contains(target: {target}, minimum: {}, maximum: {})",
            minimum
                .as_deref()
                .map(string)
                .unwrap_or_else(|| "nil".into()),
            maximum
                .as_deref()
                .map(string)
                .unwrap_or_else(|| "nil".into())
        ),
        I::PatternProperties { patterns } => format!(
            ".patternProperties({})",
            list(patterns, |(name, program, target)| format!(
                "({}, {}, {target})",
                string(name),
                pattern(program)
            ))
        ),
        I::AdditionalPropertiesWithPatterns { declared, target } => format!(
            ".additionalPropertiesWithPatterns(declared: {}, target: {target})",
            list(declared, |n| string(n))
        ),
        I::PropertyNames { target } => format!(".propertyNames({target})"),
        I::UnevaluatedProperties { target } => format!(".unevaluatedProperties({target})"),
        I::UnevaluatedItems { target } => format!(".unevaluatedItems({target})"),
    })
}

fn pattern(program: &suspect_schema::PatternProgram) -> String {
    let states = list(&program.states, |s| match s {
        PatternState::Match => ".match".into(),
        PatternState::Char { ranges, target } => format!(
            ".char(ranges: {}, target: {target})",
            list(ranges, |r| format!("({}, {})", r[0], r[1]))
        ),
        PatternState::Split { first, second } => format!(".split({first}, {second})"),
        PatternState::Jump { target } => format!(".jump({target})"),
        PatternState::Start { target } => format!(".start({target})"),
        PatternState::End { target } => format!(".end({target})"),
    });
    format!("PatternProgram(start: {}, states: {states})", program.start)
}

#[cfg(test)]
#[path = "validation_v2.rs"]
mod v2_tests;

#[cfg(test)]
#[path = "validation_v3.rs"]
mod v3_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;
    use std::{path::Path, process::Command, sync::Arc};
    use suspect_schema::{Config, OwnedCompiler};

    /// Executes the same portable-program corpus as the Wave A runtimes. This
    /// tests validation independently of the narrower native model admission.
    #[test]
    #[ignore = "requires Swift 6; native shared validation vectors and adversarial budgets"]
    fn native_shared_runtime_contract_vectors() {
        let corpus: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/runtime-contract-v1.json"
        ))
        .unwrap();
        let cases = corpus["cases"].as_array().unwrap();
        let mut schemas = Map::new();
        for (i, case) in cases.iter().enumerate() {
            schemas.insert(format!("V{i:02}"), case["schema"].clone());
        }
        schemas.insert(
            "Cycle".into(),
            serde_json::json!({"$ref":"#/components/schemas/Cycle"}),
        );
        schemas.insert(
            "FailureInUnion".into(),
            serde_json::json!({"anyOf":[true,{"$ref":"#/components/schemas/Cycle"}]}),
        );
        schemas.insert(
            "FailureInNot".into(),
            serde_json::json!({"not":{"$ref":"#/components/schemas/Cycle"}}),
        );
        schemas.insert(
            "HugeCount".into(),
            serde_json::from_str(r#"{"type":"string","maxLength":1e9999999999999999999999}"#)
                .unwrap(),
        );
        schemas.insert("UnicodeEquality".into(), serde_json::json!({"const":"é"}));
        let staging = std::env::temp_dir().join("opencode");
        std::fs::create_dir_all(&staging).unwrap();
        let root = tempfile::Builder::new()
            .prefix("swift-validation-")
            .tempdir_in(staging)
            .unwrap()
            .keep();
        let input = root.join("api.json");
        std::fs::write(&input, serde_json::json!({"openapi":"3.1.0","info":{"title":"Portable Swift","version":"1"},"paths":{},"components":{"schemas":schemas}}).to_string()).unwrap();
        let workspace = Arc::new(
            suspect_ref::WorkspaceBuilder::new()
                .root(&root)
                .build()
                .unwrap(),
        );
        let contract = Arc::new(
            Contract::from_workspace(&workspace, &suspect_source::Uri::from_path(&input).unwrap())
                .unwrap(),
        );
        let roots = contract.schema_roots().to_vec();
        let program = OwnedCompiler::new(Config {
            max_depth: 128,
            ..Default::default()
        })
        .compile(contract.clone(), &roots)
        .unwrap()
        .program();
        admit(&contract, &program).unwrap();
        let schema_index = |name: &str| {
            let root = roots
                .iter()
                .find(|root| root.pointer() == format!("/components/schemas/{name}"))
                .unwrap();
            index(&program, root)
        };
        let mut tests = String::from(
            "import XCTest\n@testable import PortableValidation\n\nfinal class Vectors: XCTestCase {\n    func testSharedVectors() throws {\n",
        );
        for (i, case) in cases.iter().enumerate() {
            let node = schema_index(&format!("V{i:02}"));
            for text in case["valid"].as_array().unwrap() {
                let _ = writeln!(
                    tests,
                    "        XCTAssertNoThrow(try ValidationSession().check({node}, JsonValue.parse({})), {})",
                    string(text.as_str().unwrap()),
                    string(case["name"].as_str().unwrap())
                );
            }
            for text in case["invalid"].as_array().unwrap() {
                let _ = writeln!(
                    tests,
                    "        XCTAssertThrowsError(try ValidationSession().check({node}, JsonValue.parse({}))) {{ error in XCTAssertEqual((error as? ValidationError)?.kind, .invalid, {}) }}",
                    string(text.as_str().unwrap()),
                    string(case["name"].as_str().unwrap())
                );
            }
        }
        tests.push_str("    }\n    func testIncompleteBranchesAndWorkBounds() throws {\n");
        for name in ["FailureInUnion", "FailureInNot"] {
            let _ = writeln!(
                tests,
                "        XCTAssertThrowsError(try ValidationSession().check({}, .null)) {{ error in XCTAssertEqual((error as? ValidationError)?.kind, .evaluationFailure) }}",
                schema_index(name)
            );
        }
        let _ = writeln!(
            tests,
            "        XCTAssertNoThrow(try ValidationSession().check({}, .string(\"x\")))",
            schema_index("HugeCount")
        );
        let _ = writeln!(
            tests,
            "        XCTAssertThrowsError(try ValidationSession().check({}, .string(\"e\\u{{301}}\"))) {{ error in XCTAssertEqual((error as? ValidationError)?.kind, .invalid) }}",
            schema_index("UnicodeEquality")
        );
        let unique = cases
            .iter()
            .position(|c| c["name"] == "unique-mathematical-values")
            .unwrap();
        let _ = writeln!(
            tests,
            "        let items = (0..<2000).map {{ JsonValue.number(JsonNumber(Int64($0))) }}\n        XCTAssertThrowsError(try ValidationSession().check({}, .array(items))) {{ error in XCTAssertEqual((error as? ValidationError)?.kind, .evaluationFailure) }}",
            schema_index(&format!("V{unique:02}"))
        );
        tests.push_str("    }\n}\n");
        std::fs::create_dir_all(root.join("Sources/PortableValidation")).unwrap();
        std::fs::create_dir_all(root.join("Tests/Vectors")).unwrap();
        for (name, text) in [
            ("Json.swift", include_str!("json.swift")),
            ("Number.swift", include_str!("number.swift")),
            ("Validation.swift", include_str!("validation.swift")),
        ] {
            std::fs::write(root.join("Sources/PortableValidation").join(name), text).unwrap();
        }
        std::fs::write(
            root.join("Sources/PortableValidation/Program.swift"),
            emit(&contract, &program).unwrap(),
        )
        .unwrap();
        std::fs::write(root.join("Tests/Vectors/Vectors.swift"), tests).unwrap();
        std::fs::write(root.join("Package.swift"),"// swift-tools-version: 6.0\nimport PackageDescription\nlet package = Package(name: \"PortableValidation\", targets: [.target(name: \"PortableValidation\", swiftSettings: [.swiftLanguageMode(.v6)]), .testTarget(name: \"Vectors\", dependencies: [\"PortableValidation\"], swiftSettings: [.swiftLanguageMode(.v6)])])\n").unwrap();
        let swift =
            std::env::var_os("SUSPECT_SWIFT_BIN").unwrap_or_else(|| "/usr/bin/swift".into());
        let swiftc = std::env::var_os("SUSPECT_SWIFTC_BIN")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(&swift).with_file_name("swiftc"));
        let mut command = Command::new(swift);
        command
            .args(["test", "--disable-swift-testing"])
            .env("SWIFT_EXEC", swiftc);
        if let Some(sdk) = std::env::var_os("SUSPECT_SWIFT_SDKROOT") {
            command.arg("--sdk").arg(&sdk).env("SDKROOT", sdk);
        }
        let output = command
            .arg("--package-path")
            .arg(&root)
            .arg("--scratch-path")
            .arg(root.join("build"))
            .args(["-Xswiftc", "-warnings-as-errors"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "fixture retained at {}\n{}{}",
            root.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(Path::new(&root).join("build").is_dir());
        println!(
            "Shared Swift portable validation corpus passed at {}",
            root.display()
        );
    }
}
