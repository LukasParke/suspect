# Schema URIs use strict RFC 3986 with our own resolver

`$id`, `$ref`, `$dynamicRef` and resource keys are parsed with `iri-string`'s strict RFC 3986 types. They are resolved by our own RFC 3986 §5.2 implementation in `suspect_ref::resource_uri`, which the Contract and the schema compiler share. Malformed references are rejected, not repaired. Resource keys fold only ASCII scheme and host case. Percent-encoded dots and slashes, path case, query and userinfo are preserved, and default ports and IDNA are not normalized.

## Considered Options

- The WHATWG URL parser (`url` crate): it silently repairs invalid input and applies scheme-specific rules.
- `iri-string`'s own resolver: it treats `%2E` segments as dots.

Either would merge or split schema identities in ways the specs do not allow.
