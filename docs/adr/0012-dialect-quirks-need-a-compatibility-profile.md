# Non-standard idioms are accepted only through named, versioned compatibility profiles

Many real specs depart from their declared OpenAPI version: OAS 3.0 `nullable` inside a 3.1 document, `/keys/:hash` colon paths, `text/event-stream` with no item schema, `type: string, format: binary` meaning raw bytes. By default these are refused (ADR-0010). A caller can opt in to a `CompatibilityProfile` such as `oas30-nullable-in-3.1-v1`, `colon-path-parameters-v1` or `legacy-binary-string-v1`, via `--compatibility-profile` or the session's `compatibility_profiles`. Each profile defines exactly one reinterpretation and matches a dialect pattern, never a vendor or a document. It emits a diagnostic wherever it applies and still refuses anything it would have to guess. Profiles are a closed enum, are part of revision and cache keys, and never rewrite the input document.

## Considered Options

- Lenient parsing by default, as Speakeasy does: rejected, because it silently changes wire meaning, for example a non-null codec for a field the API sends as `null`.
- Special cases for OpenRouter or other vendors: rejected, because they do not generalize and hide the departure from the spec author.

## Consequences

A new quirk needs a new profile, or a new version of an existing one, never an edit to a released profile.
