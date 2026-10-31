# SDK quality and developer experience

Assessment requested **2026-09-10**, while the original-plan expansion is in
progress. This assesses the five verified packages in `target/sdk-demo/`, generated
by the sealed M3 binary. Java, C#, Kotlin, Ruby, PHP, Dart and C++ are being
implemented and have not inherited that acceptance result.

## Verdict

**Strong correctness foundations; uneven customer-facing DX.** The verified
slice is usable, but it is not yet a uniformly polished public SDK product.
Native compilation, installed consumers, precise codecs, wire tests and built
documentation establish important technical properties. They do not establish
that a new developer can discover the right type or complete a task comfortably.

The 208 passing demo-relevant criteria apply to a bounded JSON/bearer profile,
the independent M2 contract, and five real OpenRouter operations. They are not
208 usability studies or verification of the complete OpenRouter API.

## Language experience at the verified checkpoint

| Target | What feels native | Current friction |
| --- | --- | --- |
| TypeScript / JavaScript | Plain-object inputs, unions, Promise/AbortSignal, ESM exports, dependency-free runtime | Flat operation names, required empty input objects, verbose inline model names, exact-number learning curve |
| Python | Keyword-only dataclasses/calls, sync/async, context managers, httpx transport seam, UNSET distinct from None | Long request-model names; exact numbers use a custom type; concrete operation errors reside in `_client.py`; thin quickstart |
| Go | context.Context, typed inputs/errors, net/http, explicit optional fields | Even a single-success operation returns a result interface requiring narrowing; constructor/response wrappers and inline names are verbose |
| Rust | Result/enums, builders, optional transports, Cargo features and Rustdoc | Long model names; single-status result enum; transport features and executor setup need a better introductory path |
| Swift | Value types/enums, async/await, Sendable, URLSession and SwiftPM | Empty input structs and status enums add ceremony; generated quickstarts expose codec machinery; exact Codable uses the SDK's encoder/decoder |
| Java / C# / Kotlin / Ruby / PHP / Dart / C++ | Approved native interface designs; implementation underway | No overall quality grade or support claim until their native packages, consumers and runtime checks pass |

## Authentication and request behavior

For the demonstrated OpenRouter operations the scheme is named `apiKey`, but
its declaration is **HTTP bearer**, so the wire header is
`Authorization: Bearer <token>`. Credentials are explicit constructor options;
the application supplies any environment-variable lookup. The source's
management-key requirement is preserved in documentation, not inferred as a
different credential type that the SDK could validate locally.

The verified profile has one bearer scheme. API-key headers/query/cookies,
basic authentication, alternative/conjunctive requirements and OAuth credential
hooks belong to the ongoing expansion. Token acquisition/refresh and login UX
are separate from applying a credential to a declared request.

The clients have injectable transports, cancellation and bounded response/error
captures. The SDK layer adds no automatic retries or pagination loops. Native
transport policy is explicit; this provides control, but application developers
still need recipes for common operational behavior.

## Actual TypeScript / JavaScript calls

`examples/sdk-demo.json` chooses the package identity used here. These are the
actual generated names; the example body is an ordinary object literal.

```ts
import { createClient, operations } from '@demo/openrouter-sdk';

const client = createClient({ auth: { apiKey: token } });

try {
  const response = await client.getCredits(
    {},
    { signal: AbortSignal.timeout(30_000) },
  );
  console.log(response.data.data.total_credits.toString());
} catch (error: unknown) {
  if (operations.isGetCreditsApiError(error)) {
    console.error(error.response.status, error.response.data.error.message);
  } else {
    throw error;
  }
}

await client.createKeys({ body: { name: 'Demo key' } });
```

The outer `response.data` is the SDK's response wrapper; the inner `data` is an
actual wire field. Exact decimals retain a `JsonNumber` instead of silently
rounding through JavaScript `number`. Both choices are explainable, but the
double `data` and mandatory `{}` are real ergonomic costs.

Presence remains faithful:

```ts
await client.updateKeys({ hash, body: {} });               // limit absent
await client.updateKeys({ hash, body: { limit: null } });  // explicit null
```

These represent different requests. No extra server-side meaning is inferred.

## Actual Python calls

```python
from openrouter_demo_sdk import Client, AsyncClient, models

with Client(auth={"apiKey": token}) as client:
    response = client.get_credits()
    print(response.data.data.total_credits.token)

    body = models.PathsKeysPostRequestBodyContentApplicationJsonSchema(
        name="Demo key",
    )
    client.create_keys(body=body)

# Inside an async function:
async with AsyncClient(auth={"apiKey": token}) as client:
    response = await client.get_credits()
```

The client lifetime and keyword calls are idiomatic. The request-model name is
not: `CreateKeysRequest` would be a much better public name, with the full source
pointer retained separately in provenance. That shorter name is a proposed
improvement, not an export in the assessed package.

## Other native call shapes

Go, with an existing context and supplied token:

```go
client, err := sdk.NewClient(sdk.ApiKey(token), sdk.ClientOptions{})
if err != nil { return err }
result, err := client.GetCredits(ctx, sdk.NewGetCreditsInput())
if err != nil { return err }
switch response := result.(type) {
case sdk.GetCreditsStatus200:
    fmt.Println(response.Data)
}
```

Rust, inside a caller-owned async runtime with `reqwest-rustls` enabled:

```rust
let client = Client::with_reqwest(Credentials::api_key(token))?;
let response = client.get_credits(GetCredits::new()).await?;
let GetCreditsSuccess::Status200(response) = response;
```

Swift, inside an async throwing context:

```swift
let client = Client(credentials: Credentials(apiKey: token))
let result = try await client.getCredits(GetCreditsInput())
switch result {
case .status200(let response):
    print(response.data)
}
```

The full import/module identities are in the generated package READMEs and
native references. These native interfaces preserve declared status variants,
but a sole success status should have a less ceremonious convenience path.

## Documentation quality

Native documentation exists and has been built: TypeDoc, Sphinx, Go doc,
Rustdoc/doctests and Swift DocC. Source descriptions, exact wire names, symbol
bindings, validation obligations and example provenance are present. Invalid
declared examples are reported; synthesized replacements are identified.

The remaining problem is **editorial and usability quality**:

- Python's generated README is only seven lines; it is not a complete onboarding guide.
- Python's operation reference prominently emits machine-readable binding JSON.
- The Swift GettingStarted page constructs simple string inputs with
  `Codecs....decode(Data("\"sess_abc123\"".utf8))`, obscuring the native call.
- TypeScript and Rust quickstarts often wrap an abstract `input` argument rather
  than showing a complete, task-oriented request.
- Source URI/pointer text is useful for auditing but is not a polished,
  publishable source-link experience.
- Authentication, errors, timeouts, omission/null, exact numbers and transport
  injection need concise end-to-end recipes in every language.

## Comparison with Speakeasy

Primary-source comparison uses the official OpenRouter TypeScript repository at
`9078199a74dc5d35714ad641a0dce2756164166f` (manifest version 1.2.116) and Python at
`c6a371052886819c6f6905a304168f5bfae745b3` (manifest version 1.1.136), retrieved
2026-09-10. These are source-inspected outputs, not a controlled generation or
runtime conformance comparison using identical inputs. Their public interfaces
also reflect OpenRouter configuration, overlays and some custom code.

**Speakeasy currently has the stronger overall consumer-DX offering.** Our
strongest potential differentiation is the tested exactness and source-level
traceability of the admitted slice, rather than breadth or onboarding polish.

| Area | Current comparison |
| --- | --- |
| Everyday calls | Speakeasy's resource namespaces, readable request/response names and Python flattened keyword inputs are easier to discover |
| Native idioms | Our five verified targets use genuine ecosystem interfaces; some unnecessary wrappers and helper exposure remain |
| Authentication | Our verified bearer path is explicit and tested; Speakeasy documents broader auth support and the inspected clients also support credential callbacks |
| Operational behavior | Speakeasy supplies configurable retries, timeouts, streaming, uploads and pagination features; our verified profile is narrower and adds no retries |
| Numeric precision | Our credits fields retain exact tokens; the inspected Speakeasy outputs use number/float for source number/double fields. Speakeasy also supports configured decimal/bigint types |
| Presence / runtime validation | Both support absence/null and runtime validation. Our semantic/resource guarantees require matched tests before claiming general superiority |
| Documentation | Speakeasy has broader resource/model references and recipes; both sets of outputs have concrete onboarding problems |
| Provenance | Both retain generation provenance. Our finer-grained source bindings and native acceptance evidence are a potential differentiator |
| Maturity | Our result is a locally verified bounded slice; Speakeasy's documented GA language support and distribution workflows represent broader product maturity |

The same credits and create-key operations in the inspected Speakeasy TypeScript SDK:

```ts
import { OpenRouter } from '@openrouter/sdk';

const sdk = new OpenRouter({ apiKey: token });
const credits = await sdk.credits.getCredits();
console.log(credits.data.totalCredits);

await sdk.apiKeys.create({
  requestBody: { name: 'Demo key', limit: 10 },
});
```

In Python, Speakeasy hides the request-model constructor for this operation:

```python
from openrouter import OpenRouter

with OpenRouter(api_key=token) as sdk:
    credits = sdk.credits.get_credits()
    sdk.api_keys.create(name="Demo key", limit=10)
```

Sources: pinned [TypeScript credits](https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/src/sdk/credits.ts),
[TypeScript keys](https://github.com/OpenRouterTeam/typescript-sdk/blob/9078199a74dc5d35714ad641a0dce2756164166f/src/sdk/apikeys.ts),
[Python credits](https://github.com/OpenRouterTeam/python-sdk/blob/c6a371052886819c6f6905a304168f5bfae745b3/src/openrouter/credits.py) and
[Python keys](https://github.com/OpenRouterTeam/python-sdk/blob/c6a371052886819c6f6905a304168f5bfae745b3/src/openrouter/api_keys.py).

The operational tradeoffs matter. OpenRouter's inspected Speakeasy outputs inherit
source-configured retries for 5XX and connection/timeout failures, including POST
and PATCH, with a one-hour elapsed retry configuration. This is not a strict
whole-call deadline; 429 is not in these operations' default retry list. Our
verified clients add no retry loop. Neither choice by itself establishes greater
security or correctness; callers need clear documented control.

Speakeasy's inspected docs are also imperfect: the TypeScript credits parameter
table marks an optional argument as required; a Python README stream example
iterates an undefined variable; and the TypeScript chat quickstart differs from
its pinned source request shape. This reinforces the need to compile the exact
published quickstarts, alongside checking native reference coverage.

Speakeasy currently documents seven GA SDK language targets: TypeScript, Python,
Go, Java, C#, PHP and Ruby. Its docs label Rust and C++ “Coming Soon”; this research
did not establish Swift's current maturity. That cannot be reduced to a claim
that our five verified targets plus seven unfinished ones are a better product.
See the [cited research](SDK-SPEAKEASY-RESEARCH.md) for the language matrix,
configuration details, precision mapping and documentation findings.

## DX acceptance work

1. Allocate readable, collision-safe names from component names and operation
   roles. Keep source identity in metadata instead of spelling it out in a class name.
2. Make the ordinary path short: convenient no-input calls, direct native model
   construction, and an ergonomic single-success result.
3. Expose operation errors and numeric/presence helpers through intentional public
   modules, with clear conversion and interoperability recipes.
4. Generate a complete first-request quickstart plus error, async/cancellation and
   update/presence examples; verify the exact snippets through installed consumers.
5. Keep native reference docs and detailed provenance, with task-oriented guides
   as the entry point.
6. Verify protocol breadth and language maturity separately from documentation
   polish and conformance. A new backend's passing build alone satisfies neither.

## Local evidence

- Frozen native acceptance: `target/sdk-greenfield-native-verified-03/report.json`.
- Assessed packages: `target/sdk-demo/{typescript,python,go,rust,swift}/`.
- Actual TypeScript/Python usage snippets passed strict TypeScript and mypy:
  `target/sdk-dx-assessment-20260910/`.
- Additional injected-transport checks exercised the shown TypeScript/Python
  bearer headers and exact credits response. They did not contact OpenRouter.
- TypeScript example symbols: `typescript/operations.ts` and `source/index.ts`.
- Python example symbols: `python/src/openrouter_demo_sdk/{_client,models}.py`.
- Swift docs observation: `swift/Sources/OpenRouterDemoSDK/OpenRouterDemoSDK.docc/GettingStarted.md`.

The separate [Speakeasy research](SDK-SPEAKEASY-RESEARCH.md) records primary-source
comparisons, including which behavior is generator configuration or customization.
