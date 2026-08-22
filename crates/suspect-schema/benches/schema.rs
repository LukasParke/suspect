//! Benchmarks for schema compilation and validation.
//!
//! Measures the two hot paths end to end over a representative OpenAPI-style
//! schema with nested objects, arrays, `$ref`s, and a recursive tree.

//! Also profiles real corpus documents (gitignored, skipped when absent):
//!
//! - `compile_corpus`: compile *every* component schema of `stripe.yaml`
//!   in one batch per iteration.
//! - `validate_instances`: 25 evenly-sampled object schemas with
//!   synthesized instances — empty object, required-keys-filled by declared
//!   type, and deliberately-wrong types to exercise error paths.

use std::hint::black_box;
use std::path::{Path, PathBuf};

use criterion::{Criterion, criterion_group, criterion_main};
use suspect_low::{LowDoc, NodeRef, ValueKind};
use suspect_schema::{Compiler, Config};
use suspect_source::{Source, Uri};
use suspect_syntax::Format;

const SCHEMA: &str = r##"{
    "$defs": {
        "name": {"type": "string", "minLength": 1, "maxLength": 64},
        "pet": {
            "type": "object",
            "required": ["name"],
            "properties": {
                "name": {"$ref": "#/$defs/name"},
                "tags": {"type": "array", "items": {"$ref": "#/$defs/name"}},
                "status": {"enum": ["available", "pending", "sold"]},
                "weight": {"type": "number", "minimum": 0, "multipleOf": 0.1},
                "friend": {"$ref": "#/$defs/pet"}
            },
            "additionalProperties": false
        }
    },
    "properties": {"pets": {"type": "array", "items": {"$ref": "#/$defs/pet"}}}
}"##;

const INSTANCE: &str = r##"{
    "pets": [
        {"name": "a", "tags": ["t1", "t2"], "status": "available", "weight": 4.2},
        {"name": "b", "friend": {"name": "c"}},
        {"name": "d", "tags": [], "status": "sold"}
    ]
}"##;

fn doc(text: &str) -> LowDoc {
    let uri = Uri::from_path(Path::new("/benches/schema.json")).expect("uri");
    LowDoc::parse(uri, Source::from_vec(text.as_bytes().to_vec()))
}

fn bench_compile(c: &mut Criterion) {
    c.bench_function("compile/medium_schema", |b| {
        b.iter_batched(
            || doc(&format!("{{\"S\": {SCHEMA}, \"I\": {INSTANCE}}}")),
            |doc| {
                let root = doc.root();
                let schema = Compiler::new(Config::default())
                    .compile(root.get("S").expect("schema slot"))
                    .expect("compile");
                black_box(schema.root().byte_range().len())
            },
            criterion::BatchSize::SmallInput,
        )
    });
}

fn bench_validate(c: &mut Criterion) {
    c.bench_function("validate/medium_instance", |b| {
        let doc = doc(&format!("{{\"S\": {SCHEMA}, \"I\": {INSTANCE}}}"));
        let root = doc.root();
        let schema = Compiler::new(Config::default())
            .compile(root.get("S").expect("schema slot"))
            .expect("compile");
        let instance = root.get("I").expect("instance slot");
        b.iter(|| assert!(schema.validate(instance).is_empty()))
    });
}

fn bench_first_error(c: &mut Criterion) {
    c.bench_function("validate_first/invalid_instance", |b| {
        let bad = format!("{{\"S\": {SCHEMA}, \"I\": {{\"pets\":[{{\"name\":3}}]}}}}");
        let doc = doc(&bad);
        let root = doc.root();
        let schema = Compiler::new(Config::default())
            .compile(root.get("S").expect("schema slot"))
            .expect("compile");
        let instance = root.get("I").expect("instance slot");
        b.iter(|| assert!(schema.validate_first(instance).is_some()))
    });
}

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus")
}

/// Loads a corpus file as a `LowDoc`, or `None` (with a note) when absent.
fn load_corpus(name: &str) -> Option<LowDoc> {
    let path = corpus_dir().join(name);
    if !path.exists() {
        eprintln!(
            "[schema] skipping {name}: corpus file {} not found",
            path.display()
        );
        return None;
    }
    match Source::from_path(&path) {
        Ok(source) => {
            let uri = Uri::from_path(&path)
                .unwrap_or_else(|e| panic!("valid URI for {}: {e}", path.display()));
            Some(LowDoc::parse(uri, source))
        }
        Err(e) => {
            eprintln!("[schema] skipping {name}: read failed ({e})");
            None
        }
    }
}

/// The `components.schemas` entries of a document.
fn component_schemas<'d>(doc: &'d LowDoc) -> Vec<(&'d str, NodeRef<'d>)> {
    doc.root()
        .get("components")
        .and_then(|c| c.get("schemas"))
        .map(|schemas| {
            schemas
                .entries()
                .into_iter()
                .filter_map(|e| e.value.map(|v| (e.key, v)))
                .collect()
        })
        .unwrap_or_default()
}

fn bench_compile_corpus(c: &mut Criterion) {
    const NAME: &str = "stripe.yaml";
    let Some(doc) = load_corpus(NAME) else { return };
    let schemas = component_schemas(&doc);
    if schemas.is_empty() {
        eprintln!("[schema] skipping compile_corpus: {NAME} has no components.schemas");
        return;
    }

    // Warmup + sanity outside timing: compilation must succeed for at least
    // some schemas (exotic keyword combinations may legitimately fail).
    let compiler = Compiler::new(Config::default());
    let ok = schemas
        .iter()
        .filter(|(_, s)| compiler.compile(*s).is_ok())
        .count();
    eprintln!(
        "[schema] {NAME}: compiled {ok}/{} component schemas (warmup)",
        schemas.len()
    );
    assert!(ok > 0, "at least one schema must compile");

    let mut group = c.benchmark_group("compile_corpus");
    group.throughput(criterion::Throughput::Elements(schemas.len() as u64));
    group.sample_size(10);
    group.bench_function("all_stripe_component_schemas", |b| {
        b.iter(|| {
            for (_, sch) in black_box(&schemas) {
                black_box(compiler.compile(*sch).is_ok());
            }
        });
    });
    group.finish();
}

/// Evenly samples up to 25 object-typed schemas declaring a non-empty
/// `required` array.
fn pick_object_schemas<'d>(
    schemas: &[(&'d str, NodeRef<'d>)],
) -> Vec<(&'d str, NodeRef<'d>, Vec<&'d str>)> {
    let eligible: Vec<_> = schemas
        .iter()
        .filter_map(|(name, s)| {
            if s.kind() != ValueKind::Object {
                return None;
            }
            let required: Vec<&'d str> = s
                .get("required")?
                .items()
                .into_iter()
                .filter_map(|r| r.as_str())
                .collect();
            (!required.is_empty()).then_some((*name, *s, required))
        })
        .collect();
    if eligible.is_empty() {
        return eligible;
    }
    const LIMIT: usize = 25;
    let step = (eligible.len() / LIMIT).max(1);
    eligible.into_iter().step_by(step).take(LIMIT).collect()
}

/// A valid value for a property of the given declared type; deliberately
/// wrong (opposite scalar kind) values exercise the error paths.
fn value_for_type(ty: Option<&str>, wrong: bool) -> &'static str {
    if wrong {
        return match ty {
            Some("string") => "42",
            _ => "\"deliberately-wrong\"",
        };
    }
    match ty {
        Some("string") => "\"bench-value\"",
        Some("integer" | "number") => "7",
        Some("boolean") => "true",
        Some("array") => "[]",
        _ => "{}",
    }
}

fn build_inner_json(schemas: &[(&str, NodeRef<'_>, Vec<&str>)], wrong: bool) -> String {
    let mut out = String::from("{");
    for (i, (name, node, required)) in schemas.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "\"{}\":{{",
            name.replace('\\', "\\\\").replace('"', "\\\"")
        ));
        let props = node.get("properties");
        for (j, key) in required.iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            let ty = props
                .as_ref()
                .and_then(|p| p.get(key))
                .and_then(|p| p.get("type"))
                .and_then(|t| t.as_str());
            let escaped = key.replace('\\', "\\\\").replace('"', "\\\"");
            out.push_str(&format!("\"{}\":{}", escaped, value_for_type(ty, wrong)));
        }
        out.push('}');
    }
    out.push('}');
    out
}

fn bench_validate_instances(c: &mut Criterion) {
    const NAME: &str = "stripe.yaml";
    let Some(doc) = load_corpus(NAME) else { return };
    let schemas = component_schemas(&doc);
    let picked = pick_object_schemas(&schemas);
    if picked.is_empty() {
        eprintln!("[schema] skipping validate_instances: no object schemas with required keys");
        return;
    }

    // One combined instance document holding every empty / filled / wrong
    // object, parsed once so nodes borrow from a long-lived source.
    let inner = [
        build_inner_json(&picked, false),
        build_inner_json(&picked, false),
        build_inner_json(&picked, true),
    ];
    let json = format!(
        "{{\"empty\":{},\"filled\":{},\"wrong\":{}}}",
        inner[0], inner[1], inner[2]
    );

    let instances = LowDoc::with_format(
        Uri::parse("memory://schema-bench-instances.json").expect("valid synthetic URI"),
        Source::from_vec(json.into_bytes()),
        Format::Json,
    );
    let root = instances.root();
    let compiler = Compiler::new(Config::default());

    struct Case<'a> {
        empty: NodeRef<'a>,
        filled: NodeRef<'a>,
        wrong: NodeRef<'a>,
    }

    // Compile each schema outside timing; drop cases that fail to compile
    // or whose synthesized instances are missing from the combined document.
    let cases: Vec<(suspect_schema::Schema<'_>, Case<'_>)> = picked
        .iter()
        .filter_map(|(name, node, _)| {
            let schema = compiler.compile(*node).ok()?;
            let lookup = |section: &str| root.get(section).and_then(|s| s.get(name));
            Some((
                schema,
                Case {
                    empty: lookup("empty")?,
                    filled: lookup("filled")?,
                    wrong: lookup("wrong")?,
                },
            ))
        })
        .collect();

    if cases.is_empty() {
        eprintln!("[schema] skipping validate_instances: no compilable schemas among picks");
        return;
    }

    // Sanity outside timing: deliberately-wrong instances must produce
    // errors on at least one case.
    let total_wrong_errors: usize = cases.iter().map(|(s, c)| s.validate(c.wrong).len()).sum();
    eprintln!(
        "[schema] {} instance cases; wrong-instance errors total: {total_wrong_errors} (warmup)",
        cases.len()
    );
    assert!(
        total_wrong_errors > 0,
        "deliberately-wrong instances must fail somewhere"
    );

    let mut group = c.benchmark_group("validate_instances");
    group.throughput(criterion::Throughput::Elements(cases.len() as u64));
    group.sample_size(20);
    group.bench_function("25_stripe_schemas_3_instances_each", |b| {
        b.iter(|| {
            for (schema, case) in &cases {
                black_box(schema.validate(case.empty));
                black_box(schema.validate(case.filled));
                black_box(schema.validate(case.wrong));
            }
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_compile,
    bench_validate,
    bench_first_error,
    bench_compile_corpus,
    bench_validate_instances
);
criterion_main!(benches);
