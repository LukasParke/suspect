#![cfg(feature = "http-protocol")]
//! Source examples use real protocol slots, without another narrow admission pass.

use serde_json::{Value, json};
use std::sync::Arc;
use suspect_codegen::{
    examples::{
        ExampleConfig, ExampleOrigin, ExamplePartPosition, ExampleRole, plan_protocol_examples,
        plan_protocol_examples_v2, plan_protocol_examples_v3,
    },
    http_protocol::{self, Capabilities, Capability},
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn load(value: Value) -> Arc<Contract> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("api.json");
    std::fs::write(&path, value.to_string()).unwrap();
    let workspace = Arc::new(WorkspaceBuilder::new().build().unwrap());
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap())
}
fn api(version: &str, operation: Value) -> Value {
    json!({"openapi":version,"info":{"title":"Protocol examples","version":"1"},
        "servers":[{"url":"https://example.test/v1"}],"paths":{"/example":{"post":operation}}})
}
fn examples(contract: Arc<Contract>) -> suspect_codegen::examples::ExamplePlan {
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let wire = http_protocol::plan(
        &contract,
        &selected,
        Capabilities::for_adapter("normative-examples", Capability::ALL.iter().copied()),
    )
    .into_result()
    .unwrap();
    plan_protocol_examples(contract, &wire, Default::default())
}

#[test]
fn scoped_examples_check_all_applicators_and_keep_declared_slot_provenance() {
    let mut value = api(
        "3.1.2",
        json!({"operationId":"scoped","security":[],
        "requestBody":{"required":true,"content":{"application/json":{
            "schema":{"$ref":"#/components/schemas/Message"},"examples":{
                "accepted":{"value":{"mode":"short","x-a":1,"x-b":2}},
                "conditional":{"value":{"mode":"full"}},
                "dependency":{"value":{"mode":"short","x-a":1}},
                "pattern":{"value":{"mode":"short","x-a":"wrong","x-b":2}},
                "unevaluated":{"value":{"mode":"short","other":7}}
            }}}},"responses":{"204":{"description":"Stored"}}}),
    );
    value["components"] = json!({"schemas":{"Message":{
        "type":"object","required":["mode"],
        "properties":{"mode":{"enum":["full","short"]}},
        "patternProperties":{"^x-":{"type":"integer"}},
        "if":{"properties":{"mode":{"const":"full"}}},
        "then":{"required":["detail"],"properties":{"detail":{"type":"string"}}},
        "dependentRequired":{"x-a":["x-b"]},"unevaluatedProperties":false
    }}});
    let contract = load(value);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let wire = http_protocol::plan(
        &contract,
        &selected,
        Capabilities::for_adapter("scoped-example-witness", Capability::ALL.iter().copied()),
    )
    .into_result()
    .unwrap();
    let legacy = plan_protocol_examples(contract.clone(), &wire, Default::default());
    assert!(legacy.operations().is_empty());
    assert!(
        legacy
            .diagnostics()
            .iter()
            .any(|d| d.code == "examples-schema-unsupported")
    );

    let scoped = plan_protocol_examples_v2(contract.clone(), &wire, Default::default());
    assert_eq!(scoped.operations().len(), 1, "{:?}", scoped.diagnostics());
    let entries = &scoped.operations()[0].entries;
    assert_eq!(entries.len(), 1, "{entries:?}");
    let entry = &entries[0];
    assert_eq!(entry.role, ExampleRole::RequestBody);
    assert_eq!(entry.origin, ExampleOrigin::Declared);
    assert_eq!(entry.value, json!({"mode":"short","x-a":1,"x-b":2}));
    assert_eq!(
        entry.container.pointer(),
        "/paths/~1example/post/requestBody/content/application~1json"
    );
    assert_eq!(
        entry.schema.pointer(),
        "/paths/~1example/post/requestBody/content/application~1json/schema"
    );
    assert_eq!(
        entry.declared_source.as_ref().unwrap(),
        &entry
            .container
            .child("examples")
            .child("accepted")
            .child("value")
    );
    let invalid = scoped
        .diagnostics()
        .iter()
        .filter(|d| d.code == "examples-declared-invalid")
        .collect::<Vec<_>>();
    assert_eq!(invalid.len(), 4, "{:?}", scoped.diagnostics());
    for name in ["conditional", "dependency", "pattern", "unevaluated"] {
        let source = entry.container.child("examples").child(name).child("value");
        assert!(
            invalid
                .iter()
                .any(|d| d.source == source && !d.at.is_empty()),
            "missing declared finding for {name}"
        );
    }

    let limited = plan_protocol_examples_v2(
        contract,
        &wire,
        ExampleConfig {
            max_validation_steps: 1,
            ..Default::default()
        },
    );
    assert!(limited.operations().iter().all(|op| op.entries.is_empty()));
    assert!(
        limited
            .diagnostics()
            .iter()
            .any(|d| d.code == "examples-evaluation-incomplete"),
        "{:?}",
        limited.diagnostics()
    );
}

#[test]
fn explicit_scoped_example_profile_preserves_base_values_and_synthesis_identity() {
    let contract = load(api(
        "3.1.2",
        json!({"operationId":"base","security":[],
        "parameters":[{"name":"tag","in":"query","schema":{"type":"string"},"example":"declared"}],
        "requestBody":{"required":true,"content":{"application/json":{"schema":{
            "type":"object","required":["name"],"additionalProperties":false,
            "properties":{"name":{"type":"string","minLength":2}}
        }}}},"responses":{"204":{"description":"Stored"}}}),
    ));
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let wire = http_protocol::plan(
        &contract,
        &selected,
        Capabilities::for_adapter("base-example-witness", Capability::ALL.iter().copied()),
    )
    .into_result()
    .unwrap();
    let legacy = plan_protocol_examples(contract.clone(), &wire, Default::default());
    let scoped = plan_protocol_examples_v2(contract, &wire, Default::default());
    let values = |plan: &suspect_codegen::examples::ExamplePlan| {
        plan.operations()
            .iter()
            .flat_map(|op| {
                op.entries.iter().map(|entry| {
                    (
                        op.source.clone(),
                        op.operation_id.clone(),
                        entry.role.clone(),
                        entry.origin,
                        entry.container.clone(),
                        entry.schema.clone(),
                        entry.declared_source.clone(),
                        entry.value.clone(),
                        entry.name.clone(),
                        entry.summary.clone(),
                    )
                })
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(values(&legacy), values(&scoped));
    assert_eq!(values(&scoped).len(), 2);
    assert!(
        scoped.operations()[0]
            .entries
            .iter()
            .any(|e| e.origin == ExampleOrigin::Synthesized)
    );
    assert!(
        legacy.diagnostics().is_empty(),
        "{:?}",
        legacy.diagnostics()
    );
    assert!(
        scoped.diagnostics().is_empty(),
        "{:?}",
        scoped.diagnostics()
    );
}

#[test]
fn explicit_v3_examples_validate_dynamic_scope_and_keep_physical_declared_locations() {
    let mut value = api(
        "3.2.0",
        json!({"operationId":"strictTree","security":[],
        "requestBody":{"required":true,"content":{"application/json":{
            "schema":{"$ref":"https://schema.test/strict"},"examples":{
                "good":{"dataValue":{"data":"root","children":[{"data":"child"}]}},
                "bad":{"dataValue":{"children":[{"unexpected":1}]}}
            }}}},"responses":{"204":{"description":"Stored"}}}),
    );
    value["$self"] = json!("https://logical.test/api");
    value["components"] = json!({"schemas":{
        "Tree":{"$id":"https://schema.test/tree","$dynamicAnchor":"node","type":"object",
            "properties":{"data":true,"children":{"type":"array","items":{"$dynamicRef":"#node"}}}},
        "Strict":{"$id":"https://schema.test/strict","$dynamicAnchor":"node","$ref":"tree","unevaluatedProperties":false}
    }});
    let contract = load(value);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let wire = http_protocol::plan(
        &contract,
        &selected,
        Capabilities::for_adapter("v3-example-witness", Capability::ALL.iter().copied()),
    )
    .into_result()
    .unwrap();
    let prior = plan_protocol_examples_v2(contract.clone(), &wire, Default::default());
    assert!(prior.operations().is_empty());
    assert!(
        prior
            .diagnostics()
            .iter()
            .any(|d| d.code == "examples-schema-unsupported")
    );
    let plan = plan_protocol_examples_v3(contract.clone(), &wire, Default::default());
    assert_eq!(
        plan.operations()[0].entries.len(),
        1,
        "{:?}",
        plan.diagnostics()
    );
    let entry = &plan.operations()[0].entries[0];
    assert_eq!(
        entry.value,
        json!({"data":"root","children":[{"data":"child"}]})
    );
    assert_eq!(entry.origin, ExampleOrigin::Declared);
    let source = entry.declared_source.as_ref().unwrap();
    assert_eq!(source.document(), contract.entry());
    assert!(source.document().as_str().starts_with("file:"));
    assert!(source.pointer().ends_with("/examples/good/dataValue"));
    assert!(
        plan.diagnostics()
            .iter()
            .any(|d| d.code == "examples-declared-invalid"
                && d.source.pointer().ends_with("/examples/bad/dataValue")
                && d.source.document() == contract.entry())
    );
    let limited = plan_protocol_examples_v3(
        contract,
        &wire,
        ExampleConfig {
            max_validation_steps: 1,
            ..Default::default()
        },
    );
    assert!(limited.operations().iter().all(|op| op.entries.is_empty()));
    assert!(
        limited
            .diagnostics()
            .iter()
            .any(|d| d.code == "examples-evaluation-incomplete")
    );
}

#[test]
fn v3_discovery_never_claims_initial_dynamic_fallback_annotations_as_active_declarations() {
    let mut value = api(
        "3.2.0",
        json!({"operationId":"dynamicValue","security":[],
        "requestBody":{"required":true,"content":{"application/json":{
            "schema":{"$ref":"https://schema.test/outer"}}}},"responses":{"204":{"description":"Stored"}}}),
    );
    value["components"] = json!({"schemas":{
        "Outer":{"$id":"https://schema.test/outer","$defs":{"Bound":{"$dynamicAnchor":"value","type":"string"}},"$ref":"use"},
        "Use":{"$id":"https://schema.test/use","type":"string","$dynamicRef":"fallback#value"},
        "Fallback":{"$id":"https://schema.test/fallback","$dynamicAnchor":"value","type":"boolean","examples":[false]}
    }});
    let contract = load(value);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let wire = http_protocol::plan(
        &contract,
        &selected,
        Capabilities::for_adapter(
            "dynamic-annotation-witness",
            Capability::ALL.iter().copied(),
        ),
    )
    .into_result()
    .unwrap();
    let plan = plan_protocol_examples_v3(contract, &wire, Default::default());
    assert!(plan.diagnostics().is_empty(), "{:?}", plan.diagnostics());
    assert_eq!(plan.operations()[0].entries.len(), 1);
    let entry = &plan.operations()[0].entries[0];
    assert_eq!(entry.origin, ExampleOrigin::Synthesized);
    assert_eq!(entry.value, json!("x"));
    assert!(entry.declared_source.is_none());
}

#[test]
fn review_form_container_examples_are_validated_before_declared_member_projection() {
    for planner in [plan_protocol_examples, plan_protocol_examples_v2] {
        let contract = load(api(
            "3.2.0",
            json!({"operationId":"form","security":[],
            "requestBody":{"required":true,"content":{"application/x-www-form-urlencoded":{
                "schema":{"type":"object","required":["count"],"additionalProperties":false,
                    "properties":{"count":{"type":"integer"}}},
                "examples":{"good":{"dataValue":{"count":7}},"bad":{"dataValue":{"count":"bad"}}}
            }}},"responses":{"204":{"description":"Stored"}}}),
        ));
        let selected = contract
            .operations()
            .map(|op| op.source().clone())
            .collect::<Vec<_>>();
        let wire = http_protocol::plan(
            &contract,
            &selected,
            Capabilities::for_adapter("aggregate-example-witness", Capability::ALL.iter().copied()),
        )
        .into_result()
        .unwrap();
        let roots = wire.codec_roots().to_vec();
        let plan = planner(contract, &wire, Default::default());
        assert!(
            plan.diagnostics()
                .iter()
                .any(|d| d.code == "examples-declared-invalid"
                    && d.source.pointer().ends_with("/examples/bad/dataValue")),
            "{:?}",
            plan.diagnostics()
        );
        let operation = &plan.operations()[0];
        assert_eq!(operation.validated_aggregates.len(), 1);
        assert_eq!(operation.validated_aggregates[0].value, json!({"count":7}));
        assert_eq!(
            operation.validated_aggregates[0].origin,
            ExampleOrigin::Declared
        );
        assert_eq!(operation.entries.len(), 1);
        let count = &operation.entries[0];
        assert_eq!(count.value, json!(7));
        assert_eq!(count.origin, ExampleOrigin::Declared);
        assert!(
            matches!(&count.role, ExampleRole::RequestPart { name:Some(name), .. } if name == "count")
        );
        assert!(
            count
                .declared_source
                .as_ref()
                .unwrap()
                .pointer()
                .ends_with("/examples/good/dataValue/count")
        );
        assert!(roots.contains(&count.schema));
        assert!(
            !roots.contains(&operation.validated_aggregates[0].schema),
            "example-only aggregate roots are not invented native JSON codecs"
        );
    }
}

#[test]
fn review_referenced_aggregates_preserve_array_members_and_optional_absence() {
    for planner in [plan_protocol_examples, plan_protocol_examples_v2] {
        let mut value = api(
            "3.2.0",
            json!({"operationId":"namedParts","security":[],
            "requestBody":{"required":true,"content":{"application/x-www-form-urlencoded":{"$ref":"#/components/mediaTypes/Form"}}},
            "responses":{"204":{"description":"Stored"}}}),
        );
        value["components"] = json!({"mediaTypes":{"Form":{
            "schema":{"type":"object","required":["name","tags"],"additionalProperties":false,"properties":{
                "name":{"type":"string"},"tags":{"type":"array","items":{"type":"integer"}},
                "omitted":{"type":"string","examples":["unwanted fallback"]}}},
            "encoding":{"tags":{"contentType":"application/json"}},
            "examples":{"source":{"dataValue":{"name":"declared","tags":[1,2]}}}
        }}});
        let contract = load(value);
        let selected = contract
            .operations()
            .map(|op| op.source().clone())
            .collect::<Vec<_>>();
        let wire = http_protocol::plan(
            &contract,
            &selected,
            Capabilities::for_adapter(
                "referenced-aggregate-witness",
                Capability::ALL.iter().copied(),
            ),
        )
        .into_result()
        .unwrap();
        let plan = planner(contract, &wire, Default::default());
        assert!(plan.diagnostics().is_empty(), "{:?}", plan.diagnostics());
        let operation = &plan.operations()[0];
        assert_eq!(operation.validated_aggregates.len(), 1);
        assert!(
            operation
                .entries
                .iter()
                .all(|entry| entry.origin == ExampleOrigin::Declared)
        );
        assert!(
            operation
                .entries
                .iter()
                .all(|entry| entry.value != json!("unwanted fallback"))
        );
        for (value, suffix) in [
            (json!("declared"), "/name"),
            (json!(1), "/tags/0"),
            (json!(2), "/tags/1"),
        ] {
            assert!(
                operation.entries.iter().any(|entry| entry.value == value
                    && entry.declared_source.as_ref().unwrap().pointer()
                        == format!(
                            "/components/mediaTypes/Form/examples/source/dataValue{suffix}"
                        )),
                "{:?}",
                operation.entries
            );
        }
    }
}

#[test]
fn review_byte_aggregate_declarations_are_located_unavailable_not_json_placeholders() {
    for planner in [plan_protocol_examples, plan_protocol_examples_v2] {
        let contract = load(api(
            "3.2.0",
            json!({"operationId":"upload","security":[],
            "requestBody":{"required":true,"content":{"multipart/form-data":{
                "schema":{"type":"object","required":["file","label"],"additionalProperties":false,
                    "properties":{"file":{},"label":{"type":"string","examples":["schema label"]}}},
                "encoding":{"file":{"contentType":"application/octet-stream"}},
                "examples":{"bytes":{"dataValue":{"file":null,"label":"container label"}}}
            }}},"responses":{"204":{"description":"Stored"}}}),
        ));
        let selected = contract
            .operations()
            .map(|op| op.source().clone())
            .collect::<Vec<_>>();
        let wire = http_protocol::plan(
            &contract,
            &selected,
            Capabilities::for_adapter("byte-aggregate-witness", Capability::ALL.iter().copied()),
        )
        .into_result()
        .unwrap();
        let plan = planner(contract, &wire, Default::default());
        assert!(
            plan.diagnostics()
                .iter()
                .any(|d| d.code == "examples-declared-unavailable"
                    && d.source.pointer().ends_with("/examples/bytes/dataValue")
                    && d.message.contains("byte")),
            "{:?}",
            plan.diagnostics()
        );
        assert!(plan.operations()[0].validated_aggregates.is_empty());
        assert!(
            plan.operations()[0]
                .entries
                .iter()
                .all(|entry| !entry.schema.pointer().ends_with("/properties/file"))
        );
        assert!(
            plan.operations()[0]
                .entries
                .iter()
                .all(|entry| entry.value != Value::Null)
        );
    }
}

#[test]
fn review_positional_occurrences_do_not_share_declared_values_with_the_same_items_schema() {
    for planner in [plan_protocol_examples, plan_protocol_examples_v2] {
        for values in [
            json!(["a", "b", "c"]),
            json!(["a"]),
            json!(["a", "b", "c", "d"]),
            json!([]),
        ] {
            let contract = load(api(
                "3.2.0",
                json!({"operationId":"positions","security":[],
                "requestBody":{"required":true,"content":{"multipart/mixed":{
                    "schema":{"type":"array","items":{"type":"string","examples":["must not fill absent positions"]}},
                    "prefixEncoding":[{"contentType":"text/plain"},{"contentType":"application/json"}],
                    "itemEncoding":{"contentType":"text/plain"},
                    "examples":{"ordered":{"dataValue":values}}
                }}},"responses":{"204":{"description":"Stored"}}}),
            ));
            let selected = contract
                .operations()
                .map(|op| op.source().clone())
                .collect::<Vec<_>>();
            let wire = http_protocol::plan(
                &contract,
                &selected,
                Capabilities::for_adapter(
                    "positional-example-occurrence",
                    Capability::ALL.iter().copied(),
                ),
            )
            .into_result()
            .unwrap();
            assert_eq!(
                wire.codec_roots().len(),
                1,
                "all occurrences intentionally share the same item schema"
            );
            let plan = planner(contract, &wire, Default::default());
            assert!(plan.diagnostics().is_empty(), "{:?}", plan.diagnostics());
            let operation = &plan.operations()[0];
            assert_eq!(operation.validated_aggregates.len(), 1);
            assert_eq!(operation.validated_aggregates[0].value, values);
            let expected = values.as_array().unwrap();
            assert_eq!(
                operation.entries.len(),
                expected.len(),
                "{:?}",
                operation.entries
            );
            for (index, value) in expected.iter().enumerate() {
                let entry = &operation.entries[index];
                assert_eq!(&entry.value, value);
                assert_eq!(entry.origin, ExampleOrigin::Declared);
                assert!(
                    entry
                        .declared_source
                        .as_ref()
                        .unwrap()
                        .pointer()
                        .ends_with(&format!("/examples/ordered/dataValue/{index}"))
                );
                assert_eq!(
                    entry.media_type,
                    if index == 1 {
                        "application/json"
                    } else {
                        "text/plain"
                    }
                );
                assert_eq!(
                    entry.part_position,
                    Some(if index < 2 {
                        ExamplePartPosition::Prefix(index)
                    } else {
                        ExamplePartPosition::Items
                    })
                );
            }
        }
    }
}

#[test]
fn unavailable_example_references_remain_findings_without_weakening_wire_reference_admission() {
    let mut value = api(
        "3.1.2",
        json!({"operationId":"exampleAvailability","security":[],
        "responses":{"200":{"description":"Value","content":{"application/json":{
            "schema":{"type":"string"},"examples":{"unavailable":{"$ref":"example.json#/value"}}
        }}}}}),
    );
    let contract = load(value.clone());
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let wire = http_protocol::plan(
        &contract,
        &selected,
        Capabilities::for_adapter("example-availability", Capability::ALL.iter().copied()),
    );
    assert!(wire.is_admitted(), "{:?}", wire.diagnostics());
    assert!(wire.diagnostics().iter().any(|finding| {
        finding.code() == "http-reference-unresolved"
            && finding.severity() == http_protocol::Severity::Warning
            && finding
                .source()
                .source()
                .pointer()
                .ends_with("/examples/unavailable/$ref")
    }));
    let examples = plan_protocol_examples(contract, &wire, Default::default());
    assert!(
        examples
            .diagnostics()
            .iter()
            .any(|finding| finding.code == "examples-declared-unavailable")
    );
    assert!(
        examples.operations()[0]
            .entries
            .iter()
            .all(|entry| entry.origin == ExampleOrigin::Synthesized)
    );
    value["paths"]["/example"]["post"]["responses"]["200"]["content"]["application/json"]["schema"] =
        json!({"$ref":"schema.json#/value"});
    let contract = load(value);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let invalid = http_protocol::plan(
        &contract,
        &selected,
        Capabilities::for_adapter("wire-reference-required", Capability::ALL.iter().copied()),
    );
    assert!(
        !invalid.is_admitted(),
        "missing wire schemas still block the complete package"
    );
}

#[test]
fn status_patterns_and_referenced_media_keep_declared_values_and_terminal_sources() {
    let mut value = api(
        "3.2.0",
        json!({"operationId":"inspect","security":[],
        "requestBody":{"required":true,"content":{"application/problem+json":{"schema":{"type":"string"},"example":"input"}}},
        "responses":{"2XX":{"description":"ok","content":{"application/json":{"$ref":"#/components/mediaTypes/Message"}}},
            "default":{"description":"other","content":{"application/json":{"schema":{"type":"string"},"example":"fallback"}}}}}),
    );
    value["components"] = json!({"mediaTypes":{"Message":{"schema":{"type":"string"},"examples":{"example":{"dataValue":"success"}}}}});
    let plan = examples(load(value));
    assert!(plan.diagnostics().is_empty(), "{:?}", plan.diagnostics());
    let entries = &plan.operations()[0].entries;
    let response = entries
        .iter()
        .find(|entry| {
            entry.role
                == ExampleRole::ResponsePattern {
                    status: "2XX".into(),
                }
        })
        .unwrap();
    assert_eq!(response.value, json!("success"));
    assert_eq!(response.origin, ExampleOrigin::Declared);
    assert!(
        response
            .declared_source
            .as_ref()
            .unwrap()
            .pointer()
            .ends_with("/components/mediaTypes/Message/examples/example/dataValue")
    );
    assert!(entries.iter().any(|entry| entry.role
        == ExampleRole::ResponsePattern {
            status: "default".into()
        }
        && entry.value == json!("fallback")));
    assert_eq!(plan.format(), "suspect-sdk-examples-v2");
}

#[test]
fn multipart_examples_validate_text_parts_without_fake_json_for_file_bytes() {
    let plan = examples(load(api(
        "3.1.0",
        json!({"operationId":"upload","security":[],
        "requestBody":{"required":true,"content":{"multipart/form-data":{"schema":{"type":"object","additionalProperties":false,
            "required":["file","label"],"properties":{"file":{},"label":{"type":"string","examples":["example label"]}}},
            "encoding":{"file":{"contentType":"application/octet-stream"}}}}},
        "responses":{"204":{"description":"saved"}}}),
    )));
    let entries = &plan.operations()[0].entries;
    assert!(entries.iter().any(
        |entry| matches!(&entry.role,ExampleRole::RequestPart{name:Some(name),..} if name=="label")
            && entry.value == json!("example label")
    ));
    assert!(
        !entries
            .iter()
            .any(|entry| entry.schema.pointer().ends_with("/properties/file"))
    );
    assert!(
        plan.diagnostics()
            .iter()
            .any(|finding| finding.code == "examples-native-bytes-required")
    );
    assert!(
        !entries
            .iter()
            .any(|entry| entry.role == ExampleRole::RequestBody)
    );
}

#[test]
fn streamed_examples_are_event_envelopes_and_do_not_parse_data_as_json() {
    let plan = examples(load(api(
        "3.2.0",
        json!({"operationId":"events","security":[],
        "responses":{"200":{"description":"events","content":{"text/event-stream":{
            "itemSchema":{"type":"object","required":["data"],"additionalProperties":false,
                "properties":{"data":{"type":"string"}},"examples":[{"data":"{\"x\":1}"}]},
            "example":"data: a whole stream is not a schema item\n\n"
        }}}}}),
    )));
    let entries = &plan.operations()[0].entries;
    assert_eq!(entries.len(), 1, "{:?}", plan.diagnostics());
    assert_eq!(
        entries[0].role,
        ExampleRole::ResponseItem {
            status: "200".into()
        }
    );
    assert_eq!(entries[0].value, json!({"data":"{\"x\":1}"}));
    assert_eq!(entries[0].origin, ExampleOrigin::Declared);
    assert!(entries[0].schema.pointer().ends_with("/itemSchema"));
}

#[test]
fn bodyless_status_does_not_gain_examples_from_a_schema_used_by_the_request() {
    let mut value = api(
        "3.2.0",
        json!({"operationId":"save","security":[],
        "requestBody":{"required":true,"content":{"application/json":{"$ref":"#/components/mediaTypes/Shared"}}},
        "responses":{"204":{"description":"saved","content":{"application/json":{"$ref":"#/components/mediaTypes/Shared"}}}}}),
    );
    value["components"] =
        json!({"mediaTypes":{"Shared":{"schema":{"type":"string"},"example":"request"}}});
    let plan = examples(load(value));
    let entries = &plan.operations()[0].entries;
    assert_eq!(entries.len(), 1, "{:?}", plan.diagnostics());
    assert_eq!(entries[0].role, ExampleRole::RequestBody);
    assert_eq!(entries[0].value, json!("request"));
}

#[test]
fn reset_content_response_never_requires_or_decodes_a_payload() {
    // RFC 9110 §15.3.6: a server MUST NOT generate content in a 205 response.
    let contract = load(api(
        "3.1.2",
        json!({"operationId":"reset","security":[],
        "responses":{"205":{"description":"Reset content","content":{"application/json":{"schema":{"type":"string"}}}}}}),
    ));
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let protocol = http_protocol::plan(
        &contract,
        &selected,
        Capabilities::for_adapter("reset-content-witness", Capability::ALL.iter().copied()),
    )
    .into_result()
    .unwrap();
    assert!(
        protocol.codec_roots().is_empty(),
        "205 metadata must not create an unused payload codec"
    );
    let response = protocol.operations()[0].match_response(205, None).unwrap();
    assert!(response.is_success());
    assert_eq!(
        response.body_disposition(),
        http_protocol::ResponseBodyDisposition::ForbiddenByHttp
    );
}

#[test]
fn review_referenced_path_item_examples_bind_to_the_terminal_operation() {
    let contract = load(
        json!({"openapi":"3.1.2","info":{"title":"Mounted","version":"1"},
        "servers":[{"url":"https://example.test"}],"paths":{"/mounted":{"$ref":"#/components/pathItems/Shared"}},
        "components":{"pathItems":{"Shared":{"get":{"operationId":"mounted","security":[],
            "parameters":[{"name":"label","in":"query","schema":{"type":"string"},"example":"declared"}],
            "responses":{"200":{"description":"ok","content":{"application/json":{"schema":{"type":"string"},"example":"response"}}}}}}}}}),
    );
    let terminal = contract.operations().next().unwrap().source().clone();
    let plan = examples(contract);
    assert!(plan.diagnostics().is_empty(), "{:?}", plan.diagnostics());
    assert_eq!(plan.operations().len(), 1);
    assert_eq!(plan.operations()[0].source, terminal);
    assert!(
        plan.operations()[0].entries.iter().any(
            |entry| entry.value == json!("declared") && entry.origin == ExampleOrigin::Declared
        )
    );
}

#[test]
fn review_content_header_examples_are_validated_at_the_media_definition() {
    let plan = examples(load(api(
        "3.1.2",
        json!({"operationId":"headers","security":[],
        "responses":{"200":{"description":"headers","headers":{"X-Counter":{"content":{"application/json":{
            "schema":{"type":"integer"},"example":"not an integer"}}}},
            "content":{"application/json":{"schema":{"type":"string"},"example":"body"}}}}}),
    )));
    assert!(
        plan.diagnostics()
            .iter()
            .any(|finding| finding.code == "examples-declared-invalid"
                && finding
                    .source
                    .pointer()
                    .ends_with("/headers/X-Counter/content/application~1json/example")),
        "{:?}",
        plan.diagnostics()
    );
}

#[test]
fn review_data_value_is_kept_when_serialization_is_external() {
    let plan = examples(load(api(
        "3.2.0",
        json!({"operationId":"send","security":[],
        "requestBody":{"required":true,"content":{"application/json":{"schema":{"type":"string"},
            "examples":{"combined":{"dataValue":"known instance","externalValue":"https://examples.invalid/serialized.json"}}}}},
        "responses":{"204":{"description":"accepted"}}}),
    )));
    let entry = plan.operations()[0]
        .entries
        .iter()
        .find(|entry| entry.role == ExampleRole::RequestBody)
        .unwrap();
    assert_eq!(entry.value, json!("known instance"));
    assert_eq!(entry.origin, ExampleOrigin::Declared);
    assert!(
        entry
            .declared_source
            .as_ref()
            .unwrap()
            .pointer()
            .ends_with("/examples/combined/dataValue")
    );
}

#[test]
fn review_invalid_data_value_is_reported_even_with_external_serialization() {
    let plan = examples(load(api(
        "3.2.0",
        json!({"operationId":"invalid","security":[],
        "responses":{"200":{"description":"value","content":{"application/json":{"schema":{"type":"integer"},
            "examples":{"bad":{"dataValue":"invalid","externalValue":"https://examples.invalid/serialized.json"}}}}}}}),
    )));
    assert!(
        plan.diagnostics()
            .iter()
            .any(|finding| finding.code == "examples-declared-invalid"
                && finding
                    .source
                    .pointer()
                    .ends_with("/examples/bad/dataValue")),
        "{:?}",
        plan.diagnostics()
    );
}

#[test]
fn review_false_positional_prefix_forbids_every_later_multipart_part() {
    let contract = load(api(
        "3.2.0",
        json!({"operationId":"parts","security":[],
        "requestBody":{"content":{"multipart/mixed":{"schema":{"type":"array","prefixItems":[{"type":"string"},false],
            "items":{"type":"string"}},"prefixEncoding":[{},{}],"itemEncoding":{}}}},
        "responses":{"204":{"description":"accepted"}}}),
    ));
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let plan = http_protocol::plan(
        &contract,
        &selected,
        Capabilities::for_adapter("positional-witness", Capability::ALL.iter().copied()),
    )
    .into_result()
    .unwrap();
    let http_protocol::Representation::Multipart {
        multipart: http_protocol::MultipartPlan::Positional { prefix, items, .. },
    } = plan.operations()[0].body().unwrap().media()[0].representation()
    else {
        panic!("positional multipart expected")
    };
    assert_eq!(prefix.len(), 1);
    assert!(
        matches!(items, http_protocol::AdditionalParts::Forbidden),
        "a false second slot cannot become an allowed repeated tail"
    );
}

#[test]
fn review_protocol_admission_ignores_only_inert_oas30_reference_siblings() {
    let mut value = api(
        "3.0.4",
        json!({"operationId":"value","security":[],"responses":{"200":{"description":"value",
        "content":{"application/json":{"schema":{"$ref":"#/components/schemas/Value","$id":"ignored",
            "properties":{"ignored":{"$dynamicRef":"#missing"}}}}}}}}),
    );
    value["components"] = json!({"schemas":{"Value":{"type":"string"}}});
    let contract = load(value);
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    let plan = http_protocol::plan(
        &contract,
        &selected,
        Capabilities::for_adapter("oas30-ref-witness", Capability::ALL.iter().copied()),
    );
    assert!(
        plan.is_admitted(),
        "ignored reference siblings cannot add protocol limitations: {:?}",
        plan.diagnostics()
    );
}
