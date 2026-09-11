#![cfg(feature = "java-sdk")]
//! V3 native witnesses compile original source fixtures through the real resource compiler.
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use suspect_codegen::java_sdk::{self, MavenConfig, PackageConfig, SdkPlan};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_schema::{Config, OwnedCompiler, OwnedProgram};
use suspect_source::Uri;

const ENTRY: &str = "https://physical.java.test/api.json";
fn crate_root() -> PathBuf {
    std::env::var_os("SUSPECT_JAVA_CRATE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_MANIFEST_DIR").into())
}
fn attempt() -> PathBuf {
    let root = crate_root().join("../../target/sdk-java-validation-v3/java");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("case-")
        .tempdir_in(root)
        .unwrap()
        .keep()
}
fn id(document: &str, pointer: &str) -> SchemaId {
    let mut id = SchemaId::new(Uri::parse(document).unwrap(), Default::default());
    for token in pointer.split('/').skip(1) {
        id = id.child(&token.replace("~1", "/").replace("~0", "~"));
    }
    id
}
fn schema(name: &str) -> SchemaId {
    id(ENTRY, "/components/schemas").child(name)
}
fn api(schemas: Value) -> Value {
    json!({"openapi":"3.2.0","info":{"title":"Java resource witnesses","version":"1"},"paths":{},"components":{"schemas":schemas}})
}
fn load(document: Value, external: Vec<(&str, &str, Vec<u8>)>) -> Arc<Contract> {
    let mut documents = vec![
        ProvidedDocument::new(
            Uri::parse(ENTRY).unwrap(),
            Uri::parse(ENTRY).unwrap(),
            document.to_string().into_bytes(),
        )
        .unwrap(),
    ];
    documents.extend(external.into_iter().map(|(requested, effective, bytes)| {
        ProvidedDocument::new(
            Uri::parse(requested).unwrap(),
            Uri::parse(effective).unwrap(),
            bytes,
        )
        .unwrap()
    }));
    let provider = Arc::new(DocumentProvider::new(documents).unwrap());
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::parse(ENTRY).unwrap()).unwrap())
}
fn program(contract: Arc<Contract>, roots: &[SchemaId], limits: Config) -> OwnedProgram {
    let value = OwnedCompiler::new(limits)
        .compile_v3(contract, roots)
        .unwrap()
        .program();
    value.check().unwrap();
    assert_eq!(value.version, OwnedProgram::V3_VERSION);
    value
}
fn limits() -> Config {
    Config {
        max_depth: 128,
        ..Default::default()
    }
}
fn case(
    name: &str,
    program: &OwnedProgram,
    root: &SchemaId,
    instance: Value,
    expected: &str,
    source: Option<&str>,
    path: &str,
) -> Value {
    let target = program
        .roots
        .iter()
        .find(|v| {
            v.source.document == root.document().as_str() && v.source.pointer == root.pointer()
        })
        .unwrap()
        .target;
    json!({"id":name,"program":program,"rootTarget":target,"instanceJson":instance.to_string(),"expected":expected,"source":source,"instancePath":path})
}
fn official_cases() -> Vec<Value> {
    let groups: Vec<Value> = serde_json::from_str(include_str!(
        "../../suspect-schema/tests/fixtures/resource-conformance/dynamicRef.json"
    ))
    .unwrap();
    let mut cases = Vec::new();
    for group in groups {
        let document = "https://physical.java.test/official-schema.json";
        let c=load(api(json!({"Use":{"$ref":document}})),vec![
            (document,document,group["schema"].to_string().into_bytes()),
            ("http://localhost:1234/draft2020-12/tree.json","http://localhost:1234/draft2020-12/tree.json",include_bytes!("../../suspect-schema/tests/fixtures/resource-conformance/tree.json").to_vec()),
            ("http://localhost:1234/draft2020-12/extendible-dynamic-ref.json","http://localhost:1234/draft2020-12/extendible-dynamic-ref.json",include_bytes!("../../suspect-schema/tests/fixtures/resource-conformance/extendible-dynamic-ref.json").to_vec()),
            ("http://localhost:1234/draft2020-12/detached-dynamicref.json","http://localhost:1234/draft2020-12/detached-dynamicref.json",include_bytes!("../../suspect-schema/tests/fixtures/resource-conformance/detached-dynamicref.json").to_vec()),
        ]);
        let root = id(document, "");
        let p = program(c, std::slice::from_ref(&root), limits());
        for test in group["tests"].as_array().unwrap() {
            cases.push(case(
                &format!(
                    "{} / {}",
                    group["description"].as_str().unwrap(),
                    test["description"].as_str().unwrap()
                ),
                &p,
                &root,
                test["data"].clone(),
                if test["valid"].as_bool().unwrap() {
                    "Valid"
                } else {
                    "Invalid"
                },
                None,
                "",
            ));
        }
    }
    assert_eq!(cases.len(), 44);
    cases
}
fn controls() -> Vec<Value> {
    let mut out = Vec::new();
    let mut document = api(
        json!({"Root":{"$id":"models/root","$defs":{"Child":{"$id":"child#","type":"integer","minimum":9007199254740993u64}},"$ref":"child"},"Use":{"$ref":"https://logical.java.test/models/root"}}),
    );
    document["$self"] = json!("https://logical.java.test/api#revision");
    let p = program(load(document, vec![]), &[schema("Use")], limits());
    out.push(case(
        "canonical-self-and-nested-id",
        &p,
        &schema("Use"),
        json!(9007199254740993u64),
        "Valid",
        None,
        "",
    ));
    out.push(case(
        "canonical-id-keeps-physical-failure",
        &p,
        &schema("Use"),
        json!(9007199254740992u64),
        "Invalid",
        Some("/components/schemas/Root/$defs/Child/minimum"),
        "",
    ));
    let requested = "https://requested.java.test/model.json";
    let effective = "https://cdn.java.test/effective/model.json";
    let fragment = "#/$defs/a~1b~0%25%20%23%C3%A9";
    let c = load(
        api(
            json!({"Alias":{"$ref":format!("{requested}{fragment}")},"Logical":{"$ref":format!("urn:escaped{fragment}")},"Physical":{"$ref":format!("{effective}{fragment}")}}),
        ),
        vec![(
            requested,
            effective,
            json!({"$id":"urn:escaped","$defs":{"a/b~% #é":{"type":"integer","maximum":0}}})
                .to_string()
                .into_bytes(),
        )],
    );
    let p = program(
        c,
        &[schema("Alias"), schema("Logical"), schema("Physical")],
        limits(),
    );
    for name in ["Alias", "Logical", "Physical"] {
        out.push(case(
            &format!("physical-escaped-{name}"),
            &p,
            &schema(name),
            json!(1),
            "Invalid",
            Some("https://cdn.java.test/effective/model.json#/$defs/a~1b~0% #é/maximum"),
            "",
        ));
    }
    let c = load(
        api(json!({
            "Tree":{"$id":"urn:tree","$dynamicAnchor":"node","type":"object","properties":{"data":true,"children":{"type":"array","items":{"$dynamicRef":"#node"}}}},
            "Strict":{"$id":"urn:strict","$dynamicAnchor":"node","$ref":"urn:tree","unevaluatedProperties":false}
        })),
        vec![],
    );
    let p = program(c, &[schema("Strict"), schema("Tree")], limits());
    out.push(case(
        "strict-tree-success",
        &p,
        &schema("Strict"),
        json!({"children":[{"data":"x"}]}),
        "Valid",
        None,
        "",
    ));
    out.push(case(
        "strict-tree-dynamic-annotation",
        &p,
        &schema("Strict"),
        json!({"children":[{"unexpected":1}]}),
        "Invalid",
        Some("/components/schemas/Strict/unevaluatedProperties"),
        "/children/0/unexpected",
    ));
    out.push(case(
        "unentered-strict-binding",
        &p,
        &schema("Tree"),
        json!({"children":[{"unexpected":1}]}),
        "Valid",
        None,
        "",
    ));
    let c = load(
        api(json!({
            "Base":{"$id":"urn:base","$dynamicAnchor":"node","properties":{"outer":true,"middle":true,"children":{"items":{"$dynamicRef":"#node"}}}},
            "Middle":{"$id":"urn:middle","$dynamicAnchor":"node","$ref":"urn:base","required":["middle"]},
            "Outer":{"$id":"urn:outer","$dynamicAnchor":"node","$ref":"urn:middle","required":["outer"]},
            "Unentered":{"$id":"urn:aaa","$dynamicAnchor":"node","not":{}}
        })),
        vec![],
    );
    let p = program(c, &[schema("Outer"), schema("Unentered")], limits());
    out.push(case(
        "outermost-entered",
        &p,
        &schema("Outer"),
        json!({"outer":true,"middle":true,"children":[{"outer":true,"middle":true}]}),
        "Valid",
        None,
        "",
    ));
    out.push(case(
        "outermost-not-nearest",
        &p,
        &schema("Outer"),
        json!({"outer":true,"middle":true,"children":[{"middle":true}]}),
        "Invalid",
        Some("/components/schemas/Outer/required"),
        "/children/0",
    ));
    let outer = "https://physical.java.test/outer.json";
    let base = "https://physical.java.test/base.json";
    let extra = "https://physical.java.test/extra.json";
    let c=load(api(json!({"Use":{"$ref":format!("{outer}#/$defs/start")}})),vec![
        (outer,outer,json!({"$id":"urn:outer","const":false,"$defs":{"binding":{"$dynamicAnchor":"node","$ref":"urn:extra"},"start":{"$ref":"urn:base#/$defs/use"}}}).to_string().into_bytes()),
        (base,base,json!({"$id":"urn:base","$defs":{"node":{"$dynamicAnchor":"node","type":"string"},"use":{"$dynamicRef":"#node"}}}).to_string().into_bytes()),
        (extra,extra,json!({"$id":"urn:extra","type":"integer"}).to_string().into_bytes()),
    ]);
    let root = id(outer, "/$defs/start");
    let p = program(c, std::slice::from_ref(&root), limits());
    assert!(
        !p.nodes
            .iter()
            .any(|n| n.source.document == outer && n.source.pointer.is_empty())
    );
    out.push(case(
        "nested-entry-unvisited-parent",
        &p,
        &root,
        json!(7),
        "Valid",
        None,
        "",
    ));
    out.push(case(
        "nested-entry-original-extra-source",
        &p,
        &root,
        json!("wrong"),
        "Invalid",
        Some("https://physical.java.test/extra.json#/type"),
        "",
    ));
    let c = load(
        api(json!({
            "Fallback":{"$id":"urn:fallback","$dynamicAnchor":"flag","not":{}},
            "Outer":{"$id":"urn:outer","if":{"$dynamicRef":"urn:fallback#flag"},"then":true,"else":{"$ref":"urn:new-context"}},
            "NewContext":{"$id":"urn:new-context","$defs":{"flag":{"$dynamicAnchor":"flag"}},"$ref":"urn:outer"},
            "Plain":{"$id":"urn:plain","$dynamicAnchor":"node","type":"string"},
            "Failed":{"$id":"urn:failed","$dynamicAnchor":"node","not":{}},
            "Passing":{"$id":"urn:passing","$defs":{"override":{"$dynamicAnchor":"node","type":"integer"}}},
            "FailedTrial":{"anyOf":[{"$ref":"urn:failed"},{"$dynamicRef":"urn:plain#node"}]},
            "PassingTrial":{"allOf":[{"$ref":"urn:passing"},{"$dynamicRef":"urn:plain#node"}]}
        })),
        vec![],
    );
    let p = program(
        c,
        &[
            schema("Outer"),
            schema("FailedTrial"),
            schema("PassingTrial"),
            schema("Passing").child("$defs").child("override"),
        ],
        limits(),
    );
    out.push(case(
        "changed-context-reentry",
        &p,
        &schema("Outer"),
        json!(7),
        "Valid",
        None,
        "",
    ));
    out.push(case(
        "failed-trial-scope-restored",
        &p,
        &schema("FailedTrial"),
        json!("string"),
        "Valid",
        None,
        "",
    ));
    out.push(case(
        "passing-trial-scope-restored",
        &p,
        &schema("PassingTrial"),
        json!("string"),
        "Valid",
        None,
        "",
    ));
    let c = load(
        api(json!({
            "Base":{"$id":"urn:base","type":"string","$defs":{"Target":{"$dynamicAnchor":"node","$anchor":"plain","type":"string"}}},
            "Outer":{"$id":"urn:outer","$defs":{"Override":{"$dynamicAnchor":"node","type":"integer"}},"properties":{"dynamic":{"$dynamicRef":"urn:base#node"},"encoded":{"$dynamicRef":"urn:base#n%6fde"},"pointer":{"$dynamicRef":"urn:base#/$defs/Target"},"plain":{"$dynamicRef":"urn:base#plain"},"empty":{"$dynamicRef":"urn:base#"},"static":{"$ref":"urn:base#node"}}}
        })),
        vec![],
    );
    let p = program(c, &[schema("Outer")], limits());
    out.push(case(
        "dynamic-name-vs-fallback-modes",
        &p,
        &schema("Outer"),
        json!({"dynamic":1,"encoded":2,"pointer":"s","plain":"s","empty":"s","static":"s"}),
        "Valid",
        None,
        "",
    ));
    for name in ["pointer", "plain", "empty", "static"] {
        out.push(case(
            &format!("fallback-{name}-never-overrides"),
            &p,
            &schema("Outer"),
            json!({name:1}),
            "Invalid",
            None,
            "",
        ));
    }
    let c = load(
        api(
            json!({"Base":{"$id":"urn:base","$dynamicAnchor":"x","type":"string"},"Use":{"$id":"urn:use","$dynamicRef":"urn:base#x"}}),
        ),
        vec![],
    );
    for (budget, expected, at) in [
        (7, "Valid", None),
        (
            6,
            "EvaluationFailure",
            Some("/components/schemas/Base/type"),
        ),
        (
            3,
            "EvaluationFailure",
            Some("/components/schemas/Use/$dynamicRef"),
        ),
    ] {
        let p = program(
            c.clone(),
            &[schema("Use")],
            Config {
                max_evaluation_steps: budget,
                ..limits()
            },
        );
        out.push(case(
            &format!("resource-entry-budget-{budget}"),
            &p,
            &schema("Use"),
            json!("s"),
            expected,
            at,
            "",
        ));
    }
    let c = load(
        api(
            json!({"Base":{"$id":"urn:base","$dynamicAnchor":"x","type":"string"},"Use":{"$id":"urn:use","$dynamicRef":"urn:base#x","$defs":{"A":{"$dynamicAnchor":"a"},"X":{"$dynamicAnchor":"x","type":"string"}}}}),
        ),
        vec![],
    );
    for (budget, expected) in [(8, "Valid"), (5, "EvaluationFailure")] {
        let p = program(
            c.clone(),
            &[schema("Use"), schema("Use").child("$defs").child("A")],
            Config {
                max_evaluation_steps: budget,
                ..limits()
            },
        );
        out.push(case(
            &format!("binding-scan-and-no-fallback-entry-{budget}"),
            &p,
            &schema("Use"),
            json!("s"),
            expected,
            if budget == 5 {
                Some("/components/schemas/Use/$dynamicRef")
            } else {
                None
            },
            "",
        ));
    }
    for (name, use_site) in [
        (
            "if",
            json!({"if":{"$dynamicRef":"urn:number#node"},"then":true,"else":true}),
        ),
        (
            "anyof",
            json!({"anyOf":[true,{"$dynamicRef":"urn:number#node"}]}),
        ),
        ("not", json!({"not":{"$dynamicRef":"urn:number#node"}})),
    ] {
        let c = load(
            api(
                json!({"Use":use_site,"Number":{"$id":"urn:number","$dynamicAnchor":"node","maximum":0}}),
            ),
            vec![],
        );
        let p = program(
            c,
            &[schema("Use")],
            Config {
                max_number_bytes: 3,
                max_errors: 1,
                ..limits()
            },
        );
        out.push(case(
            &format!("noninvertible-{name}"),
            &p,
            &schema("Use"),
            json!(12345),
            "EvaluationFailure",
            Some("/components/schemas/Number/maximum"),
            "",
        ));
    }
    let c = load(
        api(
            json!({"Root":{"$id":"urn:cycle","$dynamicAnchor":"x","anyOf":[true,{"$dynamicRef":"#x"}]}}),
        ),
        vec![],
    );
    let p = program(c, &[schema("Root")], limits());
    out.push(case(
        "nonproductive-dynamic-cycle",
        &p,
        &schema("Root"),
        Value::Null,
        "EvaluationFailure",
        Some("/components/schemas/Root"),
        "",
    ));
    out
}
fn malformed(cases: &[Value]) -> Vec<Value> {
    let original = &cases
        .iter()
        .find(|c| c["id"] == "strict-tree-success")
        .unwrap()["program"];
    let mut out = Vec::new();
    for change in 0..20 {
        let mut value = original.clone();
        match change {
            0 => {
                value.as_object_mut().unwrap().remove("resourceContext");
            }
            1 => {
                value["resourceContext"]["nodeScopes"]
                    .as_array_mut()
                    .unwrap()
                    .pop();
            }
            2 => value["resourceContext"]["nodeScopes"][0][0] = json!(-1),
            3 => value["resourceContext"]["nodeScopes"][0][2] = json!("urn:wrong"),
            4 => value["resourceContext"]["resources"][0]["aliases"] = json!([]),
            5 => {
                let alias = value["resourceContext"]["resources"][0]["canonicalUri"].clone();
                value["resourceContext"]["resources"][1]["aliases"]
                    .as_array_mut()
                    .unwrap()
                    .push(alias);
            }
            6 => value["resourceContext"]["resources"][0]["dynamicAnchors"][0][2] = json!(999999),
            7 | 8 => {
                let check = value["nodes"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .flat_map(|n| n["checks"].as_array_mut().unwrap())
                    .find(|c| c["op"] == "dynamicRef")
                    .unwrap();
                if change == 7 {
                    check["initialResource"] = json!(999999);
                } else {
                    check["anchor"] = json!("missing");
                }
            }
            9 => value["resourceContext"]["resources"][0]["kind"] = json!("invented"),
            10 => {
                value["resourceContext"]["resources"][0]["declarationSource"]["pointer"] =
                    json!("/not-the-declaration")
            }
            11 => {
                value["resourceContext"]["resources"][0]["source"]["pointer"] =
                    json!("/not-the-resource")
            }
            12 => {
                value["resourceContext"]["resources"][0]["canonicalUri"] =
                    json!("urn:other#nonempty")
            }
            13 => value["resourceContext"]["resources"][0]["baseUri"] = json!("urn:base#fragment"),
            14 => value["resourceContext"]["nodeScopes"][0][1]["pointer"] = json!("/unrelated"),
            15 => {
                let duplicate =
                    value["resourceContext"]["resources"][0]["dynamicAnchors"][0].clone();
                value["resourceContext"]["resources"][0]["dynamicAnchors"]
                    .as_array_mut()
                    .unwrap()
                    .push(duplicate);
            }
            16 => value["resourceContext"]["resources"][0]["aliases"]
                .as_array_mut()
                .unwrap()
                .push(json!("urn:bad space")),
            17 => {
                let alias = value["resourceContext"]["resources"][0]["baseUri"]
                    .as_str()
                    .unwrap()
                    .replacen("urn:", "URN:", 1)
                    + "#";
                value["resourceContext"]["resources"][1]["aliases"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!(alias));
            }
            18 => value["resourceContext"]["resources"][0]["aliases"]
                .as_array_mut()
                .unwrap()
                .push(json!("urn:bad#%FF")),
            _ => value["nodes"][0]["checks"][0]["op"] = json!("unknownResourceOpcode"),
        }
        out.push(json!({"id":format!("metadata-{change}"),"program":value}));
    }
    for (version, profile) in [
        (OwnedProgram::V1_VERSION, OwnedProgram::V1_PROFILE),
        (OwnedProgram::V2_VERSION, OwnedProgram::V2_PROFILE),
    ] {
        let mut value = original.clone();
        value["version"] = json!(version);
        value["profile"] = json!(profile);
        out.push(json!({"id":format!("downgrade-{version}"),"program":value}));
        value.as_object_mut().unwrap().remove("resourceContext");
        out.push(json!({"id":format!("old-opcode-{version}"),"program":value}));
    }
    out
}
fn depth_program() -> OwnedProgram {
    let mut schemas = serde_json::Map::new();
    for n in 0..150 {
        schemas.insert(
            format!("N{n}"),
            json!({"$id":format!("urn:n{n}"),"$ref":format!("urn:n{}",n+1)}),
        );
    }
    schemas.insert("N150".into(), json!({"$id":"urn:n150"}));
    program(
        load(api(Value::Object(schemas)), vec![]),
        &[schema("N0")],
        limits(),
    )
}
fn java() -> PathBuf {
    std::env::var_os("JAVA_HOME")
        .map(PathBuf::from)
        .expect("select JDK 21 or 25 using JAVA_HOME")
}
fn checked(command: &mut Command, root: &Path) {
    let result = command.output().unwrap();
    let logs = root.join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    let n = std::fs::read_dir(&logs).unwrap().count();
    std::fs::write(
        logs.join(format!("{n:03}.log")),
        format!(
            "{command:?}\nstatus={}\n{}{}",
            result.status,
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        ),
    )
    .unwrap();
    assert!(
        result.status.success(),
        "{}\n{command:?}\n{}{}",
        root.display(),
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
fn install(plan: &SdkPlan, root: &Path) -> PathBuf {
    for file in plan.render().unwrap() {
        let path = root.join(file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, file.content).unwrap();
    }
    let repository = crate_root().join("../../target/sdk-java-maven-cache/java/repository");
    let maven = std::env::var_os("SUSPECT_MAVEN_BIN").unwrap_or_else(|| {
        "/Users/luke/.local/share/mise/installs/maven/3.9.16/apache-maven-3.9.16/bin/mvn".into()
    });
    checked(
        Command::new(maven)
            .args(["-B", "-q", "install"])
            .arg(format!("-Dmaven.repo.local={}", repository.display()))
            .env("JAVA_HOME", java())
            .current_dir(root.join("java")),
        root,
    );
    let artifact = &plan.maven().artifact_id;
    let version = &plan.package().version;
    let installed = repository
        .join(plan.maven_group_id().replace('.', "/"))
        .join(artifact)
        .join(version)
        .join(format!("{artifact}-{version}.jar"));
    assert_eq!(
        std::fs::read(&installed).unwrap(),
        std::fs::read(root.join(format!("java/target/{artifact}-{version}.jar"))).unwrap()
    );
    installed
}
fn baseline() -> SdkPlan {
    let c = load(api(json!({"Base":{"type":"string"}})), vec![]);
    java_sdk::plan_sdk_with_maven(
        c,
        &[],
        PackageConfig {
            package: "example.schemav3".into(),
            version: "1.0.0".into(),
            api_name: "Client".into(),
        },
        &[schema("Base")],
        MavenConfig {
            artifact_id: "java-schema-v3-runtime".into(),
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
#[ignore = "unmodified 44 official source cases and independent V3 controls; installed JDK21/25 jar"]
fn native_schema_v3_resource_conformance() {
    let root = attempt();
    let jar = install(&baseline(), &root);
    let mut cases = official_cases();
    cases.extend(controls());
    let malformed = malformed(&cases);
    let depth = depth_program();
    std::fs::write(
        root.join("vectors.json"),
        json!({"cases":cases,"malformed":malformed,"depth":depth}).to_string(),
    )
    .unwrap();
    std::fs::write(
        root.join("NativeSchemaV3.java"),
        include_str!("../src/java_sdk/NativeSchemaV3.java"),
    )
    .unwrap();
    checked(
        Command::new(java().join("bin/javac"))
            .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
            .arg(&jar)
            .arg(root.join("NativeSchemaV3.java")),
        &root,
    );
    checked(
        Command::new(java().join("bin/java"))
            .args(["-ea", "-cp"])
            .arg(format!("{}:{}", jar.display(), root.display()))
            .arg("NativeSchemaV3")
            .arg(root.join("vectors.json")),
        &root,
    );
    println!("JAVA_SCHEMA_V3_CONFORMANCE {}", root.display());
}

fn sdk_contract() -> Arc<Contract> {
    let content = |reference: &str, example: Value| json!({"application/json":{"schema":{"$ref":reference},"example":example}});
    let numbers = content("urn:numbers", json!({"value":9007199254740993u64}));
    let strings = content("urn:strings", json!({"value":"text"}));
    let strict = content(
        "urn:strict",
        json!({"data":"root","children":[{"data":"child"}]}),
    );
    load(
        json!({"openapi":"3.2.0","$self":"https://logical.java.test/description.json#revision","info":{"title":"Actual resource/dynamic SDK operations","version":"1"},"servers":[{"url":"/v3"}],"security":[],
            "paths":{
                "/numbers":{"post":{"operationId":"numbers","requestBody":{"required":true,"content":numbers},"responses":{"200":{"description":"dynamic integers","content":numbers},"422":{"description":"typed dynamic error","content":numbers}}}},
                "/strings":{"post":{"operationId":"strings","requestBody":{"required":true,"content":strings},"responses":{"200":{"description":"dynamic strings","content":strings}}}},
                "/strict":{"post":{"operationId":"strictTree","requestBody":{"required":true,"content":strict},"responses":{"200":{"description":"recursive dynamic annotations","content":strict}}}},
                "/count":{"get":{"operationId":"count","responses":{"200":{"description":"logical static ref","content":content("https://logical.java.test/schema/count",json!(9007199254740993u64))}}}},
                "/number-lines":{"get":{"operationId":"numberLines","responses":{"200":{"description":"resource-bound items","content":{"application/jsonl":{"itemSchema":{"$ref":"urn:numbers"}}}}}}}
            },"components":{"schemas":{
                "Template":{"$id":"urn:template","type":"object","required":["value"],"properties":{"value":{"$dynamicRef":"#slot"}},"$defs":{"Slot":{"$dynamicAnchor":"slot"}}},
                "Numbers":{"$id":"urn:numbers","$ref":"urn:template","$defs":{"Slot":{"$dynamicAnchor":"slot","type":"integer","minimum":9007199254740993u64}}},
                "Strings":{"$id":"urn:strings","$ref":"urn:template","$defs":{"Slot":{"$dynamicAnchor":"slot","type":"string","minLength":2}}},
                "Tree":{"$id":"urn:tree","$dynamicAnchor":"node","type":"object","properties":{"data":true,"children":{"type":"array","items":{"$dynamicRef":"#node"}}}},
                "Strict":{"$id":"urn:strict","$dynamicAnchor":"node","$ref":"urn:tree","unevaluatedProperties":false},
                "Count":{"$id":"schema/count","type":"integer","minimum":9007199254740993u64}
            }}
        }),
        vec![],
    )
}
fn sdk_plan() -> SdkPlan {
    let c = sdk_contract();
    let operations = c
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    java_sdk::plan_sdk_with_protocol_v3(
        c,
        &operations,
        PackageConfig {
            package: "example.schemav3".into(),
            version: "1.0.0".into(),
            api_name: "Client".into(),
        },
        &[],
        MavenConfig {
            artifact_id: "java-schema-v3-operations".into(),
            ..Default::default()
        },
        Default::default(),
    )
    .unwrap()
}

#[test]
fn explicit_v3_preserves_base_plans_and_old_entrypoint_refusals() {
    use suspect_codegen::{
        backend::{Backend, TargetConfig},
        compatibility,
    };
    let base = baseline();
    let explicit = java_sdk::plan_sdk_with_protocol_v3(
        base.contract().clone(),
        &[],
        base.package().clone(),
        &[schema("Base")],
        base.maven().clone(),
        Default::default(),
    )
    .unwrap();
    assert_eq!(base.program(), explicit.program());
    assert_eq!(base.render().unwrap(), explicit.render().unwrap());
    assert_eq!(explicit.program().version, OwnedProgram::V1_VERSION);
    let c = load(
        api(
            json!({"Scoped":{"type":"object","properties":{"name":{"type":"string"}},"unevaluatedProperties":false}}),
        ),
        vec![],
    );
    let roots = [schema("Scoped")];
    let base = java_sdk::plan_sdk_with_protocol(
        c.clone(),
        &[],
        PackageConfig::default(),
        &roots,
        MavenConfig::default(),
        Default::default(),
    )
    .unwrap();
    let next = java_sdk::plan_sdk_with_protocol_v3(
        c,
        &[],
        PackageConfig::default(),
        &roots,
        MavenConfig::default(),
        Default::default(),
    )
    .unwrap();
    assert_eq!(base.program().version, OwnedProgram::V2_VERSION);
    assert_eq!(base.render().unwrap(), next.render().unwrap());
    let c = sdk_contract();
    let operations = c
        .operations()
        .map(|o| o.source().clone())
        .collect::<Vec<_>>();
    assert!(java_sdk::plan_sdk(c.clone(), &operations, PackageConfig::default(), &[]).is_err());
    assert!(java_sdk::validation::plan_validation(&c, &[schema("Template")]).is_err());
    let standalone = java_sdk::validation::plan_validation_v3(&c, &[schema("Template")]).unwrap();
    assert_eq!(standalone.version, OwnedProgram::V3_VERSION);
    let plan = sdk_plan();
    assert_eq!(plan.program().version, OwnedProgram::V3_VERSION);
    assert!(
        plan.protocol()
            .capabilities()
            .supports(suspect_codegen::http_protocol::Capability::SchemaResources)
    );
    assert!(
        plan.protocol()
            .capabilities()
            .supports(suspect_codegen::http_protocol::Capability::DynamicSchemaReferences)
    );
    let dynamic = schema("Template").child("properties").child("value");
    assert_eq!(plan.models().native_type(&dynamic), "JsonValue");
    assert!(
        plan.program()
            .resource_context
            .as_ref()
            .unwrap()
            .node_scopes
            .len()
            == plan.program().nodes.len()
    );
    assert!(
        plan.native_examples()
            .iter()
            .all(|e| e.input_expression.is_some())
    );
    // This item has no source example. V3 deliberately does not borrow an
    // annotation from a dynamic fallback to manufacture a caller-context value.
    assert_eq!(plan.examples().diagnostics().len(), 1);
    let unavailable = &plan.examples().diagnostics()[0];
    assert_eq!(unavailable.code, "examples-synthesis-unsupported");
    assert_eq!(
        unavailable.source.pointer(),
        "/paths/~1number-lines/get/responses/200/content/application~1jsonl/itemSchema"
    );
    let snapshot = compatibility::snapshot(
        plan.contract().clone(),
        &[],
        &[TargetConfig {
            backend: Backend::JavaHttp,
            package_name: "example.schemav3:java-schema-v3-operations".into(),
            package_version: "1.0.0".into(),
            import_name: Some("example.schemav3".into()),
        }],
    )
    .unwrap()
    .native
    .remove(0);
    assert_eq!(
        snapshot.status,
        compatibility::PlanStatus::Planned,
        "{:?}",
        snapshot.findings
    );
    assert!(snapshot.models.iter().all(|m| m.source.document == ENTRY));
    let numbers = snapshot
        .operations
        .iter()
        .find(|o| o.operation_id == "numbers")
        .unwrap();
    assert_eq!(
        numbers.descriptor["body"]["type"]["name"],
        "example.schemav3.Template"
    );
    let dynamic = snapshot
        .models
        .iter()
        .find(|m| {
            m.role == "model" && m.source.pointer == "/components/schemas/Template/properties/value"
        })
        .unwrap();
    assert_eq!(
        dynamic.descriptor.as_ref().unwrap()["nativeType"]["name"],
        "example.schemav3.JsonRuntime.JsonValue"
    );
}

#[test]
#[ignore = "installed resource/dynamic SDK, examples/Javadoc/type and wire/control consumers on JDK21/25"]
fn native_schema_v3_sdk_operations() {
    let root = attempt();
    let plan = sdk_plan();
    let jar = install(&plan, &root);
    std::fs::write(
        root.join("NativeSupport.java"),
        include_str!("../src/java_sdk/NativeSupport.java"),
    )
    .unwrap();
    std::fs::write(
        root.join("NativeSchemaV3Operations.java"),
        include_str!("../src/java_sdk/NativeSchemaV3Operations.java"),
    )
    .unwrap();
    checked(
        Command::new(java().join("bin/java"))
            .args(["-ea", "-cp"])
            .arg(&jar)
            .arg("example.schemav3.SdkExamples"),
        &root,
    );
    checked(
        Command::new(java().join("bin/javac"))
            .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
            .arg(&jar)
            .args(["NativeSupport.java", "NativeSchemaV3Operations.java"])
            .current_dir(&root),
        &root,
    );
    checked(
        Command::new(java().join("bin/java"))
            .args(["-ea", "-cp"])
            .arg(format!("{}:{}", jar.display(), root.display()))
            .arg("NativeSchemaV3Operations"),
        &root,
    );
    for (index, body) in [
        "Template.builder();",
        "Template.builder(1);",
        "String value=Template.builder(JsonNull.INSTANCE).build().value();",
        "client.numbers(NumbersInput.builder(JsonNumber.of(2)).build());",
        "Tree.builder().children(java.util.List.of(1)).build();",
        "new Strict();",
    ]
    .iter()
    .enumerate()
    {
        let path = root.join(format!("Negative{index}.java"));
        std::fs::write(&path,format!("import example.schemav3.*; import example.schemav3.Client.*; import static example.schemav3.JsonRuntime.*; final class Negative{index} {{ void call(Client client) {{ {body} }} }}")).unwrap();
        let result = Command::new(java().join("bin/javac"))
            .args(["--release", "21", "-cp"])
            .arg(&jar)
            .arg(&path)
            .output()
            .unwrap();
        std::fs::write(
            root.join(format!("negative-{index}.log")),
            format!(
                "status={}\n{}{}",
                result.status,
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            ),
        )
        .unwrap();
        assert!(
            !result.status.success(),
            "negative resource consumer {index} compiled"
        );
    }
    let quickstart = root.join("java/examples/GettingStarted.java");
    assert!(quickstart.exists());
    checked(
        Command::new(java().join("bin/javac"))
            .args(["--release", "21", "-Xlint:all", "-Werror", "-cp"])
            .arg(&jar)
            .arg("-d")
            .arg(&root)
            .arg(quickstart),
        &root,
    );
    for name in ["Template", "Numbers", "Strings", "Tree", "Strict", "Count"] {
        assert!(
            root.join(format!(
                "java/target/reports/apidocs/example/schemav3/{name}.html"
            ))
            .is_file(),
            "missing Javadoc for {name}"
        );
    }
    println!("JAVA_SCHEMA_V3_OPERATIONS {}", root.display());
}
