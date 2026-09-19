//! Directional request/response codec views: located diagnostics, view
//! binding, and native encode/decode validation through projected programs.

use std::{path::Path, process::Command, sync::Arc};

use serde_json::{Value, json};
use suspect_codegen::typescript::{
    ModelDiagnostic, ModelView,
    codecs::{CodecConfig, CodecPlan, plan_codecs, plan_codecs_with_views},
};
use suspect_ir::contract::{Contract, SchemaId};
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
        json!({"openapi":version,"info":{"title":"Directional","version":"1"},"paths":{},"components":{"schemas":schemas}})
            .to_string(),
    )
    .unwrap();
    load(&path)
}

fn plan_roots(
    views: &[ModelView],
    contract: &Arc<Contract>,
    roots: &[SchemaId],
) -> Result<CodecPlan, Vec<ModelDiagnostic>> {
    plan_codecs_with_views(contract.clone(), roots, views, CodecConfig::default())
}

fn plan_with(
    views: &[ModelView],
    contract: &Arc<Contract>,
) -> Result<CodecPlan, Vec<ModelDiagnostic>> {
    let roots = contract.schema_roots().to_vec();
    plan_roots(views, contract, &roots)
}

fn program_files(plan: &CodecPlan) -> Vec<String> {
    plan.render()
        .iter()
        .filter(|file| file.path.starts_with("typescript/validation-program"))
        .map(|file| file.path.clone())
        .collect()
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

/// One closure with every directional shape the projection must distinguish:
/// required readOnly, required writeOnly, an ordinary required property, a
/// nullable optional, a reference-carried annotation, recursion, and a oneOf
/// exclusivity whose annotated branch changes shape between views.
fn directional_fixture() -> Arc<Contract> {
    fixture(
        "3.1.0",
        json!({
            "Record":{"type":"object","additionalProperties":false,"required":["id","secret","name"],"properties":{
                "id":{"type":"string","readOnly":true},
                "secret":{"type":"string","writeOnly":true},
                "name":{"type":"string"},
                "note":{"type":["string","null"]}
            }},
            "RefId":{"type":"string","readOnly":true},
            "RefInput":{"type":"object","additionalProperties":false,"required":["id"],"properties":{"id":{"$ref":"#/components/schemas/RefId"}}},
            "Node":{"type":"object","additionalProperties":false,"required":["id"],"properties":{
                "id":{"type":"integer","readOnly":true},
                "next":{"$ref":"#/components/schemas/Node"}
            }},
            "BranchA":{"type":"object","required":["x"],"properties":{"x":{"type":"string","readOnly":true}}},
            "BranchB":{"type":"object","required":["y"],"properties":{"y":{"type":"integer"}}},
            "Choice":{"oneOf":[{"$ref":"#/components/schemas/BranchA"},{"$ref":"#/components/schemas/BranchB"}]}
        }),
    )
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn directional_views_fail_explicitly_when_annotation_applicability_is_unestablishable() {
    let contract = fixture(
        "3.1.0",
        json!({
            "Input":{"type":"object","required":["id"],"properties":{
                "id":{"anyOf":[{"type":"string","readOnly":true},{"type":"boolean"}]}
            }}
        }),
    );
    let neutral = plan_with(&[ModelView::Neutral], &contract).unwrap();
    assert!(!neutral.models().has_errors());
    for (view, keyword) in [
        (ModelView::Request, "readOnly"),
        (ModelView::Response, "writeOnly"),
    ] {
        let mut schemas = json!({"Input":{"type":"object","required":["id"],"properties":{
            "id":{"anyOf":[{"type":"string"},{"type":"boolean"}]}
        }}});
        schemas["Input"]["properties"]["id"]["anyOf"][0][keyword] = json!(true);
        let directional = fixture("3.1.0", schemas);
        let errors = plan_with(&[view], &directional).unwrap_err();
        assert!(
            errors.iter().any(|error| {
                error.code == "directional-annotation-evaluation"
                    && error.source.pointer().ends_with("/properties/id/anyOf/0")
            }),
            "{view:?}: {errors:?}"
        );
    }
    // readOnly does not change response requiredness; no applicability proof
    // is needed for the opposite annotation when validation stays neutral.
    assert!(plan_with(&[ModelView::Response], &contract).is_ok());
    // A directly annotated allOf member is equally unestablishable under the
    // approved applicability policy, and Neutral still plans.
    let composed = fixture(
        "3.1.0",
        json!({
            "Input":{"type":"object","required":["id"],"properties":{
                "id":{"allOf":[{"type":"string","readOnly":true}]}
            }}
        }),
    );
    let errors = plan_with(&[ModelView::Request], &composed).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.code == "directional-annotation-evaluation")
    );
    assert!(plan_with(&[ModelView::Neutral], &composed).is_ok());
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn oas30_directional_required_policy_stays_unsupported() {
    // OpenAPI 3.0 changes which direction `required` applies in, but the owned
    // validator compiles only the OAS 3.1 / JSON Schema 2020-12 subset, so a
    // 3.0 directional codec plan must fail instead of guessing 3.0 semantics.
    let contract = fixture(
        "3.0.3",
        json!({
            "Record":{"type":"object","required":["id"],"properties":{
                "id":{"type":"string","readOnly":true}
            }}
        }),
    );
    let errors = plan_with(&[ModelView::Request], &contract).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.code == "codec-schema-compilation"),
        "{errors:?}"
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn directional_artifacts_bind_every_codec_to_its_view_and_stay_deterministic() {
    let contract = directional_fixture();
    let plan = plan_with(
        &[ModelView::Neutral, ModelView::Request, ModelView::Response],
        &contract,
    )
    .unwrap();
    let rendered = plan.render();
    assert_eq!(rendered, plan.render());
    // View selection order must not change allocation or artifacts.
    let reversed = plan_with(
        &[ModelView::Response, ModelView::Request, ModelView::Neutral],
        &contract,
    )
    .unwrap();
    assert_eq!(rendered, reversed.render());
    // Every annotated view needs its own projected program module.
    assert_eq!(
        program_files(&plan),
        vec![
            "typescript/validation-program-request.ts",
            "typescript/validation-program-response.ts",
            "typescript/validation-program.ts",
        ]
    );
    // The manifest binds each codec to its planned model and its view.
    let manifest = manifest(&plan);
    let codecs = manifest["codecs"].as_array().unwrap();
    let binding = |model: &str| {
        codecs
            .iter()
            .find(|codec| codec["model"] == model)
            .unwrap_or_else(|| panic!("missing codec binding for {model}"))
            .clone()
    };
    assert_eq!(binding("Record")["name"], "RecordCodec");
    assert_eq!(binding("Record")["view"], "Neutral");
    assert_eq!(binding("Record")["file"], "model-codecs.ts");
    assert_eq!(binding("RecordRequest")["name"], "RecordRequestCodec");
    assert_eq!(binding("RecordRequest")["view"], "Request");
    assert_eq!(binding("RecordResponse")["name"], "RecordResponseCodec");
    assert_eq!(binding("RecordResponse")["view"], "Response");
    for codec in codecs {
        assert_eq!(
            codec["name"].as_str().unwrap(),
            format!("{}Codec", codec["model"].as_str().unwrap())
        );
    }
    assert_eq!(codecs.len(), plan.models().symbols().len());
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn unannotated_closures_keep_one_validation_program_across_views() {
    // Requesting directional views must not invent directional semantics: a
    // closure without annotations projects every view to the same program.
    let contract = fixture(
        "3.1.0",
        json!({"Record":{"type":"object","additionalProperties":false,"required":["id"],"properties":{
            "id":{"type":"string"},"note":{"type":["string","null"]}
        }}}),
    );
    let plan = plan_with(
        &[ModelView::Neutral, ModelView::Request, ModelView::Response],
        &contract,
    )
    .unwrap();
    assert_eq!(
        program_files(&plan),
        vec!["typescript/validation-program.ts"]
    );
    assert_eq!(plan.render(), plan.render());
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn neutral_only_planning_stays_backward_compatible() {
    let contract = directional_fixture();
    let neutral = plan_codecs(
        contract.clone(),
        contract.schema_roots(),
        CodecConfig::default(),
    )
    .unwrap();
    let via_views = plan_with(&[ModelView::Neutral], &contract).unwrap();
    assert_eq!(neutral.render(), via_views.render());
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn empty_view_selection_returns_a_diagnostic_without_roots() {
    let contract = fixture("3.1.0", json!({}));
    let errors = plan_codecs_with_views(contract, &[], &[], CodecConfig::default()).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.code == "missing-model-view")
    );
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

#[test]
#[ignore = "requires native TypeScript and Node.js"]
fn directional_codecs_validate_each_view_on_encode_and_decode() {
    let contract = directional_fixture();
    let plan = plan_with(
        &[ModelView::Neutral, ModelView::Request, ModelView::Response],
        &contract,
    )
    .unwrap();
    native(
        &plan,
        r#"
import { RecordCodec, RecordRequestCodec, RecordResponseCodec, RefInputCodec, RefInputRequestCodec, NodeCodec, NodeRequestCodec, ChoiceCodec, ChoiceRequestCodec, ModelCodecError } from './model-codecs.js';
import type { Record, RecordRequest, RecordResponse, NodeRequest } from './models.js';
const invalid = (error: unknown): boolean => error instanceof ModelCodecError && error.kind === 'invalid';

// Request view: readOnly `id` presence relaxed, writeOnly `secret` required.
const request = RecordRequestCodec.decode('{"secret":"s","name":"n","note":null}');
assert.equal(Object.hasOwn(request, 'id'), false);
assert.equal(request.note, null);
assert.equal(RecordRequestCodec.encode(request), '{"secret":"s","name":"n","note":null}');
const absent = RecordRequestCodec.decode('{"secret":"s","name":"n"}');
assert.equal(Object.hasOwn(absent, 'note'), false);
assert.equal(RecordRequestCodec.encode(absent), '{"secret":"s","name":"n"}');
// Provided read-only values are retained, not stripped, and still validated.
const retained = RecordRequestCodec.decode('{"id":"srv","secret":"s","name":"n"}');
assert.equal(RecordRequestCodec.encode(retained), '{"id":"srv","secret":"s","name":"n"}');
assert.throws(() => RecordRequestCodec.decode('{"id":123,"secret":"s","name":"n"}'), invalid);
// Ordinary required presence is preserved in both directions.
assert.throws(() => RecordRequestCodec.decode('{"id":"srv","name":"n"}'), invalid);
// Response view: writeOnly presence relaxed, readOnly `id` required.
const response = RecordResponseCodec.decode('{"id":"i","name":"n"}');
assert.equal(Object.hasOwn(response, 'secret'), false);
assert.equal(RecordResponseCodec.encode(response), '{"id":"i","name":"n"}');
const responseRetained = RecordResponseCodec.decode('{"id":"i","secret":"s","name":"n"}');
assert.equal(RecordResponseCodec.encode(responseRetained), '{"id":"i","secret":"s","name":"n"}');
assert.throws(() => RecordResponseCodec.decode('{"name":"n"}'), invalid);
// Neutral keeps complete requiredness in both directions.
const neutral = RecordCodec.decode('{"id":"i","secret":"s","name":"n"}');
assert.equal(RecordCodec.encode(neutral), '{"id":"i","secret":"s","name":"n"}');
assert.throws(() => RecordCodec.decode('{"secret":"s","name":"n"}'), invalid);
assert.throws(() => RecordCodec.decode('{"id":"i","name":"n"}'), invalid);
// Reference-carried annotations relax through canonical target identity.
assert.equal(Object.hasOwn(RefInputRequestCodec.decode('{}'), 'id'), false);
assert.throws(() => RefInputCodec.decode('{}'), invalid);
// Recursion relaxes at every projected node, not only at the root.
const nested = NodeRequestCodec.decode('{"next":{"next":{}}}');
assert.equal(nested.next !== undefined && Object.hasOwn(nested.next, 'next'), true);
assert.equal(NodeRequestCodec.encode(nested), '{"next":{"next":{}}}');
assert.throws(() => NodeCodec.decode('{"next":{"next":{}}}'), invalid);
// changes shape: `{"y":1}` matches both projected branches and stays invalid.
const single = ChoiceRequestCodec.decode('{"x":"a"}');
assert.equal(Object.getPrototypeOf(single), null);
assert.deepEqual({ ...single }, { x: 'a' });
assert.throws(() => ChoiceRequestCodec.decode('{"y":1}'), invalid);
const exclusive = ChoiceCodec.decode('{"y":1}');
assert.deepEqual({ ...exclusive }, { y: 1n });
assert.throws(() => ChoiceCodec.decode('{}'), invalid);

// Static view bindings: exactOptionalPropertyTypes proves required applicability per view.
const typedRequest: RecordRequest = { secret: 's', name: 'n' };
const typedRetained: RecordRequest = { id: 'srv', secret: 's', name: 'n' };
const typedResponse: RecordResponse = { id: 'i', name: 'n' };
const typedNeutral: Record = { id: 'i', secret: 's', name: 'n' };
const typedNode: NodeRequest = { next: {} };
// @ts-expect-error: the neutral view keeps the complete required contract
const incompleteNeutral: Record = { secret: 's', name: 'n' };
// @ts-expect-error: the response view keeps readOnly `id` required
const incompleteResponse: RecordResponse = { name: 'n' };
// @ts-expect-error: the request view keeps writeOnly `secret` required
const incompleteRequest: RecordRequest = { name: 'n' };
"#,
    );
}

#[test]
#[ignore = "requires tracked OpenRouter checkout, native TypeScript and Node.js"]
fn tracked_openrouter_closures_keep_one_view_program_without_annotations() {
    let root = std::env::var_os("OPENROUTER_WEB_ROOT").expect("set OPENROUTER_WEB_ROOT");
    let contract = load(&Path::new(&root).join("projects/docs/openapi/openapi.yaml"));
    let roots: Vec<_> = contract
        .schema_roots()
        .iter()
        .filter(|id| {
            [
                "/components/schemas/ChatRequest",
                "/components/schemas/ChatResult",
            ]
            .contains(&id.pointer())
        })
        .cloned()
        .collect();
    assert_eq!(roots.len(), 2);
    // The tracked public input declares no readOnly/writeOnly annotations, so
    // every requested view must project to the same single validation program;
    // requesting directional views must not invent directional semantics.
    let plan = plan_roots(
        &[ModelView::Neutral, ModelView::Request, ModelView::Response],
        &contract,
        &roots,
    )
    .unwrap();
    assert_eq!(
        program_files(&plan),
        vec!["typescript/validation-program.ts"]
    );
    assert_eq!(plan.render(), plan.render());
    let manifest = manifest(&plan);
    let codecs = manifest["codecs"].as_array().unwrap();
    for view in ["Neutral", "Request", "Response"] {
        assert!(
            codecs.iter().any(|codec| codec["source"]["pointer"]
                == "/components/schemas/ChatRequest"
                && codec["view"] == view),
            "missing {view} codec binding for ChatRequest"
        );
    }
    let request = contract
        .schema(
            roots
                .iter()
                .find(|id| id.pointer().ends_with("/ChatRequest"))
                .unwrap(),
        )
        .unwrap()
        .raw()["example"]
        .to_string();
    let response = contract
        .schema(
            roots
                .iter()
                .find(|id| id.pointer().ends_with("/ChatResult"))
                .unwrap(),
        )
        .unwrap()
        .raw()["example"]
        .to_string();
    let codec_name = |pointer: &str, view: ModelView| {
        format!(
            "{}Codec",
            plan.models()
                .symbols()
                .iter()
                .find(|symbol| symbol.source().pointer() == pointer && symbol.view() == view)
                .unwrap()
                .name()
        )
    };
    native(
        &plan,
        &format!(
            r#"
import {{ {neutralRequest} as NeutralRequest, {neutralResult} as NeutralResult, {requestRequest} as RequestRequest, {requestResult} as RequestResult, {responseRequest} as ResponseRequest, {responseResult} as ResponseResult, ModelCodecError }} from './model-codecs.js';
import type {{ ChatRequest, ChatResult, ChatRequestRequest, ChatResultRequest }} from './models.js';
const requestText = {request};
const responseText = {response};
const request: ChatRequest = NeutralRequest.decode(requestText);
assert.equal(NeutralRequest.encode(request), requestText);
const result: ChatResult = NeutralResult.decode(responseText);
assert.equal(NeutralResult.encode(result), responseText);
const requestRequest: ChatRequestRequest = RequestRequest.decode(requestText);
assert.equal(RequestRequest.encode(requestRequest), requestText);
const requestResult: ChatResultRequest = RequestResult.decode(responseText);
assert.equal(RequestResult.encode(requestResult), responseText);
assert.equal(ResponseRequest.decode(requestText) !== undefined, true);
assert.equal(ResponseResult.decode(responseText) !== undefined, true);
function roundTrip<T>(codec: {{ decode(text: string): T; encode(value: T): string }}, text: string): void {{
    assert.equal(codec.encode(codec.decode(text)), text);
}}
roundTrip(ResponseRequest, requestText);
roundTrip(ResponseResult, responseText);
"#,
            neutralRequest = codec_name("/components/schemas/ChatRequest", ModelView::Neutral),
            neutralResult = codec_name("/components/schemas/ChatResult", ModelView::Neutral),
            requestRequest = codec_name("/components/schemas/ChatRequest", ModelView::Request),
            requestResult = codec_name("/components/schemas/ChatResult", ModelView::Request),
            responseRequest = codec_name("/components/schemas/ChatRequest", ModelView::Response),
            responseResult = codec_name("/components/schemas/ChatResult", ModelView::Response),
            request = serde_json::to_string(&request).unwrap(),
            response = serde_json::to_string(&response).unwrap(),
        ),
    );
}
