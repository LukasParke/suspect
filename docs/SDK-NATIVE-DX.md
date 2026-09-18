# Native TypeScript/JavaScript and Rust SDK interfaces

Broad interface proposal, 2026-09-08. The user approved the implemented
selected-operation M0–M2 baseline on 2026-09-09; see
[the approval record](SDK-M0-M2-DX-APPROVAL.md) and
[native slice call sites](SDK-M2-CALLSITES.md). This broader proposal also covers
future media/streaming/resource APIs. Those snippets retain their own native
verification requirements and are not evidence of those features being shipped.

The API facts come from the tracked OpenRouter public description at commit
`db378a2a90d0167b9dca4f98b52074c54d249e1f`,
`projects/docs/openapi/openapi.yaml`, SHA-256
`bd4953b29f34de134ed4be27b2803c5622a756c3c63bc8617636b6abe5de1821`.
It contains 103 operations and 786 component schemas. Ignored local generated
specifications have different contents and are not the source for this note.
See [the acceptance corpus](OPENROUTER-ACCEPTANCE.md),
[semantic requirements](SDK-OPENAPI-RESEARCH.md), and
[the implementation plan](SDK-GENERATION-PLAN.md).

## Interface decisions

Use a small transport/codec module behind native operation functions and
resource methods. Both forms use the same operation implementation. Canonical
exports derive from `operationId`; optional resource aliases derive from tags
and explicitly supported naming extensions. Docs show both names and link to
the same OpenAPI operation. A naming profile must not silently activate an
unrelated vendor extension such as `x-speakeasy-ignore`.

| Concern | TypeScript / JavaScript proposal | Rust proposal |
| --- | --- | --- |
| Ordinary calls | `Promise<ApiResponse<T>>`; documented HTTP errors reject with a typed SDK error that an operation-specific predicate narrows | `Result<ApiResponse<T>, OperationError>`; documented status/body variants are exhaustively matchable |
| Inputs | One object grouped into `path`, `query`, `headers`, `body`; call options are a second argument | A generated operation-input type, constructors for required fields, and builder methods for optional fields; options are separate |
| Response metadata | `status`, normalized selected `contentType`, native `Headers`, and decoded `data`; expose declared response headers with their generated types | Status, headers, selected representation, and decoded body; generated types for declared headers |
| Models | Plain objects and discriminated unions, retaining JSON wire property names | Structs and enums with explicit wire mappings; constructors hide constant tag fields |
| Failure categories | Documented API response, unexpected response, response decoding, request validation, transport, cancellation | The same categories, expressed through native error enums |
| Streaming | Native async iteration; abort signal and iterator cleanup | `Stream<Item = Result<Event, StreamError>>`; dropping the future/stream cancels work |
| Transport seam | Fetch-compatible injection, preserving signals and response bodies | Generic transport trait; optional concrete HTTP adapters |

TypeScript has no checked promise-rejection type. Do not advertise `catch`
variables as statically typed without a guard, or describe `Promise<T>` as
encoding all failures. An opt-in generic result adapter can be considered later;
it is not needed to duplicate every operation into throwing/nonthrowing methods.

For operations with several successful representations, preserve every declared
status/media alternative in the canonical operation. A representation selector
can request and require one media type for a simpler call site. The JSON examples
below use `{ response: "application/json" }`; the runtime sets `Accept` and
rejects a different representation as an unexpected response without pretending
its body has the selected type. Omitting the selector retains the response
union. Selection is a caller runtime policy, not a rewritten OpenAPI contract.

Do not buffer an unbounded body to expose convenient raw bytes. An unexpected
response retains status, headers and a bounded raw-body capture with an explicit
truncation indicator. Request validation errors must occur before sending.
SDK-owned failures carry stable operation/source identities; exceptions from
user-provided hooks retain their causes and are not mislabeled as API errors.

## Source contracts exercised by these examples

| Operation ID and route | Required request | Declared success | Important distinction |
| --- | --- | --- | --- |
| `sendChatCompletionRequest`, POST `/chat/completions` | JSON body; `ChatRequest.messages` | 200 JSON `ChatResult` or SSE `ChatStreamingResponse` | `model` is optional; `system_fingerprint` is required and nullable |
| `createMessages`, POST `/messages` | JSON body; `model` and `messages` | 200 JSON `MessagesResult` or SSE `MessagesStreamingResponse` | `messages` is required but nullable; `max_tokens` is optional; errors use the Anthropic envelope |
| `createResponses`, POST `/responses` | JSON body; no top-level required properties in `ResponsesRequest` | 200 JSON `OpenResponsesResult` or SSE `ResponsesStreamingResponse` | Streaming event union has 49 declared alternatives |
| `uploadFile`, POST `/files` | Multipart body with `file` | 200 JSON `FileResponse` | No request `purpose` property; success is 200; `_shape` is a real wire discriminator |
| `createAudioTranscriptions`, POST `/audio/transcriptions` | JSON `STTRequest`, or multipart with `file` and `model` | 200 JSON `STTResponse` | Multipart field is literally `timestamp_granularities[]` |
| `updateWorkspace`, PATCH `/workspaces/{id}` | Path `id` and JSON body | 200 JSON `UpdateWorkspaceResponse` | Empty body `{}` is valid; description may be absent, null, or a string |

All six inherit root security `[{ apiKey: [] }]`. The security scheme named
`apiKey` has `type: http`, `scheme: bearer`; credentials therefore produce an
`Authorization: Bearer …` header. It is not an API-key header scheme inferred
from the name. The root server is `https://openrouter.ai/api/v1`.
The workspace operation additionally says a management key is required in its
description. Preserve that guidance in docs; the source does not provide a
separate machine-checkable credential type that could prove a key is suitable.

Sources: [chat][chat-operation], [Messages][messages-operation],
[Responses][responses-operation], [files][files-operation],
[transcriptions][transcriptions-operation], [workspace][workspace-operation],
[security schemes][security-schemes], [root security/server][root-security].

## TypeScript and JavaScript

### Everyday calls and typed failures

The proposed full client uses the source's `chat` / `send` naming metadata.
`OpenRouter`, package identity, auth construction and the environment-variable
name are SDK/application choices. The SDK does not discover credentials from
the environment implicitly.

```ts
import { OpenRouter } from "@example/openrouter";
import {
  isSendChatCompletionRequestError,
} from "@example/openrouter/operations/sendChatCompletionRequest";

const apiKey = process.env.OPENROUTER_API_KEY;
if (!apiKey) throw new Error("Set OPENROUTER_API_KEY");
const client = new OpenRouter({ auth: { apiKey } });

try {
  const response = await client.chat.send(
    {
      headers: { "X-OpenRouter-Metadata": "enabled" },
      body: {
        model: "openai/gpt-4",
        messages: [
          { role: "system", content: "You are a helpful assistant." },
          { role: "user", content: "What is the capital of France?" },
        ],
      },
    },
    { response: "application/json", signal: AbortSignal.timeout(30_000) },
  );

  // ChatResult.choices has no minItems; content can also be absent/null/parts.
  const content = response.data.choices[0]?.message.content;
  if (typeof content === "string") console.log(content);
  // Required nullable: the key exists even when the value is null.
  console.log(response.data.system_fingerprint);
} catch (error: unknown) {
  if (isSendChatCompletionRequestError(error)) {
    const response = error.response;
    if (response.status === 429) {
      // response.data is TooManyRequestsResponse, decoded and validated.
      console.error(response.data.error.message);
    } else {
      throw error;
    }
  } else {
    // Transport, cancellation, invalid responses, or non-SDK exceptions.
    throw error;
  }
}
```

`isSendChatCompletionRequestError` must check SDK-owned error identity and the
operation identity, not just a remotely supplied `name` property. Only validated
documented error bodies qualify. Error bodies that do not match their declared
schema produce a decoding failure with the original status and raw capture.

The chat operation declares errors 400, 401, 402, 403, 404, 408, 413, 422,
429, 500, 502, 503, 524, and 529. Generate the complete status/body union.
Do not turn all of them into one unstructured message, and do not invent a
`default` response for this operation. An undeclared response remains an
explicit unexpected-response case.

JavaScript gets the same runtime interface and a separately tested quickstart:

```js
import { OpenRouter } from "@example/openrouter";

const apiKey = process.env.OPENROUTER_API_KEY;
if (!apiKey) throw new Error("Set OPENROUTER_API_KEY");
const client = new OpenRouter({ auth: { apiKey } });
const { data } = await client.chat.send(
  { body: { messages: [{ role: "user", content: "Hello" }] } },
  { response: "application/json" },
);
console.log(data.id);
```

Here the omitted `model` is allowed by [ChatRequest][chat-request]. Runtime
request validation still matters for JavaScript, `unknown` input, and constraints
such as the nonempty messages array. Type declarations alone do not provide it.

### Presence, unions and precise values

These are projections illustrating the public types, not complete generated
declarations. Actual models must also preserve every remaining property and
permitted additional property.

```ts
type WorkspaceDescription = { description?: string | null };
type RequiredFingerprint = { system_fingerprint: string | null };

// A real tagged union: the tool branch requires tool_call_id.
type ChatMessages =
  | ChatSystemMessage
  | ChatUserMessage
  | ChatDeveloperMessage
  | ChatAssistantMessage
  | ChatToolMessage;
```

```ts
await client.workspaces.update({ path: { id: "production" }, body: {} });
await client.workspaces.update({
  path: { id: "production" }, body: { description: null },
});
await client.workspaces.update({
  path: { id: "production" }, body: { description: "Updated description" },
});
```

These calls encode omission, explicit null and a value. The schema establishes
that all three are allowed; it does not fully specify the server-side business
effect of omission or clearing. Do not invent that behavior in generated prose.
With `exactOptionalPropertyTypes`, `description: undefined` is rejected as an
explicit assignment. JavaScript runtime handling must independently distinguish
absence from null; encode absent/undefined optional properties as omission,
reject absent/undefined required properties, and never erase explicit null.

`ChatMessages` discriminates on `role` using the five explicit mappings in the
source. In particular, assistant content is optional and nullable, user content
is required and may be text or content parts, and tool messages require
`tool_call_id`. Plain JSON-compatible objects should be sufficient; callers
should not need `as ChatMessages` to construct ordinary valid branches.

Do not make all JSON numbers IEEE-754 `number` by default. The contract often
leaves integer ranges unspecified, including many counters and indices. The
strict proposal uses native `bigint` for unbounded integers and a lossless
`JsonNumber` representation for general decimals; proven safe bounded mappings
can use `number`. Request conveniences can accept finite native numbers through
an explicitly documented conversion, while exact decimal construction accepts
the decimal token. Response decoding must preserve the exact numeric value
before any user-requested conversion. `format: double` alone is not a 3.1
numeric bound. Never route an unbounded response through ordinary `JSON.parse`
and claim precision can be restored afterward. A schema-driven numeric codec
must also accept mathematically integral spellings such as `1.0` and `1e3`.
These choices need concrete numeric consumer tests before the public types are
frozen; example values and likely server behavior cannot justify narrowing.

### Anthropic Messages keeps its own types

```ts
const response = await client.anthropicMessages.create(
  {
    body: {
      model: "anthropic/claude-sonnet-4",
      messages: [{
        role: "user",
        content: [
          { type: "text", text: "Describe this image." },
          {
            type: "image",
            source: { type: "url", url: "https://example.com/image.jpg" },
          },
        ],
      }],
    },
  },
  { response: "application/json" },
);

for (const block of response.data.content) {
  switch (block.type) {
    case "text":
      console.log(block.text);
      break;
    case "tool_use":
      console.log(block.name);
      break;
  }
}
```

The image URL is an illustrative schema example, not a tested downloadable
fixture. Executable docs must supply a checked local fixture through the mock
server or another controlled input.

The [image source][image-block] has nested `base64`/`url` discrimination. The
[response content union][anthropic-content] has 16 branches with explicit `type`
mapping. Preserve `allOf` constraints in `MessagesResult` and its base response;
do not overwrite or flatten constraints merely because fields have matching
names. The request permits roles `user`, `assistant`, and `system`, and requires
`messages` while permitting null. Do not import restrictions from another
provider's similarly named API. `max_tokens` is optional here. A request
`tool_use` content block's `input` is also optional in this source.

`createMessages` errors 400, 401, 403, 404, 429, 500, 503, and 529 all use
[AnthropicMessagesErrorResponse][anthropic-error]: required `type: "error"`,
`error`, and nullable `request_id`. Its `error.type` is a typed enum. The
generated `isCreateMessagesError` guard must expose that envelope, including
`request_id`, rather than cast it to the chat error schema.

### Multipart and representation selection

```ts
const pdf = new File([pdfBytes], "document.pdf", { type: "application/pdf" });
const uploaded = await client.files.upload({ body: { file: pdf } });

switch (uploaded.data._shape) {
  case "openrouter":
    console.log(uploaded.data.size_bytes);
    break;
  case "openai":
    console.log(uploaded.data.bytes);
    break;
  case "anthropic":
    console.log(uploaded.data.mime_type);
    break;
}

await client.stt.createTranscription({
  body: {
    contentType: "multipart/form-data",
    value: {
      file: new File([audioBytes], "audio.wav", { type: "audio/wav" }),
      model: "openai/whisper-large-v3",
      response_format: "verbose_json",
      "timestamp_granularities[]": ["word", "segment"],
    },
  },
});
```

`pdfBytes` and `audioBytes` denote supplied binary fixtures. A multipart filename
example is not the binary payload. The runtime owns multipart framing and the
boundary parameter; never set a bare `Content-Type: multipart/form-data` while
delegating the body to platform `FormData`. Native `File`/`Blob` are the base
browser/Node inputs; filesystem streams can be an optional Node adapter with a
documented replay and cancellation policy.

For multiple request media, the `contentType`/`value` pair is an SDK selector,
not a nested JSON object sent to the server. JSON transcription instead selects
`application/json` and uses the source's `model`/`input_audio` properties. Its
JSON property `timestamp_granularities` differs from the multipart name above.
Honor default and explicit multipart `encoding` rules; do not invent a field
encoding because the name contains brackets.

The file source has an optional `provider` query and `workspace_id` query, but
no multipart `purpose` field. The [FileResponse][file-response] schema's `_shape`
must be preserved in both codecs and public discriminated types. Its schema
example includes that field; route success examples omit it. The docs gate must
report the invalid route examples, not silently repair them or claim that the
discriminator is synthetic. File size/empty-file restrictions in route prose
belong in docs; they are not machine-checkable schema bounds in this input.

## Rust

Keep the Rust interface native: owned request models, borrowed reusable client,
`Result`, status/body enums, and an ordinary async transport. Required constant
tags are emitted by codecs from enum variants. Do not require callers to type
`role: "user"` next to `ChatMessages::User`, or permit contradictory tags.
Constructors establish required fields; schema constraints not expressible in
Rust's type system are checked by request/response codecs.

Proposed consumer setup, with exact published versions filled in at release:

```toml
[dependencies]
openrouter-sdk = { version = "<release>", default-features = false, features = ["reqwest-rustls", "stream"] }
tokio = { version = "1", features = ["macros", "rt-multi-thread", "time"] }
futures-util = "0.3"
```

```rust
use openrouter_sdk::{ApiResponse, Client, Credentials};
use openrouter_sdk::models::{ChatMessages, ChatRequest, ChatUserMessage};
use openrouter_sdk::operations::send_chat_completion_request::{
    SendChatCompletionRequest, SendChatCompletionRequestError,
    SendChatCompletionRequestApiError,
};

async fn chat() -> Result<(), Box<dyn std::error::Error>> {
    let credentials = Credentials::api_key(std::env::var("OPENROUTER_API_KEY")?);
    let client = Client::with_reqwest(credentials)?;
    let body = ChatRequest::new(vec![
        ChatMessages::User(ChatUserMessage::new("What is the capital of France?")),
    ]).with_model("openai/gpt-4");

    match client.chat().send_json(SendChatCompletionRequest::new(body)).await {
        Ok(ApiResponse { data, .. }) => println!("{}", data.id),
        Err(SendChatCompletionRequestError::Api(
            SendChatCompletionRequestApiError::Status429(response),
        )) => eprintln!("{}", response.data.error.message),
        Err(error) => return Err(error.into()),
    }
    Ok(())
}
```

`send_json` is the media-selecting convenience; the canonical
`send_chat_completion_request` retains the complete successful response enum.
No-content responses use a unit/no-body variant, not a fabricated JSON object.
`ApiError::Status429` carries typed status-specific data and HTTP metadata.
Variants also cover request validation, decoding, unexpected responses,
transport and cancellation. Do not use a generic `serde_json::Value` for every
documented API error.

### Absence and null survive serialization

| Schema states | TypeScript | Rust proposal |
| --- | --- | --- |
| Required non-null | `field: T` | `T` |
| Required nullable | `field: T \| null` | `Nullable<T>` with `Null` / `Value(T)`; missing is a decoding error |
| Optional non-null | `field?: T` | `Option<T>` with a generated decoder that rejects explicit null |
| Optional nullable | `field?: T \| null` | `Presence<T>` with `Absent` / `Null` / `Value(T)` |

Ordinary derived Serde `Option<T>` often accepts both missing and null. A custom
codec must enforce the optional-non-null row. `Option<Option<T>>` with default
Serde behavior does not automatically preserve the three optional-nullable
states. `Nullable<T>` must not acquire a missing-field default.

```rust
use openrouter_sdk::{Nullable, Presence};
use openrouter_sdk::models::UpdateWorkspaceRequest;
use openrouter_sdk::operations::update_workspace::UpdateWorkspace;

let unchanged_field = UpdateWorkspaceRequest::default();
let explicit_null = UpdateWorkspaceRequest::default()
    .with_description(Presence::Null);
let explicit_value = UpdateWorkspaceRequest::default()
    .with_description(Presence::Value("Updated description".into()));

client.workspaces().update(UpdateWorkspace::new("production", unchanged_field)).await?;
client.workspaces().update(UpdateWorkspace::new("production", explicit_null)).await?;
client.workspaces().update(UpdateWorkspace::new("production", explicit_value)).await?;

// A ChatResult decoder must have required this property to exist.
match chat_result.system_fingerprint {
    Nullable::Null => {}
    Nullable::Value(fingerprint) => println!("{fingerprint}"),
}
```

`Default` is appropriate for this workspace request because it has no required
properties; it must not make arbitrary models with required properties valid.
Preserve permitted extra fields through a generated, collision-safe extra-field
store and codec. Typed/closed/pattern-constrained objects need their actual
schema rules; a universal `flatten` map is insufficient. A field literally
named `type` remains wire `type`, even when a Rust member becomes `r#type`.

Use bounded native integers only where the schema proves the range. For
unbounded integers, propose a small `JsonInteger` value backed by exact numeric
tokens with checked native conversions; general numbers use `JsonNumber`.
Serde JSON arbitrary precision can supply storage, but does not by itself prove
integer constraints or every composition rule. Constructors from ordinary
integers should be easy; conversion to a narrower type must be explicit and
fallible. Keep an optional big-number arithmetic dependency out of callers that
only need faithful transport and storage.

Exercise this exact dependency-feature configuration in native union tests.
Enabling `serde_json/arbitrary_precision` elsewhere in a consumer's dependency
graph can affect derived internally tagged or untagged enum buffering: a number
may travel through Serde's private map representation and fail a later `f64`
decode. The workspace journal exposed this failure during implementation.
Do not assume `#[serde(tag = "type")]` alone is a sufficient generated union
codec. Preserve numeric tokens through explicit validated dispatch, and test
both ordinary and arbitrary-precision consumer feature combinations.

The Anthropic model hierarchy follows the same source branches as TypeScript:
`MessagesMessageParam` retains its role enum; content distinguishes text and
parts; image sources become `Base64` and `Url` enum variants; response content
has all 16 declared variants. For example, the proposed constructor expression
`AnthropicImageSource::Url(AnthropicUrlImageSource::new(url))` writes
`{ "type": "url", "url": ... }`. An unknown enum/tag may be available through
an explicit raw compatibility path, but cannot silently become a declared valid
variant in the strict model.

For multipart, use a typed `UploadFile` input containing an `Upload` adapter
such as `Upload::bytes(pdf_bytes).filename("document.pdf").content_type("application/pdf")`.
This transport value does not change the OpenAPI binary field into a base64
JSON string. Preserve file ownership, stream errors, boundary generation and
non-replayable-body behavior. `FileResponse::Openrouter`, `::Openai` and
`::Anthropic` preserve the `_shape` discriminator and branch-specific fields.

## Typed streaming requires an explicit interpretation

The tracked input is OpenAPI 3.1. It declares SSE media schemas and existing
`x-speakeasy-stream-request-field: stream` /
`x-speakeasy-sse-sentinel: '[DONE]'` metadata for chat, Messages and Responses.
It does not have OpenAPI 3.2's streamed `itemSchema`. Before release, define and
verify a versioned compatibility profile for those existing declarations, or
add equivalent explicit stream metadata to the OpenAPI source. The profile
must specify how SSE frame fields and parsed JSON data map into the declared
wrapper, what sets the request flag, and how the sentinel terminates the stream.
Do not infer these transformations merely from a field named `data`.

Until that interpretation is verified, standard-only mode can expose the media
as a raw byte stream with a capability diagnostic; it cannot claim typed event
support. If a release configuration requires typed streaming, this diagnostic
blocks promotion. No upstream spec edits are part of this design document.

The following call sites are conditional design targets for that profile:

```ts
const abort = new AbortController();
const response = await client.responses.stream(
  { body: { model: "openai/gpt-4o", input: "Tell me a joke" } },
  { signal: abort.signal },
);

try {
  for await (const envelope of response.data) {
    if (envelope.data.type === "response.output_text.delta") {
      console.log(envelope.data.delta);
    }
  }
} finally {
  abort.abort();
}
```

The helper sets the declared `stream` request field and selects SSE through the
verified profile. `response.data` is an async stream of
`ResponsesStreamingResponse`; each item's `.data` is the 49-way `StreamEvents`
union. Preserve the wrapper rather than silently projecting out `.data`.
The `response.output_text.delta` branch is `TextDeltaEvent`, which intersects
`BaseTextDeltaEvent` with another `logprobs` constraint. Both constraints apply.

Messages retains its additional required `event` wrapper property:

```ts
for await (const envelope of messagesStream.data) {
  const event = envelope.data;
  if (event.type === "content_block_delta" && event.delta.type === "text_delta") {
    console.log(envelope.event, event.delta.text);
  }
}
```

`messagesStream` denotes the response from the proposed
`client.anthropicMessages.stream` with a valid `MessagesRequest` body. The
wrapper's `event` is a string, not an enum in this schema; do not invent a static
equality constraint between it and `data.type`. `MessagesStreamEvents` is an
eight-way union. Its `error` event is a declared application event, distinct
from initial HTTP errors and midstream transport/decoding failures.

```rust
use futures_util::StreamExt;
use openrouter_sdk::models::{ResponsesRequest, StreamEvents};
use openrouter_sdk::operations::create_responses::CreateResponses;

let request = CreateResponses::new(
    ResponsesRequest::default()
        .with_model("openai/gpt-4o")
        .with_input("Tell me a joke"),
);
let response = client.responses().stream(request).await?;
let mut events = response.data;
while let Some(item) = events.next().await {
    let envelope = item?;
    if let StreamEvents::ResponseOutputTextDelta(delta) = envelope.data {
        println!("{}", delta.delta);
    }
}
```

The proposed stream wrapper is `Unpin` so ordinary `.next().await` works;
otherwise docs must show the required native pinning. Do not leave this as an
unmentioned implementation detail. Source-derived variant allocation must be
stable and documented, including collisions across names normalized from tags.

Chat similarly yields `ChatStreamingResponse { data: ChatStreamChunk }`.
`choices` may be empty; `delta.content` may be absent or null. None of these
streams is guaranteed to consist only of text. Six SSE media declarations in
the public document do not establish six streaming interfaces: embeddings and
rerank contain explicitly unsupported placeholder string SSE descriptions,
while image generation lacks the request-field metadata used above.

The shared stream runtime must parse frames across arbitrary byte/UTF-8
boundaries, multiline data and comments, apply the sentinel at the configured
framing stage, preserve declared envelopes, bound buffering, and close readers
on abort, iteration break and error. No automatic reconnection or replay of
generation POST requests is implied by these declarations.

## Cancellation, transport and dependency profiles

TypeScript uses a native `AbortSignal` for the entire exchange, including body
decoding and iteration. A custom fetch receives the generated request and the
same signal. The seam supports platform fetch, instrumentation and a recording
adapter without teaching every operation a new transport interface:

```ts
const instrumentedFetch: typeof globalThis.fetch = async (input, init) => {
  const started = performance.now();
  try {
    return await globalThis.fetch(input, init);
  } finally {
    recordDuration(performance.now() - started);
  }
};
const client = new OpenRouter({ auth: { apiKey }, fetch: instrumentedFetch });
```

`recordDuration` is an application hook. This example measures fetch completion,
not the time to consume a streaming body. The SDK must not make credential
logging part of the default transport. Origin changes/redirects require explicit
credential-forwarding rules, exercised by actual transport tests. Retrying
non-idempotent requests is opt-in policy; an error code alone does not prove
that a request or uploaded body can be replayed safely.

Rust's custom constructor is proposed as
`Client::with_transport(transport, credentials)`. The public transport trait
accepts a prepared request, returns status/headers plus a streaming body, and
does not depend on reqwest types. Use generics for native static dispatch;
offer a boxed adapter only if dynamic transport substitution proves useful.
The reqwest adapter accepts an existing client as well as an ergonomic default
constructor. It must preserve caller connection-pool/TLS configuration.

For reqwest/Tokio consumers, `tokio::time::timeout(duration, operation).await`
can cancel by dropping the operation future. Other callers can use their own
future-selection mechanism. The implementation must drop the underlying
exchange/body too; claiming cancellation while a detached request continues is
insufficient. Adding a mandatory cancellation-token library is unnecessary for
this initial interface.

| Profile | Recommended initial floor / dependencies | Required evidence before support is claimed |
| --- | --- | --- |
| TS/JS | TypeScript 5.5 declaration-consumer floor; ESM, ES2022; Node 22 minimum and Node 24 recommended; platform Fetch/Headers/FormData/File/AbortSignal | Compile at the floor and the selected current compiler, execute on supported Node lines, verify exports/declarations and browser import behavior |
| Browser JS | Evergreen browsers with the enumerated Web APIs; publish an exact tested matrix at release | Browser runtime, binary upload, streaming, abort and bundle tests; no Node built-ins in browser entrypoints |
| Worker JS | Same platform-oriented core; each named runtime is an explicit support claim | Dedicated runtime tests before claiming Cloudflare Workers, Deno, Bun or other environments |
| Rust models/codecs | Rust 1.88 / edition 2024; dependency-free exact runtimes and optional `serde-json` adapters | Minimum/stable compiler checks, codecs and rustdoc examples; verify transitive MSRVs in the locked release |
| Rust HTTP | `reqwest-rustls` as the documented recommended opt-in; alternative `reqwest-native-tls` or custom transport | Dependency features inspected; no unintended duplicate TLS stack; real request/response tests |
| Rust streaming | Opt-in `stream` support; minimal stream traits and the chosen transport adapter | Split-frame tests, bounded memory, cancellation and stream-error behavior |

This table describes the broader interface proposal. Verified subsets and
floor/current toolchains are recorded in [SDK-CAPABILITIES.md](SDK-CAPABILITIES.md)
and the native profile docs. Any additional protocol/runtime profile requires
native evidence. Doc tooling is a build-time dependency, not a consumer runtime
dependency.

The [Node release schedule][node-schedule], read 2026-09-08, lists Node 22 end
of support as 2027-04-30 and Node 24 as 2028-04-30. Revisit the minimum before
release rather than promise an already-expiring line indefinitely.
[TypeScript 5.5][typescript-floor] and [Rust 1.88][rust-floor] are documented
released toolchains. These citations establish their existence; SDK compatibility
comes from native consumer evidence.

### Imports and size acceptance

The full `OpenRouter` client is an ergonomic entrypoint; a class that references
every operation and codec can retain the whole SDK. Do not promise that ESM or
`sideEffects: false` automatically makes a one-method use small.

```ts
import { createClient } from "@example/openrouter/runtime";
import {
  sendChatCompletionRequest,
} from "@example/openrouter/operations/sendChatCompletionRequest";

const client = createClient({ auth: { apiKey } });
const response = await sendChatCompletionRequest(
  client,
  { body: { messages: [{ role: "user", content: "Hello" }] } },
  { response: "application/json" },
);
```

Also offer a resource import such as `@example/openrouter/resources/chat`,
between a single operation and the full client. Export maps must expose stable
supported paths. Type-only model imports must erase cleanly; runtime codec
imports retain exactly the reachable schema graph, including recursive groups.
Sharing codecs is preferable to repeating them in each operation, provided
sharing does not reintroduce a global registry that retains every schema.

Measure three identical consumer behaviors using full-client, resource and
operation imports. Record raw/minified/gzip/Brotli JS, declaration size,
reachable dependencies, parse/startup time and representative request/stream
cost. Inspect bundler metadata for unrelated resources/codecs and Node shims.
Use at least the selected production bundler and an independent bundler before
claiming general tree-shaking behavior. Establish reviewed numerical budgets
from the first correct output; invented byte targets are not evidence.

For Rust, prefer no default HTTP/TLS/runtime feature and document the explicit
ergonomic reqwest profile shown above. Compare models/codecs only, one JSON
operation with custom transport, reqwest JSON, and reqwest streaming. Record
dependency trees/features, clean/incremental compile time, release executable
size and runtime allocation/latency. Linker elimination can reduce executable
size while full generated modules still cost compile time. Add resource feature
groups only if measurements justify their interface and feature-unification
complexity. Do not add one Cargo feature per schema by default.

## Docs and native acceptance are one deliverable

Generate native symbol documentation and language guides from the same language
plan as the code. TS and JS each get install/auth/first-call/error/upload/stream
examples; Rust gets those examples in Rustdoc and standalone consumer projects.
The reference includes every exported operation/model, exact wire names,
requiredness/nullability, representation variants, documented errors and source
links. Stable operation IDs remain searchable when resource aliases differ.

Validate examples against the resolved source contract before turning them into
snippets. Separate literal source examples from generated minimal examples and
clearly attributed application scaffolding such as credentials, binary fixtures,
timeouts and telemetry. An invalid source example yields a source-linked finding;
it must not be silently changed into evidence that the original is valid.

Acceptance cases for the proposed protocol expansion:

1. Generate/install both packages from the pinned tracked OpenRouter input;
   compile actual consumer projects without manual emitted-code fixes. Run
   TypeScript with `strict`, `exactOptionalPropertyTypes`,
   `noUncheckedIndexedAccess`, and declaration checking enabled; run a separate
   JavaScript consumer. Build Rust on the declared minimum and selected stable
   toolchains with each claimed dependency profile.
2. Check negative consumer cases: missing required `messages`, missing tool
   `tool_call_id`, mismatched image-source fields, invalid enum tags, omitted
   required nullable fields, and request-media mismatches. Use runtime tests
   for constraints the language cannot prove, including nonempty arrays,
   numeric range/integrality, object openness and composition.
3. Verify actual HTTP requests for inherited bearer security, path escaping,
   exact query/header/wire names, omitted/null/value bodies, both transcription
   media and binary multipart contents. Exercise each documented response
   family plus malformed, wrong-media and undeclared responses against an
   independent recording server, not only an SDK-generated mock.
4. Test chat, Anthropic and Responses union narrowing and codec round trips,
   preserving all declared branches and extra fields. A discriminator does not
   excuse skipping the branch constraints or `oneOf` exclusivity checks.
   Rust union tests must include numeric payloads with Serde arbitrary precision
   enabled through consumer dependency-feature unification.
5. Gate typed streaming on the verified source/profile semantics above; test
   arbitrary frame splits, UTF-8, nested event unions, sentinels, errors,
   backpressure and cancellation. Do not count unrelated placeholder SSE
   declarations as supported streams.
6. Build TypeDoc from real exports and Rustdoc with broken links rejected;
   compile every code example and execute mock-backed examples. Check docs
   coverage of operation IDs, models, errors and representation variants.
   Produce navigable references and packaged native comments with each SDK.
7. Compare the import/feature profiles above and report measured generation,
   package, runtime and documentation costs. A fast incomplete SDK is not the
   baseline from which a production fidelity regression can be justified.

The implemented baseline is documented in [SDK-M2-CALLSITES.md](SDK-M2-CALLSITES.md).
The broader interfaces above require their protocol-specific OpenRouter and
independent conformance gates before support is added.

[chat-operation]: https://github.com/OpenRouterTeam/openrouter-web/blob/db378a2a90d0167b9dca4f98b52074c54d249e1f/projects/docs/openapi/openapi.yaml#L29429
[messages-operation]: https://github.com/OpenRouterTeam/openrouter-web/blob/db378a2a90d0167b9dca4f98b52074c54d249e1f/projects/docs/openapi/openapi.yaml#L35357
[responses-operation]: https://github.com/OpenRouterTeam/openrouter-web/blob/db378a2a90d0167b9dca4f98b52074c54d249e1f/projects/docs/openapi/openapi.yaml#L38692
[files-operation]: https://github.com/OpenRouterTeam/openrouter-web/blob/db378a2a90d0167b9dca4f98b52074c54d249e1f/projects/docs/openapi/openapi.yaml#L31373
[transcriptions-operation]: https://github.com/OpenRouterTeam/openrouter-web/blob/db378a2a90d0167b9dca4f98b52074c54d249e1f/projects/docs/openapi/openapi.yaml#L28270
[workspace-operation]: https://github.com/OpenRouterTeam/openrouter-web/blob/db378a2a90d0167b9dca4f98b52074c54d249e1f/projects/docs/openapi/openapi.yaml#L39887
[security-schemes]: https://github.com/OpenRouterTeam/openrouter-web/blob/db378a2a90d0167b9dca4f98b52074c54d249e1f/projects/docs/openapi/openapi.yaml#L27524
[root-security]: https://github.com/OpenRouterTeam/openrouter-web/blob/db378a2a90d0167b9dca4f98b52074c54d249e1f/projects/docs/openapi/openapi.yaml#L40745
[chat-request]: https://github.com/OpenRouterTeam/openrouter-web/blob/db378a2a90d0167b9dca4f98b52074c54d249e1f/projects/docs/openapi/openapi.yaml#L5333
[image-block]: https://github.com/OpenRouterTeam/openrouter-web/blob/db378a2a90d0167b9dca4f98b52074c54d249e1f/projects/docs/openapi/openapi.yaml#L1496
[anthropic-content]: https://github.com/OpenRouterTeam/openrouter-web/blob/db378a2a90d0167b9dca4f98b52074c54d249e1f/projects/docs/openapi/openapi.yaml#L18418
[anthropic-error]: https://github.com/OpenRouterTeam/openrouter-web/blob/db378a2a90d0167b9dca4f98b52074c54d249e1f/projects/docs/openapi/openapi.yaml#L1607
[file-response]: https://github.com/OpenRouterTeam/openrouter-web/blob/db378a2a90d0167b9dca4f98b52074c54d249e1f/projects/docs/openapi/openapi.yaml#L8476
[node-schedule]: https://github.com/nodejs/Release/blob/main/schedule.json
[typescript-floor]: https://www.typescriptlang.org/docs/handbook/release-notes/typescript-5-5.html
[rust-floor]: https://blog.rust-lang.org/2025/06/26/Rust-1.88.0/
