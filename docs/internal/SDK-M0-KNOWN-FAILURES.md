# Historical M0 failure baseline

The archived corpus is pinned to OpenRouter revision
`db378a2a90d0167b9dca4f98b52074c54d249e1f`. Its historical CLI/native acceptance
report recorded **11 failed stages out of 98**. This is an inventory of that
pinned run, not the current SDK workflow or its expected failure count.

Evidence: `target/sdk-m0-m2-verified/corpus/report.json`. Every row has
the exact executable, arguments, working directory, exit status and log paths in
that report and in the milestone report's `knownFailures` entries.

| Input / failed stage | Classification | Concrete failure |
| --- | --- | --- |
| Public / validate | Upstream declaration | Boolean `exclusiveMinimum: true` in the OAS 3.1 video request; numeric keyword required, YAML line 26526 |
| Management / validate | Upstream declaration | Four security requirements reference undeclared `apiKey` |
| Management / lint | Upstream authoring/policy | Real enum/type mismatches and missing-operationId policy findings; omission itself is legal OpenAPI |
| Public / native-rust | Legacy compiler output | Invalid emitted inline object syntax and model/layout errors |
| Management / native-rust | Legacy compiler output | Invalid inline object types in rule/configuration models |
| Provider / native-rust | Legacy compiler output | Mislocated pattern newtypes and unboxed recursive model layout |
| Temporal / native-rust | Legacy compiler output | Inline configuration object emitted as invalid Rust type syntax |
| Public / native-go | Legacy compiler output | Undefined `Inline` types |
| Management / native-go | Legacy compiler output | Callers expect two results from one-result `doJSON` |
| Provider / native-go | Legacy compiler output | Repeated `Any` declarations, undefined models and unused imports |
| Temporal / native-go | Legacy compiler output | Undefined `Inline`, wrong `doJSON` result arity and unused import |

## Using this record

The immutable report stores exact command arrays, source/binary/tool digests,
native diagnostic fingerprints and logs. Those artifacts preserve the original
failure evidence. New runs use the current five-profile pipeline and fresh
report directories; stage definitions and totals are established by that run.

Current verification and the pending post-cleanup gate are in
[SDK-PROGRESS.md](SDK-PROGRESS.md) and [SDK-M3-M6-EXIT.md](SDK-M3-M6-EXIT.md).
