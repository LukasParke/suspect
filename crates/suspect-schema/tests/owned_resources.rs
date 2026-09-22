//! Independent canonical-resource/dynamic-scope witnesses through public v3.
use serde_json::{Value, json};
use std::sync::Arc;
use suspect_ir::contract::{Contract, SchemaId};
use suspect_low::Pointer;
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_schema::{
    Config, OwnedCompiler, OwnedOutcome, OwnedProgram, OwnedSchema, ProgramInstruction,
};
use suspect_source::Uri;

const ENTRY: &str = "https://physical.test/api.json";
fn source(uri: &str, pointer: &str) -> SchemaId {
    SchemaId::new(Uri::parse(uri).unwrap(), Pointer::parse(pointer).unwrap())
}
fn root(name: &str) -> SchemaId {
    source(ENTRY, "/components/schemas").child(name)
}
fn api(schemas: Value) -> Value {
    json!({"openapi":"3.2.0","info":{"title":"Dynamic oracle","version":"1"},"paths":{},"components":{"schemas":schemas}})
}
fn load(document: Value, external: Vec<(&str, Value)>) -> Arc<Contract> {
    let mut files = vec![(ENTRY, document)];
    files.extend(external);
    let provider = Arc::new(
        DocumentProvider::new(files.into_iter().map(|(uri, value)| {
            ProvidedDocument::new(
                Uri::parse(uri).unwrap(),
                Uri::parse(uri).unwrap(),
                value.to_string().into_bytes(),
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
fn compile(contract: Arc<Contract>, roots: &[SchemaId], config: Config) -> OwnedSchema {
    let schema = OwnedCompiler::new(config)
        .compile_v3(contract, roots)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let program = schema.program();
    program.check().unwrap();
    assert_eq!(
        (program.version, program.profile),
        (OwnedProgram::V3_VERSION, OwnedProgram::V3_PROFILE)
    );
    for node in &program.nodes {
        let id = source(&node.source.document, &node.source.pointer);
        assert!(schema.contract().schema(&id).is_some());
    }
    schema
}
fn verdict(schema: &OwnedSchema, root: &SchemaId, instance: Value, expected: bool) {
    let actual = match schema.validate(root, &instance) {
        OwnedOutcome::Valid => true,
        OwnedOutcome::Invalid(findings) => {
            assert!(!findings.is_empty());
            for finding in findings {
                assert!(schema.contract().source(&finding.source).is_some());
            }
            false
        }
        OwnedOutcome::EvaluationFailure(finding) => panic!("incomplete {instance}: {finding:?}"),
    };
    assert_eq!(actual, expected, "{instance}");
}

#[test]
fn canonical_self_and_nested_ids_keep_physical_sources_and_exact_static_refs() {
    let mut document = api(json!({
        "Root":{"$id":"models/root","$defs":{"Child":{"$id":"child#","type":"integer","minimum":9007199254740993u64}},"$ref":"child"},
        "Use":{"$ref":"https://logical.test/models/root"}
    }));
    document["$self"] = json!("https://logical.test/api#revision");
    let contract = load(document, vec![]);
    let selected = root("Use");
    let schema = compile(contract, std::slice::from_ref(&selected), Config::default());
    verdict(&schema, &selected, json!(9007199254740993u64), true);
    verdict(&schema, &selected, json!(9007199254740992u64), false);
    let program = schema.program();
    let context = program.resource_context.as_ref().unwrap();
    assert!(
        context
            .resources
            .iter()
            .any(
                |resource| resource.canonical_uri == "https://logical.test/models/child"
                    && resource.source.document == ENTRY
                    && resource.source.pointer == "/components/schemas/Root/$defs/Child"
            )
    );
    assert!(
        context
            .resources
            .iter()
            .any(
                |resource| resource.canonical_uri == "https://logical.test/api#revision"
                    && resource.base_uri == "https://logical.test/api"
            )
    );
    let OwnedOutcome::Invalid(findings) = schema.validate(&selected, &json!(0)) else {
        panic!()
    };
    assert!(findings.iter().any(
        |finding| finding.source == root("Root").child("$defs").child("Child").child("minimum")
    ));
}

fn tree_contract() -> Arc<Contract> {
    load(
        api(json!({
            "Tree":{"$id":"https://schema.test/tree","$dynamicAnchor":"node","type":"object","properties":{"data":true,"children":{"type":"array","items":{"$dynamicRef":"#node"}}}},
            "Strict":{"$id":"https://schema.test/strict","$dynamicAnchor":"node","$ref":"tree","unevaluatedProperties":false}
        })),
        vec![],
    )
}

#[test]
fn dynamic_strict_tree_propagates_scoped_annotations_through_recursive_refs() {
    let contract = tree_contract();
    let strict = root("Strict");
    let tree = root("Tree");
    let schema = compile(contract, &[strict.clone(), tree.clone()], Config::default());
    verdict(
        &schema,
        &strict,
        json!({"data":"root","children":[{"data":"child"}]}),
        true,
    );
    verdict(
        &schema,
        &strict,
        json!({"children":[{"unexpected":1}]}),
        false,
    );
    verdict(&schema, &tree, json!({"children":[{"unexpected":1}]}), true);
    let OwnedOutcome::Invalid(findings) =
        schema.validate(&strict, &json!({"children":[{"unexpected":1}]}))
    else {
        panic!()
    };
    assert!(findings.iter().any(
        |finding| finding.source == strict.child("unevaluatedProperties")
            && finding.instance_path.to_path() == "/children/0/unexpected"
    ));
}

#[test]
fn outermost_entered_resource_wins_and_unentered_candidates_do_not() {
    let contract = load(
        api(json!({
            "Base":{"$id":"urn:base","$dynamicAnchor":"node","properties":{"outer":true,"middle":true,"children":{"items":{"$dynamicRef":"#node"}}}},
            "Middle":{"$id":"urn:middle","$dynamicAnchor":"node","$ref":"urn:base","required":["middle"]},
            "Outer":{"$id":"urn:outer","$dynamicAnchor":"node","$ref":"urn:middle","required":["outer"]},
            "Unentered":{"$id":"urn:aaa","$dynamicAnchor":"node","not":{}}
        })),
        vec![],
    );
    let selected = root("Outer");
    let schema = compile(
        contract,
        &[selected.clone(), root("Unentered")],
        Config::default(),
    );
    verdict(
        &schema,
        &selected,
        json!({"outer":true,"middle":true,"children":[{"outer":true,"middle":true}]}),
        true,
    );
    verdict(
        &schema,
        &selected,
        json!({"outer":true,"middle":true,"children":[{"middle":true}]}),
        false,
    );
}

#[test]
fn plain_dynamic_anchor_is_distinct_from_pointer_empty_and_static_anchor_refs() {
    let contract = load(
        api(json!({
            "Base":{"$id":"urn:base","type":"string","$defs":{"Target":{"$dynamicAnchor":"node","$anchor":"plain","type":"string"}}},
            "Outer":{"$id":"urn:outer","$defs":{"Override":{"$dynamicAnchor":"node","type":"integer"}},"properties":{
                "dynamic":{"$dynamicRef":"urn:base#node"},"encoded":{"$dynamicRef":"urn:base#n%6fde"},
                "pointer":{"$dynamicRef":"urn:base#/$defs/Target"},"plain":{"$dynamicRef":"urn:base#plain"},
                "empty":{"$dynamicRef":"urn:base#"},"static":{"$ref":"urn:base#node"}
            }},
            "Fallback":{"$dynamicRef":"urn:base#node"}
        })),
        vec![],
    );
    let selected = root("Outer");
    let fallback = root("Fallback");
    let schema = compile(
        contract,
        &[selected.clone(), fallback.clone()],
        Config::default(),
    );
    verdict(
        &schema,
        &selected,
        json!({"dynamic":1,"encoded":2,"pointer":"s","plain":"s","empty":"s","static":"s"}),
        true,
    );
    verdict(&schema, &selected, json!({"dynamic":"s"}), false);
    for name in ["pointer", "plain", "empty", "static"] {
        verdict(&schema, &selected, json!({name:1}), false);
    }
    verdict(&schema, &fallback, json!("s"), true);
    verdict(&schema, &fallback, json!(1), false);
    let program = schema.program();
    for node in &program.nodes {
        for check in &node.checks {
            if let ProgramInstruction::DynamicRef { anchor, .. } = &check.instruction {
                let dynamic = check.source.pointer.contains("/dynamic/")
                    || check.source.pointer.contains("/encoded/")
                    || check.source.pointer.contains("/Fallback/");
                assert_eq!(anchor.is_some(), dynamic, "{}", check.source.pointer);
            }
        }
    }
}

#[test]
fn nested_entry_uses_its_indexed_parent_resource_and_unvisited_binding() {
    let outer = "https://physical.test/outer.json";
    let base = "https://physical.test/base.json";
    let extra = "https://physical.test/extra.json";
    let contract = load(
        api(json!({"Use":{"$ref":"https://physical.test/outer.json#/$defs/start"}})),
        vec![
            (
                outer,
                json!({"$id":"urn:outer","const":false,"$defs":{"binding":{"$dynamicAnchor":"node","$ref":"urn:extra"},"start":{"$ref":"urn:base#/$defs/use"}}}),
            ),
            (
                base,
                json!({"$id":"urn:base","$defs":{"node":{"$dynamicAnchor":"node","type":"string"},"use":{"$dynamicRef":"#node"}}}),
            ),
            (extra, json!({"$id":"urn:extra","type":"integer"})),
        ],
    );
    let selected = source(outer, "/$defs/start");
    let schema = compile(contract, std::slice::from_ref(&selected), Config::default());
    verdict(&schema, &selected, json!(7), true);
    verdict(&schema, &selected, json!("s"), false);
    let program = schema.program();
    assert!(
        program
            .nodes
            .iter()
            .any(|node| node.source.document == outer && node.source.pointer == "/$defs/binding")
    );
    assert!(
        !program
            .nodes
            .iter()
            .any(|node| node.source.document == outer && node.source.pointer.is_empty()),
        "entering the resource must not evaluate its unselected root const"
    );
    let OwnedOutcome::Invalid(findings) = schema.validate(&selected, &json!("s")) else {
        panic!()
    };
    assert!(
        findings
            .iter()
            .any(|finding| finding.source == source(extra, "/type"))
    );
}

#[test]
fn scopes_restore_after_failed_trials_and_changed_context_is_not_a_false_cycle() {
    let contract = load(
        api(json!({
            "Fallback":{"$id":"urn:fallback","$dynamicAnchor":"flag","not":{}},
            "Outer":{"$id":"urn:outer","if":{"$dynamicRef":"urn:fallback#flag"},"then":true,"else":{"$ref":"urn:new-context"}},
            "NewContext":{"$id":"urn:new-context","$defs":{"flag":{"$dynamicAnchor":"flag"}},"$ref":"urn:outer"},
            "Plain":{"$id":"urn:plain","$dynamicAnchor":"node","type":"string"},
            "Failed":{"$id":"urn:failed","$dynamicAnchor":"node","not":{}},
            "Trial":{"anyOf":[{"$ref":"urn:failed"},{"$dynamicRef":"urn:plain#node"}]}
        })),
        vec![],
    );
    let outer = root("Outer");
    let trial = root("Trial");
    let schema = compile(contract, &[outer.clone(), trial.clone()], Config::default());
    verdict(&schema, &outer, json!(7), true);
    verdict(&schema, &trial, json!("s"), true);
    verdict(&schema, &trial, json!(7), false);
}

#[test]
fn v1_v2_refusals_and_wire_programs_remain_frozen() {
    let contract = tree_contract();
    let roots = [root("Strict")];
    let compiler = OwnedCompiler::new(Config::default());
    assert!(compiler.compile(contract.clone(), &roots).is_err());
    assert!(compiler.compile_v2(contract.clone(), &roots).is_err());
    let program = compile(contract, &roots, Config::default()).program();
    for (version, profile) in [
        (OwnedProgram::V1_VERSION, OwnedProgram::V1_PROFILE),
        (OwnedProgram::V2_VERSION, OwnedProgram::V2_PROFILE),
    ] {
        let mut changed = program.clone();
        changed.version = version;
        changed.profile = profile;
        assert!(changed.check().is_err());
        changed.resource_context = None;
        assert!(
            changed.check().is_err(),
            "a dynamic instruction cannot enter an old profile"
        );
    }
    let basic = load(api(json!({"Root":{"type":"integer"}})), vec![]);
    let roots = [root("Root")];
    let v1 = compiler.compile(basic.clone(), &roots).unwrap().program();
    let v2 = compiler.compile_v2(basic, &roots).unwrap().program();
    assert_eq!(
        serde_json::to_string(&v1).unwrap(),
        serde_json::to_string(&v2).unwrap()
    );
    assert!(
        !serde_json::to_string(&v1)
            .unwrap()
            .contains("resourceContext")
    );
}

#[test]
fn checked_v3_rejects_mutated_resource_scope_alias_and_binding_metadata() {
    let selected = root("Strict");
    let original = compile(tree_contract(), &[selected], Config::default()).program();
    for change in 0..7 {
        let mut program = original.clone();
        let context = program.resource_context.as_mut().unwrap();
        match change {
            0 => {
                context.node_scopes.pop();
            }
            1 => {
                context.node_scopes[0].0 = usize::MAX;
            }
            2 => {
                context.node_scopes[0].2 = "urn:wrong".into();
            }
            3 => {
                context.resources[0].aliases.clear();
            }
            4 => {
                let alias = context.resources[0].canonical_uri.clone();
                context.resources[1].aliases.push(alias);
            }
            5 => {
                let resource = context
                    .resources
                    .iter_mut()
                    .find(|resource| !resource.dynamic_anchors.is_empty())
                    .unwrap();
                resource.dynamic_anchors[0].2 = usize::MAX;
            }
            _ => {
                let check = program
                    .nodes
                    .iter_mut()
                    .flat_map(|node| &mut node.checks)
                    .find(|check| {
                        matches!(check.instruction, ProgramInstruction::DynamicRef { .. })
                    })
                    .unwrap();
                if let ProgramInstruction::DynamicRef {
                    initial_resource, ..
                } = &mut check.instruction
                {
                    *initial_resource = usize::MAX;
                }
            }
        }
        assert!(program.check().is_err(), "mutation {change}");
    }
}

#[test]
fn dynamic_cycles_and_work_failures_are_noninvertible() {
    let contract = load(
        api(
            json!({"Root":{"$id":"urn:cycle","$dynamicAnchor":"node","anyOf":[true,{"$dynamicRef":"#node"}]}}),
        ),
        vec![],
    );
    let selected = root("Root");
    let schema = compile(contract, std::slice::from_ref(&selected), Config::default());
    assert!(matches!(
        schema.validate(&selected, &Value::Null),
        OwnedOutcome::EvaluationFailure(_)
    ));
    let contract = tree_contract();
    let selected = root("Strict");
    let schema = compile(
        contract,
        std::slice::from_ref(&selected),
        Config {
            max_evaluation_steps: 4,
            ..Config::default()
        },
    );
    assert!(matches!(
        schema.validate(&selected, &json!({"children":[{}]})),
        OwnedOutcome::EvaluationFailure(_)
    ));
}

#[test]
fn retrieval_aliases_and_escaped_canonical_pointers_keep_original_identity() {
    let requested = "https://requested.test/model.json";
    let physical = "https://cdn.test/effective/model.json";
    let fragment = "#/$defs/a~1b~0%25%20%23%C3%A9";
    let document = api(json!({
        "Logical":{"$ref":format!("urn:escaped{fragment}")},
        "Alias":{"$ref":format!("{requested}{fragment}")},
        "Physical":{"$ref":format!("{physical}{fragment}")}
    }));
    let dependency =
        json!({"$id":"urn:escaped","$defs":{"a/b~% #é":{"type":"integer","maximum":0}}});
    let provider = Arc::new(
        DocumentProvider::new([
            ProvidedDocument::new(
                Uri::parse(ENTRY).unwrap(),
                Uri::parse(ENTRY).unwrap(),
                document.to_string().into_bytes(),
            )
            .unwrap(),
            ProvidedDocument::new(
                Uri::parse(requested).unwrap(),
                Uri::parse(physical).unwrap(),
                dependency.to_string().into_bytes(),
            )
            .unwrap(),
        ])
        .unwrap(),
    );
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .allowed_documents(provider.logical_uris())
            .document_provider(provider)
            .build()
            .unwrap(),
    );
    let contract =
        Arc::new(Contract::from_workspace(&workspace, &Uri::parse(ENTRY).unwrap()).unwrap());
    let roots = [root("Logical"), root("Alias"), root("Physical")];
    let schema = compile(contract, &roots, Config::default());
    for root in &roots {
        verdict(&schema, root, json!(-1), true);
        let OwnedOutcome::Invalid(findings) = schema.validate(root, &json!(1)) else {
            panic!()
        };
        assert!(
            findings
                .iter()
                .any(|finding| finding.source == source(physical, "/$defs/a~1b~0% #é/maximum"))
        );
    }
    let program = schema.program();
    let context = program.resource_context.unwrap();
    let resource = context
        .resources
        .iter()
        .find(|resource| resource.canonical_uri == "urn:escaped")
        .unwrap();
    assert_eq!(resource.source.document, physical);
    assert!(resource.aliases.contains(&requested.to_owned()));
    assert!(resource.aliases.contains(&physical.to_owned()));
    assert!(
        context
            .node_scopes
            .iter()
            .any(|(_, _, address)| address == &format!("urn:escaped{fragment}"))
    );
}

#[test]
fn duplicate_logical_identities_and_invalid_resources_never_compile_as_v3() {
    for external in [
        vec![
            (
                "https://physical.test/a.json",
                json!({"$id":"urn:duplicate","type":"string"}),
            ),
            (
                "https://physical.test/b.json",
                json!({"$id":"urn:duplicate","type":"string"}),
            ),
        ],
        vec![
            (
                "https://physical.test/a.json",
                json!({"$id":"urn:duplicate","type":"string"}),
            ),
            (
                "https://physical.test/b.json",
                json!({"$id":"urn:duplicate","type":"integer"}),
            ),
        ],
    ] {
        let contract = load(api(json!({"Use":{"$ref":"urn:duplicate"}})), external);
        let errors = OwnedCompiler::new(Config::default())
            .compile_v3(contract.clone(), &[root("Use")])
            .err()
            .expect("duplicate URI cannot select the first resource");
        assert!(
            errors
                .iter()
                .any(|error| error.kind == suspect_schema::OwnedCompileErrorKind::Invalid)
        );
        assert!(
            errors
                .iter()
                .all(|error| contract.source(&error.source).is_some())
        );
    }
    for schema in [
        json!({"$id":"urn:bad#not-empty"}),
        json!({"$id":"urn:root","$defs":{"A":{"$anchor":"same"},"B":{"$dynamicAnchor":"same"}}}),
        json!({"$id":"relative bad"}),
    ] {
        let contract = load(api(json!({"Root":schema})), vec![]);
        assert!(
            OwnedCompiler::new(Config::default())
                .compile_v3(contract, &[root("Root")])
                .is_err()
        );
    }
}

#[test]
fn dynamic_evaluation_failure_does_not_select_else_or_pass_anyof() {
    for use_site in [
        json!({"if":{"$dynamicRef":"urn:number#node"},"then":true,"else":true}),
        json!({"anyOf":[true,{"$dynamicRef":"urn:number#node"}]}),
        json!({"not":{"$dynamicRef":"urn:number#node"}}),
    ] {
        let contract = load(
            api(
                json!({"Use":use_site,"Number":{"$id":"urn:number","$dynamicAnchor":"node","maximum":0}}),
            ),
            vec![],
        );
        let selected = root("Use");
        let schema = compile(
            contract,
            std::slice::from_ref(&selected),
            Config {
                max_number_bytes: 3,
                max_errors: 1,
                ..Config::default()
            },
        );
        let OwnedOutcome::EvaluationFailure(finding) = schema.validate(&selected, &json!(12345))
        else {
            panic!("failure must remain noninvertible")
        };
        assert_eq!(finding.source, root("Number").child("maximum"));
    }
}

#[test]
fn formerly_deferred_official_dynamic_unevaluated_cases_execute() {
    let mut count = 0;
    for text in [
        include_str!("conformance/draft2020-12/unevaluatedProperties.json"),
        include_str!("conformance/draft2020-12/unevaluatedItems.json"),
    ] {
        let groups: Vec<Value> = serde_json::from_str(text).unwrap();
        for group in groups {
            if !matches!(
                group["description"].as_str(),
                Some(
                    "unevaluatedProperties with $dynamicRef" | "unevaluatedItems with $dynamicRef"
                )
            ) {
                continue;
            }
            let document = "https://physical.test/official-schema.json";
            let contract = load(
                api(json!({"Use":{"$ref":document}})),
                vec![(document, group["schema"].clone())],
            );
            let selected = source(document, "");
            let schema = compile(contract, std::slice::from_ref(&selected), Config::default());
            for case in group["tests"].as_array().unwrap() {
                verdict(
                    &schema,
                    &selected,
                    case["data"].clone(),
                    case["valid"].as_bool().unwrap(),
                );
                count += 1;
            }
        }
    }
    assert_eq!(count, 4);
}

#[test]
fn official_dynamic_ref_file_executes_all_original_cases_without_acquisition() {
    let groups: Vec<Value> = serde_json::from_str(include_str!(
        "fixtures/resource-conformance/dynamicRef.json"
    ))
    .unwrap();
    let mut count = 0;
    let mut executable = Vec::new();
    for group in groups {
        let document = "https://physical.test/official-schema.json";
        let contract = load(
            api(json!({"Use":{"$ref":document}})),
            vec![
                (document, group["schema"].clone()),
                (
                    "http://localhost:1234/draft2020-12/tree.json",
                    serde_json::from_str(include_str!("fixtures/resource-conformance/tree.json"))
                        .unwrap(),
                ),
                (
                    "http://localhost:1234/draft2020-12/extendible-dynamic-ref.json",
                    serde_json::from_str(include_str!(
                        "fixtures/resource-conformance/extendible-dynamic-ref.json"
                    ))
                    .unwrap(),
                ),
                (
                    "http://localhost:1234/draft2020-12/detached-dynamicref.json",
                    serde_json::from_str(include_str!(
                        "fixtures/resource-conformance/detached-dynamicref.json"
                    ))
                    .unwrap(),
                ),
            ],
        );
        let selected = source(document, "");
        let schema = OwnedCompiler::new(Config::default())
            .compile_v3(contract, std::slice::from_ref(&selected))
            .unwrap_or_else(|errors| panic!("{}: {errors:#?}", group["description"]));
        schema.program().check().unwrap();
        for case in group["tests"].as_array().unwrap() {
            let actual = match schema.validate(&selected, &case["data"]) {
                OwnedOutcome::Valid => true,
                OwnedOutcome::Invalid(_) => false,
                OwnedOutcome::EvaluationFailure(finding) => panic!(
                    "{}/{}: {finding:?}",
                    group["description"], case["description"]
                ),
            };
            assert_eq!(
                actual,
                case["valid"].as_bool().unwrap(),
                "{}/{}",
                group["description"],
                case["description"]
            );
            count += 1;
            let program = schema.program();
            let root_target = program.roots[0].target;
            executable.push(json!({
                "id":format!("{} / {}",group["description"].as_str().unwrap(),case["description"].as_str().unwrap()),
                "program":program,"rootTarget":root_target,
                "instanceJson":case["data"].to_string(),
                "expected":if case["valid"].as_bool().unwrap() {"Valid"} else {"Invalid"}
            }));
        }
    }
    assert_eq!(count, 44);
    if let Some(path) = std::env::var_os("SUSPECT_RESOURCES_EXPORT") {
        std::fs::write(path,serde_json::to_vec_pretty(&json!({"format":"suspect.schema.resources.executable-fixtures.v3","cases":executable})).unwrap()).unwrap();
    }
    eprintln!("all 44 official dynamicRef cases passed with closed supplied documents");
}

#[test]
fn resource_depth_ceiling_is_an_explicit_failure_on_a_normal_stack() {
    let mut schemas = serde_json::Map::new();
    for index in 0..550 {
        schemas.insert(
            format!("N{index}"),
            json!({"$id":format!("urn:n{index}"),"$ref":format!("urn:n{}",index+1)}),
        );
    }
    schemas.insert("N550".into(), json!({"$id":"urn:n550"}));
    let contract = load(api(Value::Object(schemas)), vec![]);
    let selected = root("N0");
    let schema = compile(contract, std::slice::from_ref(&selected), Config::default());
    let result = std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || schema.validate(&selected, &Value::Null))
        .unwrap()
        .join()
        .unwrap();
    let OwnedOutcome::EvaluationFailure(finding) = result else {
        panic!("expected configured depth failure")
    };
    assert!(finding.message.contains("depth"));
}

#[test]
#[ignore = "requires the immutable published 32-case v2 executable checkpoint"]
fn published_v2_program_bytes_are_unchanged_with_the_v3_api_present() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    let frozen: Value = serde_json::from_slice(
        &std::fs::read(directory.join("sdk-schema-applicators-executable-v2.json")).unwrap(),
    )
    .unwrap();
    let fixtures: Value =
        serde_json::from_str(include_str!("fixtures/owned-applicators-v2.json")).unwrap();
    let mut count = 0;
    for entry in frozen["cases"].as_array().unwrap() {
        let original = &entry["program"];
        let fixture = fixtures["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["id"] == entry["id"])
            .unwrap();
        let schema: Value = serde_json::from_str(fixture["schemaJson"].as_str().unwrap()).unwrap();
        let uri = original["roots"][0]["source"]["document"].as_str().unwrap();
        let pointer = original["roots"][0]["source"]["pointer"].as_str().unwrap();
        let document = json!({"openapi":"3.1.2","info":{"title":"Applicator oracle","version":"1"},"paths":{},"components":{"schemas":{"Root":schema}}});
        let provider = Arc::new(
            DocumentProvider::new([ProvidedDocument::new(
                Uri::parse(uri).unwrap(),
                Uri::parse(uri).unwrap(),
                document.to_string().into_bytes(),
            )
            .unwrap()])
            .unwrap(),
        );
        let workspace = Arc::new(
            WorkspaceBuilder::new()
                .allowed_documents(provider.logical_uris())
                .document_provider(provider)
                .build()
                .unwrap(),
        );
        let contract =
            Arc::new(Contract::from_workspace(&workspace, &Uri::parse(uri).unwrap()).unwrap());
        let limits = &original["limits"];
        let config = Config {
            max_depth: limits["maxDepth"].as_u64().unwrap() as usize,
            max_errors: limits["maxErrors"].as_u64().unwrap() as usize,
            max_number_bytes: limits["maxNumberBytes"].as_u64().unwrap() as usize,
            max_equality_steps: limits["maxEqualitySteps"].as_u64().unwrap() as usize,
            max_evaluation_steps: limits["maxEvaluationSteps"].as_u64().unwrap() as usize,
            ..Config::default()
        };
        let program = OwnedCompiler::new(config)
            .compile_v2(contract, &[source(uri, pointer)])
            .unwrap()
            .program();
        program.check().unwrap();
        assert_eq!(
            serde_json::to_vec(&serde_json::to_value(program).unwrap()).unwrap(),
            serde_json::to_vec(original).unwrap(),
            "{}",
            entry["id"]
        );
        count += 1;
    }
    assert_eq!(count, 32);
}
