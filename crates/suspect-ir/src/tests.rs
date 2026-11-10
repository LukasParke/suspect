use std::sync::Arc;

use suspect_ref::WorkspaceBuilder;
use suspect_source::Uri;

use crate::{IrSpec, Method, OpSelector, ParamIn};

const SPEC: &str = r#"
openapi: 3.1.0
info:
  title: Pets
  version: "2.1"
servers:
  - url: https://api.example.com/v1
paths:
  /pets:
    get:
      operationId: listPets
      tags: [pets]
      parameters:
        - name: limit
          in: query
          schema: { type: integer }
      responses:
        '200':
          description: page of pets
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/PetList'
        default:
          description: error
    post:
      operationId: createPet
      requestBody:
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/Pet'
      responses:
        '201': { description: created }
  /pets/{petId}:
    parameters:
      - name: petId
        in: path
        required: true
        schema: { type: string }
    get:
      operationId: showPetById
      responses:
        '200':
          description: one pet
          content:
            application/json:
              schema:
                $ref: 'https://example.com/other.yaml#/components/schemas/Pet'
components:
  schemas:
    Pet:
      type: object
      required: [name]
      properties:
        name: { type: string }
        tag: { type: string }
        friend:
          $ref: '#/components/schemas/Pet'
    PetList:
      type: array
      items:
        $ref: '#/components/schemas/Pet'
"#;

fn ws_with(dir: &std::path::Path) -> Arc<suspect_ref::Workspace> {
    std::fs::write(dir.join("spec.yaml"), SPEC).unwrap();
    let ws = WorkspaceBuilder::new().root(dir).build().unwrap();
    let _ = ws.load_all("spec.yaml"); // remote-denied external refs are fine
    Arc::new(ws)
}

#[test]
fn indexes_operations_and_schemas() {
    let dir = tempfile::tempdir().unwrap();
    let ws = ws_with(dir.path());
    let uri = Uri::from_path(&dir.path().join("spec.yaml")).unwrap();
    let ir = IrSpec::from_workspace(&ws, &uri).expect("oas spec");

    assert_eq!(ir.title, "Pets");
    assert_eq!(ir.version, "2.1");
    assert_eq!(ir.servers, vec!["https://api.example.com/v1"]);
    assert_eq!(ir.operations.len(), 3);

    // By-id lookups.
    let list = ir.operation(OpSelector::Id("listPets")).unwrap();
    assert_eq!(list.method, Method::Get);
    assert_eq!(list.path, "/pets");
    assert_eq!(list.tags, vec!["pets"]);

    // By method+path lookup.
    let shown = ir
        .operation(OpSelector::MethodPath(Method::Get, "/pets/{petId}"))
        .unwrap();
    assert_eq!(shown.id.as_deref(), Some("showPetById"));
    // Path-item parameter merged in.
    let pet_id = shown
        .parameters
        .iter()
        .find(|p| p.name == "petId")
        .expect("path parameter");
    assert_eq!(pet_id.location, ParamIn::Path);
    assert!(pet_id.required);

    // Request body resolution.
    let create = ir.operation(OpSelector::Id("createPet")).unwrap();
    assert_eq!(create.body_schema.as_deref(), Some("Pet"));

    // Response schema resolution + ordering (200 before default).
    let responses = &list.responses;
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0].status, Some(200));
    assert_eq!(responses[0].schema.as_deref(), Some("PetList"));
    assert_eq!(responses[1].status, None);

    // Schemas and dependency edges.
    assert!(ir.schema("Pet").is_some());
    assert_eq!(
        ir.schema_edges["PetList"],
        vec!["Pet".to_owned()],
        "array item refs become edges"
    );
    assert!(
        ir.schema_edges["Pet"].contains(&"Pet".to_owned()),
        "self-referencing edge recorded"
    );
}

#[test]
fn external_refs_stay_unresolved() {
    let dir = tempfile::tempdir().unwrap();
    let ws = ws_with(dir.path());
    let uri = Uri::from_path(&dir.path().join("spec.yaml")).unwrap();
    let ir = IrSpec::from_workspace(&ws, &uri).unwrap();
    let shown = ir
        .operation(OpSelector::Id("showPetById"))
        .expect("operation exists");
    assert!(
        shown.responses[0].schema.is_none(),
        "external-file ref must not resolve to a local name"
    );
}

#[test]
fn non_oas_document_errors() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.yaml"), "arazzo: 1.0.0\n").unwrap();
    let ws = WorkspaceBuilder::new().root(dir.path()).build().unwrap();
    ws.load_all("a.yaml").unwrap();
    let uri = Uri::from_path(&dir.path().join("a.yaml")).unwrap();
    let err = IrSpec::from_workspace(&Arc::new(ws), &uri).unwrap_err();
    assert_eq!(err, "not an OpenAPI 3.x document");
}

#[test]
fn missing_document_errors() {
    let dir = tempfile::tempdir().unwrap();
    let ws = WorkspaceBuilder::new().root(dir.path()).build().unwrap();
    let uri = Uri::from_path(&dir.path().join("nope.yaml")).unwrap();
    let err = IrSpec::from_workspace(&Arc::new(ws), &uri).unwrap_err();
    assert!(err.contains("not loaded"), "{err}");
}

// ---------------------------------------------------------------------------
// Fast-path construction (IrSpec::from_file / try_parse_fast)
// ---------------------------------------------------------------------------

use std::path::Path;
use std::time::Instant;

/// Generates a deterministic, fast-subset-compatible OpenAPI document of
/// roughly `want_lines` lines exercising every scalar shape and structure
/// the fast reader supports.
fn generate_spec(want_lines: usize) -> String {
    let mut out = String::with_capacity(want_lines * 48);
    out.push_str(
        "openapi: 3.1.0\n\
         info:\n\
         \x20 title: Generated\n\
         \x20 version: \"2.1\"\n\
         servers:\n\
         - url: https://api.example.com/v1\n\
         - url: https://backup.example.com\n\
         components:\n\
         \x20 schemas:\n",
    );
    let mut lines = 9usize;
    let mut i = 0usize;
    while lines < want_lines {
        out.push_str(&format!(
            concat!(
                "    Schema{i}:\n",
                "      type: object\n",
                "      description: >-\n",
                "        Folded description {i} with\n",
                "        several folded words & symbols [ok]\n",
                "      summary: 'single {i} it''s quoted'\n",
                "      note: \"double \\\"{i}\\\" \\u00e9scaped\"\n",
                "      required:\n",
                "      - name\n",
                "      properties:\n",
                "        name:\n",
                "          type: string\n",
                "        count:\n",
                "          type: integer\n",
                "          minimum: 0x10\n",
                "          maximum: 0o17\n",
                "        ratio:\n",
                "          type: number\n",
                "          example: -3.5e2\n",
                "        flag:\n",
                "          type: boolean\n",
                "          default: TRUE\n",
                "        empty:\n",
                "        nothing: ~\n",
                "        literal: |\n",
                "          line one\n",
                "          line two\n",
                "        tags: []\n",
                "        meta: {{}}\n",
                "        friend:\n",
                "          $ref: '#/components/schemas/Schema{next}'\n"
            ),
            i = i,
            next = (i + 1) % 64,
        ));
        // One percent-encoded ref target every 16 schemas.
        if i.is_multiple_of(16) {
            out.push_str("        weird:\n          $ref: '#/components/schemas/Special%20Name'\n");
        }
        lines += 33;
        i += 1;
    }
    out.push_str("    Special%20Name:\n      type: string\n");

    out.push_str("paths:\n");
    for p in 0..(want_lines / 12).max(4) {
        out.push_str(&format!(
            concat!(
                "  /resource{p}/{{id}}:\n",
                "    parameters:\n",
                "    - name: id\n",
                "      in: path\n",
                "      required: true\n",
                "      schema:\n",
                "        type: string\n",
                "    get:\n",
                "      operationId: get{p}\n",
                "      summary: 'Fetch resource {p}'\n",
                "      tags: [group{g}, shared]\n",
                "      deprecated: {dep}\n",
                "      parameters:\n",
                "      - name: limit\n",
                "        in: query\n",
                "        schema:\n",
                "          type: integer\n",
                "      requestBody:\n",
                "        content:\n",
                "          application/json:\n",
                "            schema:\n",
                "              $ref: '#/components/schemas/Schema{n1}'\n",
                "      responses:\n",
                "        default:\n",
                "          description: error\n",
                "        '404':\n",
                "          description: missing\n",
                "        '200':\n",
                "          description: ok\n",
                "          content:\n",
                "            application/json:\n",
                "              schema:\n",
                "                $ref: '#/components/schemas/Schema{n2}'\n",
                "    post:\n",
                "      operationId: post{p}\n",
                "      responses:\n",
                "        '201':\n",
                "          description: created\n"
            ),
            p = p,
            g = p % 7,
            dep = if p % 5 == 0 { "true" } else { "false" },
            n1 = p % 64,
            n2 = (p * 3 + 1) % 64,
        ));
        lines += 35;
        if lines >= want_lines {
            break;
        }
    }
    out
}

fn ir_via_workspace(dir: &Path, file: &str) -> IrSpec {
    let ws = WorkspaceBuilder::new().root(dir).build().unwrap();
    ws.load_all(file).unwrap();
    let uri = Uri::from_path(&dir.join(file)).unwrap();
    IrSpec::from_workspace(&Arc::new(ws), &uri).expect("oas spec")
}

#[test]
fn fast_path_matches_workspace_semantics() {
    let dir = tempfile::tempdir().unwrap();
    let spec_yaml = generate_spec(2000);
    assert!(spec_yaml.lines().count() >= 1900, "medium-sized doc");
    std::fs::write(dir.path().join("spec.yaml"), &spec_yaml).unwrap();

    let via_ws = ir_via_workspace(dir.path(), "spec.yaml");
    let via_fast = IrSpec::from_file(&dir.path().join("spec.yaml")).expect("fast parse");

    // Public-field equality (index maps are #[serde(skip)]).
    let left = serde_json::to_value(&via_fast).unwrap();
    let right = serde_json::to_value(&via_ws).unwrap();
    if left != right {
        // Surface the first divergence for debugging.
        let l = serde_json::to_string_pretty(&left).unwrap();
        let r = serde_json::to_string_pretty(&right).unwrap();
        for (ll, rl) in l.lines().zip(r.lines()) {
            if ll != rl {
                panic!("first JSON divergence:\n fast:     {ll}\n workspace: {rl}");
            }
        }
        panic!("JSON length differs: {} vs {}", l.len(), r.len());
    }

    // Index maps agree for every operation id and schema name.
    assert_eq!(via_fast.by_operation_id, via_ws.by_operation_id);
    assert_eq!(via_fast.by_method_path, via_ws.by_method_path);
    assert_eq!(via_fast.schema_index, via_ws.schema_index);
    assert!(!via_fast.operations.is_empty(), "generated operations");
    for id in via_fast.by_operation_id.keys() {
        let a = via_fast.operation(OpSelector::Id(id)).unwrap();
        let b = via_ws.operation(OpSelector::Id(id)).unwrap();
        assert_eq!(a.path, b.path, "{id}");
        assert_eq!(a.method, b.method, "{id}");
    }
    for name in via_fast.schema_index.keys() {
        assert_eq!(
            via_fast.schema(name).map(|s| &s.json),
            via_ws.schema(name).map(|s| &s.json),
            "{name}"
        );
    }
}

#[test]
fn fast_path_rejects_exotic_yaml_and_falls_back() {
    let dir = tempfile::tempdir().unwrap();
    let exotic = "\
openapi: 3.1.0
info:
  title: &title Anchors
  version: \"1.0\"
paths:
  /x:
    get:
      operationId: anchored
      responses:
        '200':
          description: *title
components:
  schemas:
    A:
      type: object
      properties:
        alias: *title
";
    std::fs::write(dir.path().join("spec.yaml"), exotic).unwrap();
    let via_ws = ir_via_workspace(dir.path(), "spec.yaml");
    let via_file = IrSpec::from_file(&dir.path().join("spec.yaml")).unwrap();
    assert_eq!(
        serde_json::to_value(&via_file).unwrap(),
        serde_json::to_value(&via_ws).unwrap(),
        "fallback must reproduce workspace semantics for anchors/aliases"
    );
}

#[test]
fn non_oas_document_errors_from_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.yaml"), "arazzo: 1.0.0\n").unwrap();
    let err = IrSpec::from_file(&dir.path().join("a.yaml")).unwrap_err();
    assert_eq!(err, "not an OpenAPI 3.x document");
}

/// Stripe-like volume: ~4 MB synthetic document built from the same
/// generator. Asserts the fast path handles it well under real targets;
/// the assertion only applies to optimized builds (debug parsing is an
/// order of magnitude slower and not representative).
#[test]
fn from_file_handles_stripe_like_volume_quickly() {
    let dir = tempfile::tempdir().unwrap();
    let mut spec_yaml = generate_spec(400);
    while spec_yaml.len() < 4_000_000 {
        spec_yaml.push_str(&generate_spec(1200));
    }
    std::fs::write(dir.path().join("spec.yaml"), &spec_yaml).unwrap();
    assert!(spec_yaml.len() >= 4_000_000);

    // Warm-up (page cache, lazy statics), then time a single call.
    let warm = Instant::now();
    IrSpec::from_file(&dir.path().join("spec.yaml")).unwrap();
    println!("warm-up from_file: {:?}", warm.elapsed());

    let started = Instant::now();
    let ir = IrSpec::from_file(&dir.path().join("spec.yaml")).unwrap();
    let elapsed = started.elapsed();
    println!("timed from_file ({} B): {elapsed:?}", spec_yaml.len());
    assert!(!ir.operations.is_empty());
    if !cfg!(debug_assertions) {
        // Order-of-magnitude smoke bound: release builds parse this in
        // ~10ms locally; shared CI runners jitter past 25ms, so budget 100ms.
        assert!(elapsed.as_millis() < 100, "from_file took {elapsed:?}");
    }
}

/// Timing harness for the committed stripe corpus: prints the median
/// `from_file` wall time over 7 runs; asserts only in optimized builds.
#[test]
fn stripe_corpus_timing_report() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/stripe.yaml");
    if !path.exists() {
        eprintln!("skipping: stripe corpus not present");
        return;
    }
    // Correctness cross-check: fast path must match the workspace path.
    let via_fast = IrSpec::from_file(&path).unwrap();
    let via_ws = ir_via_workspace(path.parent().unwrap(), "stripe.yaml");
    assert_eq!(
        serde_json::to_value(&via_fast).unwrap(),
        serde_json::to_value(&via_ws).unwrap(),
        "fast path diverges from workspace path on corpus/stripe.yaml"
    );

    let mut samples = Vec::new();
    for _ in 0..7 {
        let started = Instant::now();
        IrSpec::from_file(&path).unwrap();
        samples.push(started.elapsed());
    }
    samples.sort();
    let median = samples[3];
    println!("stripe from_file samples: {samples:?}");
    println!(
        "stripe from_file median: {median:?} ({:?} ms)",
        median.as_millis()
    );
    if !cfg!(debug_assertions) {
        assert!(median.as_millis() < 25, "stripe median {median:?}");
    }
}

// ---------------------------------------------------------------------
// Contract surface: what conformance checking needs from the snapshot.

const SURFACE: &str = r#"
openapi: 3.0.3
info:
  title: Surface
  version: "1"
paths:
  /pets/{petId}:
    get:
      operationId: showPet
      parameters:
        - $ref: '#/components/parameters/PetId'
      responses:
        default:
          description: anything else
        4XX:
          description: client error
          content:
            application/problem+json:
              schema:
                type: object
                required: [title]
        '404':
          $ref: '#/components/responses/NotFound'
        '200':
          description: one pet
          headers:
            X-Rate-Limit:
              required: true
              schema: { type: integer }
            X-Trace:
              $ref: '#/components/headers/Trace'
          content:
            application/json:
              schema: { $ref: '#/components/schemas/Pet' }
            application/xml:
              schema: { type: string }
    put:
      operationId: replacePet
      requestBody:
        $ref: '#/components/requestBodies/PetBody'
      responses:
        '204': { description: replaced }
    patch:
      operationId: patchPet
      requestBody:
        content:
          application/merge-patch+json:
            schema: { $ref: '#/components/schemas/Pet' }
      responses:
        '200':
          description: patched
          content:
            application/vnd.api+json:
              schema: { $ref: '#/components/schemas/Pet' }
components:
  parameters:
    PetId:
      name: petId
      in: path
      required: true
      schema: { type: string }
  headers:
    Trace:
      schema: { type: string }
  requestBodies:
    PetBody:
      content:
        application/json:
          schema: { $ref: '#/components/schemas/Pet' }
  responses:
    NotFound:
      description: no such pet
      content:
        application/problem+json:
          schema: { $ref: '#/components/schemas/Problem' }
  schemas:
    Pet:
      type: object
      required: [name]
      properties:
        name: { type: string }
    Problem:
      type: object
      required: [title]
      properties:
        title: { type: string }
"#;

/// The surface fixture through both construction paths, which must agree.
fn surface() -> IrSpec {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("surface.yaml"), SURFACE).unwrap();
    let via_file = IrSpec::from_file(&dir.path().join("surface.yaml")).expect("oas spec");
    let via_ws = ir_via_workspace(dir.path(), "surface.yaml");
    assert_eq!(
        serde_json::to_value(&via_file).unwrap(),
        serde_json::to_value(&via_ws).unwrap(),
        "both construction paths build the same snapshot"
    );
    via_file
}

fn response<'s>(
    spec: &'s IrSpec,
    id: &str,
    pick: impl Fn(&crate::IrResponse) -> bool,
) -> &'s crate::IrResponse {
    spec.operation(OpSelector::Id(id))
        .unwrap()
        .responses
        .iter()
        .find(|r| pick(r))
        .unwrap_or_else(|| panic!("no such response on {id}"))
}

#[test]
fn records_the_openapi_version() {
    assert_eq!(surface().openapi, "3.0.3");
}

#[test]
fn response_refs_resolve_to_the_component_response() {
    let spec = surface();
    let not_found = response(&spec, "showPet", |r| r.status == Some(404));
    assert_eq!(not_found.description.as_deref(), Some("no such pet"));
    assert_eq!(not_found.content.len(), 1);
    assert_eq!(not_found.content[0].media_type, "application/problem+json");
    assert_eq!(not_found.content[0].schema.as_deref(), Some("Problem"));
}

#[test]
fn status_ranges_are_not_the_default_response() {
    let spec = surface();
    let range = response(&spec, "showPet", |r| r.range == Some(4));
    assert_eq!(range.status, None);
    assert!(!range.is_default(), "4XX is a range, not `default`");
    let default = response(&spec, "showPet", crate::IrResponse::is_default);
    assert_eq!(default.description.as_deref(), Some("anything else"));
    assert_eq!(default.range, None);
}

#[test]
fn responses_order_exact_then_range_then_default() {
    let spec = surface();
    let order: Vec<String> = spec
        .operation(OpSelector::Id("showPet"))
        .unwrap()
        .responses
        .iter()
        .map(crate::IrResponse::key)
        .collect();
    assert_eq!(order, ["200", "404", "4XX", "default"]);
}

#[test]
fn response_selection_prefers_exact_then_range_then_default() {
    let spec = surface();
    let op = spec.operation(OpSelector::Id("showPet")).unwrap();
    assert_eq!(
        op.response_for(200).map(crate::IrResponse::key).as_deref(),
        Some("200")
    );
    assert_eq!(
        op.response_for(404).map(crate::IrResponse::key).as_deref(),
        Some("404")
    );
    assert_eq!(
        op.response_for(422).map(crate::IrResponse::key).as_deref(),
        Some("4XX")
    );
    assert_eq!(
        op.response_for(500).map(crate::IrResponse::key).as_deref(),
        Some("default")
    );
    let put = spec.operation(OpSelector::Id("replacePet")).unwrap();
    assert!(put.response_for(200).is_none(), "only 204 is declared");
}

#[test]
fn every_declared_media_type_is_recorded() {
    let spec = surface();
    let ok = response(&spec, "showPet", |r| r.status == Some(200));
    let media: Vec<&str> = ok.content.iter().map(|m| m.media_type.as_str()).collect();
    assert_eq!(media, ["application/json", "application/xml"]);
    assert_eq!(ok.content[0].schema.as_deref(), Some("Pet"));
    // The inline XML schema is registered like any other inline schema.
    let xml = ok.content[1].schema.as_deref().expect("inline schema name");
    assert_eq!(
        spec.schema(xml).unwrap().json,
        serde_json::json!({"type": "string"})
    );
}

#[test]
fn inline_schemas_of_non_json_media_types_are_addressable() {
    let spec = surface();
    let range = response(&spec, "showPet", |r| r.range == Some(4));
    let name = range.content[0]
        .schema
        .as_deref()
        .expect("inline schema name");
    assert_eq!(
        spec.schema(name).unwrap().json,
        serde_json::json!({"type": "object", "required": ["title"]})
    );
}

#[test]
fn schema_names_the_json_compatible_media_type() {
    let spec = surface();
    // `application/vnd.api+json` is JSON: its schema is the response schema.
    let patched = response(&spec, "patchPet", |r| r.status == Some(200));
    assert_eq!(patched.schema.as_deref(), Some("Pet"));
    // And the request side follows the same rule.
    let patch = spec.operation(OpSelector::Id("patchPet")).unwrap();
    assert_eq!(patch.body_schema.as_deref(), Some("Pet"));
    // `application/json` wins over other media types when both exist.
    let ok = response(&spec, "showPet", |r| r.status == Some(200));
    assert_eq!(ok.schema.as_deref(), Some("Pet"));
}

#[test]
fn response_headers_are_recorded() {
    let spec = surface();
    let ok = response(&spec, "showPet", |r| r.status == Some(200));
    let limit = ok
        .headers
        .iter()
        .find(|h| h.name == "X-Rate-Limit")
        .unwrap();
    assert!(limit.required);
    assert_eq!(limit.schema, Some(serde_json::json!({"type": "integer"})));
    let trace = ok.headers.iter().find(|h| h.name == "X-Trace").unwrap();
    assert!(!trace.required);
    assert_eq!(trace.schema, Some(serde_json::json!({"type": "string"})));
}

#[test]
fn parameter_and_request_body_refs_resolve() {
    let spec = surface();
    let show = spec.operation(OpSelector::Id("showPet")).unwrap();
    assert_eq!(show.parameters.len(), 1);
    assert_eq!(show.parameters[0].name, "petId");
    assert_eq!(show.parameters[0].location, ParamIn::Path);
    assert!(show.parameters[0].required);
    let put = spec.operation(OpSelector::Id("replacePet")).unwrap();
    assert_eq!(put.body_schema.as_deref(), Some("Pet"));
}

#[test]
fn json_media_types_are_recognized() {
    for json in [
        "application/json",
        "application/json; charset=utf-8",
        "Application/JSON",
        "application/problem+json",
        "application/vnd.api+json;profile=x",
        "text/json",
    ] {
        assert!(crate::is_json_media_type(json), "{json}");
    }
    for other in [
        "text/plain",
        "application/xml",
        "application/jsonl",
        "*/*",
        "",
    ] {
        assert!(!crate::is_json_media_type(other), "{other}");
    }
}

// ---------------------------------------------------------------------
// Multi-file descriptions: references that leave the entry document.

/// Writes a description split across files and returns its directory.
fn multi_file() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, text: &str| {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write(
        "api.yaml",
        r#"
openapi: 3.1.0
info:
  title: Multi
  version: "1"
paths:
  /pets/{petId}:
    get:
      operationId: showPet
      parameters:
        - $ref: './params.yaml#/PetId'
      responses:
        '200':
          description: one pet
          content:
            application/json:
              schema:
                $ref: './schemas/pet.yaml'
        '404':
          $ref: './responses.yaml#/NotFound'
        '410':
          description: retired
          content:
            application/json:
              schema:
                $ref: './missing.yaml'
components:
  schemas:
    pet:
      type: string
      description: an unrelated component that already owns the name
    Tag:
      type: string
"#,
    );
    write(
        "params.yaml",
        "PetId:\n  name: petId\n  in: path\n  required: true\n  schema:\n    type: string\n",
    );
    write(
        "responses.yaml",
        "NotFound:\n  description: no such pet\n  content:\n    application/problem+json:\n      schema:\n        $ref: './schemas/problem.yaml'\n",
    );
    write(
        "schemas/pet.yaml",
        r#"
type: object
required: [id]
properties:
  id:
    type: string
  owner:
    $ref: './owner.yaml'
  friend:
    $ref: './pet.yaml'
  tag:
    $ref: '../api.yaml#/components/schemas/Tag'
"#,
    );
    write(
        "schemas/owner.yaml",
        "type: object\nrequired: [email]\nproperties:\n  email:\n    type: string\n",
    );
    write(
        "schemas/problem.yaml",
        "type: object\nrequired: [title]\nproperties:\n  title:\n    type: string\n",
    );
    dir
}

/// The component a `#/components/schemas/<name>` reference names.
fn schema_ref_name(value: &serde_json::Value) -> &str {
    value["$ref"]
        .as_str()
        .and_then(|reference| reference.strip_prefix("#/components/schemas/"))
        .unwrap_or_else(|| panic!("a local schema reference, got {value}"))
}

#[test]
fn external_references_become_local_components() {
    let dir = multi_file();
    let spec = IrSpec::from_file(&dir.path().join("api.yaml")).expect("oas spec");
    let show = spec.operation(OpSelector::Id("showPet")).unwrap();

    // The parameter declared in another file is the operation's parameter.
    assert_eq!(show.parameters.len(), 1);
    assert_eq!(show.parameters[0].name, "petId");
    assert!(show.parameters[0].required);

    // The response schema in another file is addressable by name...
    let ok = show.response_for(200).unwrap();
    assert_eq!(ok.content[0].unresolved, None);
    let pet_name = ok.content[0].schema.as_deref().expect("a schema name");
    assert_ne!(
        pet_name, "pet",
        "the entry document's own `pet` is not replaced"
    );
    let pet = &spec.schema(pet_name).unwrap().json;
    assert_eq!(pet["required"], serde_json::json!(["id"]));
    // ...its reference to itself stays a reference, to the same name...
    assert_eq!(schema_ref_name(&pet["properties"]["friend"]), pet_name);
    // ...the files it references in turn are components too...
    let owner = schema_ref_name(&pet["properties"]["owner"]);
    assert_eq!(
        spec.schema(owner).unwrap().json["required"],
        serde_json::json!(["email"])
    );
    // ...and a reference back into the entry document is simply local.
    assert_eq!(schema_ref_name(&pet["properties"]["tag"]), "Tag");
    assert_eq!(spec.schema("pet").unwrap().json["type"], "string");

    // A whole response declared in another file, with its own schema file.
    let not_found = show.response_for(404).unwrap();
    assert_eq!(not_found.description.as_deref(), Some("no such pet"));
    assert_eq!(not_found.content[0].media_type, "application/problem+json");
    let problem = not_found.content[0]
        .schema
        .as_deref()
        .expect("a schema name");
    assert_eq!(
        spec.schema(problem).unwrap().json["required"],
        serde_json::json!(["title"])
    );
}

#[test]
fn a_reference_to_a_missing_document_stays_unresolved() {
    let dir = multi_file();
    let spec = IrSpec::from_file(&dir.path().join("api.yaml")).expect("oas spec");
    let retired = spec
        .operation(OpSelector::Id("showPet"))
        .unwrap()
        .response_for(410)
        .unwrap();
    assert_eq!(retired.content[0].schema, None);
    assert_eq!(
        retired.content[0].unresolved.as_deref(),
        Some("./missing.yaml")
    );
}

#[test]
fn a_multi_file_description_is_the_same_through_both_paths() {
    let dir = multi_file();
    let via_file = IrSpec::from_file(&dir.path().join("api.yaml")).expect("oas spec");
    // Only the entry is opened up front (the fixture has a dangling
    // reference an eager closure load would reject); the documents it
    // references are loaded as the snapshot is built.
    let ws = WorkspaceBuilder::new().root(dir.path()).build().unwrap();
    let uri = Uri::from_path(&dir.path().join("api.yaml")).unwrap();
    ws.open(uri.as_str()).unwrap();
    let via_ws = IrSpec::from_workspace(&Arc::new(ws), &uri).expect("oas spec");
    assert_eq!(
        serde_json::to_value(&via_file).unwrap(),
        serde_json::to_value(&via_ws).unwrap()
    );
}
