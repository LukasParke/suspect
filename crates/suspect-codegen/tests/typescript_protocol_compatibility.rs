//! Compatibility witnesses through the public Contract/snapshot boundary.
//! Native equality must describe the actual expanded TypeScript surface.

#![cfg(feature = "http-protocol")]

use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};
use suspect_codegen::{
    backend::{Backend, TargetConfig},
    compatibility::{self, PlanStatus},
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

struct Fixture {
    directory: tempfile::TempDir,
    entry: PathBuf,
}
impl Fixture {
    fn new(document: &Value) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let entry = directory.path().join("api.json");
        let fixture = Self { directory, entry };
        fixture.write(document);
        fixture
    }
    fn write(&self, document: &Value) {
        std::fs::write(&self.entry, serde_json::to_vec_pretty(document).unwrap()).unwrap();
    }
    fn load(&self) -> Arc<Contract> {
        let workspace = Arc::new(
            WorkspaceBuilder::new()
                .root(self.directory.path())
                .build()
                .unwrap(),
        );
        Arc::new(
            Contract::from_workspace(&workspace, &Uri::from_path(&self.entry).unwrap()).unwrap(),
        )
    }
}
fn target() -> TargetConfig {
    TargetConfig {
        backend: Backend::TypescriptHttp,
        package_name: "@fixture/protocol".into(),
        package_version: "1.0.0".into(),
        import_name: None,
    }
}
fn selection() -> Vec<String> {
    vec!["exchange".into(), "events".into()]
}
fn document() -> Value {
    json!({"openapi":"3.2.0","info":{"title":"Native protocol compatibility","version":"1"},
        "servers":[{"url":"https://{region}.example.test/v1","variables":{"region":{"default":"eu","enum":["eu","us"]}}}],
        "security":[{"token":[]},{"basic":[]}],
        "components":{"securitySchemes":{"token":{"type":"http","scheme":"bearer"},"basic":{"type":"http","scheme":"basic"}}},
        "paths":{
            "/exchange":{"post":{"operationId":"exchange","requestBody":{"required":true,"content":{
                "application/json":{"schema":{"type":"object","properties":{"name":{"type":"string"}},"required":["name"],"additionalProperties":false}},
                "multipart/form-data":{"schema":{"type":"object","required":["files","note"],"properties":{"files":{"type":"array","minItems":1,"maxItems":2,"items":{}},"note":{"type":"string"}},"additionalProperties":false}}
            }},"responses":{
                "2XX":{"description":"success","headers":{"X-Flag":{"required":true,"schema":{"type":"boolean"}}},"links":{"next":{"operationId":"events","parameters":{"literal":{"source":"instance source","description":"instance description","schema":{"type":"integer"}}}}},"content":{"application/json":{"schema":{"type":"object","properties":{"out":{"type":"string"}}}},"application/octet-stream":{}}},
                "default":{"description":"fallback","content":{"text/plain":{"schema":{"type":"string"}}}}
            }}},
            "/events":{"get":{"operationId":"events","responses":{"200":{"description":"events","content":{"text/event-stream":{"itemSchema":{"type":"object","required":["data"],"properties":{"data":{"type":"string"}},"additionalProperties":false}}}}}}}
        }
    })
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn rich_native_capture_contains_real_media_parts_headers_streams_and_signatures() {
    let fixture = Fixture::new(&document());
    let snapshot = compatibility::snapshot(fixture.load(), &selection(), &[target()]).unwrap();
    let native = &snapshot.native[0];
    assert_eq!(native.status, PlanStatus::Planned, "{:?}", native.findings);
    let exchange = native
        .operations
        .iter()
        .find(|op| op.operation_id == "exchange")
        .unwrap();
    assert_eq!(exchange.descriptor["body"]["taggedMedia"], true);
    assert_eq!(
        exchange.descriptor["body"]["media"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(
        exchange.descriptor["body"]["type"]
            .as_str()
            .unwrap()
            .contains("Part<Uint8Array")
    );
    assert_eq!(exchange.descriptor["responses"][0]["status"], "2XX");
    assert_eq!(
        exchange.descriptor["responses"][0]["headers"][0]["required"],
        true
    );
    assert_eq!(exchange.descriptor["responses"][1]["status"], "default");
    let events = native
        .operations
        .iter()
        .find(|op| op.operation_id == "events")
        .unwrap();
    assert_eq!(events.descriptor["signature"]["inputOptional"], true);
    assert!(
        events.descriptor["successType"]
            .as_str()
            .unwrap()
            .contains("AsyncIterable<")
    );
    assert_eq!(events.descriptor["client"]["optionsRequired"], true);
    assert!(
        events.descriptor["client"]["credentials"]["basic"]
            .as_array()
            .unwrap()
            .contains(&json!("Credential<BasicCredential>"))
    );
    assert!(
        native
            .runtime
            .fingerprinted_assets
            .iter()
            .any(|path| path == "typescript/http/runtime-protocol.ts")
    );
    assert!(
        native
            .runtime
            .fingerprinted_assets
            .iter()
            .any(|path| path == "typescript/http/protocol_emit.rs")
    );
    assert!(
        native
            .runtime
            .fingerprinted_assets
            .iter()
            .any(|path| path == "http_protocol/parameters.rs")
    );
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn relocation_and_prose_do_not_change_the_native_protocol_interface() {
    let before = Fixture::new(&document());
    let mut changed = document();
    changed["info"]["description"] = json!("Longer docs shift every following byte span.");
    changed["paths"]["/exchange"]["post"]["description"] =
        json!("Native first request documentation.");
    changed["paths"]["/exchange"]["post"]["responses"]["2XX"]["description"] =
        json!("Improved response prose");
    changed["components"]["securitySchemes"]["token"]["description"] =
        json!("Caller credential docs");
    changed["servers"][0]["description"] = json!("Server docs");
    let after = Fixture::new(&changed);
    let report =
        compatibility::compare(before.load(), after.load(), &selection(), &[target()]).unwrap();
    let native = &report.native[0];
    let before = native.before.as_ref().unwrap();
    let after = native.after.as_ref().unwrap();
    assert_eq!(before.status, PlanStatus::Planned, "{:?}", before.findings);
    assert_eq!(after.status, PlanStatus::Planned, "{:?}", after.findings);
    assert_ne!(
        before.operations[0].source.document,
        after.operations[0].source.document
    );
    assert_eq!(
        before.operations[0].descriptor,
        after.operations[0].descriptor
    );
    assert!(native.changes.is_empty(), "{:?}", native.changes);
}

#[ignore = "requires npm/tsc and node on the test host"]
#[test]
fn native_types_presence_credentials_statuses_and_literal_data_mutations_are_detected() {
    for mutation in [
        "part-type",
        "header-type",
        "stream-item",
        "required-input",
        "credential",
        "status",
        "link-literal",
    ] {
        let mut changed = document();
        let fixture = Fixture::new(&changed);
        let before = fixture.load();
        match mutation {
            "part-type" => {
                changed["paths"]["/exchange"]["post"]["requestBody"]["content"]["multipart/form-data"]
                    ["schema"]["properties"]["files"]["items"] = json!({"type":"string"})
            }
            "header-type" => {
                changed["paths"]["/exchange"]["post"]["responses"]["2XX"]["headers"]["X-Flag"]["schema"]
                    ["type"] = json!("string")
            }
            "stream-item" => {
                changed["paths"]["/events"]["get"]["responses"]["200"]["content"]["text/event-stream"]
                    ["itemSchema"]["properties"]["id"] = json!({"type":"string"})
            }
            "required-input" => {
                changed["paths"]["/events"]["get"]["parameters"] = json!([{"name":"cursor","in":"query","required":true,"schema":{"type":"string"}}])
            }
            "credential" => {
                changed["components"]["securitySchemes"]["token"]["scheme"] = json!("basic")
            }
            "status" => {
                let response = changed["paths"]["/exchange"]["post"]["responses"]
                    .as_object_mut()
                    .unwrap()
                    .remove("2XX")
                    .unwrap();
                changed["paths"]["/exchange"]["post"]["responses"]["201"] = response;
            }
            "link-literal" => {
                changed["paths"]["/exchange"]["post"]["responses"]["2XX"]["links"]["next"]["parameters"]
                    ["literal"]["description"] = json!("changed instance data, not source prose")
            }
            _ => unreachable!(),
        }
        fixture.write(&changed);
        let report =
            compatibility::compare(before, fixture.load(), &selection(), &[target()]).unwrap();
        let native = &report.native[0];
        assert_eq!(
            native.before.as_ref().unwrap().status,
            PlanStatus::Planned,
            "{mutation}: {:?}",
            native.before.as_ref().unwrap().findings
        );
        assert_eq!(
            native.after.as_ref().unwrap().status,
            PlanStatus::Planned,
            "{mutation}: {:?}",
            native.after.as_ref().unwrap().findings
        );
        assert!(
            !native.changes.is_empty(),
            "{mutation}: native change was lost"
        );
    }
}
