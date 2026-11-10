# Conformance reads OpenAPI 3.0 schemas in their own dialect, including the nullable-reference idiom

The schema compiler speaks JSON Schema 2020-12. Before a 3.0 schema is compiled for a conformance check, `suspect-runtime` rewrites its two 3.0 spellings: `nullable: true` widens the declared `type` to include `null`, and a boolean `exclusiveMinimum`/`exclusiveMaximum` beside `minimum`/`maximum` becomes the numeric exclusive bound. `nullable: true` without a `type` in the same object, which 3.0.3 gives no effect, is read as "or `null`": it is how 3.0 descriptions spell a nullable `$ref` (`nullable` + `allOf: [$ref]`). In both dialects a required `writeOnly` property may be absent from a response and a required `readOnly` one from a request (ADR-0024).

## Considered Options

- Refuse the nullable-reference idiom, as SDK generation does without a compatibility profile (ADR-0012): rejected here. Generation refuses so it never emits a wrong type; a contract test that refuses would fail every response of every 3.0 description written that way, for a reason its author cannot act on.
- Compile 3.0 schemas as 2020-12 unchanged: rejected. A boolean `exclusiveMinimum` does not compile, so every reply of such an operation failed, and `nullable` properties rejected the `null` they declare.

## Consequences

The rewrite is confined to schema positions; `enum`, `const`, `default` and example values pass through. Only `3.0.x` documents get it: `nullable` in a 3.1 document stays an unknown keyword, as that dialect says.
