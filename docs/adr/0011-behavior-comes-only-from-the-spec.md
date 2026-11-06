# API behavior comes only from OpenAPI; conveniences need an explicit `sdk_defaults` policy

Endpoints, schemas, wire names, security and examples come only from the OpenAPI description. Package name, version and toolchains come from `TargetConfig`, and `package_version` is independent of `info.version`. Nothing is inferred from operation or parameter names (`list`, `cursor`, `data`), from prose, or from another vendor's `x-` extensions such as `x-speakeasy-*`; those are kept but never activate behavior. Convenience behavior (pagination detection, the `<ENV_PREFIX>_API_KEY` convention, the OAuth token lifecycle) runs only when the caller supplies the closed, versioned `sdk_defaults` policy. That policy is part of the generation fingerprint and reported per operation, and without it output is byte-identical. We chose this so that every generated behavior traces to the spec or to an opt-in a reviewer can see.

## Consequences

`sdk_defaults` pagination detection is the one sanctioned use of name-based heuristics, allowed because the application opted in. The single-target `suspect codegen` command does not take `sdk_defaults`; packages with these helpers come from a session config or the library API.
