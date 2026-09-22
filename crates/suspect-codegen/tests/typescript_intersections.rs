//! Public plan admission for numeric representations under container composition.

use std::{path::Path, process::Command, sync::Arc};

use serde_json::{Value, json};
use suspect_codegen::typescript::{DiagnosticKind, ModelPlan, ModelView, plan_models};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_ref::WorkspaceBuilder;
use suspect_schema::{Config, OwnedCompiler, OwnedOutcome};
use suspect_source::Uri;

fn contract(path: &Path) -> Arc<Contract> {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap())
}

fn fixture(schemas: Value) -> (tempfile::TempDir, Arc<Contract>) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(
        &path,
        json!({
            "openapi":"3.1.0", "info":{"title":"Native intersection cases", "version":"1"},
            "paths":{}, "components":{"schemas":schemas}
        })
        .to_string(),
    )
    .unwrap();
    let contract = contract(&path);
    (directory, contract)
}

fn root(contract: &Contract, name: &str) -> SchemaId {
    contract
        .schema_roots()
        .iter()
        .find(|root| root.pointer() == format!("/components/schemas/{name}"))
        .unwrap()
        .clone()
}

fn assert_conflict(schemas: Value, name: &str, input: Value, location: &str) {
    let (_directory, contract) = fixture(schemas);
    let root = root(&contract, name);
    let compiled = OwnedCompiler::new(Config::default())
        .compile(contract.clone(), std::slice::from_ref(&root))
        .unwrap();
    assert!(
        matches!(compiled.validate(&root, &input), OwnedOutcome::Valid),
        "wire witness must be valid"
    );
    let plan = plan_models(&contract, &[root], &[ModelView::Neutral]);
    let finding = plan
        .diagnostics()
        .iter()
        .find(|finding| finding.code == "numeric-intersection-representation")
        .unwrap_or_else(|| {
            panic!(
                "expected a representation conflict: {:?}",
                plan.diagnostics()
            )
        });
    assert_eq!(finding.kind, DiagnosticKind::Error);
    assert_eq!(finding.source.pointer(), location);
    assert!(finding.at.end > finding.at.start);
    assert!(plan.has_errors());
    assert!(!plan.release_ready());
    assert!(
        plan.render().is_err(),
        "do not emit impossible native types"
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn overlapping_properties_detect_safe_integer_and_exact_decimal_conflicts() {
    for other in [
        json!({"type":"integer","minimum":0,"maximum":10}),
        json!({"type":"number"}),
    ] {
        assert_conflict(
            json!({"Model":{"allOf":[
                {"type":"object","required":["x"],"properties":{"x":{"type":"integer"}}},
                {"type":"object","required":["x"],"properties":{"x":other}}
            ]}}),
            "Model",
            json!({"x":1}),
            "/components/schemas/Model/allOf/1/properties/x",
        );
    }
    assert_conflict(
        json!({"Model":{"allOf":[
            {"type":"object","properties":{"outer":{"type":"object","properties":{"x":{"type":"integer"}}}}},
            {"type":"object","properties":{"outer":{"type":"object","properties":{"x":{"type":"number"}}}}}
        ]}}),
        "Model",
        json!({"outer":{"x":1}}),
        "/components/schemas/Model/allOf/1/properties/outer/properties/x",
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn arrays_maps_and_named_fields_against_typed_extras_share_the_guard() {
    assert_conflict(
        json!({"Model":{"allOf":[
            {"type":"array","items":{"type":"integer"}},
            {"type":"array","items":{"type":"number"}}
        ]}}),
        "Model",
        json!([1]),
        "/components/schemas/Model/allOf/1/items",
    );
    assert_conflict(
        json!({"Model":{"allOf":[
            {"type":"object","additionalProperties":{"type":"integer"}},
            {"type":"object","additionalProperties":{"type":"number"}}
        ]}}),
        "Model",
        json!({"x":1}),
        "/components/schemas/Model/allOf/1/additionalProperties",
    );
    assert_conflict(
        json!({"Model":{"allOf":[
            {"type":"object","required":["x"],"properties":{"x":{"type":"integer"}}},
            {"type":"object","additionalProperties":{"type":"number"}}
        ]}}),
        "Model",
        json!({"x":1}),
        "/components/schemas/Model/allOf/1/additionalProperties",
    );
    assert_conflict(
        json!({"Model":{"allOf":[
            {"type":"object","additionalProperties":{"type":"integer"}},
            {"type":"object","required":["x"],"properties":{"x":{"type":"number"}}}
        ]}}),
        "Model",
        json!({"x":1}),
        "/components/schemas/Model/allOf/1/properties/x",
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn reference_siblings_and_recursive_container_pairs_are_checked() {
    assert_conflict(
        json!({
            "Base":{"type":"object","required":["x"],"properties":{"x":{"type":"integer"}}},
            "Model":{"$ref":"#/components/schemas/Base","type":"object","properties":{"x":{"type":"number"}}}
        }),
        "Model",
        json!({"x":1}),
        "/components/schemas/Model/properties/x",
    );
    assert_conflict(
        json!({
            "A":{"type":"object","properties":{"next":{"$ref":"#/components/schemas/A"},"value":{"type":"integer"}}},
            "B":{"type":"object","properties":{"next":{"$ref":"#/components/schemas/B"},"value":{"type":"number"}}},
            "Model":{"allOf":[{"$ref":"#/components/schemas/A"},{"$ref":"#/components/schemas/B"}]}
        }),
        "Model",
        json!({"next":{"value":1}}),
        "/components/schemas/B/properties/value",
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn optional_discriminators_do_not_hide_an_overlap_without_the_discriminator() {
    assert_conflict(
        json!({"Model":{"allOf":[
            {"type":"object","properties":{"kind":{"const":"integer"},"x":{"type":"integer"}}},
            {"type":"object","properties":{"kind":{"const":"decimal"},"x":{"type":"number"}}}
        ]}}),
        "Model",
        json!({"x":1}),
        "/components/schemas/Model/allOf/1/properties/x",
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn conditional_numeric_alternatives_remain_explicitly_unsupported_when_intersected() {
    assert_conflict(
        json!({"Model":{"allOf":[
            {"anyOf":[
                {"type":"object","properties":{"x":{"type":"integer"}}},
                {"type":"object","properties":{"x":{"type":"string"}}}
            ]},
            {"type":"object","properties":{"x":{"type":"number"}}}
        ]}}),
        "Model",
        json!({"x":1}),
        "/components/schemas/Model/allOf/1/properties/x",
    );
}

fn compatible_schemas() -> Value {
    let tagged = json!({"anyOf":[
        {"type":"object","required":["kind","x"],"properties":{"kind":{"const":"integer"},"x":{"type":"integer"}}},
        {"type":"object","required":["kind","x"],"properties":{"kind":{"const":"decimal"},"x":{"type":"number"}}}
    ]});
    json!({
        "Tagged":{"allOf":[tagged.clone(),tagged]},
        "Disjoint":{"allOf":[
            {"type":"object","required":["x"],"properties":{"x":{"type":"integer"}}},
            {"type":"object","required":["y"],"properties":{"y":{"type":"number"}}}
        ]},
        "Same":{"allOf":[
            {"type":"object","properties":{"x":{"type":["integer","null"]}}},
            {"type":"object","properties":{"x":{"type":["integer","null"]}}}
        ]},
        "Passthrough":{"allOf":[
            {"type":"object","properties":{"x":{"type":"integer"}}},
            {"type":"object"}
        ]},
        "Map":{"allOf":[
            {"type":"object","properties":{"x":{"type":"integer"}}},
            {"type":"object","additionalProperties":{"type":"integer"}}
        ]},
        "Array":{"allOf":[
            {"type":"array","items":{"type":"integer"}},
            {"type":"array","items":{"type":"integer"}}
        ]},
        "A":{"type":"object","properties":{"next":{"$ref":"#/components/schemas/A"},"value":{"type":"integer"}}},
        "B":{"type":"object","properties":{"next":{"$ref":"#/components/schemas/B"},"value":{"type":"integer"}}},
        "Recursive":{"allOf":[{"$ref":"#/components/schemas/A"},{"$ref":"#/components/schemas/B"}]},
        "DifferentSingletons":{"allOf":[
            {"type":"object","properties":{"x":{"const":1}}},
            {"type":"object","properties":{"x":{"const":2,"minimum":0,"maximum":10}}}
        ]}
    })
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn disjoint_compatible_passthrough_and_empty_overlap_cases_remain_admitted() {
    let (_directory, contract) = fixture(compatible_schemas());
    let plan = plan_models(&contract, contract.schema_roots(), &[ModelView::Neutral]);
    assert!(!plan.has_errors(), "{:?}", plan.diagnostics());
    assert!(plan.render().is_ok());
}

fn native(plan: &ModelPlan, consumer: &str) {
    assert!(!plan.has_errors(), "{:?}", plan.diagnostics());
    let directory = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&plan.render().unwrap(), directory.path()).unwrap();
    let root = directory.path().join("typescript");
    std::fs::write(root.join("consumer.ts"), consumer).unwrap();
    let output = Command::new("tsc")
        .current_dir(root)
        .args([
            "--strict",
            "--exactOptionalPropertyTypes",
            "--noUncheckedIndexedAccess",
            "--noEmit",
            "--target",
            "ES2022",
            "--module",
            "NodeNext",
            "--moduleResolution",
            "NodeNext",
            "--pretty",
            "false",
            "consumer.ts",
        ])
        .output()
        .expect("native TypeScript compiler required");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "requires native TypeScript compiler"]
fn compatible_intersections_have_native_inhabitants() {
    let (_directory, contract) = fixture(compatible_schemas());
    native(
        &plan_models(&contract, contract.schema_roots(), &[ModelView::Neutral]),
        r#"
import { JsonNumber } from './json.js';
import type { Disjoint, Same, Passthrough, Map, Array, Recursive, DifferentSingletons, Tagged } from './models.js';
const disjoint: Disjoint = { x:1n, y:JsonNumber.parse('2.5') };
const same: Same = { x:1n };
const nullable: Same = { x:null };
const passthrough: Passthrough = { x:1n, other:'text' };
const map: Map = { x:1n, y:2n };
const array: Array = [1n,2n];
const recursive: Recursive = { value:1n, next:{value:2n} };
const emptyOverlap: DifferentSingletons = {};
const taggedInteger: Tagged = {kind:'integer', x:1n};
const taggedDecimal: Tagged = {kind:'decimal', x:JsonNumber.parse('1')};
// @ts-expect-error: exact decimal model fields do not become native floats.
const wrong: Disjoint = { x:1n, y:2.5 };
"#,
    );
}

#[test]
#[ignore = "requires native tsc and the pinned sibling OpenRouter checkout, or SUSPECT_OPENROUTER_YAML"]
fn tracked_openrouter_compositions_keep_their_native_branches() {
    let path = std::env::var_os("SUSPECT_OPENROUTER_YAML")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../openrouter-web/projects/docs/openapi/openapi.yaml")
        });
    let contract = contract(&path);
    let selected = [
        root(&contract, "ORAnthropicNullableCaller"),
        root(&contract, "AnthropicImageBlockParam"),
    ];
    native(
        &plan_models(&contract, &selected, &[ModelView::Neutral]),
        r#"
import type { ORAnthropicNullableCaller, AnthropicImageBlockParam } from './models.js';
const caller: ORAnthropicNullableCaller = {type:'code_execution_20260120',tool_id:'tool'};
const nullable: ORAnthropicNullableCaller = null;
const image: AnthropicImageBlockParam = {type:'image',source:{type:'url',url:'https://example.test/image'}};
// @ts-expect-error: source branches keep their required payloads.
const incomplete: AnthropicImageBlockParam = {type:'image',source:{type:'url'}};
"#,
    );
}
