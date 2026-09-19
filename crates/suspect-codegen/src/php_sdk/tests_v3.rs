//! Maintained source-driven resource/dynamic native witnesses, independent of target exports.
use super::tests_v2::{install, literal, php, phpstan, repo, run, typecheck};
use super::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fmt::Write as _, fs, path::PathBuf, process::Command};
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

#[path = "tests_v3_sdk.rs"]
mod sdk;

#[test]
#[ignore = "resource/dynamic SDK operations with independent HTTP, installed native types and examples"]
#[cfg(feature = "http-protocol")]
fn native_v3_sdk_packages_models_codecs() {
    sdk::verify();
}

fn root(label: &str) -> PathBuf {
    let base = repo().join("target/sdk-php-validation-v3");
    fs::create_dir_all(&base).unwrap();
    tempfile::Builder::new()
        .prefix(label)
        .tempdir_in(base)
        .unwrap()
        .keep()
}
fn source(document: &str, pointer: &str) -> SourceId {
    let root = SourceId::new(Uri::parse(document).unwrap(), Default::default());
    pointer
        .strip_prefix('/')
        .map(|p| {
            p.split('/').fold(root.clone(), |id, p| {
                id.child(&p.replace("~1", "/").replace("~0", "~"))
            })
        })
        .unwrap_or(root)
}
fn schema(name: &str) -> SourceId {
    source(
        "https://physical.test/api.json",
        &format!("/components/schemas/{name}"),
    )
}
fn api(schemas: Value) -> Value {
    json!({"openapi":"3.1.0","info":{"title":"Native resource witnesses","version":"1"},"components":{"schemas":schemas}})
}
fn provided(entry: Value, mut documents: Vec<(String, Value)>) -> Arc<Contract> {
    documents.push(("https://physical.test/api.json".into(), entry));
    let provider = Arc::new(
        DocumentProvider::new(documents.into_iter().map(|(uri, value)| {
            ProvidedDocument::new(
                Uri::parse(&uri).unwrap(),
                Uri::parse(&uri).unwrap(),
                serde_json::to_vec(&value).unwrap(),
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
    Arc::new(
        Contract::from_workspace(
            &workspace,
            &Uri::parse("https://physical.test/api.json").unwrap(),
        )
        .unwrap(),
    )
}
struct Case {
    id: String,
    contract: Arc<Contract>,
    roots: Vec<SourceId>,
    selected: SourceId,
    instance: Value,
    expected: &'static str,
    location: Option<SourceId>,
    path: Option<String>,
    config: PhpConfig,
}
fn case(
    id: &str,
    contract: Arc<Contract>,
    selected: SourceId,
    instance: Value,
    expected: &'static str,
) -> Case {
    Case {
        id: id.into(),
        contract,
        roots: vec![selected.clone()],
        selected,
        instance,
        expected,
        location: None,
        path: None,
        config: PhpConfig::default(),
    }
}

fn native_cases(
    label: &str,
    cases: Vec<Case>,
    provenance: Value,
    extra: Option<&str>,
) -> (PathBuf, Vec<OwnedProgram>) {
    let root = root(label);
    let mut files = Vec::new();
    let mut programs = Vec::new();
    let mut records = Vec::new();
    let mut script = String::from(
        "<?php\ndeclare(strict_types=1);\nrequire __DIR__.'/vendor/autoload.php';\nfunction check(bool $ok,string $id):void{if(!$ok){throw new RuntimeException($id);}}\n$results=[];\n",
    );
    for (i, mut c) in cases.into_iter().enumerate() {
        c.config.namespace = format!("NativeV3\\C{i}");
        let program = suspect_schema::OwnedCompiler::new(c.config.validation.clone())
            .compile_v3(c.contract, &c.roots)
            .unwrap()
            .program();
        native_program_check_mode(&program, true, true).unwrap();
        let target = program
            .roots
            .iter()
            .find(|r| {
                r.source.document == c.selected.document().as_str()
                    && r.source.pointer == c.selected.pointer()
            })
            .unwrap()
            .target;
        for mut file in emit_validation(&program, &c.config).unwrap() {
            file.path = format!(
                "php/src/C{i}/{}",
                file.path.strip_prefix("php/src/").unwrap()
            );
            files.push(file);
        }
        let ns = &c.config.namespace;
        writeln!(script,"$actual='Valid';$source='';$path='';\ntry{{{ns}\\Validator::validate({target},{ns}\\JsonValue::parse({}));}}catch({ns}\\ValidationError $error){{$actual=$error->kind==='invalid'?'Invalid':'EvaluationFailure';$source=$error->source;$path=$error->instancePath;}}\ncheck($actual==={},{} . ': outcome ' . $actual);",literal(&c.instance.to_string()),literal(c.expected),literal(&c.id)).unwrap();
        if let Some(location) = &c.location {
            writeln!(
                script,
                "check($source==={},{} . ': source ' . $source);",
                literal(&format!("{}#{}", location.document(), location.pointer())),
                literal(&c.id)
            )
            .unwrap();
        }
        if let Some(path) = &c.path {
            writeln!(
                script,
                "check($path==={},{} . ': path ' . $path);",
                literal(path),
                literal(&c.id)
            )
            .unwrap();
        }
        writeln!(
            script,
            "$results[]=[{},$actual,$source,$path];",
            literal(&c.id)
        )
        .unwrap();
        records.push(json!({"id":c.id,"selected":{"document":c.selected.document().as_str(),"pointer":c.selected.pointer()},"rootTarget":target,"instanceJson":c.instance.to_string(),"expected":c.expected,"program":program}));
        programs.push(program);
    }
    if let Some(extra) = extra {
        let i = records.len() - 1;
        script.push_str(
            &extra
                .replace("__NS__", &format!("NativeV3\\C{i}"))
                .replace("__ROOT__", &records[i]["rootTarget"].to_string()),
        );
    }
    writeln!(script,"echo json_encode($results,JSON_THROW_ON_ERROR|JSON_UNESCAPED_UNICODE),PHP_EOL;\necho '{} source-driven resource/dynamic native cases passed',PHP_EOL;",records.len()).unwrap();
    fs::write(
        root.join("source-programs.json"),
        serde_json::to_string_pretty(&json!({"provenance":provenance,"cases":records})).unwrap(),
    )
    .unwrap();
    files.push(OutFile{path:"php/phpstan.neon".into(),content:"parameters:\n    level: max\n    phpVersion: 80300\n    treatPhpDocTypesAsCertain: false\n    paths: [src]\n    tmpDir: build/phpstan\n".into()});
    let consumer = install(&root, &files, "php-validation-v3");
    fs::write(consumer.join("cases.php"), script).unwrap();
    run(
        Command::new(php())
            .args(["-n", "-d", "error_reporting=-1", "cases.php"])
            .current_dir(&consumer),
        &root,
        "native-cases",
    );
    run(
        Command::new(php())
            .arg("-n")
            .arg(phpstan())
            .args([
                "analyse",
                "--no-progress",
                "--memory-limit=2G",
                "--autoload-file",
            ])
            .arg(consumer.join("vendor/autoload.php"))
            .current_dir(consumer.join("vendor/fixture/php-validation-v3")),
        &root,
        "package-types",
    );
    typecheck(&root, &consumer, &["cases.php"], "consumer-types");
    println!("PHP v3 evidence: {}", root.display());
    (root, programs)
}

#[test]
#[ignore = "44 unmodified official source cases, supplied remotes and installed PHP runtime"]
fn native_v3_source_fixtures() {
    let directory = repo().join("crates/suspect-schema/tests/fixtures/resource-conformance");
    let bytes = fs::read(directory.join("dynamicRef.json")).unwrap();
    let groups: Vec<Value> = serde_json::from_slice(&bytes).unwrap();
    let mut hashes = serde_json::Map::new();
    hashes.insert(
        "dynamicRef.json".into(),
        json!(format!("{:x}", Sha256::digest(&bytes))),
    );
    let mut remotes = Vec::new();
    for name in [
        "tree.json",
        "extendible-dynamic-ref.json",
        "detached-dynamicref.json",
    ] {
        let bytes = fs::read(directory.join(name)).unwrap();
        hashes.insert(name.into(), json!(format!("{:x}", Sha256::digest(&bytes))));
        remotes.push((
            format!("http://localhost:1234/draft2020-12/{name}"),
            serde_json::from_slice::<Value>(&bytes).unwrap(),
        ));
    }
    let mut cases = Vec::new();
    for group in groups {
        let physical = "https://physical.test/official-schema.json";
        let mut documents = remotes.clone();
        documents.push((physical.into(), group["schema"].clone()));
        let contract = provided(api(json!({"Use":{"$ref":physical}})), documents);
        for test in group["tests"].as_array().unwrap() {
            cases.push(case(
                &format!(
                    "{} / {}",
                    group["description"].as_str().unwrap(),
                    test["description"].as_str().unwrap()
                ),
                contract.clone(),
                source(physical, ""),
                test["data"].clone(),
                if test["valid"] == true {
                    "Valid"
                } else {
                    "Invalid"
                },
            ));
        }
    }
    assert_eq!(cases.len(), 44);
    native_cases(
        "official-",
        cases,
        json!({"directory":directory,"sha256":hashes,"provider":"closed supplied original remote URIs; no acquisition"}),
        None,
    );
}

#[test]
#[ignore = "independent dynamic context, scope, budget, key and ownership controls"]
fn native_v3_scope_resource_controls() {
    let contract = provided(
        api(json!({
            "Leaf":{"$id":"urn:leaf","const":false,"$defs":{"t":{"$dynamicAnchor":"t","type":"string"}}},
            "Start":{"$id":"urn:start","$dynamicRef":"urn:leaf#t"},
            "Scan":{"$id":"urn:scan","$defs":{"a":{"$dynamicAnchor":"a"},"t":{"$dynamicAnchor":"t","type":"string"}},"$dynamicRef":"urn:leaf#t"},
            "Base":{"$id":"urn:base","$defs":{"slot":{"$dynamicAnchor":"slot","type":"string"}},"properties":{"value":{"$dynamicRef":"#slot"}}},
            "Mid":{"$id":"urn:mid","$defs":{"slot":{"$dynamicAnchor":"slot","type":"boolean"}},"$ref":"urn:base"},
            "Outer":{"$id":"urn:outer","$defs":{"slot":{"$dynamicAnchor":"slot","type":"integer"}},"$ref":"urn:mid"},
            "Idle":{"$id":"urn:idle","$dynamicAnchor":"slot","not":{}},
            "Detached":{"$id":"urn:detached","const":false,"$defs":{"binding":{"$dynamicAnchor":"slot","type":"integer"},"entry":{"$ref":"urn:base"}}},
            "Scalar":{"$id":"urn:scalar","type":"string","$defs":{"slot":{"$dynamicAnchor":"slot","$anchor":"plain","type":"string"}}},
            "Forms":{"$id":"urn:forms","$defs":{"slot":{"$dynamicAnchor":"slot","type":"integer"}},"properties":{
                "dynamic":{"$dynamicRef":"urn:scalar#slot"},"pointer":{"$dynamicRef":"urn:scalar#/$defs/slot"},"plain":{"$dynamicRef":"urn:scalar#plain"},"empty":{"$dynamicRef":"urn:scalar#"},"static":{"$ref":"urn:scalar#slot"}
            }},
            "Fail":{"$id":"urn:fail","$dynamicAnchor":"flag","not":{}},
            "Loop":{"$id":"urn:loop","if":{"$dynamicRef":"urn:fail#flag"},"then":true,"else":{"$ref":"urn:new"}},
            "New":{"$id":"urn:new","$defs":{"flag":{"$dynamicAnchor":"flag"}},"$ref":"urn:loop"},
            "Trial":{"anyOf":[{"$ref":"urn:idle"},{"$dynamicRef":"urn:scalar#slot"}]},
            "Cycle":{"$id":"urn:cycle","$dynamicAnchor":"self","$dynamicRef":"#self"},
            "Number":{"$id":"urn:number","$dynamicAnchor":"n","type":"integer"},
            "Late":{"anyOf":[true,{"$dynamicRef":"urn:number#n"}]}
        })),
        vec![],
    );
    let mut cases = Vec::new();
    let mut c = case(
        "outermost entered binding",
        contract.clone(),
        schema("Outer"),
        json!({"value":7}),
        "Valid",
    );
    c.roots.push(schema("Idle"));
    cases.push(c);
    let mut c = case(
        "inner binding cannot override outer",
        contract.clone(),
        schema("Outer"),
        json!({"value":true}),
        "Invalid",
    );
    c.location = Some(schema("Outer").child("$defs").child("slot").child("type"));
    c.path = Some("/value".into());
    cases.push(c);
    cases.push(case(
        "nested entry enters parent resource only",
        contract.clone(),
        schema("Detached").child("$defs").child("entry"),
        json!({"value":9}),
        "Valid",
    ));
    cases.push(case(
        "all fallback modes",
        contract.clone(),
        schema("Forms"),
        json!({"dynamic":1,"pointer":"s","plain":"s","empty":"s","static":"s"}),
        "Valid",
    ));
    for name in ["pointer", "plain", "empty", "static"] {
        cases.push(case(
            &format!("{name} is not dynamically overridden"),
            contract.clone(),
            schema("Forms"),
            json!({name:1}),
            "Invalid",
        ));
    }
    cases.push(case(
        "changed ordered context is not old cycle",
        contract.clone(),
        schema("Loop"),
        json!(7),
        "Valid",
    ));
    cases.push(case(
        "failed trial restores resources",
        contract.clone(),
        schema("Trial"),
        json!("s"),
        "Valid",
    ));
    let mut c = case(
        "fallback is not entered before lookup",
        contract.clone(),
        schema("Start"),
        json!("s"),
        "Valid",
    );
    c.config.validation.max_evaluation_steps = 7;
    cases.push(c);
    let mut c = case(
        "new resource and node charges",
        contract.clone(),
        schema("Start"),
        json!("s"),
        "EvaluationFailure",
    );
    c.config.validation.max_evaluation_steps = 6;
    c.location = Some(schema("Leaf").child("$defs").child("t").child("type"));
    c.path = Some("".into());
    cases.push(c);
    let mut c = case(
        "each scanned binding costs work",
        contract.clone(),
        schema("Scan"),
        json!("s"),
        "Valid",
    );
    c.config.validation.max_evaluation_steps = 8;
    cases.push(c);
    let mut c = case(
        "binding scan cannot skip candidates",
        contract.clone(),
        schema("Scan"),
        json!("s"),
        "EvaluationFailure",
    );
    c.config.validation.max_evaluation_steps = 7;
    c.location = Some(schema("Scan").child("$defs").child("t").child("type"));
    c.path = Some("".into());
    cases.push(c);
    let mut c = case(
        "exact context nonprogress cycle",
        contract.clone(),
        schema("Cycle"),
        json!(null),
        "EvaluationFailure",
    );
    c.location = Some(schema("Cycle"));
    c.path = Some("".into());
    cases.push(c);
    let mut c = case(
        "late numeric failure is noninvertible",
        contract.clone(),
        schema("Late"),
        json!(12345),
        "EvaluationFailure",
    );
    c.config.validation.max_number_bytes = 3;
    c.location = Some(schema("Number").child("type"));
    c.path = Some("".into());
    cases.push(c);
    let mut chain = serde_json::Map::new();
    for i in 0..21 {
        chain.insert(
            format!("R{i}"),
            if i == 20 {
                json!({"$id":format!("urn:chain{i}"),"type":"integer"})
            } else {
                json!({"$id":format!("urn:chain{i}"),"$ref":format!("urn:chain{}",i+1)})
            },
        );
    }
    let chain = provided(api(Value::Object(chain)), vec![]);
    let mut deep = case(
        "distinct resource depth remains bounded",
        chain.clone(),
        schema("R0"),
        json!(1),
        "EvaluationFailure",
    );
    deep.config.validation.max_depth = 12;
    deep.location = Some(schema("R12"));
    deep.path = Some("".into());
    cases.push(deep);
    cases.push(case(
        "productive resource chain",
        chain,
        schema("R0"),
        json!(1),
        "Valid",
    ));
    cases.push(case(
        "cancellation and retained session",
        contract,
        schema("Start"),
        json!("s"),
        "Valid",
    ));
    let (root, programs) = native_cases(
        "controls-",
        cases,
        json!({"oracle":"independent SDK-SCHEMA-RESOURCES.md context and exact visit rules"}),
        Some(RESOURCE_CONTROL),
    );
    let original = &programs[0];
    for change in 0..8 {
        let mut p = original.clone();
        let r = p.resource_context.as_mut().unwrap();
        match change {
            0 => {
                r.node_scopes.pop();
            }
            1 => r.node_scopes[0].0 = usize::MAX,
            2 => r.node_scopes[0].2 = "urn:wrong".into(),
            3 => r.resources[0].aliases.clear(),
            4 => {
                let alias = r.resources[0].canonical_uri.clone();
                r.resources[1].aliases.push(alias);
            }
            5 => {
                r.resources
                    .iter_mut()
                    .find(|r| !r.dynamic_anchors.is_empty())
                    .unwrap()
                    .dynamic_anchors[0]
                    .2 = usize::MAX;
            }
            6 => {
                let op = p
                    .nodes
                    .iter_mut()
                    .flat_map(|n| &mut n.checks)
                    .find(|c| {
                        matches!(
                            c.instruction,
                            suspect_schema::ProgramInstruction::DynamicRef { .. }
                        )
                    })
                    .unwrap();
                if let suspect_schema::ProgramInstruction::DynamicRef {
                    initial_resource, ..
                } = &mut op.instruction
                {
                    *initial_resource = usize::MAX;
                }
            }
            _ => p.profile = "oas31-jsonschema202012-static-applicators",
        }
        assert!(
            native_program_check_mode(&p, true, true).is_err(),
            "malformed v3 metadata {change}"
        );
    }
    for (version, profile) in [
        (OwnedProgram::V1_VERSION, OwnedProgram::V1_PROFILE),
        (OwnedProgram::V2_VERSION, OwnedProgram::V2_PROFILE),
    ] {
        let mut p = original.clone();
        p.version = version;
        p.profile = profile;
        assert!(native_program_check_mode(&p, true, true).is_err());
    }
    fs::write(
        root.join("guard-controls.json"),
        json!({"malformedProgramsRefused":8,"oldEnvelopeGuards":2}).to_string(),
    )
    .unwrap();
}
const RESOURCE_CONTROL: &str = r#"
final class StopResourceValidation extends RuntimeException {}
$state=new class{public bool $stop=true;public int $checks=0;};
$control=new __NS__\CallControl(static function()use($state):void{if($state->stop&&++$state->checks===5){throw new StopResourceValidation();}});
$session=new __NS__\ValidationSession($control);$value=__NS__\JsonValue::fromString('s');
try{$session->check(__ROOT__,$value);throw new RuntimeException('missing cancellation');}catch(StopResourceValidation $error){check($state->checks===5,'cancelled work count');}unset($error);
$state->stop=false;$session->check(__ROOT__,$value);$session->check(__ROOT__,$value);
$weak=WeakReference::create($value);unset($value);check($weak->get()===null,'resource context retained the instance');
echo 'resource return/trial/cancellation restoration and ownership passed',PHP_EOL;
"#;

#[test]
#[cfg(feature = "http-protocol")]
fn ordinary_v1_v2_programs_and_entrypoints_keep_their_bytes() {
    for scoped in [false, true] {
        let mut root = json!({"type":"object","properties":{"a":{"type":"integer"},"b":{"type":"integer"}},"additionalProperties":false});
        if scoped {
            root["dependentRequired"] = json!({"a":["b"]});
        }
        let mut doc = api(json!({"Root":root}));
        doc["servers"] = json!([{"url":"https://fixture.test"}]);
        doc["paths"] = json!({"/check":{"post":{"operationId":"check","security":[],"requestBody":{"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Root"}}}},"responses":{"200":{"description":"Checked","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Root"}}}}}}}});
        let contract = provided(doc, vec![]);
        let selected = contract
            .operations()
            .map(|op| op.source().clone())
            .collect::<Vec<_>>();
        let before = protocol::plan_sdk_mode(
            contract.clone(),
            &selected,
            PhpConfig::default(),
            protocol::capabilities(),
            true,
            false,
        )
        .unwrap();
        let after = protocol::plan_sdk_mode(
            contract,
            &selected,
            PhpConfig::default(),
            protocol::capabilities(),
            true,
            true,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_vec(before.program()).unwrap(),
            serde_json::to_vec(after.program()).unwrap()
        );
        assert!(after.program().resource_context.is_none());
        assert_eq!(before.render(), after.render());
    }
}
