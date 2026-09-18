//! Verified v1 contract for XML media and `xml` schema annotations, exercised
//! through the shared Contract/protocol planner and the TypeScript/Python
//! native backends.
//!
//! This file is the executable evidence for the XML rows of
//! `docs/SDK-OPENAPI-MATRIX.md`. The v1 contract it documents is
//! **fail-loud, never fail-silent**:
//!
//! 1. A declared XML *structure* — an object schema, any typed schema, a
//!    properties map, or a `$ref` into one, under `application/xml` or a
//!    vendor `+xml` media type — is refused before artifacts are written with
//!    a source-linked binary-policy diagnostic (`http-binary-schema-type`,
//!    `http-binary-schema-keyword` or `http-binary-legacy-marker`). No XML
//!    codec is emitted, no JSON stand-in is invented, and the refusal applies
//!    to the whole operation even when a JSON sibling declaration exists.
//! 2. XML media without a representable schema is admitted only as bounded
//!    raw bytes (`Representation::Binary`); the wire representation is bytes,
//!    never parsed or serialized XML. Full XML codec emission is a future
//!    capability.
//! 3. The `xml` schema annotation is retained as source provenance and echoed
//!    in documentation that keeps original schemas, but it never changes JSON
//!    wire names, codecs or validation programs (`xml.name` does not rename a
//!    JSON property, per the OpenAPI spec). A malformed annotation fails
//!    loudly when native codecs are compiled (`codec-schema-compilation`).
//!
//! These are intentional refusals, not gaps to fill silently.

#![cfg(feature = "http-protocol")]

use serde_json::{Value, json};
use std::sync::Arc;
use suspect_codegen::backend::{Backend, GenerationOptions, TargetConfig, generate_with_options};
use suspect_codegen::http_protocol::{
    self, ByteLimits, Diagnostic, DiagnosticKind, ProtocolPlan, Representation,
    ResponseBodyDisposition, ResponseMatchError, Severity,
};
use suspect_ir::contract::{Contract, SourceId};
use suspect_ref::{DocumentProvider, ProvidedDocument, WorkspaceBuilder};
use suspect_source::Uri;

fn contract_with_document(document: Value) -> Arc<Contract> {
    let entry = Uri::parse("https://source.xml.test/openapi.json").unwrap();
    let provider = Arc::new(
        DocumentProvider::new([ProvidedDocument::new(
            entry.clone(),
            entry.clone(),
            serde_json::to_vec(&document).unwrap(),
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
    Arc::new(Contract::from_workspace(&workspace, &entry).unwrap())
}

fn selected(contract: &Arc<Contract>) -> Vec<SourceId> {
    contract
        .operations()
        .map(|operation| operation.source().clone())
        .collect()
}

/// The shared Contract/protocol planner with a native capability profile, as
/// used by every admission test in this crate.
fn shared_plan(document: Value) -> Result<ProtocolPlan, Vec<Diagnostic>> {
    let contract = contract_with_document(document);
    http_protocol::plan(
        &contract,
        &selected(&contract),
        suspect_codegen::rust_http::native_capabilities_v3(),
    )
    .into_result()
}

fn target(backend: Backend) -> TargetConfig {
    TargetConfig {
        backend,
        package_name: match backend {
            Backend::TypescriptHttp => "@xml-fixture/http".into(),
            _ => "xml-fixture".into(),
        },
        package_version: "0.0.0".into(),
        import_name: None,
    }
}

fn backend_files(document: Value, backend: Backend) -> Vec<suspect_codegen::OutFile> {
    let contract = contract_with_document(document);
    let selected = selected(&contract);
    generate_with_options(
        contract,
        &selected,
        &target(backend),
        &GenerationOptions::default(),
    )
    .unwrap_or_else(|diagnostics| {
        panic!("{backend:?} generation should be admitted, refused with {diagnostics:?}")
    })
}

fn backend_errors(
    document: Value,
    backend: Backend,
) -> Vec<suspect_codegen::backend::BackendDiagnostic> {
    let contract = contract_with_document(document);
    let selected = selected(&contract);
    generate_with_options(
        contract,
        &selected,
        &target(backend),
        &GenerationOptions::default(),
    )
    .expect_err("generation should be refused")
}

fn refusal<'a>(diagnostics: &'a [Diagnostic], code: &str) -> &'a Diagnostic {
    let found: Vec<_> = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code() == code)
        .collect();
    assert_eq!(
        found.len(),
        1,
        "expected exactly one {code} diagnostic in {diagnostics:?}"
    );
    found[0]
}

fn pointer_ends_with<'a>(
    diagnostics: &'a [Diagnostic],
    code: &str,
    suffix: &str,
) -> &'a Diagnostic {
    let diagnostic = refusal(diagnostics, code);
    assert!(
        diagnostic.source().source().pointer().ends_with(suffix),
        "{code} pointed at {} not ...{suffix}",
        diagnostic.source().source().pointer()
    );
    diagnostic
}

/// (a) A response declared `content: {"application/xml": {schema: ...}}` with a
/// JSON-object structure is refused with a source-linked diagnostic, in the
/// shared planner and identically through the TypeScript and Python backends.
#[test]
fn xml_structured_response_is_refused_with_a_source_linked_diagnostic() {
    let document = |schema: Value| {
        json!({
            "openapi": "3.1.0", "info": {"title": "XML", "version": "1"},
            "paths": {"/pets": {"get": {"operationId": "listPets", "responses": {"200": {
                "description": "Pets", "content": {"application/xml": {"schema": schema}}
            }}}}}
        })
    };
    let object = document(json!({
        "type": "object", "properties": {"name": {"type": "string"}}
    }));
    let diagnostics = shared_plan(object.clone()).expect_err("XML structure must be refused");
    let diagnostic = pointer_ends_with(&diagnostics, "http-binary-schema-type", "/schema/type");
    assert_eq!(
        diagnostic.message(),
        "unencoded bytes are not JSON values; a JSON type constraint cannot be validated using a placeholder null"
    );
    assert_eq!(diagnostic.severity(), Severity::Error);
    assert_eq!(diagnostic.kind(), DiagnosticKind::Unsupported);

    for backend in [Backend::TypescriptHttp, Backend::PythonHttp] {
        let errors = backend_errors(object.clone(), backend);
        let found = errors
            .iter()
            .find(|error| error.code == "http-binary-schema-type")
            .unwrap_or_else(|| panic!("{backend:?} refused with {errors:?}"));
        assert_eq!(
            found.message,
            "unencoded bytes are not JSON values; a JSON type constraint cannot be validated using a placeholder null"
        );
        assert!(
            found
                .source
                .as_ref()
                .map(SourceId::pointer)
                .unwrap()
                .ends_with("/schema/type"),
            "refusal must point at the XML schema type keyword"
        );
    }

    // A `$ref` is refused at its target, with the use site linked.
    let referenced = json!({
        "openapi": "3.1.0", "info": {"title": "XML", "version": "1"},
        "components": {"schemas": {"Pet": {
            "type": "object", "properties": {"name": {"type": "string"}}
        }}},
        "paths": {"/pets": {"get": {"operationId": "listPets", "responses": {"200": {
            "description": "Pets",
            "content": {"application/xml": {"schema": {"$ref": "#/components/schemas/Pet"}}}
        }}}}}
    });
    let diagnostics =
        shared_plan(referenced).expect_err("referenced XML structure must be refused");
    let diagnostic = pointer_ends_with(
        &diagnostics,
        "http-binary-schema-type",
        "/components/schemas/Pet/type",
    );
    assert!(diagnostic.related().iter().any(|related| {
        related
            .source()
            .pointer()
            .ends_with("/content/application~1xml/schema")
    }));

    // A vendor `+xml` media type and a plain string schema are the same
    // byte-domain refusal: no JSON type can be evaluated on XML bytes.
    for (media, schema) in [
        ("application/vnd.pet+xml", json!({"type": "object"})),
        ("application/xml", json!({"type": "string"})),
    ] {
        let vendor = json!({
            "openapi": "3.1.0", "info": {"title": "XML", "version": "1"},
            "paths": {"/pets": {"get": {"operationId": "listPets", "responses": {"200": {
                "description": "Pets", "content": {media: {"schema": schema}}}
            }}}}
        });
        let diagnostics = shared_plan(vendor).expect_err("typed XML must be refused");
        pointer_ends_with(&diagnostics, "http-binary-schema-type", "/schema/type");
    }
}

/// (a) The remaining XML declaration surface also fails loudly, each at its
/// own source keyword: schema keywords without a type, and the OAS 3.1+
/// `format: binary` marker without the explicit legacy compatibility profile.
#[test]
fn xml_response_schema_keywords_and_legacy_marker_are_refused_at_their_source() {
    let properties = json!({
        "openapi": "3.1.0", "info": {"title": "XML", "version": "1"},
        "paths": {"/pets": {"get": {"operationId": "listPets", "responses": {"200": {
            "description": "Pets",
            "content": {"application/xml": {"schema": {"properties": {"name": {"type": "string"}}}}}
        }}}}}
    });
    let diagnostics = shared_plan(properties).expect_err("properties must be refused");
    let diagnostic = pointer_ends_with(
        &diagnostics,
        "http-binary-schema-keyword",
        "/schema/properties",
    );
    assert_eq!(
        diagnostic.message(),
        "this binary assertion/encoding cannot be enforced by the finite byte policy; JSON Schema evaluation on a substitute JSON value is forbidden"
    );

    let legacy = json!({
        "openapi": "3.1.0", "info": {"title": "XML", "version": "1"},
        "paths": {"/pets": {"get": {"operationId": "listPets", "responses": {"200": {
            "description": "Pets",
            "content": {"application/xml": {"schema": {"type": "string", "format": "binary"}}}}
        }}}}}
    );
    let diagnostics = shared_plan(legacy).expect_err("legacy marker must be refused");
    let diagnostic = pointer_ends_with(&diagnostics, "http-binary-legacy-marker", "/schema/format");
    assert_eq!(
        diagnostic.message(),
        "OAS 3.1+ format: binary does not change JSON Schema's string type; use an unconstrained binary schema or explicitly opt in to legacy-binary-string-v1"
    );
}

/// (a) XML media without a representable schema is an explicit bounded-bytes
/// representation, never silent XML support: the declaration is matched by
/// `application/xml` responses only, and the emitted representation is bytes.
#[test]
fn xml_response_without_declared_structure_is_bounded_bytes() {
    for (label, media) in [
        ("schemaless", json!({})),
        ("annotation-only", json!({"description": "opaque"})),
        (
            "oas30-binary",
            json!({"type": "string", "format": "binary"}),
        ),
    ] {
        let version = if label == "oas30-binary" {
            "3.0.3"
        } else {
            "3.1.0"
        };
        let document = json!({
            "openapi": version, "info": {"title": "XML", "version": "1"},
            "paths": {"/pets": {"get": {"operationId": "listPets", "responses": {"200": {
                "description": "Pets", "content": {"application/xml": {"schema": media}}
            }}}}}
        });
        let plan = shared_plan(document).unwrap_or_else(|diagnostics| {
            panic!("{label} must be admitted as bytes: {diagnostics:?}")
        });
        assert!(
            plan.diagnostics().is_empty(),
            "{label} must have no diagnostics"
        );
        let operation = &plan.operations()[0];
        let matched = operation
            .match_response(200, Some("application/xml"))
            .unwrap_or_else(|error| panic!("{label} must match its own media: {error:?}"));
        assert_eq!(
            matched.body_disposition(),
            ResponseBodyDisposition::Declared
        );
        let media_plan = matched.media().expect("matched media plan");
        assert_eq!(media_plan.media_type().declared(), "application/xml");
        match media_plan.representation() {
            Representation::Binary { bytes, .. } => {
                assert_eq!(bytes.max_bytes(), ByteLimits::default().body());
            }
            other => panic!("{label} must be a bounded byte representation, got {other:?}"),
        }
        // Without a JSON declaration, a JSON Content-Type stays undeclared
        // rather than being coerced into the XML entry.
        assert_eq!(
            operation
                .match_response(200, Some("application/json"))
                .unwrap_err(),
            ResponseMatchError::UndeclaredMediaType("application/json".to_owned())
        );
    }
}

/// (d) A request body in `application/xml` with a declared structure is
/// refused identically in the shared planner and both native backends; a 3.0
/// `string`/`binary` body is admitted only as bounded bytes.
#[test]
fn xml_request_body_is_refused_or_explicit_bytes() {
    let document = json!({
        "openapi": "3.1.0", "info": {"title": "XML", "version": "1"},
        "paths": {"/pets": {"post": {"operationId": "createPet", "requestBody": {
            "required": true,
            "content": {"application/xml": {"schema": {
                "type": "object", "properties": {"name": {"type": "string"}}
            }}}
        }, "responses": {"201": {"description": "Created"}}}}}
    });
    let diagnostics =
        shared_plan(document.clone()).expect_err("XML request structure must be refused");
    pointer_ends_with(
        &diagnostics,
        "http-binary-schema-type",
        "/requestBody/content/application~1xml/schema/type",
    );
    for backend in [Backend::TypescriptHttp, Backend::PythonHttp] {
        let errors = backend_errors(document.clone(), backend);
        assert!(
            errors
                .iter()
                .any(|error| error.code == "http-binary-schema-type"),
            "{backend:?} must refuse the XML request body, got {errors:?}"
        );
    }

    let legacy = json!({
        "openapi": "3.0.3", "info": {"title": "XML", "version": "1"},
        "paths": {"/pets": {"post": {"operationId": "createPet", "requestBody": {
            "required": true,
            "content": {"application/xml": {"schema": {"type": "string", "format": "binary"}}}
        }, "responses": {"201": {"description": "Created"}}}}}
    });
    let plan = shared_plan(legacy).expect("OAS 3.0 string/binary XML body is admitted as bytes");
    assert!(plan.diagnostics().is_empty());
    let body = plan.operations()[0].body().expect("request body planned");
    let media = &body.media()[0];
    assert_eq!(media.media_type().declared(), "application/xml");
    assert!(matches!(
        media.representation(),
        Representation::Binary { .. }
    ));
}

/// (b) `xml` annotations on a JSON response are retained as source provenance
/// and never applied to JSON wire names: the emitted models, codecs, manifests
/// and validation programs use the JSON property name, while the annotation
/// values survive only in documentation that retains original schemas.
#[test]
fn xml_annotations_do_not_rename_json_wire_properties() {
    let document = json!({
        "openapi": "3.1.0", "info": {"title": "XML", "version": "1"},
        "paths": {"/pets": {"get": {"operationId": "listPets", "responses": {"200": {
            "description": "Pets", "content": {"application/json": {"schema": {
                "type": "object",
                "xml": {"name": "PetRoot", "namespace": "urn:pets"},
                "properties": {
                    "petName": {"type": "string", "xml": {"name": "Renamed", "attribute": true, "namespace": "urn:pets"}},
                    "tags": {"type": "array", "items": {"type": "string", "xml": {"name": "tag"}}, "xml": {"wrapped": true}}
                },
                "required": ["petName"]
            }}}
        }}}}}
    });
    let plan =
        shared_plan(document.clone()).expect("xml annotations must not block a JSON response");
    assert!(
        plan.diagnostics().is_empty(),
        "a well-formed xml annotation is uninterpreted, got {:?}",
        plan.diagnostics()
    );
    let matched = plan.operations()[0]
        .match_response(200, Some("application/json"))
        .unwrap();
    assert!(matches!(
        matched.media().unwrap().representation(),
        Representation::Json { .. }
    ));

    let typescript = backend_files(document.clone(), Backend::TypescriptHttp);
    let file = |files: &[suspect_codegen::OutFile], path: &str| {
        files
            .iter()
            .find(|file| file.path == path)
            .unwrap_or_else(|| panic!("missing {path}"))
            .content
            .clone()
    };
    let models = file(&typescript, "typescript/models.ts");
    assert!(
        models.contains(r#""petName": string"#),
        "JSON wire property is emitted"
    );
    assert!(
        !models.contains("Renamed"),
        "xml.name must not rename the JSON wire property"
    );
    let validation = file(&typescript, "typescript/validation-program.ts");
    assert!(validation.contains(r#""name":"petName""#));
    assert!(!validation.contains("Renamed"));
    // The annotation is retained only in the documentation artifact that
    // deliberately keeps the original source schemas.
    let documentation = file(&typescript, "typescript/models.md");
    assert!(documentation.contains("Renamed") && documentation.contains("PetRoot"));
    let annotating: Vec<_> = typescript
        .iter()
        .filter(|file| file.content.contains("Renamed") || file.content.contains("PetRoot"))
        .map(|file| file.path.clone())
        .collect();
    assert_eq!(annotating, vec!["typescript/models.md".to_owned()]);

    let python = backend_files(document, Backend::PythonHttp);
    let python_file = |path: &str| file(&python, path);
    let models = python_file("python/src/xml_fixture/models.py");
    assert!(
        models.contains("pet_name"),
        "native field uses the JSON name as its source"
    );
    assert!(!models.contains("Renamed"));
    let codec_plan = python_file("python/src/xml_fixture/codec-plan.json");
    assert!(
        codec_plan.contains(r#""wire":"petName""#),
        "the wire name stays the JSON name"
    );
    let annotating: Vec<_> = python
        .iter()
        .filter(|file| file.content.contains("Renamed") || file.content.contains("PetRoot"))
        .map(|file| file.path.clone())
        .collect();
    assert_eq!(annotating, vec!["python/docs/api.rst".to_owned()]);
}

/// (b) A malformed `xml` annotation is not a protocol-level assertion, so the
/// shared plan still records the codec root — and native codec compilation
/// then fails loudly instead of silently dropping the declaration.
#[test]
fn malformed_xml_annotation_fails_loudly_at_native_codec_compilation() {
    let document = json!({
        "openapi": "3.1.0", "info": {"title": "XML", "version": "1"},
        "paths": {"/pets": {"get": {"operationId": "listPets", "responses": {"200": {
            "description": "Pets", "content": {"application/json": {"schema": {
                "type": "object",
                "properties": {"petName": {"type": "string", "xml": {"name": 5}}}
            }}}
        }}}}}
    });
    let plan = shared_plan(document.clone()).expect("the protocol plan itself is not refused");
    assert!(plan.diagnostics().is_empty());
    for backend in [Backend::TypescriptHttp, Backend::PythonHttp] {
        let errors = backend_errors(document.clone(), backend);
        let found = errors
            .iter()
            .find(|error| error.code == "codec-schema-compilation")
            .unwrap_or_else(|| panic!("{backend:?} must fail codec compilation, got {errors:?}"));
        assert_eq!(found.message, "Invalid: `name` must be a string");
        assert!(
            found
                .source
                .as_ref()
                .map(SourceId::pointer)
                .unwrap()
                .ends_with("/schema/properties/petName/xml/name"),
            "the failure points at the malformed annotation"
        );
    }
}

/// (c) Media negotiation: a JSON+XML response with a declared XML structure is
/// refused as a whole (fail-loud), while JSON plus schemaless XML negotiates
/// correctly — each Content-Type matches only its own declaration.
#[test]
fn mixed_json_and_xml_negotiates_or_refuses_loudly() {
    let object = json!({"type": "object", "properties": {"name": {"type": "string"}}});
    let structured = json!({
        "openapi": "3.1.0", "info": {"title": "XML", "version": "1"},
        "paths": {"/pets": {"get": {"operationId": "listPets", "responses": {"200": {
            "description": "Pets", "content": {
                "application/json": {"schema": object},
                "application/xml": {"schema": object}
            }
        }}}}}
    });
    let diagnostics =
        shared_plan(structured).expect_err("the unrepresentable XML sibling refuses the response");
    pointer_ends_with(
        &diagnostics,
        "http-binary-schema-type",
        "/content/application~1xml/schema/type",
    );

    let mixed = json!({
        "openapi": "3.1.0", "info": {"title": "XML", "version": "1"},
        "paths": {"/pets": {"get": {"operationId": "listPets", "responses": {"200": {
            "description": "Pets", "content": {
                "application/json": {"schema": object},
                "application/xml": {}
            }
        }}}}}
    });
    let plan = shared_plan(mixed).expect("JSON plus schemaless XML is admitted");
    assert!(plan.diagnostics().is_empty());
    let operation = &plan.operations()[0];
    let json = operation
        .match_response(200, Some("application/json"))
        .expect("JSON Content-Type must select the JSON declaration");
    assert_eq!(
        json.media().unwrap().media_type().declared(),
        "application/json"
    );
    assert!(matches!(
        json.media().unwrap().representation(),
        Representation::Json { .. }
    ));
    assert_eq!(json.body_disposition(), ResponseBodyDisposition::Declared);
    let xml = operation
        .match_response(200, Some("application/xml"))
        .expect("XML Content-Type must select the byte declaration");
    assert_eq!(
        xml.media().unwrap().media_type().declared(),
        "application/xml"
    );
    assert!(matches!(
        xml.media().unwrap().representation(),
        Representation::Binary { .. }
    ));
}
