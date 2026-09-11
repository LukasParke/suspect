//! Integration tests for the `suspect-ref` `$ref` resolution engine.
//!
//! Every test names the invariant it defends. Fixtures use hand-rolled temp
//! dirs (std-only; no external dev-deps) and are cleaned up on drop.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use suspect_low::Pointer;
use suspect_ref::{CycleKind, RefError, Resolution, WorkspaceBuilder};

static DIR_SEQ: AtomicU32 = AtomicU32::new(0);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "suspect-ref-{name}-{}-{}",
            std::process::id(),
            DIR_SEQ.fetch_add(1, Ordering::SeqCst),
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        TempDir { path }
    }

    fn write(&self, name: &str, content: &str) -> PathBuf {
        let p = self.path.join(name);
        fs::write(&p, content).unwrap();
        p
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn ws(dir: &Path) -> suspect_ref::Workspace {
    WorkspaceBuilder::new().root(dir).build().unwrap()
}

/// Defends: RFC 6901 `~1` unescaping — pointer tokens are matched against
/// unescaped keys, so `#/a~1b` reaches the key literally named `a/b`.
#[test]
fn local_pointer_unescapes_tilde() {
    let d = TempDir::new("tilde");
    d.write("doc.yaml", "a/b:\n  deep: hit\n");
    let w = ws(&d.path);
    let h = w.open("doc.yaml").unwrap();
    let ptr = Pointer::parse("#/a~1b").unwrap();
    match h.resolve_pointer(h.id(), &ptr).unwrap() {
        Resolution::Node(n) => assert_eq!(n.get("deep").unwrap().as_str(), Some("hit")),
        other => panic!("expected Node, got {other:?}"),
    }
}

/// Defends: external file refs resolve relative to the referencing
/// document's URI and land on the pointed-to node in the other file.
#[test]
fn external_pointer_ref_crosses_files() {
    let d = TempDir::new("ext");
    d.write("a.yaml", "root:\n  $ref: 'b.yaml#/B'\n");
    d.write("b.yaml", "B:\n  kind: object\n");
    let w = ws(&d.path);
    let a = w.open("a.yaml").unwrap();
    assert_eq!(a.edges().len(), 1);
    match a.resolve_edge(0).unwrap() {
        Resolution::Node(n) => assert_eq!(n.get("kind").unwrap().as_str(), Some("object")),
        other => panic!("expected Node, got {other:?}"),
    }
}

/// Defends: a `$ref` with a document part but no fragment resolves to the
/// whole target document (`Resolution::WholeDoc`) with the right DocId.
#[test]
fn whole_doc_external_ref() {
    let d = TempDir::new("whole");
    d.write("a.yaml", "$ref: 'pet.yaml'\n");
    d.write("pet.yaml", "name: rex\nkind: dog\n");
    let w = ws(&d.path);
    let a = w.open("a.yaml").unwrap();
    let pet = w.open("pet.yaml").unwrap();
    match a.resolve_edge(0).unwrap() {
        Resolution::WholeDoc(id) => {
            assert_eq!(id, pet.id(), "whole-doc ref must target pet.yaml's slot")
        }
        other => panic!("expected WholeDoc, got {other:?}"),
    }
}

#[test]
fn root_reference_cycles_terminate_across_documents() {
    let d = TempDir::new("root-cycle");
    d.write("a.yaml", "$ref: 'b.yaml'\n");
    d.write("b.yaml", "$ref: 'a.yaml'\n");
    for entrypoint in [0, 1, 2] {
        let w = ws(&d.path);
        let a = w.open("a.yaml").unwrap();
        let result = match entrypoint {
            0 => a.resolve_edge(0),
            1 => a.resolve_pointer(a.id(), &Pointer::root()),
            _ => a.resolve_ref_value(a.doc().root().get("$ref").unwrap()),
        };
        match result.unwrap() {
            Resolution::Cycle { path } => {
                assert_eq!(path.len(), 2);
                assert_ne!(
                    path[0].doc, path[1].doc,
                    "byte ranges alone cannot identify cross-file steps"
                );
            }
            other => panic!("entrypoint {entrypoint}: expected Cycle, got {other:?}"),
        }
    }
}

#[test]
fn direct_reference_targets_preserve_each_hop_in_a_cycle() {
    let d = TempDir::new("direct-target");
    d.write("a.yaml", "$ref: 'b.yaml'\n");
    d.write("b.yaml", "$ref: 'a.yaml'\n");
    let w = ws(&d.path);
    let a = w.open("a.yaml").unwrap();
    let target = a.ref_target(a.doc().root().get("$ref").unwrap()).unwrap();
    let b = w.get_by_id(target.doc).unwrap();
    assert!(b.uri().to_string().ends_with("b.yaml"));
    assert!(target.pointer.is_root());
    let back = b.ref_target(b.doc().root().get("$ref").unwrap()).unwrap();
    assert_eq!(back.doc, a.id());
}

#[test]
fn direct_pointer_reads_preserve_unfollowed_refs_aliases_and_source_identity() {
    let d = TempDir::new("direct-pointer");
    d.write(
        "a.yaml",
        "target: &target {type: string}\nalias: *target\nref: {$ref: '#/target'}\n",
    );
    d.write(
        "b.yaml",
        "target: &target {type: number}\nalias: *target\nref: {$ref: '#/target'}\n",
    );
    let w = ws(&d.path);
    for (name, kind) in [("a.yaml", "string"), ("b.yaml", "number")] {
        let h = w.open(name).unwrap();
        for _ in 0..2 {
            for pointer in ["/target", "/alias"] {
                let node = h
                    .node_at_pointer(&Pointer::parse(pointer).unwrap())
                    .unwrap();
                assert_eq!(node.get("type").unwrap().as_str(), Some(kind));
                assert_eq!(node.syntax().doc().uri(), h.uri());
            }
            let reference = h.node_at_pointer(&Pointer::parse("/ref").unwrap()).unwrap();
            assert!(reference.get("$ref").is_some());
            assert!(reference.get("type").is_none());
        }
    }
}

#[test]
fn cached_pointer_keeps_the_semantic_container_when_ranges_are_shared() {
    let d = TempDir::new("same-range");
    d.write(
        "doc.yaml",
        "target:\n  type: string\nsequence:\n  - value\n",
    );
    let w = ws(&d.path);
    let h = w.open("doc.yaml").unwrap();
    for _ in 0..2 {
        let target = h
            .node_at_pointer(&Pointer::parse("/target").unwrap())
            .unwrap();
        assert_eq!(target.kind(), suspect_low::ValueKind::Object);
        assert_eq!(target.get("type").unwrap().as_str(), Some("string"));
        let sequence = h
            .node_at_pointer(&Pointer::parse("/sequence").unwrap())
            .unwrap();
        assert_eq!(sequence.kind(), suspect_low::ValueKind::Array);
        assert_eq!(sequence.items()[0].as_str(), Some("value"));
    }
}

/// Defends: chains of refs (A→B→C→value) resolve fully to the terminal
/// node, not to an intermediate ref object.
#[test]
fn chain_resolves_to_terminal_node() {
    let d = TempDir::new("chain");
    d.write("a.yaml", "A:\n  $ref: 'b.yaml#/B'\n");
    d.write("b.yaml", "B:\n  $ref: 'c.yaml#/C'\n");
    d.write("c.yaml", "C:\n  leaf: 42\n");
    let w = ws(&d.path);
    let a = w.open("a.yaml").unwrap();
    match a.resolve_edge(0).unwrap() {
        Resolution::Node(n) => assert_eq!(n.get("leaf").unwrap().as_i64(), Some(42)),
        other => panic!("expected Node, got {other:?}"),
    }
}

/// Defends: cycle safety — A→B→A must come back as `Resolution::Cycle`
/// with exactly the two loop steps instead of looping forever.
#[test]
fn two_file_cycle_reports_cycle() {
    let d = TempDir::new("cycle2");
    d.write("a.yaml", "A:\n  $ref: 'b.yaml#/B'\n");
    d.write("b.yaml", "B:\n  $ref: 'a.yaml#/A'\n");
    let w = ws(&d.path);
    let a = w.open("a.yaml").unwrap();
    match a.resolve_edge(0).unwrap() {
        Resolution::Cycle { path } => {
            assert_eq!(path.len(), 2, "A→B→A is a two-step loop: {path:?}");
            // Visit order starts at the resolving document's mapping.
            assert_eq!(path[0].doc, a.id());
            assert_eq!(path[1].doc, w.open("b.yaml").unwrap().id());
        }
        other => panic!("expected Cycle, got {other:?}"),
    }
}

/// Defends: census classification — a self-reference nested under the
/// recursion-point keyword `properties` is `LegalRecursion`.
#[test]
fn census_legal_recursion_under_properties() {
    let d = TempDir::new("legal");
    d.write(
        "node.yaml",
        "Node:\n  type: object\n  properties:\n    next:\n      $ref: '#/Node'\n",
    );
    let w = ws(&d.path);
    let h = w.open("node.yaml").unwrap();
    let report = h.cycles();
    assert_eq!(report.cycles.len(), 1, "one self-loop expected");
    assert_eq!(report.cycles[0].kind, CycleKind::LegalRecursion);
    assert_eq!(report.cycles[0].steps.len(), 1);
}

/// Defends: census classification — a loop threaded through `required`
/// (not a recursion-point keyword) is `Illegal`.
#[test]
fn census_illegal_loop_through_required() {
    let d = TempDir::new("illegal");
    d.write("bad.yaml", "A:\n  required:\n    - $ref: '#/A'\n");
    let w = ws(&d.path);
    let h = w.open("bad.yaml").unwrap();
    let report = h.cycles();
    assert_eq!(report.cycles.len(), 1, "one self-loop expected");
    assert_eq!(report.cycles[0].kind, CycleKind::Illegal);
}

/// Defends: remote (http/https) references are never fetched in v1 — they
/// produce `RemoteDenied`, not I/O errors or hangs.
#[test]
fn remote_refs_denied() {
    let d = TempDir::new("remote");
    d.write(
        "a.yaml",
        "x:\n  $ref: 'https://example.com/schemas/pet.yaml#/Pet'\n",
    );
    let w = ws(&d.path);
    let h = w.open("a.yaml").unwrap();
    match h.resolve_edge(0) {
        Err(RefError::RemoteDenied { uri }) => {
            assert!(uri.starts_with("https://example.com/"), "{uri}")
        }
        other => panic!("expected RemoteDenied, got {other:?}"),
    }
}

/// Defends: memoization — the second identical `resolve_pointer` must hit
/// the cache (memo_hits increases, no re-walk).
#[test]
fn memo_hits_increase_on_repeat() {
    let d = TempDir::new("memo");
    d.write("doc.yaml", "a:\n  b:\n    c: 1\n");
    let w = ws(&d.path);
    let h = w.open("doc.yaml").unwrap();
    let ptr = Pointer::parse("#/a/b").unwrap();
    let before = w.stats().memo_hits;
    h.resolve_pointer(h.id(), &ptr).unwrap();
    h.resolve_pointer(h.id(), &ptr).unwrap();
    let after = w.stats().memo_hits;
    assert!(
        after > before,
        "second resolve must be a memo hit ({before} → {after})"
    );
}

/// Defends: load_all dedupes — a B→C diamond over D loads exactly 4
/// documents, with D loaded once despite two inbound edges.
#[test]
fn load_all_diamond_loads_each_doc_once() {
    let d = TempDir::new("diamond");
    d.write(
        "a.yaml",
        "toB:\n  $ref: 'b.yaml#/B'\ntoC:\n  $ref: 'c.yaml#/C'\n",
    );
    d.write("b.yaml", "B:\n  $ref: 'd.yaml#/D'\n");
    d.write("c.yaml", "C:\n  $ref: 'd.yaml#/D'\n");
    d.write("d.yaml", "D:\n  leaf: true\n");
    let w = ws(&d.path);
    let total = w.load_all("a.yaml").unwrap();
    assert_eq!(total, 4, "A, B, C, D");
    assert_eq!(w.len(), 4);
    assert_eq!(w.uris().len(), 4);
    // Every doc is retrievable and D resolves from both parents.
    let b = w.open("b.yaml").unwrap();
    assert!(matches!(b.resolve_edge(0).unwrap(), Resolution::Node(_)));
    let c = w.open("c.yaml").unwrap();
    assert!(matches!(c.resolve_edge(0).unwrap(), Resolution::Node(_)));
}

/// Defends: `MissingPointer` errors name both the searched document and
/// the pointer that missed (diagnosability contract).
#[test]
fn missing_pointer_error_names_uri_and_pointer() {
    let d = TempDir::new("missing");
    d.write("doc.yaml", "a: 1\n");
    let w = ws(&d.path);
    let h = w.open("doc.yaml").unwrap();
    let err = h
        .resolve_pointer(h.id(), &Pointer::parse("#/no/such").unwrap())
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("doc.yaml"),
        "message must contain the uri: {msg}"
    );
    assert!(
        msg.contains("/no/such"),
        "message must contain the pointer: {msg}"
    );
}

/// Defends: percent-decoding happens *before* pointer parsing inside
/// `$ref` values — `#/paths/~1pets~1%7Bid%7D/get` reaches the key
/// `/pets/{id}`.
#[test]
fn percent_encoded_fragment_decodes() {
    let d = TempDir::new("pct");
    d.write(
        "api.yaml",
        "x:\n  $ref: '#/paths/~1pets~1%7Bid%7D/get'\npaths:\n  /pets/{id}:\n    get:\n      operationId: getPet\n",
    );
    let w = ws(&d.path);
    let h = w.open("api.yaml").unwrap();
    match h.resolve_edge(0).unwrap() {
        Resolution::Node(n) => assert_eq!(n.get("operationId").unwrap().as_str(), Some("getPet")),
        other => panic!("expected Node, got {other:?}"),
    }
}

/// Defends: plain-name fragments resolve through the `$anchor` index —
/// `#foo` finds the schema carrying `$anchor: foo`.
#[test]
fn plain_name_fragment_resolves_anchor() {
    let d = TempDir::new("anchor");
    d.write(
        "schemas.yaml",
        "root:\n  $ref: '#foo'\nschemas:\n  Pet:\n    $anchor: foo\n    type: object\n",
    );
    let w = ws(&d.path);
    let h = w.open("schemas.yaml").unwrap();
    match h.resolve_edge(0).unwrap() {
        Resolution::Node(n) => assert_eq!(n.get("type").unwrap().as_str(), Some("object")),
        other => panic!("expected Node, got {other:?}"),
    }
}

#[test]
fn external_plain_name_fragment_keeps_its_target_document() {
    let d = TempDir::new("external-anchor");
    d.write(
        "a.yaml",
        "use: {$ref: 'b.json#P%65t'}\nwrong: {$anchor: Pet, type: integer}\n",
    );
    d.write(
        "b.json",
        r#"{"defs":{"Pet":{"$anchor":"P\u0065t","type":"string"}}}"#,
    );
    let w = ws(&d.path);
    assert_eq!(
        w.load_all("a.yaml").unwrap(),
        2,
        "anchor target must join load closure"
    );
    let h = w.open("a.yaml").unwrap();
    match h.resolve_edge(0).unwrap() {
        Resolution::Node(n) => {
            assert_eq!(n.get("type").unwrap().as_str(), Some("string"));
            assert!(n.syntax().doc().uri().to_string().ends_with("b.json"));
        }
        other => panic!("expected external anchor target, got {other:?}"),
    }
}

/// Defends: concurrent opens of the same document converge on one DocId —
/// the load path is idempotent under thread contention.
#[test]
fn concurrent_open_same_doc_single_id() {
    let d = TempDir::new("concurrent");
    d.write("shared.yaml", "value: 7\n");
    let w = Arc::new(ws(&d.path));
    let path = d.path.join("shared.yaml");
    let ids: Vec<suspect_ref::DocId> = std::thread::scope(|s| {
        let mut handles = Vec::new();
        for _ in 0..4 {
            let w = Arc::clone(&w);
            let p = path.clone();
            handles.push(s.spawn(move || w.open(p.to_str().unwrap()).unwrap().id()));
        }
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert!(
        ids.windows(2).all(|w| w[0] == w[1]),
        "all threads must agree: {ids:?}"
    );
    assert_eq!(w.len(), 1, "exactly one slot for one document");
}

/// Defends: `$id` base-URI inheritance — a local ref under an `$id`
/// ancestor whose base leaves the document resolves as an external ref.
#[test]
fn id_base_inheritance_redirects_to_external() {
    let d = TempDir::new("idbase");
    d.write(
        "a.yaml",
        "defs:\n  $id: 'b.yaml'\n  inner:\n    $ref: '#/Shared'\n",
    );
    d.write("b.yaml", "Shared:\n  from: b\n");
    let w = ws(&d.path);
    let h = w.open("a.yaml").unwrap();
    match h.resolve_edge(0).unwrap() {
        Resolution::Node(n) => assert_eq!(n.get("from").unwrap().as_str(), Some("b")),
        other => panic!("expected Node from b.yaml, got {other:?}"),
    }
}

/// Defends: `resolve_ref_value` resolves the string value of a `$ref` key
/// directly, independent of edge indexing.
#[test]
fn resolve_ref_value_works_directly() {
    let d = TempDir::new("refvalue");
    d.write("doc.yaml", "target:\n  ok: yes\nref:\n  $ref: '#/target'\n");
    let w = ws(&d.path);
    let h = w.open("doc.yaml").unwrap();
    let ptr = Pointer::parse("#/ref/$ref").unwrap();
    let node = match h.resolve_pointer(h.id(), &ptr).unwrap() {
        Resolution::Node(n) => n,
        other => panic!("expected Node, got {other:?}"),
    };
    match h.resolve_ref_value(node).unwrap() {
        Resolution::Node(n) => assert_eq!(n.get("ok").unwrap().as_str(), Some("yes")),
        other => panic!("expected Node, got {other:?}"),
    }
}

#[test]
fn reference_strings_and_fragments_decode_before_pointer_lookup() {
    let d = TempDir::new("decoded-ref");
    for raw in [
        r##""#\/caf\u00e9""##,
        r##""#%2Fcaf%C3%A9""##,
        r##""#/caf%C3%A9""##,
    ] {
        d.write(
            "doc.json",
            &format!(r#"{{"caf\u00e9":{{"type":"string"}},"use":{{"$ref":{raw}}}}}"#),
        );
        for direct in [true, false] {
            let w = ws(&d.path);
            let h = w.open("doc.json").unwrap();
            let result = if direct {
                h.resolve_ref_value(h.doc().root().get("use").unwrap().get("$ref").unwrap())
            } else {
                h.resolve_edge(0)
            };
            match result.unwrap_or_else(|e| panic!("{raw}, direct={direct}: {e}")) {
                Resolution::Node(n) => {
                    assert_eq!(n.get("type").unwrap().as_str(), Some("string"));
                }
                other => panic!("{raw}, direct={direct}: expected Node, got {other:?}"),
            }
        }
    }
}

#[test]
fn malformed_references_remain_observable_with_source_locations() {
    let d = TempDir::new("invalid-ref-diagnostics");
    let source = r##"{
      "good":{"$ref":"#/target"}, "target":{"type":"string"},
      "badPercent":{"$ref":"#/bad%GG"},
      "badUtf8":{"$ref":"#/%FF"},
      "badPointer":{"$ref":"#/bad~2"},
      "nonString":{"$ref":42},
      "nullValue":{"$ref":null},
      "objectValue":{"$ref":{}},
      "escapedKey":{"\u0024ref":false}
    }"##;
    d.write("doc.json", source);
    let w = ws(&d.path);
    let h = w.open("doc.json").unwrap();
    assert_eq!(h.edges().len(), 1);
    let diagnostics = h.ref_diagnostics();
    assert_eq!(diagnostics.len(), 7);
    for diagnostic in diagnostics.iter() {
        assert!(!diagnostic.reason.is_empty());
        assert!(!diagnostic.at.is_empty());
        let value = h
            .doc()
            .root()
            .pointer(&diagnostic.path)
            .unwrap()
            .get("$ref")
            .unwrap();
        assert_eq!(diagnostic.at, value.byte_range());
        assert!(matches!(
            h.resolve_ref_value(value),
            Err(RefError::InvalidRef { .. })
        ));
    }
}

#[test]
fn empty_yaml_reference_values_are_diagnosed_at_the_key() {
    let d = TempDir::new("empty-ref");
    d.write("doc.yaml", "empty:\n  $ref:\nexample:\n  $ref: false\n");
    let w = ws(&d.path);
    let h = w.open("doc.yaml").unwrap();
    let diagnostics = h.ref_diagnostics();
    // The generic scanner includes arbitrary data; OpenAPI consumers must
    // scope this evidence themselves instead of globally skipping key names.
    assert_eq!(diagnostics.len(), 2);
    assert_eq!(diagnostics[0].path.to_path(), "/empty");
    assert_eq!(&h.doc().inner().bytes()[diagnostics[0].at.clone()], b"$ref");
    assert!(diagnostics[0].raw.is_none());
}

/// Defends: relative entries resolve against the builder root (CLI
/// contract), not the process CWD.
#[test]
fn builder_root_resolves_relative_entries() {
    let d = TempDir::new("root");
    d.write("entry.yaml", "top: true\n");
    // Build without root but open via absolute path to prove both work.
    let w = WorkspaceBuilder::new().root(&d.path).build().unwrap();
    let h = w.open("entry.yaml").unwrap();
    assert!(matches!(
        h.resolve_pointer(h.id(), &Pointer::parse("#/top").unwrap())
            .unwrap(),
        Resolution::Node(_)
    ));
}

/// Defends: max_docs is enforced by load_all — a frontier beyond the cap
/// fails with `TooManyDocs` rather than growing unbounded.
#[test]
fn load_all_enforces_doc_cap() {
    let d = TempDir::new("cap");
    d.write("a.yaml", "toB:\n  $ref: 'b.yaml#/B'\n");
    d.write("b.yaml", "B: {}\n");
    let w = WorkspaceBuilder::new()
        .root(&d.path)
        .max_docs(1)
        .build()
        .unwrap();
    match w.load_all("a.yaml") {
        Err(suspect_ref::WorkspaceError::TooManyDocs { max }) => assert_eq!(max, 1),
        other => panic!("expected TooManyDocs, got {other:?}"),
    }
}

/// Defends: `$ref` written as a folded block scalar (Stripe style) must be
/// decoded to its pointer text, not read as the raw `>-` source.
#[test]
fn block_scalar_refs_resolve() {
    let d = TempDir::new("blockref");
    d.write(
        "stripe-style.yaml",
        "components:\n  schemas:\n    A:\n      $ref: >-\n        #/components/schemas/B\n    B:\n      type: object\n",
    );
    let w = ws(&d.path);
    if let Err(e) = w.load_all("stripe-style.yaml") {
        panic!("LOAD FAILED: {e:?}");
    }
    let content = fs::read_to_string(d.path.join("stripe-style.yaml")).unwrap();
    eprintln!("FULL CONTENT:\n{content}");
    let _ = content;
    let h = w.open("stripe-style.yaml").unwrap();
    let edges = h.edges();
    assert_eq!(edges.len(), 1, "one edge");
    match h.resolve_edge(0).unwrap() {
        Resolution::Node(n) => {
            assert_eq!(
                n.kind(),
                suspect_low::ValueKind::Object,
                "resolves to schema B"
            );
        }
        other => panic!("expected Node, got {other:?}"),
    }
}
