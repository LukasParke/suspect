//! Native consumers of the exact JSON runtime emitted for generated codecs.

use std::{path::Path, process::Command, sync::Arc};

use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn execute(consumer: &str, input: Option<&str>) -> tempfile::TempDir {
    execute_files(
        &[suspect_codegen::typescript::json::runtime()],
        consumer,
        input,
    )
}

fn execute_files(
    files: &[suspect_codegen::OutFile],
    consumer: &str,
    input: Option<&str>,
) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    suspect_codegen::write_files(files, directory.path()).unwrap();
    let root = directory.path().join("typescript");
    std::fs::write(root.join("consumer.ts"), consumer).unwrap();
    if let Some(input) = input {
        std::fs::write(root.join("input.json"), input).unwrap();
    }
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
            "json.ts",
            "consumer.ts",
        ])
        .output()
        .expect("requires native tsc");
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
        .expect("requires native Node.js");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    directory
}

#[test]
#[ignore = "requires native TypeScript and Node.js"]
fn canonical_models_share_the_validated_exact_number_runtime() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("openapi.json");
    std::fs::write(&path, r#"{"openapi":"3.1.0","info":{"title":"Numbers","version":"1"},"paths":{},"components":{"schemas":{"Decimal":{"type":"number"},"Integer":{"type":"integer"}}}}"#).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(directory.path())
            .build()
            .unwrap(),
    );
    let contract = Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap();
    let plan = suspect_codegen::typescript::plan_models(
        &contract,
        contract.schema_roots(),
        &[suspect_codegen::typescript::ModelView::Neutral],
    );
    assert!(
        !plan.release_ready(),
        "a JSON runtime does not replace schema codecs"
    );
    execute_files(
        &plan.render().unwrap(),
        r#"import { JsonNumber } from './models.js';
import type { Decimal, Integer, JsonValue } from './models.js';
import { stringifyJson } from './json.js';
declare function require(name: string): any;
const assert = require('node:assert/strict');
const decimal: Decimal = JsonNumber.parse('1.0000000000000000001');
const integer: Integer = 9007199254740993n;
const value: JsonValue = { decimal, integer };
assert.equal(stringifyJson(value), '{"decimal":1.0000000000000000001,"integer":9007199254740993}');
// @ts-expect-error: general decimals require the exact representation
const rounded: Decimal = 1.1;
"#,
        None,
    );
}

#[test]
#[ignore = "requires native TypeScript and Node.js"]
fn exact_json_preserves_wire_numbers_strings_and_prototype_named_properties() {
    execute(
        r##"import { JsonNumber, parseJson, stringifyJson } from './json.js';
declare function require(name: string): any;
const assert = require('node:assert/strict');
const input = '{"integer":9007199254740993,"decimal":1.0000000000000000001,"tiny":1e-999999,"huge":1e999999,"zero":-0,"__proto__":{"polluted":true},"text":"a\\u0000\\uD834\\uDD1E","null":null}';
const value = parseJson(input);
assert(value !== null && typeof value === 'object' && !Array.isArray(value) && !JsonNumber.is(value));
const object = value as { [name: string]: unknown };
assert(JsonNumber.is(object.integer));
assert.equal((object.integer as JsonNumber).toString(), '9007199254740993');
assert.equal((object.integer as JsonNumber).toBigInt(), 9007199254740993n);
assert.equal((object.integer as JsonNumber).toSafeInteger(), undefined);
assert.equal((object.tiny as JsonNumber).toBigInt(), undefined);
assert.equal((object.huge as JsonNumber).toBigInt(), undefined);
assert.equal((object.zero as JsonNumber).toString(), '-0');
assert.equal(object.text, 'a\u0000\u{1D11E}');
assert.equal(Object.prototype.hasOwnProperty.call(object, '__proto__'), true);
assert.equal(({} as {polluted?: boolean}).polluted, undefined);
const output = stringifyJson(value);
assert(output.includes('9007199254740993') && output.includes('1.0000000000000000001'));
assert(output.includes('1e999999') && output.includes('1e-999999') && output.includes('"zero":-0'));
assert.equal(stringifyJson(parseJson(output)), output);
for (const [token, expected] of [['1.0', 1n], ['1e3', 1000n], ['-10.00e-1', -1n], ['0e-99999', 0n]] as const) {
    assert.equal(JsonNumber.parse(token).toBigInt(), expected);
}
assert.equal(JsonNumber.parse('1e3').toSafeInteger(), 1000);
assert.throws(() => JSON.stringify(JsonNumber.parse('1.1')));
"##,
        None,
    );
}

#[test]
#[ignore = "requires native TypeScript and Node.js"]
fn json_runtime_rejects_ambiguous_unsafe_values_and_bounds_resource_work() {
    execute(
        r##"import { JsonCodecError, JsonNumber, parseJson, stringifyJson } from './json.js';
declare function require(name: string): any;
const assert = require('node:assert/strict');
function failure(action: () => unknown, kind: JsonCodecError['kind']): void {
    assert.throws(action, (error: unknown) => error instanceof JsonCodecError && error.kind === kind);
}

for (const input of ['', ' ', '+1', '01', '-01', '1.', '.1', '1e', '1e+', 'NaN', 'Infinity', '[1,]', '{"a":1,}', 'true false', '{a:1}', '"\\x61"', '"\\uGGGG"', '"\u0000"', '\u00a0null']) {
    failure(() => parseJson(input), 'syntax');
}
for (const input of ['{"a":1,"a":2}', '{"a":1,"\\u0061":2}', '{"__proto__":1,"__proto__":2}']) {
    failure(() => parseJson(input), 'duplicate-key');
}
for (const input of ['-0', '1e+2', '0.0001', '"\\uD800"', '"\\uDC00"', '[true,false,null]', ' {"empty":[]} \r\n\t']) {
    // Independent native JSON comparison is limited to values exactly representable by it.
    assert.deepEqual(JSON.parse(stringifyJson(parseJson(input))), JSON.parse(input));
}
for (const input of [undefined, () => 1, Symbol('x'), NaN, Infinity, -Infinity, 9007199254740992, new Date(), new Map(), Object.create(JsonNumber.prototype)]) {
    failure(() => stringifyJson(input), 'type');
}
const cycle: {next?: unknown} = {}; cycle.next = cycle;
failure(() => stringifyJson(cycle), 'cycle');
let called = false;
const getter = Object.defineProperty({}, 'value', {enumerable:true, get(){ called = true; return 1; }});
failure(() => stringifyJson(getter), 'type'); assert.equal(called, false);
failure(() => stringifyJson({toJSON(){ called = true; return {}; }}), 'type'); assert.equal(called, false);
failure(() => stringifyJson({[Symbol('secret')]: 'hidden'}), 'type');
failure(() => stringifyJson(Object.defineProperty({}, 'hidden', {value:1})), 'type');
failure(() => stringifyJson([, 1]), 'type');
failure(() => stringifyJson(Object.assign([1], {extra:2})), 'type');
assert.equal(stringifyJson({a: {x:1}, b: {x:1}}), '{"a":{"x":1},"b":{"x":1}}');
const shared = {x:1}; assert.equal(stringifyJson([shared, shared]), '[{"x":1},{"x":1}]');
for (const options of [{maxNodes:0}, {maxLength:0}]) {
    failure(() => parseJson('null', options), 'limit');
    failure(() => stringifyJson(null, options), 'limit');
}
for (const options of [{maxDepth:0}, {maxNodes:1}, {maxLength:2}]) {
    failure(() => parseJson('[1]', options), 'limit');
    failure(() => stringifyJson([1], options), 'limit');
}
failure(() => parseJson('1e12345', {maxNumberLength:4}), 'limit');
failure(() => stringifyJson(12345n, {maxNumberLength:4}), 'limit');
failure(() => parseJson('null', {maxDepth:NaN}), 'limit');
failure(() => stringifyJson(null, {maxNodes:-1}), 'limit');
const deep = '['.repeat(5000) + 'null' + ']'.repeat(5000);
failure(() => parseJson(deep), 'limit');
assert.equal(stringifyJson(parseJson(deep, {maxDepth:5000}), {maxDepth:5000}), deep);
assert.equal(stringifyJson(parseJson('null', {maxDepth:0}), {maxDepth:0}), 'null');
"##,
        None,
    );
}

#[test]
#[ignore = "requires OPENROUTER_WEB_ROOT, native TypeScript and Node.js; missing inputs fail"]
fn tracked_openrouter_contract_documents_roundtrip_through_native_json_runtime() {
    let root = std::env::var_os("OPENROUTER_WEB_ROOT")
        .expect("set OPENROUTER_WEB_ROOT to the source checkout");
    for relative in [
        "projects/docs/openapi/openapi.yaml",
        "openrouter-management.openapi.yaml",
        "projects/docs/assets/provider-monitor-schema-v2.openapi.json",
        "packages/temporal/benchmarks.openapi.json",
    ] {
        let path = Path::new(&root).join(relative);
        let workspace = Arc::new(
            WorkspaceBuilder::new()
                .root(path.parent().unwrap())
                .build()
                .unwrap(),
        );
        let contract = Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap())
            .expect("tracked OpenRouter contract input must load");
        let original = contract.document(contract.entry()).unwrap();
        let text = serde_json::to_string(original).unwrap();
        let directory = execute(
            r#"import { parseJson, stringifyJson } from './json.js';
declare function require(name: string): any;
const fs = require('node:fs');
const input: string = fs.readFileSync('input.json', 'utf8');
const parsed = parseJson(input);
const output = stringifyJson(parsed);
fs.writeFileSync('output.json', output);
if (stringifyJson(parseJson(output)) !== output) throw new Error('unstable exact JSON roundtrip');
"#,
            Some(&text),
        );
        let encoded = std::fs::read(directory.path().join("typescript/output.json")).unwrap();
        let roundtripped: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(
            &roundtripped, original,
            "{relative}: native runtime changed the contract document"
        );
    }
}

#[test]
#[ignore = "requires native TypeScript and Node.js"]
fn exact_numbers_ignore_subclass_dispatch_and_limits_admit_before_descriptor_expansion() {
    execute(
        r#"import { JsonCodecError, JsonNumber, stringifyJson } from './json.js';
declare function require(name: string): any;
const assert = require('node:assert/strict');
let called = false;
class Changed extends (JsonNumber as any) {
    constructor() { super('123', 4096); }
    toString() { called = true; return 'false'; }
}
class Getter extends (JsonNumber as any) {
    constructor() { super('123', 4096); }
    get toString() { called = true; return () => 'false'; }
}
class Expanded extends (JsonNumber as any) {
    constructor() { super('123', 4096); }
    toString() { called = true; return {length:0, toString: () => '7'.repeat(1000)}; }
}
for (const number of [new Changed(), new Getter(), new Expanded()]) {
    assert.equal(stringifyJson(number), '123');
    assert.throws(() => stringifyJson(number, {maxLength:1, maxNumberLength:1}), (error:unknown) => error instanceof JsonCodecError && error.kind === 'limit');
}
assert.equal(called, false, 'encoding cannot dispatch through untrusted numeric methods');
const object = Object.fromEntries(Array.from({length:1000}, (_, index) => [String(index), 0]));
let descriptors = 0;
const observed = new Proxy(object, { getOwnPropertyDescriptor(target, key) {
    descriptors++; return Reflect.getOwnPropertyDescriptor(target, key);
}});
for (const options of [{maxLength:0}, {maxNodes:1}, {maxLength:2}]) {
    assert.throws(() => stringifyJson(observed, options), (error:unknown) => error instanceof JsonCodecError && error.kind === 'limit');
    assert.equal(descriptors, 0, 'reject unaffordable containers before allocating descriptors');
}
"#,
        None,
    );
}

#[test]
#[ignore = "requires native TypeScript and Node.js"]
fn optional_undefined_omission_is_explicit_preserves_null_and_charges_inspected_slots() {
    execute(
        r#"import { JsonCodecError, stringifyJson } from './json.js';
declare function require(name: string): any;
const assert = require('node:assert/strict');
const input = { absent: undefined, null: null, nested: { optional: undefined, value: 'yes' } };
assert.throws(() => stringifyJson(input), JsonCodecError);
assert.equal(stringifyJson(input, {omitUndefinedProperties:true}), '{"null":null,"nested":{"value":"yes"}}');
assert(Object.hasOwn(input, 'absent'), 'encoding does not mutate caller objects');
assert.equal(stringifyJson({optional:undefined}, {omitUndefinedProperties:true,maxLength:2,maxNodes:2}), '{}');
assert.throws(() => stringifyJson({optional:undefined}, {omitUndefinedProperties:true,maxNodes:1}), (error:unknown) => error instanceof JsonCodecError && error.kind === 'limit');
assert.throws(() => stringifyJson([undefined], {omitUndefinedProperties:true}), (error:unknown) => error instanceof JsonCodecError && error.kind === 'type');
let called=false;
const accessor=Object.defineProperty({},'optional',{enumerable:true,get(){called=true;return undefined;}});
assert.throws(()=>stringifyJson(accessor,{omitUndefinedProperties:true}),JsonCodecError);
assert.equal(called,false);
const wide = { a: { x:undefined,y:undefined }, b:{ x:undefined,y:undefined } };
assert.equal(stringifyJson(wide,{omitUndefinedProperties:true,maxNodes:7}),'{"a":{},"b":{}}');
assert.throws(()=>stringifyJson(wide,{omitUndefinedProperties:true,maxNodes:6}), (error:unknown)=>error instanceof JsonCodecError && error.kind==='limit');
"#,
        None,
    );
}
