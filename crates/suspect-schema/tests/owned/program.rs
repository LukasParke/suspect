use std::sync::Arc;

use serde_json::{Value, json};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_low::Pointer;
use suspect_ref::WorkspaceBuilder;
use suspect_schema::{
    Config, OwnedCompileErrorKind, OwnedCompiler, OwnedOutcome, OwnedProgram, ProgramCountTarget,
    ProgramInstruction, ProgramLimits, ProgramNode, ProgramType,
};
use suspect_source::Uri;

pub(super) fn contract(schemas: &str) -> Arc<Contract> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    let schemas: Value = serde_json::from_str(schemas).unwrap();
    std::fs::write(&path, serde_json::to_vec(&json!({"openapi":"3.1.0","info":{"title":"Portable program","version":"1"},"components":{"schemas":schemas}})).unwrap()).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(directory.path())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap())
}

pub(super) fn id(contract: &Contract, name: &str) -> SchemaId {
    SchemaId::new(
        contract.entry().clone(),
        Pointer::parse(&format!("/components/schemas/{name}")).unwrap(),
    )
}

#[test]
fn recursive_program_export_is_finite_deterministic_and_preserves_exact_operands() {
    let contract = contract(
        r##"{
      "Node":{"type":"object","properties":{
        "amount":{"minimum":9007199254740993,"multipleOf":0.01},
        "next":{"anyOf":[{"type":"null"},{"$ref":"#/components/schemas/Node"}]},
        "items":{"maxItems":1e999999999999999999999999},
        "choice":{"enum":[9007199254740993,1e-400,{"x":1.0}]},
        "constant":{"const":{"wide":18446744073709551616}}
      }},
      "Index":{"type":"integer"},
      "Unrelated":{"pattern":"unsupported"}
    }"##,
    );
    let node = id(&contract, "Node");
    let index = id(&contract, "Index");
    let count_token = contract
        .source(&node.child("properties").child("items").child("maxItems"))
        .unwrap()
        .as_number()
        .unwrap()
        .as_str()
        .to_owned();
    let compiler = OwnedCompiler::new(Config::default());
    let first = compiler
        .compile(Arc::clone(&contract), &[node.clone(), index.clone()])
        .unwrap()
        .program();
    let second = compiler
        .compile(contract, &[index, node.clone(), node])
        .unwrap()
        .program();
    first.check().unwrap();
    second.check().unwrap();
    let first = serde_json::to_value(first).unwrap();
    assert_eq!(first, serde_json::to_value(second).unwrap());
    assert_eq!(first["version"], "suspect.validation.experimental.v1");
    assert_eq!(first["roots"].as_array().unwrap().len(), 2);
    let nodes = first["nodes"].as_array().unwrap();
    assert!(nodes.len() < 20, "recursive refs remain finite indices");
    assert!(nodes.iter().all(|n| {
        !n["source"]["pointer"]
            .as_str()
            .unwrap()
            .contains("Unrelated")
    }));
    let checks: Vec<_> = nodes
        .iter()
        .flat_map(|node| node["checks"].as_array().unwrap())
        .collect();
    assert!(
        checks
            .iter()
            .any(|c| c["op"] == "bound" && c["value"] == "9007199254740993")
    );
    assert!(
        checks
            .iter()
            .any(|c| c["op"] == "multipleOf" && c["value"] == "0.01")
    );
    assert!(
        checks
            .iter()
            .any(|c| c["op"] == "count" && c["value"] == count_token)
    );
    assert!(
        checks.iter().any(|c| c["op"] == "enum"
            && c["values"][0].as_number().unwrap().as_str() == "9007199254740993")
    );
    assert!(checks.iter().any(|c| c["op"] == "const"
        && c["value"]["wide"].as_number().unwrap().as_str() == "18446744073709551616"));
    let node_index = nodes
        .iter()
        .position(|n| n["source"]["pointer"] == "/components/schemas/Node")
        .unwrap();
    assert!(
        checks
            .iter()
            .any(|c| c["op"] == "ref" && c["target"] == node_index)
    );
    assert!(checks.iter().all(|c| {
        c["source"]["document"]
            .as_str()
            .unwrap()
            .starts_with("file:")
    }));
    assert_eq!(first["limits"]["maxEvaluationSteps"], 100_000);
}

fn node_at<'a>(program: &'a OwnedProgram, pointer: &str) -> &'a ProgramNode {
    program
        .nodes
        .iter()
        .find(|node| node.source.pointer == pointer)
        .unwrap_or_else(|| panic!("missing node {pointer}"))
}

fn check_at<'a>(node: &'a ProgramNode, keyword: &str) -> &'a ProgramInstruction {
    &node
        .checks
        .iter()
        .find(|check| check.source.pointer == format!("{}/{keyword}", node.source.pointer))
        .unwrap_or_else(|| panic!("missing {keyword} check at {}", node.source.pointer))
        .instruction
}

#[test]
fn typed_program_exposes_applicator_targets_decoded_names_and_all_limits() {
    let contract = contract(
        r##"{
      "Model":{
        "type":["object","array","string","number","integer","boolean","null"],
        "properties":{"a/b~":{"type":"string"}},
        "additionalProperties":false,"required":["a/b~","undeclared"],
        "prefixItems":[true,false],"items":{"const":null},
        "allOf":[{},true],"anyOf":[true,false],"oneOf":[false,true],"not":false,
        "minimum":-9007199254740993,"maximum":9007199254740993,
        "exclusiveMinimum":-1e-400,"exclusiveMaximum":1e-400,
        "multipleOf":0.0100,"minLength":1,"maxLength":10,
        "minItems":2,"maxItems":20,"minProperties":3,"maxProperties":30,
        "enum":[null,true,9007199254740993],"const":{"x":1e-400},"uniqueItems":true,
        "title":"Prose stays in Contract","default":{"a/b~":"no insertion"}
      }
    }"##,
    );
    let root = id(&contract, "Model");
    let compiled = OwnedCompiler::new(Config {
        max_depth: 31,
        max_errors: 0,
        max_number_bytes: 40,
        max_equality_steps: 0,
        max_evaluation_steps: 701,
        ..Config::default()
    })
    .compile(contract, std::slice::from_ref(&root))
    .unwrap();
    let program = compiled.program();
    program.check().unwrap();
    drop(compiled);
    assert_eq!(program.profile, "oas31-jsonschema202012-static-subset");
    assert_eq!(
        program.limits,
        ProgramLimits {
            max_depth: 31,
            max_errors: 0,
            max_number_bytes: 40,
            max_equality_steps: 0,
            max_evaluation_steps: 701,
        }
    );
    assert_eq!(program.roots.len(), 1);
    let selected = &program.roots[0];
    assert_eq!(selected.source.document, root.document().to_string());
    assert_eq!(selected.source.pointer, root.pointer());
    let model = &program.nodes[selected.target];
    assert_eq!(model.source, selected.source);
    assert_eq!(
        check_at(model, "type"),
        &ProgramInstruction::Type {
            types: vec![
                ProgramType::Null,
                ProgramType::Boolean,
                ProgramType::Integer,
                ProgramType::Number,
                ProgramType::String,
                ProgramType::Array,
                ProgramType::Object
            ],
        }
    );
    let ProgramInstruction::Properties { properties } = check_at(model, "properties") else {
        panic!()
    };
    assert_eq!(properties.len(), 1);
    assert_eq!(properties[0].name, "a/b~");
    assert_eq!(
        program.nodes[properties[0].target].source.pointer,
        "/components/schemas/Model/properties/a~1b~0"
    );
    let ProgramInstruction::AdditionalProperties { declared, target } =
        check_at(model, "additionalProperties")
    else {
        panic!()
    };
    assert_eq!(declared, &["a/b~"]);
    assert_eq!(
        program.nodes[*target].checks[0].instruction,
        ProgramInstruction::Always { value: false }
    );
    assert_eq!(
        check_at(model, "required"),
        &ProgramInstruction::Required {
            names: vec!["a/b~".into(), "undeclared".into()],
        }
    );
    let ProgramInstruction::Items { target, start } = check_at(model, "items") else {
        panic!()
    };
    assert_eq!(*start, 2);
    assert_eq!(
        check_at(&program.nodes[*target], "const"),
        &ProgramInstruction::Const { value: Value::Null }
    );
    for (keyword, targets) in [
        (
            "prefixItems",
            match check_at(model, "prefixItems") {
                ProgramInstruction::PrefixItems { targets } => targets,
                _ => panic!(),
            },
        ),
        (
            "allOf",
            match check_at(model, "allOf") {
                ProgramInstruction::AllOf { targets } => targets,
                _ => panic!(),
            },
        ),
        (
            "anyOf",
            match check_at(model, "anyOf") {
                ProgramInstruction::AnyOf { targets } => targets,
                _ => panic!(),
            },
        ),
        (
            "oneOf",
            match check_at(model, "oneOf") {
                ProgramInstruction::OneOf { targets } => targets,
                _ => panic!(),
            },
        ),
    ] {
        assert_eq!(targets.len(), 2);
        for (index, target) in targets.iter().enumerate() {
            assert_eq!(
                program.nodes[*target].source.pointer,
                format!("/components/schemas/Model/{keyword}/{index}")
            );
        }
    }
    let ProgramInstruction::Not { target } = check_at(model, "not") else {
        panic!()
    };
    assert_eq!(
        program.nodes[*target].source.pointer,
        "/components/schemas/Model/not"
    );
    for (keyword, value, maximum, exclusive) in [
        ("minimum", "-9007199254740993", false, false),
        ("maximum", "9007199254740993", true, false),
        ("exclusiveMinimum", "-1e-400", false, true),
        ("exclusiveMaximum", "1e-400", true, true),
    ] {
        assert_eq!(
            check_at(model, keyword),
            &ProgramInstruction::Bound {
                value: value.into(),
                maximum,
                exclusive
            }
        );
    }
    assert_eq!(
        check_at(model, "multipleOf"),
        &ProgramInstruction::MultipleOf {
            value: "0.0100".into()
        }
    );
    for (keyword, value, maximum, target) in [
        ("minLength", "1", false, ProgramCountTarget::String),
        ("maxLength", "10", true, ProgramCountTarget::String),
        ("minItems", "2", false, ProgramCountTarget::Array),
        ("maxItems", "20", true, ProgramCountTarget::Array),
        ("minProperties", "3", false, ProgramCountTarget::Object),
        ("maxProperties", "30", true, ProgramCountTarget::Object),
    ] {
        assert_eq!(
            check_at(model, keyword),
            &ProgramInstruction::Count {
                value: value.into(),
                maximum,
                target
            }
        );
    }
    assert_eq!(
        check_at(model, "uniqueItems"),
        &ProgramInstruction::UniqueItems
    );
    assert!(model.checks.iter().all(|check| {
        !["title", "default"]
            .iter()
            .any(|keyword| check.source.pointer.ends_with(&format!("/{keyword}")))
    }));
    let wire = serde_json::to_value(program).unwrap();
    assert_eq!(
        wire["limits"],
        json!({"maxDepth":31,"maxErrors":0,"maxNumberBytes":40,"maxEqualitySteps":0,"maxEvaluationSteps":701})
    );
}

#[test]
fn unsupported_invalid_and_unknown_selected_closures_never_yield_a_program() {
    let contract = contract(
        r##"{
      "Pattern":{"pattern":"[a-z]+"},
      "Referent":{"$ref":"#/components/schemas/Pattern"},
      "Invalid":{"minItems":1.5},
      "Format":{"format":"uri"}
    }"##,
    );
    for name in ["Pattern", "Referent"] {
        let compiled = OwnedCompiler::new(Config::default())
            .compile(Arc::clone(&contract), &[id(&contract, name)])
            .expect("supported portable pattern closure");
        compiled.program().check().unwrap();
    }
    for (name, format_assertion, kind) in [
        ("Invalid", false, OwnedCompileErrorKind::Invalid),
        ("Format", true, OwnedCompileErrorKind::Unsupported),
        ("Missing", false, OwnedCompileErrorKind::UnknownRoot),
    ] {
        let result = OwnedCompiler::new(Config {
            format_assertion,
            ..Config::default()
        })
        .compile(Arc::clone(&contract), &[id(&contract, name)])
        .map(|compiled| compiled.program());
        let errors = result.expect_err("a portable program requires proved compilation support");
        assert!(
            errors.iter().any(|error| error.kind == kind),
            "{name}: {errors:?}"
        );
    }
}

#[test]
fn external_references_preserve_document_and_escaped_pointer_identity_after_drop() {
    let directory = tempfile::tempdir().unwrap();
    let entry = directory.path().join("api.json");
    let external = directory.path().join("models.json");
    std::fs::write(&entry, r##"{"openapi":"3.1.0","info":{"title":"External","version":"1"},"components":{"schemas":{"Use":{"$ref":"./models.json#/components/schemas/a~1b~0"}}}}"##).unwrap();
    std::fs::write(&external, r##"{"openapi":"3.1.0","info":{"title":"Models","version":"1"},"components":{"schemas":{"a/b~":{"type":"integer"}}}}"##).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(directory.path())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&entry).unwrap()).unwrap());
    let root = id(&contract, "Use");
    let external_uri = Uri::from_path(&external).unwrap().to_string();
    let compiled = OwnedCompiler::new(Config::default())
        .compile(contract, &[root])
        .unwrap();
    let program = compiled.program();
    program.check().unwrap();
    drop(compiled);
    drop(workspace);
    drop(directory);
    let model = &program.nodes[program.roots[0].target];
    let ProgramInstruction::Ref { target } = check_at(model, "$ref") else {
        panic!()
    };
    let external = &program.nodes[*target];
    assert_eq!(external.source.document, external_uri);
    assert_eq!(external.source.pointer, "/components/schemas/a~1b~0");
    assert_eq!(external.checks[0].source.document, external_uri);
    assert_eq!(
        external.checks[0].source.pointer,
        "/components/schemas/a~1b~0/type"
    );
    assert_eq!(
        external.checks[0].instruction,
        ProgramInstruction::Type {
            types: vec![ProgramType::Integer]
        }
    );
}

#[test]
#[ignore = "requires OPENROUTER_WEB_ROOT; missing tracked input fails this acceptance test"]
fn tracked_openrouter_roots_export_their_actual_compiled_checks_and_reference_closure() {
    let root_dir = std::env::var_os("OPENROUTER_WEB_ROOT")
        .map(std::path::PathBuf::from)
        .expect("set OPENROUTER_WEB_ROOT to the source checkout");
    let path = root_dir.join("projects/docs/openapi/openapi.yaml");
    let workspace = Arc::new(WorkspaceBuilder::new().root(&root_dir).build().unwrap());
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap());
    let index = SchemaId::new(
        contract.entry().clone(),
        Pointer::parse("/components/schemas/ChatChoice/properties/index").unwrap(),
    );
    let messages = SchemaId::new(
        contract.entry().clone(),
        Pointer::parse("/components/schemas/ChatRequest/properties/messages").unwrap(),
    );
    let compiled = OwnedCompiler::new(Config::default())
        .compile(contract, &[index.clone(), messages.clone()])
        .unwrap();
    assert!(matches!(
        compiled.validate(&index, &serde_json::from_str("1e-400").unwrap()),
        OwnedOutcome::Invalid(_)
    ));
    assert!(matches!(
        compiled.validate(&messages, &json!([{"role":"user","content":"Hello!"}])),
        OwnedOutcome::Valid
    ));
    let program = compiled.program();
    program.check().unwrap();
    drop(compiled);
    drop(workspace);
    assert_eq!(program.roots.len(), 2);
    assert_eq!(
        check_at(node_at(&program, index.pointer()), "type"),
        &ProgramInstruction::Type {
            types: vec![ProgramType::Integer]
        }
    );
    assert_eq!(
        check_at(node_at(&program, messages.pointer()), "minItems"),
        &ProgramInstruction::Count {
            value: "1".into(),
            maximum: false,
            target: ProgramCountTarget::Array
        }
    );
    let items = node_at(&program, &format!("{}/items", messages.pointer()));
    let ProgramInstruction::Ref { target } = check_at(items, "$ref") else {
        panic!()
    };
    assert_eq!(
        program.nodes[*target].source.pointer,
        "/components/schemas/ChatMessages"
    );
    let ProgramInstruction::OneOf { targets } = check_at(&program.nodes[*target], "oneOf") else {
        panic!()
    };
    assert!(!targets.is_empty());
    assert!(targets.iter().all(|target| *target < program.nodes.len()));
    assert!(
        program
            .nodes
            .iter()
            .all(|node| node.source.document == Uri::from_path(&path).unwrap().to_string())
    );
    assert!(
        !program
            .nodes
            .iter()
            .any(|node| node.source.pointer.contains("VideoGenerationRequest"))
    );
}
