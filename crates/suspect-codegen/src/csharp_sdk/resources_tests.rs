//! Source-compiled v3 resource/dynamic native and installed SDK witnesses.
use super::validation_tests::{Native, write};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use suspect_ir::contract::{Contract, SchemaId};
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_schema::{Config, OwnedCompiler, OwnedProgram};
use suspect_source::Uri;

const TIERS: [(&str, &str); 2] = [("8.0.424", "net8.0"), ("10.0.400", "net10.0")];
const ENTRY: &str = "https://physical.csharp.test/api.json";
const OFFICIAL: &str =
    include_str!("../../../suspect-schema/tests/fixtures/resource-conformance/dynamicRef.json");
const REMOTES: [(&str, &str); 3] = [
    (
        "tree.json",
        include_str!("../../../suspect-schema/tests/fixtures/resource-conformance/tree.json"),
    ),
    (
        "extendible-dynamic-ref.json",
        include_str!(
            "../../../suspect-schema/tests/fixtures/resource-conformance/extendible-dynamic-ref.json"
        ),
    ),
    (
        "detached-dynamicref.json",
        include_str!(
            "../../../suspect-schema/tests/fixtures/resource-conformance/detached-dynamicref.json"
        ),
    ),
];

fn directory(prefix: &str) -> PathBuf {
    let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/sdk-csharp-schema-v3");
    fs::create_dir_all(&parent).unwrap();
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir_in(fs::canonicalize(parent).unwrap())
        .unwrap()
        .keep()
}
fn source(document: &str, pointer: &str) -> SchemaId {
    let root = SchemaId::new(Uri::parse(document).unwrap(), Default::default());
    pointer.strip_prefix('/').map_or(root.clone(), |tail| {
        tail.split('/').fold(root, |id, part| {
            id.child(&part.replace("~1", "/").replace("~0", "~"))
        })
    })
}
fn api(schemas: Value) -> Value {
    json!({"openapi":"3.2.0","info":{"title":"Resource native witnesses","version":"1"},"paths":{},"components":{"schemas":schemas}})
}
fn provided(entry: &str, documents: &[(String, String, Vec<u8>)]) -> Arc<Contract> {
    let provider = Arc::new(
        DocumentProvider::new(documents.iter().map(|(requested, effective, bytes)| {
            ProvidedDocument::new(
                Uri::parse(requested).unwrap(),
                Uri::parse(effective).unwrap(),
                bytes.clone(),
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
    Arc::new(Contract::from_workspace(&workspace, &Uri::parse(entry).unwrap()).unwrap())
}
fn compiled(contract: Arc<Contract>, roots: &[SchemaId], config: Config) -> OwnedProgram {
    let result = OwnedCompiler::new(config)
        .compile_v3(contract.clone(), roots)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let program = result.program();
    program.check().unwrap();
    assert_eq!(
        (program.version, program.profile),
        (OwnedProgram::V3_VERSION, OwnedProgram::V3_PROFILE)
    );
    for node in &program.nodes {
        let id = source(&node.source.document, &node.source.pointer);
        assert!(contract.schema(&id).is_some());
        assert!(contract.source_span(&id).is_some());
    }
    program
}
fn official_vectors(root: &Path) -> Value {
    write(&root.join("fixtures/dynamicRef.json"), OFFICIAL);
    for (name, text) in REMOTES {
        write(&root.join("fixtures").join(name), text);
    }
    let groups: Vec<Value> = serde_json::from_str(OFFICIAL).unwrap();
    let mut cases = Vec::new();
    for (group_index, group) in groups.iter().enumerate() {
        let physical = "https://physical.csharp.test/official-schema.json";
        let entry = api(json!({"Use":{"$ref":physical}}));
        let mut documents = vec![
            (ENTRY.into(), ENTRY.into(), entry.to_string().into_bytes()),
            (
                physical.into(),
                physical.into(),
                group["schema"].to_string().into_bytes(),
            ),
        ];
        documents.extend(REMOTES.iter().map(|(name, text)| {
            let uri = format!("http://localhost:1234/draft2020-12/{name}");
            (uri.clone(), uri, text.as_bytes().to_vec())
        }));
        write(&root.join(format!("sources/group-{group_index:02}.json")), serde_json::to_vec_pretty(&json!({"entry":ENTRY,"documents":documents.iter().map(|(requested,effective,bytes)|json!({"requested":requested,"effective":effective,"text":String::from_utf8_lossy(bytes)})).collect::<Vec<_>>()})).unwrap());
        let program = compiled(
            provided(ENTRY, &documents),
            &[source(physical, "")],
            Config {
                max_depth: 128,
                ..Default::default()
            },
        );
        for (case_index, case) in group["tests"].as_array().unwrap().iter().enumerate() {
            cases.push(json!({"id":format!("official-{group_index:02}-{case_index:02}"),"group":group["description"],"description":case["description"],"rootTarget":program.roots[0].target,"program":program,"instanceJson":case["data"].to_string(),"expected":if case["valid"].as_bool().unwrap(){"Valid"}else{"Invalid"}}));
        }
    }
    assert_eq!(cases.len(), 44);
    json!({"format":"suspect-csharp-resources-source-vectors-v1","oracle":"unmodified resource-conformance/dynamicRef.json and closed supplied remote documents","expectedCount":44,"cases":cases})
}
fn run_vectors(root: &Path, vectors: &Value) {
    write(
        &root.join("source-programs.json"),
        serde_json::to_vec_pretty(vectors).unwrap(),
    );
    for (sdk, framework) in TIERS {
        let native = Native::new(root, sdk, framework);
        native.project("validation", None, true);
        let mut assets = super::resources::runtime();
        assets.push(("JsonRuntime.cs", include_str!("JsonRuntime.cs").into()));
        for (name, text) in assets {
            write(
                &native.root.join("validation").join(name),
                text.replace("__NAMESPACE__", "Resources.Csharp"),
            );
        }
        write(
            &native.root.join("validation/Program.cs"),
            include_str!("testdata/ResourceValidationConsumer.cs"),
        );
        write(
            &native.root.join("validation/Controls.cs"),
            include_str!("testdata/ResourceValidationControls.cs"),
        );
        write(
            &native.root.join("validation/vectors.json"),
            serde_json::to_vec_pretty(vectors).unwrap(),
        );
        native.checked(
            "validation",
            "restore",
            &["restore", "--configfile", "../NuGet.Config"],
        );
        native.checked(
            "validation",
            "source-vectors",
            &["run", "-c", "Release", "--no-restore"],
        );
        native.finish("source-driven-resources-v3");
    }
    println!("C# resource native evidence: {}", root.display());
}

#[test]
#[ignore = "44 original official dynamicRef cases compiled by compile_v3, on pinned .NET 8/10"]
fn native_resource_source_vectors() {
    let root = directory("vectors-");
    let vectors = official_vectors(&root);
    run_vectors(&root, &vectors);
}

fn control_vectors(root: &Path) -> Value {
    let text = include_str!("testdata/resources-controls.json");
    write(&root.join("fixtures/resources-controls.json"), text);
    let fixture: Value = serde_json::from_str(text).unwrap();
    let mut cases = Vec::new();
    for group in fixture["groups"].as_array().unwrap() {
        let entry = api(group["schemas"].clone());
        let mut documents = vec![(ENTRY.into(), ENTRY.into(), entry.to_string().into_bytes())];
        for extra in group["extraDocuments"].as_array().into_iter().flatten() {
            documents.push((
                extra["requested"].as_str().unwrap().into(),
                extra["effective"].as_str().unwrap().into(),
                extra["value"].to_string().into_bytes(),
            ));
        }
        write(&root.join(format!("sources/{}.json",group["id"].as_str().unwrap())),serde_json::to_vec_pretty(&json!({"entry":ENTRY,"documents":documents.iter().map(|(requested,effective,bytes)|json!({"requested":requested,"effective":effective,"text":String::from_utf8_lossy(bytes)})).collect::<Vec<_>>()})).unwrap());
        let contract = provided(ENTRY, &documents);
        let mut roots = group["roots"]
            .as_array()
            .unwrap()
            .iter()
            .map(|name| source(ENTRY, "/components/schemas").child(name.as_str().unwrap()))
            .collect::<Vec<_>>();
        roots.extend(
            group["rootSources"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|at| source(at[0].as_str().unwrap(), at[1].as_str().unwrap())),
        );
        for case in group["tests"].as_array().unwrap() {
            let mut config = Config {
                max_depth: 128,
                ..Default::default()
            };
            for (name, value) in case["limits"].as_object().into_iter().flatten() {
                let value = value.as_u64().unwrap() as usize;
                match name.as_str() {
                    "maxEvaluationSteps" => config.max_evaluation_steps = value,
                    "maxEqualitySteps" => config.max_equality_steps = value,
                    "maxNumberBytes" => config.max_number_bytes = value,
                    "maxDepth" => config.max_depth = value,
                    _ => panic!("unsupported control limit {name}"),
                }
            }
            let program = compiled(contract.clone(), &roots, config);
            for absent in group["absentNodes"].as_array().into_iter().flatten() {
                assert!(
                    !program
                        .nodes
                        .iter()
                        .any(|node| node.source.document == absent[0].as_str().unwrap()
                            && node.source.pointer == absent[1].as_str().unwrap()),
                    "entering a nested resource may not invent root evaluation"
                );
            }
            let selected = case["root"].as_str().map_or_else(
                || {
                    source(
                        case["rootDocument"].as_str().unwrap(),
                        case["rootPointer"].as_str().unwrap(),
                    )
                },
                |name| source(ENTRY, "/components/schemas").child(name),
            );
            let index = program
                .roots
                .iter()
                .find(|r| {
                    r.source.document == selected.document().as_str()
                        && r.source.pointer == selected.pointer()
                })
                .unwrap()
                .target;
            let mut vector = json!({"id":case["id"],"rootTarget":index,"program":program,"instanceJson":case["instanceJson"],"expected":case["expected"]});
            if let Some(at) = case["source"].as_str() {
                vector["source"] = json!(format!(
                    "{}#{at}",
                    case["document"].as_str().unwrap_or(ENTRY)
                ));
            }
            if case["instancePointer"].is_string() {
                vector["instancePointer"] = case["instancePointer"].clone();
            }
            cases.push(vector);
        }
    }
    assert_eq!(cases.len(), 38);
    let mut schemas = serde_json::Map::new();
    for i in 0..140 {
        schemas.insert(format!("Depth{i}"),if i==139 {json!({"$id":format!("urn:csharp:depth-{i}"),"type":"integer"})} else {json!({"$id":format!("urn:csharp:depth-{i}"),"$ref":format!("urn:csharp:depth-{}",i+1)})});
    }
    let entry = api(Value::Object(schemas));
    write(
        &root.join("sources/depth.json"),
        serde_json::to_vec_pretty(&entry).unwrap(),
    );
    let program = compiled(
        provided(
            ENTRY,
            &[(ENTRY.into(), ENTRY.into(), entry.to_string().into_bytes())],
        ),
        &[source(ENTRY, "/components/schemas/Depth0")],
        Config {
            max_depth: 128,
            ..Default::default()
        },
    );
    cases.push(json!({"id":"distinct-resource-depth-on-two-mib-stack","rootTarget":program.roots[0].target,"program":program,"instanceJson":"1","expected":"EvaluationFailure","source":format!("{ENTRY}#/components/schemas/Depth128"),"instancePointer":"","stackBytes":2_097_152}));
    for (name, text) in [
        (
            "unevaluatedProperties",
            include_str!(
                "../../../suspect-schema/tests/conformance/draft2020-12/unevaluatedProperties.json"
            ),
        ),
        (
            "unevaluatedItems",
            include_str!(
                "../../../suspect-schema/tests/conformance/draft2020-12/unevaluatedItems.json"
            ),
        ),
    ] {
        let groups: Vec<Value> = serde_json::from_str(text).unwrap();
        for group in groups
            .iter()
            .filter(|g| g["description"] == format!("{name} with $dynamicRef"))
        {
            let entry = api(json!({"Root":group["schema"]}));
            write(
                &root.join(format!("sources/official-{name}.json")),
                serde_json::to_vec_pretty(&entry).unwrap(),
            );
            let program = compiled(
                provided(
                    ENTRY,
                    &[(ENTRY.into(), ENTRY.into(), entry.to_string().into_bytes())],
                ),
                &[source(ENTRY, "/components/schemas/Root")],
                Config {
                    max_depth: 128,
                    ..Default::default()
                },
            );
            for (index, case) in group["tests"].as_array().unwrap().iter().enumerate() {
                cases.push(json!({"id":format!("official-{name}-{index}"),"rootTarget":program.roots[0].target,"program":program,"instanceJson":case["data"].to_string(),"expected":if case["valid"].as_bool().unwrap(){"Valid"}else{"Invalid"}}));
            }
        }
    }
    assert_eq!(cases.len(), 43);
    json!({"format":"suspect-csharp-resources-source-controls-v1","expectedCount":cases.len(),"cases":cases,"nativeControls":true})
}

#[test]
#[ignore = "independent v3 resource scopes, context cycles, exact costs, admission and ownership on .NET 8/10"]
fn native_resource_scope_and_admission() {
    let root = directory("controls-");
    let vectors = control_vectors(&root);
    run_vectors(&root, &vectors);
}

fn sdk_contract(fixture: &Value) -> Arc<Contract> {
    let documents = fixture["documents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["requested"].as_str().unwrap().to_owned(),
                d["effective"].as_str().unwrap().to_owned(),
                d["value"].to_string().into_bytes(),
            )
        })
        .collect::<Vec<_>>();
    provided(fixture["entry"].as_str().unwrap(), &documents)
}
fn sdk_plan(contract: Arc<Contract>) -> super::SdkPlan {
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    super::plan_sdk(
        contract,
        &selected,
        super::SdkConfig {
            name: "Resources.Csharp".into(),
            version: "1.0.0".into(),
            namespace: "Resources.Csharp".into(),
        },
    )
    .unwrap_or_else(|errors| panic!("{errors:#?}"))
}
fn sdk_fixture(root: &Path) -> super::SdkPlan {
    let text = include_str!("testdata/resource-sdk-documents.json");
    write(&root.join("source-documents.json"), text);
    sdk_plan(sdk_contract(&serde_json::from_str(text).unwrap()))
}
fn native_name<'a>(plan: &'a super::SdkPlan, document: &str, pointer: &str) -> &'a str {
    plan.models().codec_name(&source(document, pointer))
}
fn consumer(plan: &super::SdkPlan) -> String {
    let mut text = include_str!("testdata/ResourceSdkConsumer.cs").to_owned();
    let entry = "http://api.csharp.test/v1/spec/openapi.json";
    for name in ["Tree", "Strict", "Envelope", "NullableEnvelope"] {
        text = text.replace(
            &format!("__{}__", name.to_uppercase()),
            native_name(plan, entry, &format!("/components/schemas/{name}")),
        );
    }
    for (name, document, pointer) in [
        (
            "CHOICE",
            "http://storage.csharp.test/schema/choice.json",
            "",
        ),
        (
            "INTEGER",
            "http://storage.csharp.test/schema/catalog.json",
            "/$defs/a~1b~0% #é",
        ),
        (
            "NESTED",
            "http://storage.csharp.test/schema/detached.json",
            "/$defs/start",
        ),
    ] {
        text = text.replace(&format!("__{name}__"), native_name(plan, document, pointer));
    }
    let strict = plan
        .operations()
        .iter()
        .find(|op| op.operation_id == "sendStrictTree")
        .unwrap();
    text = text.replace(
        "__STRICT_ERROR__",
        &strict
            .responses
            .iter()
            .find(|r| r.wire.status_key() == "422")
            .unwrap()
            .error_type_name,
    );
    let integer = plan
        .operations()
        .iter()
        .find(|op| op.operation_id == "sendInteger")
        .unwrap();
    text.replace("__REVISION__", &integer.parameters[0].property_name)
}

#[test]
fn source_resource_sdk_models_and_metadata() {
    let root = directory("planning-");
    let plan = sdk_fixture(&root);
    assert_eq!(
        (plan.program().version, plan.program().profile),
        (OwnedProgram::V3_VERSION, OwnedProgram::V3_PROFILE)
    );
    assert_eq!(plan.operations().len(), 8);
    let entry = "http://api.csharp.test/v1/spec/openapi.json";
    let tree = source(entry, "/components/schemas/Tree");
    let super::models::CsDecl::Record { fields, .. } =
        &plan.models().declarations()[&super::models::key(&tree)]
    else {
        panic!("declared object fields must remain typed")
    };
    assert_eq!(
        plan.models()
            .render_type(&fields.iter().find(|f| f.wire == "amount").unwrap().ty),
        "JsonNumber"
    );
    assert_eq!(
        plan.models()
            .render_type(&fields.iter().find(|f| f.wire == "children").unwrap().ty),
        "global::System.Collections.Generic.List<global::System.Text.Json.JsonElement>"
    );
    let envelope = source(entry, "/components/schemas/Envelope");
    let super::models::CsDecl::Record { fields, extras } =
        &plan.models().declarations()[&super::models::key(&envelope)]
    else {
        panic!("typed envelope")
    };
    assert_eq!(
        plan.models()
            .render_type(&fields.iter().find(|f| f.wire == "value").unwrap().ty),
        "global::System.Text.Json.JsonElement"
    );
    assert_eq!(
        plan.models().render_type(extras.as_ref().unwrap()),
        "global::System.Text.Json.JsonElement"
    );
    assert!(
        plan.models()
            .is_json_carrier(&source("http://storage.csharp.test/schema/choice.json", ""))
    );
    assert!(!plan.program().nodes.iter().any(|node| node.source.document
        == "http://storage.csharp.test/schema/detached.json"
        && node.source.pointer.is_empty()));
    for node in &plan.program().nodes {
        if node.source.pointer.contains("/$defs/Binding") {
            assert!(
                !plan
                    .protocol()
                    .codec_roots()
                    .contains(&source(&node.source.document, &node.source.pointer)),
                "candidate declaration cannot become an HTTP codec input"
            );
        }
    }
    assert!(plan.protocol().codec_schema_closure().len() > plan.protocol().codec_roots().len());
    assert!(
        plan.examples().diagnostics().is_empty(),
        "{:?}",
        plan.examples().diagnostics()
    );
    assert_eq!(plan.examples().operations().len(), 8);
    for entry in plan
        .examples()
        .operations()
        .iter()
        .flat_map(|op| &op.entries)
    {
        assert_eq!(
            plan.contract().source(
                entry
                    .declared_source
                    .as_ref()
                    .expect("explicit source example")
            ),
            Some(&entry.value)
        );
    }
    let files = plan.render().unwrap();
    crate::write_files(&files, &root).unwrap();
    write(&root.join("consumer.cs"), consumer(&plan));
    write(&root.join("PASS.json"),json!({"gate":"resource-model-metadata-admission","models":plan.models().declarations().len(),"codecRoots":plan.protocol().codec_roots().len(),"closure":plan.protocol().codec_schema_closure().len(),"examples":plan.examples().operations().iter().map(|op|op.entries.len()).sum::<usize>(),"result":"passed"}).to_string());
    println!("C# resource planning evidence: {}", root.display());
}

#[test]
#[ignore = "installed v3 SDK models, mutation, TCP wire, compiler types and source-bound native docs/examples on .NET 8/10"]
fn native_resource_sdk_packages() {
    let root = directory("sdk-");
    let plan = sdk_fixture(&root);
    let files = plan.render().unwrap();
    assert!(
        plan.examples().diagnostics().is_empty(),
        "{:?}",
        plan.examples().diagnostics()
    );
    for (sdk, framework) in TIERS {
        let native = Native::new(&root, sdk, framework);
        crate::write_files(&files, &native.root).unwrap();
        native.checked(
            "csharp",
            "package-restore",
            &["restore", "--configfile", "../NuGet.Config"],
        );
        native.checked(
            "csharp",
            "package-pack",
            &["pack", "-c", "Release", "--no-restore", "-o", "../feed"],
        );
        native.project("consumer", Some("Resources.Csharp"), true);
        write(&native.root.join("consumer/Program.cs"), consumer(&plan));
        native.checked(
            "consumer",
            "consumer-restore",
            &["restore", "--configfile", "../NuGet.Config"],
        );
        native.checked(
            "consumer",
            "consumer-run",
            &[
                "run",
                "-c",
                "Release",
                "--no-restore",
                "--",
                "../feed/Resources.Csharp.1.0.0.nupkg",
            ],
        );
        native.project("negative", Some("Resources.Csharp"), false);
        let tree = native_name(
            &plan,
            "http://api.csharp.test/v1/spec/openapi.json",
            "/components/schemas/Tree",
        );
        let envelope = native_name(
            &plan,
            "http://api.csharp.test/v1/spec/openapi.json",
            "/components/schemas/Envelope",
        );
        write(
            &native.root.join("negative/Negative.cs"),
            format!(
                "using Resources.Csharp;using System.Text.Json;public static class Negative {{public static object Missing()=>new {tree}();public static object WrongNumber()=>new {tree}{{Name=\"x\",Amount=1.5}};public static object StaticFallback()=>new {envelope}{{Id=\"x\",Value=\"wrong\"}};public static object WrongUnion()=>new SendChoiceInput{{Body=\"wrong\"}};public static object WrongInteger()=>new SendIntegerInput{{Body=1.5}};}}"
            ),
        );
        native.checked(
            "negative",
            "negative-restore",
            &["restore", "--configfile", "../NuGet.Config"],
        );
        let output = native.run(
            "negative",
            "negative-build",
            &["build", "-c", "Release", "--no-restore"],
        );
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(
            !output.status.success()
                && [
                    "CS9035",
                    "CS0029",
                    "JsonElement",
                    "JsonNumber",
                    "JsonInteger"
                ]
                .iter()
                .all(|s| text.contains(s)),
            "{text}"
        );
        native.checked(
            "csharp/examples",
            "examples-restore",
            &["restore", "--configfile", "../../NuGet.Config"],
        );
        native.checked(
            "csharp/examples",
            "examples-run",
            &["run", "-c", "Release", "--no-restore"],
        );
        native.finish("installed-resource-v3-sdk");
    }
    println!("C# installed resource SDK evidence: {}", root.display());
}

#[test]
fn source_ordinary_profiles_keep_frozen_programs_and_runtimes() {
    let root = directory("profile-retention-");
    for (name, schema, version, asset) in [
        (
            "base",
            json!({"type":"object","required":["value"],"properties":{"value":{"type":"integer","minimum":0}},"additionalProperties":false}),
            OwnedProgram::V1_VERSION,
            "ValidationRuntime.cs",
        ),
        (
            "scoped",
            json!({"type":"object","properties":{"value":{"type":"integer"}},"dependentRequired":{"value":["peer"]},"unevaluatedProperties":true}),
            OwnedProgram::V2_VERSION,
            "ScopedValidationRuntime.cs",
        ),
    ] {
        let entry = json!({"openapi":"3.2.0","info":{"title":name,"version":"1"},"servers":[{"url":"https://ordinary.csharp.test"}],"paths":{"/value":{"post":{"operationId":"ordinary","requestBody":{"required":true,"content":{"application/json":{"schema":schema,"example":if name=="base"{json!({"value":1})}else{json!({"value":1,"peer":true})}}}},"responses":{"204":{"description":"done"}}}}}});
        let contract = provided(
            ENTRY,
            &[(ENTRY.into(), ENTRY.into(), entry.to_string().into_bytes())],
        );
        let plan = sdk_plan(contract.clone());
        let reachable = contract.reachable_from(plan.protocol().codec_roots());
        let frozen = OwnedCompiler::new(Config {
            max_depth: 128,
            ..Default::default()
        })
        .compile_v2(contract, &reachable)
        .unwrap()
        .program();
        assert_eq!(plan.program().version, version);
        assert_eq!(
            serde_json::to_vec(plan.program()).unwrap(),
            serde_json::to_vec(&frozen).unwrap()
        );
        assert!(plan.program().resource_context.is_none());
        let files = plan.render().unwrap();
        let emitted = files
            .iter()
            .find(|file| file.path == format!("csharp/src/{asset}"))
            .unwrap();
        let template = if name == "base" {
            include_str!("ValidationRuntime.cs")
        } else {
            include_str!("ScopedValidationRuntime.cs")
        };
        assert_eq!(
            emitted.content,
            template.replace("__NAMESPACE__", "Resources.Csharp")
        );
        assert!(
            !files
                .iter()
                .any(|file| file.path.ends_with("ResourceScope.cs")
                    || file.path.ends_with("ResourceProgramGuard.cs"))
        );
        if name == "scoped" {
            assert_eq!(
                files
                    .iter()
                    .find(|f| f.path == "csharp/src/ValidationProgramGuard.cs")
                    .unwrap()
                    .content,
                include_str!("ValidationProgramGuard.cs")
                    .replace("__NAMESPACE__", "Resources.Csharp")
            );
        }
        crate::write_files(&files, &root.join(name)).unwrap();
        write(&root.join(name).join("PASS.json"),json!({"version":version,"program":"identical-to-frozen-explicit-v2-entrypoint","runtime":"identical-frozen-asset","result":"passed"}).to_string());
    }
    println!("C# profile retention evidence: {}", root.display());
}

#[test]
fn source_resource_examples_and_unsupported_boundaries() {
    let root = directory("source-boundaries-");
    let fixture: Value =
        serde_json::from_str(include_str!("testdata/resource-sdk-documents.json")).unwrap();
    let mut invalid = fixture.clone();
    invalid["documents"][0]["value"]["paths"]["/envelope"]["post"]["requestBody"]["content"]["application/json"]
        ["example"] = json!({"id":"invalid","value":"fallback-string"});
    let plan = sdk_plan(sdk_contract(&invalid));
    assert!(
        plan.examples()
            .diagnostics()
            .iter()
            .any(|d| d.code == "examples-declared-invalid"
                && d.source.document().as_str() == "http://api.csharp.test/v1/spec/openapi.json"
                && d.source.pointer()
                    == "/paths/~1envelope/post/requestBody/content/application~1json/example"
                && d.at.end > d.at.start)
    );
    write(
        &root.join("invalid-example.json"),
        serde_json::to_vec_pretty(&invalid).unwrap(),
    );
    write(
        &root.join("invalid-example-findings.json"),
        crate::http_examples::manifest(plan.examples()),
    );
    let mut missing = fixture.clone();
    missing["documents"][0]["value"]["paths"]["/fallback"]["post"]["requestBody"]["content"]["application/json"].as_object_mut().unwrap().remove("example");
    let plan = sdk_plan(sdk_contract(&missing));
    let fallback = plan
        .examples()
        .operations()
        .iter()
        .find(|op| op.operation_id == "sendFallback")
        .unwrap();
    assert!(
        fallback
            .entries
            .iter()
            .all(|e| e.value != json!("fallback annotation must not become an overriding value"))
    );
    assert!(
        fallback.entries.iter().all(|e| e
            .declared_source
            .as_ref()
            .is_none_or(
                |s| s.document().as_str() != "http://storage.csharp.test/schema/fallback.json"
            ))
    );
    assert!(
        !plan.examples().diagnostics().is_empty(),
        "a missing dynamic request example must retain its bounded unavailable finding"
    );
    write(
        &root.join("missing-dynamic-example-findings.json"),
        crate::http_examples::manifest(plan.examples()),
    );
    for (name, schema, keyword) in [
        (
            "vocabulary",
            json!({"$id":"urn:csharp:custom","$vocabulary":{"urn:custom:vocabulary":true}}),
            "$vocabulary",
        ),
        (
            "legacy",
            json!({"$id":"urn:csharp:legacy","$recursiveRef":"#"}),
            "$recursiveRef",
        ),
        (
            "pattern",
            json!({"$id":"urn:csharp:pattern","type":"string","pattern":"(?=x)"}),
            "pattern",
        ),
        (
            "unresolved",
            json!({"$dynamicRef":"https://unprovided.csharp.test/schema#value"}),
            "$dynamicRef",
        ),
    ] {
        let entry = json!({"openapi":"3.2.0","info":{"title":name,"version":"1"},"servers":[{"url":"https://api.csharp.test"}],"paths":{"/boundary":{"post":{"operationId":"boundary","requestBody":{"required":true,"content":{"application/json":{"schema":schema}}},"responses":{"204":{"description":"done"}}}}}});
        let contract = provided(
            ENTRY,
            &[(ENTRY.into(), ENTRY.into(), entry.to_string().into_bytes())],
        );
        let selected = contract
            .operations()
            .map(|op| op.source().clone())
            .collect::<Vec<_>>();
        let errors = super::plan_sdk(contract.clone(), &selected, Default::default()).unwrap_err();
        let keyword_source = source(
            ENTRY,
            "/paths/~1boundary/post/requestBody/content/application~1json/schema",
        )
        .child(keyword);
        assert!(
            errors
                .iter()
                .any(|error| error.source.document().as_str() == ENTRY
                    && contract.source(&error.source).is_some()
                    && error.at.end > error.at.start
                    && (error.source == keyword_source
                        || contract.source_span(&keyword_source) == Some(error.at.clone()))),
            "{name}: {errors:#?}"
        );
        write(
            &root.join(format!("{name}.json")),
            serde_json::to_vec_pretty(&entry).unwrap(),
        );
        write(
            &root.join(format!("{name}-findings.txt")),
            format!("{errors:#?}"),
        );
    }
    write(
        &root.join("PASS.json"),
        json!({"result":"passed","gate":"source-bound-v3-examples-and-located-refusals"})
            .to_string(),
    );
    println!("C# resource source-boundary evidence: {}", root.display());
}

#[test]
fn source_canonical_resource_capture_keeps_dynamic_codec_obligations() {
    use crate::{
        backend::{Backend, TargetConfig},
        compatibility,
    };
    let root = directory("capture-");
    let fixture: Value =
        serde_json::from_str(include_str!("testdata/resource-sdk-documents.json")).unwrap();
    let target = TargetConfig {
        backend: Backend::CsharpHttp,
        package_name: "Resources.Csharp".into(),
        package_version: "1.0.0".into(),
        import_name: Some("Resources.Csharp".into()),
    };
    let before =
        compatibility::snapshot(sdk_contract(&fixture), &[], std::slice::from_ref(&target))
            .unwrap();
    assert_eq!(
        before.native[0].status,
        compatibility::PlanStatus::Planned,
        "{:?}",
        before.native[0].findings
    );
    let envelope = before.native[0]
        .models
        .iter()
        .find(|m| m.source.pointer == "/components/schemas/Envelope" && m.role == "model")
        .unwrap()
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(
        envelope["sourceValidation"]["profile"],
        OwnedProgram::V3_PROFILE
    );
    assert_eq!(
        envelope["sourceValidation"]["dynamicScope"]["candidateSchemas"],
        "validation-only"
    );
    assert_eq!(
        envelope["sourceValidation"]["dynamicScope"]["acquisition"],
        false
    );
    let carrier = before.native[0]
        .models
        .iter()
        .find(|m| {
            m.source.document == "http://storage.csharp.test/schema/choice.json"
                && m.source.pointer.is_empty()
                && m.role == "erased-alias"
        })
        .unwrap()
        .descriptor
        .as_ref()
        .unwrap();
    assert_eq!(carrier["sourceValidation"]["jsonCarrier"], true);
    assert_eq!(carrier["type"]["name"], "System.Text.Json.JsonElement");
    let mut changed = fixture.clone();
    changed["documents"][3]["value"]["anyOf"][0] = json!({"type":"string"});
    let after = compatibility::snapshot(sdk_contract(&changed), &[], &[target]).unwrap();
    assert_eq!(
        after.native[0].status,
        compatibility::PlanStatus::Planned,
        "{:?}",
        after.native[0].findings
    );
    let report = compatibility::compare_snapshots(&before, &after);
    assert!(
        report.native[0]
            .changes
            .iter()
            .any(|c| c.code == "native-model-shape-changed"
                && c.subject.contains("NullableEnvelope")),
        "{:?}",
        report.native[0].changes
    );
    for (name, snapshot) in [("before", before), ("after", after)] {
        write(
            &root.join(format!("{name}.json")),
            serde_json::to_vec_pretty(
                &json!({"metadata":snapshot.metadata,"native":snapshot.native}),
            )
            .unwrap(),
        );
    }
    write(
        &root.join("comparison.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    );
    write(
        &root.join("PASS.json"),
        json!({"result":"passed","gate":"canonical-v3-csharp-capture"}).to_string(),
    );
    println!("C# canonical resource capture evidence: {}", root.display());
}
