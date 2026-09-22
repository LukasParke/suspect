//! Canonical-contract planning through generated code, docs and native consumers.

use std::{path::Path, process::Command, sync::Arc};

use serde_json::{Value, json};
use suspect_codegen::typescript::{ModelPlan, ModelView, plan_models};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn contract(path: &Path) -> Contract {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap()
}

fn with_spec(schemas: Value, test: impl FnOnce(&Contract)) {
    with_version("3.1.0", schemas, test);
}

fn with_version(version: &str, schemas: Value, test: impl FnOnce(&Contract)) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("openapi.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&json!({
                "openapi":version, "info":{"title":"Contract model cases", "version":"1"},
            "paths":{}, "components":{"schemas":schemas}
        }))
        .unwrap(),
    )
    .unwrap();
    test(&contract(&path));
}

fn roots(contract: &Contract, names: &[&str]) -> Vec<SchemaId> {
    contract
        .schema_roots()
        .iter()
        .filter(|id| {
            names
                .iter()
                .any(|name| id.pointer() == format!("/components/schemas/{name}"))
        })
        .cloned()
        .collect()
}

fn native(plan: &ModelPlan, consumer: &str) {
    assert!(!plan.has_errors(), "{:?}", plan.diagnostics());
    let files = plan.render().unwrap();
    let directory = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&files, directory.path()).unwrap();
    let root = directory.path().join("typescript");
    std::fs::write(root.join("consumer.ts"), consumer).unwrap();
    let output = Command::new("tsc")
        .current_dir(&root)
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
            "models.ts",
            "consumer.ts",
        ])
        .output()
        .expect("native contract tests require tsc on PATH");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let docs = &files
        .iter()
        .find(|file| file.path == "typescript/models.md")
        .unwrap()
        .content;
    for symbol in plan.symbols() {
        assert!(docs.contains(&format!("`{}`", symbol.name())));
        assert!(docs.contains(symbol.source().pointer()));
        assert_eq!(symbol.file(), "typescript/models.ts");
    }
}

#[test]
#[ignore = "requires the native TypeScript compiler (tsc)"]
fn empty_enums_are_uninhabited_and_duplicate_members_remain_valid() {
    for version in ["3.0.3", "3.1.0"] {
        with_version(
            version,
            json!({
                "Empty":{"enum":[]},
                "TypedEmpty":{"type":"string","enum":[]},
                "Duplicate":{"type":"string","enum":["same","same"]}
            }),
            |contract| {
                let plan = plan_models(contract, contract.schema_roots(), &[ModelView::Neutral]);
                assert!(!plan.release_ready());
                native(
                    &plan,
                    r#"import type { Empty, TypedEmpty, Duplicate } from './models.js';
type IsNever<T> = [T] extends [never] ? true : false;
const empty: IsNever<Empty> = true;
const typedEmpty: IsNever<TypedEmpty> = true;
const duplicate: Duplicate = 'same';
// @ts-expect-error: duplicate enum members do not broaden the value set
const invalid: Duplicate = 'different';
"#,
                );
            },
        );
    }
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn numeric_literal_compositions_do_not_erase_valid_number_values() {
    with_spec(
        json!({
            "NullableDecimalEnum":{"type":["number","null"],"enum":[1,null]},
            "DecimalConst":{"type":["number"],"const":1},
            "LiteralIntersection":{"allOf":[{"enum":[1]},{"enum":[1],"minimum":0,"maximum":2}]}
        }),
        |contract| {
            for (name, code) in [
                ("NullableDecimalEnum", "decimal-literal-representation"),
                ("DecimalConst", "decimal-literal-representation"),
                ("LiteralIntersection", "numeric-composition-representation"),
            ] {
                let plan = plan_models(contract, &roots(contract, &[name]), &[ModelView::Neutral]);
                assert!(
                    plan.has_errors(),
                    "{name}: incompatible native representations must not erase the valid JSON value 1"
                );
                assert!(plan.render().is_err());
                assert!(
                    plan.diagnostics()
                        .iter()
                        .any(|diagnostic| diagnostic.code == code)
                );
            }
        },
    );
}

#[test]
#[ignore = "requires the native TypeScript compiler (tsc)"]
fn canonical_models_preserve_presence_null_constraints_and_recursion() {
    with_spec(
        json!({
            "Anything":true, "Nothing":false,
            "Node":{"type":"object", "additionalProperties":false, "required":["name","nullable"], "properties":{
                "name":{"type":"string"}, "nullable":{"type":["string","null"]},
                "optional":{"type":"string"}, "next":{"anyOf":[{"$ref":"#/components/schemas/Node"},{"type":"null"}]}
            }},
            "Untyped":{"properties":{"value":{"type":"string"}}, "required":["value"]},
            "Both":{"allOf":[{"type":"object","required":["a"],"properties":{"a":{"const":"a"}}},{"type":"object","required":["b"],"properties":{"b":{"enum":[true,null,"b"]}}}]}
        }),
        |contract| {
            let plan = plan_models(contract, contract.schema_roots(), &[ModelView::Neutral]);
            assert!(!plan.release_ready());
            native(
                &plan,
                r#"import type { Anything, Nothing, Node, Untyped, Both } from './models.js';
const any: Anything = { list: [null, true, 'yes', 123n] };
// @ts-expect-error: functions are not JSON values
const notJson: Anything = () => 1;
// @ts-expect-error: false schema is uninhabited
const impossible: Nothing = null;
const leaf: Node = { name:'leaf', nullable:null };
const branch: Node = { name:'branch', nullable:'x', next:leaf };
// @ts-expect-error: nullability does not make presence optional
const missing: Node = { name:'x' };
// @ts-expect-error: explicit undefined is not omission
const undefinedValue: Node = { name:'x', nullable:null, optional:undefined };
const untypedScalar: Untyped = 'allowed';
const untypedNull: Untyped = null;
const untypedArray: Untyped = [];
const untypedObject: Untyped = { value:'yes', extra:true };
// @ts-expect-error: object constraints still apply to objects
const untypedBad: Untyped = { value:2n };
const both: Both = { a:'a', b:null };
// @ts-expect-error: all intersection constraints must hold
const missingB: Both = { a:'a' };
"#,
            );
        },
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn unsupported_semantics_and_codec_obligations_remain_source_linked() {
    use suspect_codegen::typescript::{
        DiagnosticKind,
        codecs::{CodecConfig, plan_codecs},
    };
    use suspect_schema::OwnedProgram;

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("openapi.json");
    std::fs::write(&path, serde_json::to_vec(&json!({
        "openapi":"3.1.0","info":{"title":"Located native obligations","version":"1"},"paths":{},
        "components":{"schemas":{
            "Pattern":{"type":"string","pattern":"^[a-z]+$"},
            "TypedExtras":{"type":"object","properties":{"name":{"type":"string"}},"additionalProperties":{"type":"integer"}},
            "Negated":{"type":"string","not":{"const":"x"}},
            "InvalidNot":{"type":"string","not":7},
            "UnsupportedPattern":{"type":"string","pattern":"^\\p{Letter}+$"}
        }}
    })).unwrap()).unwrap();
    let contract = Arc::new(contract(&path));
    let pattern = plan_models(
        &contract,
        &roots(&contract, &["Pattern"]),
        &[ModelView::Neutral],
    );
    assert!(!pattern.has_errors());
    assert!(!pattern.release_ready());
    assert!(pattern.diagnostics().iter().any(|diagnostic| {
        diagnostic.source.pointer().ends_with("/Pattern/pattern")
            && diagnostic.blocks_release()
            && contract.source_span(&diagnostic.source).as_ref() == Some(&diagnostic.at)
    }));
    assert!(pattern.render().is_ok());

    let extras = plan_models(
        &contract,
        &roots(&contract, &["TypedExtras"]),
        &[ModelView::Neutral],
    );
    assert!(extras.has_errors(), "{:?}", extras.diagnostics());
    assert!(extras.render().is_err());
    assert!(
        extras
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.kind == DiagnosticKind::Error
                && diagnostic.source.pointer()
                    == "/components/schemas/TypedExtras/additionalProperties"
                && !diagnostic.at.is_empty()),
        "{:?}",
        extras.diagnostics()
    );

    // Negation now has a checked native codec. The model-only API must still
    // expose its located obligation rather than claiming static enforcement.
    let selected = roots(&contract, &["Negated"]);
    let negated = plan_models(&contract, &selected, &[ModelView::Neutral]);
    assert!(!negated.has_errors(), "{:?}", negated.diagnostics());
    assert!(!negated.release_ready());
    assert!(negated.render().is_ok());
    assert!(
        negated
            .diagnostics()
            .iter()
            .any(
                |diagnostic| diagnostic.code == "applicator-validation-required"
                    && diagnostic.kind == DiagnosticKind::CodecObligation
                    && diagnostic.source == selected[0].child("not")
                    && contract.source_span(&diagnostic.source).as_ref() == Some(&diagnostic.at)
            )
    );
    let codecs = plan_codecs(contract.clone(), &selected, CodecConfig::default()).unwrap();
    assert_eq!(
        codecs.validation_profile(),
        (OwnedProgram::V1_VERSION, OwnedProgram::V1_PROFILE)
    );
    let name = codecs
        .models()
        .symbols()
        .iter()
        .find(|symbol| symbol.source() == &selected[0])
        .unwrap()
        .name();
    let validation = &codecs.interfaces()[name]["validation"];
    let root = validation["root"].as_u64().unwrap() as usize;
    let not = validation["nodes"][root]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["op"] == "not")
        .expect("native negation instruction");
    let target = not["target"].as_u64().unwrap() as usize;
    assert!(
        validation["nodes"][target]["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| check["op"] == "const" && check["value"] == "x")
    );

    for (name, keyword) in [("InvalidNot", "not"), ("UnsupportedPattern", "pattern")] {
        let selected = roots(&contract, &[name]);
        let at = selected[0].child(keyword);
        let errors = plan_codecs(contract.clone(), &selected, CodecConfig::default()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|diagnostic| diagnostic.kind == DiagnosticKind::Error
                    && (diagnostic.source == at || diagnostic.source == selected[0])
                    && contract.source_span(&at).as_ref() == Some(&diagnostic.at)),
            "{name}: {errors:?}"
        );
    }
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn aliases_and_numeric_compositions_do_not_emit_impossible_representation_types() {
    with_spec(
        json!({
            "Integer":{"type":"integer"},
            "Refined":{"$ref":"#/components/schemas/Integer","type":"number"},
            "Combined":{"allOf":[{"type":"integer"},{"type":"number"}]},
            "CycleA":{"$ref":"#/components/schemas/CycleB"},
            "CycleB":{"$ref":"#/components/schemas/CycleA"},
            "FakeType":{"type":"json-number"},
            "TinyLiteral":{"enum":[1e-200]},
            "StructuredLiteral":{"const":{"a":"x"}}
        }),
        |contract| {
            for name in [
                "Refined",
                "Combined",
                "CycleA",
                "FakeType",
                "TinyLiteral",
                "StructuredLiteral",
            ] {
                let plan = plan_models(contract, &roots(contract, &[name]), &[ModelView::Neutral]);
                assert!(plan.has_errors(), "{name}: {:?}", plan.diagnostics());
                assert!(plan.render().is_err());
            }
        },
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn directional_views_do_not_guess_annotation_applicability_through_composition() {
    with_spec(
        json!({
            "Input":{"type":"object","required":["id"],"properties":{
                "id":{"anyOf":[{"type":"string","readOnly":true},{"type":"boolean"}]}
            }}
        }),
        |contract| {
            let neutral = plan_models(contract, contract.schema_roots(), &[ModelView::Neutral]);
            assert!(!neutral.has_errors());
            let request = plan_models(contract, contract.schema_roots(), &[ModelView::Request]);
            assert!(
                request
                    .diagnostics()
                    .iter()
                    .any(
                        |diagnostic| diagnostic.code == "directional-annotation-evaluation"
                            && diagnostic.source.pointer().ends_with("/id/anyOf/0")
                    )
            );
            assert!(request.render().is_err());
        },
    );
}

#[test]
#[ignore = "requires the native TypeScript compiler (tsc)"]
fn numeric_literal_members_do_not_round_even_when_outside_safe_bounds() {
    with_spec(
        json!({
            "BoundedEnum":{"type":"integer","minimum":0,"maximum":100,"enum":[2,9007199254740993_u64]},
            "DecimalInteger":{"type":"integer","enum":[1.0,1e3]}
        }),
        |contract| {
            native(
                &plan_models(contract, contract.schema_roots(), &[ModelView::Neutral]),
                r#"
import type { BoundedEnum, DecimalInteger } from './models.js';
const bounded: BoundedEnum = 2;
const one: DecimalInteger = 1n;
const thousand: DecimalInteger = 1000n;
// @ts-expect-error: the unsafe out-of-bounds literal must not become a rounded number
const rounded: BoundedEnum = 9007199254740992;
"#,
            )
        },
    );
}

#[test]
#[ignore = "requires the native TypeScript compiler (tsc)"]
fn symbols_share_collision_allocation_with_references_and_documentation() {
    with_spec(
        json!({
            "JsonValue":{"type":"string"}, "a-b":{"type":"string"}, "a_u2d_b":{"type":"boolean"},
            "Foo":{"type":"string"}, "FooRequest":{"type":"boolean"},
            "Holder":{"type":"object","required":["a","b"],"properties":{
                "a":{"$ref":"#/components/schemas/a-b"},"b":{"$ref":"#/components/schemas/a_u2d_b"}
            }}
        }),
        |contract| {
            let first = plan_models(
                contract,
                contract.schema_roots(),
                &[ModelView::Neutral, ModelView::Request],
            );
            let mut reversed = contract.schema_roots().to_vec();
            reversed.reverse();
            let second = plan_models(
                contract,
                &reversed,
                &[ModelView::Request, ModelView::Neutral],
            );
            assert_eq!(first.render().unwrap(), second.render().unwrap());
            let names: std::collections::BTreeSet<_> =
                first.symbols().iter().map(|symbol| symbol.name()).collect();
            assert_eq!(names.len(), first.symbols().len());
            native(
                &first,
                r#"import type { Holder } from './models.js';
const valid: Holder = { a:'text', b:true };
// @ts-expect-error: colliding source names must keep distinct reference targets
const wrong: Holder = { a:true, b:'text' };
"#,
            );
        },
    );
}

#[test]
#[ignore = "requires the native TypeScript compiler (tsc)"]
fn numeric_models_do_not_round_unbounded_values_into_javascript_numbers() {
    with_spec(
        json!({
            "Count":{"type":"integer"}, "Ratio":{"type":"number"},
            "Small":{"type":"integer","minimum":0,"maximum":100},
            "Large":{"type":"integer","minimum":0,"maximum":9007199254740993_u64},
            "ExactEnum":{"type":"integer","enum":[9007199254740993_u64,2]}
        }),
        |contract| {
            native(
                &plan_models(contract, contract.schema_roots(), &[ModelView::Neutral]),
                r#"
import type { Count, Ratio, Small, Large, ExactEnum, JsonNumber } from './models.js';
const count: Count = 9007199254740993n;
const large: Large = 9007199254740993n;
const small: Small = 42;
declare const exact: JsonNumber;
const ratio: Ratio = exact;
const enumValue: ExactEnum = 9007199254740993n;
// @ts-expect-error: an unbounded integer is not an IEEE-754 number
const rounded: Count = 9007199254740993;
// @ts-expect-error: decimal precision cannot be recovered from a native number
const lossy: Ratio = 0.1;
// @ts-expect-error: enum membership remains exact
const invalid: ExactEnum = 3n;
"#,
            )
        },
    );
}

#[test]
#[ignore = "requires the native TypeScript compiler (tsc)"]
fn directional_views_keep_wire_fields_and_adjust_required_applicability() {
    with_spec(
        json!({"Record":{"type":"object","required":["id","secret","name"],"properties":{
            "id":{"type":"string","readOnly":true}, "secret":{"type":"string","writeOnly":true}, "name":{"type":"string"}
        }}}),
        |contract| {
            native(
                &plan_models(
                    contract,
                    contract.schema_roots(),
                    &[ModelView::Neutral, ModelView::Request, ModelView::Response],
                ),
                r#"
import type { Record, RecordRequest, RecordResponse } from './models.js';
const neutral: Record = { id:'id', secret:'secret', name:'n' };
const request: RecordRequest = { secret:'secret', name:'n' };
const response: RecordResponse = { id:'id', name:'n' };
const retained: RecordRequest = { id:'id', secret:'secret', name:'n' };
// @ts-expect-error: neutral contract retains complete requiredness
const incomplete: Record = { name:'n' };
"#,
            )
        },
    );
}

#[test]
#[ignore = "requires the native TypeScript compiler (tsc)"]
fn directional_annotations_follow_reference_identity_in_each_openapi_version() {
    for version in ["3.0.3", "3.1.0", "3.2.0"] {
        with_version(
            version,
            json!({
                "Identifier":{"type":"string","readOnly":true},
                "Input":{"type":"object","required":["id","name"],"properties":{
                    "id":{"$ref":"#/components/schemas/Identifier"}, "name":{"type":"string"}
                }}
            }),
            |contract| {
                native(
                    &plan_models(
                        contract,
                        &roots(contract, &["Input"]),
                        &[ModelView::Neutral, ModelView::Request],
                    ),
                    r#"
import type { Input, InputRequest } from './models.js';
const request: InputRequest = { name:'value' };
const retained: InputRequest = { id:'server-id', name:'value' };
// @ts-expect-error: the neutral view retains the complete required contract
const missingNeutral: Input = { name:'value' };
"#,
                )
            },
        );
    }
}

#[test]
#[ignore = "requires tsc and the pinned sibling OpenRouter checkout, or SUSPECT_OPENROUTER_YAML"]
fn tracked_openrouter_nullable_caller_and_image_source_keep_their_branches() {
    let path = std::env::var_os("SUSPECT_OPENROUTER_YAML")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../../openrouter-web/projects/docs/openapi/openapi.yaml")
        });
    let contract = contract(&path);
    let roots = roots(
        &contract,
        &["ORAnthropicNullableCaller", "AnthropicImageBlockParam"],
    );
    assert_eq!(roots.len(), 2);
    let caller = roots
        .iter()
        .find(|id| id.pointer().ends_with("ORAnthropicNullableCaller"))
        .unwrap();
    assert_eq!(
        contract.schema(caller).unwrap().raw()["oneOf"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    native(
        &plan_models(&contract, &roots, &[ModelView::Neutral]),
        r#"
import type { ORAnthropicNullableCaller, AnthropicImageBlockParam } from './models.js';
const nil: ORAnthropicNullableCaller = null;
const direct: ORAnthropicNullableCaller = { type:'direct' };
const execution: ORAnthropicNullableCaller = { type:'code_execution_20260120', tool_id:'tool' };
// @ts-expect-error: code execution payload must retain its required tool_id
const incomplete: ORAnthropicNullableCaller = { type:'code_execution_20260120' };
const image: AnthropicImageBlockParam = { type:'image', source:{type:'url', url:'https://example.test/image'} };
const base64: AnthropicImageBlockParam = { type:'image', source:{type:'base64', media_type:'image/png', data:'abc'} };
// @ts-expect-error: source alternatives cannot exchange their payload fields
const badImage: AnthropicImageBlockParam = { type:'image', source:{type:'base64', url:'https://example.test/image'} };
declare const caller: ORAnthropicNullableCaller;
if (caller !== null && caller.type === 'code_execution_20250825') caller.tool_id satisfies string;
"#,
    );
}
