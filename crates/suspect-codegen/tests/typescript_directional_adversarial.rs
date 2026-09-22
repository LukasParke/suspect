//! Adversarial directional projection boundaries beyond the basic coverage in
//! `typescript_directional.rs`: `$ref` siblings, retained constraints on
//! supplied values, cardinality interactions, duplicate/reordered view
//! selection, and view-suffix symbol collisions.

use serde_json::{Value, json};
use std::collections::HashSet;
use std::{path::Path, process::Command, sync::Arc};

use suspect_codegen::typescript::{
    ModelDiagnostic, ModelView,
    codecs::{CodecConfig, CodecPlan, plan_codecs_with_views},
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn load(path: &Path) -> Arc<Contract> {
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(path.parent().unwrap())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(path).unwrap()).unwrap())
}

fn fixture(version: &str, schemas: Value) -> Arc<Contract> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(
        &path,
        json!({"openapi":version,"info":{"title":"Directional adversarial","version":"1"},"paths":{},"components":{"schemas":schemas}})
            .to_string(),
    )
    .unwrap();
    load(&path)
}

fn plan_with(
    views: &[ModelView],
    contract: &Arc<Contract>,
) -> Result<CodecPlan, Vec<ModelDiagnostic>> {
    plan_codecs_with_views(
        contract.clone(),
        contract.schema_roots(),
        views,
        CodecConfig::default(),
    )
}

fn program_files(plan: &CodecPlan) -> Vec<String> {
    let mut files = plan
        .render()
        .iter()
        .filter(|file| file.path.starts_with("typescript/validation-program"))
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn manifest(plan: &CodecPlan) -> Value {
    serde_json::from_str(
        &plan
            .render()
            .iter()
            .find(|file| file.path == "typescript/docs-manifest.json")
            .unwrap()
            .content,
    )
    .unwrap()
}

fn native(plan: &CodecPlan, consumer: &str) {
    let directory = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&plan.render(), directory.path()).unwrap();
    let root = directory.path().join("typescript");
    std::fs::write(
        root.join("consumer.ts"),
        format!("declare function require(name: string): any;\nconst assert = require('node:assert/strict');\n{consumer}"),
    )
    .unwrap();
    let output = Command::new("tsc")
        .current_dir(&root)
        .args([
            "--strict",
            "--exactOptionalPropertyTypes",
            "--noUncheckedIndexedAccess",
            "--target",
            "ES2022",
            "--module",
            "commonjs",
            "--pretty",
            "false",
            "--outDir",
            "dist",
            "consumer.ts",
        ])
        .output()
        .expect("native TypeScript required");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new("node")
        .current_dir(&root)
        .arg("dist/consumer.js")
        .output()
        .expect("native Node required");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Independent boundary closures: a ref-carried annotation with a constraining
/// sibling, an explicit non-annotating `readOnly: false` sibling, a cardinality
/// bound interacting with a relaxed required entry, and relaxation inside array
/// items.
fn adversarial_fixture() -> Arc<Contract> {
    fixture(
        "3.1.0",
        json!({
            "Id":{"type":"string","readOnly":true},
            "Sibling":{"type":"object","required":["id"],"properties":{
                "id":{"$ref":"#/components/schemas/Id","minLength":5}
            }},
            "Plain":{"type":"string"},
            "Override":{"type":"object","required":["id"],"properties":{
                "id":{"$ref":"#/components/schemas/Plain","readOnly":false}
            }},
            "Bounded":{"type":"object","minProperties":2,"required":["a","b"],"properties":{
                "a":{"type":"string","readOnly":true},
                "b":{"type":"string"}
            }},
            "Batch":{"type":"array","items":{"type":"object","required":["id"],"properties":{
                "id":{"type":"string","readOnly":true}
            }}}
        }),
    )
}

#[test]
#[ignore = "requires native TypeScript and Node"]
fn unannotated_ref_sibling_with_explicit_false_keeps_requiredness() {
    // `readOnly: false` proves no exception: without an annotated target the
    // required entry survives the request projection and provided values still
    // satisfy the complete unmodified schema.
    let contract = adversarial_fixture();
    let request = plan_with(&[ModelView::Neutral, ModelView::Request], &contract).unwrap();
    native(
        &request,
        r#"
import { OverrideCodec, OverrideRequestCodec, ModelCodecError } from './model-codecs.js';
const invalid = (error: unknown): boolean => error instanceof ModelCodecError && error.kind === 'invalid';
assert.throws(() => OverrideRequestCodec.decode('{}'), invalid);
assert.equal(OverrideRequestCodec.encode(OverrideRequestCodec.decode('{"id":"v"}')), '{"id":"v"}');
assert.throws(() => OverrideCodec.decode('{}'), invalid);
"#,
    );
}

#[test]
#[ignore = "requires native TypeScript and Node"]
fn projected_required_relaxation_keeps_every_supplied_value_constraint() {
    // Relaxing presence through a ref-carried annotation must not weaken the
    // ref sibling `minLength`, the neutral or response requiredness, or the
    // cardinality bound interacting with the relaxed entry, and relaxation
    // reaches every array item position.
    let contract = adversarial_fixture();
    let request = plan_with(
        &[ModelView::Neutral, ModelView::Request, ModelView::Response],
        &contract,
    )
    .unwrap();
    assert_eq!(
        program_files(&request),
        vec![
            "typescript/validation-program-request.ts",
            "typescript/validation-program.ts",
        ]
    );
    native(
        &request,
        r#"
import { SiblingCodec, SiblingRequestCodec, SiblingResponseCodec, BoundedCodec, BoundedRequestCodec, BatchCodec, BatchRequestCodec, ModelCodecError } from './model-codecs.js';
import type { SiblingRequest, BoundedRequest, SiblingResponse } from './models.js';
const invalid = (error: unknown): boolean => error instanceof ModelCodecError && error.kind === 'invalid';
assert.throws(() => SiblingRequestCodec.decode('{"id":"abc"}'), invalid);
const sibling = SiblingRequestCodec.decode('{"id":"abcdef"}');
assert.equal(SiblingRequestCodec.encode(sibling), '{"id":"abcdef"}');
assert.equal(Object.hasOwn(SiblingRequestCodec.decode('{}'), 'id'), false);
assert.throws(() => SiblingCodec.decode('{}'), invalid);
assert.throws(() => SiblingResponseCodec.decode('{}'), invalid);
assert.throws(() => BoundedRequestCodec.decode('{"b":"y"}'), invalid);
const bounded = BoundedRequestCodec.decode('{"b":"y","c":"z"}');
assert.equal(BoundedRequestCodec.encode(bounded), '{"b":"y","c":"z"}');
assert.throws(() => BoundedCodec.decode('{"b":"y"}'), invalid);
assert.equal(BatchRequestCodec.decode('[{},{"id":"x"}]').length, 2);
assert.throws(() => BatchCodec.decode('[{}]'), invalid);
assert.throws(() => BatchRequestCodec.decode('["x"]'), invalid);
const typedSibling: SiblingRequest = {};
const typedBounded: BoundedRequest = { b: 'y' };
// @ts-expect-error: the response view keeps the ref-annotated id required
const incompleteResponse: SiblingResponse = {};
"#,
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn annotation_reachable_only_through_applicators_inside_a_ref_target_fails() {
    // A required property whose ref target hides the annotation under allOf
    // has no established applicability: planning fails with a located
    // diagnostic instead of relaxing presence.
    let contract = fixture(
        "3.1.0",
        json!({
            "Masked":{"type":"object","required":["id"],"properties":{
                "id":{"$ref":"#/components/schemas/MaskedTarget"}
            }},
            "MaskedTarget":{"allOf":[{"type":"string","readOnly":true}]}
        }),
    );
    let errors = plan_with(&[ModelView::Request], &contract).unwrap_err();
    assert!(
        errors.iter().any(|error| {
            error.code == "directional-annotation-evaluation"
                && error.source.pointer().ends_with("/MaskedTarget/allOf/0")
        }),
        "{errors:?}"
    );
    assert!(plan_with(&[ModelView::Neutral], &contract).is_ok());
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn duplicate_and_reordered_view_selection_produce_identical_artifacts() {
    let contract = adversarial_fixture();
    let single = plan_with(
        &[ModelView::Neutral, ModelView::Request, ModelView::Response],
        &contract,
    )
    .unwrap();
    let reordered = plan_with(
        &[ModelView::Response, ModelView::Neutral, ModelView::Request],
        &contract,
    )
    .unwrap();
    let duplicated = plan_with(
        &[
            ModelView::Request,
            ModelView::Request,
            ModelView::Neutral,
            ModelView::Response,
            ModelView::Response,
        ],
        &contract,
    )
    .unwrap();
    assert_eq!(single.render(), reordered.render());
    assert_eq!(single.render(), duplicated.render());
    assert_eq!(
        single.models().symbols().len(),
        duplicated.models().symbols().len()
    );
}

/// A source symbol whose name already carries a view suffix collides with the
/// allocated request-view name of another schema; both must be disambiguated
/// deterministically and every codec must keep its own view and validator.
fn collision_fixture() -> Arc<Contract> {
    fixture(
        "3.1.0",
        json!({
            "Record":{"type":"object","required":["id"],"properties":{
                "id":{"type":"string","readOnly":true}
            }},
            "RecordRequest":{"type":"object","required":["x"],"properties":{
                "x":{"type":"string"}
            }}
        }),
    )
}

#[test]
#[ignore = "requires native TypeScript and Node"]
fn view_suffix_symbol_collisions_bind_each_codec_to_its_own_view() {
    let contract = collision_fixture();
    let plan = plan_with(
        &[ModelView::Neutral, ModelView::Request, ModelView::Response],
        &contract,
    )
    .unwrap();
    let symbols = plan.models().symbols();
    let name = |pointer: &str, view: ModelView| {
        symbols
            .iter()
            .find(|symbol| symbol.source().pointer() == pointer && symbol.view() == view)
            .unwrap()
            .name()
            .to_owned()
    };
    let record_request_view = name("/components/schemas/Record", ModelView::Request);
    let plain_neutral = name("/components/schemas/RecordRequest", ModelView::Neutral);
    let plain_request = name("/components/schemas/RecordRequest", ModelView::Request);
    assert_eq!(
        name("/components/schemas/Record", ModelView::Neutral),
        "Record"
    );
    assert_ne!(record_request_view, plain_neutral);
    assert_ne!(record_request_view, plain_request);
    let distinct: HashSet<&str> = symbols.iter().map(|symbol| symbol.name()).collect();
    assert_eq!(distinct.len(), symbols.len());
    assert_eq!(plan.render(), plan.render());
    let reversed = plan_with(
        &[ModelView::Response, ModelView::Request, ModelView::Neutral],
        &contract,
    )
    .unwrap();
    assert_eq!(plan.render(), reversed.render());
    let codecs = manifest(&plan)["codecs"].as_array().unwrap().clone();
    assert_eq!(codecs.len(), symbols.len());
    for codec in &codecs {
        assert_eq!(
            codec["name"].as_str().unwrap(),
            format!("{}Codec", codec["model"].as_str().unwrap())
        );
    }
    native(
        &plan,
        &format!(
            r#"
import {{ {recordRequestView}Codec as RecordViewCodec, {plainNeutral}Codec as PlainNeutralCodec, {plainRequest}Codec as PlainRequestCodec, RecordCodec, ModelCodecError }} from './model-codecs.js';
import type {{ {recordRequestView} as RecordView, {plainNeutral} as PlainNeutral }} from './models.js';
const invalid = (error: unknown): boolean => error instanceof ModelCodecError && error.kind === 'invalid';
assert.equal(Object.hasOwn(RecordViewCodec.decode('{{}}'), 'id'), false);
assert.equal(Object.hasOwn(RecordViewCodec.decode('{{"id":"i"}}'), 'id'), true);
assert.throws(() => RecordCodec.decode('{{}}'), invalid);
assert.throws(() => PlainNeutralCodec.decode('{{}}'), invalid);
assert.throws(() => PlainRequestCodec.decode('{{}}'), invalid);
const typed: RecordView = {{}};
// @ts-expect-error: the unrelated RecordRequest keeps x required
const incomplete: PlainNeutral = {{}};
"#,
            recordRequestView = record_request_view,
            plainNeutral = plain_neutral,
            plainRequest = plain_request,
        ),
    );
}

/// JSON Schema Validation 2020-12 §9.4 says combined readOnly/writeOnly
/// annotations SHOULD behave as true if any applicable occurrence is true.
/// A false sibling therefore does not cancel an unconditional referenced true.
#[test]
#[ignore = "requires native TypeScript and Node"]
fn unconditional_true_annotations_are_not_cancelled_by_false_ref_siblings() {
    let contract = fixture(
        "3.1.0",
        json!({
            "Id":{"type":"string","readOnly":true},
            "Conflict":{"type":"object","required":["id"],"properties":{
                "id":{"$ref":"#/components/schemas/Id","readOnly":false}
            }}
        }),
    );
    let plan = plan_with(&[ModelView::Neutral, ModelView::Request], &contract).unwrap();
    native(
        &plan,
        r#"
import { ConflictCodec, ConflictRequestCodec, ModelCodecError } from './model-codecs.js';
assert.equal(ConflictRequestCodec.encode(ConflictRequestCodec.decode('{}')), '{}');
assert.equal(ConflictRequestCodec.encode(ConflictRequestCodec.decode('{"id":"value"}')), '{"id":"value"}');
assert.throws(() => ConflictCodec.decode('{}'), (error: unknown) => error instanceof ModelCodecError && error.kind === 'invalid');
"#,
    );
}
