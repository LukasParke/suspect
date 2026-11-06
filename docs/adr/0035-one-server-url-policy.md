# Every SDK validates server URLs with one policy; encoded octets are opaque

All twelve generated SDKs check server URLs with the same rules:

- a source-declared server keeps its scheme: a plain `http` server in the description is the API owner's explicit choice (internal service networks rely on it) and is honored;
- a caller-supplied override must be `https`, or plain `http` to a loopback host (`localhost`, `127.0.0.1`, `[::1]`), unless the caller explicitly opts in to insecure HTTP on that client (Swift `allowHTTP`, and the idiomatic equivalent in every other SDK), so credentials never travel in cleartext by accident;
- no userinfo, query or fragment, and a valid port;
- a literal `.` or `..` path segment is refused rather than resolved, since resolving it would silently rewrite the request path;
- percent-encoded octets (`%2E`, `%2F`, …) are opaque path data: never decoded, normalized or rejected for what they decode to, and sent byte for byte. RFC 3986 dot-segment removal applies only to literal segments, and a document's physical base must survive unchanged (ADR 0015);
- malformed escapes, non-ASCII bytes, controls and backslashes are refused.

## Considered Options

- Also refuse percent-encoded dot segments and slashes: rejected. It breaks legitimate document bases (the physical-document-server suites rely on `…/%2E/%2f/`), and a conforming hop does not resolve encoded dots.
- Leave each backend's own rule: rejected. The backends had drifted (one accepted any `http` host, one refused encoded dots, one resolved literal dots via `URI.normalize`), so the same configuration behaved differently per language.

## Consequences

Two URL stacks cannot carry an encoded dot segment verbatim: Rust (`url`/reqwest) and the TypeScript v1 runtime (WHATWG URL with native Fetch) would rewrite `%2E%2E` into a real `..`, so those SDKs refuse encoded dot segments rather than send a different path. Dart's override parameter is a `Uri`, which `Uri.parse` has already normalized before the SDK sees it; source-declared Dart servers conform fully.
