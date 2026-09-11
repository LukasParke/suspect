//! Scalar and local-reference fidelity through the public IR loaders.

use std::path::Path;
use std::sync::Arc;

use suspect_ir::{IrSpec, OpSelector};
use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

fn load_both(path: &Path) -> [IrSpec; 2] {
    let workspace = WorkspaceBuilder::new()
        .root(path.parent().unwrap())
        .build()
        .unwrap();
    workspace
        .load_all(path.file_name().unwrap().to_str().unwrap())
        .unwrap();
    let uri = Uri::from_path(path).unwrap();
    [
        IrSpec::from_file(path).unwrap(),
        IrSpec::from_workspace(&Arc::new(workspace), &uri).unwrap(),
    ]
}

#[test]
fn exact_numeric_values_survive_json_and_yaml_loading() {
    let cases = [
        ("-9223372036854775808", "-9223372036854775808"),
        ("18446744073709551615", "18446744073709551615"),
        (
            "1234567890123456789012345678901234567890",
            "1234567890123456789012345678901234567890",
        ),
        (
            "-1234567890123456789012345678901234567890",
            "-1234567890123456789012345678901234567890",
        ),
        (
            "0.123456789012345678901234567890123456789",
            "0.123456789012345678901234567890123456789",
        ),
        ("1E400", "1e+400"),
        ("-1e-9999", "-1e-9999"),
    ];
    let values = cases.map(|(input, _)| input).join(", ");
    let json = format!(
        r#"{{"openapi":"3.1.0","info":{{"title":"Numbers","version":"1"}},"paths":{{}},"components":{{"schemas":{{"Numbers":{{"enum":[{values}]}}}}}}}}"#
    );
    let yaml = format!(
        "openapi: 3.1.0\ninfo: {{title: Numbers, version: '1'}}\npaths: {{}}\ncomponents:\n  schemas:\n    Numbers:\n      enum: [{values}]\n"
    );
    let directory = tempfile::tempdir().unwrap();
    for (file, input) in [("numbers.json", json), ("numbers.yaml", yaml)] {
        let path = directory.path().join(file);
        std::fs::write(&path, input).unwrap();
        for spec in load_both(&path) {
            let numbers = spec.schema("Numbers").unwrap().json["enum"]
                .as_array()
                .unwrap();
            for (value, (input, expected)) in numbers.iter().zip(cases) {
                assert!(value.is_number(), "{file}: {input} is not a JSON number");
                assert_eq!(value.to_string(), expected, "{file}: {input}");
            }
            assert_eq!(numbers.len(), cases.len());
        }
    }
}

#[test]
fn yaml_number_spellings_normalize_without_losing_precision() {
    let cases = [
        ("+18446744073709551615", "18446744073709551615"),
        ("00018446744073709551615", "18446744073709551615"),
        ("-0x8000000000000000", "-9223372036854775808"),
        (
            "0x100000000000000000000000000000000",
            "340282366920938463463374607431768211456",
        ),
        ("0o2000000000000000000000", "18446744073709551616"),
        (
            ".12345678901234567890123456789",
            "0.12345678901234567890123456789",
        ),
        ("1.", "1.0"),
        ("+001.23000e+400", "1.23000e+400"),
        ("-.00001e-400", "-0.00001e-400"),
    ];
    let values = cases.map(|(input, _)| input).join(", ");
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("numbers.yaml");
    std::fs::write(
        &path,
        format!(
            "openapi: 3.1.0\ninfo: {{title: Numbers, version: '1'}}\npaths: {{}}\ncomponents:\n  schemas:\n    Numbers:\n      enum: [{values}]\n"
        ),
    )
    .unwrap();
    for spec in load_both(&path) {
        let numbers = spec.schema("Numbers").unwrap().json["enum"]
            .as_array()
            .unwrap();
        for (value, (input, expected)) in numbers.iter().zip(cases) {
            assert!(value.is_number(), "{input} is not a JSON number");
            assert_eq!(value.to_string(), expected, "{input}");
        }
        assert_eq!(numbers.len(), cases.len());
    }
}

#[test]
fn quoted_strings_and_keys_decode_once_in_json_and_yaml() {
    let json = r#"{
      "openapi": "3.1.0",
      "info": {"title": "Strings", "version": "1"},
      "paths": {},
      "components": {"schemas": {"Message": {
        "description": "line\n\"quoted\"\\slash\/\u00E9\uD83D\uDE00",
        "properties": {"caf\u00e9": {"default": "it's literal \\n"}},
        "enum": ["true", "null", "18446744073709551615"]
      }}}
    }"#;
    let yaml = r#"openapi: 3.1.0
info: {title: Strings, version: '1'}
paths: {}
components:
  schemas:
    Message:
      description: "line\n\"quoted\"\\slash\/\u00E9\U0001F600"
      properties:
        "caf\u00e9": {default: 'it''s literal \n'}
      enum: ['true', "null", '18446744073709551615']
"#;
    let directory = tempfile::tempdir().unwrap();
    for (file, input) in [("strings.json", json), ("strings.yaml", yaml)] {
        let path = directory.path().join(file);
        std::fs::write(&path, input).unwrap();
        for spec in load_both(&path) {
            let schema = &spec.schema("Message").unwrap().json;
            assert_eq!(
                schema["description"], "line\n\"quoted\"\\slash/é😀",
                "{file}"
            );
            assert_eq!(
                schema["properties"]["café"]["default"], "it's literal \\n",
                "{file}"
            );
            assert_eq!(
                schema["enum"],
                serde_json::json!(["true", "null", "18446744073709551615"]),
                "{file}"
            );
        }
    }
}

#[test]
fn yaml_quoted_escapes_and_line_folding_retain_their_values() {
    let yaml = r#"openapi: 3.1.0
info: {title: Strings, version: '1'}
paths: {}
components:
  schemas:
    Message:
      description: "\0\a\b\t\n\v\f\r\e\ \/\N\_\L\P\xE9\u00E9\U0001F600"
      examples:
        - "line one
          line two

          line three"
        - "joined\
          without a gap"
        - 'it''s
          folded'
"#;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("strings.yaml");
    std::fs::write(&path, yaml).unwrap();
    for spec in load_both(&path) {
        let schema = &spec.schema("Message").unwrap().json;
        assert_eq!(
            schema["description"],
            "\0\u{7}\u{8}\t\n\u{b}\u{c}\r\u{1b} /\u{85}\u{a0}\u{2028}\u{2029}éé😀"
        );
        assert_eq!(
            schema["examples"],
            serde_json::json!([
                "line one line two\nline three",
                "joinedwithout a gap",
                "it's folded"
            ])
        );
    }
}

#[test]
fn malformed_quoted_scalars_are_rejected_instead_of_repaired() {
    let directory = tempfile::tempdir().unwrap();
    for (extension, token) in [
        ("json", r#""\uD83D""#),
        ("json", r#""\uDE00""#),
        ("json", r#""\uD83D\u0041""#),
        ("json", r#""\x41""#),
        ("json", r#""\q""#),
        ("yaml", r#""\U00110000""#),
        ("yaml", r#""\q""#),
    ] {
        let path = directory.path().join(format!("invalid.{extension}"));
        let input = if extension == "json" {
            format!(
                r#"{{"openapi":"3.1.0","info":{{"title":"Strings","version":"1"}},"paths":{{}},"components":{{"schemas":{{"Message":{{"description":{token}}}}}}}}}"#
            )
        } else {
            format!(
                "openapi: 3.1.0\ninfo: {{title: Strings, version: '1'}}\npaths: {{}}\ncomponents:\n  schemas:\n    Message:\n      description: {token}\n"
            )
        };
        std::fs::write(&path, input).unwrap();
        let workspace = WorkspaceBuilder::new()
            .root(directory.path())
            .build()
            .unwrap();
        workspace
            .load_all(path.file_name().unwrap().to_str().unwrap())
            .unwrap();
        let uri = Uri::from_path(&path).unwrap();
        for result in [
            IrSpec::from_file(&path),
            IrSpec::from_workspace(&Arc::new(workspace), &uri),
        ] {
            let error = result.expect_err(token);
            assert!(error.contains("invalid."), "{token}: {error}");
        }
    }
}

#[test]
fn local_reference_identity_preserves_unicode_and_decodes_fragment_before_pointer() {
    let cases = [
        ("café", "#/components/schemas/café"),
        ("café", "#/components/schemas/caf%C3%A9"),
        ("cats/dogs~", "#/components/schemas/cats~1dogs~0"),
        ("cats/dogs~", "#/components/schemas/cats%7E1dogs%7E0"),
        ("~1", "#/components/schemas/~01"),
        ("literal%2F", "#/components/schemas/literal%252F"),
        ("雪", "#%2Fcomponents%2Fschemas%2F%E9%9B%AA"),
    ];
    let directory = tempfile::tempdir().unwrap();
    for (name, reference) in cases {
        let name_json = serde_json::to_string(name).unwrap();
        let ref_json = serde_json::to_string(reference).unwrap();
        let path = directory.path().join("refs.yaml");
        std::fs::write(
            &path,
            format!(
                "openapi: 3.1.0\ninfo: {{title: References, version: '1'}}\npaths:\n  /value:\n    post:\n      operationId: echo\n      requestBody:\n        content:\n          application/json:\n            schema: {{$ref: {ref_json}}}\n      responses:\n        '200':\n          description: OK\n          content:\n            application/json:\n              schema: {{$ref: {ref_json}}}\ncomponents:\n  schemas:\n    {name_json}: {{type: string}}\n    Wrapper:\n      properties:\n        value: {{$ref: {ref_json}}}\n"
            ),
        )
        .unwrap();
        for spec in load_both(&path) {
            let operation = spec.operation(OpSelector::Id("echo")).unwrap();
            assert_eq!(operation.body_schema.as_deref(), Some(name), "{reference}");
            assert_eq!(
                operation.responses[0].schema.as_deref(),
                Some(name),
                "{reference}"
            );
            assert_eq!(spec.schema_edges["Wrapper"], [name], "{reference}");
            assert!(spec.schema(name).is_some(), "{reference}");
        }
    }

    // Bad escapes and pointers into a nested schema must not invent a
    // component identity or panic while inspecting UTF-8 bytes.
    for reference in [
        "#/components/schemas/%é",
        "#/components/schemas/%FF",
        "#/components/schemas/bad%2",
        "#/components/schemas/bad%GG",
        "#/components/schemas/bad~2",
        "#/components/schemas/bad~",
        "#/components/schemas/Thing/properties/id",
        "#/components/schemas/Thing%2Fproperties%2Fid",
    ] {
        let path = directory.path().join("invalid.json");
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "openapi": "3.1.0",
                "info": {"title": "References", "version": "1"},
                "paths": {"/value": {"post": {
                    "operationId": "echo",
                    "requestBody": {"content": {"application/json": {
                        "schema": {"$ref": reference}
                    }}},
                    "responses": {"200": {"description": "OK"}}
                }}},
                "components": {"schemas": {"Wrapper": {"$ref": reference}}}
            }))
            .unwrap(),
        )
        .unwrap();
        for spec in load_both(&path) {
            assert!(
                spec.operation(OpSelector::Id("echo"))
                    .unwrap()
                    .body_schema
                    .is_none(),
                "{reference}"
            );
            assert!(spec.schema_edges["Wrapper"].is_empty(), "{reference}");
            assert_eq!(spec.schema("Wrapper").unwrap().json["$ref"], reference);
        }
    }
}

#[test]
#[ignore = "set SUSPECT_SCALAR_SPEC to a real JSON OpenAPI corpus file"]
fn real_json_schema_values_match_the_source() {
    let path = std::env::var_os("SUSPECT_SCALAR_JSON_SPEC")
        .or_else(|| std::env::var_os("SUSPECT_SCALAR_SPEC"))
        .expect("SUSPECT_SCALAR_SPEC must name the JSON OpenAPI corpus");
    let path = Path::new(&path);
    assert_schema_values_match(path, path);
}

#[test]
#[ignore = "set SUSPECT_SCALAR_SPEC and SUSPECT_SCALAR_ORACLE to YAML and independent JSON oracle"]
fn real_yaml_schema_values_match_an_independent_oracle() {
    let path = std::env::var_os("SUSPECT_SCALAR_YAML_SPEC")
        .or_else(|| std::env::var_os("SUSPECT_SCALAR_SPEC"))
        .expect("SUSPECT_SCALAR_SPEC must name the YAML OpenAPI corpus");
    let oracle = std::env::var_os("SUSPECT_SCALAR_ORACLE")
        .expect("SUSPECT_SCALAR_ORACLE must name independently normalized JSON");
    assert_schema_values_match(Path::new(&path), Path::new(&oracle));
}

fn assert_schema_values_match(path: &Path, oracle: &Path) {
    let source: serde_json::Value =
        serde_json::from_slice(&std::fs::read(oracle).unwrap()).unwrap();
    let schemas = source["components"]["schemas"].as_object().unwrap();
    for spec in load_both(path) {
        assert_eq!(spec.schemas.len(), schemas.len());
        for (name, expected) in schemas {
            let actual = &spec.schema(name).unwrap().json;
            assert!(
                actual == expected,
                "schema {name:?} differs from source JSON"
            );
        }
    }
}
