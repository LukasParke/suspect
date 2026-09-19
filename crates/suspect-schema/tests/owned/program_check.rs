use serde_json::{Value, json};
use suspect_schema::{Config, OwnedCompiler, OwnedProgram, ProgramInstruction, ProgramType};

use super::program::{contract, id};

fn program(schema: Value) -> OwnedProgram {
    let contract = contract(&json!({"Model":schema}).to_string());
    let root = id(&contract, "Model");
    OwnedCompiler::new(Config::default())
        .compile(contract, &[root])
        .unwrap()
        .program()
}

fn operand(program: &mut OwnedProgram, keyword: &str, token: &str) {
    let root = program.roots[0].target;
    let check = program.nodes[root]
        .checks
        .iter_mut()
        .find(|check| check.source.pointer.ends_with(&format!("/{keyword}")))
        .unwrap();
    match &mut check.instruction {
        ProgramInstruction::Bound { value, .. }
        | ProgramInstruction::Count { value, .. }
        | ProgramInstruction::MultipleOf { value } => *value = token.into(),
        _ => panic!("expected numeric instruction"),
    }
}

#[test]
fn program_admission_uses_exact_numeric_semantics_after_snapshot_mutation() {
    let original = program(json!({"minimum":0,"multipleOf":1,"minItems":0}));
    original.check().unwrap();
    for token in [
        "0",
        "-0e9",
        "-0e-999999999999999999999999",
        "100e-2",
        "1e999999999999999999999999",
    ] {
        let mut snapshot = original.clone();
        operand(&mut snapshot, "minItems", token);
        snapshot
            .check()
            .unwrap_or_else(|error| panic!("{token}: {error:?}"));
    }
    for (keyword, tokens) in [
        (
            "minItems",
            vec!["-1", "1e-400", "1.5", "-1e999999999999999999999999"],
        ),
        ("multipleOf", vec!["0", "-0e9", "-1", "-1e-400"]),
        (
            "minimum",
            vec![
                "+1", "01", "1.", ".1", "0x10", "NaN", " 1", "1 ", "1e", "1e+", "true", "[1]",
                "\"1\"",
            ],
        ),
    ] {
        for token in tokens {
            let mut snapshot = original.clone();
            operand(&mut snapshot, keyword, token);
            let error = snapshot
                .check()
                .expect_err("invalid portable operand must be rejected");
            assert_eq!(
                error.source.unwrap().pointer,
                format!("/components/schemas/Model/{keyword}")
            );
        }
    }
    for token in [
        "0.01",
        "9007199254740993",
        "1e-999999999999999999999999",
        "1e999999999999999999999999",
    ] {
        let mut snapshot = original.clone();
        operand(&mut snapshot, "multipleOf", token);
        snapshot
            .check()
            .unwrap_or_else(|error| panic!("{token}: {error:?}"));
    }
}

#[test]
fn program_admission_bounds_numeric_instructions_but_defers_literal_budgets() {
    let mut snapshot = program(json!({"minimum":0}));
    snapshot.limits.max_number_bytes = 3;
    operand(&mut snapshot, "minimum", "12345");
    let error = snapshot.check().unwrap_err();
    assert_eq!(
        error.source.unwrap().pointer,
        "/components/schemas/Model/minimum"
    );
    assert!(error.message.contains("3"));

    let mut snapshot = program(serde_json::from_str(r#"{"const":123456789012345678901234567890,"enum":[1e-400,123456789012345678901234567890]}"#).unwrap());
    snapshot.limits.max_number_bytes = 0;
    snapshot.limits.max_equality_steps = 0;
    snapshot
        .check()
        .expect("enum/const operand limits are evaluation-time semantics");
}

fn instruction<'a>(program: &'a mut OwnedProgram, keyword: &str) -> &'a mut ProgramInstruction {
    let root = program.roots[0].target;
    &mut program.nodes[root]
        .checks
        .iter_mut()
        .find(|check| check.source.pointer.ends_with(&format!("/{keyword}")))
        .unwrap()
        .instruction
}

#[test]
fn program_admission_rejects_broken_graph_type_and_applicator_invariants() {
    let original = program(json!({
        "type":["string","object"],"required":["a"],"properties":{"a":true},
        "additionalProperties":false,"prefixItems":[true],"items":true,
        "allOf":[true],"anyOf":[true],"oneOf":[true],"not":false,
        "$ref":"#/components/schemas/Model"
    }));
    original.check().unwrap();
    type Mutation = fn(&mut OwnedProgram);
    let cases: &[(&str, Mutation)] = &[
        ("out of range root", |p| p.roots[0].target = p.nodes.len()),
        ("mismatched root source", |p| {
            p.roots[0].source.pointer = "/other".into()
        }),
        ("duplicate root", |p| p.roots.push(p.roots[0].clone())),
        ("duplicate node", |p| p.nodes.push(p.nodes[0].clone())),
        ("relative document URI", |p| {
            p.nodes[0].source.document = "relative.json".into()
        }),
        ("fragmented document URI", |p| {
            p.nodes[0].source.document.push_str("#part")
        }),
        ("invalid pointer escape", |p| {
            p.nodes[0].source.pointer = "/bad~2".into()
        }),
        ("fragment-form pointer", |p| {
            p.nodes[0].source.pointer = "#/bad".into()
        }),
        ("mislocated check", |p| {
            p.nodes[p.roots[0].target].checks[0].source.pointer = "/other/keyword".into()
        }),
        ("empty types", |p| {
            *instruction(p, "type") = ProgramInstruction::Type { types: vec![] }
        }),
        ("duplicate types", |p| {
            *instruction(p, "type") = ProgramInstruction::Type {
                types: vec![ProgramType::String, ProgramType::String],
            }
        }),
        ("empty allOf", |p| {
            *instruction(p, "allOf") = ProgramInstruction::AllOf { targets: vec![] }
        }),
        ("empty anyOf", |p| {
            *instruction(p, "anyOf") = ProgramInstruction::AnyOf { targets: vec![] }
        }),
        ("empty oneOf", |p| {
            *instruction(p, "oneOf") = ProgramInstruction::OneOf { targets: vec![] }
        }),
        ("empty prefixItems", |p| {
            *instruction(p, "prefixItems") = ProgramInstruction::PrefixItems { targets: vec![] }
        }),
        ("out of range ref", |p| {
            let index = p.nodes.len();
            *instruction(p, "$ref") = ProgramInstruction::Ref { target: index };
        }),
        ("mislocated applicator child", |p| {
            let index = p.roots[0].target;
            *instruction(p, "not") = ProgramInstruction::Not { target: index };
        }),
        ("duplicate property", |p| {
            let ProgramInstruction::Properties { properties } = instruction(p, "properties") else {
                panic!()
            };
            properties.push(properties[0].clone());
        }),
        ("duplicate required name", |p| {
            *instruction(p, "required") = ProgramInstruction::Required {
                names: vec!["a".into(), "a".into()],
            }
        }),
        ("mismatched declared names", |p| {
            let ProgramInstruction::AdditionalProperties { declared, .. } =
                instruction(p, "additionalProperties")
            else {
                panic!()
            };
            *declared = vec!["other".into()];
        }),
        ("mismatched item offset", |p| {
            let ProgramInstruction::Items { start, .. } = instruction(p, "items") else {
                panic!()
            };
            *start = 2;
        }),
    ];
    for (name, mutate) in cases {
        let mut changed = original.clone();
        mutate(&mut changed);
        let error = changed.check().unwrap_err();
        assert!(error.source.is_some(), "{name}: {error:?}");
        assert!(!error.message.is_empty(), "{name}");
    }
    for change_version in [true, false] {
        let mut changed = original.clone();
        if change_version {
            changed.version = "future";
        } else {
            changed.profile = "different";
        }
        assert!(changed.check().unwrap_err().source.is_none());
    }
}

#[test]
fn empty_selected_closure_remains_a_valid_portable_program() {
    let contract = contract(r#"{"Unused":{"pattern":"unsupported"}}"#);
    let snapshot = OwnedCompiler::new(Config::default())
        .compile(contract, &[])
        .unwrap()
        .program();
    assert!(snapshot.nodes.is_empty());
    assert!(snapshot.roots.is_empty());
    snapshot.check().unwrap();
}
