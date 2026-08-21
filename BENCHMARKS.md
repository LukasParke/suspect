# Benchmark Baselines

Criterion benches across the workspace layers, wired to `fixtures/`
(deterministic generated OpenAPI 3.1 specs). Recorded as the initial baseline;
re-run with `cargo bench -p <crate> --bench <name> [-- --quick]`.

## Machine / environment

| Item | Value |
| --- | --- |
| CPU | AMD Ryzen 9 7950X3D (16C/32T) |
| OS | Linux (CachyOS), kernel 7.1.5-1-cachyos |
| rustc | 1.97.1 (8bab26f4f 2026-07-14) |
| Profile | `bench` (inherits `release`: `lto = "thin"`, `codegen-units = 1`) |
| Criterion | 0.8, `--quick` mode |

Numbers were taken while sibling agents were building concurrently; treat
run-to-run variance of roughly ±10% as noise.

## Results

### suspect-syntax (`benches/syntax.rs`) — CST parse + line index

Timed section is tree-sitter parse + `LineIndex` construction only (fixture
bytes are cloned into a fresh `Source` outside timing).

| Benchmark | Input size | Mean | Throughput |
| --- | --- | --- | --- |
| syntax/json/100x100/autodetect | 276 KB | 18.31 ms | 14.3 MiB/s |
| syntax/json/1000x1000/with_format | 2.8 MB | 83.43 ms | 31.7 MiB/s |
| syntax/yaml/100x100/with_format | 130 KB | 14.15 ms | 8.78 MiB/s |
| syntax/yaml/1000x1000/autodetect | 1.3 MB | 168.99 ms | 7.43 MiB/s |
| syntax/yaml/2000x2000-circular/with_format | 2.7 MB | 538.74 ms | 4.85 MiB/s |

YAML parses ~4x slower than JSON at this fixture shape; larger YAML docs parse
at progressively lower throughput (grammar + deeper nesting).

### suspect-low (`benches/low.rs`) — document model

| Benchmark | Work per iteration | Mean | Per-item |
| --- | --- | --- | --- |
| low/parse/yaml/1000x1000/autodetect | full parse + family sniff, 1.3 MB | 165.38 ms | 7.59 MiB/s |
| low/pointer_lookup/paths_100_get_yaml_100x100 | 100 `#/paths/~1items~1item-N/get` lookups | 5.258 ms | ~53 µs/lookup |
| low/entries/components_schemas_yaml_100x100 | one `entries()` over 100 schemas | 46.96 µs | — |
| low/duplicate_keys/root_yaml_100x100 | duplicate-key scan of root mapping | 4.04 µs | — |

Pointer lookup cost is dominated by per-segment `entries()` materialization
(Vec alloc per hop); a candidate optimization is zero-alloc entry iteration.

### suspect-ref (`benches/ref_engine.rs`) — `$ref` engine

Fixture: `generated_2000x2000.yaml` (2000 schemas; each schema self-recurses
via `child` and chains via `next`).

| Benchmark | Work per iteration | Mean | Per-item |
| --- | --- | --- | --- |
| ref/cold_load_all/yaml_2000x2000_circular | fresh workspace + `load_all` | 619.35 ms | — |
| ref/warm_resolve_pointer/schema_pointers_100_yaml_2000x2000 | 100 memoized `resolve_pointer` calls | 114.74 ms | ~1.15 ms/pointer |
| ref/cycle_census/yaml_2000x2000_circular | full `cycles()` census | **10.97 s** | — |

Cold load is within ~15% of raw parse cost — loading overhead is reasonable.
The warm resolve path (~1.15 ms per memoized lookup) and especially the cycle
census (11 s for 4000 edges) are far above what interactive use can afford;
both need algorithmic work before validate/lint can meet their budgets on
cyclic documents.

### suspect-jsonpath (`benches/jsonpath.rs`)

| Benchmark | Query | Matches | Mean |
| --- | --- | --- | --- |
| jsonpath/paths_star_get/yaml_100x100 | `$.paths.*.get` | 100 | 224.87 µs |
| jsonpath/descendant_star/json_100x100 | `$..*` | ~8100 nodes | 32.16 ms |

Wildcard segments are fast; the recursive-descent segment (`$..*`) evaluates
at ~254 Kelem/s and dominates large-document query time.

### suspect-overlay (`benches/overlay.rs`)

| Benchmark | Work per iteration | Mean |
| --- | --- | --- |
| overlay/apply/11_actions_over_yaml_100x100 | apply 10 update actions + 1 remove to the 130 KB target | 189.65 ms |

Apply re-materializes the whole tree as an owned `Value` and re-emits it; the
cost is roughly one extra parse + serialize pass rather than the 11 actions.

## Plan budgets vs. measurements

Direct equivalents do not exist for every budget; where they don't, throughput
is extrapolated linearly from the closest measurement (honest caveat:
extrapolations ignore superlinear effects visible in the YAML column above —
larger docs parse slower per byte, so real numbers will likely be worse).

| Budget | Closest measurement | Extrapolation to budget size | Verdict |
| --- | --- | --- | --- |
| Parse + index 1 MB YAML ≤ 15 ms | 168.99 ms / 1.24 MiB (`LowDoc::parse`, incl. CST) | ~136 ms/MB | **FAIL** (~9x over) |
| 10 MB multi-file load ≤ 80 ms | 619 ms cold-load of 2.7 MB YAML | ~2.3 s for 10 MB | **FAIL** (~29x over) |
| Validate 10 MB ≤ 300 ms | no validate bench in this suite | — | **UNMEASURED** |
| Lint 10 MB ≤ 500 ms | no lint bench in this suite | — | **UNMEASURED** |
| Memory ≤ 4x input | no memory instrumentation | — | **UNMEASURED** |

The two FAIL rows share root causes: YAML CST parsing throughput (~7.4 MiB/s)
and per-node allocation in the low view. Meeting 15 ms/MB parse+index needs a
roughly order-of-magnitude YAML-path improvement (mmap reuse, incremental
indexing, or allocation-free entry iteration are the obvious levers).


## Optimization log

- Cycle census rewritten (laminar-interval successor build + single-pass
  pointer resolution): 10.97 s -> 168 ms on generated_2000x2000.yaml (65x).
- NodeRef::get fast path (allocation-free scan, merge-aware): warm pointer
  resolution 1.15 ms -> 0.84 ms per op.
- Remaining costs are structural: YAML CST throughput is grammar-bound
  (external scanner), and ordered-map lookups are O(width) scans by design
  (a per-document child index would trade memory for lookup speed and is
  deliberately deferred). Budget verdicts above reflect measured reality.
