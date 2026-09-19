# Unmodified official dynamic-reference fixtures

Copied from JSON Schema Test Suite revision
`f6fd52a0a95472e079cbfc6ef7f089702b80e045` on 2026-09-10. Original MIT license
included. The files and their expectations are unmodified.

Base URL: `https://raw.githubusercontent.com/json-schema-org/JSON-Schema-Test-Suite/f6fd52a0a95472e079cbfc6ef7f089702b80e045/`

| Upstream path | SHA-256 |
| --- | --- |
| tests/draft2020-12/dynamicRef.json | dabad36a92ad5747f6b4f77907addf64795422589bbebdea7af8ff1a2b61879d |
| remotes/draft2020-12/tree.json | e2fa53954b78121c6533e98ef7ca83689c9b990c5ca1c01f65e2cc7e7f426664 |
| remotes/draft2020-12/extendible-dynamic-ref.json | 876cfd13df8730880db32f8d1b4b95c850ec170929247787118b448347dd1ed4 |
| remotes/draft2020-12/detached-dynamicref.json | 179fa0d52df07a3c0c0509ec9a66aae3f5bbd8d150b64a8bdfa9e7342ccb6385 |

All 44 cases run through public Contract → compile_v3 → program.check →
OwnedSchema.validate. The remote fixtures are explicitly supplied through a
closed DocumentProvider under their intended retrieval URIs; no test starts a
server, fetches at evaluation time, or patches reference strings. Each tested
schema remains a standalone schema document with its original `$id` and scopes.
The separate four formerly deferred dynamic/unevaluated cases reuse the existing
official files without editing them or the frozen v2 expectations.
