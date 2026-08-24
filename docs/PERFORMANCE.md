# Performance Profiling Report & Plan of Attack

Corpus: `corpus/stripe.yaml` (6.1 MB, 589 operations, 88 530 nodes,
1 440 component schemas). All measurements are release-build medians on a
Ryzen 9 7950X3D (32 HW threads).

---

## 1. Current State vs Targets

| Metric | Target | Measured | Status |
|---|---|---|---|
| Parse + IR cold | < 10 ms | **7.5–9.5 ms** | ✅ PASS |
| Lint p95 | ≤ 50 ms | **41–46 ms** | ✅ PASS |
| Validate p95 | ≤ 50 ms | **39–51 ms** | ⚠️ borderline |
| Gateway startup | < 1 s | **~1014 ms** | ❌ MISS |
| Gen throughput | ≥ 1 GB/s | **~1.65 GB/s** | ✅ PASS |

---

## 2. Phase-by-phase cost attribution

### 2.1 Parse + IR (`IrSpec::from_file`, fast path)

| Stage | Median | % of total |
|---|---|---|
| `fs::read` | ~1.0 ms | 11% |
| `try_parse_fast` | ~4.6–5.1 ms | 52% |
|   ├─ guards + UTF-8 check | ~0.6 ms | |
|   ├─ `scan_lines` | ~1.1 ms | |
|   └─ bounds collection + parallel parse | ~3.5 ms | |
| `ir_from_fast` | ~1.9–2.2 ms | 22% |

**Total: 7.5–9.5 ms** (variance from rayon scheduling)

Remaining costs in `try_parse_fast`:
- Per-scalar `String` allocation via `decode_scalar` / `decode_key`
  (~100k+ small heap allocs per file).
- `entries.extend(part)` merge in `parse_map` copies vectors.
- Rayon scheduling overhead for 1861 sections (many are tiny).

Remaining costs in `ir_from_fast`:
- `json()` clones every key String into the serde_json map.
- `collect_local_refs` re-walks each schema JSON after materialization.

**Headroom estimate**: 3–5 ms achievable by:
1. Interned key pool (`Arc<str>` for repeated keys like `"type"`,
   `"properties"`) → eliminates ~40% of allocs.
2. Fused single-pass bytes→IR that skips the intermediate `FastValue` tree
   for leaf scalars (borrow `&[u8]` slices directly).
3. mmap instead of read (saves the kernel copy).

### 2.2 Lint

| Stage | Cost | Notes |
|---|---|---|
| Plan compile | ~0.02 ms | negligible |
| Phase A (section collection) | ~3 ms | 1861 sections |
| Phase B (parallel sweep) | **~20–30 ms** | dominates |
| Apply (rule dispatch) | **~10–13 ms** | second |
| Generic rules | ~0.02 ms | |
| Finalize | ~0.03 ms | |
| **Total** | **~35–46 ms** | |

Phase B walks every section against every matched rule. The top rules by
node count are:
- `no-$ref-siblings`: 3895 nodes
- `typed-enum`: 4177 nodes

These two alone account for most of phase B. Both do per-node string
comparisons and optional regex matching. A SIMD-assisted scanner or a
precomputed bloom-filter-per-section could skip sections that can't match.

**Headroom estimate**: 15–25 ms achievable by:
1. Rule-level bloom filters: pre-compute which section keys could match
   each rule; skip entire sections.
2. Allocation diet: finding construction currently allocates a String per
   match even when the finding is later deduplicated.

### 2.3 Validate

Sequential check costs (from `SUSPECT_PROFILE=1`):

| Check | Sequential cost | Parallel wall contribution |
|---|---|---|
| examples | ~40–49 ms | highest bucket |
| parameters::fields | ~36–42 ms | |
| paths::templates | ~26 ms | |
| required_path_params | ~25 ms | |
| duplicate_header_params | ~24 ms | |
| schemas walk | ~12 ms (was 7000+ before parallelization) | |
| responses::descriptions | ~13 ms | |
| everything else | < 10 ms each | |

With parallel buckets, wall time ≈ max(bucket) not sum. Currently
~39–45 ms p95 because the heaviest bucket still takes that long.

**Headroom estimate**: 10–20 ms achievable by:
1. Splitting the `examples` check into finer sub-sections (it walks all
   media types across all responses).
2. Same for `parameters::fields`.
3. Sharing one traversal across related checks (e.g. all three parameter
   checks in a single pass).

### 2.4 Gateway startup

| Phase | Stripe corpus | Small spec |
|---|---|---|
| `load_ir` | **1004 ms** | 1.6 ms |
| `mock::compile_all` | 9.6 ms (589 ops) | < 1 µs |
| Router registration | negligible | |
| **Total** | **~1014 ms** | **~1.7 ms** |

**Root cause**: `load_ir` calls `load_workspace` which does a whole-
directory scan, builds a full `Workspace` with tree-sitter parsing of
every YAML file, then runs `IrSpec::from_workspace`. This is the legacy
CST pipeline (~1000 ms) — NOT the fast path.

**Fix**: `IrSpec::from_file(path)` already exists at ~7–9 ms. Switching
`load_ir` to use it drops startup to ~20 ms total. The workspace is only
needed for *validate* mode (NodeRef-based contract checking); mock,
replay, and record modes work purely from IR + cassettes.

### 2.5 Gen throughput

Current: **~98 971 MB/min = 1.65 GB/s** (docs-md preset on stripe).

The pipeline is: ctx build (one-pass, memoized) → minijinja render
(printer-style templates over precomputed strings) → content-hash compare →
write.

Per-file render p95 is 0.28–0.45 ms — well under the 1 ms budget.
Throughput is limited by minijinja's interpolation machinery (string
formatting, escape scanning) rather than data transformation.

**Headroom estimate**: 2–3 GB/s achievable by writing specialized Rust
emitters for shipped presets that bypass minijinja entirely (pure
`fmt::Write` with pre-reserved buffers). However, this sacrifices the
"user templates override presets" property unless both paths are
maintained.

---

## 3. Cross-cutting opportunities

| Lever | Expected gain | Effort |
|---|---|---|
| mimalloc/jemalloc global allocator | 10–30% on alloc-heavy paths | Low (add dep + `#[global_allocator]`) |
| PGO (`-Cprofile-use`) | 5–15% across the board | Medium (build infrastructure) |
| Fat LTO (replace thin) | 2–5% | Low (change one line) |
| `memchr` for byte scans in fast parser | ~0.5–1 ms on stripe | Low (dep exists) |

---

## 4. Ranked plan of attack

Ranked by impact × feasibility. Each item lists expected measured
improvement and effort.

### Tier 1 — immediate wins (hours)

| # | Action | Expected improvement | Why |
|---|---|---|---|
| T1 | **Switch gateway `load_ir` to `IrSpec::from_file`** for mock/replay/record modes | Startup: 1014 ms → ~20 ms on stripe | One-line change; workspace only needed for validate mode |
| T2 | **Add mimalloc as global allocator** for suspect-cli binary | 10–30% across lint/validate/parse/gen | Single attribute + dependency |
| T3 | **Combine guard scans** in try_parse_fast (single pass for tab+CR+utf8) | ~0.5 ms saved per parse | Trivial loop fusion |

### Tier 2 — structural improvements (days)

| # | Action | Expected improvement | Why |
|---|---|---|---|
| T4 | **Interned key pool** in fast parser (`Arc<str>` for repeated object keys) | Parse: −1–2 ms; gen: −5–10% | ~40% of scalar allocs are duplicate keys ("type", "properties", etc.) |
| F5 | **Fuse validate parameter checks** (fields + required_path + duplicate_header in one traversal) | Validate: −10–15 ms wall | Three traversals of the same tree collapse into one |
| T6 | **Fuse lint rule pairs** (no-$ref-siblings + typed-enum share 80% of nodes) | Lint: −5–10 ms | Two walks of overlapping node sets become one |

### Tier 3 — deeper architectural (week+)

| # | Action | Expected improvement | Why |
|---|---|---|---|
| T7 | **Zero-copy FastValue** (borrow `&[u8]` from input buffer for plain scalars) | Parse: −1.5–2 ms; IR: −0.5–1 ms | Eliminates ~60k String allocs per stripe-sized file; requires lifetime threading through FastValue |
| T8 | **Validate mode value-based checking** (port NodeRef-based checks to operate on `serde_json::Value` from IrSpec) | Gateway validate-mode startup: 1000 ms → ~50 ms | Removes workspace dependency from all gateway modes; larger refactor |
| T9 | **Specialized Rust emitters for docs-md/ts-sdk/rust-sdk presets** (bypass minijinja for shipped presets) | Gen: 1.65 → 3+ GB/s | Only worth it if gen throughput becomes a bottleneck in practice |
| T10 | **SIMD-assisted line scanning** (memchr for newline/indent detection in scan_lines) | Parse: scan_lines 1.1 ms → ~0.3 ms | memchr crate already in tree |

---

## 5. Projected end state after full plan

| Metric | Now | After Tier 1 | After Tier 2 | Ceiling |
|---|---|---|---|---|
| Parse + IR cold | 7.5–9.5 ms | ~7–9 ms | ~5–7 ms | ~2–4 ms |
| Lint p95 | 41–46 ms | 37–42 ms | ~27–32 ms | ~5–15 ms |
| Validate p95 | 39–51 ms | 39–51 ms | ~25–35 ms | ~10–20 ms |
| Gateway startup | 1014 ms | **~20 ms** | ~20 ms | ~10 ms |
| Gen throughput | 1.65 GB/s | 1.8–2.0 GB/s | 2.0–2.5 GB/s | 3–6 GB/s |
