# Native SDK interface design

Each backend shares source semantics and validation obligations while allocating
an ecosystem-specific public interface. The [capability matrix](SDK-CAPABILITIES.md)
links to the implemented native guides. The self-contained TypeScript/Rust
[installed-package fixture](SDK-M2-CALLSITES.md) illustrates the compiler's public
plan and consumer interfaces.

## Interface principles

- Operation IDs, wire names, status/media alternatives and security declarations
  come from OpenAPI. Package identity, native imports and runtime options are
  explicit configuration.
- Ordinary valid calls use native typed inputs and results. Symbol allocation
  handles collisions consistently across models, codecs, operations and docs.
- Required/optional and absent/null states remain distinct. Exact numbers use
  representations that preserve the source's admitted values.
- Declared API errors are distinct from transport failures, unexpected
  responses, decoding errors, invalid requests and exhausted runtime budgets.
- HTTP response metadata accompanies decoded values. Unexpected bodies have
  bounded captures and explicit truncation rather than unbounded buffering.
- Caller-owned transports, cancellation and resource cleanup retain native
  lifetime semantics. Reusable clients do not imply undocumented retries.
- Comments, task guides and executable examples derive from the same typed plan
  and bind back to the physical source declaration.

## TypeScript and Rust

| Concern | TypeScript / JavaScript | Rust |
| --- | --- | --- |
| Calls | Async operation methods and native promises | Async methods with `Result` and operation error enums |
| Inputs | Typed objects grouped into path/query/headers/body, with separate call options | Generated input types, required-field constructors and optional-field builders |
| Models | Source wire properties, typed objects and discriminated unions | Structs/enums, explicit wire mappings and presence types |
| Errors | Runtime predicates narrow caught `unknown` values | Exhaustively matchable source-status and SDK failure variants |
| Transport | Fetch-compatible injection with cancellation | Transport trait and optional concrete HTTP adapters |
| Documentation | Declarations, TSDoc and TypeDoc | Native comments, Rustdoc and doctests |

TypeScript promises do not statically type rejected values. Consumers use the
generated guards for documented error alternatives. Rust model/codec-only users
can opt into HTTP support when needed.

## Configuration boundaries

Source descriptions can explain credential requirements, but prose does not
create machine-checkable roles/scopes. Environment defaults require an explicit
[credential policy](SDK-CREDENTIAL-ENV.md). Vendor extensions do not implicitly
activate pagination, retries, stream transformations or source reinterpretation.

Protocol and schema support is admitted per target. The
[HTTP protocol contract](SDK-HTTP-PROTOCOL.md),
[schema dialect rules](SDK-SCHEMA-DIALECTS.md) and native guides define those
boundaries. Native build/type/wire tests establish runtime behavior separately
from host-side planning tests.
