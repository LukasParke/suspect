//! Native consumers of codecs generated from the same canonical model plan.

use std::{path::Path, process::Command, sync::Arc};

use serde_json::{Value, json};
use suspect_codegen::typescript::codecs::{CodecConfig, CodecPlan, plan_codecs};
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

fn fixture(schemas: Value) -> Arc<Contract> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(&path, json!({"openapi":"3.1.0","info":{"title":"Codecs","version":"1"},"paths":{},"components":{"schemas":schemas}}).to_string()).unwrap();
    load(&path)
}

fn plan(schemas: Value) -> CodecPlan {
    let contract = fixture(schemas);
    plan_codecs(
        contract.clone(),
        contract.schema_roots(),
        CodecConfig::default(),
    )
    .unwrap()
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn codec_planning_refuses_unenforced_assertions_and_unsafe_resource_metadata() {
    let contract = fixture(json!({"Pattern":{"type":"string","pattern":r"^\p{Letter}+$"}}));
    let errors = plan_codecs(
        contract.clone(),
        contract.schema_roots(),
        CodecConfig::default(),
    )
    .unwrap_err();
    assert!(errors.iter().any(
        |e| e.source.pointer() == "/components/schemas/Pattern/pattern"
            && e.code == "codec-schema-compilation"
            && e.message.contains("unsupported")
    ));
    if usize::BITS > 53 {
        let contract = fixture(json!({"Boolean":{"type":"boolean"}}));
        let config = CodecConfig {
            max_integer_digits: usize::MAX,
            ..CodecConfig::default()
        };
        let errors = plan_codecs(contract.clone(), contract.schema_roots(), config).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.code == "codec-resource-policy"
                    && e.message.contains("max_integer_digits"))
        );
    }
}

fn native(plan: &CodecPlan, consumer: &str) {
    let directory = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(&plan.render(), directory.path()).unwrap();
    let root = directory.path().join("typescript");
    std::fs::write(root.join("consumer.ts"), format!("declare function require(name: string): any;\nconst assert = require('node:assert/strict');\n{consumer}")).unwrap();
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

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn codec_planning_is_deterministic_and_does_not_promote_an_sdk() {
    let plan = plan(json!({"Flag":{"type":"boolean"}}));
    let files = plan.render();
    assert_eq!(
        files
            .iter()
            .map(|f| (&f.path, &f.content))
            .collect::<Vec<_>>(),
        plan.render()
            .iter()
            .map(|f| (&f.path, &f.content))
            .collect::<Vec<_>>()
    );
    let manifest: Value = serde_json::from_str(
        &files
            .iter()
            .find(|f| f.path == "typescript/docs-manifest.json")
            .unwrap()
            .content,
    )
    .unwrap();
    assert_eq!(manifest["codecsImplemented"], true);
    assert_eq!(manifest["releaseReady"], false);
    assert_eq!(manifest["codecs"][0]["name"], "FlagCodec");
    assert!(
        manifest["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["kind"] != "CodecObligation")
    );
}

#[test]
#[ignore = "requires native TypeScript and Node.js"]
fn generated_codecs_preserve_recursive_models_exact_values_and_presence() {
    let plan = plan(json!({
        "Model":{"type":"object","additionalProperties":false,"required":["n","d","tag","nullable"],"properties":{
            "n":{"type":"integer"},"d":{"type":"number"},"tag":{"type":"string","const":"ok"},
            "nullable":{"type":["string","null"]},"optional":{"type":"string"},
            "next":{"$ref":"#/components/schemas/Model"}
        }},
        "Safe":{"type":"integer","minimum":0,"maximum":10},
        "Literal":{"type":"integer","const":42}
    }));
    native(
        &plan,
        r#"
import { ModelCodec, SafeCodec, LiteralCodec, ModelCodecError } from './model-codecs.js';
import { JsonNumber } from './json.js';
import type { Model } from './models.js';
const text = '{"n":9007199254740993,"d":1.0000000000000000001,"tag":"ok","nullable":null,"next":{"n":2,"d":0,"tag":"ok","nullable":"x"}}';
const value: Model = ModelCodec.decode(text);
assert.equal(value.n, 9007199254740993n);
assert.equal(value.next?.n, 2n);
assert.equal(value.d.toString(), '1.0000000000000000001');
assert.equal(value.nullable, null);
assert.equal(Object.hasOwn(value, 'optional'), false);
assert.equal(ModelCodec.encode(value), text);
assert.equal(SafeCodec.decode('1.0'), 1);
assert.equal(LiteralCodec.decode('42.0'), 42n);
assert.throws(() => SafeCodec.decode('11'), (e: unknown) => e instanceof ModelCodecError && e.kind === 'invalid');
const optional = { ...value, optional: undefined };
// @ts-expect-error: deliberate JavaScript input; typed callers must omit the key
assert.equal(ModelCodec.encode(optional), text);
assert.throws(() => ModelCodec.decode(text.replace('"nullable":null,', '')), (e: unknown) => e instanceof ModelCodecError && e.kind === 'invalid');
// @ts-expect-error: exact decimals are not arbitrary JS numbers
const rounded: Model = { n: 1n, d: 1.1, tag: 'ok', nullable: null };
// @ts-expect-error: presence and nullable are distinct
const omitted: Model = { n: 1n, d: JsonNumber.parse('0'), tag: 'ok' };
"#,
    );
}

#[test]
#[ignore = "requires native TypeScript and Node.js"]
fn generated_refined_unions_and_intersections_follow_valid_schema_branches() {
    let plan = plan(json!({
        "Choice":{"anyOf":[{"type":"integer","minimum":10,"maximum":20},{"type":"integer"}]},
        "Exclusive":{"oneOf":[{"type":"integer","minimum":10,"maximum":20},{"type":"integer"}]},
        "Merged":{"allOf":[{"type":"object","properties":{"n":{"type":"integer"}}},{"type":"object","properties":{"flag":{"type":"boolean"}}}]}
    }));
    native(
        &plan,
        r#"
import { ChoiceCodec, ExclusiveCodec, MergedCodec, ModelCodecError } from './model-codecs.js';
assert.equal(ChoiceCodec.decode('12'), 12);
assert.equal(ChoiceCodec.decode('2'), 2n);
assert.equal(ExclusiveCodec.decode('2'), 2n);
assert.throws(() => ExclusiveCodec.decode('12'), (e: unknown) => e instanceof ModelCodecError && e.kind === 'invalid');
const merged = MergedCodec.decode('{"n":9007199254740993,"flag":true,"extra":null}');
assert.equal(merged.n, 9007199254740993n);
assert.equal(merged.flag, true);
assert.equal(merged.extra, null);
assert.equal(MergedCodec.encode(merged), '{"n":9007199254740993,"flag":true,"extra":null}');
"#,
    );
}

#[test]
#[ignore = "requires native TypeScript and Node.js"]
fn extra_property_types_preserve_named_fields_and_separate_schema_constraints() {
    let plan = plan(json!({
        "StringValue":{"type":"string"},
        "Open":{"type":"object","required":["known"],"properties":{"known":{"type":"integer"}},"additionalProperties":{}},
        "Config":{"type":"object","properties":{"model":{"$ref":"#/components/schemas/StringValue"}},"additionalProperties":{"anyOf":[{"type":"string"},{"type":"number"},{"type":"array","items":{}}]}},
        "Text":{"type":"object","properties":{"known":{"type":"string","minLength":1}},"additionalProperties":{"type":"string","minLength":5}},
        "Arrays":{"type":"object","properties":{"known":{"type":"array","items":{"type":"string","enum":["ok","yes"]}}},"additionalProperties":{"type":"array","items":{"type":"string"}}},
        "Node":{"type":"object","properties":{"next":{"$ref":"#/components/schemas/Node"}},"additionalProperties":false},
        "Refs":{"type":"object","properties":{"known":{"$ref":"#/components/schemas/Node"}},"additionalProperties":{"$ref":"#/components/schemas/Node"}}
    }));
    native(
        &plan,
        r#"
import { OpenCodec, ConfigCodec, TextCodec, ArraysCodec, RefsCodec, ModelCodecError } from './model-codecs.js';
import type { Config, Arrays } from './models.js';
import { JsonNumber } from './json.js';
const open = OpenCodec.decode('{"known":9007199254740993,"anything":{"nested":true}}');
assert.equal(open.known, 9007199254740993n);
assert.equal(OpenCodec.encode(open), '{"known":9007199254740993,"anything":{"nested":true}}');
const configured: Config = { model: 'image-model', count: JsonNumber.parse('2.5'), items: [true, null] };
assert.equal(ConfigCodec.encode(configured), '{"model":"image-model","count":2.5,"items":[true,null]}');
assert.equal(ConfigCodec.decode('{"model":"image-model","quality":"high"}').model, 'image-model');
assert.throws(() => ConfigCodec.decode('{"unexpected":true}'), (e: unknown) => e instanceof ModelCodecError && e.kind === 'invalid');
const text = TextCodec.decode('{"known":"x","other":"valid"}');
assert.equal(TextCodec.encode(text), '{"known":"x","other":"valid"}');
assert.throws(() => TextCodec.decode('{"known":"x","other":"bad"}'), (e: unknown) => e instanceof ModelCodecError && e.kind === 'invalid' && e.findings.some(f => f.instancePath === '/other'));
const arrays: Arrays = { known: ['ok'], other: ['anything'] };
assert.equal(ArraysCodec.encode(arrays), '{"known":["ok"],"other":["anything"]}');
assert.equal(RefsCodec.encode(RefsCodec.decode('{"known":{"next":{}},"other":{}}')), '{"known":{"next":{}},"other":{}}');
// @ts-expect-error: the index signature does not widen the declared model field
const invalidModel: Config = { model: JsonNumber.parse('1') };
// @ts-expect-error: declared array literals remain narrower than additional arrays
const invalidArray: Arrays = { known: ['wrong'] };
"#,
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn additional_property_admission_refuses_misleading_native_index_values() {
    for (name, known, extra) in [
        (
            "scalar",
            json!({"type":["string","boolean"]}),
            json!({"type":"string"}),
        ),
        (
            "numbers",
            json!({"type":"integer"}),
            json!({"type":"number"}),
        ),
        (
            "optional_map",
            json!({"type":"object","additionalProperties":{"type":"string"}}),
            json!({"type":"object","additionalProperties":false,"properties":{"count":{"type":"integer","minimum":0,"maximum":10}}}),
        ),
        (
            "weak_object",
            json!({"type":"object","additionalProperties":false,"required":["label"],"properties":{"label":{"type":"string"}}}),
            json!({"type":"object","additionalProperties":false,"properties":{"count":{"type":"integer","minimum":0,"maximum":10}}}),
        ),
        (
            "required",
            json!({"type":"object","additionalProperties":{"type":"integer"}}),
            json!({"type":"object","required":["count"],"properties":{"count":{"type":"integer"}}}),
        ),
        (
            "array_union",
            json!({"type":"array","items":{"type":["string","boolean"]}}),
            json!({"anyOf":[{"type":"array","items":{"type":"string"}},{"type":"array","items":{"type":"boolean"}}]}),
        ),
        (
            "required_never",
            json!({"type":"object","additionalProperties":false}),
            json!({"type":"object","required":["impossible"],"properties":{"impossible":false},"additionalProperties":false}),
        ),
        (
            "optional_impossible_weak_object",
            json!({"type":"object","required":["impossible"],"properties":{"impossible":false},"additionalProperties":false}),
            json!({"type":"object","properties":{"value":{"type":"string"}},"additionalProperties":false}),
        ),
    ] {
        let contract = fixture(
            json!({"Root":{"type":"object","properties":{"known":known},"additionalProperties":extra}}),
        );
        let errors = plan_codecs(
            contract.clone(),
            contract.schema_roots(),
            CodecConfig::default(),
        )
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.code == "typed-extra-fields-representation"
                    && error.source.pointer() == "/components/schemas/Root/additionalProperties"),
            "{name}: {errors:?}"
        );
    }
}

#[test]
#[ignore = "requires tracked OpenRouter YAML, native TypeScript and Node.js"]
fn tracked_openrouter_chat_closure_and_typed_extras_have_native_codecs() {
    let root = std::env::var_os("OPENROUTER_WEB_ROOT").expect("set OPENROUTER_WEB_ROOT");
    let contract = load(&Path::new(&root).join("projects/docs/openapi/openapi.yaml"));
    let names = [
        "ChatRequest",
        "ChatResult",
        "ImageGenerationServerToolConfig",
        "RouterParams",
        "SubagentNestedTool",
        "TraceConfig",
    ];
    let roots = contract
        .schema_roots()
        .iter()
        .filter(|id| {
            names
                .iter()
                .any(|name| id.pointer() == format!("/components/schemas/{name}"))
        })
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(roots.len(), names.len());
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
    let plan = plan_codecs(contract, &roots, CodecConfig::default()).unwrap();
    native(
        &plan,
        &format!(
            r#"
import {{ ChatRequestCodec, ChatResultCodec, ImageGenerationServerToolConfigCodec as Image, RouterParamsCodec as Router, SubagentNestedToolCodec as Tool, TraceConfigCodec as Trace, ModelCodecError, type Codec }} from './model-codecs.js';
import type {{ ChatRequest, ChatResult, ImageGenerationServerToolConfig }} from './models.js';
import {{ JsonNumber }} from './json.js';
const requestText = {request};
const responseText = {response};
const request: ChatRequest = ChatRequestCodec.decode(requestText);
const response: ChatResult = ChatResultCodec.decode(responseText);
assert.equal(ChatRequestCodec.encode(request), requestText);
assert.equal(ChatResultCodec.encode(response), responseText);
assert.equal(request.messages[0]?.role, 'system');
assert.equal(response.choices[0]?.index, 0n);
const image: ImageGenerationServerToolConfig = {{model:'openai/gpt-5-image',quality:'high',output_compression:JsonNumber.parse('85'),custom:[true,null]}};
assert.equal(Image.encode(image), '{{"model":"openai/gpt-5-image","quality":"high","output_compression":85,"custom":[true,null]}}');
assert.equal(Image.decode(Image.encode(image)).model,'openai/gpt-5-image');
assert.throws(() => Image.decode('{{"invalid":true}}'), (e: unknown) => e instanceof ModelCodecError && e.kind === 'invalid');
function roundTrip<T>(codec: Codec<T>, text: string): void {{
    assert.equal(codec.encode(codec.decode(text)), text);
}}
roundTrip(Router, '{{"version_group":"anthropic/claude-sonnet-4","custom":true}}');
roundTrip(Tool, '{{"type":"openrouter:web_search","parameters":{{"custom":null}},"extra":[1]}}');
roundTrip(Trace, '{{"trace_id":"trace-abc","custom":true}}');
"#,
            request = serde_json::to_string(&request).unwrap(),
            response = serde_json::to_string(&response).unwrap()
        ),
    );
}

#[test]
#[ignore = "requires tracked OpenRouter checkout, native TypeScript and Node.js"]
fn tracked_openrouter_models_decode_and_encode_through_generated_codecs() {
    let root = std::env::var_os("OPENROUTER_WEB_ROOT").expect("set OPENROUTER_WEB_ROOT");
    let contract = load(&Path::new(&root).join("projects/docs/openapi/openapi.yaml"));
    let roots = contract
        .schema_roots()
        .iter()
        .filter(|id| {
            [
                "/components/schemas/ORAnthropicNullableCaller",
                "/components/schemas/AnthropicImageBlockParam",
                "/components/schemas/ChatChoice/properties/index",
            ]
            .contains(&id.pointer())
        })
        .cloned()
        .collect::<Vec<_>>();
    // The operation-index schema is an inline position, not necessarily a root.
    let mut roots = roots;
    let index = contract
        .schema_roots()
        .iter()
        .find(|id| id.pointer() == "/components/schemas/ChatChoice")
        .unwrap()
        .child("properties")
        .child("index");
    if !roots.contains(&index) {
        roots.push(index);
    }
    assert_eq!(roots.len(), 3);
    let plan = plan_codecs(contract, &roots, CodecConfig::default()).unwrap();
    let name = |pointer| {
        plan.models()
            .symbols()
            .iter()
            .find(|s| s.source().pointer() == pointer)
            .unwrap()
            .name()
            .to_owned()
    };
    native(
        &plan,
        &format!(
            r#"
import {{ {caller}Codec as Caller, {image}Codec as Image, {index}Codec as Index, ModelCodecError }} from './model-codecs.js';
assert.equal(Caller.decode('null'), null);
assert.equal(Caller.encode(null), 'null');
const image = Image.decode('{{"type":"image","source":{{"type":"url","url":"https://example.test/image.png"}}}}');
if (image.source.type === 'url') assert.equal(image.source.url, 'https://example.test/image.png');
else throw new Error('discriminator did not survive');
assert.equal(Image.encode(image), '{{"type":"image","source":{{"type":"url","url":"https://example.test/image.png"}}}}');
assert.equal(Index.decode('2'), 2n);
assert.throws(() => Index.decode('1.5'), (e: unknown) => e instanceof ModelCodecError && e.kind === 'invalid');
assert.throws(() => Image.decode('{{"type":"image","source":{{"type":"url"}}}}'), (e: unknown) => e instanceof ModelCodecError && e.kind === 'invalid');
"#,
            caller = name("/components/schemas/ORAnthropicNullableCaller"),
            image = name("/components/schemas/AnthropicImageBlockParam"),
            index = name("/components/schemas/ChatChoice/properties/index")
        ),
    );
}

#[test]
#[ignore = "requires tracked OpenRouter checkout"]
fn tracked_upstream_invalid_numeric_declaration_still_blocks_codec_artifacts() {
    let root = std::env::var_os("OPENROUTER_WEB_ROOT").expect("set OPENROUTER_WEB_ROOT");
    let contract = load(&Path::new(&root).join("projects/docs/openapi/openapi.yaml"));
    let source = contract
        .schema_roots()
        .iter()
        .find(|id| id.pointer() == "/components/schemas/VideoGenerationRequest")
        .unwrap()
        .child("properties")
        .child("upscale_factor");
    let errors = plan_codecs(contract, &[source], CodecConfig::default()).unwrap_err();
    assert!(
        errors.iter().any(|e| e
            .source
            .pointer()
            .ends_with("/upscale_factor/exclusiveMinimum")
            && e.at.start < e.at.end),
        "{errors:?}"
    );
}
