# M0–M2 native interface approval

Approved by the user in this conversation on 2026-09-09: **Adopt baseline**.

The approved baseline uses the already demonstrated source-selected interfaces:

- TypeScript/JavaScript: explicit client configuration and one operation-input
  object, e.g. `client.createKeys({ body: { name: "CI" } })`.
- Rust: explicit credentials/transport and typed operation inputs/builders,
  e.g. `client.create_keys(CreateKeys::new(body)).await`.
- Both: explicit omission/null states, exact numbers, typed results/errors,
  source-defined bearer credentials, reusable clients and transport injection.

M2's representative contract is the original create/update/list/get vertical
slice: the five tracked credits/key/container-file operations, plus an independent
shared contract exercising unions, presence, recursion and negative native type
cases. Both languages must generate, install, execute and document that same
contract. The approval covers the interface baseline; it does not substitute for
the native gates or certify all future protocol/language coverage.

The broader upload/streaming/resource-interface examples in `SDK-NATIVE-DX.md`
remain design targets with their stated conditional support gates. Current scope
and verification requirements are in [SDK-GENERATION-PLAN.md](SDK-GENERATION-PLAN.md).
