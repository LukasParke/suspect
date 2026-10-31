# Depth analysis — where suspect is deep, where it is shallow, and what the next iteration should be

A depth analysis asks a different question from a feature list: for each
capability, is the implementation *thin* (a correct-shaped surface over
approximations) or *deep* (the semantics, the evidence, and the failure
modes are all handled)? Thin is not bad; it is honest work in progress.
The point of naming it is so the next iteration is chosen deliberately
rather than by momentum.

Measurements below are from this repository at `370ff69`+ (main after PR
#12 and the workspace-scale work), reproducible with the commands shown.

## How the surface actually looks

| Crate | src lines | test lines | test:src |
|---|---|---|---|
| suspect-codegen | 172,432 | 139,757 | 0.81 |
| suspect-lsp | 26,100 | 819 | 0.03 |
| suspect-cli | 10,825 | 4,620 | 0.43 |
| suspect-schema | 9,774 | 7,516 | 0.77 |
| suspect-ir | 7,446 | 5,751 | 0.77 |
| suspect-test | 5,434 | 0 (inline) | 0.00 |
| suspect-ref | 4,770 | 2,539 | 0.53 |
| suspect-lint | 4,226 | 580 | 0.14 |
| suspect-gateway | 3,902 | 0 (inline) | 0.00 |
| suspect-validate | 3,763 | 2,607 | 0.69 |
| suspect-arazzo | 1,976 | 542 | 0.27 |
| suspect-low | 1,381 | 549 | 0.40 |
| suspect-overlay | 1,229 | 478 | 0.39 |

Two structural facts stand out. 19 of 26 crates enforce
`#![deny(missing_docs)]`, so the public surface is documented by
construction. And 5 `TODO`/`FIXME`/`unimplemented!` sites remain in
non-test code — the codebase is not carrying a backlog of abandoned work.

The third fact is the interesting one: **test mass is concentrated where
the semantics are hardest** (codegen, schema, ir, ref, validate) and thin
where the surface is widest (lsp 0.03, lint 0.14, arazzo 0.27). That is
mostly a *counting artifact* — LSP and gateway tests live inside
`src/` as `#[cfg(test)]` modules and are not counted above. Corrected for
that, the real gaps are narrower than the table suggests, and are named
individually below.

## Capability-by-capability

### Deep

**JSON Schema 2020-12 (`suspect-schema`).** A real compiler: `$ref`
resolution with lazy memoized compilation, a document-root fallback for
schemas compiled from inside a larger document, 2020-12 resource and
`$dynamicRef` support, owned checked programs with nine scoped
applicators, finite evaluation budgets, and conformance vectors. This is
the foundation everything else trusts, and it is the most defensively
built part of the codebase.

**`$ref` engine (`suspect-ref`).** Workspace graph, cycle census
distinguishing legal schema recursion from illegal unresolvable loops, and
memoized resolution at 63k-line scale. The test:src ratio of 0.53 with no
losses is the right shape.

**Lossless parsing (`suspect-low`, `suspect-syntax`).** Vendored
tree-sitter grammars with a documented upstream patch for the >32k-line
YAML corruption bug, zero-copy scalars, arena-indexed nodes, YAML 1.2
typing. The recent work on comment-preserving formatting and
`x-suspect-cyclic` markers both lean on this being genuinely lossless.

**Contract-driven SDK generation (`suspect-codegen`).** 172k lines
across twelve backends with 0.81 test:src, a single contract pipeline,
per-backend behavioral fragments authored from real output, and a claims
manifest that fails CI when a claim loses its evidence. The fragments and
the coverage coupling are the strongest verification story in the project.

**Publishing, release, impact (`suspect-cli`).** Newest and least
battle-tested, but each command has the shape of a deep module: a
testable core function taking plain arguments, a thin dispatch wrapper,
and per-command CLI tests that assert real exit codes and real artifact
contents.

### Shallow, with the specific missing depth

**LSP (`suspect-lsp`, 26k lines).** Breadth is genuinely large — the
method surface is complete. Depth is concentrated in diagnostics and
navigation; the *code actions, semantic tokens, inlay hints, and folding*
are generated from templates and heuristics rather than from a model of
what the document means at that node. The 17 quick fixes were authored
one by one. There is no LSP-level conformance suite (only per-feature
tests), so an LSP client regression would be caught by our tests but
not by a protocol-shaped harness.

**Lint (`suspect-lint`, 40 OAS rules + 10 security + 4 overlay/arazzo).**
The rule engine is real, but the ruleset format is a thin subset of
Spectral's: no custom-function loading, no `extends` of external
rulesets, no per-directory overrides. Rules are also asserted one at a
time in `tests/packs.rs` with hand-written fixtures rather than through a
shared corpus, so rule interaction (a doc that trips 12 rules) is
untested.

**Gateway (`suspect-gateway`).** Five modes over one router, a stateful
CRUD mock, deterministic fault injection, a playground, and now a served
contract. Depth is missing where it costs most in production: there is no
request-level rate limiting, no TLS, no graceful shutdown, no request
body size ceiling, and the fault injection is percentage-based rather than
keyed to a test id. The modes are well factored; the operational surface
is a demo surface.

**Arazzo execution (`suspect-test`).** Steps execute sequentially, and
`dependsOn` is validated but does not change execution order (the
executor still runs steps in document order). Message steps work over a
file broker and a loopback; there is no real broker adapter. The
executor is a linear interpreter, not a scheduler.

**Arazzo/Overlay models (`suspect-arazzo`, `suspect-overlay`).** The
1.1 validation layer is thorough and the standards conformance is real.
But the models are view-based: no owned round-trip model of an Arazzo
document, so "load, modify, save" is not yet a first-class operation the
way Overlay synthesis now is.

**Docs generator.** Three styles, one content model, SvelteKit verified by
an actual `npm install && vite build`. Depth is missing in the
component library: the generated site is a reference, not a system with a
request builder, runnable examples, or maintained prose importing
generated content.

**Schema engine surface (`suspect-lsp`, `suspect-cli` integrations).**
`format` assertion, document-root `$ref` fallback, and the owned-program
applicators exist and are tested, but the *default* path still inlines
component graphs at depth 8 in the response-validation path, and
`compile_v2` native adoption is tracked rather than done. The foundation
is deeper than its adoption.

## The three risks I would rank first

1. **Codegen breadth outruns codegen evidence.** Twelve backends at 0.81
   test:src is good, but the *native* acceptance suites are opt-in
   (`#[ignore]`) and run only when a toolchain is present. A regression
   in, say, the PHP emitter on a machine without PHP is caught only by
   static tests. This is the largest surface in the codebase and the one
   with the thinnest continuous evidence.

2. **Configuration now spans three surfaces.** `.suspect.yaml` is read by
   the CLI and by the LSP, and the project manifest is read by the CLI.
   Three configuration surfaces with no single schema is a coherence risk
   that will bite at scale. The first version of config precedence was
   already wrong twice (flags losing to config) before the tests caught it.

3. **A correctness bug class lives in the response-validation inliner.**
   `crates/suspect-test/src/exec.rs` still inlines component graphs with a
   depth-8 cap and a permissive collapse beyond it, even though the
   document-root `$ref` fallback now makes that unnecessary. It is a
   known approximation, and it is in the path that decides whether a
   contract test passes.

## What the next iteration should be

In priority order, weighted by evidence-per-effort:

1. **Close the configuration coherence gap.** One schema, one loader,
   one precedence rule (`flag > env > config > default`), read
   identically by CLI, LSP, and the project manifest. Precedence already
   bit us twice; formalize it before it spreads.

2. **Make the response-validation path deep.** Replace the depth-8
   inliner in the executor with the document-root `$ref` fallback the
   compiler now has. One file, removes an entire class of
   false-pass/false-fail, and deletes an approximation.

3. **Continuous native evidence for codegen.** Promote the per-backend
   acceptance suites from opt-in to a scheduled (not blocking) job with
   a coverage matrix published as an artifact, so "which backend is
   verified against which toolchain version on which date" is a fact we
   can read rather than infer from whether CI happened to run.

4. **Execute `dependsOn`.** The 1.1 dependency graph is validated but
   the executor ignores it. This is the difference between "Arazzo 1.1
   documents validate" and "Arazzo 1.1 documents run as written."

5. **A shared contract runtime.** The gateway, the Arazzo runner,
   generated server stubs, and the docs playground each interpret the
   contract slightly differently. One runtime, consumed by all four,
   would make "the toolchain agrees with itself" a property rather than
   a hope.

Items 1 and 2 are small and eliminate real risk. Items 3–5 are the
difference between a broad toolkit and a platform that holds together
under a large portfolio.

**Status:** all five are now built. Configuration is one schema
(`suspect-config`) read by the CLI and the language server; the depth-8
inliner is deleted and recursive response schemas validate natively;
`dependsOn` is executed by a scheduler; `suspect evidence` plus a daily
workflow publish the SDK verification matrix; and `suspect-runtime` owns
the contract decision for the gateway and the executor, replacing two
bespoke validators. The remaining depth work — LSP first — is planned in
[LSP-DEPTH-ENRICHMENT.md](LSP-DEPTH-ENRICHMENT.md).

## Reproducing these numbers

```sh
for c in crates/*/; do
  n=$(find "$c/src" -name '*.rs' 2>/dev/null | xargs cat 2>/dev/null | wc -l)
  t=$(find "$c/tests" -name '*.rs' 2>/dev/null | xargs cat 2>/dev/null | wc -l)
  printf '%-28s %7d src %7d test\n' "$c" "$n" "$t"
done
grep -rln 'deny(missing_docs)' crates/*/src/lib.rs | wc -l
grep -rn 'TODO\|FIXME\|unimplemented!' crates/*/src --include='*.rs' \
  | grep -v test | wc -l
```
