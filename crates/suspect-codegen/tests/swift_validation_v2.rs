//! Source Contract -> checked v2 program -> installed Swift models/codecs/DocC.
use serde_json::{Value, json};
use std::{path::Path, sync::Arc};
use suspect_codegen::swift_sdk::{SwiftConfig, plan_sdk};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::WorkspaceBuilder;
use suspect_schema::{OwnedCompiler, OwnedProgram};
use suspect_source::Uri;

#[path = "../src/swift_sdk/validation_v2_support.rs"]
mod support;

fn document() -> Value {
    let schemas = json!({
        "Checkout":{"type":"object","properties":{
            "amount":{"type":"number","minimum":0},"mode":{"type":"string","enum":["card","cash"]},
            "card":{"type":["string","null"]},"billing":{"type":"string"}},"required":["amount","mode"],
            "dependentRequired":{"card":["billing"]},"if":{"properties":{"mode":{"const":"card"}},"required":["mode"]},
            "then":{"required":["card"]},"else":{"not":{"required":["card"]}},"unevaluatedProperties":false,
            "example":{"amount":1,"mode":"card","card":null,"billing":"address"}},
        "DynamicRecord":{"type":"object","properties":{"id":{"type":"string"}},"required":["id"],
            "patternProperties":{"^s_":{"type":"string"},"^n_":{"type":"integer"},"_positive$":{"minimum":1}},
            "propertyNames":{"pattern":"^(id|[sn]_[A-Za-z_]+)$"},"additionalProperties":false,
            "example":{"id":"d","n_positive":1,"s_label":"yes"}},
        "UnicodeRecord":{"type":"object","patternProperties":{"^é$":{"type":"integer"},"^e\u{301}$":{"type":"boolean"}},
            "additionalProperties":false,"dependentRequired":{"é":["e\u{301}"]},"example":{"é":1,"e\u{301}":true}},
        "DependentRecord":{"type":"object","properties":{"enabled":{"type":"boolean"},"peer":{"type":"string"}},
            "dependentSchemas":{"enabled":{"required":["peer"],"properties":{"peer":{"minLength":2}}}},"unevaluatedProperties":false,
            "example":{"enabled":false,"peer":"ok"}},
        "ConditionalCarrier":{"if":{"type":"object","properties":{"tag":{"const":"a"}},"required":["tag"]},
            "then":{"type":"object","properties":{"tag":{"const":"a"},"payload":{"type":"integer"}},"required":["tag","payload"],"additionalProperties":false},
            "else":{"type":"array","items":{"type":"string"},"minItems":1},"example":{"tag":"a","payload":2}},
        "RefBase":{"type":"object","properties":{"known":{"type":"integer"}},"required":["known"]},
        "RefScope":{"$ref":"#/components/schemas/RefBase","unevaluatedProperties":false,"example":{"known":1}},
        "TupleEnvelope":{"type":"array","prefixItems":[{"type":"string"},{"type":"integer"}],"contains":{"type":"integer"},
            "minContains":1,"maxContains":1,"unevaluatedItems":false,"example":["label",1]},
        "UnionEnvelope":{"anyOf":[{"type":"object","properties":{"alpha":{"type":"integer"}},"required":["alpha"]},
            {"type":"object","properties":{"beta":{"type":"string"}},"required":["beta"]}],"unevaluatedProperties":false,"example":{"alpha":1,"beta":"b"}},
        "PresenceModel":{"type":"object","properties":{"present":{"type":["string","null"]},"optional":{"type":["string","null"]}},
            "required":["present"],"dependentRequired":{"optional":["present"]},"additionalProperties":false,"example":{"present":null}},
        "NullableCarrier":{"if":{"type":"object"},"then":{"type":"object","properties":{"id":{"type":"integer"}},"required":["id"],"additionalProperties":false},"else":{"type":"null"},"example":{"id":1}},
        "CarrierBox":{"type":"object","properties":{"choice":{"$ref":"#/components/schemas/NullableCarrier"}},"additionalProperties":false,"example":{"choice":null}}
    });
    let mut paths = serde_json::Map::new();
    for name in [
        "Checkout",
        "DynamicRecord",
        "UnicodeRecord",
        "DependentRecord",
        "ConditionalCarrier",
        "RefScope",
        "TupleEnvelope",
        "UnionEnvelope",
        "PresenceModel",
        "CarrierBox",
    ] {
        let content =
            json!({"application/json":{"schema":{"$ref":format!("#/components/schemas/{name}")}}});
        let operation = json!({"operationId":format!("roundTrip{name}"),"requestBody":{"required":true,"content":content},
            "responses":{"200":{"description":"checked response","content":content}}});
        paths.insert(format!("/v2/{name}"), json!({"post":operation}));
    }
    json!({"openapi":"3.1.0","info":{"title":"Swift v2 checked model witnesses","version":"1"},"servers":[{"url":"https://example.test"}],"paths":paths,"components":{"schemas":schemas}})
}
fn load(path: &Path) -> Arc<Contract> {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap())
}
fn fixture(value: &Value) -> Arc<Contract> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v2.json");
    std::fs::write(&path, value.to_string()).unwrap();
    load(&path)
}
fn selected(contract: &Contract) -> Vec<SourceId> {
    contract.operations().map(|o| o.source().clone()).collect()
}

#[test]
fn typed_program_and_checked_carriers_are_source_bound() {
    let contract = fixture(&document());
    let plan = plan_sdk(
        contract.clone(),
        &selected(&contract),
        SwiftConfig::default(),
    )
    .unwrap();
    assert_eq!(plan.program().version, OwnedProgram::V2_VERSION);
    assert_eq!(plan.program().profile, OwnedProgram::V2_PROFILE);
    plan.program().check().unwrap();
    assert!(!plan.examples().operations().is_empty());
    assert!(
        !plan
            .examples()
            .diagnostics()
            .iter()
            .any(|d| d.code.contains("unsupported")),
        "{:?}",
        plan.examples().diagnostics()
    );
    let operations = plan
        .program()
        .nodes
        .iter()
        .flat_map(|n| &n.checks)
        .filter(|c| c.instruction.requires_v2())
        .map(|c| {
            serde_json::to_value(&c.instruction).unwrap()["op"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect::<std::collections::BTreeSet<_>>();
    for name in [
        "if",
        "dependentRequired",
        "dependentSchemas",
        "contains",
        "patternProperties",
        "additionalPropertiesWithPatterns",
        "propertyNames",
        "unevaluatedProperties",
        "unevaluatedItems",
    ] {
        assert!(operations.contains(name), "missing v2 instruction {name}");
    }
    let names = selected(&contract);
    for source in &names {
        assert_eq!(source.document(), contract.entry());
    }
    let files = plan.render();
    let quickstart = &files
        .iter()
        .find(|f| f.path.ends_with("GettingStarted.md"))
        .unwrap()
        .content;
    assert!(quickstart.contains("ConditionalCarrier(value:"));
    assert!(!quickstart.contains("Codecs."));
    let metadata = suspect_codegen::compatibility::snapshot(
        contract.clone(),
        &contract
            .operations()
            .map(|o| o.operation_id().unwrap().to_owned())
            .collect::<Vec<_>>(),
        &[suspect_codegen::backend::TargetConfig {
            backend: suspect_codegen::backend::Backend::SwiftHttp,
            package_name: "GeneratedSDK".into(),
            package_version: "0.1.0".into(),
            import_name: None,
        }],
    )
    .unwrap();
    assert_eq!(
        metadata.native[0].status,
        suspect_codegen::compatibility::PlanStatus::Planned
    );
    for name in ["ConditionalCarrier", "RefScope", "TupleEnvelope"] {
        assert!(
            metadata.native[0].models.iter().any(|m| m.name == name
                && m.role == "model"
                && m.descriptor.as_ref().unwrap()["kind"] == "checked-carrier"),
            "missing checked carrier {name}"
        );
    }
}

#[test]
fn base_closures_keep_the_frozen_v1_program() {
    let mut value = document();
    let content = json!({"application/json":{"schema":{"$ref":"#/components/schemas/RefBase"}}});
    value["paths"] = json!({"/plain":{"post":{"operationId":"plain","requestBody":{"required":true,"content":content},"responses":{"200":{"description":"OK","content":content}}}}});
    let contract = fixture(&value);
    let plan = plan_sdk(contract.clone(), &selected(&contract), Default::default()).unwrap();
    assert_eq!(plan.program().version, OwnedProgram::V1_VERSION);
    let roots = plan
        .program()
        .roots
        .iter()
        .map(|root| {
            contract
                .schemas()
                .find(|s| {
                    s.id().document().as_str() == root.source.document
                        && s.id().pointer() == root.source.pointer
                })
                .unwrap()
                .id()
                .clone()
        })
        .collect::<Vec<_>>();
    let compiler = OwnedCompiler::new(plan.config().validation.clone());
    assert_eq!(
        compiler
            .compile(contract.clone(), &roots)
            .unwrap()
            .program(),
        compiler.compile_v2(contract, &roots).unwrap().program()
    );
}

#[test]
fn unsupported_v2_layouts_and_resource_policy_stay_located() {
    let mut value = document();
    value["components"]["schemas"]["Checkout"]["$dynamicRef"] = json!("#unknown");
    let contract = fixture(&value);
    let errors = plan_sdk(contract.clone(), &selected(&contract), Default::default()).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.source.pointer().contains("Checkout") && e.at.end > e.at.start)
    );
    let contract = fixture(&document());
    let errors = plan_sdk(
        contract.clone(),
        &selected(&contract),
        SwiftConfig {
            validation: suspect_schema::Config {
                max_depth: 513,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|e| e.code == "swift-validation-resource-policy")
    );
}

#[test]
#[ignore = "installed Swift v2 SDK models/codecs, positive-controlled typing and DocC on selected toolchain"]
fn native_installed_v2_codecs_types_and_docs() {
    let root = support::root("sdk-v2-");
    let input = root.join("api.json");
    std::fs::write(&input, document().to_string()).unwrap();
    let contract = load(&input);
    let plan = plan_sdk(contract.clone(), &selected(&contract), Default::default()).unwrap();
    suspect_codegen::write_files(&plan.render(), &root.join("sdk")).unwrap();
    support::checked(
        support::swift("test")
            .arg("--package-path")
            .arg(root.join("sdk"))
            .arg("--scratch-path")
            .arg(root.join("build/sdk"))
            .args(["-Xswiftc", "-warnings-as-errors"]),
        &root,
    );
    std::fs::create_dir_all(root.join("consumer/Tests/Consumer")).unwrap();
    std::fs::write(root.join("consumer/Package.swift"),"// swift-tools-version: 6.0\nimport PackageDescription\nlet package = Package(name: \"Consumer\", platforms: [.macOS(.v13)], dependencies: [.package(path: \"../sdk\")], targets: [.testTarget(name: \"Consumer\", dependencies: [.product(name: \"GeneratedSDK\", package: \"sdk\")], swiftSettings: [.swiftLanguageMode(.v6)])])\n").unwrap();
    std::fs::write(
        root.join("consumer/Tests/Consumer/V2Tests.swift"),
        include_str!("../src/swift_sdk/validation_v2_native.swift"),
    )
    .unwrap();
    support::checked(
        support::swift("test")
            .arg("--package-path")
            .arg(root.join("consumer"))
            .arg("--scratch-path")
            .arg(root.join("build/consumer"))
            .args(["-Xswiftc", "-warnings-as-errors"]),
        &root,
    );
    let modules = support::directory(&root.join("build/sdk"), "Modules").unwrap();
    let positive = root.join("positive.swift");
    std::fs::write(&positive,"import GeneratedSDK\nlet _ = Checkout(amount: 1, mode: .cash)\nlet _ = ConditionalCarrier(value: .array([.string(\"x\")]))\nlet _ = TupleEnvelope(value: [.string(\"s\"), .number(1)])\nlet _ = CarrierBox(choice: .null)\n").unwrap();
    support::checked(
        support::compiler()
            .arg("-typecheck")
            .arg("-I")
            .arg(&modules)
            .arg(&positive),
        &root,
    );
    for (index, source) in [
        "let _ = Checkout(mode: .cash)",
        "let _ = Checkout(amount: 1, mode: \"cash\")",
        "let _ = Checkout(amount: nil, mode: .cash)",
        "let _ = ConditionalCarrier(value: \"unchecked\")",
        "let _ = TupleEnvelope(value: [1])",
        "let _ = CarrierBox(choice: nil)",
        "let _ = DynamicRecord(id: \"x\", additionalProperties: [\"n_count\": 1])",
    ]
    .iter()
    .enumerate()
    {
        let path = root.join(format!("negative-{index}.swift"));
        std::fs::write(&path, format!("import GeneratedSDK\n{source}\n")).unwrap();
        let output = support::compiler()
            .arg("-typecheck")
            .arg("-I")
            .arg(&modules)
            .arg(&path)
            .output()
            .unwrap();
        assert!(!output.status.success(), "negative compiled: {source}");
        let errors = String::from_utf8_lossy(&output.stderr);
        assert!(
            !errors.contains("no such module")
                && !errors.contains("unable to load standard library"),
            "{errors}"
        );
        std::fs::write(
            root.join(format!("negative-{index}.log")),
            errors.as_bytes(),
        )
        .unwrap();
    }
    support::docs(&root, "GeneratedSDK");
    println!(
        "Swift installed v2 SDK, native carrier typing and DocC passed at {}",
        root.display()
    );
}
