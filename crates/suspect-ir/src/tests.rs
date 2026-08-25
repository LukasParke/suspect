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
