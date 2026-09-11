//! Model capabilities are independent of the portable validator envelope.

use serde_json::{Value, json};
use std::{process::Command, sync::Arc};
use suspect_codegen::{
    backend::{self, Backend, TargetConfig},
    go_http,
};
use suspect_ir::contract::Contract;
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn contract(schema: Value) -> Arc<Contract> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("api.json");
    let document = json!({
        "openapi": "3.2.0",
        "info": {"title": "Independent model admission", "version": "1"},
        "servers": [{"url": "https://example.test"}],
        "components": {"schemas": {"Root": schema}},
        "paths": {"/value": {"get": {
            "operationId": "getValue",
            "responses": {"200": {"description": "Value", "content": {
                "application/json": {"schema": {"$ref": "#/components/schemas/Root"}}
            }}}
        }}}
    });
    std::fs::write(&path, document.to_string()).unwrap();
    let workspace = Arc::new(
        WorkspaceBuilder::new()
            .root(directory.path())
            .build()
            .unwrap(),
    );
    Arc::new(Contract::from_workspace(&workspace, &Uri::from_path(&path).unwrap()).unwrap())
}

fn plan(schema: Value) -> Result<go_http::HttpPlan, Vec<go_http::HttpDiagnostic>> {
    let contract = contract(schema);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    go_http::plan_http(contract, &selected, Default::default())
}

fn intersection() -> Value {
    json!({"allOf": [
        {"type": "object", "required": ["name"], "properties": {"name": {"type": "string", "minLength": 1}}},
        {"type": "object", "required": ["active"], "properties": {"active": {"type": "boolean"}}}
    ]})
}

#[test]
fn ordinary_object_intersections_are_admitted_without_unrelated_applicators() {
    assert_native_admission(intersection());
}

#[test]
fn nested_intersections_and_alias_refinements_keep_checked_carriers() {
    assert_native_admission(json!({"allOf":[intersection(), {"type":"object","properties":{}}]}));
    assert_native_admission(json!({"allOf":[{"type":"string"},{"minLength":1}]}));
}

fn assert_native_admission(schema: Value) {
    let contract = contract(schema);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let packages = [
        ("typescript-http", "@example/sdk", None),
        ("python-http", "generated_sdk", Some("generated_sdk")),
        ("go-http", "example.com/sdk", None),
        ("rust-http", "generated_sdk", None),
        ("swift-http", "GeneratedSDK", Some("GeneratedSDK")),
        ("java-http", "example:generated-sdk", Some("example.sdk")),
        ("csharp-http", "Generated.SDK", Some("Generated.SDK")),
        ("kotlin-http", "example:generated-sdk", Some("example.sdk")),
        ("ruby-http", "generated_sdk", Some("GeneratedSDK")),
        ("php-http", "example/generated-sdk", Some("GeneratedSDK")),
        ("dart-http", "generated_sdk", None),
        ("cpp-http", "generated_sdk", Some("generated_sdk")),
    ];
    for (name, package, import) in packages {
        let Some(&backend) = Backend::ALL.iter().find(|backend| backend.name() == name) else {
            continue;
        };
        let config = TargetConfig {
            backend,
            package_name: package.into(),
            package_version: "0.1.0".into(),
            import_name: import.map(str::to_owned),
        };
        let result = backend::generate(contract.clone(), &selected, &config);
        assert!(
            result.is_ok(),
            "{name}: ordinary allOf must reach a checked representation: {result:?}"
        );
        assert!(!result.unwrap().is_empty());
    }
}

#[test]
fn numeric_enums_are_admitted_without_unrelated_applicators() {
    let result = plan(json!({"type": "integer", "enum": [0, -1, -2, -3, -5, -10]}));
    assert!(
        result.is_ok(),
        "finite numeric enum must retain its source codec: {result:?}"
    );
}

#[test]
fn native_examples_select_refined_integer_over_unconstrained_members() {
    let contract = contract(json!({"allOf":[
        {"type":"object","properties":{"name":{"type":"string"}}},
        {"type":"object","properties":{"version":{"type":"integer"}}}
    ],"example":{"name":"sdk","version":1}}));
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let files = backend::generate(
        contract,
        &selected,
        &TargetConfig {
            backend: Backend::TypescriptHttp,
            package_name: "@example/sdk".into(),
            package_version: "0.1.0".into(),
            import_name: None,
        },
    )
    .unwrap();
    let examples = &files
        .iter()
        .find(|file| file.path == "typescript/examples/validated.ts")
        .unwrap()
        .content;
    assert!(
        examples.contains("\"version\": 1n"),
        "source example must satisfy the intersection's bigint field: {examples}"
    );
    assert!(!examples.contains("\"version\": JsonNumber.parse"));
}

#[test]
fn native_examples_resolve_nullable_object_carriers_inside_intersections() {
    let schema = json!({"allOf":[
        {"type":"object","properties":{"inner":{"anyOf":[
            {"type":"object","required":["version"],"properties":{"version":{"type":"integer"}}},
            {"type":"null"}
        ]}}},
        {"type":"object","properties":{}}
    ],"example":{"inner":{"version":1}}});
    let contract = contract(schema);
    let selected = contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect::<Vec<_>>();
    let files = backend::generate(
        contract,
        &selected,
        &TargetConfig {
            backend: Backend::TypescriptHttp,
            package_name: "@example/sdk".into(),
            package_version: "0.1.0".into(),
            import_name: None,
        },
    )
    .unwrap();
    let examples = &files
        .iter()
        .find(|file| file.path == "typescript/examples/validated.ts")
        .unwrap()
        .content;
    assert!(
        examples.contains("\"version\": 1n"),
        "nested nullable carrier lost its integer representation: {examples}"
    );
    assert!(!examples.contains("\"version\": JsonNumber.parse"));
}

#[test]
#[ignore = "requires native Go toolchain"]
fn native_go_checked_carriers_enforce_composition_and_exact_numeric_membership() {
    let cases = [
        (
            intersection(),
            r#"
func TestComposition(t *testing.T) {
 good := []byte(`{"name":"Ada","active":false,"extra":100.50000000000000001}`)
 value, err := Codecs.Root.Decode(good); if err != nil { t.Fatal(err) }
 encoded, err := Codecs.Root.Encode(value); if err != nil { t.Fatal(err) }
 if !bytes.Contains(encoded, []byte(`100.50000000000000001`)) { t.Fatal(string(encoded)) }
 for _, bad := range []string{`{"name":"Ada"}`, `{"name":"","active":true}`, `{"name":"Ada","active":"false"}`} {
  if _, err := Codecs.Root.Decode([]byte(bad)); err == nil { t.Fatal("accepted invalid composition", bad) }
 }
 if _, err := Codecs.Root.Encode(map[string]Value{"name":"Ada"}); err == nil { t.Fatal("encode lost required field") }
 if _, err := Codecs.Root.Encode(map[string]Value{"name":"", "active":true}); err == nil { t.Fatal("encode lost minLength") }
}
"#,
        ),
        (
            json!({"type":"integer","enum":[0,-1,-2,-3,-5,-10]}),
            r#"
func TestNumericEnum(t *testing.T) {
 value, err := Codecs.Root.Decode([]byte(`-1e0`)); if err != nil { t.Fatal(err) }
 encoded, err := Codecs.Root.Encode(value); if err != nil || !bytes.Equal(encoded, []byte(`-1e0`)) { t.Fatal(string(encoded), err) }
 for _, bad := range []string{`-4`, `0.5`, `"-1"`, `null`} {
  if _, err := Codecs.Root.Decode([]byte(bad)); err == nil { t.Fatal("accepted invalid enum", bad) }
 }
 if _, err := Codecs.Root.Encode("-1"); err == nil { t.Fatal("encode accepted wrong JSON kind") }
}
"#,
        ),
    ];
    for (schema, source) in cases {
        let plan = plan(schema).unwrap();
        assert_eq!(
            plan.codecs().validation_program().version,
            suspect_schema::OwnedProgram::V1_VERSION
        );
        let directory = tempfile::tempdir().unwrap();
        suspect_codegen::write_files(&plan.render(), directory.path()).unwrap();
        let root = directory.path().join("go");
        std::fs::write(
            root.join("admission_test.go"),
            format!("package sdk\nimport (\"bytes\"; \"testing\")\n{source}"),
        )
        .unwrap();
        let output = Command::new("go")
            .args(["test", "./..."])
            .current_dir(&root)
            .env("GOWORK", "off")
            .env(
                "GOTOOLCHAIN",
                std::env::var_os("SUSPECT_GO_TOOLCHAIN").unwrap_or_else(|| "local".into()),
            )
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
