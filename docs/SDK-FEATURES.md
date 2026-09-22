# Generated-SDK feature matrix

Machine-generated from [`crates/suspect-codegen/src/features.rs`](../crates/suspect-codegen/src/features.rs). Every claim is verified against the acceptance test named in the evidence column; the sync test (`tests/feature_manifest.rs`) fails if this file, the registry, or the evidence drift apart. An unclaimed cell records test debt, not necessarily absence of implementation.

| Feature | cpp | csharp | dart | go | java | kotlin | php | python | ruby | rust | swift | typescript | Evidence |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **Pagination** (pagination) | yes | yes | yes | yes | yes | yes | yes | yes | yes | yes | yes | yes | `tests/cpp_pagination.rs`, `tests/csharp_pagination.rs`, `tests/dart_pagination.rs`, `tests/go_pagination.rs`, `tests/java_pagination.rs`, `tests/kotlin_pagination.rs`, `tests/php_pagination.rs`, `tests/python_pagination.rs`, `tests/ruby_pagination.rs`, `tests/rust_pagination.rs`, `tests/swift_pagination.rs`, `tests/typescript_pagination.rs` |
| **OAuth 2.0 client credentials** (oauth2-client-credentials) | yes | yes | yes | yes | yes | yes | yes | yes | yes | yes | yes | yes | `tests/cpp_oauth.rs`, `tests/csharp_oauth.rs`, `tests/dart_oauth.rs`, `tests/go_oauth.rs`, `tests/java_oauth.rs`, `tests/kotlin_oauth.rs`, `tests/php_oauth.rs`, `tests/python_oauth.rs`, `tests/ruby_oauth.rs`, `tests/rust_oauth.rs`, `tests/swift_oauth.rs`, `tests/typescript_oauth.rs` |
| **401 auth replay** (auth-replay-401) | yes | yes | yes | yes | yes | yes | yes | yes | yes | yes | yes | yes | `tests/cpp_oauth.rs`, `tests/csharp_oauth.rs`, `tests/dart_oauth.rs`, `tests/go_oauth.rs`, `tests/java_oauth.rs`, `tests/kotlin_oauth.rs`, `tests/php_oauth.rs`, `tests/python_oauth.rs`, `tests/ruby_oauth.rs`, `tests/rust_oauth.rs`, `tests/swift_oauth.rs`, `tests/typescript_oauth.rs` |
| **Typed server-sent events** (typed-stream-events) | yes | yes | yes | yes | yes | yes | yes | yes | yes | yes | yes | yes | `tests/cpp_stream_typed.rs`, `tests/csharp_stream_typed.rs`, `tests/dart_stream_typed.rs`, `tests/go_stream_typed.rs`, `tests/java_stream_typed.rs`, `tests/kotlin_stream_typed.rs`, `tests/php_stream_typed.rs`, `tests/python_stream_typed.rs`, `tests/ruby_stream_typed.rs`, `tests/rust_stream_typed.rs`, `tests/swift_stream_typed.rs`, `tests/typescript_stream_typed.rs` |
| **Incoming webhooks** (incoming-webhooks) | yes | yes | yes | yes | yes | yes | yes | yes | yes | yes | yes | yes | `tests/cpp_incoming.rs`, `tests/csharp_incoming.rs`, `tests/dart_incoming.rs`, `tests/go_incoming.rs`, `tests/java_incoming.rs`, `tests/kotlin_incoming.rs`, `tests/php_incoming.rs`, `tests/python_incoming.rs`, `tests/ruby_incoming.rs`, `tests/rust_incoming.rs`, `tests/swift_incoming.rs`, `tests/typescript_incoming.rs` |
| **Environment-variable credentials** (env-var-credentials) | yes | — | yes | yes | yes | yes | yes | yes | yes | yes | yes | yes | `tests/cpp_credential_env.rs`, `tests/dart_credential_env.rs`, `tests/go_credential_env.rs`, `tests/java_credential_env.rs`, `tests/kotlin_credential_env.rs`, `tests/php_credential_env.rs`, `tests/python_credential_env.rs`, `tests/ruby_credential_env.rs`, `tests/rust_credential_env.rs`, `tests/swift_credential_env.rs`, `tests/typescript_credential_env.rs` |
| **User-agent attribution** (user-agent-attribution) | yes | yes | yes | — | yes | yes | yes | yes | yes | yes | yes | yes | `tests/cpp_attribution.rs`, `tests/csharp_attribution.rs`, `tests/dart_attribution.rs`, `tests/java_attribution.rs`, `tests/kotlin_attribution.rs`, `tests/php_attribution.rs`, `tests/sdk_attribution.rs`, `tests/ruby_attribution.rs`, `tests/swift_attribution.rs` |

## Feature notes

- **Pagination** — Detected pagination is compiled into native page iterators with `patterns` and `page_size` overrides.
- **OAuth 2.0 client credentials** — Token acquisition, refresh, and revocation/introspection runtimes with OIDC discovery.
- **401 auth replay** — A 401 response triggers one credential refresh and request replay before surfacing the failure.
- **Typed server-sent events** — `text/event-stream` responses compile into typed event iteration instead of raw strings.
- **Incoming webhooks** — Webhook payloads compile into decoders and constructors with strict package compilation.
- **Environment-variable credentials** — Security schemes resolve credentials from documented environment variables at client construction.
- **User-agent attribution** — Requests carry the `ua/v1` attribution grammar identifying the SDK, engine, and application.

## Known evidence gaps

- **Environment-variable credentials**: no acceptance evidence for csharp.
- **User-agent attribution**: no acceptance evidence for go.
