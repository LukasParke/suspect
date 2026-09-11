# M2 native slice call sites

Callsites and coverage of the shared TS/JS–Rust installed-package gate.
The gate uses a self-contained create/update/list/get contract and independent
native consumers. See [SDK capabilities](SDK-CAPABILITIES.md) for other profiles.

## Gate

- Test: `crates/suspect-codegen/tests/m2_vertical.rs`
  `m2_vertical_canonical_contract_drives_both_native_packages`.
- Focused gate: `cargo test --locked -p suspect-codegen --test m2_vertical -- --include-ignored`.

## Source contract

- Fixture: `crates/suspect-codegen/tests/fixtures/m2/canonical.openapi.yaml`
  (OpenAPI 3.1; loaded through `WorkspaceBuilder` + `Contract::from_workspace`
  from its real path so source pointers stay intact).

| Required feature | Where in the contract |
| --- | --- |
| Create/update/list/get | `createWidget` POST `/widgets`, `updateWidget` PATCH `/widgets/{widget_id}`, `getWidget` GET `/widgets/{widget_id}`, `listWidgets` GET `/widgets` |
| Required / optional / non-null | `WidgetInput.name` required; `WidgetInput.amount` optional; all response members non-null unless declared nullable |
| Nullable | `Widget.meta`: `type: [string, "null"]`, optional; asserted `null` vs `"present"` on the wire in both consumers |
| Exact unbounded numeric | `number` members without bounds (`amount`, `WidgetPatch.amount`); examples `9007199254740993.000000000000000001`, `0.0000000000000000001`; asserted by string in both consumers |
| Source-tagged oneOf | `Widget.payload` = oneOf `StandardPayload`/`SecurePayload`, each branch carrying its own required `kind` const tag |
| Recursive optional model | `WidgetNode.child: WidgetNode` (object-guarded), `Widget.child` optional |
| Typed error responses | `Failure` for 401/422, 404, 400 responses on all four operations |
| Simple string path | `widget_id` path parameter, exercised with `a/b 雪!'()*` percent-encoding |
| Form scalar/array queries | `tag` scalar, `tags` array (explode), `labels` array (`explode: false`), bounded `limit` integer |
| Kept narrow | JSON media only, single bearer (`apiKey`), static HTTPS server URL; examples are source-valid; no custom `x-` extensions, no pagination inference |

## Pipeline callsites (both languages, one contract)

1. Selection: `Contract::operations()` filtered by the four `operationId`s
   (`m2_vertical.rs::selected`).
2. Planning: `suspect_codegen::typescript::http::plan_http` and
   `suspect_codegen::rust_http::plan_http` on the same `Arc<Contract>` and the
   same selected `SourceId`s.
3. Name location (public plan symbols only, no emitter-text parsing):
   `plan.operations()` for `function_name`/`input_type`/`success_type`/
   `error_type` (+ Rust `module_name`/`api_error_type`);
   `plan.codecs().models().symbols()` matched by source pointer for the inline
   request-body models (`m2_vertical.rs::assert_planned_names`, `body_symbol`).
4. Packaging: `suspect_codegen::typescript::package::emit_http`
   (`@suspect-fixtures/m2-canonical@0.1.0-m2.1`) and
   `suspect_codegen::rust_http::emit_http` (`m2-canonical-sdk@0.0.0`), both
   private, then `suspect_codegen::write_files`.
5. TS pack/install: pinned Node 22.23.1 / npm 10.9.8 (`SUSPECT_PACKAGE_NODE`),
   `npm ci` + `npm run build` + `npm pack --json`, tarball installed
   `--offline` into two separate consumers; installed bytes provenance-checked
   against emitted files.
6. Native consumers:
   - JS: `tests/fixtures/m2/js_consumer.mjs` — local `node:http` recording
     server with hand-authored response bytes; asserts recorded method/URL/
     auth/accept/body for all four operations, exact decimal strings, both
     oneOf branches, recursion absence, `null` meta, and that invalid inputs
     (`request-validation`/`request-representation`) never reach transport.
   - strict TS: `tests/fixtures/m2/ts_consumer.ts` — compiled with
     `--strict --exactOptionalPropertyTypes --noUncheckedIndexedAccess`,
     executed, plus `@ts-expect-error` negative type cases.
   - Rust: `tests/fixtures/m2/rust_consumer.rs` installed from the real
     `cargo package` tarball (extracted via `tar` into `vendor/`) — recording
     `Transport` with hand-authored request/response bytes; positive wire
      values, `Presence` omission/null, guarded recursion, tagged enum variants, and
     negatives: `RequestValidation`, `RequestRepresentation`,
     `ResponseDecoding`, `UnexpectedResponse`.
7. Native docs: `cargo test --doc` and `cargo doc --no-deps` under
   `RUSTFLAGS`/`RUSTDOCFLAGS=-Dwarnings`; actual installed TypeDoc HTML,
   model/codec/operation symbol coverage and source-binding/link checks.
   Strict TS positive/negative consumers compile on 5.5.4 and 5.9.3.
8. Cache policy: consumer verification target dirs are keyed by the SHA-256
   digest of the actual `.crate` archive (`m2_vertical.rs::digest`), so a
   same-version stale Cargo archive can never be reused after the generated
   bytes change; the doctest phase is serialized behind a mutex to avoid
   doctest cache races.

## Opt-in requirements and failure modes

- Missing `cargo`, `tar`, `node` or `npm` fails the gate (no silent skip).
- Pinned Node/npm versions and the emitted TypeScript version are asserted;
  mismatch fails.
- Rust MSRV is opt-in through `SUSPECT_NATIVE_RUST_TOOLCHAIN`
  (mapped to `RUSTUP_TOOLCHAIN`), matching the existing native gates.
- All network traffic is loopback (`127.0.0.1` recording server /
  in-process transport); no real external API calls are made.

## Approved native call style

```ts
const created = await client.createWidget({ body: { name: "alpha" } });
if (created.data.payload.kind === "standard") {
  console.log(created.data.payload.text);
}
```

```rust
let input = CreateWidget::new(WidgetInput::new("alpha".into()));
let CreateWidgetSuccess::Status200(response) = client.create_widget(input).await?;
```

The user approved this object-input/typed-constructor baseline in
`SDK-M0-M2-DX-APPROVAL.md`. Native consumers independently verify every call's
bytes, typed errors, mutable input validation and positive/negative types.
`sdk_example_packages` separately executes the shared provenance-bearing
example artifacts from both installed packages.
