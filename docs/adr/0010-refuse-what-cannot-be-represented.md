# Unsupported features are refused with a located diagnostic, never approximated

For each selected operation, every schema and HTTP feature must be represented exactly in native types, enforced by the generated codecs, or refused before any artifact is written. The diagnostic points at the source keyword and carries fix guidance. Examples are XML structures, streamed multipart, `mutualTLS`, untyped multipart extras, non-regular regex and custom dialects. Generation never widens to `any` or raw bytes, guesses an encoding, or rewrites a construct into something nearby. Capabilities belong to each language adapter (`http_protocol::Capabilities`), so modeling a feature in the shared protocol layer does not extend any backend's support until that backend has native tests. We accept refusing valid OpenAPI so that "it generated" always means "it is supported".

## Consequences

Features in operations that are not selected never block generation. Central operations of a real API can fail generation outright rather than produce a loose client.
