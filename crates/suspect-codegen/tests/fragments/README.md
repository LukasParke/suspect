# Admission fragments

One focused, self-describing OpenAPI document per tricky construct, in the
style of Speakeasy's `tests/specs/fragments/` (ideas only — our own
documents). Each fragment:

1. carries a header comment stating the invariant it pins,
2. satisfies the admission profile (one static absolute HTTPS server, one
   bearer scheme, `application/json` responses with schemas) unless the
   invariant is about profile admission itself, and
3. is executed by `tests/fragments.rs`, which runs the public admission
   review and asserts the expected verdict.

The `# INVARIANT:` marker is enforced by the test suite, so every
fragment declares what it pins in the same words the tests look for.

Add a fragment when a new edge case needs coverage; name it in kebab-case
and state the invariant in the header comment.
