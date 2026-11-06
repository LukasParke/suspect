# Generated SDKs do not aim for drop-in parity with existing published SDKs

We compared our output against the Speakeasy-generated `@openrouter/sdk` (npm) and `openrouter` (PyPI) packages and kept suspect's own interface. Responses are body-first `ApiResponse` values that still expose status and media. Failures use one `SdkError` family with per-operation `ApiError` guards. Nothing is retried automatically (ADR-0025), there is no global mutable client state, and parameter names stay as they are on the wire.

## Considered Options

- Configurable parity through new `sdk_defaults` policies for globals, parameter renames, resource nesting, retries, enum fallbacks and native datetimes: proposed and not built. If it is revisited, parity must come from general declared policy, never from generator logic that knows about one vendor (ADR-0012).

## Consequences

Users migrating from those SDKs need a migration guide (for example, read `.data`, handle retries themselves).
