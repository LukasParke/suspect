# readOnly/writeOnly relax only requiredness and never strip fields

Following JSON Schema 2020-12 annotation semantics, a Request view (for `readOnly`) or Response view (for `writeOnly`) only relaxes a required property's presence requirement. It does so only when the annotation provably applies at that position: on the property itself or through unconditional `$ref` chains. Every declared field stays in the type, and supplied values are still validated and sent. Annotations reachable only through `allOf`/`anyOf`/`oneOf`/`if`/`not` are a located planning error, not a guess. OpenAPI 3.0's reversed directional `required` is refused (ADR-0016). Most generators drop `readOnly` fields from request models; we do not, because that changes what the wire accepts.

## Consequences

Only the TypeScript adapter emits directional views today (`typescript/directional.rs`). Other adapters keep neutral models, and adding views to them must reuse this policy.
