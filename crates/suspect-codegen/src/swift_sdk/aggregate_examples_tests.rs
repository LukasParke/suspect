//! Complete declared values drive native grouping without aggregate JSON codecs.
use super::validation_v3_support as support;
use super::*;
use serde_json::{Value, json};

fn plan() -> SdkPlan {
    let form = json!({"schema":{"type":"object","properties":{
        "id":{"type":"string"},"note":{"type":"string"},"tags":{"type":"array","items":{"type":"integer"}},"omitted":{"type":"string"}},
        "required":["id"],"additionalProperties":{"type":"array","items":{"type":"integer"}}},
        "examples":{"bad":{"dataValue":{"id":false}},"good":{"dataValue":{"id":"alpha beta","note":"provided","tags":[],"x":[1,2],"z":[]}},"zInvalid":{"dataValue":{"id":"valid","tags":["bad"]}}}});
    let named = json!({"schema":{"type":"object","properties":{"id":{"type":"string"},"items":{"type":"array","items":{"type":"integer"}},"omitted":{"type":"string"}},"required":["id"],"additionalProperties":{"type":"string"}},"example":{"id":"A","items":[1,2],"x":"tail"}});
    let ordered = json!({"schema":{"type":"array","prefixItems":[{"type":"integer"},{"type":"string"}],"items":{"type":"boolean"},"minItems":1,"maxItems":4},"prefixEncoding":[{"contentType":"application/json"},{"contentType":"text/plain"}],"itemEncoding":{"contentType":"application/json"},"example":[7,"declared",false,true]});
    let empty = json!({"schema":{"type":"array","prefixItems":[{"type":"string"}],"items":false},"prefixEncoding":[{"contentType":"text/plain"}],"example":[]});
    let unrepresentable = json!({"schema":{"type":"object","properties":{"values":{"type":"array","items":{"type":"integer"}}},"required":["values"],"additionalProperties":false},"example":{"values":[]}});
    let bytes = json!({"schema":{"type":"object","properties":{"data":{}},"required":["data"],"additionalProperties":false},"encoding":{"data":{"contentType":"application/octet-stream"}},"example":{"data":"not JSON bytes"}});
    let mut paths = serde_json::Map::new();
    for (path, name, media, value, required) in [
        (
            "/form",
            "declaredForm",
            "application/x-www-form-urlencoded",
            form,
            true,
        ),
        (
            "/named",
            "declaredNamed",
            "multipart/form-data",
            named,
            false,
        ),
        (
            "/ordered",
            "declaredOrdered",
            "multipart/mixed",
            ordered,
            true,
        ),
        ("/empty", "declaredEmpty", "multipart/mixed", empty, true),
        (
            "/required-empty",
            "requiredEmpty",
            "application/x-www-form-urlencoded",
            unrepresentable,
            true,
        ),
        ("/bytes", "byteControl", "multipart/form-data", bytes, true),
    ] {
        paths.insert(path.into(), json!({"post":{"operationId":name,"requestBody":{"required":required,"content":{media:value}},"responses":{"204":{"description":"Accepted"}}}}));
    }
    let contract = support::load(
        json!({"openapi":"3.2.0","$self":"https://logical.swift.test/recipes.json","info":{"title":"Native declared aggregate construction","version":"1"},"servers":[{"url":"https://api.swift.test"}],"paths":paths}),
        vec![],
    );
    let selected = contract
        .operations()
        .map(|op| op.source().clone())
        .collect::<Vec<_>>();
    plan_sdk(contract, &selected, SwiftConfig::default())
        .unwrap_or_else(|errors| panic!("{errors:#?}"))
}

#[test]
fn aggregate_recipes_preserve_source_grouping_and_do_not_invent_codec_roots() {
    let plan = plan();
    assert_eq!(
        plan.program.version,
        suspect_schema::OwnedProgram::V3_VERSION
    );
    for examples in plan.examples.operations() {
        for aggregate in &examples.validated_aggregates {
            assert!(!plan.protocol.codec_roots().contains(&aggregate.schema));
            assert!(!plan.models.types.contains_key(&aggregate.schema));
            assert_eq!(aggregate.origin, crate::examples::ExampleOrigin::Declared);
            assert!(aggregate.declared_source.is_some());
        }
    }
    for suffix in ["/examples/bad/dataValue", "/examples/zInvalid/dataValue"] {
        assert!(
            plan.examples
                .diagnostics()
                .iter()
                .any(|d| d.code == "examples-declared-invalid"
                    && d.source.pointer().ends_with(suffix)),
            "{:?}",
            plan.examples.diagnostics()
        );
    }
    assert!(plan.examples.diagnostics().iter().any(
        |d| d.code == "examples-declared-unavailable" && d.source.pointer().contains("~1bytes")
    ));
    let files = plan.render();
    let calls = &files
        .iter()
        .find(|f| f.path.ends_with("/Examples.swift"))
        .unwrap()
        .content;
    for name in [
        "declaredForm",
        "declaredNamed",
        "declaredOrdered",
        "declaredEmpty",
    ] {
        assert!(
            calls.contains(&format!("public static func {name}")),
            "{calls}"
        );
    }
    assert!(
        !calls.contains("public static func requiredEmpty"),
        "do not invent an item for a required empty repeated array"
    );
    let coverage: Value = serde_json::from_str(
        &files
            .iter()
            .find(|f| f.path == "example-coverage.json")
            .unwrap()
            .content,
    )
    .unwrap();
    assert_eq!(coverage["validatedAggregates"].as_array().unwrap().len(), 5);
}

#[test]
#[ignore = "focused installed native declared aggregate examples and DocC; no base/protocol matrix replay"]
fn native_installed_declared_aggregate_examples() {
    let root = support::root("aggregate-examples-");
    let plan = plan();
    super::resources_tests::installed(
        &plan,
        &root,
        include_str!("aggregate_examples_native.swift"),
        "https://api.swift.test",
    );
    support::docs(&root, "GeneratedSDK");
    println!(
        "Swift declared aggregate native constructors/wire/DocC passed at {}",
        root.display()
    );
}
