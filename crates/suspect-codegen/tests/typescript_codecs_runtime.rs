//! Native typed-conversion fixtures independent of the Rust model planner.

use std::{process::Command, sync::Arc};

use serde_json::{Value, json};
use suspect_codegen::{OutFile, typescript::validation::emit};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_schema::{Config, OwnedCompiler, OwnedProgram};
use suspect_source::Uri;

fn execute(schemas: Value, consumer: &str) {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("api.json");
    std::fs::write(&source,json!({"openapi":"3.1.0","info":{"title":"Codec runtime fixtures","version":"1"},"components":{"schemas":schemas}}).to_string()).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(directory.path())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&source).unwrap()).unwrap());
    let roots = contract.schema_roots().to_vec();
    let compiled = OwnedCompiler::new(Config::default())
        .compile(contract, &roots)
        .unwrap();
    execute_program(&compiled.program(), consumer);
}

fn execute_program(program: &OwnedProgram, consumer: &str) {
    let directory = tempfile::tempdir().unwrap();
    let mut files = emit(program).unwrap();
    files.push(OutFile {
        path: "typescript/codecs.ts".into(),
        content: include_str!("../src/typescript/codecs.ts").into(),
    });
    files.push(OutFile {path:"typescript/consumer.ts".into(),content:format!(r#"
import {{ JsonNumber, parseJson, stringifyJson }} from './json.js';
import {{ createCodec, ModelCodecError, type Conversion, type ConversionProgram }} from './codecs.js';
import {{ validate, validationRoots }} from './validation-program.js';
import type {{ ValidationSource, ValidationTrace }} from './validation.js';
declare function require(name:string): any;
const assert = require('node:assert/strict');
const root = (name:string): ValidationSource => validationRoots.find(source=>source.pointer === '/components/schemas/'+name)!;
const at = (name:string, suffix:string, expression:Conversion):Conversion => ({{kind:'source',source:{{...root(name),pointer:root(name).pointer+suffix}},expression}});
const any: Conversion = {{kind:'any'}}, integer: Conversion = {{kind:'integer'}}, number: Conversion = {{kind:'number'}}, string: Conversion = {{kind:'string'}};
function codec<T>(name:string, expression:Conversion, limits:Partial<Omit<ConversionProgram,'symbols'>> = {{}}) {{
    return createCodec<T>(root(name),0,{{symbols:[at(name,'',expression)],maxDepth:256,maxSteps:100000,maxIntegerDigits:4096,...limits}},validate);
}}
function error(kind:string, run:()=>unknown) {{ assert.throws(run,(value:unknown)=>value instanceof ModelCodecError && value.kind === kind); }}
{consumer}
"#)});
    suspect_codegen::write_files(&files, directory.path()).unwrap();
    let output = directory.path().join("typescript");
    let result = Command::new("tsc")
        .current_dir(&output)
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
        .expect("native tsc required");
    assert!(
        result.status.success(),
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let result = Command::new("node")
        .current_dir(&output)
        .arg("dist/consumer.js")
        .output()
        .expect("native Node required");
    assert!(
        result.status.success(),
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
#[ignore = "requires native TypeScript and Node.js"]
fn native_codec_preserves_representation_presence_names_and_validates_encoding() {
    execute(serde_json::from_str(r#"{
      "Safe":{"type":"integer","minimum":-10,"maximum":10},"Integer":{"type":"integer"},"Number":{"type":"number"},
      "Literal":{"enum":[9007199254740993,1000]},"AnyNumber":{"type":["integer","number"]},
      "Object":{"type":"object","properties":{"value":{"type":["integer","null"]},"optional":{"type":"integer"},"text":{"type":"string"},"__proto__":{"type":"integer"}},"required":["value"],"additionalProperties":{"type":"integer"}},
      "Array":{"type":"array","items":{"type":"integer"}}
    }"#).unwrap(),r#"
const safe = codec<number>('Safe',{kind:'safeInteger'});
const wide = codec<bigint>('Integer',integer);
const decimal = codec<JsonNumber>('Number',number);
const broad = codec<number|bigint|JsonNumber>('AnyNumber',{kind:'anyNumber'});
const literal = codec<9007199254740993n|1000n>('Literal',{kind:'union',alternatives:[{kind:'integerLiteral',value:'9007199254740993',safe:false},{kind:'integerLiteral',value:'1000',safe:false}]});
assert.equal(safe.decode('7.0'),7);
assert.equal(wide.decode('9007199254740993'),9007199254740993n);
assert.equal(literal.decode('10e2'),1000n);
assert.equal(decimal.decode('1.0000000000000000001').toString(),'1.0000000000000000001');
assert(JsonNumber.is(broad.decode('7')));
assert.equal(wide.encode(9007199254740993n),'9007199254740993');
error('invalid',()=>safe.decode('11'));
error('invalid',()=>wide.decode('1e-400'));
error('json',()=>wide.decode('7 trailing'));
error('json',()=>wide.encode(9007199254740992 as any));
type Model = {value:bigint|null;optional?:bigint;text?:string;[name:string]:unknown};
const object = codec<Model>('Object',{kind:'object',fields:[
 {name:'value',required:true,expression:{kind:'union',alternatives:[integer,{kind:'null'}]}},
 {name:'optional',required:false,expression:integer},{name:'text',required:false,expression:string},
 {name:'__proto__',required:false,expression:integer}
],extra:integer});
const value = object.decode('{"value":null,"__proto__":9,"extra":9007199254740993}');
assert.equal(value.value,null); assert.equal(value.__proto__,9n); assert.equal(value.extra,9007199254740993n);
assert.equal(Object.getPrototypeOf(value),null); assert(!Object.hasOwn(value,'optional'));
assert.equal(object.encode({value:7n,optional:undefined} as any),'{"value":7}');
error('invalid',()=>object.encode({value:undefined} as any));
error('invalid',()=>object.encode({value:7n,optional:null} as any));
let calls = 0;
const getter = Object.defineProperty({value:7n},'optional',{enumerable:true,get(){calls++;return undefined;}});
error('json',()=>object.encode(getter)); assert.equal(calls,0);
const array = codec<bigint[]>('Array',{kind:'array',item:integer});
assert.deepEqual(array.decode('[1,2.0]'),[1n,2n]);
error('json',()=>array.encode([undefined] as any));
error('json',()=>array.encode(new Array(1)));
"#);
}

#[test]
#[ignore = "requires native TypeScript and Node.js"]
fn native_codec_uses_one_completed_validation_trace_for_union_choices() {
    execute(
        json!({
          "Any":{"anyOf":[{"type":"integer","minimum":5},{"type":"number"}]},
          "TypeUnion":{"type":["integer","number"]},"One":{"oneOf":[{"type":"integer"},{"type":"number"}]}
        }),
        r#"
const expression:Conversion = {kind:'union',alternatives:[at('Any','/anyOf/0',integer),at('Any','/anyOf/1',number)]};
let calls = 0;
const observed = (source:ValidationSource,value:ReturnType<typeof parseJson>,trace?:ValidationTrace) => {calls++;return validate(source,value,trace);};
const choice = createCodec<bigint|JsonNumber>(root('Any'),0,{symbols:[at('Any','',expression)],maxDepth:128,maxSteps:1000,maxIntegerDigits:4096},observed);
assert(JsonNumber.is(choice.decode('4'))); assert.equal(calls,1);
assert.equal(choice.decode('6'),6n); assert.equal(calls,2);
assert(JsonNumber.is(choice.decode('6.5'))); assert.equal(calls,3);
assert.equal(choice.encode(6n),'6'); assert.equal(calls,4);
const typeUnion = codec<bigint|JsonNumber>('TypeUnion',{kind:'union',alternatives:[integer,number]});
assert.equal(typeUnion.decode('4'),4n); assert(JsonNumber.is(typeUnion.decode('4.5')));
const one = codec<bigint|JsonNumber>('One',{kind:'union',alternatives:[at('One','/oneOf/0',integer),at('One','/oneOf/1',number)]});
error('invalid',()=>one.decode('4')); assert(JsonNumber.is(one.decode('4.5')));
"#,
    );
}

#[test]
#[ignore = "requires native TypeScript and Node.js"]
fn native_codec_merges_compatible_object_array_and_map_intersections() {
    let record = |name: &str, value: Value| json!({"type":"object","properties":{name:value}});
    execute(
        json!({
          "Disjoint":{"allOf":[record("left",json!({"type":"integer"})),record("right",json!({"type":"string"}))]},
          "Nested":{"allOf":[record("items",json!({"type":"array","items":record("id",json!({"type":"integer"}))})),record("items",json!({"type":"array","items":record("name",json!({"type":"string"}))}))]},
          "Map":{"allOf":[{"type":"object","additionalProperties":record("id",json!({"type":"integer"}))},{"type":"object","additionalProperties":record("name",json!({"type":"string"}))}]},
          "Array":{"allOf":[{"type":"array","items":{"type":"integer"}},{"type":"array","items":{}}]},
          "NeutralNumber":{"allOf":[{"type":"integer","minimum":0,"maximum":10},{"type":"number"}]}
        }),
        r#"
const record = (name:string,expression:Conversion):Conversion => ({kind:'object',fields:[{name,required:false,expression}],extra:any});
const disjoint = codec<any>('Disjoint',{kind:'intersection',members:[at('Disjoint','/allOf/0',record('left',integer)),at('Disjoint','/allOf/1',record('right',string))]});
const d = disjoint.decode('{"left":9007199254740993,"right":"r","untouched":1.5}');
assert.equal(d.left,9007199254740993n);assert.equal(d.right,'r');assert(JsonNumber.is(d.untouched));
const nested = codec<any>('Nested',{kind:'intersection',members:[
 at('Nested','/allOf/0',record('items',{kind:'array',item:record('id',integer)})),
 at('Nested','/allOf/1',record('items',{kind:'array',item:record('name',string)}))
]});
const n = nested.decode('{"items":[{"id":9007199254740993,"name":"item","other":1}]}');
assert.equal(n.items[0].id,9007199254740993n);assert.equal(n.items[0].name,'item');assert(JsonNumber.is(n.items[0].other));
const map = codec<any>('Map',{kind:'intersection',members:[
 at('Map','/allOf/0',{kind:'object',fields:[],extra:record('id',integer)}),
 at('Map','/allOf/1',{kind:'object',fields:[],extra:record('name',string)})
]});
const m = map.decode('{"__proto__":{"id":1,"name":"entry"}}');
assert.equal(m.__proto__.id,1n);assert.equal(m.__proto__.name,'entry');assert.equal(Object.getPrototypeOf(m),null);
const array = codec<bigint[]>('Array',{kind:'intersection',members:[{kind:'array',item:integer},{kind:'array',item:any}]});
assert.deepEqual(array.decode('[1,2]'),[1n,2n]);
const neutral = codec<number>('NeutralNumber',{kind:'intersection',members:[{kind:'safeInteger'},{kind:'anyNumber'}]});
assert.equal(neutral.decode('7'),7);
"#,
    );
}

#[test]
#[ignore = "requires native TypeScript and Node.js"]
fn native_codec_limits_survive_unions_recursion_and_validation_failures() {
    let mut values: Vec<Value> = (0..20).map(|i| json!(format!("option{i}"))).collect();
    values.push(json!(7));
    execute(serde_json::from_str(&format!(r##"{{
      "Union":{{"anyOf":[{{"type":"integer"}},{{"type":"number"}}]}},
      "Many":{{"enum":{}}},"Safe":{{"type":"integer","minimum":0,"maximum":10}},
      "Node":{{"type":"object","properties":{{"value":{{"type":"integer"}},"next":{{"$ref":"#/components/schemas/Node"}}}}}},
      "Bad":{{"anyOf":[true,{{"$ref":"#/components/schemas/Bad"}}]}}
    }}"##,json!(values))).unwrap(),r#"
const union = codec<bigint|JsonNumber>('Union',{kind:'union',alternatives:[integer,number]},{maxIntegerDigits:3});
assert(JsonNumber.is(union.decode('1.5')));
error('limit',()=>union.decode('1e1000')); // must not fall through to number
error('limit',()=>codec<number>('Safe',{kind:'safeInteger'},{maxIntegerDigits:0}).decode('7'));
const alternatives:Conversion[] = Array.from({length:20},(_,i)=>({kind:'literal',value:'option'+i}));
alternatives.push({kind:'integerLiteral',value:'7',safe:false});
const many = codec<bigint>('Many',{kind:'union',alternatives},{maxSteps:20});
error('limit',()=>many.decode('7'));
assert.equal(codec<bigint>('Many',{kind:'union',alternatives},{maxSteps:1000}).decode('7'),7n);
const node:Conversion = at('Node','',{kind:'object',fields:[{name:'value',required:false,expression:integer},{name:'next',required:false,expression:{kind:'reference',target:0}}],extra:any});
const recursive = createCodec<any>(root('Node'),0,{symbols:[node],maxDepth:12,maxSteps:10000,maxIntegerDigits:4096},validate);
assert.equal(recursive.decode('{"next":{"value":7}}').next.value,7n);
error('limit',()=>recursive.decode('{"next":{"next":{"next":{"next":{"next":{}}}}}}'));
error('evaluation',()=>codec<unknown>('Bad',any).decode('null'));
error('limit',()=>codec<unknown>('Bad',any).decode('null',{maxNodes:0}));
"#);
}

#[test]
#[ignore = "requires native TypeScript/Node.js and OPENROUTER_WEB_ROOT"]
fn tracked_openrouter_caller_and_index_decode_and_encode_in_native_runtime() {
    let root_dir = std::path::PathBuf::from(
        std::env::var_os("OPENROUTER_WEB_ROOT").expect("set OPENROUTER_WEB_ROOT"),
    );
    let path = root_dir.join("projects/docs/openapi/openapi.yaml");
    let workspace = Arc::new(WorkspaceBuilder::new().root(&root_dir).build().unwrap());
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap());
    let caller = contract
        .schemas()
        .find(|schema| schema.id().pointer() == "/components/schemas/ORAnthropicNullableCaller")
        .unwrap()
        .id()
        .clone();
    let index = contract
        .schemas()
        .find(|schema| schema.id().pointer() == "/components/schemas/ChatChoice/properties/index")
        .unwrap()
        .id()
        .clone();
    let branches = contract.schema(&caller).unwrap().raw()["oneOf"]
        .as_array()
        .unwrap();
    assert_eq!(branches.len(), 4);
    assert!(
        branches[0]["$ref"]
            .as_str()
            .unwrap()
            .ends_with("DirectCaller")
    );
    assert!(
        branches[1]["$ref"]
            .as_str()
            .unwrap()
            .ends_with("CodeExecution20250825Caller")
    );
    assert!(
        branches[2]["$ref"]
            .as_str()
            .unwrap()
            .ends_with("CodeExecution20260120Caller")
    );
    assert_eq!(branches[3]["type"], "null");
    let compiled = OwnedCompiler::new(Config::default())
        .compile(contract, &[caller, index])
        .unwrap();
    execute_program(
        &compiled.program(),
        r#"
type Caller = null | {type:'direct';[name:string]:unknown} | {type:'code_execution_20250825'|'code_execution_20260120';tool_id:string;[name:string]:unknown};
const branch = (type:string, tool:boolean):Conversion => ({kind:'object',fields:[{name:'type',required:true,expression:{kind:'literal',value:type}},...(tool?[{name:'tool_id',required:true,expression:string}]:[])],extra:any});
const caller = codec<Caller>('ORAnthropicNullableCaller',{kind:'union',alternatives:[
 at('ORAnthropicNullableCaller','/oneOf/0',branch('direct',false)),
 at('ORAnthropicNullableCaller','/oneOf/1',branch('code_execution_20250825',true)),
 at('ORAnthropicNullableCaller','/oneOf/2',branch('code_execution_20260120',true)),
 at('ORAnthropicNullableCaller','/oneOf/3',{kind:'null'})
]});
assert.equal(caller.decode('null'),null);
assert.equal(caller.decode('{"type":"direct"}')!.type,'direct');
const execution = caller.decode('{"type":"code_execution_20260120","tool_id":"tool"}')!;
assert.equal(execution.type,'code_execution_20260120');assert.equal(execution.tool_id,'tool');
assert.equal(caller.encode(execution),'{"type":"code_execution_20260120","tool_id":"tool"}');
assert.equal(caller.encode({type:'code_execution_20250825',tool_id:'tool'}),'{"type":"code_execution_20250825","tool_id":"tool"}');
error('invalid',()=>caller.decode('{"type":"code_execution_20260120"}'));
error('invalid',()=>caller.encode({type:'unknown'} as any));
const index = codec<bigint>('ChatChoice/properties/index',integer);
assert.equal(index.decode('9007199254740993'),9007199254740993n);
assert.equal(index.encode(9007199254740993n),'9007199254740993');
error('invalid',()=>index.decode('1e-400'));
"#,
    );
}
