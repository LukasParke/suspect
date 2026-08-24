//! End-to-end tests: STG lifting fidelity, emitter shapes, determinism,
//! drift detection.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use suspect_codegen::{
    Base, StgNode, StgType, WellKnownFormat, build_graph, emit_all, matches_disk,
};
use suspect_ir::IrSpec;
use suspect_ref::{Workspace, WorkspaceBuilder};
use suspect_source::Uri;

const FIXTURE: &str = r#"
openapi: 3.1.0
info:
  title: Codegen fixture
  version: "1.0"
paths:
  /pets:
    get:
      operationId: listPets
      summary: List pets
      parameters:
        - name: limit
          in: query
          schema: { type: integer, minimum: 1 }
      responses:
        '200':
          description: page
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/PetList'
    post:
      operationId: createPet
      requestBody:
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/PetInput'
      responses:
        '201':
          description: created
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Pet'
components:
  schemas:
    Pet:
      type: object
      required: [id, kind]
      properties:
        id:
          type: string
          format: uuid
        kind:
          $ref: '#/components/schemas/PetKind'
        email:
          type: string
          format: email
        note: { type: string }
    PetKind:
      type: string
      enum: [cat, dog]
    PetList:
      type: array
      items:
        $ref: '#/components/schemas/Pet'
    PetInput:
      allOf:
        - $ref: '#/components/schemas/Pet'
        - type: object
          properties:
            vetId:
              type: string
              pattern: '^vet_[0-9]+$'
"#;

/// Creates the fixture directory; leaked for test-process lifetime.
fn spec_dir() -> (PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let kept = dir.path().to_path_buf();
    std::mem::forget(dir);
    std::fs::write(kept.join("fixture.yaml"), FIXTURE).unwrap();
    (kept.clone(), kept.join("fixture.yaml"))
}

/// Loads a workspace + lifted IrSpec for the fixture.
struct Loaded {
    #[allow(dead_code)]
    ws: Arc<Workspace>,
    spec: IrSpec,
}

fn load_spec(path: &Path) -> anyhow::Result<Loaded> {
    let dir = path.parent().unwrap();
    let ws = WorkspaceBuilder::new()
        .root(dir)
        .build()
        .map_err(|e| anyhow::anyhow!("build: {e}"))?;
    ws.load_all("fixture.yaml")
        .map_err(|e| anyhow::anyhow!("load: {e}"))?;
    let uri = Uri::from_path(path).map_err(|e| anyhow::anyhow!("uri: {e}"))?;
    let ws = std::sync::Arc::new(ws);
    let spec = IrSpec::from_workspace(&ws, &uri).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(Loaded { ws, spec })
}

#[test]
fn stg_lifts_struct_enum_and_constraints() -> Result<(), anyhow::Error> {
    let (_dir, path) = spec_dir();
    let loaded = load_spec(&path)?;
    let graph = build_graph(&loaded.spec);

    // Pet: struct with uuid-branded id, named-enum kind, optional note.
    let pet = graph.components.get("Pet").expect("Pet lifted");
    let StgNode::Struct(s) = pet else {
        panic!("Pet must be a struct");
    };
    assert_eq!(s.fields.len(), 4);
    let id = s.fields.iter().find(|f| f.ident.original == "id").unwrap();
    let StgType::Prim(p) = &id.ty else {
        panic!("id is primitive");
    };
    assert_eq!(p.base, Base::Str);
    assert_eq!(p.refs.format, Some(WellKnownFormat::Uuid));

    // PetKind: closed string enum with two variants.
    let kind_node = graph.components.get("PetKind").expect("enum lifted");
    let StgNode::StringEnum(e) = kind_node else {
        panic!("PetKind must be an enum");
    };
    assert_eq!(e.variants.len(), 2);
    assert_eq!(e.variants[0].0, "cat");

    // Field types referencing components resolve as Named.
    let kind_field = s
        .fields
        .iter()
        .find(|f| f.ident.original == "kind")
        .unwrap();
    assert!(matches!(&kind_field.ty, StgType::Named(n) if n == "PetKind"));
    Ok(())
}

#[test]
fn stg_lifts_all_of_flattening() -> anyhow::Result<()> {
    let (_dir, path) = spec_dir();
    let loaded = load_spec(&path)?;
    let graph = build_graph(&loaded.spec);

    let input = graph.components.get("PetInput").expect("PetInput");
    let StgNode::Struct(s) = input else {
        panic!("allOf lifts to struct");
    };
    // Parent fields (id/kind/email/note) + child field (vetId).
    assert_eq!(s.fields.len(), 5, "{s:#?}");
    assert!(
        s.fields.iter().any(|f| f.ident.original == "vetId"),
        "child property present"
    );
    // vetId is optional (not in `required`) -> Option-wrapped primitive.
    let vet = s
        .fields
        .iter()
        .find(|f| f.ident.original == "vetId")
        .unwrap();
    assert!(!vet.required);
    let StgType::Optional(inner) = &vet.ty else {
        panic!("optional field");
    };
    let StgType::Prim(p) = inner.as_ref() else {
        panic!("vetId primitive inside option");
    };
    assert_eq!(
        p.refs.pattern.as_deref(),
        Some("^vet_[0-9]+$"),
        "pattern refinement folded"
    );
    Ok(())
}

#[test]
fn emitters_are_deterministic_and_cover_shapes() -> anyhow::Result<()> {
    let (_dir, path) = spec_dir();
    let loaded = load_spec(&path)?;
    let graph = build_graph(&loaded.spec);

    for target in ["ts", "rust", "go"] {
        let files_a = emit_all(&graph, &[target], &Default::default()).unwrap();
        let files_b = emit_all(&graph, &[target], &Default::default()).unwrap();
        assert_eq!(files_a, files_b, "{target} emission deterministic");
        assert!(!files_a.is_empty());
        for f in &files_a {
            assert!(
                !f.content.contains("undefined value"),
                "{} contains error text",
                f.path
            );
        }
    }

    // TS: branded/enum shapes present.
    let ts_all: String = emit_all(&graph, &["ts"], &Default::default())
        .unwrap()
        .into_iter()
        .map(|f| f.content)
        .collect();
    assert!(ts_all.contains("export interface Pet {"), "struct emitted");
    assert!(ts_all.contains("Uuid"), "branded uuid referenced");
    assert!(
        ts_all.contains("export type PetKind ="),
        "string enum union"
    );

    // Rust: serde derives + newtype refinements.
    let rs_all: String = emit_all(&graph, &["rust"], &Default::default())
        .unwrap()
        .into_iter()
        .map(|f| f.content)
        .collect();
    assert!(rs_all.contains("pub struct Pet {"), "rust struct");
    assert!(rs_all.contains("PatternVet09Newtype"), "newtype emitted");

    // Go: exported structs + JSON tags.
    let go_all: String = emit_all(&graph, &["go"], &Default::default())
        .unwrap()
        .into_iter()
        .map(|f| f.content)
        .collect();
    assert!(go_all.contains("type Pet struct {"), "go struct");
    assert!(go_all.contains("json:\"id"), "json tag present");
    Ok(())
}

#[test]
fn zod_twin_gated_by_option() -> anyhow::Result<()> {
    let (_dir, path) = spec_dir();
    let loaded = load_spec(&path)?;
    let graph = build_graph(&loaded.spec);
    let without = emit_all(&graph, &["ts"], &Default::default()).unwrap();
    assert!(
        !without.iter().any(|f| f.path.ends_with("zod.ts")),
        "zod off by default"
    );
    let with = emit_all(&graph, &["ts"], &suspect_codegen::EmitOptions { zod: true }).unwrap();
    let zod = with
        .iter()
        .find(|f| f.path.ends_with("zod.ts"))
        .expect("zod twin");
    assert!(zod.content.contains("z.object("));
    Ok(())
}

#[test]
fn drift_check_detects_modified_output() -> anyhow::Result<()> {
    let (_dir, path) = spec_dir();
    let loaded = load_spec(&path)?;
    let graph = build_graph(&loaded.spec);
    let out_dir = tempfile::tempdir().unwrap();

    let files = emit_all(&graph, &["ts"], &Default::default()).unwrap();
    suspect_codegen::write_files(&files, out_dir.path()).unwrap();
    assert!(
        matches_disk(&files, out_dir.path()),
        "freshly written tree matches"
    );

    // Mutate one file -> drift detected.
    let victim = out_dir.path().join("ts/types.ts");
    let original = std::fs::read_to_string(&victim).unwrap();
    std::fs::write(&victim, format!("{original}\n// drifted\n")).unwrap();
    assert!(!matches_disk(&files, out_dir.path()));
    Ok(())
}

#[test]
fn rust_models_have_balanced_braces() -> anyhow::Result<()> {
    let (_dir, path) = spec_dir();
    let loaded = load_spec(&path)?;
    let graph = build_graph(&loaded.spec);
    let files = emit_all(&graph, &["rust"], &Default::default()).unwrap();

    let models = files
        .iter()
        .find(|f| f.path.ends_with("models.rs"))
        .expect("models file");
    let mut depth = 0i32;
    for c in models.content.chars() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
        assert!(depth >= 0, "unbalanced braces");
    }
    assert_eq!(depth, 0, "brace depth returns to zero");
    Ok(())
}
