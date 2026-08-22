//! Arazzo benchmarks for `suspect-arazzo`: document model construction +
//! structural/cross-reference validation over a generated 2-workflow ×
//! 5-step Arazzo 1.0 document, and runtime-expression parsing throughput.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use suspect_arazzo::{ArazzoDoc, parse_embedded, validate_arazzo};
use suspect_low::LowDoc;
use suspect_source::{Source, Uri};

/// Builds an Arazzo 1.0 document with `wf` workflows of `steps` steps each.
/// Every step carries a success criterion and an output; later workflows
/// reference earlier outputs through `$workflows...` expressions.
fn arazzo_yaml(wf: usize, steps: usize) -> String {
    let mut s = String::from(
        "arazzo: 1.0.0\n\
         info:\n  title: bench arazzo\n  version: 1.0.0\n\
         sourceDescriptions:\n\
         \x20 - name: petstore\n\
         \x20   type: openapi\n\
         \x20   url: ../fixtures/petstore-expanded.yaml\n\
         workflows:\n",
    );
    for w in 0..wf {
        s.push_str(&format!("  - workflowId: flow{w}\n    steps:\n"));
        for t in 0..steps {
            s.push_str(&format!(
                "      - stepId: step{w}_{t}\n\
                 \x20\x20\x20\x20\x20\x20\x20 operationPath: $sourceDescriptions.petstore./pets\n\
                 \x20\x20\x20\x20\x20\x20\x20 successCriteria:\n\
                 \x20\x20\x20\x20\x20\x20\x20\x20\x20 - condition: $statusCode == 200\n\
                 \x20\x20\x20\x20\x20\x20\x20 outputs:\n"
            ));
            if w == 0 {
                s.push_str("          petId: $response.body#/id\n");
            } else {
                s.push_str(&format!(
                    "          chained: $workflows.flow0.outputs.petId\n\
                     \x20\x20\x20\x20\x20\x20\x20\x20\x20 prev: $workflows.flow{}.outputs.chained\n",
                    w - 1
                ));
            }
        }
    }
    s
}

fn bench_arazzo(c: &mut Criterion) {
    let text = arazzo_yaml(2, 5);
    let doc = LowDoc::parse(
        Uri::parse("memory://bench-arazzo.yaml").expect("valid synthetic URI"),
        Source::from_vec(text.into_bytes()),
    );

    // Warmup + sanity: the generated document must model and validate
    // without panicking.
    let parsed = ArazzoDoc::new(&doc);
    let diags = validate_arazzo(&parsed);
    eprintln!(
        "[arazzo] generated doc: {} diagnostics (warmup)",
        diags.len()
    );

    let mut group = c.benchmark_group("arazzo");
    group.bench_function("new_and_validate_2_workflows_x_5_steps", |b| {
        b.iter(|| {
            let parsed = ArazzoDoc::new(black_box(&doc));
            black_box(validate_arazzo(&parsed).len())
        });
    });

    // Ten embedded runtime expressions in one string, parsed 1000 times per
    // iteration.
    const EMBEDDED: &str = concat!(
        "url={$url} status={$statusCode} ",
        "id={$request.path.id} q={$request.query.q} h={$request.header#/X-Req}; ",
        "body={$request.body#/id} in={$inputs.limit}; ",
        "resp={$response.body#/name}; ",
        "prev={$steps.step0_0.outputs.petId} wf={$workflows.flow0.outputs.chained}"
    );
    assert_eq!(
        EMBEDDED.matches("{$").count(),
        10,
        "ten embedded expressions"
    );

    group.bench_function("parse_embedded_10_exprs_x1000", |b| {
        b.iter(|| {
            for _ in 0..1000 {
                black_box(parse_embedded(EMBEDDED).len());
            }
        });
    });

    group.finish();
}

criterion_group!(benches, bench_arazzo);
criterion_main!(benches);
