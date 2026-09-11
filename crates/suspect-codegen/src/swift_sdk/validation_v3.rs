//! Maintained source -> real compile_v3 -> native execution. Official expected
//! outcomes and independent controls are literals, never Rust-evaluator results.
use super::*;
use crate::swift_sdk::validation_v3_support as support;
use serde_json::json;
use std::sync::Arc;
use support::{api, load, schema, source as id};
use suspect_schema::{Config, OwnedCompiler};

struct Case {
    name: String,
    root: SchemaId,
    instance: String,
    expected: &'static str,
    location: Option<(SchemaId, String)>,
}
impl Case {
    fn new(name: &str, root: SchemaId, instance: &str, expected: &'static str) -> Self {
        Self {
            name: name.into(),
            root,
            instance: instance.into(),
            expected,
            location: None,
        }
    }
    fn at(mut self, source: SchemaId, path: &str) -> Self {
        self.location = Some((source, path.into()));
        self
    }
}
struct Group {
    name: String,
    contract: Arc<Contract>,
    config: Config,
    roots: Vec<SchemaId>,
    cases: Vec<Case>,
}
impl Group {
    fn new(name: &str, contract: Arc<Contract>, cases: Vec<Case>) -> Self {
        let roots = cases.iter().map(|c| c.root.clone()).collect();
        Self {
            name: name.into(),
            contract,
            config: Config::default(),
            roots,
            cases,
        }
    }
    fn program(&self) -> OwnedProgram {
        let program = OwnedCompiler::new(self.config.clone())
            .compile_v3(self.contract.clone(), &self.roots)
            .unwrap_or_else(|errors| panic!("{}: {errors:#?}", self.name))
            .program();
        program.check().unwrap();
        assert_eq!(
            (program.version, program.profile),
            (OwnedProgram::V3_VERSION, OwnedProgram::V3_PROFILE)
        );
        program
    }
}

fn official() -> Vec<Group> {
    const DOCUMENT: &str = "https://swift-resources.test/official-schema.json";
    let groups: Vec<Value> = serde_json::from_str(include_str!(
        "../../../suspect-schema/tests/fixtures/resource-conformance/dynamicRef.json"
    ))
    .unwrap();
    let mut result = Vec::new();
    let mut count = 0;
    for group in groups {
        // Extract the original schema unchanged. In particular no $id/$ref is
        // rewritten to an OpenAPI component pointer or a native fixture alias.
        let entry = api(json!({"Use":{"$ref":DOCUMENT}}));
        let contract = support::provided(support::ENTRY, vec![
            (support::ENTRY, support::ENTRY, entry.to_string().into_bytes()),
            (DOCUMENT, DOCUMENT, group["schema"].to_string().into_bytes()),
            ("http://localhost:1234/draft2020-12/tree.json", "http://localhost:1234/draft2020-12/tree.json", include_bytes!("../../../suspect-schema/tests/fixtures/resource-conformance/tree.json").to_vec()),
            ("http://localhost:1234/draft2020-12/extendible-dynamic-ref.json", "http://localhost:1234/draft2020-12/extendible-dynamic-ref.json", include_bytes!("../../../suspect-schema/tests/fixtures/resource-conformance/extendible-dynamic-ref.json").to_vec()),
            ("http://localhost:1234/draft2020-12/detached-dynamicref.json", "http://localhost:1234/draft2020-12/detached-dynamicref.json", include_bytes!("../../../suspect-schema/tests/fixtures/resource-conformance/detached-dynamicref.json").to_vec()),
        ]);
        let cases = group["tests"]
            .as_array()
            .unwrap()
            .iter()
            .map(|case| {
                count += 1;
                Case::new(
                    case["description"].as_str().unwrap(),
                    id(DOCUMENT, ""),
                    &case["data"].to_string(),
                    if case["valid"].as_bool().unwrap() {
                        "Valid"
                    } else {
                        "Invalid"
                    },
                )
            })
            .collect();
        result.push(Group::new(
            group["description"].as_str().unwrap(),
            contract,
            cases,
        ));
    }
    assert_eq!(count, 44);
    result
}

fn deferred() -> Vec<Group> {
    let mut result = Vec::new();
    for text in [
        include_str!(
            "../../../suspect-schema/tests/conformance/draft2020-12/unevaluatedProperties.json"
        ),
        include_str!(
            "../../../suspect-schema/tests/conformance/draft2020-12/unevaluatedItems.json"
        ),
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
            let document = "https://swift-resources.test/unevaluated.json";
            let contract = load(
                api(json!({"Use":{"$ref":document}})),
                vec![(document, group["schema"].clone())],
            );
            let cases = group["tests"]
                .as_array()
                .unwrap()
                .iter()
                .map(|case| {
                    Case::new(
                        case["description"].as_str().unwrap(),
                        id(document, ""),
                        &case["data"].to_string(),
                        if case["valid"].as_bool().unwrap() {
                            "Valid"
                        } else {
                            "Invalid"
                        },
                    )
                })
                .collect();
            result.push(Group::new(
                group["description"].as_str().unwrap(),
                contract,
                cases,
            ));
        }
    }
    assert_eq!(result.iter().map(|g| g.cases.len()).sum::<usize>(), 4);
    result
}

fn controls() -> Vec<Group> {
    let contract = load(
        api(json!({
            "Tree":{"$id":"urn:tree","$dynamicAnchor":"node","type":"object","properties":{"data":true,"children":{"type":"array","items":{"$dynamicRef":"#node"}}}},
            "Strict":{"$id":"urn:strict","$dynamicAnchor":"node","$ref":"urn:tree","unevaluatedProperties":false},
            "Middle":{"$id":"urn:middle","$dynamicAnchor":"node","$ref":"urn:tree","required":["middle"]},
            "Outer":{"$id":"urn:outer","$dynamicAnchor":"node","$ref":"urn:middle","required":["outer"]},
            "Unentered":{"$id":"urn:aaa-unentered","$dynamicAnchor":"node","not":{}},
            "String":{"$id":"urn:string","type":"string","$defs":{"Target":{"$dynamicAnchor":"value","$anchor":"plain","type":"string"}}},
            "Numbers":{"$id":"urn:numbers","$defs":{"Override":{"$dynamicAnchor":"value","type":["integer","null"]}},"properties":{
                "dynamic":{"$dynamicRef":"urn:string#value"},"encoded":{"$dynamicRef":"urn:string#v%61lue"},
                "pointer":{"$dynamicRef":"urn:string#/$defs/Target"},"plain":{"$dynamicRef":"urn:string#plain"},
                "empty":{"$dynamicRef":"urn:string#"},"static":{"$ref":"urn:string#value"}}},
            "Fallback":{"$dynamicRef":"urn:string#value"},
            "Failed":{"$id":"urn:failed","$defs":{"Override":{"$dynamicAnchor":"value","not":{}}},"not":{}},
            "Passed":{"$id":"urn:passed","$defs":{"Override":{"$dynamicAnchor":"value","not":{}}},"type":"string"},
            "Trial":{"anyOf":[{"$ref":"urn:failed"},{"$dynamicRef":"urn:string#value"}]},
            "Conditional":{"if":{"$ref":"urn:passed"},"then":{"$dynamicRef":"urn:string#value"},"else":false},
            "FalseFlag":{"$id":"urn:false-flag","$dynamicAnchor":"flag","not":{}},
            "Changed":{"$id":"urn:changed","if":{"$dynamicRef":"urn:false-flag#flag"},"then":true,"else":{"$ref":"urn:new-context"}},
            "NewContext":{"$id":"urn:new-context","$defs":{"Flag":{"$dynamicAnchor":"flag"}},"$ref":"urn:changed"},
            "Fresh":{"$id":"urn:fresh","properties":{"a":true},"$dynamicRef":"urn:target#fresh"},
            "Target":{"$id":"urn:target","$dynamicAnchor":"fresh","unevaluatedProperties":false},
            "Cycle":{"$id":"urn:cycle","$dynamicAnchor":"cycle","$dynamicRef":"#cycle"},
            "AnyCycle":{"anyOf":[true,{"$ref":"urn:cycle"}]},
            "NotCycle":{"not":{"$ref":"urn:cycle"}},
            "IfCycle":{"if":{"$ref":"urn:cycle"},"then":true,"else":true}
        })),
        vec![],
    );
    let mut cases = vec![
        Case::new(
            "strict tree annotations",
            schema("Strict"),
            r#"{"children":[{"data":9007199254740993.000}]}"#,
            "Valid",
        ),
        Case::new(
            "strict recursive child rejects extra",
            schema("Strict"),
            r#"{"children":[{"unexpected":1}]}"#,
            "Invalid",
        )
        .at(
            schema("Strict").child("unevaluatedProperties"),
            "/children/0/unexpected",
        ),
        Case::new(
            "base tree remains open",
            schema("Tree"),
            r#"{"children":[{"unexpected":1}]}"#,
            "Valid",
        ),
        Case::new(
            "outermost wins; catalogue candidate inert",
            schema("Outer"),
            r#"{"outer":true,"middle":true,"children":[{"outer":true,"middle":true}]}"#,
            "Valid",
        ),
        Case::new(
            "middle cannot replace outer",
            schema("Outer"),
            r#"{"outer":true,"middle":true,"children":[{"middle":true}]}"#,
            "Invalid",
        )
        .at(schema("Outer").child("required"), "/children/0/outer"),
        Case::new(
            "unentered assertion is genuinely invalid",
            schema("Unentered"),
            "null",
            "Invalid",
        ),
        Case::new(
            "all fallback modes",
            schema("Numbers"),
            r#"{"dynamic":1,"encoded":2,"pointer":"s","plain":"s","empty":"s","static":"s"}"#,
            "Valid",
        ),
        Case::new(
            "dynamic may change nullability",
            schema("Numbers"),
            r#"{"dynamic":null}"#,
            "Valid",
        ),
        Case::new(
            "dynamic is not static fallback",
            schema("Numbers"),
            r#"{"dynamic":"s"}"#,
            "Invalid",
        )
        .at(
            schema("Numbers")
                .child("$defs")
                .child("Override")
                .child("type"),
            "/dynamic",
        ),
        Case::new(
            "fallback without entered override",
            schema("Fallback"),
            r#""s""#,
            "Valid",
        ),
        Case::new("fallback invalid", schema("Fallback"), "1", "Invalid").at(
            schema("String")
                .child("$defs")
                .child("Target")
                .child("type"),
            "",
        ),
        Case::new(
            "failed anyOf scope is restored",
            schema("Trial"),
            r#""s""#,
            "Valid",
        ),
        Case::new(
            "successful if scope is restored",
            schema("Conditional"),
            r#""s""#,
            "Valid",
        ),
        Case::new(
            "changed exact context is not a false cycle",
            schema("Changed"),
            "7",
            "Valid",
        ),
        Case::new(
            "dynamic target starts with fresh annotations",
            schema("Fresh"),
            r#"{"a":1}"#,
            "Invalid",
        )
        .at(schema("Target").child("unevaluatedProperties"), "/a"),
    ];
    for field in ["pointer", "plain", "empty", "static"] {
        cases.push(Case::new(
            &format!("{field} never overrides"),
            schema("Numbers"),
            &json!({field:1}).to_string(),
            "Invalid",
        ));
    }
    for name in ["Cycle", "AnyCycle", "NotCycle", "IfCycle"] {
        cases.push(
            Case::new(
                &format!("noninvertible {name}"),
                schema(name),
                "null",
                "EvaluationFailure",
            )
            .at(schema("Cycle"), ""),
        );
    }
    let mut result = vec![Group::new("scope-and-cycle-controls", contract, cases)];

    let outer = "https://swift-resources.test/outer.json";
    let base = "https://swift-resources.test/base.json";
    let extra = "https://swift-resources.test/extra.json";
    let contract = load(
        api(json!({"Use":{"$ref":format!("{outer}#/$defs/start")}})),
        vec![
            (
                outer,
                json!({"$id":"urn:parent","const":false,"$defs":{"Binding":{"$dynamicAnchor":"value","$ref":"urn:extra"},"start":{"$ref":"urn:base#/$defs/use"}}}),
            ),
            (
                base,
                json!({"$id":"urn:base","$defs":{"Value":{"$dynamicAnchor":"value","type":"string"},"use":{"$dynamicRef":"#value"}}}),
            ),
            (extra, json!({"$id":"urn:extra","type":"integer"})),
        ],
    );
    result.push(Group::new(
        "nested-entry-without-resource-root",
        contract,
        vec![
            Case::new(
                "indexed enclosing resource enters at nested root",
                id(outer, "/$defs/start"),
                "7",
                "Valid",
            ),
            Case::new(
                "detached binding keeps physical source",
                id(outer, "/$defs/start"),
                r#""s""#,
                "Invalid",
            )
            .at(id(extra, "/type"), ""),
        ],
    ));

    let requested = "https://download.swift.test/model.json";
    let physical = "https://cdn.swift.test/release/model.json";
    let fragment = "#/$defs/a~1b~0%25%20%23%C3%A9";
    let mut entry = api(json!({
        "Logical":{"$ref":format!("urn:escaped{fragment}")},
        "Requested":{"$ref":format!("{requested}{fragment}")},
        "Physical":{"$ref":format!("{physical}{fragment}")}
    }));
    entry["$self"] = json!("https://logical.swift.test/api#revision");
    let contract = support::provided(support::ENTRY, vec![
        (support::ENTRY, support::ENTRY, entry.to_string().into_bytes()),
        (requested, physical, json!({"$id":"urn:escaped","$defs":{"a/b~% #é":{"type":"integer","minimum":9007199254740993u64}}}).to_string().into_bytes()),
    ]);
    let mut cases = Vec::new();
    for name in ["Logical", "Requested", "Physical"] {
        cases.push(Case::new(
            &format!("{name} exact integer"),
            schema(name),
            "9007199254740993.000",
            "Valid",
        ));
        cases.push(
            Case::new(
                &format!("{name} original pointer"),
                schema(name),
                "9007199254740992",
                "Invalid",
            )
            .at(id(physical, "/$defs/a~1b~0% #é/minimum"), ""),
        );
    }
    result.push(Group::new("logical-physical-aliases", contract, cases));

    let numbers = load(
        api(json!({
            "Number":{"$id":"urn:number","$dynamicAnchor":"value","maximum":0},
            "Any":{"anyOf":[true,{"$dynamicRef":"urn:number#value"}]},
            "Not":{"not":{"$dynamicRef":"urn:number#value"}},
            "If":{"if":{"$dynamicRef":"urn:number#value"},"then":true,"else":true}
        })),
        vec![],
    );
    let mut group = Group::new(
        "numeric-failure-survives-dynamic-trials",
        numbers,
        ["Any", "Not", "If"]
            .into_iter()
            .map(|name| {
                Case::new(name, schema(name), "12345", "EvaluationFailure")
                    .at(schema("Number").child("maximum"), "")
            })
            .collect(),
    );
    group.config.max_number_bytes = 3;
    group.config.max_errors = 1;
    result.push(group);

    let budgets = load(
        api(json!({
            "Empty":{"$id":"urn:empty"},
            "Same":{"$id":"urn:same","$ref":"#/$defs/End","$defs":{"End":{}}},
            "Distinct":{"$id":"urn:distinct","$ref":"urn:end"},"End":{"$id":"urn:end"},
            "Lookup":{"$id":"urn:lookup","$dynamicRef":"urn:fallback#pick","$defs":{
                "A":{"$dynamicAnchor":"a"},"Pick":{"$dynamicAnchor":"pick"},"Z":{"$dynamicAnchor":"z"}}},
            "LookupFallback":{"$id":"urn:fallback","$dynamicAnchor":"pick","const":false}
        })),
        vec![],
    );
    for (name, steps, expected, at) in [
        ("Empty", 1, "EvaluationFailure", schema("Empty")),
        ("Empty", 2, "Valid", schema("Empty")),
        (
            "Same",
            3,
            "EvaluationFailure",
            schema("Same").child("$defs").child("End"),
        ),
        ("Same", 4, "Valid", schema("Same")),
        ("Distinct", 4, "EvaluationFailure", schema("End")),
        ("Distinct", 5, "Valid", schema("Distinct")),
        (
            "Lookup",
            5,
            "EvaluationFailure",
            schema("Lookup").child("$dynamicRef"),
        ),
        (
            "Lookup",
            6,
            "EvaluationFailure",
            schema("Lookup").child("$defs").child("Pick"),
        ),
        ("Lookup", 7, "Valid", schema("Lookup")),
    ] {
        let case = Case::new(
            &format!("{name} exactly {steps} visits"),
            schema(name),
            "null",
            expected,
        )
        .at(at, "");
        let mut group = Group::new(
            &format!("budget-{name}-{steps}"),
            budgets.clone(),
            vec![case],
        );
        // Include nonmatching bindings in the catalogue without entering them.
        group.roots.extend([
            schema("Lookup").child("$defs").child("A"),
            schema("Lookup").child("$defs").child("Z"),
        ]);
        group.config.max_evaluation_steps = steps;
        result.push(group);
    }
    let mut schemas = serde_json::Map::new();
    for index in 0..550 {
        schemas.insert(
            format!("N{index}"),
            json!({"$id":format!("urn:depth:{index}"),"$ref":format!("urn:depth:{}",index+1)}),
        );
    }
    schemas.insert("N550".into(), json!({"$id":"urn:depth:550"}));
    result.push(Group::new(
        "normal-stack-depth",
        load(api(Value::Object(schemas)), vec![]),
        vec![
            Case::new(
                "512 depth ceiling on a 2 MiB stack",
                schema("N0"),
                "null",
                "EvaluationFailure",
            )
            .at(schema("N512"), ""),
        ],
    ));
    result
}

#[test]
fn checked_resource_emission_rejects_corrupt_envelopes_scopes_and_bindings() {
    let group = controls().remove(0);
    let original = group.program();
    assert!(emit(&group.contract, &original).is_ok());
    for mutation in 0..12 {
        let mut program = original.clone();
        match mutation {
            0 => {
                program.version = OwnedProgram::V1_VERSION;
                program.profile = OwnedProgram::V1_PROFILE;
            }
            1 => {
                program.version = OwnedProgram::V2_VERSION;
                program.profile = OwnedProgram::V2_PROFILE;
            }
            2 => program.resource_context = None,
            3 => {
                program.resource_context.as_mut().unwrap().node_scopes.pop();
            }
            4 => program.resource_context.as_mut().unwrap().node_scopes[0].0 = usize::MAX,
            5 => program.resource_context.as_mut().unwrap().node_scopes[0].2 = "urn:wrong".into(),
            6 => program.resource_context.as_mut().unwrap().resources[0]
                .aliases
                .clear(),
            7 => {
                let graph = program.resource_context.as_mut().unwrap();
                let alias = graph.resources[0].canonical_uri.clone();
                graph.resources[1].aliases.push(alias);
            }
            8 => {
                program
                    .resource_context
                    .as_mut()
                    .unwrap()
                    .resources
                    .iter_mut()
                    .find(|r| !r.dynamic_anchors.is_empty())
                    .unwrap()
                    .dynamic_anchors[0]
                    .2 = usize::MAX
            }
            9 => program
                .resource_context
                .as_mut()
                .unwrap()
                .resources
                .iter_mut()
                .find(|r| !r.dynamic_anchors.is_empty())
                .unwrap()
                .dynamic_anchors[0]
                .1
                .pointer
                .push_str("/not-an-anchor"),
            10 => {
                let check = program
                    .nodes
                    .iter_mut()
                    .flat_map(|n| &mut n.checks)
                    .find(|c| matches!(c.instruction, ProgramInstruction::DynamicRef { .. }))
                    .unwrap();
                if let ProgramInstruction::DynamicRef {
                    initial_resource, ..
                } = &mut check.instruction
                {
                    *initial_resource = usize::MAX;
                }
            }
            _ => {
                program.version = "suspect.validation.experimental.v99";
            }
        }
        let errors =
            emit(&group.contract, &program).expect_err("no artifact from a corrupt resource graph");
        assert_eq!(
            errors[0].code,
            if mutation == 11 {
                "swift-validation-profile-unsupported"
            } else {
                "swift-validation-program-invalid"
            },
            "mutation {mutation}: {errors:?}"
        );
        assert_eq!(errors[0].source.document().as_str(), support::ENTRY);
        // Mutating a source itself cannot manufacture a real span; every other
        // control must retain an original source location and nonempty span.
        if mutation != 9 {
            assert!(
                errors[0].at.end > errors[0].at.start,
                "mutation {mutation}: {errors:?}"
            );
        }
    }
}

#[test]
#[ignore = "Swift v3 official source fixtures, dynamic-scope controls and normal-stack budgets on selected installed toolchain"]
fn native_resource_dynamic_source_vectors() {
    let root = support::root("runtime-v3-");
    let mut groups = official();
    groups.extend(deferred());
    groups.extend(controls());
    let mut modules = Vec::new();
    let mut tests = String::from("import Foundation\nimport XCTest\n");
    for index in 0..groups.len() {
        let _ = writeln!(tests, "@testable import ResourceGroup{index}");
    }
    tests.push_str("private final class CapturedOutcome: @unchecked Sendable {\n    private let lock = NSLock()\n    private var value = \"unfinished\"\n    func set(_ value: String) { lock.lock(); self.value = value; lock.unlock() }\n    func get() -> String { lock.lock(); defer { lock.unlock() }; return value }\n}\nfinal class ResourceVectors: XCTestCase {\n");
    let mut count = 0;
    for (group_index, group) in groups.iter().enumerate() {
        let module = format!("ResourceGroup{group_index}");
        modules.push(module.clone());
        let program = group.program();
        if group.name == "nested-entry-without-resource-root" {
            assert!(
                !program
                    .nodes
                    .iter()
                    .any(|n| n.source.document.ends_with("/outer.json")
                        && n.source.pointer.is_empty())
            );
        }
        let directory = root.join("Sources").join(&module);
        std::fs::create_dir_all(&directory).unwrap();
        for (name, text) in [
            ("Json.swift", include_str!("json.swift")),
            ("Number.swift", include_str!("number.swift")),
            ("Validation.swift", include_str!("validation.swift")),
        ] {
            std::fs::write(directory.join(name), text).unwrap();
        }
        std::fs::write(
            directory.join("Program.swift"),
            emit(&group.contract, &program).unwrap(),
        )
        .unwrap();
        std::fs::write(
            root.join(format!("{module}-program.json")),
            serde_json::to_string_pretty(&program).unwrap(),
        )
        .unwrap();
        let mut inputs = Vec::new();
        let documents: std::collections::BTreeSet<_> = group
            .contract
            .schemas()
            .map(|s| s.id().document().clone())
            .collect();
        for document in documents {
            let document_id = SchemaId::new(document, Default::default());
            inputs.push(json!({"document":document_id.document().as_str(),"value":group.contract.source(&document_id)}));
        }
        std::fs::write(root.join(format!("{module}-inputs.json")), serde_json::to_string_pretty(&json!({"group":group.name,"documents":inputs,"cases":group.cases.iter().map(|c|json!({"name":c.name,"root":c.root.pointer(),"instanceJson":c.instance,"expected":c.expected})).collect::<Vec<_>>()})).unwrap()).unwrap();
        for (case_index, case) in group.cases.iter().enumerate() {
            count += 1;
            let target = index(&program, &case.root);
            let _ = writeln!(
                tests,
                "    func testGroup{group_index}Case{case_index}() throws {{ // {}: {}",
                group.name.replace('\n', " "),
                case.name.replace('\n', " ")
            );
            if group.name == "normal-stack-depth" {
                let _ = writeln!(
                    tests,
                    "        let outcome = CapturedOutcome()\n        let done = DispatchSemaphore(value: 0)\n        let thread = Thread {{\n            defer {{ done.signal() }}\n            do {{ try {module}.ValidationSession().check({target}, .null); outcome.set(\"accepted\") }}\n            catch let error as {module}.ValidationError {{ outcome.set(error.kind.rawValue + \"|\" + error.source.pointer + \"|\" + (error.message.contains(\"depth\") ? \"depth\" : \"other\")) }}\n            catch {{ outcome.set(\"wrong failure\") }}\n        }}\n        thread.stackSize = 2 * 1024 * 1024\n        thread.start()\n        XCTAssertEqual(done.wait(timeout: .now() + 15), .success)\n        XCTAssertEqual(outcome.get(), \"evaluationFailure|/components/schemas/N512|depth\")\n    }}"
                );
                continue;
            }
            let call = format!(
                "{module}.ValidationSession().check({target}, {module}.JsonValue.parse({}))",
                string(&case.instance)
            );
            if case.expected == "Valid" {
                let _ = writeln!(tests, "        XCTAssertNoThrow(try {call})");
            } else {
                let kind = if case.expected == "Invalid" {
                    "invalid"
                } else {
                    "evaluationFailure"
                };
                let _ = writeln!(
                    tests,
                    "        XCTAssertThrowsError(try {call}) {{ error in\n            guard let failure = error as? {module}.ValidationError else {{ return XCTFail(\"wrong failure type\") }}\n            XCTAssertEqual(failure.kind, .{kind})"
                );
                if let Some((at, path)) = &case.location {
                    let _ = writeln!(
                        tests,
                        "            XCTAssertEqual(failure.source.document, {})\n            XCTAssertEqual(failure.source.pointer, {})\n            XCTAssertEqual(failure.instancePath, {})",
                        string(at.document().as_str()),
                        string(at.pointer()),
                        string(path)
                    );
                }
                tests.push_str("        }\n");
            }
            tests.push_str("    }\n");
        }
        if group.name == "scope-and-cycle-controls" {
            count += 1;
            let cycle = index(&program, &schema("Cycle"));
            let fallback = index(&program, &schema("Fallback"));
            let _ = writeln!(
                tests,
                "    func testContextRestoresAfterThrowAndAcrossTopLevelChecks() throws {{\n        let session = {module}.ValidationSession()\n        XCTAssertThrowsError(try session.check({cycle}, .null)) {{ error in XCTAssertEqual((error as? {module}.ValidationError)?.kind, .evaluationFailure) }}\n        XCTAssertNoThrow(try session.check({fallback}, .string(\"s\")))\n        XCTAssertThrowsError(try session.check({fallback}, .number(1))) {{ error in XCTAssertEqual((error as? {module}.ValidationError)?.kind, .invalid) }}\n        XCTAssertNoThrow(try session.check({fallback}, .string(\"again\")))\n    }}"
            );
        }
    }
    tests.push_str("}\n");
    std::fs::create_dir_all(root.join("Tests/Vectors")).unwrap();
    std::fs::write(root.join("Tests/Vectors/Vectors.swift"), tests).unwrap();
    let mut targets = modules
        .iter()
        .map(|m| {
            format!(
                ".target(name: {}, swiftSettings: [.swiftLanguageMode(.v6)])",
                string(m)
            )
        })
        .collect::<Vec<_>>();
    targets.push(format!(".testTarget(name: \"Vectors\", dependencies: [{}], swiftSettings: [.swiftLanguageMode(.v6)])", modules.iter().map(|m|string(m)).collect::<Vec<_>>().join(", ")));
    std::fs::write(root.join("Package.swift"), format!("// swift-tools-version: 6.0\nimport PackageDescription\nlet package = Package(name: \"ResourceRuntime\", platforms: [.macOS(.v13)], targets: [{}])\n",targets.join(", "))).unwrap();
    support::checked(
        support::swift("test")
            .args(["--jobs", "4"])
            .arg("--package-path")
            .arg(&root)
            .arg("--scratch-path")
            .arg(root.join("build/runtime"))
            .args(["-Xswiftc", "-warnings-as-errors"]),
        &root,
    );
    println!(
        "Swift v3: 44 official + 4 deferred + independent controls = {count} native tests passed at {}",
        root.display()
    );
}
