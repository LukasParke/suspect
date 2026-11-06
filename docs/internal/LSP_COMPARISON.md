# LSP Competitive Analysis: suspect vs vacuum vs Telescope

Comparison of `suspect-lsp` against the two most capable OpenAPI language
servers: **vacuum** (daveshanley/pb33f, Go) and **Telescope**
(sailpoint-oss/Telescope, Go + TypeScript). All claims verified against
source code, not documentation.

## Executive summary

| | suspect | vacuum | Telescope |
|---|---|---|---|
| Language | Rust | Go | Go (server) + TS (client) |
| Spec versions | OAS 2.0 (sniff), 3.0/3.1/3.2 | 2.0, 3.0, 3.1, 3.2 | 2.0, 3.0, 3.1, 3.2 |
| Arazzo | ✅ validation + expressions | ❌ | ✅ validation + runner |
| Overlay | ✅ apply engine | ❌ | ❌ |
| Parse engine | tree-sitter CST | libopenapi (kin-openapi) | go-tree-sitter |

suspect wins on: multi-spec-family coverage in one server, cycle-aware `$ref`
engine with classification, live-dirty-buffer navigation, incremental reparse
performance (1.5 ms on 6.4 MB), semantic tokens, formatting, workspace symbols,
and the breadth of its diagnostic pipeline (four merged sources).

vacuum wins on: ruleset ecosystem maturity (Spectral parity, remote rule sets),
auto-fix coverage from Spectral functions, and documentation links per
diagnostic. Its LSP is diagnostics-only — no hover, goto-def, completion,
rename, or any navigation feature is implemented.

Telescope wins on: completion richness (schema-driven snippets, security
schemes, media types, operation templates, vendor extensions), hover depth
(constraint tables, composition diagrams), call hierarchy, contract testing
integration, code-generation loop, and embedded-Markdown virtual documents.
It has known critical bugs (rename misses requestBodies/headers/links/examples,
UTF-16 offset errors for non-ASCII).

## Feature-by-feature comparison

### Diagnostics

| Feature | suspect | vacuum | Telescope |
|---|---|---|---|
| Structural validation | ✅ 22 semantic check codes | ✅ oas3-schema via libopenapi-validator | ✅ meta-schema via Navigator |
| Ruleset-based linting | ✅ 21 builtin rules, Spectral-compat rulesets | ✅ full Spectral parity + custom rules | ✅ Barometer rule packs + vacuum autoFix port |
| Arazzo validation | ✅ runtime expressions, goto targets, outputs | ❌ | ✅ workflow structure |
| Schema instance validation | ✅ JSON Schema 2020-12 compiler | ✅ kin-openapi-backed | ✅ Navigator-backed |
| Multi-source merge | ✅ 4 producers, single publish | Single source (rules only) | ✅ DiagnosticMux (5+ sources) |
| Debounce / cancellation | ✅ 150 ms + per-doc generation counter | ❌ fire-and-forget goroutine | ✅ dirty-node graph invalidation |
| Compute outside state lock | ✅ Arc-shared doc handles | N/A (goroutine) | ✅ graph pipeline |
| did_close cleanup | ✅ clears diagnostics + frees doc | ✅ drops doc | ✅ |
| Cross-file diagnostics | ✅ multi-file workspaces | ⚠ positions may mismatch root doc | ✅ |

### Navigation

| Feature | suspect | vacuum | Telescope |
|---|---|---|---|
| goto-definition ($ref) | ✅ chains, multi-file, dirty buffers | ❌ not implemented | ✅ incl. cross-file |
| goto-definition (operationId) | ❌ | ❌ | ✅ links/callbacks |
| references (reverse lookup) | ✅ workspace-wide, local+external | ❌ | ⚠ misses some component types |
| document highlights | ✅ $ref occurrences + declaration | ❌ | ⚠ O(n²), misses types |
| selection ranges | ✅ CST ancestor chain | ❌ | ❌ |
| Dirty-buffer navigation | ✅ live LowDoc for home doc | N/A | ⚠ legacy IndexCache reads |
| Hover (resolved target preview) | ✅ 40-line excerpt, fenced markdown | ❌ | ✅ constraint tables, composition |
| Call hierarchy | ❌ | ❌ | ✅ |

### Completion

| Feature | suspect | vacuum | Telescope |
|---|---|---|---|
| Implemented at all | ✅ context-aware keys + $ref candidates | ❌ advertised, returns nil | ✅ most comprehensive |
| $ref value candidates | ✅ all component sections, multi-file | ❌ | ✅ with resolve details |
| Context-aware key lists | ✅ by ancestor chain (root/path-item/operation/schema) | ❌ | ⚠ heuristic |
| Snippets | ❌ plain items only | ❌ | ✅ property patterns, op templates |
| Trigger characters | none advertised | none working | $ / # / : plus client-side |
| Security scheme names | ❌ | ❌ | ✅ with details |
| Media types | ❌ | ❌ | ✅ |
| HTTP status codes | ❌ | ❌ | ✅ |
| Vendor extensions | ❌ | ❌ | ✅ registry-based |
| Documentation on resolve | ❌ | ❌ | ✅ markdown |

### Refactoring

| Feature | suspect | vacuum | Telescope |
|---|---|---|---|
| Rename components | ✅ workspace-wide, deep refs, duplicate guard | ❌ | ⚠ misses some component types |
| prepareRename guard | ✅ component sections only | ❌ | ⚠ range heuristic issues |
| Quick fixes (code actions) | ✅ 8 fixes + fixAll | ⚠ docs-link command only | ✅ TODO placeholders, auto-operationId, path casing |
| Document formatting | ✅ canonical YAML/JSON | ❌ | ❌ (delegates to editor) |
| Format conversion (JSON↔YAML) | ✅ fmt command | ❌ | ✅ replace/copy variants |
| Source actions (sort tags etc.) | ❌ | ❌ | ✅ sort tags |

### Editor infrastructure

| Feature | suspect | vacuum | Telescope |
|---|---|---|---|
| Semantic tokens | ✅ full-document (8 types + definition modifier) | ❌ | ✅ ⚠ UTF-16 offset bug for non-ASCII |
| Folding ranges | ✅ mappings/sequences >3 lines | ❌ | ✅ |
| Workspace/symbol search | ✅ across loaded docs, filtered | ❌ | ✅ |
| Incremental sync | FULL sync (internal incremental reparse) | INCREMENTAL protocol sync | tree-sitter incremental |
| Inlay hints | ✅ $ref targets + property types | ❌ | ❌ |
| Embedded Markdown (vdoc) | ❌ | ❌ | ✅ virtual documents |

### Performance

| Metric | suspect | vacuum | Telescope |
|---|---|---|---|
| Incremental edit (1 KB into 6.4 MB) | ~1.5 ms (tree-sitter incremental) | Full re-lint each change | Tree-sitter incremental + graph cache |
| Full parse (6.4 MB stripe.yaml) | ~343 ms | ~500–800 ms (est.) | ~200–400 ms (est.) |
| node_at position lookup | O(log n) byte-range probe | N/A | O(n) scan (documented) |
| Diagnostics debounce | 150 ms + per-doc generation counters | None (goroutine free-for-all) | Graph-node version caching |
| Memory model | Zero-copy scalars, arena-indexed nodes | GC-managed Go structs | GC-managed + tree-sitter |

## What we should adopt from competitors

### From vacuum
1. **Diagnostic documentation links**: every diagnostic carries a
   `CodeDescription.HRef` linking to online rule docs. Trivial to add — we
   already have stable rule/check codes.
2. **Remote/custom ruleset loading**: vacuum supports fetching rulesets from
   URLs; we only load local files.
3. **Turbo mode**: skip JSON conversion when no schema-validation rule needs
   it — we could similarly lazy-init the schema validator only when needed.

### From Telescope
1. **Schema-driven completions**: our completions are key-list based; Telescope
   offers type-aware snippets (property patterns, operation templates).
   Medium-effort upgrade using our existing schema compiler.
2. **Security scheme / tag / status-code completions**: we don't offer these
   contexts at all.
3. **Call hierarchy**: `callHierarchy/incomingCalls` + `outgoingCalls` for
   refactoring impact analysis — we have the edge data already.
4. **Embedded Markdown virtual documents**: descriptions with CommonMark get
   full LSP treatment. High effort but differentiating.
5. **Contract testing integration**: running HTTP tests from the editor is a
   unique differentiator (out of scope for v1 but worth noting).

## What we have that they don't

1. **Three spec families in one server**: OpenAPI + Arazzo + Overlay with
   family-aware diagnostics, symbols, and lint packs. Neither competitor
   handles Arazzo or Overlay in their LSP.
2. **Cycle-aware $ref engine**: classified cycle census (legal recursion vs
   illegal) with exact paths, memoized resolution, no stack overflow. Vacuum
   has circular-reference detection but no LSP-visible reporting.
3. **Live-dirty-buffer navigation**: goto-def/hover/rename use the editor's
   unsaved buffer text, not disk state. Both competitors resolve against disk.
4. **Incremental reparse performance**: 1.5 ms for a 1 KB edit into a
   6.4 MB document (tree-sitter incremental + unchanged-text short-circuit).
5. **Semantic validation depth**: 22 structural checks + JSON Schema 2020-12
   compiler (unevaluated*, $dynamicRef) + Arazzo runtime-expression checking.
   Vacuum validates against the OAS meta-schema only; Telescope uses Navigator
   but without our check granularity.
6. **Zero-copy scalar architecture**: arena-indexed nodes, no per-value heap
   allocation during parse — enables the throughput numbers above.
7. **Document formatting**: neither competitor formats documents.
8. **Workspace symbol search**: neither competitor implements it (Telescope
   has documentSymbol but not workspace/symbol).
