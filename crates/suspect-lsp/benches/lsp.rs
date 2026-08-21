//! LSP hot-path benchmarks for `suspect-lsp`.
//!
//! Each group models one language-server operation against real corpus
//! specifications, using the same public helpers the server backend calls:
//!
//! - `did_open`: `OpenDoc::parse` (buffer text -> `LowDoc` + line index)
//! - `did_change`: incremental `SourceDoc::reparse` vs. full reparse control
//! - `goto_def` / `hover`: `navigation::{node_at, goto_definition, hover_markdown}`
//! - `completion`: `completion::{context_at, ref_candidates, key_items, ref_items}`
//! - `symbols_folding`: `symbols::{document_symbols, folding_ranges}`
//! - `diagnostics_pipeline`: `diagnostics::compute_diagnostics` (syntax +
//!   semantic validation via `suspect_validate` + spectral lint)
//!
//! Corpus files are read once at setup; when a corpus file is missing the
//! group is skipped with a note on stderr.

use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use criterion::{BenchmarkGroup, Criterion, Throughput, criterion_group, criterion_main};
use suspect_lsp::completion::{CompletionContext, context_at, key_items, ref_candidates, ref_items, SCHEMA_KEYS};
use suspect_lsp::diagnostics::compute_diagnostics;
use suspect_lsp::navigation::{goto_definition, hover_markdown, node_at};
use suspect_lsp::state::OpenDoc;
use suspect_lsp::symbols::{document_symbols, folding_ranges};
use suspect_low::LowDoc;
use suspect_ref::WorkspaceBuilder;
use suspect_source::{Source, Uri};
use suspect_syntax::{Edit, Format, SourceDoc};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus")
}

/// Reads a corpus file, or `None` (with a note) when it is absent so the
/// bench group can be skipped gracefully.
fn read_corpus(name: &str) -> Option<(PathBuf, Vec<u8>)> {
    let path = corpus_dir().join(name);
    match std::fs::read(&path) {
        Ok(bytes) => Some((path, bytes)),
        Err(e) => {
            eprintln!("skipping: corpus file {} unavailable ({e})", path.display());
            None
        }
    }
}

fn uri_for(path: &Path) -> Uri {
    Uri::from_path(path).unwrap_or_else(|e| panic!("bad URI for {}: {e}", path.display()))
}

fn group<'a>(
    c: &'a mut Criterion,
    label: &str,
    bytes: usize,
    sample_size: Option<usize>,
) -> BenchmarkGroup<'a, criterion::measurement::WallTime> {
    let mut g = c.benchmark_group(label);
    g.throughput(Throughput::Bytes(bytes as u64));
    // Corpus operations run 10-20 samples; a short warm-up keeps full
    // `cargo bench` runs within a few minutes.
    g.warm_up_time(std::time::Duration::from_millis(500));
    if let Some(n) = sample_size {
        g.sample_size(n);
    }
    g
}

/// Byte offset just inside the string value of the first `$ref` mapping
/// entry, so `node_at`/`ref_value_node`/`context_at` land on a ref value.
fn first_ref_value_offset(bytes: &[u8]) -> Option<usize> {
    let mut from = 0;
    while let Some(pos) = bytes[from..].windows(5).position(|w| w == b"$ref:") {
        let abs = from + pos + 5;
        let mut i = abs;
        while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
            i += 1;
        }
        if i < bytes.len() {
            return Some(i + 1);
        }
        from = abs;
    }
    None
}

/// Byte offset inside the `components.schemas` section (first entry name).
fn schemas_section_offset(bytes: &[u8]) -> Option<usize> {
    let comp = bytes.windows(11).position(|w| w == b"components:")?;
    let rest = &bytes[comp..];
    let sch = rest.windows(8).position(|w| w == b"schemas:")?;
    Some(comp + sch + 9)
}

// ---------------------------------------------------------------- did_open

fn bench_did_open(c: &mut Criterion) {
    for (name, file, sample) in [
        ("did_open/stripe_yaml", "stripe.yaml", Some(10)),
        ("did_open/petstore_expanded_yaml", "petstore-expanded.yaml", None),
    ] {
        let Some((path, bytes)) = read_corpus(file) else { continue };
        let uri = uri_for(&path);
        // The editor hands us UTF-8 text; clone cost (one memcpy) is paid
        // identically on every iteration and is small vs. the parse.
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let mut g = group(c, name, bytes.len(), sample);
        g.bench_function("parse", |b| {
            b.iter(|| black_box(OpenDoc::parse(black_box(uri.clone()), text.clone())))
        });
        g.finish();
    }
}

// -------------------------------------------------------------- did_change

/// Inserts `payload` at mid-file of stripe.yaml; measures incremental
/// `reparse` against a full `SourceDoc::parse` control on the same text.
fn bench_did_change(c: &mut Criterion) {
    let Some((path, bytes)) = read_corpus("stripe.yaml") else { return };
    let uri = uri_for(&path);
    let base = SourceDoc::with_format(uri.clone(), Source::from_vec(bytes.clone()), Format::Yaml);
    let mid = bytes.len() / 2;
    let unit: &[u8] = b"\n# bench insertion: x: 1\n";

    let mut g = group(c, "did_change/stripe_yaml", bytes.len(), Some(10));
    for size in [100usize, 1_000, 10_000] {
        let mut payload = Vec::with_capacity(size);
        while payload.len() < size {
            let left = size - payload.len();
            payload.extend_from_slice(&unit[..unit.len().min(left)]);
        }
        let mut modified = Vec::with_capacity(bytes.len() + size);
        modified.extend_from_slice(&bytes[..mid]);
        modified.extend_from_slice(&payload);
        modified.extend_from_slice(&bytes[mid..]);

        // Precomputed outside the timed loop: the LSP client would send the
        // edit once; the server then reparses on every notification.
        let edit = Edit::from_bytes(&base, mid, mid, payload.len());
        let edit = black_box(edit);
        let modified = Source::from_vec(modified);

        g.bench_function(format!("incremental_{}", size), |b| {
            b.iter(|| {
                let doc = base.reparse(Source::from_vec(black_box(&modified).bytes().to_vec()), &[edit]);
                black_box(doc.root().byte_range())
            })
        });
    }
    // Control: same final text, full parse from scratch (what a non-
    // incremental server would do per change).
    let full_modified = {
        let mut m = Vec::with_capacity(bytes.len() + 10_000);
        m.extend_from_slice(&bytes[..mid]);
        m.extend_from_slice(&unit.repeat(10_000 / unit.len() + 1)[..10_000]);
        m.extend_from_slice(&bytes[mid..]);
        m
    };
    g.bench_function("full_reparse_control_10kb", |b| {
        b.iter(|| {
            let doc = SourceDoc::parse(uri.clone(), Source::from_vec(full_modified.clone()));
            black_box(doc.root().byte_range())
        })
    });
    g.finish();
}

// ---------------------------------------------------------------- goto_def

fn bench_goto_def(c: &mut Criterion) {
    let Some((path, bytes)) = read_corpus("stripe.yaml") else { return };
    let uri = uri_for(&path);
    let Some(offset) = first_ref_value_offset(&bytes) else {
        eprintln!("skipping goto_def: no $ref found in stripe.yaml");
        return;
    };
    let ws = WorkspaceBuilder::new()
        .root(corpus_dir())
        .build()
        .unwrap_or_else(|e| panic!("workspace build failed: {e}"));
    if let Err(e) = ws.load_all("stripe.yaml") {
        eprintln!("skipping goto_def: stripe.yaml failed to load into workspace: {e:?}");
        return;
    }
    let low = LowDoc::parse(uri.clone(), Source::from_vec(bytes.clone()));

    let mut g = group(c, "goto_def/stripe_yaml", bytes.len(), Some(20));
    g.bench_function("node_at", |b| {
        b.iter(|| black_box(node_at(&low, offset).map(|n| n.byte_range())))
    });
    g.bench_function("goto_definition", |b| {
        b.iter(|| black_box(goto_definition(&ws, &low, offset).map(|d| d.range)))
    });
    g.finish();
}

// ------------------------------------------------------------------- hover

fn bench_hover(c: &mut Criterion) {
    let Some((path, bytes)) = read_corpus("stripe.yaml") else { return };
    let uri = uri_for(&path);
    let Some(offset) = schemas_section_offset(&bytes) else {
        eprintln!("skipping hover: no components.schemas section in stripe.yaml");
        return;
    };
    let ws = WorkspaceBuilder::new()
        .root(corpus_dir())
        .build()
        .unwrap_or_else(|e| panic!("workspace build failed: {e}"));
    if let Err(e) = ws.load_all("stripe.yaml") {
        eprintln!("skipping hover: stripe.yaml failed to load into workspace: {e:?}");
        return;
    }
    let low = LowDoc::parse(uri.clone(), Source::from_vec(bytes.clone()));

    let mut g = group(c, "hover/stripe_schema_node", bytes.len(), Some(20));
    g.bench_function("hover_markdown", |b| {
        b.iter(|| black_box(hover_markdown(&ws, &low, offset).map(|s| s.len())))
    });
    g.finish();
}

// --------------------------------------------------------------- completion

fn bench_completion(c: &mut Criterion) {
    let Some((path, bytes)) = read_corpus("petstore-expanded.yaml") else { return };
    let uri = uri_for(&path);
    let Some(offset) = first_ref_value_offset(&bytes) else {
        eprintln!("skipping completion: no $ref found in petstore-expanded.yaml");
        return;
    };
    let ws = WorkspaceBuilder::new()
        .root(corpus_dir())
        .build()
        .unwrap_or_else(|e| panic!("workspace build failed: {e}"));
    if ws.load_all("petstore-expanded.yaml").is_err() {
        eprintln!("skipping completion: petstore failed to load into workspace");
        return;
    }
    let low = LowDoc::parse(uri.clone(), Source::from_vec(bytes.clone()));

    let mut g = group(c, "completion/petstore", bytes.len(), None);
    g.bench_function("context_at_ref_candidates", |b| {
        b.iter(|| {
            let ctx = context_at(&low, offset);
            let items = match ctx {
                CompletionContext::Refs => ref_items(ref_candidates(&ws, &uri)),
                CompletionContext::Keys(keys) => key_items(keys),
                CompletionContext::None => Vec::new(),
            };
            black_box(items.len())
        })
    });
    g.bench_function("key_items_schema", |b| {
        b.iter(|| black_box(key_items(SCHEMA_KEYS).len()))
    });
    g.finish();
}

// --------------------------------------------------------- symbols_folding

fn bench_symbols_folding(c: &mut Criterion) {
    let Some((path, bytes)) = read_corpus("api.github.com.yaml") else { return };
    let uri = uri_for(&path);
    let low = LowDoc::parse(uri, Source::from_vec(bytes.clone()));

    let mut g = group(c, "symbols_folding/github_yaml", bytes.len(), Some(10));
    g.bench_function("document_symbols", |b| {
        b.iter(|| black_box(document_symbols(&low).len()))
    });
    g.bench_function("folding_ranges", |b| {
        b.iter(|| black_box(folding_ranges(&low).len()))
    });
    g.finish();
}

// ---------------------------------------------------- diagnostics_pipeline

fn bench_diagnostics(c: &mut Criterion) {
    for (name, file, sample) in [
        ("diagnostics_pipeline/digitalocean", "digitalocean.yaml", Some(20)),
        ("diagnostics_pipeline/stripe_yaml", "stripe.yaml", Some(10)),
    ] {
        let Some((path, bytes)) = read_corpus(file) else { continue };
        let uri = uri_for(&path);
        let ws = Arc::new(
            WorkspaceBuilder::new()
                .root(corpus_dir())
                .build()
                .unwrap_or_else(|e| panic!("workspace build failed: {e}")),
        );
        // digitalocean.yaml $refs ~660 sibling .yml files that are not in
        // corpus, so load_all cannot complete; that is fine — the LSP
        // runs the same battery regardless and validate_diagnostics
        // degrades to no semantic diagnostics on load failure. stripe.yaml
        // is self-contained and exercises validate_entry fully.
        if let Err(e) = ws.load_all(file) {
            eprintln!("{name}: note: ref closure incomplete, validation degrades ({e:?})");
        }
        let low = LowDoc::parse(uri, Source::from_vec(bytes.clone()));

        let mut g = group(c, name, bytes.len(), sample);
        g.bench_function("compute_diagnostics", |b| {
            b.iter(|| black_box(compute_diagnostics(Some(&ws), &low).len()))
        });
        g.finish();
    }
}

criterion_group!(
    benches,
    bench_did_open,
    bench_did_change,
    bench_goto_def,
    bench_hover,
    bench_completion,
    bench_symbols_folding,
    bench_diagnostics,
);
criterion_main!(benches);
