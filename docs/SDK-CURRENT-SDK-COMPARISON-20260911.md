# Our generated SDKs versus OpenRouter’s current TypeScript, Go and Python SDKs

Research date: **11 September 2026 (UTC)**. Registry/default-branch acquisition began at **02:32 UTC**. This is a like-for-like **TS → TS, Go → Go, Python → Python** comparison, with qualitative expectations for the other nine languages in §11.

## Verdict

**Our packages are credible, stricter, source-linked SDK building blocks, but the inspected candidates are not replacements for OpenRouter’s current client SDKs yet. The decisive gap is usable OpenRouter chat coverage, followed by automatic credential and operational conveniences.** All three current official distributions successfully handled controlled chat JSON, tool/structured-output request serialization, vendor SSE, file upload and pagination probes. Our pinned CLIs reject the selected real chat and upload operations under the inspected configuration. That is considerably more important to an OpenRouter consumer than winning an exact-decimal example. [T-probe] [P-probe] [G-probe] [T-features] [P-features] [G-features] [admission-current]

| Matched comparison | Upfront assessment |
| --- | --- |
| **TypeScript** | Ours has short root operation calls, exact numbers, source-linked failures and readily available response metadata. The official SDK has resource namespaces, automatic environment fallback, plain-number inputs, operational options and working chat/file/pagination APIs. Its current README chat example does **not** typecheck against its published API. Both already support absence/null and runtime validation. **Useful narrow alternative; incomplete replacement.** [T-probe] [T-negative] [T-features] [O-ts] |
| **Go** | Ours preserves familiar `context.Context`/`net/http`, adds checked construction/encoding and a useful `…Data` convenience, retains response metadata, and declares a lower Go floor. The official SDK has substantially broader API coverage and convenient grouped methods, but the probe exposed an actual stream-timeout lifetime problem and missing-required-response-field acceptance. Its constructor attribution-header options did not attach headers to the inspected calls. **Our bounded Go interface compares well; API breadth still prevents replacement.** [G-probe] [G-features] [G-package] [O-go] |
| **Python** | The official client is easier for routine key management: `api_keys.create(name=…, limit=…)`, one client with sync/async methods, host floats and environment fallback. Ours uses explicit dataclasses, separate sync/async clients, exact-number wrappers and an extra response layer, in exchange for stricter schema behavior and straightforward metadata. The official package also has a narrowly reproduced unknown-enum serialization defect. **Strong semantic tradeoff; less convenient ordinary call sites and insufficient chat coverage.** [P-probe] [P-features] [P-keys] [O-py] |

**The latest six-operation, clean-name candidate now has real installed/native `/key` evidence:** twelve native builds and **39 controlled checks** (TS plus independent JS, and the other eleven targets; key 200, credits 200 and typed 401). This closes the earlier import/normal-key-read proof gap. Environment lookup is still in the application wrapper, and no real-token service call is claimed. The latest staging proof, the original five-operation package, and the intermediate two-operation generation are distinguished below. [L-evidence] [L-readme] [L-verification] [O-native] [M-report]

## 1. Exactly what was compared

### Current official distributions, independently acquired and installed

The baselines are the first-party client SDK repositories linked by OpenRouter’s SDK documentation: `OpenRouterTeam/typescript-sdk`, `OpenRouterTeam/go-sdk`, and `OpenRouterTeam/python-sdk`. This report does not substitute the OpenRouter Agent SDK, an OpenAI-compatible third-party client, or a generic Speakeasy demo. [OR-ts] [OR-go] [OR-py] [pins]

| Language | Published package selected | Publication / version timestamp, UTC | Current repository default head at acquisition |
| --- | --- | --- | --- |
| TS | npm **`@openrouter/sdk@1.2.117`** | npm publication **2026-09-10 22:26:17.176** | `main` → **`9724d06630911551912c0e6d5c892a632943e190`**, commit **22:23:09**, “Update SDK … 1.2.117” |
| Go | Go module **`github.com/OpenRouterTeam/go-sdk@v0.7.130`** | module version time **2026-09-10 22:24:21**; release commit **`200f1f03bc772dcaacc8bafbdff2ab65274a0753`** | `main` → **`97e4f4fc0f1974da55c920930f428d270e864a04`**, commit **22:26:31**, “bump examples to SDK v0.7.130” |
| Python | PyPI **`openrouter==1.1.137`** | wheel upload **2026-09-10 22:25:07.790431** | `main` → **`2b1df428c5345be00215a8e3a3153d6e62bcf299`**, commit **22:22:46**, “Update SDK … 1.1.137” |

Sources: fresh registry responses, GitHub repository/commit metadata, acquisition receipts and archives in [pins]. The Go timestamp is a module-version timestamp, not a claimed registry-upload timestamp. GitHub actor/author/committer data and complete commit messages are retained in `sources/*/head.json`. [G-release]

**Distribution hashes actually checked:**

| Distribution | SHA-256 |
| --- | --- |
| npm `sdk-1.2.117.tgz` | `c3862f72b883d92bf41a0665b948d5f5d92378796d749d021135abf8837b0ab4` |
| Python `openrouter-1.1.137-py3-none-any.whl` | `e860618d6636b39802b3cde27abf0270eb4e8f26f30ad7e2348a27bd46c04a6d` |
| Python `openrouter-1.1.137.tar.gz` | `2f9ae393b22bb4091a2d837f6a0023aeeb4b218523e56acb812fcdbc49bed6c6` |
| Go `v0.7.130.zip` | `eed8ecbd440bdb9f9a5a192b9d34d046a09e9a351ad910c339a87ca1aa381af5` |

The npm registry’s additional integrity value is `sha512-ThL0JVomOfwSjSqHbhAUAccUCW+mOjDW7CX7h4V5m6shjlhy4fTbhRBR0oo4SCr9kgKr/9NVh03sQUXJpO1yow==`. Go’s verified module sum is `h1:057rrxal2wIZhYoYAjHWNMjb8aoz3rX41aJljBz/Osc=` and its `go.mod` sum is `h1:8vFtUZ9I2YiDntYbck42wQhrx9DsJN5PHbViaHwTkXc=`. Downloads, registry metadata and package-manager receipts are retained; the wheel and npm tarball, rather than just repository source, were installed. [pins] [G-download] [G-sumdb] [install]

**Repository versus distribution:** npm’s `gitHead` equals the captured TS head. All **832** Python wheel package files compared byte-for-byte equal the corresponding `src/openrouter` files at the captured Python head. Go’s head is one commit ahead of the released module; GitHub’s comparison identifies changes only to eight example modules’ `go.mod`/`go.sum` files. All **2,610 common files** between the downloaded module and head archive match. These are identity checks, not quality scores. [parity] [G-head-diff]

The older [10 September research](SDK-SPEAKEASY-RESEARCH.md) remains historical: its TS `1.2.116` / Python `1.1.136` repository pins are **not** this report’s current-release evidence.

### Our three different evidence boundaries

| Label used here | Immutable available evidence | What it establishes |
| --- | --- | --- |
| **O — installed five-operation baseline** | `target/sdk-demo-readme-20260911-01/candidate-02/packages`; CLI SHA-256 **`8e567dc987b7cff03d37b328a1a1b746ae764bf20aa54bc14465d127f211b499`** | Twelve real native packages, plus an independent JS consumer of the TS package; existing installation/type/wire/401 receipts. Selected operations: `getCredits`, `createKeys`, `updateKeys`, `listContainerFiles`, `getContainerFile`. The new matching TS/Python probes install its retained tarball/wheel; Go imports its sealed module. [O-pins] [O-native] [install] |
| **M — intermediate clean-name read generation** | `target/sdk-main-live-admission-20260911-01/packages`; CLI SHA-256 **`5d3cecd1bc0d5bd6bdda4b086ff7038977a7bb4939cefca213235e2d225e84cb`** | All twelve targets generated for **`getCurrentKey` + `getCredits`**, one contract compile/twelve renders, 434 SDK artifacts plus ownership manifest. **Generation only**, superseded for current demo DX by L. Retained as separate admission evidence. [M-report] [M-session] |
| **L — CURRENT six-operation live staging** | **`target/sdk-demo-live-20260911-01/candidate-02/packages`**, the same pinned **`5d3cecd1…`** CLI | **`getCurrentKey` plus O’s five operations**; 592 SDK artifacts plus ownership manifest. Twelve prepared native programs plus JS; all 39 controlled key/credits/401 checks pass. The live runner reads env/prompt and passes explicit credentials. **No automatic native env policy or actual live-token call is claimed.** `delivery-01` now records `staged-ready-not-live-executed`; the separately observed runner seal mismatch is recorded in §12. [L-session] [L-evidence] [L-verification] [L-readme] [L-delivery] |

O’s package manifest hash is `0335ddf1a96b9352e21d0c2edcc0ca4ee19a8ee8583b35c4de1b5f783eb0c486`; M’s is **`78f015dc32982d006e186dadbe5e7142a4cb9bae7420d1079e5e4a7cd635e86a`**. M’s session hash is **`f0eab554dc5f94dcc0542262e7023397daf75c48917931c0b2d58902d5f45842`**. L’s package-pins hash is **`52532ff70dd8750e0d4bf9c969f855a3b95db42137568214e789c5fc537e43f8`** and its session hash is **`c29551b85ba8d0da62b042b511e47b9eea101aa49ce34cc08dabf21195db2efd`**. All 566 O, 435 M and 593 L files matched their recorded content hashes; L’s ready/snippet and controlled-result inventory is captured separately. [local-verification] [L-evidence] [M-report]

All three local candidates use the same public source bytes, SHA-256 **`bd4953b29f34de134ed4be27b2803c5622a756c3c63bc8617636b6abe5de1821`**, from upstream revision **`db378a2a90d0167b9dca4f98b52074c54d249e1f`**. Their operation selection, package identities and CLI differ. O and L preserve the four-input source inventory; the packages here are generated from the public specification, not a merger with the separate management/monitor/benchmark contracts. [O-pins] [L-sources] [M-report]

The current official repositories’ common **input** specification is different: SHA-256 **`0caf69e2c3f76a4bf2260cee14cdd3759ac45074b1b3542897c7004024475b6d`**. It contains 105 HTTP operation declarations versus 103 in our source snapshot. These counts describe source inventories, not proven complete SDK/API support. Crucially, the matched credits numeric declarations and `createKeys.name.minLength: 1` agree in the inspected inputs and official overlay outputs. [source-analysis]

The HTTP/auth/JSON/codec runtime files enumerated in `current-live-evidence.json#runtime_parity_to_O` are byte-identical between O and L for all three matched languages. That makes the older controlled probes useful implementation context, but does not relabel them as executions of L’s expanded generated schema closure. L has its own 39 read/error checks and the new clean-import call-site typechecks. [L-evidence] [L-callsites]

### Controlled execution, not live account activity

All new HTTP probes use injected Fetch, HTTPX mock transports or a Go `Do` implementation. Even when a recorded URL is `https://openrouter.ai/api/v1/key`, the transport returns local fixture bytes. Credentials are dummy strings; `OPENROUTER_*` variables are removed from inherited probe environments before controlled values are set. There were no live OpenRouter requests or account mutations. Existing native matrices were reused; only the additional matched questions below were exercised. [probe-sources] [commands]

## 2. Installation, imports and the normal-key first read

### Package identity is configuration, not a generator naming limitation

| Language | Official installed release | O: earlier installed local identity | L: current installed clean identity |
| --- | --- | --- | --- |
| TS/JS | `@openrouter/sdk`; `import { OpenRouter } from "@openrouter/sdk"` | `@demo/openrouter-all-sdk`; `createClient` | **`@openrouter/sdk`**; `createClient` |
| Go | `github.com/OpenRouterTeam/go-sdk`; package `openrouter` | `example.com/openrouter-all-sdk`; package `sdk` | **`github.com/openrouter/sdk-go`**; package `sdk` |
| Python | distribution/import `openrouter`; `OpenRouter` | distribution `openrouter-all-sdk`, import `openrouter_all_sdk`; `Client`, `AsyncClient` | distribution/import **`openrouter`**; `Client`, `AsyncClient` |

Sources: package manifests, session configuration and actual public exports. The branded TS/Python names match the official package names but **the APIs do not become source-compatible**. Registry installation still selects the published official SDK; our generated candidates are local version `0.1.0` artifacts. [T-package] [P-package] [G-package] [O-session] [L-session] [L-ts-ops] [L-py-client] [L-native]

Normal current-release installation is `npm install @openrouter/sdk@1.2.117`, `go get github.com/OpenRouterTeam/go-sdk@v0.7.130`, or `python -m pip install openrouter==1.1.137`. For the experiments, exact downloaded npm/wheel files were installed privately, and Go downloaded the exact module with checksum verification. O’s real local artifacts were installed alongside them. L now has its own separately installed tarball/wheel/local Go module and checked consumers, whose preparation receipts are reused. [install] [G-download] [L-native]

### First read: `getCurrentKey`, GET `/key`

This is the appropriate first authenticated read for a normal user token. `getCredits`, **both** `createKeys` and `updateKeys` require a **management key** according to their operation descriptions. Neither SDK can determine the account role merely from constructing its bearer credentials. [source-analysis] [T-keys] [P-keys] [G-keys]

Use the same already-obtained `token` in both columns. The official side below is exercised with controlled transports in this report. L’s real installed consumers now exercise `/key`, credits and typed 401 against controlled HTTP responses; the short read/management call sites were additionally typechecked against the current three packages. These are local preparation results, not actual live OpenRouter responses. [T-probe] [P-probe] [G-probe] [L-verification] [L-callsites]

| Task | Official current SDK | Our current L package |
| --- | --- | --- |
| TS construction | `new OpenRouter({ apiKey: token })` | `createClient({ auth: { apiKey: token } })` |
| TS call/value | `const r = await client.apiKeys.getCurrentKeyMetadata(); r.data.label` | `const r = await client.getCurrentKey(); r.data.data.label` |
| Go construction | `openrouter.New(openrouter.WithSecurity(token))` | `sdk.NewClient(sdk.ApiKey(token), sdk.ClientOptions{})` → `(*Client, error)` |
| Go call/value | `r, err := client.APIKeys.GetCurrentKeyMetadata(ctx); r.Data.Label` | `r, err := client.GetCurrentKeyData(ctx); r.Data.Label` |
| Python construction | `with OpenRouter(api_key=token) as client:` | `with Client(auth={"apiKey": token}) as client:` |
| Python call/value | `r = client.api_keys.get_current_key_metadata(); r.data.label` | `r = client.get_current_key(); r.data.data.label` |

Sources: actual exports, staged consumers and checked call sites. The extra `.data` in TS/Python is our HTTP wrapper around OpenRouter’s own `data` envelope. Go’s `…Data` removes the HTTP wrapper when metadata is unnecessary; L’s staged program uses canonical `GetCurrentKey(ctx)` and the value-type `GetCurrentKeyStatus200` to retain status. [T-keys] [P-keys] [G-keys] [L-ts-ops] [L-py-client] [L-go-ops] [L-callsites] [L-go-example]

**What already works:** the source supplies `https://openrouter.ai/api/v1`; callers do not need to repeat it. The matched O credits probes captured that default URL in all three languages. L retains the source server, clean imports and the normal-key operation. Its controlled verification uses an explicit test URL; the live-mode source-default branch is ready but has not contacted the service with a real token. [T-probe] [P-probe] [G-probe] [L-readme] [L-ts-example] [L-py-example] [L-go-example]

**What L does not provide automatically:** `createClient()` / `Client()` / a Go env factory obtaining `OPENROUTER_API_KEY`. Its `5d3…` CLI uses the existing explicit-credential APIs and its session does not enable the new policy. The staged application reads `process.env`, `os.environ`, or `os.Getenv` and passes the token to the real SDK constructor. That is working runtime credential setup, accurately labeled as wrapper/application code. [L-session] [L-ts-example] [L-py-example] [L-go-example] [L-readme] [env-contract]

## 3. Matched key/credits call sites and return shapes

### TypeScript: plain objects on both sides, different public field names

The following complete task functions typecheck against the official installed package and **current L clean imports** with TS **5.9.3**, `strict` and `exactOptionalPropertyTypes`. The detailed controlled create/update probes remain pinned to O; L’s separate current receipts prove `/key`, credits and typed 401. The functions take an explicit token in both versions. [T-callsites] [T-typecheck] [T-probe] [L-callsites] [L-verification]

```ts
// Official @openrouter/sdk@1.2.117
import { OpenRouter } from "@openrouter/sdk";

async function manage(token: string): Promise<string> {
  const client = new OpenRouter({ apiKey: token });
  const credits = await client.credits.getCredits();
  await client.apiKeys.create({
    requestBody: { name: "Comparison key", limit: 50.25 },
  });
  await client.apiKeys.update({ hash: "fixture-hash", requestBody: {} });
  await client.apiKeys.update({ hash: "fixture-hash", requestBody: { limit: null } });
  return credits.data.totalCredits.toString();
}
```

```ts
// Our installed L package, local version 0.1.0
import { createClient, JsonNumber } from "@openrouter/sdk";

async function manage(token: string): Promise<string> {
  const client = createClient({ auth: { apiKey: token } });
  const credits = await client.getCredits();
  await client.createKeys({
    body: { name: "Comparison key", limit: JsonNumber.parse("50.25") },
  });
  await client.updateKeys({ hash: "fixture-hash", body: {} });
  await client.updateKeys({ hash: "fixture-hash", body: { limit: null } });
  return credits.data.data.total_credits.toString();
}
```

The official create **does require `requestBody`** at this pin. Its readable models live under `@openrouter/sdk/models/operations`: `CreateKeysRequest`, `CreateKeysRequestBody`, `UpdateKeysRequestBody`. O exports `models.CreateKeysRequest` / `models.UpdateKeysRequest` as the body types and `operations.CreateKeysInput` / `UpdateKeysInput` as the operation inputs. The official `includeByokInLimit`/`limitReset` map to the same wire names as O’s `include_byok_in_limit`/`limit_reset`. These are presentation choices, not different service fields. [T-create-model] [T-update-model] [O-ts-ops] [source-analysis]

### Go: grouped functions versus source-operation methods

The official calls in the compiled consumer are: [G-probe-source] [G-build]

```go
client := openrouter.New(openrouter.WithSecurity(token))
credits, err := client.Credits.GetCredits(ctx)
if err != nil { return err }
fmt.Println(credits.Data.TotalCredits)

_, err = client.APIKeys.Create(ctx, operations.CreateKeysRequest{
    Name: "Comparison key",
    Limit: optionalnullable.From(openrouter.Float64(50.25)),
})
if err != nil { return err }
_, err = client.APIKeys.Update(ctx, "fixture-hash", operations.UpdateKeysRequestBody{})
if err != nil { return err }
_, err = client.APIKeys.Update(ctx, "fixture-hash", operations.UpdateKeysRequestBody{
    Limit: optionalnullable.From[float64](nil),
})
if err != nil { return err }
```

Imports are `openrouter "github.com/OpenRouterTeam/go-sdk"`, its `/models/operations` and `/optionalnullable` packages, plus the normal standard library. The corresponding current L code uses **`sdk "github.com/openrouter/sdk-go"`**. Its body/presence/read call-site forms pass a fresh bounded compile; O’s detailed controlled mutation probes remain a separate runtime receipt. [G-probe-source] [L-go-callsites] [L-callsites] [G-probe]

```go
client, err := sdk.NewClient(sdk.ApiKey(token), sdk.ClientOptions{})
if err != nil { return err }
defer client.CloseIdleConnections()
credits, err := client.GetCreditsData(ctx)
if err != nil { return err }
fmt.Println(credits.Data.TotalCredits.String())

amount, err := sdk.ParseNumber("50.25")
if err != nil { return err }
body := sdk.NewCreateKeysRequest("Comparison key")
body.Limit = sdk.PresenceSome(amount)
_, err = client.CreateKeys(ctx, sdk.NewCreateKeysInput(body))
if err != nil { return err }
patch := sdk.NewUpdateKeysRequest()
_, err = client.UpdateKeys(ctx, sdk.NewUpdateKeysInput("fixture-hash", patch))
if err != nil { return err }
patch.Limit = sdk.PresenceNull[sdk.Number]()
_, err = client.UpdateKeys(ctx, sdk.NewUpdateKeysInput("fixture-hash", patch))
if err != nil { return err }
```

Both use context first and ordinary returned errors. O’s constructor returns an error for invalid client policy; the official `New` returns a client directly and `WithServer` can panic on an unknown server name. The official update avoids an additional input struct through its configured parameter flattening. O’s required-field constructors are useful guidance, though exported Go structs can still be constructed directly; encode/call validation supplies the runtime check. [G-root] [G-config] [O-go-runtime] [O-go-ops]

### Python: the official SDK removes more request-model ceremony

These functions pass strict mypy **1.19.1** against the official package and the current L installed wheel. The detailed controlled mutation/validation probes are retained on O; L supplies the new installed `/key`/credits/401 receipt. [P-callsites] [P-typecheck] [P-probe] [L-callsites] [L-verification]

```python
# Official openrouter==1.1.137
from openrouter import OpenRouter

def manage(token: str) -> str:
    with OpenRouter(api_key=token) as client:
        credits = client.credits.get_credits()
        client.api_keys.create(name="Comparison key", limit=50.25)
        client.api_keys.update(hash="fixture-hash")
        client.api_keys.update(hash="fixture-hash", limit=None)
        return str(credits.data.total_credits)
```

```python
# Our installed L package, local version 0.1.0
from openrouter import Client, JsonNumber, models

def manage(token: str) -> str:
    with Client(auth={"apiKey": token}) as client:
        credits = client.get_credits()
        client.create_keys(body=models.CreateKeysRequest(
            name="Comparison key", limit=JsonNumber("50.25")))
        client.update_keys(hash="fixture-hash", body=models.UpdateKeysRequest())
        client.update_keys(hash="fixture-hash", body=models.UpdateKeysRequest(limit=None))
        return credits.data.data.total_credits.token
```

The official functions build Pydantic operation models internally and also publish models/TypedDicts. O publishes keyword dataclasses and asks callers to supply the body model. This is a real ergonomic cost for a small request, and a useful explicit reusable object for more complex inputs. It is not accurate to describe either package as untyped. [P-keys] [P-update-model] [O-py-export] [O-py-client]

### Credits/result access at a glance

| Concern | Official TS / Go / Python | O: TS / Go / Python |
| --- | --- | --- |
| Purchased credits | `r.data.totalCredits` / `r.Data.TotalCredits` / `r.data.total_credits` | `r.data.data.total_credits` / `GetCreditsData(ctx)` then `r.Data.TotalCredits` / `r.data.data.total_credits` |
| Numeric type | `number` / `float64` / `float` | `JsonNumber` / `Number` / `JsonNumber` |
| Precision fixture `100.50000000000000001` | Displayed value **100.5** in all three | Original token **`100.50000000000000001`** retained in all three |
| Ordinary success metadata | Flattened payloads for these operations | Canonical response includes actual status/headers/media; Go has a separate direct-data convenience |
| TS advanced metadata/error-result path | `creditsGetCredits(new OpenRouterCore(…))` returns a typed `Result`; `APIPromise.$inspect()` exposes request/response metadata | Standalone source-operation functions also exist, but still return promises/reject; operation-specific error guards narrow `unknown` |

Sources: executed probes and public runtime/types. The official TS standalone success branch was also executed. `$inspect()` is source-verified; it is not the class method’s ordinary awaited return value. Python can inspect HTTPX responses through its transport/hooks; Go callers can wrap/inject `HTTPClient`. Flattened return values do not mean the HTTP response is inaccessible everywhere. [T-probe] [P-probe] [G-probe] [T-async] [T-functions] [P-root] [G-root] [O-ts-ops] [O-go-runtime]

Exact tokens are a **fidelity advantage**, not an automatic overall usability win. Official fields are declared `number`/`double` in the source and map naturally to ordinary arithmetic and libraries. O requires explicit parsing/conversion for fractional inputs and decimal interoperability, e.g. Python `Decimal(value.token)`. Neither the fixture nor these mappings establish a whole-generator decimal-support limitation or a performance result. [source-analysis] [T-credit-model] [P-credit-model] [G-credit-model] [O-demo]

## 4. Presence, validation, literals and errors

### What the matched wire probes establish

| Controlled case | Official current TS | Official current Go | Official current Python | O in the matching language |
| --- | --- | --- | --- | --- |
| Omitted update limit | Sends `{}` | Sends `{}` | Sends `{}` | Sends `{}` in all three |
| Explicit null update limit | Sends `{"limit":null}` | Sends `{"limit":null}` | Sends `{"limit":null}` | Same in all three |
| Create `name=""` | Sends a request | Sends a request | Sends a request | All reject **before HTTP** |
| Nonnumeric `limit="wrong"`, bypassing static types where needed | `SDKValidationError`, zero requests | Native field is `float64`; this invalid assignment is not the Go runtime probe | Pydantic `ValidationError`, zero requests | TS/Python reject before HTTP |
| Credits response missing required `total_usage` | `ResponseValidationError` | Accepted; field becomes **0** | `ResponseValidationError` | All reject during response decoding |
| Credits JSON numeric field supplied as string `"100.5"` | Rejected | Rejected by JSON unmarshal | Accepted by Pydantic coercion | All reject |

Evidence: [T-probe] [P-probe] [G-probe]. The empty-name constraint is present in **both** sides’ inspected specifications and all official overlay outputs; this particular difference cannot be explained away as a different input constraint. It is a bounded constraint-enforcement result, not a claim about every JSON Schema keyword. The Go missing-field result is likewise specific to this response and its generated decoder/configuration. [source-analysis] [T-create-model] [P-create-model] [G-credit-model] [G-config]

**Presence is already a baseline feature.** Official TS uses optional/nullable object members; Go publishes `optionalnullable.OptionalNullable[T]` with `From`, `IsSet`, `IsNull`, `Set` and `Unset`; Python uses `OptionalNullable` and `UNSET`. O uses TS absence/null, Go `PresenceMissing` / `PresenceNull` / `PresenceSome`, and Python `UNSET` / `None`. The different representations are migration work, not a binary “supports PATCH semantics” distinction. [T-update-model] [G-presence] [P-update-model] [O-demo]

### Open enums are an overlay decision, with language-specific APIs

The official overlay opens multi-valued enums using `x-speakeasy-unknown-values: allow`. O retains the original closed `daily`/`weekly`/`monthly`/null declaration. Consequently “ours rejects an unknown limit reset” and “the official SDK can represent future values” reflect different declared policies, not an unconditional validation win. [open-enums] [source-analysis]

- **TS:** `UpdateKeysLimitReset` is an `OpenEnum`, including a branded `Unrecognized<string>`. `unrecognized("yearly")` from `@openrouter/sdk/types/unrecognized.js` typechecks and sends `{"limit_reset":"yearly"}`. A bare `"yearly"` is rejected by the static type. O rejects the invalid closed-enum value before transport. [T-enums] [T-unrecognized] [T-features]
- **Go:** a named-string enum and `IsExact()` retain known-value information; `operations.UpdateKeysLimitReset("yearly")` is representable and was sent. O’s named-string conversion is also syntactically possible, but its codec/call rejects the value against the source enum. [G-update-model] [G-features]
- **Python:** the public type includes `UnrecognizedStr`. A fresh probe with the supported **`UnrecognizedStr("yearly")`** retains that type in `UpdateKeysRequestBody.limit_reset`, but `model_dump()` becomes `{}` and the actual update sends `{}` on **Pydantic 2.12.5**. This is a specific observed unknown-enum serialization problem; ordinary omission/null behavior above still passes. No claim is made about all enums or other Pydantic versions. [P-update-model] [P-base-model] [P-features] [P-dependencies]

### Exact error names and the information they carry

| Case | Official current API | O API |
| --- | --- | --- |
| TS declared credits 401 | `errors.UnauthorizedResponseError` from `@openrouter/sdk/models/errors`; `e.statusCode`, `e.error.message`, `e.headers` | `operations.isGetCreditsApiError(e)` then `e.response.status === 401`; `e.response.data.error.message`, `e.response.headers` |
| Go declared credits 401 | `var e *sdkerrors.UnauthorizedResponseError; errors.As(err, &e)`; typed body at `e.Error_.Message` | `var e *sdk.GetCreditsStatus401; errors.As(err, &e)`; actual HTTP status/headers and typed `e.Data.Error.Message` |
| Python declared credits 401 | `from openrouter import errors`; catch `errors.UnauthorizedResponseError`; **`e.data.error.message`**, `e.status_code`, `e.headers` | catch `operations.GetCreditsStatus401`; `e.data.error.message`, `e.status`, header tuples |
| Unmatched HTTP failure | TS/Python `OpenRouterDefaultError`; Go `sdkerrors.APIError` | Classified unexpected-response failure with source/bounded capture |
| TS transport/input/decode | `ConnectionError`, `RequestAbortedError`, `RequestTimeoutError`, `InvalidRequestError`, `UnexpectedClientError`; `SDKValidationError`; `ResponseValidationError` | Public `SdkError` shape and `kind` such as `request-validation`, `cancelled`, `transport`, `response-decoding` |
| Python input/decode | Pydantic `ValidationError`; `errors.ResponseValidationError` | Public `CodecError` can fail at model construction/encoding; `SdkError` for classified HTTP/runtime failures |

The concrete 401 branches executed successfully. Official Go’s modeled `UnauthorizedResponseError` has body fields, **not** an embedded HTTP status/headers/raw-response object; its fallback `APIError` is a different type with HTTP metadata. O exposes metadata on its typed declared error. Neither official TS/Python HTTP-error superclass is an exhaustive superclass of input, cancellation and native transport failures. [T-probe] [P-probe] [G-probe] [T-error] [P-error] [G-error] [G-fallback] [O-ts-types] [O-py-types] [O-go-runtime]

O’s source-located error/provenance data is particularly useful when diagnosing a schema mismatch. Its terser classified error display can require deliberately inspecting the structured cause/source; official Zod/Pydantic validation messages often contain more immediate field/value detail. Both are genuine debugging tradeoffs. [T-probe] [P-probe] [O-ts-types] [O-py-types]

## 5. Environment credentials, defaults and control policy

### Current runtime behavior

| Concern | Official TS | Official Go | Official Python | O / current L |
| --- | --- | --- | --- | --- |
| Omitted token | `OPENROUTER_API_KEY` fallback | `OPENROUTER_API_KEY` fallback | `OPENROUTER_API_KEY` fallback | Explicit SDK credentials; no automatic mapping in these candidates |
| When env is observed | Module-memoized env; probe A → change B remained A even for a new client | Client-creation snapshot: existing client A, new client B | When building calls without configured security: existing/new client see B | Application decides when its explicit lookup runs |
| Explicit nonempty token | Wins over env | Wins over env | Wins over env | Explicit supplied credentials |
| Explicit empty token | Sends without bearer in the probe | Sends `Bearer ` to controlled transport | Sends `Bearer ` to controlled transport | Unavailable/invalid credentials fail before protected HTTP |
| Explicit `None` / undefined semantics | Nullish token falls through the resolver’s `??` env expression | `WithSecurity` takes a string; source callback supplies a security struct | `api_key=None` uses env; tested | Planned native-env whole-argument precedence must not be assumed already shipped |
| No token and no env | Probe reached controlled transport without bearer | Same | Same | No protected request issued |
| Already-prefixed `Bearer …` input | Prefix preserved | Prefix preserved | Prefix preserved | Raw bearer token expected; the prefixed value was rejected before HTTP in all three |
| Token provider | `() => Promise<string>` | `WithSecuritySource(func(context.Context) (components.Security, error))` | Synchronous `Callable[[], Optional[str]]` | Source-aware per-call credential providers; TS/Python can await callbacks |
| Default API base | `https://openrouter.ai/api/v1` | Same | Same | Same source-declared base |
| Base URL env | `OPENROUTER_BASE_URL` | No corresponding option in inspected config | No corresponding fallback in inspected config | Explicit source/default or caller URL policy |

Sources: executed env/default/prefix/provider probes, runtime source and L’s explicit wrapper/session. “Reached controlled transport” means the SDK sent a request to a mock; it does **not** mean the service accepts missing/empty credentials. All three official provider probes resolved new dummy tokens on the second method call. O’s provider API is source-verified here and has existing native evidence; the new matched probes do not re-certify all OR/AND/provider combinations. [T-probe] [P-probe] [G-probe] [T-env] [T-security] [T-config-runtime] [P-security] [P-root] [G-root] [O-ts-types] [O-py] [O-go] [L-readme] [L-session]

The new **`credential_env` v1** design is explicit configuration: `{ "version": "v1", "schemes": { "apiKey": "OPENROUTER_API_KEY" } }`. It binds an actual uniquely identified used bearer/API-key scheme; it is not inferred from a package name. Its specified runtime contract snapshots at client creation, makes the **whole explicit credential argument authoritative**, preserves OR/AND/anonymous selection, and treats missing/empty/unavailable env honestly before required-auth HTTP. Browser/portable runtimes must remain usable with explicit credentials. This is a valuable concrete plan, but **L’s `5d3…` binary/session does not provide that automatic native env behavior**. No generator-time secret or pending helper is credited as a shipped convenience. [env-contract] [L-session] [L-readme]

### Headers, retries, timeouts and cancellation

| Concern | Current official behavior | Our inspected behavior / migration consequence |
| --- | --- | --- |
| App attribution | TS `httpReferer`, `appTitle`, `appCategories`; Python `http_referer`, `x_open_router_title`, `x_open_router_categories`. Referer/title attachment verified. | These headers are absent from our selected raw source operation inputs. Use a transport wrapper or explicit source/config support; package branding alone does not create attribution fields. |
| Go attribution | `WithHTTPReferer` / `WithXTitle` exist, but the `/key`, credits and chat probes did not send those headers. `operations.WithSetHeaders` **did** send them in the upload probe. | This official Go overlay defines globals but does not add the path-level references used by the TS/Python overlay. It is an observed configuration/output difference, not evidence that Go cannot send custom headers. |
| Custom headers | TS per-call `headers`; Go `operations.WithSetHeaders`; Python per-call `http_headers` | O has declared input headers plus the native transport seam; its generic call options do not expose the same arbitrary-header convenience. |
| Retry defaults | Selected official methods embed backoff for `5XX`, with connection-error retry enabled; initial 500 ms, max interval 60 s, exponent 1.5, max elapsed 3,600,000 ms. Controlled 500 → 200 produced **two** attempts in all three; 429 on credits produced **one** attempt. | No SDK retry loop. A 500 produced one typed failure. Moving existing applications changes automatic retry behavior. |
| Retry controls | TS `retryConfig` / per-call `retries`, `retryCodes`; Python `retry_config` / `retries` (`None` disables), `RetryConfig.status_codes_override`; Go `WithRetryConfig` / `operations.WithRetries` | Caller transport/application policy must supply any intended retry behavior. Preserve operation/replay semantics deliberately. |
| Default timeout | TS no SDK timeout by default. Go default `http.Client.Timeout = 60s`. Python defaults to its HTTPX client’s timeout, 5s per phase in the installed probe. | O TS uses caller `AbortSignal`; Go has no default whole-call timeout but supports context/options; Python’s constructor defaults to 30s per transport phase. |
| Explicit timeout | TS constructor/per-call `timeoutMs`, with an explicit signal taking precedence; timeout generated for each attempt. Python `timeout_ms`, converted to HTTPX seconds. Go `WithTimeout` / `WithOperationTimeout`, implemented using a context. | Native controls remain: TS `AbortSignal.timeout`, Go caller context / `ClientOptions.Timeout`, Python constructor timeout plus outer async deadline. These are not interchangeable deadline semantics. |
| Cancellation | TS `RequestAbortedError`; Go wrapped context error; Python async task cancellation | TS classified `cancelled`; Go `SDKError`/cause; Python retains `asyncio.CancelledError`. Controlled cancel paths were exercised. |

Sources: [T-probe] [P-probe] [G-probe] [G-features] [T-headers] [G-headers] [P-keys] [T-http] [T-retries] [P-retries] [G-retries] [G-root] [O-ts-types] [O-go-runtime] [O-py-runtime]. Root `x-speakeasy-retries` explains the official policy; adjacent `x-retry-strategy` metadata is not the generated runtime’s three-attempt cap. These loops have their own retry budgets; a maximum elapsed retry setting is not a reliable substitute for a whole-operation deadline. [source-analysis]

Default transport behavior also changes: O Go/Python ignore ambient proxy configuration, follow no redirects and request/require identity encoding; official Python’s default clients enable redirect following, and official Go uses a default `http.Client`. TS uses Fetch, with O’s runtime enforcing its explicit redirect policy. Existing application transport assumptions therefore need review during migration. These are source/runtime-policy differences, not a security ranking. [O-go-runtime] [O-py-runtime] [O-ts] [G-root] [P-root]

## 6. OpenRouter-critical chat, streaming, tools and uploads

### Actual official client APIs

| Task | TS | Go | Python |
| --- | --- | --- | --- |
| Chat JSON | `client.chat.send({ chatRequest: { messages, stream: false, … } })` | `client.Chat.Send(ctx, components.ChatRequest{…}, metadataLevel, opts…)` | `client.chat.send(messages=…, stream=False, …)` |
| Streaming | Same method with nested `chatRequest.stream: true`; return type is `ChatResult \| EventStream<ChatStreamChunk>` | Response tagged struct has `ChatResult` or `EventStream`; stream values are `ChatStreamingResponse`, with chunk at `.Data` | `send(..., stream=True)` → `EventStream[ChatStreamChunk]`; `send_async` → `EventStreamAsync` |
| Read delta | Narrow to `EventStream`, then `chunk.choices[0]?.delta.content` | `stream.Value().Data.Choices[0].Delta.Content` | `chunk.choices[0].delta.content` |
| Tool/structured-output request | `tools`, `responseFormat` in `chatRequest` | `ChatRequest.Tools`, `ResponseFormat` typed unions | `tools=`, `response_format=` model/TypedDict inputs |
| Raw file upload | `files.upload({ requestBody: { file: { fileName, content } } })` | `Files.Upload(ctx, UploadFileRequestBody{File: …}, workspaceID, provider, opts…)` | `files.upload(file={"file_name": …, "content": …})` |

All three official packages executed a valid JSON chat response, a typed text chunk, `[DONE]` termination and a small multipart upload. Tool-definition and JSON-schema response-format request bodies were captured. This proves client serialization/decoding for those fixtures, **not model execution, tool execution, structured-output adherence or service compatibility for every provider**. In fact, the controlled chat response is the text `Hello`; the client does not validate that string against the JSON schema requested for the model’s answer. [T-probe] [P-probe] [G-probe] [T-features] [P-features] [G-features]

The official packages also expose the Responses API, embeddings and additional platform resources. Their typed message/tool/provider/routing metadata models are a substantial surface beyond our **current six selected operations**. This report inspects their source but does not claim every endpoint or multimodal/provider combination was executed. [T-root] [P-root] [G-root] [T-chat] [P-chat] [G-chat] [L-session]

### Stream lifetime differences that matter

- **TS:** `EventStream` is a `ReadableStream` with typed async iteration. The controlled early `for await … break` canceled its upstream reader; `[DONE]` yielded one chunk and terminated. The source still declares a union return even for its streaming overload, so the checked consumer narrows the result. [T-probe] [T-stream] [T-chat]
- **Python:** sync/async event streams have context-manager/close APIs. A sync `with events:` plus early break closed the controlled body. Full sentinel consumption also terminated. Async client use and task cancellation were separately exercised; the async stream lifecycle implementation was source-inspected. A bare loop break should not be treated as a universal Python iterator-closing guarantee. [P-probe] [P-stream]
- **Go:** explicitly close `response.EventStream`, including after sentinel/EOF. The probe observed the body still open after sentinel consumption and closed after `Close()`, matching its documented contract. **`WithTimeout(time.Second)` and `WithOperationTimeout(time.Second)` caused `Next()` to return false with `context canceled` before the first chunk**. The method defers cancellation of the context it hands to the returned stream. An application-owned `context.WithTimeout` successfully consumed the chunk. This is a reproduced issue in **v0.7.130**, not a theoretical criticism of context-based streaming. [G-probe] [G-stream] [G-chat]

O’s generated native runtimes and prior normative OAS 3.2 stream witnesses have appropriate native iteration/cleanup APIs: TS async iteration/AbortSignal, Python closeable sync/async stream contexts, and Go `Next`/`Value`/`Err`/`Close` with lifetime context. **Those witnesses do not establish OpenRouter chat support.** [O-demo] [O-ts] [O-py] [O-go] [admission-current]

### Admission attempted against the actual selected source

The report ran read-only `codegen-session --preview` separately for TS, Go and Python, first with O’s `8e567…` CLI and then repeated only the unresolved chat/upload admissions with the newly supplied **`5d3cecd1…`** CLI. No source was edited and no output package root was created by these previews. [admission-old] [admission-current]

| Real operation | Result with the inspected source/configuration |
| --- | --- |
| `getCurrentKey` | O CLI rendered artifacts for all three; preview exited 1 with **`status: drift`**, empty diagnostics and existing-disk differences because the output roots were intentionally absent. That is admission/render success, not a planner rejection. M then generated all twelve; L now adds installed native and controlled `/key` proof across all twelve plus JS. |
| `sendChatCompletionRequest` | Both pinned CLIs reject it before artifacts with **`http-stream-item-schema-required`** at `/paths/~1chat~1completions/post/responses/200/content/text~1event-stream/schema`. The real OAS 3.1 response uses `schema: $ref ChatStreamingResponse` and `x-speakeasy-sse-sentinel: '[DONE]'`; it is not standard OAS 3.2 `itemSchema`. |
| `uploadFile` | Both reject it with **`http-form-untyped-extras`** at `/paths/~1files/post/requestBody/content/multipart~1form-data/schema`: the part object lacks `additionalProperties: false` or an explicit additional-part schema required by this admission policy. |

The chat diagnostics explicitly retain vendor extensions as annotations without inferring their streaming/sentinel behavior. These results establish a **current source/configuration admission gap**. They do not establish that the generator can never produce a chat SDK, can never handle multipart, or that a feature absent from a selected demo package is inherently impossible. No compatible overlay/profile/source transformation has been certified by this comparison. [admission-old] [admission-current] [source-analysis]

**Separate accepted generator evidence:** the ignored-multipart-style/content-descriptor repair is now Main-accepted and native-verified in `typescript_multipart_ignored_encoding::ignored_multipart_styles_preserve_content_in_installed_requests_and_responses`. Its immutable acceptance receipt (`bd36160b148fba1e27d61f5e64d107913cdc94bec85fa6398c61b3f4b7f702ce`) covers TS 5.5.4/5.9.3 × Node 22.23.1/24.21.0, with 11 exchanges, 11 request controls and nine response controls per combination. The defensive `http-typescript-multipart-content-plan-required` guard remains for incomplete plans. This is real additional generator capability proof, not an unfixed issue. It is distinct from the **`http-form-untyped-extras`** failure above, and the handoff is not a regenerated L package or a new admission run on OpenRouter’s upload source. No native matrix was repeated here. [multipart-handoff] [multipart-acceptance]

### Pagination and binary transfers

All three official BYOK list probes fetched offset **0 → 1**, using limit 1 and returning a nonempty then empty page. The native interfaces differ: TS async page iteration; Go `res.Next()`; Python `res.next()`. For this endpoint the SDK wraps the API payload under `result` / `Result`; it is not the same return shape as credits. This behavior comes from its pagination declaration, not from the mere presence of a list in a response. [T-features] [P-features] [G-features] [T-byok] [P-byok] [G-byok]

For the **matched container-files** operation, the official methods return `ContainerFileListResponse` directly and leave `limit`/`after` traversal to the caller, just like O. Official TS injects the declared limit default through its schema, Python’s convenience signature defaults `limit=100`, and Go’s parameter model carries `default:"100"`; O preserves omitted input and does not inject schema defaults. The API description itself says absent limit defaults to 100. This is a wire-policy difference even where the service’s expected result is the same. [T-containers] [T-container-model] [P-containers] [G-container-model] [O-ts] [O-py-client] [source-analysis]

The official file-input types support TS `Blob`/`ReadableStream`/byte buffers, Python bytes/file-like objects, and Go `[]byte`/`io.Reader` through a content field typed `any`. The small upload probes establish filename/content framing and decoding, **not large-file memory behavior or throughput**. Official download methods expose native streams/readers/HTTPX responses. O’s inspected generic multipart/binary representations are finite in-memory values, with explicit budgets; selected OpenRouter upload remains blocked as above. [T-upload-model] [P-upload-model] [G-upload-model] [T-files] [P-files] [G-files] [O-ts] [O-go]

### Client SDK versus Agent SDK

The TS README directs `callModel` and associated tool-orchestration users to **`@openrouter/agent`**. The current client repository and npm package still contain a `callModel` method in an explicit custom-code region, plus hand-written orchestration helpers. The root public export is the client SDK; automatic tool execution and turn orchestration must not be attributed to unconfigured generated HTTP methods. Python and Go chat tool fields send definitions to the service; this report did not run an agent loop or install the Agent SDK. [T-readme] [T-root] [T-callmodel] [T-probe] [P-probe] [G-probe]

## 7. Attribution: generator, service schema, overlay or custom code?

| Observed UX/behavior | Supported attribution |
| --- | --- |
| Official package names, `OpenRouter` class, model casing and request flattening | `.speakeasy/gen.yaml`: TS `maxMethodParams: 0`, camelCase, flat responses; Python `flattenRequests: true`, `maxMethodParams: 999`, sync+async mode; Go `maxMethodParams: 4`, inferred optional arguments, flat responses. Our identities are likewise explicit target config. [T-config] [P-config] [G-config] [L-session] |
| `apiKeys.create`, `.update`, chat `.send` and grouping | OpenRouter source name/group overrides plus generator interpretation. O deliberately retains source operation IDs in native casing. [source-analysis] [T-keys] [P-keys] [G-keys] [O-ts-ops] |
| Attribution-header convenience | OpenRouter header overlays/globals; the TS/Python path-level injection and Go difference described above. [T-headers] [G-headers] |
| Future enum values | Open-enum overlay and generated native unknown-value representations, with the observed Python serialization caveat. [open-enums] [T-enums] [P-base-model] [G-update-model] [P-features] |
| Retry/backoff, pagination and SSE sentinel/stream selection | Explicit source extensions interpreted by the official generator/runtime. Our no-profile configuration does not infer them. [source-analysis] [T-byok] [G-byok] [P-byok] [admission-current] |
| Other official source normalization | TS/Python each apply seven overlays; Go applies six. Shared overlays include open enums, RSS response removal, header globals, allOf simplification, boolean query handling and deprecated beta-response aliasing. TS adds nullable-model fields; Python adds nullable-pagination repair. [T-workflow] [P-workflow] [G-workflow] |
| High-level TS model/tool orchestration | Explicit custom code and helper modules, accompanied by an Agent SDK migration notice. [T-root] [T-callmodel] [T-readme] |
| Registered lifecycle hooks | Inspected registration files are empty/no-op; a hooks framework’s existence does not mean a custom hook caused the measured behavior. [T-hooks] [P-hooks] [G-hooks] |
| Our exact validation/provenance and policy | Source-backed generated codecs/runtime, selected canonical protocol plan and explicit generation options. Scope is the admitted selected closure. [O-ts] [O-go] [O-py] [O-session] |

All three official workflows pin **Speakeasy CLI 1.787.0**; generated runtime metadata reports **generator 2.914.0** and OpenAPI document version **1.0.0**. Those are distinct from the SDK release versions and from our CLI binary hashes. The repositories retain input/output specifications, workflow locks and generated markers. They already have generation provenance; O’s distinctive evidence is its physical source pointers/spans, operation/model-level links and checked artifact/compatibility workflow, not the assertion that the baseline has no provenance. [T-workflow] [P-workflow] [G-workflow] [T-config-runtime] [G-root] [P-version] [O-pins] [O-demo]

## 8. Documentation, distribution and runtime footprint

### Documentation and examples

The official repositories provide resource and model references, normal registry installation recipes, environment/native-transport recipes, file/pagination examples and OpenRouter-specific feature examples. Go’s README has a full resource list and runtime-control sections. TS has `RUNTIMES.md` and a standalone-function guide; Python has sync/async, TypedDict/Pydantic and context-manager guidance. These are substantial consumer-facing assets. Source existence alone is not a claim that every published example works. [T-readme] [T-runtimes] [T-functions] [P-readme] [G-readme] [OR-ts] [OR-py] [OR-go]

Concrete current documentation problems found:

1. **TS README chat usage fails the actual published typecheck:** fields are shown directly under `chat.send`, while the method requires `chatRequest`. The retained negative check reports **TS2769**, and the unchecked direct iteration also reports **TS2504** for the union result. The working controlled consumer uses the actual wrapper and stream narrowing. [T-readme] [T-negative] [T-probe-source]
2. **Python README assigns `res` but iterates `event_stream`** in both shown sync/async chat examples. That variable is undefined in the snippet. Its pagination prose says `Next`; its code and actual public API use `.next()`. [P-readme] [P-features]
3. **Go README says “Go 1.25 or higher”; the module actually declares `go 1.25.10`.** The release is labeled beta. The manifest is the precise floor for consumers. [G-readme] [G-package]
4. TS/Python READMEs suppress several generated authentication, retry, error, server and HTTP-client sections, even though the implementation supports them. Their runtime APIs must be checked in source/reference docs rather than guessed from absent README sections. [T-readme] [P-readme]

Our documentation has made meaningful progress since the historical DX report: source-linked model/HTTP docs, checked task examples, native install receipts, concise model names and no-input/direct-data conveniences. O’s generated quickstart choice was not uniformly the simplest onboarding: its TS README starts with container files and Python with creating a management key. **The current `LIVE-DEMO-README.md` fixes that demonstration path:** clean imports, `/key` by default, a separate management credits mode, explicit wrapper credential lookup, real prepared native programs and controlled read/error receipts. It still accurately says no real-token request has been made. [O-ts] [O-py] [O-demo] [L-readme] [L-verification]

### Dependencies and platform floors

| Language | Current official distribution | Current L package (same floors/dependencies as O) | Actual tools used for the new probes |
| --- | --- | --- | --- |
| TS/JS | ESM; runtime dependency `zod: ^3.25.0 \|\| ^4.0.0`, imports Zod v4; `sideEffects: false`, root and subpath exports. No numeric Node engine floor in package manifest. `RUNTIMES.md` lists Fetch/Streams/ES2020-compatible targets. | ESM; zero runtime npm dependencies; package declares **Node ≥22**; local package marked private/prototype/releaseReady false. | **Node 22.23.1**, npm **10.9.8**, TS **5.9.3**, installed Zod **4.6.2**, `@types/node` **22.13.12** |
| Go | **Go ≥1.25.10**; module requires `github.com/spyzhov/ajson v0.8.0` and `github.com/stretchr/testify v1.12.1`, with YAML indirect. The runtime pagination implementation imports `ajson`; test dependencies are not automatically runtime imports. | **Go 1.23**; standard-library-only `go.mod`. | **Go 1.26.5**, `GOTOOLCHAIN=local`, `GOWORK=off`; both packages compiled in the same private consumer module |
| Python | **Python ≥3.10**; `httpcore>=1.0.9`, `httpx>=0.28.1`, `jsonpath-python>=1.0.6`, `pydantic>=2.11.2,<2.13`; typed wheel. | **Python ≥3.11**; `httpx==0.28.1`; native typed dataclasses/codecs. | **Python 3.11.15**, HTTPX **0.28.1**, Pydantic **2.12.5**, mypy **1.19.1** |

Sources: actual manifests and install/version receipts. Full dependency resolutions, tool executable paths/hashes, compiler commands and stdout/stderr are retained. The matched official/O Go module and current L call-site compile use Go 1.26.5; L’s separate staged read consumer was built on Go 1.23.12. PyYAML was separately installed only as a source-inspection tool, not as an SDK dependency. The official TS runtime document’s “currently v18/v20/v22” wording is reported as repository documentation, not independently certified September-2026 runtime support. These probes do not validate every stated platform floor. [T-package] [T-runtimes] [P-package] [G-package] [L-ts-package] [L-py-package] [L-go-package] [L-native] [install] [commands] [P-dependencies]

**No import-cost, tree-shaking, cold-start, bundle-size or throughput ranking is claimed.** There was no measurement using equivalent operation breadth/build settings/runtime boundaries. Official TS’s standalone API and `sideEffects: false` are real packaging features; O’s zero dependency count is real. Neither alone proves a smaller application bundle or a faster import. [T-package] [T-functions] [O-ts-package]

## 9. Feature/DX matrix: better, same, worse or unknown

The judgments here are task-specific, not an aggregate score. “Missing” refers to the compared selected package; an admission failure is separately identified.

| Dimension | Current official baseline | Our state | Assessment |
| --- | --- | --- | --- |
| Native package imports | Installed published TS/Go/Python releases | L clean-name packages installed and native-checked | **Same basic mechanism; official published distribution ahead** [install] [L-native] |
| Normal-token first read | `/key` executes in controlled probes | L’s `/key`, credits and typed 401 pass the new controlled consumers | **Native read proof gap closed; no real service request on either side in this research** [T-probe] [P-probe] [G-probe] [L-verification] |
| Automatic env setup | Available now, with differing snapshot/precedence behavior | Working env/prompt wrapper; automatic native v1 policy absent from L | **Official SDK construction remains more convenient** [§5](#5-environment-credentials-defaults-and-control-policy) |
| Small TS requests | Plain objects plus resource and request-body wrapper | Plain objects, short root methods, exact-number input | **Tradeoff** [T-callsites] |
| Small Go requests | Grouped methods, optional pointers/wrappers | Required constructors, extra input wrappers, direct-data convenience | **Tradeoff; O stronger preflight on tested constraint** [G-probe] |
| Small Python requests | Flattened keyword convenience | Explicit body dataclass | **Official easier for these tasks** [P-callsites] |
| Absent/null PATCH | Supported and tested in all three | Supported and tested in all three | **Same capability, migration syntax differs** [T-probe] [G-probe] [P-probe] |
| Exact numeric token preservation | Host floating point for credits | Exact token in all compared targets | **O stronger preservation, more conversion ceremony** [T-probe] [G-probe] [P-probe] |
| Schema validation | Zod/Pydantic/Go JSON wrappers, with concrete gaps/coercion | More faithful tested name/missing-field/type checks | **O better on measured cases; universal semantics unknown** [§4](#4-presence-validation-literals-and-errors) |
| Resource discoverability | Grouped keys, credits, chat, containers, files, etc. | Flat selected operation methods; source identities retained | **Official easier to browse as API grows** [T-root] [P-root] [G-root] [O-ts-ops] |
| Error/status/media metadata | Structured errors; ordinary results mostly flattened; Go modeled errors lose HTTP metadata | Canonical typed status/media/headers and source-linked errors; Go direct-data option | **O clearer default metadata; TS official standalone Result is a useful alternative** [§4](#4-presence-validation-literals-and-errors) |
| Retry/timeout knobs | Available and implemented, including the Go stream-timeout defect | Native cancellation/deadline seams; no retry policy | **Official broader operational convenience; behavior must be migrated explicitly** [§5](#5-environment-credentials-defaults-and-control-policy) |
| Chat/tool/structured request coverage | Executed bounded fixtures | No chat in current L; real selected source rejected by both CLIs | **Blocking replacement gap** [admission-current] [L-session] |
| SSE | Vendor chat SSE and sentinel executed; native lifecycle APIs | Normative OAS 3.2 support/evidence; vendor 3.1 chat unproven/rejected | **Different proven protocols, not parity** [§6](#6-openrouter-critical-chat-streaming-tools-and-uploads) |
| File upload | Actual small multipart upload passes in three current packages | Generic multipart capability; selected real upload blocked | **Blocking for file consumers** [T-features] [P-features] [G-features] [admission-current] |
| Pagination | BYOK automatic traversal tested; container-files caller-driven | Container-files caller-driven; no inferred pagination loop | **Same on matched container operation; official broader convenience elsewhere** [§6](#6-openrouter-critical-chat-streaming-tools-and-uploads) |
| Client-side tool orchestration | TS custom/Agent SDK surface is separate | Not part of these packages/proofs | **Separate product scope, not a client-generator score** [T-callmodel] [T-readme] |
| Provenance, generation/review loop | Input/output specs, locks and generation markers | Fine-grained source provenance plus sealed native/zero-rewrite/compatibility receipts | **O has a useful stronger local workflow story; no controlled generator-wide ranking** [O-demo] [O-pins] [T-workflow] |
| Runtime footprint/performance | No matched measurement here | No matched measurement here | **Unknown** [§8](#8-documentation-distribution-and-runtime-footprint) |

## 10. Migration guide and priorities before claiming replacement

### Source-breaking code changes

| Existing application concept | Required change for our inspected SDK |
| --- | --- |
| Import / construction | `OpenRouter` → `createClient` in TS; `openrouter.New` → `sdk.NewClient` plus error handling in Go; `OpenRouter` → `Client`/`AsyncClient` in Python. Select the actual local generated distribution, even where its name now matches. |
| Resource methods | `apiKeys` / `APIKeys` / `api_keys` methods become root operation-ID methods; `getCurrentKeyMetadata` becomes `getCurrentKey` / `GetCurrentKey[Data]` / `get_current_key` in L. |
| Request structure | TS `requestBody` → `body`; Python flattened kwargs → `body=models.…`; Go flattened update args → `NewUpdateKeysInput(hash, body)`. |
| Field names | TS official camelCase fields → O’s source wire spellings; Go exported fields remain Go-style; Python wire-like snake_case largely stays familiar. |
| Numbers | Replace ordinary fractional values with `JsonNumber.parse`, `ParseNumber`, or `JsonNumber`; migrate arithmetic/serialization/storage intentionally. |
| Optional/null values | Go `optionalnullable` → generated `Presence`; Python imports/sentinels change; preserve actual absence rather than converting every missing value to null. |
| Return shapes | TS/Python add the HTTP response layer; Go choose canonical status result versus `…Data` convenience. BYOK/other official wrappers are endpoint-specific. |
| Error handling | Replace shared official HTTP error catches with exact operation errors/guards and handle codec/SDK failures separately. Python official message path is `e.data.error.message`, not `e.error.message`; Go O declared errors are pointer types for `errors.As`. |
| Async code | Python `_async` method suffixes become an `AsyncClient` with ordinary snake_case methods. TS promises and Go contexts stay native. |
| Chat, Responses, uploads, automatic pagination | No demonstrated complete replacement path in L. Keep these as explicitly unresolved coverage work, not a mechanical import rename. |

These changes follow the inspected exports, matching consumers and current L call-site typechecks. L’s normal-key import/read path is now native-checked; detailed mutation/validation probes remain pinned to O. [T-callsites] [P-callsites] [G-probe-source] [L-callsites] [L-verification] [T-probe] [P-probe] [G-probe] [admission-current]

### Behavior changes even after the code compiles

1. **Credential timing/precedence changes.** Current official TS memoizes environment values, Python can reread them per call, and Go snapshots per client. Our explicit wrapper chooses its own timing today; v1’s proposed construction-time/whole-argument policy is distinct. Prefixed bearer strings need to become raw tokens. [§5](#5-environment-credentials-defaults-and-control-policy)
2. **Retries, timeouts, redirects and proxy/compression policy change.** Equivalent method names do not preserve these operational defaults. For current official Go chat, use caller-owned context deadlines rather than its reproduced prematurely canceled SDK timeout path. [§5](#5-environment-credentials-defaults-and-control-policy) [G-probe]
3. **Previously accepted invalid/malformed values may fail earlier.** Empty names, missing required response fields and numeric strings illustrate the distinction. Code that relies on permissive response coercion needs an explicit decision; early rejection is useful but can also surface service/schema mismatches. [§4](#4-presence-validation-literals-and-errors)
4. **Unknown enum policy changes.** The official open-enum overlay is not automatically reproduced by our standard-only configuration. Decide which fields should remain closed and which require an explicit forward-compatible representation. [source-analysis] [open-enums]
5. **Model JSON and native JSON are not interchangeable.** Preserve our exact numeric tokens through generated codecs rather than ordinary host-float serialization, and keep absence/null states intact. Request schema validation does not establish LLM-output adherence to a requested JSON schema. [O-demo] [§6](#6-openrouter-critical-chat-streaming-tools-and-uploads)

### Highest-value usability/replacement work

**P0 — support and prove OpenRouter’s actual inference path.** Resolve explicit chat SSE/source admission, JSON and streaming call shapes, vendor `[DONE]`, error chunks, cancellation/early close, tool definitions and structured-output request models. Then exercise real selected-source generation plus controlled native consumers for TS/Go/Python. Normative 3.2 streams and a five-operation key demo are insufficient substitutes. [admission-current]

**P0 — finish automatic native credential convenience.** L already supplies clean installed imports, a checked `getCurrentKey` read, a management-only credits mode and working wrapper env/prompt setup. Preserve that proven path and expose native automatic env construction only after its snapshot/precedence/unavailable-env controls pass. Keep the runner handoff aligned with its delivery seal (§12). This calls for new-policy evidence, not rerunning the old all-language matrices. [L-readme] [L-evidence] [L-delivery] [env-contract]

**P1 — choose an explicit operational convenience policy.** Consumers currently have app headers, retries, per-operation timeout/header controls and automatic traversal for declared paginated endpoints. Provide documented native helpers or deliberate application recipes; preserve the semantics rather than silently inheriting a baseline bug or inventing retries from an operation name. [§5](#5-environment-credentials-defaults-and-control-policy) [T-features] [P-features] [G-features]

**P1 — reduce ordinary-call ceremony without losing the canonical API.** Add resource/task discovery and consider flattened Python conveniences and shorter Go input construction where unambiguous. Keep status/source-rich responses available alongside direct-data helpers. Exact-number conversions deserve prominent native examples rather than a claim that precision is always worth the friction. [§3](#3-matched-keycredits-call-sites-and-return-shapes)

**P1 — close the file-upload admission and distribution/docs gaps.** A real upload consumer already has working official SDK APIs; explain and prove any chosen multipart source normalization. Bring generated package quickstarts/reference material and eventual distribution in line with L’s curated clean-import `/key` path. Reuse the existing native packaging receipts rather than rebuilding the old matrices for each documentation change. [admission-current] [O-native] [L-readme]

**P2 — measure cost only at matched boundaries.** If bundle/import/latency claims become important, compare the same operations, compiler/bundler settings, runtime, dependencies included/excluded and cold/warm conditions. Current package/source file counts do not answer that question. [§8](#8-documentation-distribution-and-runtime-footprint)

## 11. Expectations for our other nine languages

There is no official same-language OpenRouter package installed for these nine in this research. The table asks qualitatively whether their demonstrated native interfaces supply the conveniences already present in the TS/Go/Python client baseline. It reuses O’s body/semantic examples and **L’s new clean-name read/error consumers**; it is not nine extra head-to-head benchmarks or 36 quantitative comparisons. All nine now have L’s installed `/key`/credits/401 preparation evidence. [O-native] [O-demo] [L-evidence] [L-readme]

| Our target | Where its demonstrated DX meets the baseline expectation | Remaining friction / expectation to carry forward |
| --- | --- | --- |
| **Rust** | Native `Result`, required-field constructors/builders, async futures, typed API variants, `Presence` and exact numbers; direct/default read convenience. | Enable a transport Cargo feature and choose the application executor; boxed error variants and numeric/presence conversions add ceremony. Provide the same clear `/key`/credential/timeout story in Rust terms. [Rust-example] [O-demo] |
| **Swift** | `async/await`, value models, `Sendable`, typed enum errors and no-input reads; ownership-aware stream guidance. | Exact/presence wrappers and generated `SDKJSONEncoder`/`SDKJSONDecoder` boundary need a short task-oriented explanation. [Swift-example] [O-demo] |
| **Java** | Immutable models/builders, direct single-success results, sync and `CompletableFuture`, native try-with-resources. | Required builder factories and nested input/result types are more ceremony than official Python kwargs; examples should make the first request and optional/null choices obvious. [Java-example] [O-demo] |
| **C#** | Records, `Task`, `CancellationToken`, typed operation exceptions and injected `HttpClient`; native disposal. | `Optional<T>` alongside nullable types adds a concept; demonstrate named cancellation arguments and practical exact-number interop. [Csharp-example] [O-demo] |
| **Kotlin** | Native suspend methods, named data-class arguments, typed exceptions and coroutine cancellation; documented cold-Flow streaming model. | `Presence<T?>` and mutable-collection validation need concise recipes; generation of a stream-capable runtime still does not prove vendor chat admission. [Kotlin-example] [O-demo] [admission-current] |
| **Ruby** | Keyword models/methods, `UNSET` versus `nil`, block-managed client lifetime and real typed exception classes. | Runtime checks and numeric wrappers are unavoidable visible concepts; explain the allocated `hash_value:` name and show env setup directly. [Ruby-example] [O-demo] |
| **PHP** | Composer/autoload integration, native typed constructors, exceptions, interface/transport seam and omission/null representation. | Calls are synchronous; this should be presented idiomatically rather than scored against TS promises. Response payload is `body`, and retained streams require explicit cleanup guidance. [PHP-example] [O-demo] |
| **Dart** | Null-safe named arguments, `Future`, cancellation and portable/native transport split; installed VM consumer. | Explicit `IoTransport` and presence wrappers add setup. Portable/browser env limitations must remain honest; L still uses explicit credentials. [Dart-example] [O-demo] [L-readme] [env-contract] |
| **C++** | CMake-installed package, value models, `Result`/`variant`, RAII and stop-token/deadline controls. | Result/variant handling and curl/CMake setup are more verbose; a compact complete read example matters. No implicit coroutine runtime or vendor-chat capability is established. [Cpp-example] [O-demo] |

L’s checked clean import/read forms are: Rust `openrouter` / `get_current_key_default`; Swift module `OpenRouter` / `getCurrentKey`; Java `ai.openrouter.sdk` / `getCurrentKey`; C# namespace `OpenRouter` / `GetCurrentKeyAsync`; Kotlin `ai.openrouter.kotlin` / `getCurrentKey`; Ruby `require 'openrouter'` / `get_current_key`; PHP namespace `OpenRouter` / `getCurrentKey`; Dart `package:openrouter/openrouter_io.dart` / `getCurrentKey`; C++ `<openrouter/sdk.hpp>` / `get_current_key`. The complete programs include the native setup and cleanup omitted from these shorthand names. [L-readme] [L-evidence]

Across these nine, real native/package evidence and the now-checked normal-key read are meaningful progress. Remaining replacement gaps are broader operation coverage, automatic native credential convenience, explicit operational policy and actual OpenRouter chat/file compatibility. Additional target count does not compensate for missing a core customer task. [O-native] [L-evidence] [admission-current]

## 12. Evidence ledger and limits

Everything newly written for this research is under **`target/sdk-current-comparison-20260911-01/`**, plus this report. Production code, upstream source and the historical research report were not edited. [evidence-root]

| Evidence | Location / interpretation |
| --- | --- |
| Fresh versions, repository heads and distribution hashes | `primary-pins.json`; `registry/`; `distributions/`; HTTP receipts include URL/time/headers/hash |
| Primary source archives/config/overlays | `sources/{typescript,python,go}/`; short local links `T`, `P`, `G` (Go release), `G-head` |
| Release/head/package identity checks | `distribution-source-parity.json`, `sources/go/release-to-head.json`, `local-package-verification.json` |
| Source policy attribution | `source-analysis.json`, produced by `source_analysis.py` from pinned input/output specs |
| Private actual installations | `typescript/node_modules`, `python/venv`, `go/go.mod`/`go.sum`, `go-mod-cache`; `install-result.json` |
| TS matched/feature evidence | `commands/ts-probe-typecheck-01`; **`ts-probe-run-02`**; `ts-features-typecheck-02`; **`ts-features-run-02`**; expected-negative `ts-readme-negative` |
| Python matched/feature evidence | `commands/py-callsites-typecheck`; **`py-probe-run-02`**; **`py-features-run-02`** |
| Go matched/feature evidence | `commands/go-probe-build-03`; **`go-probe-run-02`**; `go-features-build`; **`go-features-run`** |
| Real-source admission | `admission-summary.json` and `admission-current-summary.json`; each case has full JSON diagnostics and command output |
| Reused O native proof | Existing `native-index.json`, `native/*/prepared.json`, commands and consumers linked below; no full old matrix replay |
| CURRENT L proof | `current-live-evidence.json` plus retained `current-live-context/`; original `native/*/ready.json` and `verification/*/REPORT.json`, 39 passing controlled checks |
| Current clean-import call-site checks | `current-callsites-result.json`, `ours-current/{typescript,python,go}/`, and `commands/current-*-callsites`; type/build only, no extra mutation or live calls |
| Additional accepted generator capability | `additional-generator-context/`; original ignored-multipart acceptance receipt and Main runner handoff, consumed without matrix replay |

**Delivery observation:** the later `delivery-01` report hashes to `9729e34a8d536e0136512b4e346d8ba701faff1e74f3d43a54d9fc694838d082` and its 1,250-file manifest to `4cdb34eed8c3f424c60e190da3857b9f01fa9841902195c99d5d09f602b6c286`, matching `seal.json`. At the read/hash observation recorded in `delivery-and-additional-context.json`, **only `tools/sdk-demo-live/run.py` differed from that manifest**. L’s package pins, native programs and controlled SDK receipts remain the comparison evidence; a runner edit after sealing is not an SDK runtime failure and is left to its owner’s handoff. [L-delivery] [delivery-observation]

**Dated clarification — 11 September 2026:** that mismatch is the **known, explicit post-`delivery-01` truncation/redaction fix**. Main’s independent red receipt reproduced a canary straddling the 65,536-byte capture cap: the full canary was absent, but a partial prefix remained. The corrected `tools/sdk-demo-live/run.py` hashes to **`578b79a4a34bf5a55966e217c96d3322ac31955ad9bf2015de09d9bcb85d3ee7`**, matching both the fix receipt and the independently green-tested source. It discards each truncated stream entirely before persistence, parsing or display. The independent green stdout and stderr cases used real controlled-child capture; both refused success and returned orchestration exit **1** despite child exit **0**, with no stored canary prefix and no confirmation-parser call. These were wrapper-only checks with the prepared-consumer source stubbed: no SDK, API or real token was exercised. [redaction-red] [redaction-green] [redaction-fix] [redaction-observation]

**Successor seal status:** Main’s correction handoff approved a fresh `delivery-02` seal with sealing pending. During this addendum check, `delivery-02` became available as `staged-ready-not-live-executed`: report SHA-256 **`bc482dc56ecdedfc0af44f4564f723a062e7cf6bed2efcb10063354e2bdba85b`** and manifest SHA-256 **`2da9b61493e01e83566036c2a09aa8b0e475f3abb7a0991fb9164e05764cc43b`** match its new `seal.json`, and its manifest records the fixed runner hash above. This is a successor receipt, not a claim that the corrected runner matches `delivery-01`. The original observation and old seal remain preserved; the twelve native builds, 39 controlled SDK checks, package baseline and comparison findings stand. This addendum checked receipts/source hashes and report citations only. [redaction-delivery02] [redaction-seal-check] [L-evidence]

**Separate generator-context postscript — 11 September 2026:** Main’s pinned integration handoff now qualifies native environment-credential adapters for **Python, Java, PHP, Dart, Ruby and C#**. Its `canonical-six-01` all-feature receipt passes eight checks, and the CLI policy-process check passes: direct/Session output equality, shared-contract reuse without recompilation on policy changes, typed capture, unchanged wire semantics with changed native policy, and cached `Arc` reuse on reverts. At that handoff, the other six—including **TS and Go**—remain gated pending proof. This is subsequent source/integration progress: **L’s frozen `5d3…` packages still use manual wrapper credentials**, so the pinned comparison tables and matched-probe findings are not relabeled. The handoff, canonical/CLI receipts and Python native completion receipt are hash-pinned in the addendum evidence; no SDK/native probes were replayed. [env-progress-pins] [env-canonical-six] [env-cli-policy] [env-python-completion] [L-session]

**Later generator qualification — 11 September 2026:** the source/native qualification now advances from the preceding six-adapter checkpoint to **all twelve accepted/open credential-env adapters**. Main’s remaining-five receipt covers TS, Go, Rust, Swift and Kotlin, with **338 verified entries / 50 pinned files**, alongside the earlier six and C++ receipts. `all-twelve-bridge-01` passes **eight credential-env checks** and **two resource/ordinary-output checks**: all-twelve generation/Session/capture admission and full-file Rust V2/V3 parity for the witnessed ordinary fixture. Canonical Rust generation and its matching capture now use `plan_http_v3`. The separate CLI report red/green receipt, using its Python fixture, proves conditional `credentialEnv` names-only reporting, identical generated files/revision under two different generator-environment canaries, and restoration of ordinary files with no helper when the policy is disabled. [env-all12-pins] [env-all12-bridge] [env-final-five] [env-defaults-green]

This later qualification is pinned in the receipt/source inventory SHA-256 **`580247a5ecdd42c15398a0d5819b07f0fa21e663ee85223610c32a8ec253b156`**. At Main’s forwarded checkpoint, `target/sdk-main-env-candidate-20260911-01` was **building, without a final artifact receipt**; this postscript therefore qualifies generator/native integration, not a new live env-enabled package candidate. **L’s frozen `5d3…` bytes and wrapper-auth observations retain their original scope**, as do the comparison’s chat/vendor-SSE/upload coverage findings. Only receipts/source hashes and this report were checked for the update; no native or matched SDK matrix was replayed. [env-all12-pins] [L-session] [admission-current]

**Frozen automatic-env generation checkpoint — 11 September 2026:** the separate CLI is now built at `target/sdk-main-env-candidate-20260911-01/bin/suspect`, SHA-256 **`f971ead49e99a15e205325b186b2a7effb640e412da31f8c24e100983c59f496`**. Its source manifest is **`651f17f5c645cd5135d84b0cc3d5216669ee1a6aa15507a8d0e3615c977ee1d2`**, covering 1,157 frozen inputs. `live-generation-01/packages` contains the same six source operations/twelve clean identities with `credential_env` v1 mapping `apiKey` to `OPENROUTER_API_KEY`: **610 desired artifacts plus ownership = 611 files**. Session SHA-256 is **`877a532f9b2e40bdca356937f3df134d24d021b0164435b980ee46f670c6274e`**; package-pins SHA-256 is **`c648df1130b975dce56e9299f3ede23467d15a468ef8330d92928eab6d091413`**. These pins and all 611 configured package-file hashes were checked read-only for this addendum. [env-frozen-cli] [env-frozen-pins]

The definitive **`live-generation-01/completion-02/REPORT.json`** records all **593 unconfigured files exactly equal to L**, absent generator canaries and identical configured files/revisions across two generator processes, twelve Planned native captures with six operations and typed env descriptors each, and native-policy-only changes with no wire changes when the variable is edited. The initial reader-shape mistake and completed generations remain retained. **This is frozen generation/canonical evidence, not new-cohort compiled-consumer evidence:** the receipt has `nativeExecution: false` and `realApiContacted: false`; the docs owner’s native-helper edition needs its own preparation. L remains the compared, prepared wrapper-auth entry, and the official version pins, matched findings and chat/vendor-SSE/upload coverage scope are not revised. No generation or native/matched probes were replayed for this update. [env-frozen-completion] [env-frozen-pins] [L-session]

**Pending-review qualifier — 11 September 2026:** Main reports two independently confirmed **P2** findings in the new credential-env work: Go environment-factory renaming is omitted from native compatibility capture, and Dart’s nullable configuration relaxes pre-existing type/null errors. **Final env acceptance is held pending the existing Go/Dart owners’ fixes, rechecks and review closure.** “Accepted/open” above describes the bounded native/canonical gates, not complete review closure. The all-twelve proofs, frozen `f971…` generation/capture and 593-file ordinary parity remain valid at their recorded scopes; L’s comparison is not changed. Exact fix/review receipts await Main’s handoff; this is an attributed status update, not a new reproduction. [env-review-hold]

**Adjacent delivery update — 11 September 2026:** Main accepted the additive **F10 consumer-observability overlay**, available through **`./demo-live-v2.sh all`** and its separate addendum. It replaces only the TS/Python consumers so their failure records retain SDK status/category metadata already present in the installed packages; the other ten prepared programs and SDK bytes are reused. Main’s receipt verifies **680 seal entries / 3,113 protected file-metadata records**, and accepts **16 controlled failure transcripts**; this comparison addendum checked its report/manifest hashes and two execution pins read-only. The failed initial Python hook escaped its intended mock, made an invalid-canary remote GET and received 401; that attempt is explicitly disclosed and **excluded**. The corrected Python controls block socket access, and the accepted proof records no API contact or real-credential use. This is later consumer-overlay evidence, not a retroactive expansion of the original SDK comparison’s API/token claims or a change to its sealed L findings. [observability-owner] [observability-addendum] [observability-pins]

**ENV review remains open:** Main’s latest status keeps final acceptance held for the Go/Dart P2 work plus fresh review of the common empty-capture boundary correction. The new ninth check, `empty_operation_capture_cannot_skip_configured_native_admission`, has a retained red/green pair; its focused green run is **one passed, eight filtered out**, not a replayed nine-test suite or a new native matrix. Earlier passing native/canonical proofs retain their bounded scope. [empty-capture-green] [observability-pins]

**Dart review follow-up — 11 September 2026:** the independent **Standards recheck is now CLEAN**, resolving the original Dart P2 for the pinned repair. Its owner report hashes to **`aa2c1f73ca59d3f4a12cd6972d26307210a9a8a588bd90ca6974a02eae69d346`** and `standards-dart-recheck-01.md` to **`17750b578e6597ea591e19bea97a20c32dca0146402b018837228ceffdeba5f2`**; both were verified for this status update. This supersedes the earlier pending Dart recheck status only. Main still reports **Go Spec environment-factory capture closure pending**, so overall env acceptance and the fresh combined CLI remain separate, unclosed steps. These are current repair/review deltas, not retroactive changes to frozen `f971…` or L; no matched/native probes were replayed. [dart-standards-recheck] [dart-review-pins]

**Combined ENV source/CLI review closure — 11 September 2026:** the preceding source/canonical review hold is now **CLOSED for the new replacement boundary**. Both `fixes-01` independent axes are CLEAN: Standards SHA-256 **`08fe99c7a637ad728fa7b4737c8c60422e1656919dd176036c5e066e788e1a76`** and Spec **`2419fb3ccca64561f3a2e1f9b0b2dd393b622742786859496e825ca1b684627e`**. Go’s factory-capture P2 and Dart’s whole-null P2 are resolved, with Main’s configured-empty admission correction included. The definitive **`ENV-ACCEPTANCE.json`**, SHA-256 **`841ff131324377c5d551dba7aa527fb6a34efd66cebc100836cefc5044574c4a`**, records `accepted-env-source-cli-integration-boundary`: **20 host tests passed, zero failed, three native tests ignored**, plus formatting and focused Clippy/Rustdoc with warnings denied passed. These closure/acceptance receipts supersede the earlier pending-review checkpoint, without rewriting its historical artifacts. [env-combined-closure] [env-source-acceptance] [env-closed-pins]

The replacement is `target/sdk-main-env-candidate-20260911-02/bin/suspect`, SHA-256 **`31c2fe23c760f191fdb8546cc10d2d78973935d3a728cd8875dfb66f45975261`**; its 1,161-input source manifest hashes to **`35bbf1114bee47cab2b5c72460eec878130c0a48e0ef723d3e8bb9ae8ab0d1be`**. The same six-operation/twelve-target env-configured session retains SHA **`877a532f9b2e40bdca356937f3df134d24d021b0164435b980ee46f670c6274e`**. Its **610 desired artifacts plus ownership = 611 files** are pinned by **`7d27ae8229b1d3bddcef76cab66da5f7b4ba738cf33847b514fb2e2a5ec22e27`**. The completion receipt records twelve Planned six-operation env captures, generator-canary-independent files/revisions, all 593 ordinary files exact to L, actual Go factory Breaking detection, and configured-empty refusal with ordinary `EmptySelection` retained. Eleven non-Dart package trees are byte-identical to `f971…`; changes are limited to Dart’s client and two guides, plus the root ownership manifest. It records 1,211 prior protected files preserved. These receipt pins and all 611 new package hashes were inspected read-only here. [env-fixed-completion] [env-fixed-parity] [env-closed-pins]

**Closure boundary:** this resolves ENV **source/canonical/review/host-quality** acceptance, not fresh-cohort compiled-consumer or live-service proof. The completion receipt still has `nativeConsumersExecuted: false`; the sole docs owner’s additive native-helper edition has its own preparation/delivery boundary. Full-plan, native-live and numerical acceptance remain separate. L and `f971…` observations stay at their original pins; no matched/native probes, generation or full matrices were replayed for this postscript. [env-source-acceptance] [env-fixed-completion] [env-closed-pins]

**Accepted native ENV staging — 11 September 2026:** the previously pending **native-helper consumer edition is now independently accepted by Main**. Receipt `live-env-delivery-owner-receipt-01.json` hashes to **`3ba90129fd219ac246e0d5b028fe9123d9e22e911a91985248ec57305558c3c9`**. The additive entry is **`./demo-live-env.sh all`** or **`./demo-live-env.sh <language>`**, documented in `LIVE-ENV-DEMO-README.md` (SHA-256 **`680457b1b4a4b1794e4d26d47d2bb77b72eb06554c431bfb079b1beca7fcf13b`**). Its prepared root is `target/sdk-demo-live-20260911-01/environment-01/candidate-02/`, using the corrected **`31c2…` / 611-file cohort**. The native SDK constructors/factories now perform the configured environment lookup—e.g. TS `createClient()`, Python `Client()`, Go `sdk.NewClientFromEnv()`—rather than application code passing the token into an auth constructor. Default `getCurrentKey` remains source-server **GET `/key`**, with management-key credits an opt-in mode. This closes the automatic-env consumer-preparation gap for this separately qualified edition. [env-native-acceptance] [env-native-readme] [env-native-handoff] [env-native-pins]

The accepted native evidence is **52 actual controlled outcomes across twelve targets plus JS: 44 reused by exact prepared-scope parity and eight new Go/Dart outcomes**. It contains **39 loopback GET exchanges** and **13 missing-env cases with zero HTTP**; all 52 retained stderr files are empty. Main verified 3,179 seal hashes, 4,377 old file-metadata records, 1,541 provisional entries and 1,161 source pins, plus 13 four-line code excerpts, 23 links and 14 documentation-command receipts. The recorded **12/12 preflight** is reused, not executed by this comparison update; adapter simulations remain separately labeled. F10 lineage and its excluded historical invalid-canary request disclosure are retained, and its old 16 outcomes are **not** counted again in the 52. No real-token live account request or successful account result is claimed. [env-native-acceptance] [env-native-handoff] [env-native-verification] [env-native-pins]

This accepted staging receipt supersedes only the preceding **pending new-cohort native-edition** status. The official/L/f971 matched-probe comparison remains at its original pins and observations; chat, uploads and the other API/DX differences were not re-tested or changed by this environment delivery. Full SDK, editor, 92-cost and numerical/performance acceptance remain separate. This postscript used receipt/hash/source inspection and report checks only—no SDK, preflight, simulation, live or matrix replay. [env-native-acceptance] [env-native-pins]

Each command directory preserves argv, working directory, controlled environment overrides, timestamps, exit code, executable hash and stdout/stderr. Initial failed harness attempts are retained: TS’s first chat fixture omitted required `system_fingerprint`; Python’s first error probe used the wrong `.error` accessor; Go first needed `go mod tidy`, then the probe incorrectly asserted a pointer success wrapper instead of the generated value wrapper. Those were corrected in the research consumers/fixture; they are **not SDK defects**. The TS bare unknown-enum input failed typechecking and was replaced with the public `unrecognized` helper; Python’s retained enum finding uses its supported `UnrecognizedStr`. [commands] [probe-sources]

The positive scope is deliberately concrete: installed package/type checks, controlled HTTP/codec/control behavior, inspected public source/configuration, and admission on pinned binaries. It is not a full API compatibility certification, live-provider test, security ranking, conformance percentage, or performance benchmark. L’s 39 new read/error checks are included; native automatic env implementations and later candidates still require their own fresh receipts before further changing the verdict. [install] [L-evidence] [admission-current]

## Primary and local evidence references

[pins]: ../target/sdk-current-comparison-20260911-01/primary-pins.json
[evidence-root]: ../target/sdk-current-comparison-20260911-01/
[commands]: ../target/sdk-current-comparison-20260911-01/commands/
[install]: ../target/sdk-current-comparison-20260911-01/install-result.json
[probe-sources]: ../target/sdk-current-comparison-20260911-01/
[source-analysis]: ../target/sdk-current-comparison-20260911-01/source-analysis.json
[parity]: ../target/sdk-current-comparison-20260911-01/distribution-source-parity.json
[local-verification]: ../target/sdk-current-comparison-20260911-01/local-package-verification.json
[admission-old]: ../target/sdk-current-comparison-20260911-01/admission-summary.json
[admission-current]: ../target/sdk-current-comparison-20260911-01/admission-current-summary.json
[OR-ts]: https://openrouter.ai/docs/client-sdks/typescript/overview
[OR-go]: https://openrouter.ai/docs/client-sdks/go/overview
[OR-py]: https://openrouter.ai/docs/client-sdks/python/overview
[G-release]: ../target/sdk-current-comparison-20260911-01/sources/go/release-commit.json
[G-head-diff]: ../target/sdk-current-comparison-20260911-01/sources/go/release-to-head.json
[G-download]: ../target/sdk-current-comparison-20260911-01/go-installed.json
[G-sumdb]: ../target/sdk-current-comparison-20260911-01/registry/go-sumdb.txt
[T-probe]: ../target/sdk-current-comparison-20260911-01/commands/ts-probe-run-02/stdout.txt
[P-probe]: ../target/sdk-current-comparison-20260911-01/commands/py-probe-run-02/stdout.txt
[G-probe]: ../target/sdk-current-comparison-20260911-01/commands/go-probe-run-02/stdout.txt
[T-features]: ../target/sdk-current-comparison-20260911-01/commands/ts-features-run-02/stdout.txt
[P-features]: ../target/sdk-current-comparison-20260911-01/commands/py-features-run-02/stdout.txt
[G-features]: ../target/sdk-current-comparison-20260911-01/commands/go-features-run/stdout.txt
[T-negative]: ../target/sdk-current-comparison-20260911-01/commands/ts-readme-negative/stdout.txt
[T-typecheck]: ../target/sdk-current-comparison-20260911-01/commands/ts-probe-typecheck-01/
[P-typecheck]: ../target/sdk-current-comparison-20260911-01/commands/py-callsites-typecheck/
[G-build]: ../target/sdk-current-comparison-20260911-01/commands/go-probe-build-03/
[T-callsites]: ../target/sdk-current-comparison-20260911-01/typescript/callsites.ts
[P-callsites]: ../target/sdk-current-comparison-20260911-01/python/callsites.py
[T-probe-source]: ../target/sdk-current-comparison-20260911-01/typescript/probe.ts
[G-probe-source]: ../target/sdk-current-comparison-20260911-01/go/main.go
[P-dependencies]: ../target/sdk-current-comparison-20260911-01/commands/python-installed/stdout.txt
[O-pins]: ../target/sdk-demo-readme-20260911-01/candidate-02/pins.json
[O-session]: ../target/sdk-demo-readme-20260911-01/candidate-02/session.json
[O-native]: ../target/sdk-demo-readme-20260911-01/candidate-02/native-index.json
[O-demo]: ../DEMO-README.md
[O-ts]: ../target/sdk-demo-readme-20260911-01/candidate-02/packages/typescript/README.md
[O-py]: ../target/sdk-demo-readme-20260911-01/candidate-02/packages/python/README.md
[O-go]: ../target/sdk-demo-readme-20260911-01/candidate-02/packages/go/README.md
[O-ts-export]: ../target/sdk-demo-readme-20260911-01/candidate-02/packages/typescript/source/index.ts
[O-py-export]: ../target/sdk-demo-readme-20260911-01/candidate-02/packages/python/src/openrouter_all_sdk/__init__.py
[O-ts-ops]: ../target/sdk-demo-readme-20260911-01/candidate-02/packages/typescript/operations.ts
[O-go-ops]: ../target/sdk-demo-readme-20260911-01/candidate-02/packages/go/operations.go
[O-py-client]: ../target/sdk-demo-readme-20260911-01/candidate-02/packages/python/src/openrouter_all_sdk/_client.py
[O-go-consumer]: ../examples/sdk-demo-all/go/main.go
[O-ts-types]: ../target/sdk-demo-readme-20260911-01/candidate-02/packages/typescript/http/types.ts
[O-py-types]: ../target/sdk-demo-readme-20260911-01/candidate-02/packages/python/src/openrouter_all_sdk/_types.py
[O-go-runtime]: ../target/sdk-demo-readme-20260911-01/candidate-02/packages/go/http_runtime.go
[O-py-runtime]: ../target/sdk-demo-readme-20260911-01/candidate-02/packages/python/src/openrouter_all_sdk/_runtime.py
[O-ts-package]: ../target/sdk-demo-readme-20260911-01/candidate-02/packages/typescript/package.json
[O-py-package]: ../target/sdk-demo-readme-20260911-01/candidate-02/packages/python/pyproject.toml
[O-go-package]: ../target/sdk-demo-readme-20260911-01/candidate-02/packages/go/go.mod
[M-report]: ../target/sdk-main-live-admission-20260911-01/REPORT.json
[M-session]: ../target/sdk-main-live-admission-20260911-01/session.json
[M-ts-ops]: ../target/sdk-main-live-admission-20260911-01/packages/typescript/operations.ts
[M-py-client]: ../target/sdk-main-live-admission-20260911-01/packages/python/src/openrouter/_client.py
[M-go-ops]: ../target/sdk-main-live-admission-20260911-01/packages/go/operations.go
[env-contract]: SDK-CREDENTIAL-ENV.md
[L-evidence]: ../target/sdk-current-comparison-20260911-01/current-live-evidence.json
[L-readme]: ../LIVE-DEMO-README.md
[L-session]: ../target/sdk-demo-live-20260911-01/candidate-02/session.json
[L-sources]: ../target/sdk-demo-live-20260911-01/candidate-02/source-pins.json
[L-native]: ../target/sdk-demo-live-20260911-01/candidate-02/native/
[L-verification]: ../target/sdk-demo-live-20260911-01/candidate-02/verification/
[L-ts-ops]: ../target/sdk-demo-live-20260911-01/candidate-02/packages/typescript/operations.ts
[L-py-client]: ../target/sdk-demo-live-20260911-01/candidate-02/packages/python/src/openrouter/_client.py
[L-go-ops]: ../target/sdk-demo-live-20260911-01/candidate-02/packages/go/operations.go
[L-ts-example]: ../examples/sdk-demo-live/typescript/main.ts
[L-py-example]: ../examples/sdk-demo-live/python/main.py
[L-go-example]: ../examples/sdk-demo-live/go/main.go
[L-callsites]: ../target/sdk-current-comparison-20260911-01/current-callsites-result.json
[L-go-callsites]: ../target/sdk-current-comparison-20260911-01/ours-current/go/callsites.go
[L-ts-package]: ../target/sdk-demo-live-20260911-01/candidate-02/packages/typescript/package.json
[L-py-package]: ../target/sdk-demo-live-20260911-01/candidate-02/packages/python/pyproject.toml
[L-go-package]: ../target/sdk-demo-live-20260911-01/candidate-02/packages/go/go.mod
[L-delivery]: ../target/sdk-demo-live-20260911-01/candidate-02/delivery-01/REPORT.json
[delivery-observation]: ../target/sdk-current-comparison-20260911-01/delivery-and-additional-context.json
[redaction-red]: ../target/sdk-credential-env-integration-20260911-01/live-redaction-red-01/REPORT.json
[redaction-green]: ../target/sdk-credential-env-integration-20260911-01/live-redaction-green-01/REPORT.json
[redaction-fix]: ../target/sdk-demo-live-20260911-01/candidate-02/wrapper-redaction-fix-01/fix-result.json
[redaction-observation]: ../target/sdk-current-comparison-20260911-01/redaction-addendum-20260911-01/observation.json
[redaction-delivery02]: ../target/sdk-demo-live-20260911-01/candidate-02/delivery-02/REPORT.json
[redaction-seal-check]: ../target/sdk-current-comparison-20260911-01/redaction-addendum-20260911-01/delivery02-observation.json
[env-progress-pins]: ../target/sdk-current-comparison-20260911-01/redaction-addendum-20260911-01/generator-env-pins.json
[env-canonical-six]: ../target/sdk-credential-env-integration-20260911-01/canonical-six-01.json
[env-cli-policy]: ../target/sdk-credential-env-integration-20260911-01/cli-policy-01.json
[env-python-completion]: ../target/sdk-python-credential-env-completion-20260911-01/completion.json
[env-all12-pins]: ../target/sdk-current-comparison-20260911-01/credential-env-all12-addendum-20260911-01/pins.json
[env-all12-bridge]: ../target/sdk-credential-env-integration-20260911-01/all-twelve-bridge-01/REPORT.json
[env-final-five]: ../target/sdk-credential-env-integration-20260911-01/native-final-five-owner-receipt-01.json
[env-defaults-green]: ../target/sdk-credential-env-integration-20260911-01/cli-defaults-process-01/green.json
[env-frozen-cli]: ../target/sdk-main-env-candidate-20260911-01/REPORT.json
[env-frozen-completion]: ../target/sdk-main-env-candidate-20260911-01/live-generation-01/completion-02/REPORT.json
[env-frozen-pins]: ../target/sdk-current-comparison-20260911-01/frozen-env-cohort-addendum-20260911-01/pins.json
[env-review-hold]: ../target/sdk-current-comparison-20260911-01/env-review-hold-addendum-20260911-01/status.json
[observability-owner]: ../target/sdk-credential-env-integration-20260911-01/live-observability-owner-receipt-01.json
[observability-addendum]: ../examples/sdk-demo-live/observability/README.md
[observability-pins]: ../target/sdk-current-comparison-20260911-01/delivery-observability-addendum-20260911-01/pins.json
[empty-capture-green]: ../target/sdk-credential-env-integration-20260911-01/empty-capture-01/green.json
[dart-standards-recheck]: ../target/sdk-credential-env-integration-20260911-01/review-env-01/standards-dart-recheck-01.md
[dart-review-pins]: ../target/sdk-current-comparison-20260911-01/dart-review-closure-addendum-20260911-01/pins.json
[env-combined-closure]: ../target/sdk-credential-env-integration-20260911-01/review-env-01/fixes-01/closure-and-replacement-cli-01.json
[env-source-acceptance]: ../target/sdk-main-env-candidate-20260911-02/ENV-ACCEPTANCE.json
[env-fixed-completion]: ../target/sdk-main-env-candidate-20260911-02/live-generation-01/completion-02/REPORT.json
[env-fixed-parity]: ../target/sdk-main-env-candidate-20260911-02/live-generation-01/prepared-scope-parity.json
[env-closed-pins]: ../target/sdk-current-comparison-20260911-01/env-review-closed-addendum-20260911-01/pins.json
[env-native-acceptance]: ../target/sdk-credential-env-integration-20260911-01/live-env-delivery-owner-receipt-01.json
[env-native-readme]: ../LIVE-ENV-DEMO-README.md
[env-native-handoff]: ../target/sdk-demo-live-20260911-01/environment-01/HANDOFF-01.md
[env-native-verification]: ../target/sdk-demo-live-20260911-01/environment-01/candidate-02/verification.json
[env-native-pins]: ../target/sdk-current-comparison-20260911-01/env-native-delivery-addendum-20260911-01/pins.json
[multipart-handoff]: ../target/sdk-full-runner-check-20260910-14/HANDOFF.md
[multipart-acceptance]: ../target/sdk-typescript-ignored-encoding-20260910/acceptance-01.json
[T-package]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/package.json
[T-config]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/.speakeasy/gen.yaml
[T-workflow]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/.speakeasy/workflow.yaml
[T-root]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/src/sdk/sdk.ts
[T-config-runtime]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/src/lib/config.ts
[T-keys]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/src/sdk/apikeys.ts
[T-create-model]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/src/models/operations/createkeys.ts
[T-update-model]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/src/models/operations/updatekeys.ts
[T-credit-model]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/src/models/operations/getcredits.ts
[T-error]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/src/models/errors/unauthorizedresponseerror.ts
[T-http]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/src/lib/sdks.ts
[T-env]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/src/lib/env.ts
[T-security]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/src/lib/security.ts
[T-retries]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/src/lib/retries.ts
[T-async]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/src/types/async.ts
[T-chat]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/src/sdk/chat.ts
[T-stream]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/src/lib/event-streams.ts
[T-callmodel]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/src/funcs/call-model.ts
[T-containers]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/src/sdk/containers.ts
[T-container-model]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/src/models/operations/listcontainerfiles.ts
[T-byok]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/src/funcs/byokList.ts
[T-files]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/src/sdk/files.ts
[T-upload-model]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/src/models/operations/uploadfile.ts
[T-headers]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/.speakeasy/overlays/add-headers.overlay.yaml
[open-enums]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/.speakeasy/overlays/open-enums.overlay.yaml
[T-enums]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/src/types/enums.ts
[T-unrecognized]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/src/types/unrecognized.ts
[T-hooks]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/src/hooks/registration.ts
[T-readme]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/README.md
[T-functions]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/FUNCTIONS.md
[T-runtimes]: https://github.com/OpenRouterTeam/typescript-sdk/blob/9724d06630911551912c0e6d5c892a632943e190/RUNTIMES.md
[P-package]: https://github.com/OpenRouterTeam/python-sdk/blob/2b1df428c5345be00215a8e3a3153d6e62bcf299/pyproject.toml
[P-config]: https://github.com/OpenRouterTeam/python-sdk/blob/2b1df428c5345be00215a8e3a3153d6e62bcf299/.speakeasy/gen.yaml
[P-workflow]: https://github.com/OpenRouterTeam/python-sdk/blob/2b1df428c5345be00215a8e3a3153d6e62bcf299/.speakeasy/workflow.yaml
[P-root]: https://github.com/OpenRouterTeam/python-sdk/blob/2b1df428c5345be00215a8e3a3153d6e62bcf299/src/openrouter/sdk.py
[P-version]: https://github.com/OpenRouterTeam/python-sdk/blob/2b1df428c5345be00215a8e3a3153d6e62bcf299/src/openrouter/_version.py
[P-keys]: https://github.com/OpenRouterTeam/python-sdk/blob/2b1df428c5345be00215a8e3a3153d6e62bcf299/src/openrouter/api_keys.py
[P-create-model]: https://github.com/OpenRouterTeam/python-sdk/blob/2b1df428c5345be00215a8e3a3153d6e62bcf299/src/openrouter/operations/createkeys.py
[P-update-model]: https://github.com/OpenRouterTeam/python-sdk/blob/2b1df428c5345be00215a8e3a3153d6e62bcf299/src/openrouter/operations/updatekeys.py
[P-credit-model]: https://github.com/OpenRouterTeam/python-sdk/blob/2b1df428c5345be00215a8e3a3153d6e62bcf299/src/openrouter/operations/getcredits.py
[P-error]: https://github.com/OpenRouterTeam/python-sdk/blob/2b1df428c5345be00215a8e3a3153d6e62bcf299/src/openrouter/errors/unauthorizedresponse_error.py
[P-security]: https://github.com/OpenRouterTeam/python-sdk/blob/2b1df428c5345be00215a8e3a3153d6e62bcf299/src/openrouter/utils/security.py
[P-retries]: https://github.com/OpenRouterTeam/python-sdk/blob/2b1df428c5345be00215a8e3a3153d6e62bcf299/src/openrouter/utils/retries.py
[P-chat]: https://github.com/OpenRouterTeam/python-sdk/blob/2b1df428c5345be00215a8e3a3153d6e62bcf299/src/openrouter/chat.py
[P-stream]: https://github.com/OpenRouterTeam/python-sdk/blob/2b1df428c5345be00215a8e3a3153d6e62bcf299/src/openrouter/utils/eventstreaming.py
[P-containers]: https://github.com/OpenRouterTeam/python-sdk/blob/2b1df428c5345be00215a8e3a3153d6e62bcf299/src/openrouter/containers.py
[P-byok]: https://github.com/OpenRouterTeam/python-sdk/blob/2b1df428c5345be00215a8e3a3153d6e62bcf299/src/openrouter/byok.py
[P-files]: https://github.com/OpenRouterTeam/python-sdk/blob/2b1df428c5345be00215a8e3a3153d6e62bcf299/src/openrouter/files.py
[P-upload-model]: https://github.com/OpenRouterTeam/python-sdk/blob/2b1df428c5345be00215a8e3a3153d6e62bcf299/src/openrouter/operations/uploadfile.py
[P-base-model]: https://github.com/OpenRouterTeam/python-sdk/blob/2b1df428c5345be00215a8e3a3153d6e62bcf299/src/openrouter/types/basemodel.py
[P-hooks]: https://github.com/OpenRouterTeam/python-sdk/blob/2b1df428c5345be00215a8e3a3153d6e62bcf299/src/openrouter/_hooks/registration.py
[P-readme]: https://github.com/OpenRouterTeam/python-sdk/blob/2b1df428c5345be00215a8e3a3153d6e62bcf299/README.md
[G-package]: https://github.com/OpenRouterTeam/go-sdk/blob/200f1f03bc772dcaacc8bafbdff2ab65274a0753/go.mod
[G-config]: https://github.com/OpenRouterTeam/go-sdk/blob/200f1f03bc772dcaacc8bafbdff2ab65274a0753/.speakeasy/gen.yaml
[G-workflow]: https://github.com/OpenRouterTeam/go-sdk/blob/200f1f03bc772dcaacc8bafbdff2ab65274a0753/.speakeasy/workflow.yaml
[G-root]: https://github.com/OpenRouterTeam/go-sdk/blob/200f1f03bc772dcaacc8bafbdff2ab65274a0753/openrouter.go
[G-keys]: https://github.com/OpenRouterTeam/go-sdk/blob/200f1f03bc772dcaacc8bafbdff2ab65274a0753/apikeys.go
[G-credit-model]: https://github.com/OpenRouterTeam/go-sdk/blob/200f1f03bc772dcaacc8bafbdff2ab65274a0753/models/operations/getcredits.go
[G-update-model]: https://github.com/OpenRouterTeam/go-sdk/blob/200f1f03bc772dcaacc8bafbdff2ab65274a0753/models/operations/updatekeys.go
[G-presence]: https://github.com/OpenRouterTeam/go-sdk/blob/200f1f03bc772dcaacc8bafbdff2ab65274a0753/optionalnullable/optionalnullable.go
[G-error]: https://github.com/OpenRouterTeam/go-sdk/blob/200f1f03bc772dcaacc8bafbdff2ab65274a0753/models/sdkerrors/unauthorizedresponseerror.go
[G-fallback]: https://github.com/OpenRouterTeam/go-sdk/blob/200f1f03bc772dcaacc8bafbdff2ab65274a0753/models/sdkerrors/apierror.go
[G-retries]: https://github.com/OpenRouterTeam/go-sdk/blob/200f1f03bc772dcaacc8bafbdff2ab65274a0753/internal/utils/retries.go
[G-chat]: https://github.com/OpenRouterTeam/go-sdk/blob/200f1f03bc772dcaacc8bafbdff2ab65274a0753/chat.go
[G-stream]: https://github.com/OpenRouterTeam/go-sdk/blob/200f1f03bc772dcaacc8bafbdff2ab65274a0753/types/stream/stream.go
[G-container-model]: https://github.com/OpenRouterTeam/go-sdk/blob/200f1f03bc772dcaacc8bafbdff2ab65274a0753/models/operations/listcontainerfiles.go
[G-byok]: https://github.com/OpenRouterTeam/go-sdk/blob/200f1f03bc772dcaacc8bafbdff2ab65274a0753/byok.go
[G-files]: https://github.com/OpenRouterTeam/go-sdk/blob/200f1f03bc772dcaacc8bafbdff2ab65274a0753/files.go
[G-upload-model]: https://github.com/OpenRouterTeam/go-sdk/blob/200f1f03bc772dcaacc8bafbdff2ab65274a0753/models/operations/uploadfile.go
[G-headers]: https://github.com/OpenRouterTeam/go-sdk/blob/200f1f03bc772dcaacc8bafbdff2ab65274a0753/.speakeasy/overlays/add-headers.overlay.yaml
[G-hooks]: https://github.com/OpenRouterTeam/go-sdk/blob/200f1f03bc772dcaacc8bafbdff2ab65274a0753/internal/hooks/registration.go
[G-readme]: https://github.com/OpenRouterTeam/go-sdk/blob/200f1f03bc772dcaacc8bafbdff2ab65274a0753/README.md
[Rust-example]: ../examples/sdk-demo-all/rust/main.rs
[Swift-example]: ../examples/sdk-demo-all/swift/Main.swift
[Java-example]: ../examples/sdk-demo-all/java/Main.java
[Csharp-example]: ../examples/sdk-demo-all/csharp/Program.cs
[Kotlin-example]: ../examples/sdk-demo-all/kotlin/Smoke.kt
[Ruby-example]: ../examples/sdk-demo-all/ruby/main.rb
[PHP-example]: ../examples/sdk-demo-all/php/main.php
[Dart-example]: ../examples/sdk-demo-all/dart/main.dart
[Cpp-example]: ../examples/sdk-demo-all/cpp/main.cpp
