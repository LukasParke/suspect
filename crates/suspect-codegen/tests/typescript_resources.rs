//! Source-driven native resource/dynamic validation through the checked v3 seam.

#![cfg(feature = "http-protocol")]

use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::typescript::validation::emit;
use suspect_ir::contract::{Contract, SchemaId};
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_schema::{Config, OwnedCompiler, OwnedProgram};
use suspect_source::Uri;

const ENTRY: &str = "https://physical.test/api.json";
const OFFICIAL: &str = "https://physical.test/official-schema.json";

fn source(document: &str, pointer: &str) -> SchemaId {
    let root = SchemaId::new(Uri::parse(document).unwrap(), Default::default());
    pointer.strip_prefix('/').map_or(root.clone(), |path| {
        path.split('/').fold(root, |id, key| {
            id.child(&key.replace("~1", "/").replace("~0", "~"))
        })
    })
}
fn root(name: &str) -> SchemaId {
    source(ENTRY, "/components/schemas").child(name)
}
fn api(schemas: Value) -> Value {
    json!({"openapi":"3.2.0","info":{"title":"Native TypeScript resources","version":"1"},"paths":{},"components":{"schemas":schemas}})
}
fn load(document: Value, dependencies: Vec<(&str, &str, Vec<u8>)>) -> Arc<Contract> {
    load_at(ENTRY, document, dependencies)
}
fn load_at(
    physical: &str,
    document: Value,
    dependencies: Vec<(&str, &str, Vec<u8>)>,
) -> Arc<Contract> {
    let mut inputs = vec![(ENTRY, physical, serde_json::to_vec(&document).unwrap())];
    inputs.extend(dependencies);
    let provider = Arc::new(
        DocumentProvider::new(inputs.into_iter().map(|(requested, physical, bytes)| {
            ProvidedDocument::new(
                Uri::parse(requested).unwrap(),
                Uri::parse(physical).unwrap(),
                bytes,
            )
            .unwrap()
        }))
        .unwrap(),
    );
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::parse(ENTRY).unwrap()).unwrap())
}
fn compile(contract: Arc<Contract>, roots: &[SchemaId], config: Config) -> OwnedProgram {
    let program = OwnedCompiler::new(config)
        .compile_v3(contract, roots)
        .unwrap()
        .program();
    program.check().unwrap();
    assert_eq!(program.version, OwnedProgram::V3_VERSION);
    program
}
fn tree() -> Arc<Contract> {
    load(
        api(json!({
            "Tree":{"$id":"urn:tree","$dynamicAnchor":"node","type":"object","properties":{"data":true,"children":{"type":"array","items":{"$dynamicRef":"#node"}}}},
            "Strict":{"$id":"urn:strict","$dynamicAnchor":"node","$ref":"urn:tree","unevaluatedProperties":false}
        })),
        vec![],
    )
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn v3_metadata_and_dynamic_ops_cannot_enter_older_or_mismatched_envelopes() {
    let program = compile(tree(), &[root("Strict")], Config::default());
    for (version, profile) in [
        (OwnedProgram::V1_VERSION, OwnedProgram::V1_PROFILE),
        (OwnedProgram::V2_VERSION, OwnedProgram::V2_PROFILE),
        (OwnedProgram::V3_VERSION, OwnedProgram::V2_PROFILE),
        ("unknown", OwnedProgram::V3_PROFILE),
    ] {
        let mut changed = program.clone();
        changed.version = version;
        changed.profile = profile;
        assert!(emit(&changed).is_err());
        changed.resource_context = None;
        assert!(emit(&changed).is_err());
    }
}

fn official_vectors() -> (Vec<OwnedProgram>, Value) {
    let groups: Vec<Value> = serde_json::from_str(include_str!(
        "../../suspect-schema/tests/fixtures/resource-conformance/dynamicRef.json"
    ))
    .unwrap();
    let mut programs = Vec::new();
    let mut cases = Vec::new();
    for group in groups {
        let contract=load(api(json!({"Use":{"$ref":OFFICIAL}})),vec![
            (OFFICIAL,OFFICIAL,serde_json::to_vec(&group["schema"]).unwrap()),
            ("http://localhost:1234/draft2020-12/tree.json","http://localhost:1234/draft2020-12/tree.json",include_bytes!("../../suspect-schema/tests/fixtures/resource-conformance/tree.json").to_vec()),
            ("http://localhost:1234/draft2020-12/extendible-dynamic-ref.json","http://localhost:1234/draft2020-12/extendible-dynamic-ref.json",include_bytes!("../../suspect-schema/tests/fixtures/resource-conformance/extendible-dynamic-ref.json").to_vec()),
            ("http://localhost:1234/draft2020-12/detached-dynamicref.json","http://localhost:1234/draft2020-12/detached-dynamicref.json",include_bytes!("../../suspect-schema/tests/fixtures/resource-conformance/detached-dynamicref.json").to_vec()),
        ]);
        let index = programs.len();
        programs.push(compile(
            contract,
            &[source(OFFICIAL, "")],
            Config::default(),
        ));
        for case in group["tests"].as_array().unwrap() {
            cases.push(json!({"program":index,"root":0,"id":format!("{} / {}",group["description"].as_str().unwrap(),case["description"].as_str().unwrap()),"instanceJson":case["data"].to_string(),"expected":if case["valid"].as_bool().unwrap(){"valid"}else{"invalid"}}));
        }
    }
    assert_eq!(cases.len(), 44);
    (programs, json!(cases))
}

fn checked(command: &mut Command, root: &Path) -> std::process::Output {
    let result = command.output().unwrap();
    assert!(
        result.status.success(),
        "{}\n{command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    result
}
fn working_directory(label: &str) -> (PathBuf, Option<tempfile::TempDir>) {
    if let Some(base) = std::env::var_os("SUSPECT_TYPESCRIPT_V3_ARTIFACTS") {
        std::fs::create_dir_all(&base).unwrap();
        let path = tempfile::Builder::new()
            .prefix(label)
            .tempdir_in(base)
            .unwrap()
            .keep();
        println!("retained TypeScript v3 evidence: {}", path.display());
        (path, None)
    } else {
        let temporary = tempfile::tempdir().unwrap();
        (temporary.path().to_owned(), Some(temporary))
    }
}
fn native(label: &str, programs: &[OwnedProgram], cases: &Value, extra: &str) {
    let (root, _temporary) = working_directory(label);
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
            "import * as p{index} from './dist/program{index}.js';\n"
        ));
    }
    suspect_codegen::write_files(&files, &root).unwrap();
    let generated = root.join("typescript");
    std::fs::write(
        root.join("programs.json"),
        serde_json::to_vec_pretty(programs).unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.join("cases.json"),
        serde_json::to_vec_pretty(cases).unwrap(),
    )
    .unwrap();
    std::fs::write(
        generated.join("package.json"),
        "{\"type\":\"module\",\"private\":true}",
    )
    .unwrap();
    let script = format!(
        "{imports}\nimport assert from 'node:assert/strict';\nimport {{parseJson}} from './dist/json.js';\nimport {{createValidator}} from './dist/validation.js';\nconst programs=[{}],cases={cases};\nfor(const test of cases){{const program=programs[test.program],root=program.validationRoots[test.root],value=parseJson(test.instanceJson,{{maxDepth:2000,maxNumberLength:100000}});const result=program.validate(root,value);assert.equal(result.kind,test.expected,test.id+': '+JSON.stringify(result));assert.deepEqual(program['validateRoot'+test.root](root,value),result,test.id+': selected root');if(test.source){{const findings=result.kind==='invalid'?result.findings:[result.finding];assert.ok(findings.some(f=>f.source.document===test.source.document&&f.source.pointer===test.source.pointer&&f.instancePath===test.instancePath),test.id+': '+JSON.stringify(result));}}}}\n{extra}\nconsole.log('resource-vectors',cases.length,process.version);\n",
        (0..programs.len())
            .map(|index| format!("p{index}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    let node = std::env::var_os("SUSPECT_DOCS_NODE").unwrap_or_else(|| "node".into());
    let tools = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools");
    let matrix = std::env::var_os("SUSPECT_TYPESCRIPT_V3_MATRIX").is_some();
    let compilers = if matrix {
        vec![
            (
                "5.5.4",
                tools.join("typescript-floor/node_modules/typescript/bin/tsc"),
            ),
            (
                "5.9.3",
                tools.join("typescript-docs/node_modules/typescript/bin/tsc"),
            ),
        ]
    } else {
        vec![(
            "5.9.3",
            tools.join("typescript-docs/node_modules/typescript/bin/tsc"),
        )]
    };
    if matrix {
        assert!(
            std::env::var_os("SUSPECT_NODE24_BIN").is_some(),
            "matrix requires SUSPECT_NODE24_BIN"
        );
    }
    for (version, compiler) in compilers {
        let actual = checked(
            Command::new(&node).arg(&compiler).arg("--version"),
            &generated,
        );
        assert_eq!(
            String::from_utf8_lossy(&actual.stdout).trim(),
            format!("Version {version}")
        );
        let dist = format!("dist-{version}");
        let mut command = Command::new(&node);
        command.arg(compiler).current_dir(&generated).args([
            "--strict",
            "--exactOptionalPropertyTypes",
            "--noUncheckedIndexedAccess",
            "--target",
            "ES2022",
            "--module",
            "NodeNext",
            "--moduleResolution",
            "NodeNext",
            "--declaration",
            "--outDir",
            &dist,
        ]);
        for index in 0..programs.len() {
            command.arg(format!("program{index}.ts"));
        }
        checked(&mut command, &generated);
        let consumer = format!("consumer-{version}.mjs");
        std::fs::write(
            generated.join(&consumer),
            script.replace("'./dist/", &format!("'./{dist}/")),
        )
        .unwrap();
        println!("resource validator compiler: Version {version}");
        for node in std::iter::once(node.clone()).chain(std::env::var_os("SUSPECT_NODE24_BIN")) {
            let result = checked(
                Command::new(node).current_dir(&generated).arg(&consumer),
                &generated,
            );
            println!("{}", String::from_utf8_lossy(&result.stdout).trim());
        }
    }
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn source_driven_v3_executes_all_44_official_cases_from_closed_supplied_documents() {
    let (programs, cases) = official_vectors();
    native("official-", &programs, &cases, "");
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn native_v3_scope_identity_lookup_budgets_and_malformed_metadata_controls() {
    let mut programs = Vec::new();
    let mut cases = Vec::new();
    let mut add = |label: &str,
                   contract: Arc<Contract>,
                   roots: &[SchemaId],
                   selected: &SchemaId,
                   config: Config,
                   input: &str,
                   expected: &str,
                   finding: Option<(&str, &str, &str)>| {
        let program = compile(contract, roots, config);
        let selected = program
            .roots
            .iter()
            .position(|root| {
                root.source.document == selected.document().as_str()
                    && root.source.pointer == selected.pointer()
            })
            .unwrap();
        cases.push(json!({"program":programs.len(),"root":selected,"id":label,"instanceJson":input,"expected":expected,
            "source":finding.map(|(document,pointer,_)|json!({"document":document,"pointer":pointer})),"instancePath":finding.map(|(_,_,path)|path)}));
        programs.push(program);
    };
    for (name, input, expected) in [
        (
            "Strict",
            r#"{"data":null,"children":[{"data":9007199254740993}]}"#,
            "valid",
        ),
        ("Strict", r#"{"children":[{"extra":1}]}"#, "invalid"),
        ("Tree", r#"{"children":[{"extra":1}]}"#, "valid"),
    ] {
        add(
            name,
            tree(),
            &[root("Strict"), root("Tree")],
            &root(name),
            Config::default(),
            input,
            expected,
            if expected == "invalid" {
                Some((
                    ENTRY,
                    "/components/schemas/Strict/unevaluatedProperties",
                    "/children/0/extra",
                ))
            } else {
                None
            },
        );
    }
    let bindings = load(
        api(json!({
            "Base":{"$id":"urn:base","$dynamicAnchor":"entry","type":"object","properties":{"outer":true,"middle":true,"children":{"items":{"$dynamicRef":"#entry"}}}},
            "Middle":{"$id":"urn:middle","$dynamicAnchor":"entry","$ref":"urn:base","required":["middle"]},
            "Outer":{"$id":"urn:outer","$dynamicAnchor":"entry","$ref":"urn:middle","required":["outer"]},
            "Unentered":{"$id":"urn:unentered","$dynamicAnchor":"entry","not":{}}
        })),
        vec![],
    );
    add(
        "outermost entered binding",
        bindings.clone(),
        &[root("Outer"), root("Unentered")],
        &root("Outer"),
        Config::default(),
        r#"{"outer":true,"middle":true,"children":[{"outer":true,"middle":true}]}"#,
        "valid",
        None,
    );
    add(
        "outermost does not become nearest",
        bindings,
        &[root("Outer"), root("Unentered")],
        &root("Outer"),
        Config::default(),
        r#"{"outer":true,"middle":true,"children":[{"middle":true}]}"#,
        "invalid",
        Some((ENTRY, "/components/schemas/Outer/required", "/children/0")),
    );
    let contexts = load(
        api(json!({
            "Fallback":{"$id":"urn:deny","$dynamicAnchor":"flag","not":{}},
            "Start":{"$id":"urn:start","if":{"$dynamicRef":"urn:deny#flag"},"then":true,"else":{"$ref":"urn:extended"}},
            "Extended":{"$id":"urn:extended","$defs":{"accept":{"$dynamicAnchor":"flag"}},"$ref":"urn:start"},
            "String":{"$id":"urn:string","$dynamicAnchor":"node","type":"string"},
            "Failed":{"$id":"urn:failed","$dynamicAnchor":"node","not":{}},
            "Trial":{"anyOf":[{"$ref":"urn:failed"},{"$dynamicRef":"urn:string#node"}]}
        })),
        vec![],
    );
    add(
        "context-changing revisit is progress",
        contexts.clone(),
        &[root("Start"), root("Trial")],
        &root("Start"),
        Config::default(),
        "7",
        "valid",
        None,
    );
    add(
        "trial restores entered resources",
        contexts,
        &[root("Start"), root("Trial")],
        &root("Trial"),
        Config::default(),
        "\"value\"",
        "valid",
        None,
    );
    let outer = "https://physical.test/outer.json";
    let base = "https://physical.test/base.json";
    let nested=load(api(json!({"Use":{"$ref":format!("{outer}#/$defs/start")}})),vec![
        (outer,outer,serde_json::to_vec(&json!({"$id":"urn:nested","const":false,"$defs":{"binding":{"$dynamicAnchor":"value","type":"integer"},"start":{"$ref":"urn:base#/$defs/use"}}})).unwrap()),
        (base,base,serde_json::to_vec(&json!({"$id":"urn:base","$defs":{"binding":{"$dynamicAnchor":"value","type":"string"},"use":{"$dynamicRef":"#value"}}})).unwrap()),
    ]);
    let selected = source(outer, "/$defs/start");
    add(
        "nested entry enters resource without evaluating root",
        nested.clone(),
        std::slice::from_ref(&selected),
        &selected,
        Config::default(),
        "9007199254740993",
        "valid",
        None,
    );
    add(
        "unvisited in-resource binding is active",
        nested,
        std::slice::from_ref(&selected),
        &selected,
        Config::default(),
        "\"fallback must not win\"",
        "invalid",
        Some((outer, "/$defs/binding/type", "")),
    );
    let fallback = load(
        api(json!({
            "Base":{"$id":"urn:base","type":"string","$defs":{"Target":{"$dynamicAnchor":"node","$anchor":"plain","type":"string"}}},
            "Outer":{"$id":"urn:outer","$defs":{"Override":{"$dynamicAnchor":"node","type":"integer"}},"properties":{
                "dynamic":{"$dynamicRef":"urn:base#n%6fde"},"pointer":{"$dynamicRef":"urn:base#/$defs/Target"},"plain":{"$dynamicRef":"urn:base#plain"},"empty":{"$dynamicRef":"urn:base#"},"static":{"$ref":"urn:base#node"}
            }}
        })),
        vec![],
    );
    add(
        "only a dynamic plain name overrides",
        fallback.clone(),
        &[root("Outer")],
        &root("Outer"),
        Config::default(),
        r#"{"dynamic":7,"pointer":"s","plain":"s","empty":"s","static":"s"}"#,
        "valid",
        None,
    );
    add(
        "pointer fallback retains initial assertion",
        fallback,
        &[root("Outer")],
        &root("Outer"),
        Config::default(),
        r#"{"pointer":7}"#,
        "invalid",
        Some((
            ENTRY,
            "/components/schemas/Base/$defs/Target/type",
            "/pointer",
        )),
    );
    for (limit, expected) in [(1, "evaluationFailure"), (2, "valid")] {
        add(
            "distinct resource entry charge",
            load(api(json!({"Empty":{"$id":"urn:empty"}})), vec![]),
            &[root("Empty")],
            &root("Empty"),
            Config {
                max_evaluation_steps: limit,
                ..Config::default()
            },
            "null",
            expected,
            if limit == 1 {
                Some((ENTRY, "/components/schemas/Empty", ""))
            } else {
                None
            },
        );
    }
    for (limit, expected) in [(5, "evaluationFailure"), (6, "valid")] {
        add(
            "resource and binding scan charge",
            load(
                api(
                    json!({"Lookup":{"$id":"urn:lookup","$defs":{"value":{"$dynamicAnchor":"v"}},"$dynamicRef":"#v"}}),
                ),
                vec![],
            ),
            &[root("Lookup")],
            &root("Lookup"),
            Config {
                max_evaluation_steps: limit,
                ..Config::default()
            },
            "null",
            expected,
            if limit == 5 {
                Some((ENTRY, "/components/schemas/Lookup/$defs/value", ""))
            } else {
                None
            },
        );
    }
    for use_site in [
        json!({"if":{"$dynamicRef":"urn:numeric#value"},"then":true,"else":true}),
        json!({"anyOf":[true,{"$dynamicRef":"urn:numeric#value"}]}),
        json!({"not":{"$dynamicRef":"urn:numeric#value"}}),
    ] {
        add(
            "dynamic numeric failure is noninvertible",
            load(
                api(
                    json!({"Use":use_site,"Numeric":{"$id":"urn:numeric","$dynamicAnchor":"value","maximum":0}}),
                ),
                vec![],
            ),
            &[root("Use")],
            &root("Use"),
            Config {
                max_number_bytes: 3,
                max_errors: 1,
                ..Config::default()
            },
            "12345",
            "evaluationFailure",
            Some((ENTRY, "/components/schemas/Numeric/maximum", "")),
        );
    }
    add(
        "dynamic cycle cannot be suppressed by true sibling",
        load(
            api(
                json!({"Cycle":{"$id":"urn:cycle","$dynamicAnchor":"v","anyOf":[true,{"$dynamicRef":"#v"}]}}),
            ),
            vec![],
        ),
        &[root("Cycle")],
        &root("Cycle"),
        Config::default(),
        "null",
        "evaluationFailure",
        Some((ENTRY, "/components/schemas/Cycle", "")),
    );
    let guarded = serde_json::to_string(&programs[0]).unwrap();
    let extra=r#"
const make=()=>JSON.parse(__PROGRAM__);
const frozen=make();createValidator(frozen);assert.ok(Object.isFrozen(frozen.resourceContext.resources[0].dynamicAnchors));
const accessor=make(),nodes=accessor.nodes;let getterCalls=0;
Object.defineProperty(accessor,'nodes',{enumerable:true,get(){getterCalls++;return nodes;}});
assert.throws(()=>createValidator(accessor));assert.equal(getterCalls,0,'program admission cannot execute mutable metadata accessors');
for(const mutate of [
    p=>delete p.resourceContext,p=>p.resourceContext.nodeScopes.pop(),
    p=>p.resourceContext.nodeScopes[0][0]=99999,p=>p.resourceContext.nodeScopes[0][2]='urn:invented',
    p=>p.resourceContext.nodeScopes[0][1].document='https://wrong.test/schema',
    p=>p.resourceContext.resources[0].aliases=[],
    p=>p.resourceContext.resources[1].aliases.push(p.resourceContext.resources[0].canonicalUri),
    p=>p.resourceContext.resources[0].baseUri='https://[broken',
    p=>p.resourceContext.resources[0].declarationSource={document:p.nodes[0].source.document,pointer:'/invented'},
    p=>p.resourceContext.resources.find(r=>r.dynamicAnchors.length).dynamicAnchors[0][2]=99999,
    p=>p.resourceContext.resources.find(r=>r.dynamicAnchors.length).dynamicAnchors[0][0]+='\n',
    p=>p.nodes.flatMap(n=>n.checks).find(c=>c.op==='dynamicRef').initialResource=99999,
    p=>p.nodes.flatMap(n=>n.checks).find(c=>c.op==='dynamicRef').anchor='not/anchor',
    p=>p.nodes.flatMap(n=>n.checks).find(c=>c.op==='dynamicRef').anchor='unbound',
    p=>{p.version='suspect.validation.experimental.v2';p.profile='oas31-jsonschema202012-static-applicators';},
    p=>{p.version='suspect.validation.experimental.v1';p.profile='oas31-jsonschema202012-static-subset';delete p.resourceContext;}
]){const program=make();mutate(program);assert.throws(()=>createValidator(program));}
import {parseUriReference,resolveUriDocument,uriKey,encodeUriFragment} from './dist/uri.js';
for(const [relative,expected] of [
    ['g:h','g:h'],['g','http://a/b/c/g'],['../g','http://a/b/g'],['../../g','http://a/g'],['../../../g','http://a/g'],
    ['/./g','http://a/g'],['g/../h','http://a/b/c/h'],['?y','http://a/b/c/d;p?y'],['#s','http://a/b/c/d;p?q'],
    ['g?y/../x','http://a/b/c/g?y/../x'],['//g','http://g'],['http:g','http:g'],['%2E%2E/x','http://a/b/c/%2E%2E/x'],['x%2Fy','http://a/b/c/x%2Fy']
])assert.equal(resolveUriDocument('http://a/b/c/d;p?q',relative),expected,relative);
assert.equal(uriKey('https://A.test/schema#n%6fde'),'https://a.test/schema#node');
assert.equal(encodeUriFragment('/a/b~% #é'),'/a/b~%25%20%23%C3%A9');
for(const value of [' leading','trailing ','a\\b','café','bad%','bad%2','bad%GG','1bad:scheme','https://[broken/a','https://example.test:port/a','a#b#c'])assert.throws(()=>parseUriReference(value),value);
for(const ending of ['\n','\r\n','\u2028','\u2029'])for(const prefix of ['https://a.test/path','https://a.test:80','urn:resource#name'])assert.throws(()=>parseUriReference(prefix+ending));
assert.throws(()=>uriKey('https://a.test/schema#%FF'));
assert.throws(()=>resolveUriDocument('scheme:','.///path'));
"#.replace("__PROGRAM__",&serde_json::to_string(&guarded).unwrap());
    native("controls-", &programs, &json!(cases), &extra);
}

fn sdk_document() -> Value {
    let mut document = api(json!({
        "Tree":{"$id":"urn:tree","$dynamicAnchor":"node","type":"object","required":["value"],"properties":{"value":{"type":"integer"},"children":{"type":"array","items":{"$dynamicRef":"#node"}}}},
        "Strict":{"$id":"urn:strict","$dynamicAnchor":"node","$ref":"urn:tree","unevaluatedProperties":false},
        "Exact":{"$id":"models/exact","type":"object","required":["amount"],"properties":{"amount":{"type":"number","minimum":0}},"additionalProperties":false},
        "Contextual":{"$id":"urn:contextual","$ref":"urn:context-base","if":{"$ref":"urn:probe"},"then":true,"else":true},
        "ContextBase":{"$id":"urn:context-base","type":"object","properties":{"value":{"$dynamicRef":"urn:integer#leaf"}}},
        "Integer":{"$id":"urn:integer","$dynamicAnchor":"leaf","type":"integer"},
        "Probe":{"$id":"urn:probe","$defs":{"text":{"$dynamicAnchor":"leaf","type":"string"}},"$ref":"urn:context-base"}
    }));
    document["$self"] = json!("https://logical.test/catalog/api.json#revision");
    document["servers"] = json!([{"url":"https://unused.fixture.test"}]);
    for (name, reference, value) in [
        (
            "strict",
            "urn:strict",
            json!({"value":1,"children":[{"value":2}]}),
        ),
        (
            "loose",
            "urn:tree",
            json!({"value":1,"children":[{"value":2,"extra":true}]}),
        ),
        (
            "exact",
            "https://logical.test/catalog/models/exact",
            serde_json::from_str(r#"{"amount":9007199254740993.25}"#).unwrap(),
        ),
        ("contextual", "urn:contextual", json!({"value":7})),
    ] {
        document["paths"][format!("/{name}")] = json!({"post":{"operationId":name,"requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":reference},"example":value.clone()}}},"responses":{"200":{"description":"source validated","content":{"application/json":{"schema":{"$ref":reference},"example":value}}}}}});
    }
    document
}
fn http_plan(contract: Arc<Contract>) -> suspect_codegen::typescript::http::HttpPlan {
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    suspect_codegen::typescript::http::plan_http(
        contract,
        &selected,
        suspect_codegen::typescript::http::HttpConfig::expanded(),
    )
    .unwrap()
}

fn installed(
    label: &str,
    plan: &suspect_codegen::typescript::http::HttpPlan,
    types: &str,
    runtime: &str,
) {
    use suspect_codegen::typescript::package::{PackageConfig, emit_http};
    let (root, _temporary) = working_directory(label);
    let files = emit_http(
        plan,
        &PackageConfig {
            name: "@fixture/resource-sdk".into(),
            version: "0.0.0".into(),
        },
    )
    .unwrap();
    suspect_codegen::write_files(&files, &root).unwrap();
    let package = root.join("typescript");
    let selected_node = std::env::var_os("SUSPECT_DOCS_NODE").unwrap_or_else(|| "node".into());
    let executable = checked(
        Command::new(&selected_node).args(["--print", "process.execPath"]),
        &root,
    );
    let node = PathBuf::from(String::from_utf8(executable.stdout).unwrap().trim());
    let npm = node
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("lib/node_modules/npm/bin/npm-cli.js");
    let npm_command = |cwd: &Path| {
        let mut command = Command::new(&node);
        command.arg(&npm).current_dir(cwd);
        command.env(
            "PATH",
            std::env::join_paths(std::iter::once(node.parent().unwrap().to_owned()).chain(
                std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
            ))
            .unwrap(),
        );
        command
    };
    checked(
        npm_command(&package).args([
            "ci",
            "--offline",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
        ]),
        &root,
    );
    let floor = root.join("floor");
    std::fs::create_dir(&floor).unwrap();
    let tools = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools");
    for file in ["package.json", "package-lock.json"] {
        std::fs::copy(tools.join("typescript-floor").join(file), floor.join(file)).unwrap();
    }
    checked(
        npm_command(&floor).args([
            "ci",
            "--offline",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
        ]),
        &root,
    );
    let node24 = std::env::var_os("SUSPECT_NODE24_BIN")
        .expect("set SUSPECT_NODE24_BIN for the second required runtime");
    for (version, compiler) in [
        ("5.9.3", package.join("node_modules/typescript/bin/tsc")),
        ("5.5.4", floor.join("node_modules/typescript/bin/tsc")),
    ] {
        let actual = checked(Command::new(&node).arg(&compiler).arg("--version"), &root);
        assert_eq!(
            String::from_utf8_lossy(&actual.stdout).trim(),
            format!("Version {version}")
        );
        checked(
            Command::new(&node)
                .arg(&compiler)
                .current_dir(&package)
                .args(["--project", "tsconfig.json"]),
            &root,
        );
        if version == "5.9.3" {
            checked(
                Command::new(&node)
                    .arg(tools.join("typescript-docs/build.mjs"))
                    .arg(&package),
                &root,
            );
        }
        let packed = checked(
            npm_command(&package).args(["pack", "--offline", "--ignore-scripts", "--json"]),
            &root,
        );
        let packed: Value = serde_json::from_slice(&packed.stdout).unwrap();
        let archive = root.join(format!("resource-sdk-ts{version}.tgz"));
        std::fs::rename(
            package.join(packed[0]["filename"].as_str().unwrap()),
            &archive,
        )
        .unwrap();
        let consumer = root.join(format!("consumer-{version}"));
        std::fs::create_dir(&consumer).unwrap();
        std::fs::write(
            consumer.join("package.json"),
            "{\"type\":\"module\",\"private\":true}",
        )
        .unwrap();
        checked(
            npm_command(&consumer)
                .args([
                    "install",
                    "--offline",
                    "--ignore-scripts",
                    "--no-audit",
                    "--no-fund",
                ])
                .arg(&archive),
            &root,
        );
        std::fs::write(consumer.join("consumer.ts"), types).unwrap();
        std::fs::write(consumer.join("consumer.mjs"), runtime).unwrap();
        checked(
            Command::new(&node)
                .arg(&compiler)
                .current_dir(&consumer)
                .args([
                    "--strict",
                    "--exactOptionalPropertyTypes",
                    "--noUncheckedIndexedAccess",
                    "--target",
                    "ES2022",
                    "--module",
                    "NodeNext",
                    "--moduleResolution",
                    "NodeNext",
                    "--noEmit",
                    "consumer.ts",
                ]),
            &root,
        );
        for node in [node.clone().into_os_string(), node24.clone()] {
            checked(
                Command::new(&node)
                    .current_dir(&consumer)
                    .arg("node_modules/@fixture/resource-sdk/dist/examples/validated.js"),
                &root,
            );
            let result = checked(
                Command::new(&node)
                    .current_dir(&consumer)
                    .arg("consumer.mjs"),
                &root,
            );
            println!(
                "TypeScript {version}: {}",
                String::from_utf8_lossy(&result.stdout).trim()
            );
        }
    }
}

#[test]
#[ignore = "requires pinned Node 22/24, offline npm and TypeScript 5.5/5.9, and TypeDoc"]
fn installed_v3_sdk_preserves_dynamic_models_exact_values_examples_and_native_docs() {
    use suspect_codegen::typescript::{ModelView, plan_models};
    let document = sdk_document();
    let contract = load(document.clone(), vec![]);
    let models = plan_models(&contract, contract.schema_roots(), &[ModelView::Neutral]);
    assert!(!models.has_errors(), "{:?}", models.diagnostics());
    assert!(models.diagnostics().iter().any(|finding|finding.code=="resource-validation-required"&&!finding.at.is_empty()));
    let plan = http_plan(contract);
    assert_eq!(
        plan.codecs().validation_profile(),
        (OwnedProgram::V3_VERSION, OwnedProgram::V3_PROFILE)
    );
    assert_eq!(plan.examples().operations().len(), 4);
    assert!(
        plan.examples()
            .operations()
            .iter()
            .all(|operation| operation
                .entries
                .iter()
                .filter(|entry| entry.origin == suspect_codegen::examples::ExampleOrigin::Declared)
                .count()
                == 2),
        "{:?}",
        plan.examples().diagnostics()
    );
    installed("installed-sdk-", &plan, V3_TYPES, V3_RUNTIME);
}

const V3_TYPES: &str = r#"
import { createClient, codecs, JsonNumber, type models } from '@fixture/resource-sdk';
const client=createClient();
export async function resources(){
    const value:models.Strict={value:1n,children:[{value:2n}]};
    const response=await client.strict({body:value});
    const integer:bigint=response.data.value;
    await client.loose({body:{value:1n,children:[{value:2n,extra:true}]}});
    await client.exact({body:{amount:JsonNumber.parse('9007199254740993.25')}});
    await client.contextual({body:{value:7n}});
    codecs.StrictCodec.encode(value);
    // @ts-expect-error the known native integer field stays required
    await client.strict({body:{children:[]}});
    // @ts-expect-error declared native integer representation is not a string
    await client.strict({body:{value:'1'}});
    // @ts-expect-error dynamic positions are a checked JSON carrier, not arbitrary functions
    await client.strict({body:{value:1n,children:[()=>true]}});
    // @ts-expect-error a general exact decimal is not silently widened to a JS number
    await client.exact({body:{amount:0.1}});
    // @ts-expect-error explicit undefined is distinct from omission
    const absent:models.Strict={value:1n,children:undefined};
    return integer;
}
"#;
const V3_RUNTIME: &str = r#"
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {createClient,operations,codecs,JsonNumber,ModelCodecError} from '@fixture/resource-sdk';
let calls=0;const requests=[];
let badResponse=false;
const server=createServer(async(request,response)=>{
    const chunks=[];for await(const chunk of request)chunks.push(chunk);calls++;requests.push(Buffer.concat(chunks).toString());
    response.writeHead(200,{'content-type':'application/json'});
    response.end(request.url==='/exact'?'{"amount":9007199254740993.25}':request.url==='/contextual'?'{"value":7}':request.url==='/strict'&&!badResponse?'{"value":9007199254740993,"children":[{"value":2}]}':'{"value":1,"children":[{"value":2,"extra":true}]}');
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
try{
    const client=createClient({serverURL:`http://127.0.0.1:${server.address().port}`});
    const strict=(await client.strict({body:{value:1n,children:[{value:9007199254740993n}]}})).data;
    assert.equal(strict.value,9007199254740993n);assert.equal(strict.children[0].value.toString(),'2');
    assert.equal((await client.loose({body:{value:1n,children:[{value:2n,extra:true}]}})).data.children[0].extra,true);
    assert.equal((await client.exact({body:{amount:JsonNumber.parse('9007199254740993.25')}})).data.amount.toString(),'9007199254740993.25');
    assert.equal((await client.contextual({body:{value:7n}})).data.value.toString(),'7','failed condition in another resource context must not overwrite the passing static projection trace');
    assert.ok(requests[0].includes('9007199254740993'));assert.equal(requests[2],'{"amount":9007199254740993.25}');
    await assert.rejects(client.strict({body:{value:1n,children:[{value:2n,extra:true}]}}),error=>operations.isSdkError(error)&&error.kind==='request-validation');
    await assert.rejects(client.strict({body:{value:1n,children:[null]}}),error=>operations.isSdkError(error)&&error.kind==='request-validation');
    assert.equal(calls,4,'invalid dynamic input is rejected before transport');
    strict.children[0].extra=true;
    assert.throws(()=>codecs.StrictCodec.encode(strict),error=>error instanceof ModelCodecError&&error.kind==='invalid'&&error.findings.some(f=>f.source.document==='https://physical.test/api.json'&&f.source.pointer==='/components/schemas/Strict/unevaluatedProperties'&&f.instancePath==='/children/0/extra'));
    assert.equal(codecs.TreeCodec.decode('{"value":1,"children":[{"value":2,"extra":null}]}').children[0].extra,null);
    assert.equal(Object.hasOwn(codecs.StrictCodec.decode('{"value":1}'),'children'),false);
    badResponse=true;await assert.rejects(client.strict({body:{value:1n}}),error=>operations.isSdkError(error)&&error.kind==='response-decoding');
    const metadata=operations.operationMetadata.strict.body.media[0].source;
    assert.equal(metadata.terminal.source.document,'https://physical.test/api.json');
    assert.equal(metadata.terminal_resource.canonical_uri,'https://logical.test/catalog/api.json#revision');
    assert.ok(Object.isFrozen(metadata.terminal_resource));
    console.log('resource SDK models/http/examples/docs',process.version);
}finally{await new Promise(resolve=>server.close(resolve));}
"#;

#[test]
#[ignore = "requires pinned Node 22/24, installed TypeScript 5.5/5.9 and TypeDoc"]
fn physical_document_server_bases_preserve_redirect_provenance_overrides_and_encoded_paths() {
    let requested = "http://requested.test/paths.json";
    let physical = "http://effective.test/cdn/spec/paths.json";
    let mut document = api(json!({}));
    document["$self"] = json!("https://entry.logical.test/catalog/api.json#revision");
    for (path, item) in [
        ("declared", "Declared"),
        ("absent", "Absent"),
        ("empty", "Empty"),
        ("encoded", "Encoded"),
    ] {
        document["paths"][format!("/{path}")] =
            json!({"$ref":format!("{requested}#/components/pathItems/{item}")});
    }
    let operation =
        |name: &str| json!({"operationId":name,"responses":{"204":{"description":"accepted"}}});
    let mut declared = operation("fromDocument");
    declared["security"] = json!([{"auth":["read"]}]);
    let external = json!({"openapi":"3.2.0","$self":"https://logical.test/catalog/paths.json#revision","info":{"title":"Resource-routed Path Items","version":"1"},"components":{
        "securitySchemes":{"auth":{"type":"oauth2","oauth2MetadataUrl":"../oauth-metadata","flows":{"clientCredentials":{"tokenUrl":"token","scopes":{"read":"Read API"}}}}},
        "pathItems":{
            "Declared":{"servers":[{"url":"../v%2F1/{region}","variables":{"region":{"default":"eu","enum":["eu","us"]}}}],"get":declared},
            "Absent":{"get":operation("absent")},
            "Empty":{"servers":[],"get":operation("empty")},
            "Encoded":{"servers":[{"url":"../%2e%2e/kept"}],"get":operation("encoded")}
        }
    }});
    let contract = load_at(
        "http://entry.test/specs/api.json",
        document,
        vec![(requested, physical, serde_json::to_vec(&external).unwrap())],
    );
    let plan = http_plan(contract);
    assert_eq!(
        plan.codecs().validation_profile().0,
        OwnedProgram::V1_VERSION,
        "metadata-only resources do not promote empty codec input closures"
    );
    let operation = |name: &str| {
        plan.protocol()
            .operations()
            .iter()
            .find(|operation| operation.operation_id().unwrap().value() == name)
            .unwrap()
    };
    assert_eq!(
        operation("fromDocument").servers().candidates()[0]
            .document_base()
            .source()
            .document()
            .as_str(),
        physical
    );
    assert_eq!(
        operation("absent").servers().candidates()[0]
            .document_base()
            .source()
            .document()
            .as_str(),
        "http://entry.test/specs/api.json"
    );
    assert_eq!(
        operation("empty").servers().candidates()[0]
            .document_base()
            .source()
            .document()
            .as_str(),
        physical
    );
    installed("physical-servers-", &plan, PHYSICAL_TYPES, PHYSICAL_RUNTIME);
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn resource_selection_keeps_base_profiles_and_source_linked_refusals() {
    use suspect_codegen::typescript::{
        ModelView,
        codecs::{CodecConfig, plan_codecs, plan_codecs_with_views},
    };
    let contract = load(
        api(json!({
            "Base":{"properties":{"a":true}},"Scoped":{"properties":{"a":true},"unevaluatedProperties":false},"Resource":{"$id":"urn:resource","type":"string"}
        })),
        vec![],
    );
    for (name, expected) in [
        ("Base", OwnedProgram::V1_VERSION),
        ("Scoped", OwnedProgram::V2_VERSION),
        ("Resource", OwnedProgram::V3_VERSION),
    ] {
        let plan = plan_codecs(contract.clone(), &[root(name)], CodecConfig::default()).unwrap();
        assert_eq!(plan.validation_profile().0, expected);
    }
    let base = OwnedCompiler::new(Config {
        max_evaluation_steps: 5,
        ..Config::default()
    })
    .compile(contract.clone(), &[root("Base")])
    .unwrap()
    .program();
    let additive = OwnedCompiler::new(Config {
        max_evaluation_steps: 5,
        ..Config::default()
    })
    .compile_v2(contract.clone(), &[root("Base")])
    .unwrap()
    .program();
    assert_eq!(
        serde_json::to_vec(&base).unwrap(),
        serde_json::to_vec(&additive).unwrap()
    );
    let scoped = OwnedCompiler::new(Config {
        max_evaluation_steps: 8,
        ..Config::default()
    })
    .compile_v2(contract, &[root("Scoped")])
    .unwrap()
    .program();
    assert!(base.resource_context.is_none() && scoped.resource_context.is_none());
    native(
        "base-profiles-",
        &[base, scoped],
        &json!([
            {"program":0,"root":0,"id":"frozen v1 budget","instanceJson":"{\"a\":1}","expected":"valid"},
            {"program":1,"root":0,"id":"frozen v2 budget","instanceJson":"{\"a\":1}","expected":"valid"}
        ]),
        r#"
for(const program of [p0,p1]){let calls=0;const result=program.validate(program.validationRoots[0],parseJson('{"a":1}'),function(){assert.equal(arguments.length,3,'old trace ABI stays three arguments');calls++;});assert.equal(result.kind,'valid');assert.ok(calls>0);}
"#,
    );
    for (schema, suffix) in [
        (
            json!({"$id":"urn:unsupported","$recursiveRef":"#"}),
            "/$recursiveRef",
        ),
        (
            json!({"$id":"urn:unsupported","$schema":"https://unsupported.test/dialect"}),
            "/$schema",
        ),
    ] {
        let contract = load(api(json!({"Unsupported":schema})), vec![]);
        let keyword = root("Unsupported").child(suffix.strip_prefix('/').unwrap());
        let span = contract.source_span(&keyword).unwrap();
        let errors =
            plan_codecs(contract, &[root("Unsupported")], CodecConfig::default()).unwrap_err();
        // Contract can own a keyword finding at its containing SchemaId with
        // the keyword's exact span. Retain that original location rather than
        // inventing a replacement source identity in the native adapter.
        assert!(
            errors.iter().any(|finding| (finding.source == keyword
                || finding.source == root("Unsupported"))
                && finding.at == span
                && finding.kind == suspect_codegen::typescript::DiagnosticKind::Error),
            "{errors:?}"
        );
    }
    let directional = load(
        api(json!({
            "Dynamic":{"$id":"urn:dynamic","$dynamicAnchor":"value","readOnly":true,"type":"string"},
            "Use":{"type":"object","required":["value"],"properties":{"value":{"$dynamicRef":"urn:dynamic#value"}}}
        })),
        vec![],
    );
    let errors = plan_codecs_with_views(
        directional,
        &[root("Use")],
        &[ModelView::Request],
        CodecConfig::default(),
    )
    .unwrap_err();
    assert!(errors.iter().any(
        |finding| finding.code == "directional-annotation-evaluation"
            && finding.source.pointer().ends_with("/$dynamicRef")
            && !finding.at.is_empty()
    ));
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn resource_capture_uses_typed_context_graphs_and_keeps_physical_locations_separate() {
    use suspect_codegen::{
        backend::{Backend, TargetConfig},
        compatibility::{self, PlanStatus},
    };
    let target = TargetConfig {
        backend: Backend::TypescriptHttp,
        package_name: "@fixture/resource-sdk".into(),
        package_version: "0.0.0".into(),
        import_name: None,
    };
    let selected = ["strict", "loose", "exact", "contextual"].map(str::to_owned);
    let document = sdk_document();
    let before = load(document.clone(), vec![]);
    let mut changed = document.clone();
    changed["info"]["description"] = json!("Relocated physical source and improved docs");
    changed["components"]["schemas"]["Strict"]["description"] = json!("Strict dynamic extension");
    let report = compatibility::compare(
        before.clone(),
        load_at("https://relocated.test/spec/api.json", changed, vec![]),
        &selected,
        std::slice::from_ref(&target),
    )
    .unwrap();
    let native = &report.native[0];
    assert_eq!(
        native.before.as_ref().unwrap().status,
        PlanStatus::Planned,
        "{:?}",
        native.before.as_ref().unwrap().findings
    );
    assert_eq!(
        native.after.as_ref().unwrap().status,
        PlanStatus::Planned,
        "{:?}",
        native.after.as_ref().unwrap().findings
    );
    assert!(native.changes.is_empty(), "{:?}", native.changes);
    assert_ne!(
        native.before.as_ref().unwrap().models[0].source.document,
        native.after.as_ref().unwrap().models[0].source.document
    );
    let strict = native
        .before
        .as_ref()
        .unwrap()
        .models
        .iter()
        .find(|model| model.name == "Strict")
        .unwrap();
    let graph = &strict.descriptor.as_ref().unwrap()["codec"]["validation"];
    assert_eq!(graph["version"], OwnedProgram::V3_VERSION);
    assert!(
        graph["resourceContext"]["resources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|resource| !resource["dynamicAnchors"].as_array().unwrap().is_empty())
    );
    assert!(
        graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|node| node["checks"].as_array().unwrap())
            .any(|check| check["op"] == "dynamicRef")
    );
    let (root, _temporary) = working_directory("capture-");
    std::fs::write(
        root.join("snapshot.json"),
        serde_json::to_vec_pretty(native.before.as_ref().unwrap()).unwrap(),
    )
    .unwrap();
    let mut changed = document;
    changed["components"]["schemas"]["Strict"]["$dynamicAnchor"] = json!("other");
    let report =
        compatibility::compare(before, load(changed, vec![]), &selected, &[target]).unwrap();
    assert_eq!(
        report.native[0].after.as_ref().unwrap().status,
        PlanStatus::Planned
    );
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|finding| finding.code == "native-model-shape-changed")
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn v3_example_admission_retains_dynamic_invalidity_and_declared_source() {
    let mut document = sdk_document();
    document["paths"]["/strict"]["post"]["requestBody"]["content"]["application/json"]["example"] =
        json!({"value":1,"children":[{"value":2,"extra":true}]});
    let plan = http_plan(load(document, vec![]));
    assert!(
        plan.examples()
            .diagnostics()
            .iter()
            .any(|finding| finding.code == "examples-declared-invalid"
                && finding.source.document().as_str() == ENTRY
                && finding.source.pointer()
                    == "/paths/~1strict/post/requestBody/content/application~1json/example"
                && !finding.at.is_empty()),
        "{:?}",
        plan.examples().diagnostics()
    );
    assert!(
        !plan
            .examples()
            .operations()
            .iter()
            .find(|operation| operation.operation_id == "strict")
            .unwrap()
            .entries
            .iter()
            .any(
                |entry| entry.role == suspect_codegen::examples::ExampleRole::RequestBody
                    && entry.value["children"][0].get("extra").is_some()
            )
    );
}

#[test]
#[ignore = "requires pinned Node 22, TypeScript 5.9 and isolated Chromium (SUSPECT_CHROMIUM)"]
fn browser_v3_executes_official_sources_dynamic_sdk_calls_and_contextual_codecs() {
    use suspect_codegen::typescript::package::{PackageConfig, emit_http};
    let mut document = sdk_document();
    document["paths"]["/strict"]["post"]["servers"] = json!([{"url":"../{base}","variables":{"base":{"default":"api","enum":["api","%2E%2E/kept"]}}}]);
    let plan = http_plan(load(document, vec![]));
    let mut files = emit_http(
        &plan,
        &PackageConfig {
            name: "@fixture/resource-sdk".into(),
            version: "0.0.0".into(),
        },
    )
    .unwrap();
    let (programs, cases) = official_vectors();
    for (index, program) in programs.iter().enumerate() {
        let mut file = emit(program)
            .unwrap()
            .into_iter()
            .find(|file| file.path == "typescript/validation-program.ts")
            .unwrap();
        file.path = format!("typescript/vector{index}.ts");
        files.push(file);
    }
    let (root, _temporary) = working_directory("browser-");
    suspect_codegen::write_files(&files, &root).unwrap();
    let generated = root.join("typescript");
    std::fs::write(
        generated.join("vectors.json"),
        serde_json::to_vec_pretty(&cases).unwrap(),
    )
    .unwrap();
    std::fs::write(
        generated.join("browser.mjs"),
        include_str!("fixtures/typescript-resources-browser.mjs"),
    )
    .unwrap();
    let node = std::env::var_os("SUSPECT_DOCS_NODE").unwrap_or_else(|| "node".into());
    let compiler = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tools/typescript-docs/node_modules/typescript/bin/tsc");
    let mut command = Command::new(&node);
    command.arg(compiler).current_dir(&generated).args([
        "--strict",
        "--exactOptionalPropertyTypes",
        "--noUncheckedIndexedAccess",
        "--target",
        "ES2022",
        "--module",
        "NodeNext",
        "--moduleResolution",
        "NodeNext",
        "--declaration",
        "--outDir",
        "dist",
        "source/index.ts",
        "examples/validated.ts",
        "examples/first-request.ts",
    ]);
    for index in 0..programs.len() {
        command.arg(format!("vector{index}.ts"));
    }
    checked(&mut command, &root);
    let result = checked(
        Command::new(node)
            .current_dir(&generated)
            .arg("browser.mjs"),
        &root,
    );
    println!("{}", String::from_utf8_lossy(&result.stdout).trim());
}

const PHYSICAL_TYPES: &str = r#"
import {createClient} from '@fixture/resource-sdk';
import type {CredentialContext,ResourceContext,ServerPlan} from '@fixture/resource-sdk/operations';
export function credentials(context:CredentialContext){
    const apiBase:string=context.effectiveServerURL;
    const metadata:ResourceContext|null|undefined=context.requirement.scheme.terminal_resource;
    // @ts-expect-error logical names are immutable metadata
    if(metadata)metadata.canonical_uri='https://rewritten.test';
    return {authorization:'Custom native'};
}
export async function calls(){const client=createClient({auth:{auth:credentials}});await client.fromDocument();await client.empty();await client.fromDocument({}, {server:{variables:{region:'us'},documentURL:'https://physical.test/spec.json'}});
    // @ts-expect-error explicit document-base overrides are URI strings
    await client.fromDocument({}, {server:{documentURL:7}});
    // @ts-expect-error source credential hooks return a complete Authorization value
    createClient({auth:{auth:'bare-token'}});
}
export function location(server:ServerPlan){const physical:string=server.document_base.source.document;return physical;}
"#;
const PHYSICAL_RUNTIME: &str = r#"
import assert from 'node:assert/strict';
import {createServer,request as httpRequest} from 'node:http';
import {createClient,operations} from '@fixture/resource-sdk';
const received=[],contexts=[];
const server=createServer((request,response)=>{received.push({path:request.url,host:request.headers.host,authorization:request.headers.authorization});response.writeHead(204);response.end();});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const local=`http://127.0.0.1:${server.address().port}`;
// A loopback transport supplies the fixture's virtual-host DNS/proxy routing.
// Request-target spelling and the physical authority stay literal on the wire.
const rawFetch=(input,init)=>new Promise((resolve,reject)=>{
    const match=/^http:\/\/([^/]+)(\/.*)$/.exec(String(input));assert.ok(match,'physical HTTP URI');
    const request=httpRequest({hostname:'127.0.0.1',port:server.address().port,method:init.method,path:match[2],headers:{...Object.fromEntries(init.headers),host:match[1]}},response=>{response.resume();response.on('end',()=>resolve(new Response(null,{status:response.statusCode})));});request.on('error',reject);request.end();
});
const auth={auth:context=>{contexts.push(context.effectiveServerURL);assert.equal(context.requirement.credential.metadata_url.value,'../oauth-metadata');assert.equal(context.requirement.credential.flows[0].token_url.value,'token');assert.equal(context.requirement.scheme.terminal.source.document,'http://effective.test/cdn/spec/paths.json');assert.equal(context.requirement.scheme.terminal_resource.canonical_uri,'https://logical.test/catalog/paths.json#revision');return {authorization:'Custom native'};}};
try{
    const client=createClient({auth,fetch:rawFetch});
    await client.fromDocument();await client.fromDocument({}, {server:{variables:{region:'us'}}});await client.absent();await client.empty();await client.encoded();
    assert.deepEqual(received.slice(0,5).map(({path,host})=>[path,host]),[
        ['/cdn/v%2F1/eu/declared','effective.test'],['/cdn/v%2F1/us/declared','effective.test'],['/absent','entry.test'],['/empty','effective.test'],['/cdn/%2e%2e/kept/encoded','effective.test']
    ]);
    const browserCompatible=createClient({auth});
    await browserCompatible.fromDocument({}, {server:{documentURL:local+'/overrides/root.json'}});
    assert.equal(received[5].path,'/v%2F1/eu/declared');
    assert.deepEqual(contexts,['http://effective.test/cdn/v%2F1/eu','http://effective.test/cdn/v%2F1/us',local+'/v%2F1/eu']);
    await assert.rejects(browserCompatible.encoded({}, {server:{documentURL:local+'/overrides/root.json'}}),error=>operations.isSdkError(error)&&error.kind==='request-representation');
    await assert.rejects(client.fromDocument({}, {server:{variables:{region:'invalid'}}}),error=>operations.isSdkError(error)&&error.kind==='request-representation');
    await assert.rejects(browserCompatible.fromDocument({}, {server:{documentURL:'file:///local/spec.json'}}),error=>operations.isSdkError(error)&&error.kind==='request-representation');
    assert.equal(received.length,6,'normalization/invalid overrides never reach transport; metadata endpoints are not acquired');
    const origin=operations.operationMetadata.fromDocument.source;
    assert.equal(origin.use_site.source.document,'http://entry.test/specs/api.json');assert.equal(origin.terminal.source.document,'http://effective.test/cdn/spec/paths.json');
    assert.equal(origin.terminal_resource.canonical_uri,'https://logical.test/catalog/paths.json#revision');
    assert.equal(operations.operationMetadata.absent.servers.candidates[0].document_base.source.document,'http://entry.test/specs/api.json');
    console.log('physical document server bases and exact wire paths',process.version);
}finally{await new Promise(resolve=>server.close(resolve));}
"#;
