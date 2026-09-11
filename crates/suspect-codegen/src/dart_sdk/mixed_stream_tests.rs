//! A response media choice can contain both finite data and an item stream.
use super::{DartConfig, Plan, PlannedPayload, support};
use serde_json::json;
use std::path::Path;

fn fixture(root: &Path) -> Plan {
    let path = root.join("api.json");
    let object = json!({"type":"object","required":["message"],"properties":{"message":{"type":"string"}}});
    std::fs::write(&path, json!({
        "openapi":"3.2.0", "info":{"title":"Mixed response","version":"1"},
        "servers":[{"url":"https://example.test"}],
        "paths":{"/mixed":{"get":{"operationId":"mixed","responses":{
            "200":{"description":"Complete value or events","content":{
                "application/json":{"schema":object},
                "text/event-stream":{"itemSchema":{"type":"object","required":["data"],"properties":{"data":{"type":"string"}}}}
            }},
            "204":{"description":"No content"},
            "400":{"description":"Bounded error","content":{"application/json":{"schema":object}}}
        }}}}
    }).to_string()).unwrap();
    let contract = support::load(&path);
    let selected = contract.operations().map(|operation| operation.source().clone()).collect::<Vec<_>>();
    super::plan_sdk(contract, &selected, DartConfig::default()).unwrap()
}

#[test]
fn mixed_response_media_keep_every_success_alternative() {
    let root = tempfile::tempdir().unwrap();
    let plan = fixture(root.path());
    let operation = &plan.operations()[0];
    assert!(operation.stream);
    let status = operation.statuses.iter().find(|status| status.wire.status_key() == "200").unwrap();
    assert_eq!(status.media.len(), 2);
    assert!(status.media.iter().any(|media| matches!(media.payload, PlannedPayload::Json { .. })));
    assert!(status.media.iter().any(|media| matches!(media.payload, PlannedPayload::Stream { .. })));
    assert!(operation.statuses.iter().any(|status| status.wire.status_key() == "204" && status.success_name.is_some()));
}

#[test]
#[ignore = "requires native Dart SDK"]
fn native_mixed_response_stream_dispatches_by_actual_status_and_media() {
    let root = support::root("mixed-stream-");
    let plan = fixture(&root);
    crate::write_files(&plan.render(), &root).unwrap();
    let operation = &plan.operations()[0];
    let status = operation.statuses.iter().find(|status| status.wire.status_key() == "200").unwrap();
    let mut source = include_str!("native_mixed_stream.dart").to_owned();
    for media in &status.media {
        let placeholder = match media.payload {
            PlannedPayload::Json { .. } => "__JSON_VARIANT__",
            PlannedPayload::Stream { .. } => "__SSE_VARIANT__",
            _ => unreachable!(),
        };
        source = source.replace(placeholder, &media.variant_name);
    }
    std::fs::create_dir(root.join("dart/bin")).unwrap();
    std::fs::write(root.join("dart/bin/mixed.dart"), source).unwrap();
    for (name, arguments) in [
        ("pub", vec!["pub", "get", "--offline"]),
        ("analyze", vec!["analyze", "--fatal-infos"]),
        ("run", vec!["run", "bin/mixed.dart"]),
    ] {
        support::check(support::dart(&root).args(arguments).current_dir(root.join("dart")), &root, name);
    }
}
