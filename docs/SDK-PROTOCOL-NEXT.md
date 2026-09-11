# Extending native HTTP protocol support

The source-backed [HTTP protocol contract](SDK-HTTP-PROTOCOL.md) defines shared
descriptors for methods, servers, security, parameters, status/media dispatch,
forms, multipart and item streams. The [capability matrix](SDK-CAPABILITIES.md)
links to each native adapter's admission boundaries.

## Extension requirements

1. Establish semantics from the applicable OpenAPI version. Preserve source
   declarations, use sites, references and physical document identity.
2. Express the feature in typed shared descriptors before native emission.
3. Admit only the representations and runtime behavior that the adapter can
   implement. Unsupported or ambiguous selected declarations produce located
   diagnostics before artifacts.
4. Carry the feature through codecs, operation signatures, examples, docs,
   session configuration and native/wire compatibility descriptors.
5. Exercise independent native type and wire fixtures, including failure,
   cancellation and bounded-resource cases.

## Interpretation boundaries

An operation name cannot imply pagination, retry, security or body behavior.
Source extensions remain metadata until an explicit versioned interpretation
profile gives them semantics. For example,
[`legacy-binary-string-v1`](SDK-INTERPRETATION-PROFILES.md) opts into legacy
binary markers without changing ordinary JSON string meaning.

Different representations of a successful response remain explicit alternatives.
No-content responses do not gain synthetic JSON values. Streamed items follow
declared framing and item schemas; a complete-content schema is not silently
converted into a streaming contract. Form and multipart serialization follows
the declared media/encoding rules rather than guessed query conventions.

Shared descriptor support is distinct from native executable support. Adding a
descriptor must not automatically promote every adapter's capability claim.
