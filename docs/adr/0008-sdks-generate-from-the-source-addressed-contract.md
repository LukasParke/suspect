# SDKs are generated from a source-addressed Contract, not the platform IR

Every SDK backend is generated from `suspect_ir::contract::Contract`, through `backend::generate(Arc<Contract>, operations, TargetConfig)`. `IrSpec`, the normalized model used by the gateway, mock generator and CLI analysis, is not used. The Contract owns each reachable document once and identifies every schema, operation and parameter by `SourceId`: the physical retrieval URI plus a JSON Pointer. References stay graph edges, so recursion stays finite, and every diagnostic, codec and generated doc comment can point back to `file#pointer`. `$id`, OAS 3.2 `$self` and redirect aliases are indexed as metadata that resolves to a `SourceId`; they never replace it. Two physical resources claiming the same logical URI is an error.

## Considered Options

- Generating from `IrSpec`: normalization discards source positions, reference structure and dialect context, which located refusals and exact codecs need.
- Keying schemas by `$id`/`$self`: those names can be absent, duplicated or relocated, so identity and cache keys would depend on author-chosen strings.
- Keeping the earlier template-based SDK emitters over `IrSpec`: they were deleted. `suspect-gen` templates now serve only the `docs-md` preset and custom manifests.
