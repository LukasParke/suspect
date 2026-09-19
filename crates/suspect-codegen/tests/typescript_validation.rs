//! Generated native validation consumes compiled instructions, never raw OAS.

use std::{path::Path, process::Command, sync::Arc};

use serde_json::{Value, json};
use suspect_codegen::{OutFile, typescript::validation::emit};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_ref::WorkspaceBuilder;
use suspect_schema::{Config, OwnedCompiler, OwnedOutcome, OwnedProgram, OwnedSchema};
use suspect_source::Uri;

fn compile(schemas: Value, config: Config) -> (OwnedSchema, Vec<SchemaId>) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(&path, json!({"openapi":"3.1.0","info":{"title":"TS validation","version":"1"},"components":{"schemas":schemas}}).to_string()).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(directory.path())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap());
    let roots = contract.schema_roots().to_vec();
    let schema = OwnedCompiler::new(config)
        .compile(contract, &roots)
        .unwrap();
    (schema, roots)
}

fn native(programs: &[OwnedProgram], cases: &Value, extra: &str) {
    let directory = tempfile::tempdir().unwrap();
    let mut files = Vec::new();
    let mut imports = String::new();
    for (index, program) in programs.iter().enumerate() {
        for mut file in emit(program).unwrap() {
            if file.path == "typescript/validation-program.ts" {
                file.path = format!("typescript/program{index}.ts");
                files.push(file);
            } else if index == 0 {
                files.push(file);
            }
        }
        imports.push_str(&format!(
            "import * as program{index} from './program{index}.js';\n"
        ));
    }
    let consumer = format!(
        r#"{imports}
import {{ JsonNumber, parseJson }} from './json.js';
declare function require(name: string): any;
const assert = require('node:assert/strict');
const programs = [{}];
const cases = JSON.parse({});
for (const test of cases) {{
    const program = programs[test.program]!;
    const root = program.validationRoots.find(source => source.pointer === test.pointer)!;
    assert(root, test.pointer);
    const value = parseJson(test.input, {{maxNumberLength:10000, maxDepth:1000}});
    const result = program.validate(root, value);
    assert.equal(result.kind, test.expected, test.pointer + ': ' + test.input + ': ' + JSON.stringify(result));
    if (result.kind === 'invalid' && test.findings) {{
        assert.deepEqual(result.findings.map(f => [f.source.document, f.source.pointer, f.instancePath]), test.findings);
    }}
    if (result.kind === 'evaluationFailure' && test.location) {{
        assert.deepEqual([result.finding.source.document,result.finding.source.pointer,result.finding.instancePath],test.location);
    }}
}}
{extra}
"#,
        (0..programs.len())
            .map(|i| format!("program{i}"))
            .collect::<Vec<_>>()
            .join(","),
        serde_json::to_string(&cases.to_string()).unwrap()
    );
    files.push(OutFile {
        path: "typescript/consumer.ts".into(),
        content: consumer,
    });
    suspect_codegen::write_files(&files, directory.path()).unwrap();
    let root = directory.path().join("typescript");
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
        .expect("native tsc required");
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
fn versioned_shared_runtime_vectors_use_independent_expected_results() {
    let vectors: Value =
        serde_json::from_str(include_str!("fixtures/runtime-contract-v1.json")).unwrap();
    let schemas = vectors["cases"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .map(|(index, case)| (format!("Case{index:02}"), case["schema"].clone()))
        .collect::<serde_json::Map<_, _>>();
    let (compiled, _) = compile(Value::Object(schemas), Config::default());
    let mut cases = Vec::new();
    for (index, case) in vectors["cases"].as_array().unwrap().iter().enumerate() {
        for expected in ["valid", "invalid"] {
            for input in case[expected].as_array().unwrap() {
                cases.push(json!({"program":0,"pointer":format!("/components/schemas/Case{index:02}"),"input":input,"expected":expected}));
            }
        }
    }
    native(&[compiled.program()], &json!(cases), "");
}

fn case(program: usize, compiled: &OwnedSchema, root: &SchemaId, input: &str) -> Value {
    let value: Value = serde_json::from_str(input).unwrap();
    let mut case = json!({"program":program,"pointer":root.pointer(),"input":input});
    match compiled.validate(root, &value) {
        OwnedOutcome::Valid => case["expected"] = json!("valid"),
        OwnedOutcome::Invalid(findings) => {
            case["expected"] = json!("invalid");
            case["findings"] = json!(
                findings
                    .iter()
                    .map(|f| (
                        f.source.document().to_string(),
                        f.source.pointer(),
                        f.instance_path.to_path()
                    ))
                    .collect::<Vec<_>>()
            );
        }
        OwnedOutcome::EvaluationFailure(finding) => {
            case["expected"] = json!("evaluationFailure");
            case["location"] = json!([
                finding.source.document().to_string(),
                finding.source.pointer().to_owned(),
                finding.instance_path.to_path()
            ]);
        }
    }
    case
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn emission_is_lossless_and_rejects_unsafe_or_inconsistent_metadata() {
    let (schema, _) = compile(serde_json::from_str(r#"{"Model":{"enum":[9007199254740993,1e-400,{"__proto__":18446744073709551616}]},"Zero":{"maxItems":-0e9}}"#).unwrap(),Config::default());
    let program = schema.program();
    let files = emit(&program).unwrap();
    let code = &files
        .iter()
        .find(|f| f.path.ends_with("validation-program.ts"))
        .unwrap()
        .content;
    assert!(code.contains("parseJson(\"9007199254740993\""));
    assert!(code.contains("parseJson(\"1e-400\""));
    assert!(!code.contains("\"values\":[9007199254740993"));
    let mut malformed = program.clone();
    malformed.limits.max_evaluation_steps = usize::MAX;
    assert!(emit(&malformed).is_err());
    malformed = program.clone();
    malformed.roots[0].target = program.nodes.len();
    assert!(emit(&malformed).is_err());
    malformed = program.clone();
    malformed.roots[0].source.pointer.push_str("/wrong");
    assert!(emit(&malformed).is_err());
    malformed = program;
    malformed.version = "unknown";
    assert!(emit(&malformed).is_err());
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn source_prose_in_documentation_is_inert() {
    let (schema, _) = compile(
        json!({"line\n<script>alert(1)</script>`![x](https://example.test)":{"type":"string"}}),
        Config::default(),
    );
    let files = emit(&schema.program()).unwrap();
    let docs = &files
        .iter()
        .find(|f| f.path.ends_with("validation.md"))
        .unwrap()
        .content;
    assert!(!docs.contains("<script>"));
    assert!(!docs.contains("![x]"));
    assert!(docs.contains("&#10;"));
    assert!(docs.contains("&#60;script&#62;"));
}

#[test]
#[ignore = "requires native TypeScript and Node.js"]
fn native_validation_preserves_exact_numbers_composition_presence_and_locations() {
    let schemas: Value = serde_json::from_str(r##"{
      "Integer":{"type":"integer"},"Range":{"minimum":9007199254740993,"exclusiveMaximum":9007199254740994},
      "Multiple":{"multipleOf":0.01},"Tiny":{"minimum":1e-400,"maximum":1e-400},
      "Enum":{"enum":[9007199254740993,1e-400,{"x":1.0,"__proto__":null}]},
      "Const":{"const":{"a/b~":[9007199254740993,1e-400]}},"Unique":{"uniqueItems":true},
      "Count":{"minLength":2,"maxLength":3,"minItems":2,"maxItems":3,"minProperties":2,"maxProperties":3},
      "HugeCount":{"maxItems":1e999999999999999999999999,"minProperties":-0e9},
      "Object":{"type":"object","properties":{"a/b~":{"type":["integer","null"]}},"required":["a/b~"],"additionalProperties":false},
      "Array":{"prefixItems":[{"type":"string"}],"items":{"type":"integer"}},
      "Choice":{"oneOf":[{"type":"integer"},{"type":"number"}]},
      "Any":{"anyOf":[{"required":["a","b"]},{"type":"integer"}]},
      "Not":{"not":{"const":9007199254740993}},
      "Node":{"type":"object","properties":{"value":{"type":"integer"},"next":{"$ref":"#/components/schemas/Node"}}},
      "RefSibling":{"$ref":"#/components/schemas/Integer","minimum":5}
    }"##).unwrap();
    let (compiled, roots) = compile(schemas, Config::default());
    let mut cases = Vec::new();
    for (name, inputs) in [
        (
            "Integer",
            vec![
                "7",
                "7.0",
                "70e-1",
                "1e-400",
                "1e999999999999999999999999",
                "null",
            ],
        ),
        (
            "Range",
            vec![
                "9007199254740992",
                "9007199254740993",
                "9007199254740994",
                "\"text\"",
            ],
        ),
        (
            "Multiple",
            vec![
                "0.29",
                "0.29000000000000001",
                "-0.29",
                "1e-400",
                "1e999999999999999999999999",
            ],
        ),
        ("Tiny", vec!["1e-400", "0", "1.1e-400"]),
        (
            "Enum",
            vec![
                "9007199254740993",
                "9007199254740992",
                "1e-400",
                "{\"__proto__\":null,\"x\":10e-1}",
            ],
        ),
        (
            "Const",
            vec![
                "{\"a/b~\":[9007199254740993,1e-400]}",
                "{\"a/b~\":[9007199254740992,1e-400]}",
            ],
        ),
        (
            "Unique",
            vec![
                "[1,1.0]",
                "[1e-400,0]",
                "[{\"a\":1,\"b\":2},{\"b\":2.0,\"a\":10e-1}]",
            ],
        ),
        (
            "Count",
            vec!["\"𝄞a\"", "\"𝄞\"", "[1,2]", "[1]", "{}", "{\"a\":1,\"b\":2}"],
        ),
        ("HugeCount", vec!["[1,2,3]", "{}"]),
        (
            "Object",
            vec![
                "{}",
                "{\"a/b~\":null}",
                "{\"a/b~\":1.5}",
                "{\"a/b~\":7,\"extra\":1}",
            ],
        ),
        (
            "Array",
            vec!["[]", "[\"a\",1,2.0]", "[\"a\",1.5]", "[1]", "true"],
        ),
        ("Choice", vec!["7", "7.5", "null"]),
        ("Any", vec!["7", "{}", "{\"a\":1,\"b\":2}"]),
        ("Not", vec!["9007199254740993", "9007199254740992"]),
        (
            "Node",
            vec![
                "{\"value\":1,\"next\":{\"value\":2}}",
                "{\"next\":{\"value\":1.1}}",
            ],
        ),
        ("RefSibling", vec!["4", "5", "5.1"]),
    ] {
        let root = roots
            .iter()
            .find(|r| r.pointer() == format!("/components/schemas/{name}"))
            .unwrap();
        for input in inputs {
            cases.push(case(0, &compiled, root, input));
        }
    }
    native(
        &[compiled.program()],
        &json!(cases),
        r#"
const root = program0.validationRoots.find(r => r.pointer.endsWith('/Integer'))!;
assert.equal(program0.validate({document:root.document,pointer:'/not-selected'},parseJson('7')).kind,'evaluationFailure');
const Base = JsonNumber as unknown as new(token: string, max: number) => JsonNumber;
class Forged extends Base { override toString(): string { return '7'; } }
const branded = new Forged('1.5',4096);
assert.equal(program0.validate(root,branded).kind,'invalid');
const subclass = Object.setPrototypeOf(Object.create(null), Forged.prototype);
assert.equal(program0.validate(root,subclass).kind,'evaluationFailure');
for(let i=0;i<32;i++) assert.equal(program0.validate(root,parseJson('7')).kind,'valid');
"#,
    );
}

#[test]
#[ignore = "requires native TypeScript and Node.js"]
fn native_patterns_match_ecma_unicode_semantics_and_share_evaluation_budgets() {
    let expressions = [
        "",
        "a",
        "^a$",
        "^.$",
        "^[^]$",
        "[]",
        "[^a-z]",
        r"^\w+$",
        r"^\d+$",
        r"^\s$",
        r"^[\w-]+$",
        r"^a\.b$",
        r"^(a|ab)+$",
        r"^(a?)*$",
        r"^(?:ab){1,3}$",
        r"^a{2,}$",
        r"^(a+)+$",
        r"^💩+$",
        r"[a-z]{0,2}x",
    ];
    let schemas = expressions
        .iter()
        .enumerate()
        .map(|(index, pattern)| (format!("Pattern{index}"), json!({"pattern":pattern})))
        .collect::<serde_json::Map<_, _>>();
    let (compiled, roots) = compile(Value::Object(schemas), Config::default());
    let mut cases = Vec::new();
    let inputs = [
        "",
        "a",
        "aa",
        "aaa",
        "ab",
        "abab",
        "ababab",
        "abababab",
        "aab",
        "xay",
        "a\n",
        "a.b",
        "a\\b",
        "aéb",
        "é",
        "💩",
        "💩💩",
        "1",
        "١",
        " ",
        "\u{a0}",
        "\u{feff}",
        "\u{85}",
        "\n",
        "\r",
        "\u{2028}",
        "\u{2029}",
        "sess_abc123",
        "aax",
        "aaax",
    ];
    for root in &roots {
        for input in inputs {
            cases.push(case(0, &compiled, root, &json!(input).to_string()));
        }
        for input in [json!(null), json!(7), json!([]), json!({})] {
            let result = case(0, &compiled, root, &input.to_string());
            assert_eq!(
                result["expected"], "valid",
                "pattern applies only to strings"
            );
            cases.push(result);
        }
    }
    let oracle = format!(
        r#"
const expressions: string[] = {};
const inputs: string[] = {};
for (let index = 0; index < expressions.length; index++) {{
    const root = program0.validationRoots.find(r => r.pointer.endsWith('/Pattern' + index))!;
    const regex = new RegExp(expressions[index]!, 'u');
    for (const input of inputs) {{
        const actual = program0.validate(root, input);
        assert.equal(actual.kind, regex.test(input) ? 'valid' : 'invalid', JSON.stringify([expressions[index], input, actual]));
    }}
}}
const scalarRoot = program0.validationRoots.find(r => r.pointer.endsWith('/Pattern3'))!;
const nonScalar = program0.validate(scalarRoot, parseJson('"\\ud800"'));
assert.equal(nonScalar.kind, 'evaluationFailure');
if (nonScalar.kind === 'evaluationFailure') {{
    assert.equal(nonScalar.finding.source.pointer, scalarRoot.pointer + '/pattern');
    assert.equal(nonScalar.finding.instancePath, '');
}}
"#,
        serde_json::to_string(&expressions).unwrap(),
        serde_json::to_string(&inputs).unwrap()
    );
    native(&[compiled.program()], &json!(cases), &oracle);

    let expensive = json!({"pattern":"^(a+)+$"});
    let (limited, roots) = compile(
        json!({"Pattern":expensive,"Not":{"not":expensive},"Any":{"anyOf":[true,expensive]},"One":{"oneOf":[true,expensive]}}),
        Config {
            max_evaluation_steps: 64,
            ..Config::default()
        },
    );
    let input = json!(format!("{}!", "a".repeat(100_000))).to_string();
    let limited_cases = roots
        .iter()
        .map(|root| {
            let result = case(0, &limited, root, &input);
            assert_eq!(result["expected"], "evaluationFailure");
            result
        })
        .collect::<Vec<_>>();
    native(&[limited.program()], &json!(limited_cases), "");
}

#[test]
#[ignore = "requires native TypeScript and Node.js"]
fn pinned_official_vectors_execute_through_compiled_program_and_native_typescript() {
    let fixture_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../suspect-schema/tests/conformance/draft2020-12");
    let mut schemas = serde_json::Map::new();
    let mut vectors = Vec::new();
    for fixture in [
        "type",
        "minimum",
        "maximum",
        "exclusiveMinimum",
        "exclusiveMaximum",
        "multipleOf",
        "minLength",
        "maxLength",
        "minItems",
        "maxItems",
        "minProperties",
        "maxProperties",
        "enum",
        "const",
        "uniqueItems",
    ] {
        let groups: Value = serde_json::from_slice(
            &std::fs::read(fixture_root.join(format!("{fixture}.json"))).unwrap(),
        )
        .unwrap();
        for (index, group) in groups.as_array().unwrap().iter().enumerate() {
            let name = format!("{fixture}_{index}");
            schemas.insert(name.clone(), group["schema"].clone());
            for test in group["tests"].as_array().unwrap() {
                vectors.push((
                    name.clone(),
                    test["data"].to_string(),
                    test["valid"].as_bool().unwrap(),
                ));
            }
        }
    }
    let (compiled, roots) = compile(Value::Object(schemas), Config::default());
    let mut cases = Vec::new();
    for (name, input, valid) in vectors {
        let root = roots
            .iter()
            .find(|r| r.pointer() == format!("/components/schemas/{name}"))
            .unwrap();
        let result = case(0, &compiled, root, &input);
        assert_eq!(
            result["expected"],
            if valid { "valid" } else { "invalid" },
            "{name}: {input}"
        );
        cases.push(result);
    }
    assert!(
        cases.len() >= 300,
        "normative cases must be executed, not skipped"
    );
    native(&[compiled.program()], &json!(cases), "");
}

#[test]
#[ignore = "requires native TypeScript and Node.js"]
fn native_limits_are_global_non_invertible_and_preserve_locations() {
    let mut programs = Vec::new();
    let mut cases = Vec::new();
    let expensive = json!({"allOf":vec![true;40]});
    let invalid = json!({"required":["missing1","missing2","missing3"]});
    let mut branches = vec![invalid; 40];
    branches.push(json!(true));
    for (schemas, config, inputs) in [
        (json!({"Not":{"not":expensive},"Any":{"anyOf":[true,expensive]},"One":{"oneOf":[true,expensive]},"InvalidTrials":{"anyOf":branches},"Wide":{"additionalProperties":true}}),
            Config {max_evaluation_steps:32,max_errors:1,..Config::default()},
            vec![("Not",json!(7)),("Any",json!(7)),("One",json!(7)),("InvalidTrials",json!({})),("Wide",Value::Object((0..100).map(|i|(format!("k{i}"),json!(i))).collect()))]),
        (json!({"Negated":{"not":{"const":[1,2]}},"Any":{"anyOf":[true,{"const":[1,2]}]},"Enum":{"enum":[null,1]}}),
            Config {max_equality_steps:1,..Config::default()},
            vec![("Negated",json!([1,2])),("Any",json!([1,2])),("Enum",json!(1))]),
        (json!({"Integer":{"type":"integer"},"Number":{"type":["integer","number"]},"Enum":{"enum":[1000,"x"]},"Unicode":{"const":{"\u{e000}":0,"\u{10000}":1000}}}),
            Config {max_number_bytes:1,..Config::default()},
            vec![("Integer",json!(1000)),("Number",json!(1000)),("Enum",json!(1000)),("Enum",json!("x")),("Unicode",json!({"\u{e000}":1,"\u{10000}":1000})),("Unicode",json!({"\u{e000}":0,"\u{10000}":1000}))]),
        (json!({"Any":true}),Config {max_evaluation_steps:0,..Config::default()},vec![("Any",json!(null))]),
        (serde_json::from_str(r##"{"Cycle":{"$ref":"#/components/schemas/Cycle"},"Node":{"type":"object","properties":{"next":{"$ref":"#/components/schemas/Node"}}}}"##).unwrap(),
            Config {max_depth:8,..Config::default()},
            vec![("Cycle",json!(null)),("Node",json!({"next":{"next":{"next":{"next":{"next":{}}}}}}))]),
    ] {
        let (compiled,roots) = compile(schemas,config);
        let index = programs.len();
        for (name,input) in inputs {
            let root = roots.iter().find(|root| root.pointer() == format!("/components/schemas/{name}")).unwrap();
            cases.push(case(index,&compiled,root,&input.to_string()));
        }
        programs.push(compiled.program());
    }
    assert!(
        cases
            .iter()
            .filter(|case| case["expected"] == "evaluationFailure")
            .count()
            >= 12
    );
    native(&programs, &json!(cases), "");
}

#[test]
#[ignore = "requires native TypeScript and Node.js"]
fn native_trace_observes_completed_branches_without_resetting_budgets() {
    let mut programs = Vec::new();
    for steps in [8, 7] {
        let (compiled, _) = compile(
            json!({"Choice":{"anyOf":[{"type":"integer"},{"type":"string"}]}}),
            Config {
                max_evaluation_steps: steps,
                ..Config::default()
            },
        );
        programs.push(compiled.program());
    }
    native(
        &programs,
        &json!([]),
        r#"
const root = program0.validationRoots[0]!;
const value = parseJson('7');
const seen: [string,string,boolean][] = [];
assert.equal(program0.validate(root,value).kind,'valid');
assert.equal(program0.validate(root,value,(source,path,valid)=>seen.push([source.pointer,path,valid])).kind,'valid');
assert.deepEqual(seen,[[root.pointer+'/anyOf/0','',true],[root.pointer+'/anyOf/1','',false],[root.pointer,'',true]]);
assert.equal(program0.validate(root,value,()=>{throw new Error('observer failure');}).kind,'evaluationFailure');
const tight = program1.validationRoots[0]!;
const incomplete: string[] = [];
assert.equal(program1.validate(tight,value).kind,'evaluationFailure');
assert.equal(program1.validate(tight,value,(source)=>incomplete.push(source.pointer)).kind,'evaluationFailure');
assert.deepEqual(incomplete,[tight.pointer+'/anyOf/0']);
"#,
    );
}

#[test]
#[ignore = "requires native TypeScript/Node.js and OPENROUTER_WEB_ROOT"]
fn tracked_openrouter_index_and_messages_run_in_native_typescript() {
    let root_dir = std::path::PathBuf::from(
        std::env::var_os("OPENROUTER_WEB_ROOT").expect("set OPENROUTER_WEB_ROOT"),
    );
    let path = root_dir.join("projects/docs/openapi/openapi.yaml");
    let workspace = Arc::new(WorkspaceBuilder::new().root(&root_dir).build().unwrap());
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap());
    let index = contract
        .schemas()
        .find(|schema| schema.id().pointer() == "/components/schemas/ChatChoice/properties/index")
        .unwrap()
        .id()
        .clone();
    let messages = contract
        .schemas()
        .find(|schema| {
            schema.id().pointer() == "/components/schemas/ChatRequest/properties/messages"
        })
        .unwrap()
        .id()
        .clone();
    let compiled = OwnedCompiler::new(Config::default())
        .compile(contract, &[index.clone(), messages.clone()])
        .unwrap();
    let mut cases = Vec::new();
    for (root, input, valid) in [
        (&index, "0", true),
        (&index, "9007199254740993", true),
        (&index, "1e-400", false),
        (&index, "\"0\"", false),
        (&messages, "[]", false),
        (&messages, r#"[{"role":"user","content":"Hello!"}]"#, true),
        (&messages, r#"[{"role":"user"}]"#, false),
        (
            &messages,
            r#"[{"role":"unknown","content":"Hello!"}]"#,
            false,
        ),
    ] {
        let result = case(0, &compiled, root, input);
        assert_eq!(
            result["expected"],
            if valid { "valid" } else { "invalid" },
            "{input}"
        );
        cases.push(result);
    }
    native(&[compiled.program()], &json!(cases), "");
}
