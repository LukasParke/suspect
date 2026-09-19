//! Source-driven TypeScript adoption of the checked scoped v2 program contract.

use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::typescript::validation::emit;
use suspect_ir::contract::{Contract, SchemaId};
use suspect_ref::WorkspaceBuilder;
use suspect_schema::{Config, OwnedCompiler, OwnedProgram};
use suspect_source::Uri;

fn compile(schema: Value, config: Config) -> (OwnedProgram, SchemaId) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(&path,json!({"openapi":"3.1.0","info":{"title":"Scoped native validation","version":"1"},"components":{"schemas":{"Root":schema}}}).to_string()).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(directory.path())
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap());
    let root = contract
        .schema_roots()
        .iter()
        .find(|id| id.pointer() == "/components/schemas/Root")
        .unwrap()
        .clone();
    let compiled = OwnedCompiler::new(config)
        .compile_v2(contract, std::slice::from_ref(&root))
        .unwrap();
    (compiled.program(), root)
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn v2_instructions_cannot_enter_v1_or_unknown_program_envelopes() {
    let (program, _) = compile(
        json!({"if":{"type":"string"},"then":{"minLength":1},"unevaluatedProperties":false}),
        Config::default(),
    );
    assert_eq!(program.version, OwnedProgram::V2_VERSION);
    let mut malformed = program.clone();
    malformed.version = OwnedProgram::V1_VERSION;
    malformed.profile = OwnedProgram::V1_PROFILE;
    assert!(emit(&malformed).unwrap_err().source.is_some());
    malformed = program.clone();
    malformed.profile = OwnedProgram::V1_PROFILE;
    assert!(emit(&malformed).is_err());
    malformed = program;
    malformed.version = "unknown";
    assert!(emit(&malformed).is_err());
}

fn checked(command: &mut Command, root: &Path) -> std::process::Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn working_directory(label: &str) -> (PathBuf, Option<tempfile::TempDir>) {
    if let Some(base) = std::env::var_os("SUSPECT_TYPESCRIPT_V2_ARTIFACTS") {
        std::fs::create_dir_all(&base).unwrap();
        let directory = tempfile::Builder::new()
            .prefix(label)
            .tempdir_in(base)
            .unwrap()
            .keep();
        println!("retained TypeScript v2 evidence: {}", directory.display());
        (directory, None)
    } else {
        let directory = tempfile::tempdir().unwrap();
        (directory.path().to_owned(), Some(directory))
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
        root.join("source-vectors.json"),
        include_str!("../../suspect-schema/tests/fixtures/owned-applicators-v2.json"),
    )
    .unwrap();
    std::fs::write(
        generated.join("package.json"),
        "{\"type\":\"module\",\"private\":true}",
    )
    .unwrap();
    let script = format!(
        "{imports}\nimport assert from 'node:assert/strict';\nimport {{parseJson}} from './dist/json.js';\nimport {{createValidator}} from './dist/validation.js';\nconst programs=[{}],cases={};\nfor(const test of cases){{const program=programs[test.program];const root=program.validationRoots[0];const value=parseJson(test.instanceJson,{{maxDepth:2000,maxNumberLength:100000}});const result=program.validate(root,value);assert.equal(result.kind,test.expected,test.id+': '+JSON.stringify(result));assert.deepEqual(program.validateRoot0(root,value),result,test.id+' root-slice mismatch');if(test.source){{const findings=result.kind==='invalid'?result.findings:[result.finding];assert.ok(findings.some(f=>f.source.document===root.document&&f.source.pointer===test.source&&f.instancePath===test.instancePath),test.id+': '+JSON.stringify(result));}}}}\n{extra}\nconsole.log('scoped-vectors',cases.length,process.version);\n",
        (0..programs.len())
            .map(|i| format!("p{i}"))
            .collect::<Vec<_>>()
            .join(","),
        cases
    );
    let node = std::env::var_os("SUSPECT_DOCS_NODE").unwrap_or_else(|| "node".into());
    let matrix = std::env::var_os("SUSPECT_TYPESCRIPT_V2_MATRIX").is_some();
    let compilers = if matrix {
        let tools = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools");
        vec![
            (
                "5.5.4",
                Some(tools.join("typescript-floor/node_modules/typescript/bin/tsc")),
            ),
            (
                "5.9.3",
                Some(tools.join("typescript-docs/node_modules/typescript/bin/tsc")),
            ),
        ]
    } else {
        vec![("selected", None)]
    };
    if matrix {
        assert!(
            std::env::var_os("SUSPECT_NODE24_BIN").is_some(),
            "matrix requires SUSPECT_NODE24_BIN"
        );
    }
    for (version, path) in compilers {
        let command = || {
            if let Some(path) = &path {
                let mut command = Command::new(&node);
                command.arg(path);
                command
            } else {
                Command::new("tsc")
            }
        };
        let actual = checked(command().arg("--version"), &generated);
        if matrix {
            assert_eq!(
                String::from_utf8_lossy(&actual.stdout).trim(),
                format!("Version {version}")
            );
        }
        println!(
            "scoped validator compiler: {}",
            String::from_utf8_lossy(&actual.stdout).trim()
        );
        let dist = format!("dist-{version}");
        let mut compiler = command();
        compiler.current_dir(&generated).args([
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
            "--pretty",
            "false",
        ]);
        for index in 0..programs.len() {
            compiler.arg(format!("program{index}.ts"));
        }
        checked(&mut compiler, &generated);
        let consumer = format!("consumer-{version}.mjs");
        std::fs::write(
            generated.join(&consumer),
            script.replace("'./dist/", &format!("'./{dist}/")),
        )
        .unwrap();
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
fn source_driven_v2_vectors_preserve_scope_failures_and_original_locations() {
    let (programs, cases) = source_vectors();
    native("vectors-", &programs, &cases, "");
}

fn source_vectors() -> (Vec<OwnedProgram>, Value) {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../suspect-schema/tests/fixtures/owned-applicators-v2.json"
    ))
    .unwrap();
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 32);
    let mut programs = Vec::new();
    let mut cases = Vec::new();
    for (index, case) in fixture["cases"].as_array().unwrap().iter().enumerate() {
        let mut config = Config::default();
        if let Some(value) = case["limits"]["maxNumberBytes"].as_u64() {
            config.max_number_bytes = value as usize;
        }
        if let Some(value) = case["limits"]["maxEvaluationSteps"].as_u64() {
            config.max_evaluation_steps = value as usize;
        }
        let (program, _) = compile(
            serde_json::from_str(case["schemaJson"].as_str().unwrap()).unwrap(),
            config,
        );
        assert_eq!(program.version, OwnedProgram::V2_VERSION);
        let path = match case["id"].as_str().unwrap() {
            "contains-zero-does-not-mark-unmatched" | "contains-exact-integrality" => "/0",
            "contains-failure-after-exceeded-maximum" => "/1",
            "pattern-overlap-rejects" | "named-and-pattern-both-apply" => "/x",
            "property-names-checks-key-not-value" => "/long",
            "property-names-does-not-annotate-values" => "/ok",
            "failed-anyof-branch-does-not-leak"
            | "allof-cousins-have-independent-scopes"
            | "not-discards-annotations"
            | "required-is-not-an-evaluation" => "/a",
            "nested-members-do-not-mark-parent" => "/inner",
            "prefix-and-contains-leave-unmatched-item" => "/2",
            _ => "",
        };
        cases.push(json!({"program":index,"id":case["id"],"instanceJson":case["instanceJson"],"expected":match case["expected"].as_str().unwrap(){"Valid"=>"valid","Invalid"=>"invalid",_=>"evaluationFailure"},"source":case.get("source"),"instancePath":path}));
        programs.push(program);
    }
    (programs, json!(cases))
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn scoped_work_identity_and_mutation_controls_are_native_failures_not_mismatches() {
    let schemas = [
        (
            json!({"properties":{"a":true},"unevaluatedProperties":false}),
            Config {
                max_evaluation_steps: 7,
                ..Config::default()
            },
            r#"{"a":1}"#,
            "evaluationFailure",
            "/unevaluatedProperties",
            "",
        ),
        (
            json!({"properties":{"a":true},"unevaluatedProperties":false}),
            Config {
                max_evaluation_steps: 8,
                ..Config::default()
            },
            r#"{"a":1}"#,
            "valid",
            "",
            "",
        ),
        (
            json!({"anyOf":[{"properties":{"a":true}},{"properties":{"a":true}}],"unevaluatedProperties":false}),
            Config {
                max_evaluation_steps: 18,
                ..Config::default()
            },
            r#"{"a":1}"#,
            "evaluationFailure",
            "/anyOf",
            "",
        ),
        (
            json!({"anyOf":[{"properties":{"a":true}},{"properties":{"a":true}}],"unevaluatedProperties":false}),
            Config {
                max_evaluation_steps: 21,
                ..Config::default()
            },
            r#"{"a":1}"#,
            "valid",
            "",
            "",
        ),
        (
            json!({"propertyNames":{"$ref":"#/components/schemas/Root/propertyNames"}}),
            Config::default(),
            r#"{"a":1}"#,
            "evaluationFailure",
            "/propertyNames",
            "/a",
        ),
        (
            json!({"propertyNames":{"enum":["a/b~"]}}),
            Config {
                max_equality_steps: 0,
                ..Config::default()
            },
            r#"{"a/b~":null}"#,
            "evaluationFailure",
            "/propertyNames/enum",
            "/a~1b~0",
        ),
        (
            serde_json::from_str(
                r#"{"contains":true,"minContains":-0.0,"maxContains":1e999999999999999999999}"#,
            )
            .unwrap(),
            Config::default(),
            "[9007199254740993]",
            "valid",
            "",
            "",
        ),
        (
            json!({"contains":true}),
            Config {
                max_number_bytes: 0,
                ..Config::default()
            },
            "[12345]",
            "valid",
            "",
            "",
        ),
        (
            json!({"properties":{"a":true}}),
            Config {
                max_evaluation_steps: 5,
                ..Config::default()
            },
            r#"{"a":1}"#,
            "valid",
            "",
            "",
        ),
        (
            json!({"patternProperties":{"^x":{"type":"string"}},"additionalProperties":false}),
            Config::default(),
            r#"{"x":"valid"}"#,
            "valid",
            "",
            "",
        ),
    ];
    let mut programs = Vec::new();
    let mut cases = Vec::new();
    for (index, (schema, config, input, expected, suffix, path)) in schemas.into_iter().enumerate()
    {
        let (program, _) = compile(schema, config);
        cases.push(json!({"program":index,"id":format!("native-{index}"),"instanceJson":input,"expected":expected,"source":if suffix.is_empty(){Value::Null}else{json!(format!("/components/schemas/Root{suffix}"))},"instancePath":path}));
        programs.push(program);
    }
    assert_eq!(
        programs[8].version,
        OwnedProgram::V1_VERSION,
        "base closures retain the frozen v1 program"
    );
    let guarded = programs[9].clone();
    let metadata = serde_json::to_string(&guarded).unwrap();
    let extra=r#"
let getterCalls=0;const input={};Object.defineProperty(input,'x',{enumerable:true,get(){getterCalls++;return 'value';}});
const root=p9.validationRoots[0];assert.equal(p9.validate(root,input).kind,'evaluationFailure');assert.equal(getterCalls,0,'validation must not call instance accessors');
const valid=parseJson('{"x":"value"}');assert.equal(p9.validate(root,valid,()=>{throw new Error('observer failed');}).kind,'evaluationFailure');
assert.equal(p9.validate(root,valid).kind,'valid','a failed call cannot poison the next state');
const make=()=>JSON.parse(__PROGRAM_TEXT__);
const program=make();createValidator(program);assert.ok(Object.isFrozen(program));assert.ok(Object.isFrozen(program.nodes[0]));
for(const mutate of [
    p=>p.version='unknown',p=>p.profile='oas31-jsonschema202012-static-subset',
    p=>{p.version='suspect.validation.experimental.v1';p.profile='oas31-jsonschema202012-static-subset';},
    p=>delete p.limits.maxEvaluationSteps,
    p=>{const c=p.nodes.flatMap(n=>n.checks).find(c=>c.op==='patternProperties');c.patterns[0][2]=99999;},
    p=>{const c=p.nodes.flatMap(n=>n.checks).find(c=>c.op==='additionalPropertiesWithPatterns');c.declared=['invented'];},
    p=>p.resourceContext={resources:[],nodeScopes:[]},
    p=>p.nodes[0].checks.push({op:'const',source:{document:p.nodes[0].source.document,pointer:p.nodes[0].source.pointer+'/const'},value:()=>true}),
    p=>{const cycle={};cycle.self=cycle;p.nodes[0].checks.push({op:'const',source:{document:p.nodes[0].source.document,pointer:p.nodes[0].source.pointer+'/const'},value:cycle});}
]){const program=make();mutate(program);assert.throws(()=>createValidator(program));}
"#.replace("__PROGRAM_TEXT__",&serde_json::to_string(&metadata).unwrap());
    native("controls-", &programs, &json!(cases), &extra);
}

fn sdk_document() -> Value {
    let schemas = json!({
        "Conditional":{"type":"object","properties":{"kind":{"enum":["text","number"]},"text":{"type":"string"},"number":{"type":"integer"}},"required":["kind"],"additionalProperties":false,"if":{"properties":{"kind":{"const":"text"}}},"then":{"required":["text"]},"else":{"required":["number"]}},
        "Dependency":{"type":"object","properties":{"flag":{"type":["boolean","null"]},"peer":{"type":"string"}},"dependentRequired":{"flag":["peer"]},"dependentSchemas":{"flag":{"properties":{"flag":true,"peer":true}}},"unevaluatedProperties":false},
        "Patterns":{"type":"object","properties":{"name":{"type":"string"}},"required":["name"],"patternProperties":{"^x-":{"type":"integer","minimum":0},"amount$":{"minimum":2},"^__proto__$":{"type":"string"}},"propertyNames":{"not":{"const":"x-denied"}},"additionalProperties":false},
        "Sequence":{"type":"array","prefixItems":[{"type":"string"}],"contains":{"type":"integer"},"minContains":1,"maxContains":2,"unevaluatedItems":{"type":"string"}}
    });
    let mut document = json!({"openapi":"3.1.0","info":{"title":"Scoped native SDK","version":"1"},"servers":[{"url":"https://example.test"}],"components":{"schemas":schemas},"paths":{}});
    for (name, schema, example) in [
        (
            "conditional",
            "Conditional",
            json!({"kind":"text","text":"example"}),
        ),
        (
            "dependency",
            "Dependency",
            json!({"flag":null,"peer":"present"}),
        ),
        (
            "patterns",
            "Patterns",
            json!({"name":"example","x-count":3}),
        ),
        ("sequence", "Sequence", json!(["prefix", 1, "tail"])),
    ] {
        document["paths"][format!("/{name}")] = json!({"post":{"operationId":name,"requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":format!("#/components/schemas/{schema}")},"example":example.clone()}}},"responses":{"200":{"description":"validated","content":{"application/json":{"schema":{"$ref":format!("#/components/schemas/{schema}")},"example":example}}}}}});
    }
    document
}

fn load_document(document: &Value) -> Arc<Contract> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    std::fs::write(&path, document.to_string()).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(directory.path())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap())
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn scoped_sdk_capture_tracks_checked_graphs_and_preserves_literal_instance_data() {
    use suspect_codegen::{
        backend::{Backend, TargetConfig},
        compatibility::{self, PlanStatus},
    };
    let target = TargetConfig {
        backend: Backend::TypescriptHttp,
        package_name: "@fixture/scoped-sdk".into(),
        package_version: "0.0.0".into(),
        import_name: None,
    };
    let selected = ["conditional", "dependency", "patterns", "sequence"].map(str::to_owned);
    let mut document = sdk_document();
    let literal = json!({"source":"instance source","description":"instance prose","schema":{"type":"string"},"span":[1,2]});
    document["components"]["schemas"]["Conditional"]["if"] =
        json!({"anyOf":[{"properties":{"kind":{"const":"text"}}},{"const":literal}]});
    let before = load_document(&document);
    let mut prose = document.clone();
    prose["info"]["description"] = json!("Relocated document and changed byte offsets");
    prose["components"]["schemas"]["Conditional"]["description"] =
        json!("Native conditional model docs");
    prose["components"]["schemas"]["Conditional"]["if"]["description"] = json!("Condition prose");
    let report = compatibility::compare(
        before.clone(),
        load_document(&prose),
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
    let before_native = native.before.as_ref().unwrap();
    let checks = before_native
        .models
        .iter()
        .flat_map(|model| {
            let validation = &model.descriptor.as_ref().unwrap()["codec"]["validation"];
            assert_eq!(validation["version"], OwnedProgram::V2_VERSION);
            validation["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|node| node["checks"].as_array().unwrap())
        })
        .collect::<Vec<_>>();
    for op in [
        "if",
        "dependentRequired",
        "dependentSchemas",
        "contains",
        "patternProperties",
        "additionalPropertiesWithPatterns",
        "propertyNames",
        "unevaluatedProperties",
        "unevaluatedItems",
    ] {
        assert!(
            checks.iter().any(|check| check["op"] == op),
            "missing captured {op}"
        );
    }
    assert!(
        checks
            .iter()
            .any(|check| check["op"] == "const" && check["value"] == literal)
    );
    let (evidence, _temporary) = working_directory("capture-");
    std::fs::write(
        evidence.join("snapshot.json"),
        serde_json::to_vec_pretty(before_native).unwrap(),
    )
    .unwrap();
    for mutation in [
        "conditional",
        "dependency",
        "patterns",
        "contains",
        "literal",
    ] {
        let mut changed = document.clone();
        let schemas = &mut changed["components"]["schemas"];
        match mutation {
            "conditional" => schemas["Conditional"]["then"]["required"] = json!(["text", "number"]),
            "dependency" => {
                schemas["Dependency"]["dependentRequired"]["flag"] = json!(["peer", "other"])
            }
            "patterns" => schemas["Patterns"]["patternProperties"]["^x-"]["minimum"] = json!(1),
            "contains" => schemas["Sequence"]["maxContains"] = json!(1),
            "literal" => {
                schemas["Conditional"]["if"]["anyOf"][1]["const"]["description"] =
                    json!("changed instance data")
            }
            _ => unreachable!(),
        }
        let report = compatibility::compare(
            before.clone(),
            load_document(&changed),
            &selected,
            std::slice::from_ref(&target),
        )
        .unwrap();
        assert_eq!(
            report.native[0].after.as_ref().unwrap().status,
            PlanStatus::Planned,
            "{mutation}: {:?}",
            report.native[0].after.as_ref().unwrap().findings
        );
        assert!(
            report.native[0]
                .changes
                .iter()
                .any(|change| change.code == "native-model-shape-changed"),
            "{mutation}: {:?}",
            report.native[0].changes
        );
    }
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn scoped_http_examples_keep_valid_declared_values_and_locate_invalid_ones() {
    use suspect_codegen::{
        examples::{ExampleOrigin, ExampleRole},
        typescript::http::{HttpConfig, plan_http},
    };
    let mut document = sdk_document();
    for (name, invalid) in [
        ("conditional", json!({"kind":"text"})),
        ("dependency", json!({"flag":null})),
        ("patterns", json!({"name":"invalid","extra":true})),
        ("sequence", json!(["prefix", false])),
    ] {
        let media = &mut document["paths"][format!("/{name}")]["post"]["requestBody"]["content"]["application/json"];
        let valid = media.as_object_mut().unwrap().remove("example").unwrap();
        media["examples"] = json!({"good":{"value":valid},"bad":{"value":invalid}});
    }
    let contract = load_document(&document);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = plan_http(contract, &selected, HttpConfig::expanded()).unwrap();
    assert_eq!(plan.examples().operations().len(), 4);
    assert_eq!(
        plan.examples()
            .diagnostics()
            .iter()
            .filter(|finding| finding.code == "examples-declared-invalid"
                && finding.source.pointer().ends_with("/examples/bad/value")
                && !finding.at.is_empty())
            .count(),
        4,
        "{:?}",
        plan.examples().diagnostics()
    );
    for operation in plan.examples().operations() {
        let requests = operation
            .entries
            .iter()
            .filter(|entry| entry.role == ExampleRole::RequestBody)
            .collect::<Vec<_>>();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].origin, ExampleOrigin::Declared);
        assert_eq!(requests[0].name.as_deref(), Some("good"));
        assert!(
            requests[0]
                .declared_source
                .as_ref()
                .unwrap()
                .pointer()
                .ends_with("/examples/good/value")
        );
    }
    assert!(plan.first_request().is_some());
}

#[test]
#[ignore = "requires pinned Node 22/24, cached npm/TypeScript 5.5/5.9 and TypeDoc"]
fn installed_scoped_sdk_preserves_models_values_examples_and_docs() {
    use suspect_codegen::typescript::{
        ModelView,
        http::{HttpConfig, plan_http},
        package::{PackageConfig, emit_http},
        plan_models,
    };
    let document = sdk_document();
    let contract = load_document(&document);
    let roots = contract.schema_roots().to_vec();
    let models = plan_models(&contract, &roots, &[ModelView::Neutral]);
    assert!(!models.has_errors(), "{:?}", models.diagnostics());
    assert!(
        !models.release_ready(),
        "model-only lowering retains codec obligations"
    );
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = plan_http(contract, &selected, HttpConfig::expanded()).unwrap();
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
    let files = emit_http(
        &plan,
        &PackageConfig {
            name: "@fixture/scoped-sdk".into(),
            version: "0.0.0".into(),
        },
    )
    .unwrap();
    let (directory, _temporary) = working_directory("installed-sdk-");
    suspect_codegen::write_files(&files, &directory).unwrap();
    let package = directory.join("typescript");
    std::fs::write(
        directory.join("api.json"),
        serde_json::to_vec_pretty(&document).unwrap(),
    )
    .unwrap();
    let selected_node = std::env::var_os("SUSPECT_DOCS_NODE").unwrap_or_else(|| "node".into());
    let node = checked(
        Command::new(&selected_node).args(["--print", "process.execPath"]),
        &package,
    );
    let node = std::path::PathBuf::from(String::from_utf8(node.stdout).unwrap().trim());
    let npm = node
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("lib/node_modules/npm/bin/npm-cli.js");
    let npm_command = |root: &Path| {
        let mut command = Command::new(&node);
        command.arg(&npm).current_dir(root);
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
        &package,
    );
    checked(npm_command(&package).args(["run", "build"]), &package);
    checked(
        Command::new(&node)
            .current_dir(&package)
            .arg("dist/examples/validated.js"),
        &package,
    );
    let doc_tool = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/typescript-docs/build.mjs");
    checked(Command::new(&node).arg(doc_tool).arg(&package), &package);
    let floor = directory.join("floor");
    std::fs::create_dir(&floor).unwrap();
    let reviewed = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools/typescript-floor");
    for file in ["package.json", "package-lock.json"] {
        std::fs::copy(reviewed.join(file), floor.join(file)).unwrap();
    }
    checked(
        npm_command(&floor).args([
            "ci",
            "--offline",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
        ]),
        &floor,
    );
    let node24 = std::env::var_os("SUSPECT_NODE24_BIN")
        .expect("set SUSPECT_NODE24_BIN for the required second runtime");
    for (label, compiler) in [
        ("5.9.3", package.join("node_modules/typescript/bin/tsc")),
        ("5.5.4", floor.join("node_modules/typescript/bin/tsc")),
    ] {
        let version = checked(
            Command::new(&node).arg(&compiler).arg("--version"),
            &package,
        );
        assert_eq!(
            String::from_utf8_lossy(&version.stdout).trim(),
            format!("Version {label}")
        );
        if label == "5.5.4" {
            checked(
                Command::new(&node)
                    .arg(&compiler)
                    .current_dir(&package)
                    .args(["--project", "tsconfig.json"]),
                &package,
            );
        }
        let packed = checked(
            npm_command(&package).args(["pack", "--offline", "--ignore-scripts", "--json"]),
            &package,
        );
        let packed: Value = serde_json::from_slice(&packed.stdout).unwrap();
        let archive = directory.join(format!("scoped-sdk-ts{label}.tgz"));
        std::fs::rename(
            package.join(packed[0]["filename"].as_str().unwrap()),
            &archive,
        )
        .unwrap();
        let consumer = directory.join(format!("consumer-{label}"));
        std::fs::create_dir(&consumer).unwrap();
        std::fs::write(
            consumer.join("package.json"),
            "{\"private\":true,\"type\":\"module\"}",
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
            &consumer,
        );
        std::fs::write(consumer.join("consumer.ts"), SCOPED_TYPES).unwrap();
        std::fs::write(consumer.join("consumer.mjs"), SCOPED_RUNTIME).unwrap();
        checked(
            Command::new(&node)
                .arg(compiler)
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
            &consumer,
        );
        println!(
            "scoped installed declarations: {}",
            String::from_utf8_lossy(&version.stdout).trim()
        );
        for node in [node.clone().into_os_string(), node24.clone()] {
            let output = checked(
                Command::new(&node)
                    .current_dir(&consumer)
                    .arg("consumer.mjs"),
                &consumer,
            );
            println!(
                "TypeScript {label}: {}",
                String::from_utf8_lossy(&output.stdout).trim()
            );
            checked(
                Command::new(&node)
                    .current_dir(&consumer)
                    .arg("node_modules/@fixture/scoped-sdk/dist/examples/validated.js"),
                &consumer,
            );
        }
    }
}

#[test]
#[ignore = "requires pinned Node 22, TypeScript 5.9 and isolated Chromium (SUSPECT_CHROMIUM)"]
fn browser_scoped_sdk_executes_source_vectors_and_native_http_controls() {
    use suspect_codegen::typescript::{
        http::{HttpConfig, plan_http},
        package::{PackageConfig, emit_http},
    };
    let document = sdk_document();
    let contract = load_document(&document);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let plan = plan_http(contract, &selected, HttpConfig::expanded()).unwrap();
    let mut files = emit_http(
        &plan,
        &PackageConfig {
            name: "@fixture/scoped-sdk".into(),
            version: "0.0.0".into(),
        },
    )
    .unwrap();
    let (programs, cases) = source_vectors();
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
        root.join("api.json"),
        serde_json::to_vec_pretty(&document).unwrap(),
    )
    .unwrap();
    std::fs::write(
        generated.join("vectors.json"),
        serde_json::to_vec_pretty(&cases).unwrap(),
    )
    .unwrap();
    std::fs::write(
        generated.join("browser.mjs"),
        include_str!("fixtures/typescript-v2-browser.mjs"),
    )
    .unwrap();
    let node = std::env::var_os("SUSPECT_DOCS_NODE").unwrap_or_else(|| "node".into());
    let compiler = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tools/typescript-docs/node_modules/typescript/bin/tsc");
    let version = checked(
        Command::new(&node).arg(&compiler).arg("--version"),
        &generated,
    );
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        "Version 5.9.3"
    );
    let mut compile = Command::new(&node);
    compile.arg(&compiler).current_dir(&generated).args([
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
        compile.arg(format!("vector{index}.ts"));
    }
    checked(&mut compile, &generated);
    let output = checked(
        Command::new(node)
            .current_dir(&generated)
            .arg("browser.mjs"),
        &generated,
    );
    println!("{}", String::from_utf8_lossy(&output.stdout).trim());
}

const SCOPED_TYPES: &str = r#"
import {createClient, type models, JsonNumber} from '@fixture/scoped-sdk';
const client=createClient();
export async function requests(){
    await client.conditional({body:{kind:'text',text:'native'}});
    await client.dependency({body:{flag:null,peer:'present'}});
    await client.patterns({body:{name:'native','x-count':5n,['__proto__']:'owned'}});
    await client.sequence({body:['prefix',JsonNumber.parse('9007199254740993'),'tail']});
    // @ts-expect-error the known required property remains statically required
    await client.patterns({body:{'x-count':1n}});
    // @ts-expect-error functions are not a faithful exact JSON carrier
    await client.patterns({body:{name:'native','x-count':()=>1}});
    // @ts-expect-error undefined array values are not JSON values
    await client.sequence({body:['prefix',undefined]});
    // @ts-expect-error declared integers do not become strings
    await client.conditional({body:{kind:'number',number:'1'}});
    // @ts-expect-error omitted optional and explicit undefined remain distinct
    const dependency:models.Dependency={flag:undefined};
}
"#;
const SCOPED_RUNTIME: &str = r#"
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {createClient,operations,codecs,JsonNumber,ModelCodecError} from '@fixture/scoped-sdk';
let calls=0;const requests=[];
const replies={
    '/conditional':'{"kind":"number","number":9007199254740993}',
    '/dependency':'{"flag":null,"peer":"present"}',
    '/patterns':'{"name":"reply","x-count":9007199254740993,"__proto__":"kept"}',
    '/sequence':'["prefix",1e3,"tail"]'
};
const server=createServer((request,response)=>{const chunks=[];request.on('data',chunk=>chunks.push(chunk));request.on('end',()=>{calls++;requests.push(Buffer.concat(chunks).toString());response.writeHead(200,{'content-type':'application/json'});response.end(replies[request.url]);});});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
try{
    const client=createClient({serverURL:`http://127.0.0.1:${server.address().port}`});
    assert.equal((await client.conditional({body:{kind:'text',text:'native'}})).data.number,9007199254740993n);
    assert.equal((await client.dependency({body:{flag:null,peer:'present'}})).data.flag,null);
    const patterns=(await client.patterns({body:{name:'native','x-count':5n,['__proto__']:'owned'}})).data;
    assert.equal(patterns['x-count'].toString(),'9007199254740993');assert.equal(patterns.__proto__,'kept');assert.equal(Object.getPrototypeOf(patterns),null);
    const sequence=(await client.sequence({body:['prefix',JsonNumber.parse('9007199254740993'),'tail']})).data;
    assert.equal(sequence[1].toString(),'1e3');assert.ok(requests[2].includes('"__proto__":"owned"'));assert.ok(requests[3].includes('9007199254740993'));
    const invalid=[()=>client.conditional({body:{kind:'text'}}),()=>client.dependency({body:{flag:null}}),()=>client.dependency({body:{other:1}}),()=>client.patterns({body:{name:'x','x-count':-1n}}),()=>client.patterns({body:{name:'x','x-amount':1n}}),()=>client.patterns({body:{name:'x','x-denied':1n}}),()=>client.patterns({body:{name:'x',extra:true}}),()=>client.sequence({body:['prefix',true]}),()=>client.sequence({body:['prefix',1n,2n,3n]})];
    for(const call of invalid)await assert.rejects(call(),error=>operations.isSdkError(error)&&error.kind==='request-validation');
    assert.equal(calls,4,'invalid scoped inputs never reach HTTP');
    patterns['x-count']=JsonNumber.parse('-1');assert.throws(()=>codecs.PatternsCodec.encode(patterns),error=>error instanceof ModelCodecError&&error.kind==='invalid');
    const mutable={kind:'text',text:'first'};codecs.ConditionalCodec.encode(mutable);mutable.kind='number';assert.throws(()=>codecs.ConditionalCodec.encode(mutable),error=>error.kind==='invalid','encoding revalidates mutation');
    const absent=codecs.DependencyCodec.decode('{}');assert.equal(Object.hasOwn(absent,'flag'),false);assert.throws(()=>codecs.DependencyCodec.decode('{"flag":null}'),error=>error.kind==='invalid');
    const hostile={name:'x'};let invoked=0;Object.defineProperty(hostile,'x-count',{enumerable:true,get(){invoked++;return 1;}});assert.throws(()=>codecs.PatternsCodec.encode(hostile));assert.equal(invoked,0);
    console.log('scoped SDK models/http/examples',process.version);
}finally{await new Promise(resolve=>server.close(resolve));}
"#;
