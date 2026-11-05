# SDK Contract Readers (M1 parity seam)

`Contract::from_workspace_with_reader(&Arc<Workspace>, &Uri, ContractReader)`
selects how normalized document **values** are materialized.
`Contract::from_workspace` is unchanged and equivalent to
`ContractReader::Lossless`.

## Readers

- `ContractReader::Lossless` — full lossless CST decoding; historical
  behavior and the default.
- `ContractReader::Fast` — materializes document values independently
  through `suspect_syntax::fast::try_parse_fast`/`FastValue` and the same
  exact core-scalar conversion (`suspect_ir::fast::json`, which mirrors the
  lossless YAML path). JSON documents use the exact JSON parser. A `Fast`
  mode that encounters YAML outside the supported block-style subset
  (anchors, aliases, tags, directives, multi-line flow, tabs, …) declines
  with an explicit error; CST values are never silently substituted. There
  is no implicit `Auto` fallback policy.

## What Fast does *not* change

Reference resolution, reference targets, and source spans still come from
the lossless workspace sidecar. Graph identity (`SchemaId`), duplicate-key
detection, alias/depth guards, diagnostics, and default output are produced
by the same lossless indexing for both readers. No parse-speed or span
improvement is claimed for `Fast`.

## Invariants

For accepted inputs, canonical graph/source identity, duplicate decoded
keys, exact numbers (large/scientific values), YAML/JSON equivalence,
reference closure and recursion, effective HTTP metadata, and malformed
declaration handling are identical for both readers.
`tests/contract_readers.rs` pins block YAML vs JSON equivalents, key
reordering through split external schema documents, recursive references,
quoted scalars, invalid syntax, and one documented fast-decline construct
(YAML anchor).

## Defensive verification

Fast mode always compares independently materialized values with the lossless
sidecar. A divergence produces a `ContractError`; it never replaces Fast values
with CST output. This conservative parity check is part of admission, rather
than a throughput optimization.

## Non-goals

No alternate reference resolver; no corpus parse-time claims in this
milestone.
