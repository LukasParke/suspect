# Standalone schema URI references

The borrowed JSON Schema compiler validates resource identifiers and reference
syntax with `iri-string` 0.7.14's RFC 3986 `UriAbsoluteStr` and
`UriReferenceStr` types. This dependency belongs to the compiler, not generated
SDK packages. It has no required transitive dependencies with the selected
`alloc` feature.

Malformed `$id`, `$ref`, and `$dynamicRef` values fail compilation when they
are compiled. Whitespace, backslashes, incomplete percent escapes, malformed
authorities, and unescaped non-ASCII characters are rejected instead of being
repaired by a browser URL parser. References must be URIs; an internationalized
identifier needs its URI encoding. The entire reference is parsed before
checking whether its target resource is locally available, including external
reference fragments.

## Resolution and identity

`crates/suspect-schema/src/resources/uri.rs` implements RFC 3986 section 5.2
component resolution using those validated components. Dot removal applies to
literal `.` and `..` path segments. Queries, encoded path delimiters, encoded
dots, and opaque URI paths are preserved. A query-only reference can resolve
against an opaque base. An explicit scheme remains absolute, including the
RFC's strict interpretation of `http:g`.

The dependency's resolver is not used: it treats percent-encoded dot segments
as literal dots even without optional normalization. The local resolver keeps
that policy out of schema resource identity and rejects exceptional
authorityless results that cannot be represented without changing their
components. It does not insert WHATWG serialization repairs.

Resource keys fold ASCII scheme and host case, as described by RFC 3986
section 6.2.2.1. They preserve user information, path case, query case, and
fragment spelling. Fragment splitting precedes percent decoding for JSON
Pointer or anchor lookup; encoded `#` and `/` in a document path do not become
URI delimiters. Retrieval aliases and `$id` values use the same key rules.

This is not complete URI equivalence handling. It does not unify default
ports, absent versus empty paths, alternate percent-escape spellings,
percent-encoded unreserved characters, alternate IPv6 text, IDNA, or
scheme-specific equivalents. Schema authors should use consistent normalized
identifiers. These additional equivalence policies remain separate from strict
syntax and generic reference resolution.

External fetching is still unsupported by this standalone compiler. A valid
reference to an unavailable resource produces an Evaluation failure, which
cannot become a passing validation verdict through logical inversion.

## Evidence

The helper tests cover all **42** normal and abnormal examples in RFC 3986
section 5.4, using the strict-parser interpretation and comparing document
identifiers separately from fragments. Additional cases cover opaque bases,
encoded delimiters, encoded dots, and fragment splitting.

`crates/suspect-schema/tests/resource_uris.rs` exercises the public compiler
and validator: **16** malformed spellings across all three reference keywords,
**9** valid resource-resolution fixtures, scheme/host case equivalence with
case-sensitive controls, retrieval aliases, and unrepresentable path rejection.
The malformed-space, opaque query reference, and scheme/host identity tests
were observed failing before their repairs.

Primary references:

- [RFC 3986 syntax and reference resolution](https://www.rfc-editor.org/rfc/rfc3986)
- [JSON Schema Core 2020-12, identifiers and references](https://json-schema.org/draft/2020-12/json-schema-core)
- [iri-string 0.7.14 types](https://docs.rs/iri-string/0.7.14/iri_string/types/index.html)
- [iri-string resolver's encoded-dot behavior](https://docs.rs/iri-string/0.7.14/iri_string/resolve/struct.FixedBaseResolver.html#method.resolve)
