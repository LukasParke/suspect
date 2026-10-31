# Diagnostic codes

<!-- Generated from the suspect-check rule registry by crates/suspect-cli/tests/reference_docs.rs; do not edit by hand. Regenerate with `SUSPECT_BLESS=1 cargo test -p suspect-cli --test reference_docs`. -->

Every code a builtin check reports, with the check that owns it, its default severity, its class and its aliases. `lint.rules.<code>` in `.suspect.yaml` overrides a severity (or turns a code `off`), `lint.design` sets every design-class code at once, and SDK-class codes appear only when the project configures code generation. An alias names the same problem under another code (usually the Spectral rule name); it is accepted wherever a code is, and findings report the canonical code. See the [linting guide](../guide/linting.md) for settings, suppression comments and baselines.

Checks: [`syntax`](#syntax), [`ref`](#ref), [`validate`](#validate), [`swagger`](#swagger), [`arazzo`](#arazzo), [`overlay`](#overlay), [`load-profile`](#load-profile), [`lint`](#lint), [`markdown`](#markdown), [`extension`](#extension), [`config`](#config), [`rules`](#rules), [`admission`](#admission), [`suppression`](#suppression).

## syntax

| Code | Default | Class | Summary | How to fix | Aliases |
|---|---|---|---|---|---|
| `syntax-error` | error | document | YAML or JSON syntax error |  | `syntax` |

## ref

| Code | Default | Class | Summary | How to fix | Aliases |
|---|---|---|---|---|---|
| `invalid-ref` | error | document | Malformed reference string | Write the reference as a URI followed by `#` and a JSON Pointer, e.g. `#/components/schemas/Pet`. |  |
| `ref-outside-allowlist` | error | document | Reference points outside the allowed document set | Point the reference at a document inside the configured reference allowlist. |  |
| `unresolved-ref` | error | document | Reference does not resolve | Fix the reference's document or pointer so it resolves to an existing node. |  |

## validate

| Code | Default | Class | Summary | How to fix | Aliases |
|---|---|---|---|---|---|
| `oas-callback-invalid` | error | document | Invalid callback object | Give the callback a valid expression key and a Path Item value with operations. |  |
| `oas-deprecated-operation` | info | document | Deprecated operation | Set `deprecated: false`, or document the replacement in the description and plan removal. |  |
| `oas-discriminator-missing-property` | error | document | Discriminator without propertyName | Add `propertyName` naming the field that distinguishes the composed schemas. |  |
| `oas-discriminator-unknown-mapping` | error | document | Discriminator mapping points at an unknown schema | Point the mapping entry at a schema that exists under `components/schemas`. |  |
| `oas-duplicate-header-param` | error | document | Duplicate header parameter | Keep one header parameter per header name; merge the duplicated definitions. |  |
| `oas-duplicate-operation-id` | error | document | Duplicate operationId | Give every operation a unique `operationId`; generated SDK method names depend on it. | `operation-operationId-unique` |
| `oas-empty-path-template` | error | document | Empty path template | Remove the empty path key, or name the resource in the template (`/items`). |  |
| `oas-example-type-mismatch` | warning | document | Example does not match the schema type set | Make the example value match the schema's declared type (and enum values when present). |  |
| `oas-header-invalid` | error | document | Invalid header entry | Header entries must be Parameter Objects with `schema`, or references to one. |  |
| `oas-http-array-invalid` | error | document | HTTP array shape is not valid | Declare the array with a JSON-Schema `type: array` and `items` instead of an ad-hoc shape. |  |
| `oas-http-boolean-invalid` | error | document | HTTP boolean shape is not valid | Declare the value with JSON-Schema `type: boolean` instead of an ad-hoc shape. |  |
| `oas-http-map-invalid` | error | document | HTTP map shape is not valid | Declare the map with `type: object` and `additionalProperties` instead of an ad-hoc shape. |  |
| `oas-license-missing-url` | warning | document | License without a URL | Add `url` to the license object pointing at the license text. | `license-url` |
| `oas-link-invalid` | error | document | Invalid link object | Give the link either `operationId` or `operationRef`, plus its parameters or request body. |  |
| `oas-media-type-invalid` | error | document | Invalid media type entry | Use a valid RFC 2046 media type key such as `application/json`. |  |
| `oas-operation-invalid` | error | document | Invalid operation object | Replace the operation value with an Operation Object. |  |
| `oas-operation-missing-operationId` | warning | document | Operation without an operationId | Add an `operationId` so generated SDK methods carry a stable name. | `operation-operationId` |
| `oas-operation-missing-responses` | error | document | Operation without responses | Add a `responses` object with at least one status code or a `default`. |  |
| `oas-operation-no-error-response` | info | document | Operation without an error response | Add a `default` response or a 4XX/5XX response so client error handling is predictable. |  |
| `oas-parameter-invalid` | error | document | Invalid parameter object | Replace the parameter value with a Parameter Object or a `$ref` to one. |  |
| `oas-parameter-missing-in` | error | document | Parameter without a location | Add `in` with one of `query`, `header`, `path`, or `cookie`. |  |
| `oas-parameter-missing-name` | error | document | Parameter without a name | Add `name` identifying the parameter on the wire. |  |
| `oas-parameter-required-missing` | error | document | Path parameter not marked required | Path parameters are always mandatory: set `required: true`. |  |
| `oas-path-item-invalid` | error | document | Invalid path item | Replace the path-item value with a Path Item Object or a `$ref` to one. |  |
| `oas-path-item-security-invalid` | error | document | Invalid path-item security requirement | Path-item security must be an array of requirement objects, or omitted. |  |
| `oas-path-param-not-declared` | error | document | Path template variable without a declared parameter | Declare every `{var}` in the path as an `in: path` parameter on the path item or operation. |  |
| `oas-path-trailing-slash` | warning | document | Path key ends with a trailing slash | Drop the trailing slash so `/pets/` becomes `/pets`. | `path-keys-no-trailing-slash` |
| `oas-request-body-content-missing` | error | document | Request body without content | Add a `content` object with at least one media type. |  |
| `oas-request-body-invalid` | error | document | Invalid request body | Replace the request body with a Request Body Object or a `$ref` to one. |  |
| `oas-response-entry-invalid` | error | document | Invalid response entry | Response keys must be status codes, status ranges, or `default`. |  |
| `oas-response-invalid` | error | document | Invalid response object | Replace the response value with a Response Object or a `$ref` to one. |  |
| `oas-response-map-invalid` | error | document | Invalid responses map | Responses must be an object keyed by status code, range, or `default`. |  |
| `oas-response-missing-description` | error | document | Response without a description | Add a `description` string to every response. |  |
| `oas-responses-invalid` | error | document | Invalid responses object | Replace `responses` with an object of status-code keys mapping to Response Objects. |  |
| `oas-schema-instance-invalid` | warning | document | Example, default or enum value violates its schema | Change the value so it validates against its schema, or correct the schema. |  |
| `oas-schema-invalid-keyword` | error | document | Schema keyword not valid in this dialect | Remove the keyword or move it into `x-` extension form; it is not valid in this dialect. |  |
| `oas-schema-invalid-kind` | error | document | Schema node is the wrong kind | Schemas must be objects or `$ref` values, not plain scalars. |  |
| `oas-schema-invalid-type` | error | document | Schema type value is not valid | `type` accepts a single type name, or an array of them in 3.1. |  |
| `oas-schema-unknown-type` | error | document | Schema type is not a known type name | Use one of `string`, `number`, `integer`, `boolean`, `object`, `array`, or `null`. |  |
| `oas-schema-unsupported-dialect` | error | document | Schema dialect not supported by this document version | Use the JSON-Schema dialect declared by the document's OpenAPI version. |  |
| `oas-security-invalid` | error | document | Invalid security declaration | Declare security schemes under `components/securitySchemes` and reference them by name. |  |
| `oas-security-requirement-invalid` | error | document | Invalid security requirement | Each requirement must map a declared scheme name to a list of required scopes. |  |
| `oas-security-scope-invalid` | error | document | Invalid security scope entry | Scope entries must be strings. |  |
| `oas-security-scopes-invalid` | error | document | Invalid security scopes list | Give the scheme a list of required scopes; use `[]` when the scheme needs none. |  |
| `oas-security-scopes-nonempty` | error | document | Security scheme requires non-empty scopes | This scheme type requires at least one scope in each requirement. |  |
| `oas-security-unknown-scheme` | error | document | Security requirement names an undeclared scheme | Declare the scheme under `components/securitySchemes`, or fix the requirement's spelling. |  |
| `oas-server-invalid` | error | document | Invalid server object | Servers must be objects with a `url`. |  |
| `oas-server-url-invalid` | error | document | Invalid server URL | Use a valid URL, optionally with `{variable}` templates. |  |
| `oas-server-variable-default-invalid` | error | document | Server variable default outside its enumeration | Set `default` to one of the values listed in the variable's `enum`. |  |
| `oas-server-variable-enum-invalid` | error | document | Server variable enumeration is not valid | Make the variable's `enum` a non-empty array of strings. |  |
| `oas-server-variable-invalid` | error | document | Invalid server variable | Server variables must be objects with a `default`. |  |
| `oas-server-variable-unknown` | error | document | Server URL references an undeclared variable | Declare the variable in the server's `variables` object, or remove it from the URL. |  |
| `oas-tag-parent-cycle` | error | document | Tag parent chain forms a cycle | Break the cycle so every tag's `parent` chain ends at a tag without a parent. |  |
| `oas-tag-parent-unknown` | error | document | Tag parent is not a declared tag | Declare the parent in the root `tags` list, or correct the `parent` name. |  |
| `oas-tag-undeclared` | warning | document | Operation tag not declared in the root tags list | Add the tag to the root `tags` list, or use one already declared there. | `operation-tag-defined` |
| `oas-unused-path-param` | warning | document | Declared path parameter does not appear in the path template | Remove the unused `in: path` parameter or add its `{name}` to the path template. |  |
| `oas-webhook-unsupported-version` | error | document | Webhooks require a newer OpenAPI version | Move the webhook into a 3.1+ document, or express it as a callback. |  |
| `sdk-colon-path-parameters` | info | sdk | Colon-style path template | Prefer `{var}` templates over `:var` so path parameters need no compatibility profile. |  |
| `sdk-operation-missing-id` | info | sdk | Operation will be selected by method and path | Add an `operationId` so generated SDK methods carry a stable name. |  |
| `sdk-stream-response-untyped` | info | sdk | Stream response will generate untyped events | Declare a `schema` on the `text/event-stream` media type to get typed stream events. |  |

## swagger

| Code | Default | Class | Summary | How to fix | Aliases |
|---|---|---|---|---|---|
| `swagger-body-param-schema` | error | document | Body parameter without a schema | Give the body parameter a `schema` describing the payload. |  |
| `swagger-definition-shape` | error | document | Definition does not declare a schema shape | Give the definition a `type`, a `$ref`, or a composition (`allOf`/`anyOf`/`oneOf`). |  |
| `swagger-info-field-required` | error | document | Info object missing a required field | Add the missing field to `info`. |  |
| `swagger-info-required` | error | document | Document without an info object | Add an `info` object with `title` and `version`. |  |
| `swagger-operation-missing-responses` | error | document | Operation without responses | Add a `responses` object with at least one status code. |  |
| `swagger-parameter-missing-in` | error | document | Parameter without a location | Add `in` with one of `query`, `header`, `path`, `formData`, or `body`. |  |
| `swagger-parameter-missing-name` | error | document | Parameter without a name | Add `name` identifying the parameter on the wire. |  |
| `swagger-parameter-not-in-template` | error | document | Path parameter does not appear in the path template | Remove the parameter or add `{name}` to the path template. |  |
| `swagger-path-param-undeclared` | error | document | Path template variable without a declared parameter | Declare every `{var}` in the path as an `in: path` parameter with `required: true`. |  |
| `swagger-response-missing-description` | error | document | Response without a description | Add a `description` string to every response. |  |
| `swagger-security-undefined` | error | document | Security requirement names an undeclared scheme | Declare the scheme under `securityDefinitions`, or fix the requirement's spelling. |  |

## arazzo

| Code | Default | Class | Summary | How to fix | Aliases |
|---|---|---|---|---|---|
| `arazzo-action-missing-type` | error | document | Action without a type |  |  |
| `arazzo-async-fields-on-http-step` | error | document | AsyncAPI step fields on an HTTP step |  |  |
| `arazzo-channel-on-non-asyncapi-source` | error | document | channelPath on a source that is not AsyncAPI |  |  |
| `arazzo-criterion-missing-condition` | error | document | Criterion without a condition |  |  |
| `arazzo-depends-on-cycle` | error | document | Workflows depend on each other in a cycle |  |  |
| `arazzo-depends-on-unknown-step` | error | document | dependsOn names an unknown or later step |  |  |
| `arazzo-depends-on-unknown-workflow` | error | document | dependsOn names an undeclared workflow |  |  |
| `arazzo-duplicate-source-name` | error | document | Duplicate source description name |  |  |
| `arazzo-duplicate-step-id` | error | document | Duplicate stepId within a workflow |  |  |
| `arazzo-duplicate-workflow-id` | error | document | Duplicate workflowId |  |  |
| `arazzo-forward-output-reference` | error | document | Reference to a later step's outputs without dependsOn |  |  |
| `arazzo-goto-missing-target` | error | document | `goto` action without a workflowId or stepId |  |  |
| `arazzo-goto-unknown-workflow` | error | document | `goto` action names an unknown workflow |  |  |
| `arazzo-invalid-action-kind` | error | document | Step action is neither `send` nor `receive` |  |  |
| `arazzo-invalid-condition` | error | document | Invalid criterion condition |  |  |
| `arazzo-invalid-depends-on` | error | document | Invalid dependsOn reference |  |  |
| `arazzo-invalid-operation-path` | error | document | Invalid operationPath expression |  |  |
| `arazzo-invalid-target` | error | document | Parameter target is not a runtime expression |  |  |
| `arazzo-invalid-timeout` | error | document | Step timeout is not a positive integer |  |  |
| `arazzo-missing-info-title` | error | document | Info object without a title |  |  |
| `arazzo-missing-source-descriptions` | error | document | Document without source descriptions |  |  |
| `arazzo-missing-step-id` | error | document | Step without a stepId |  |  |
| `arazzo-missing-version` | error | document | Document without an `arazzo` version |  |  |
| `arazzo-missing-workflow-id` | error | document | Workflow without a workflowId |  |  |
| `arazzo-output-unknown-name` | error | document | Output reference names an undeclared output |  |  |
| `arazzo-output-unknown-step` | error | document | Output reference names an unknown step |  |  |
| `arazzo-output-unknown-workflow` | error | document | Output reference names an unknown workflow |  |  |
| `arazzo-parameter-incomplete` | error | document | Parameter without a name and location or target |  |  |
| `arazzo-selector-output-unsupported` | error | document | Selector Object outputs are not executed |  |  |
| `arazzo-self-fragment` | error | document | `$self` contains a fragment identifier |  |  |
| `arazzo-step-missing-operation` | error | document | Step without an operation, channel or workflow target |  | `arazzo-step-operation` |
| `arazzo-step-multiple-targets` | error | document | Step with more than one target |  | `arazzo-step-operation` |
| `arazzo-unknown-action-type` | error | document | Unknown action type |  |  |
| `arazzo-unsupported-version` | error | document | Arazzo version not supported |  |  |

## overlay

| Code | Default | Class | Summary | How to fix | Aliases |
|---|---|---|---|---|---|
| `overlay-action-missing-description` | hint | document | Action without a description |  | `overlay-action-description` |
| `overlay-empty-actions` | error | document | Overlay without actions |  |  |
| `overlay-update-with-remove` | warning | document | Action sets `update` while `remove` is true |  |  |

## load-profile

| Code | Default | Class | Summary | How to fix | Aliases |
|---|---|---|---|---|---|
| `load-condition-shape` | error | document | Condition without an expression |  |  |
| `load-condition-syntax` | error | document | Condition expression does not parse |  |  |
| `load-data-fields` | error | document | CSV data without field names |  |  |
| `load-data-missing` | error | document | Data file not found |  |  |
| `load-data-order` | error | document | Unknown data order |  |  |
| `load-data-path` | error | document | Data source without a path |  |  |
| `load-duration` | error | document | Invalid duration |  |  |
| `load-empty-phases` | error | document | Profile with an empty phase list |  |  |
| `load-empty-pool` | error | document | Input pool is empty |  |  |
| `load-empty-scenarios` | error | document | Profile with an empty scenario list |  |  |
| `load-hook-is-scenario` | warning | document | Hook workflow is also a scenario |  |  |
| `load-missing-phases` | error | document | Profile without phases |  |  |
| `load-missing-scenarios` | error | document | Profile without scenarios |  |  |
| `load-missing-version` | error | document | Profile without a `suspectLoad` version |  |  |
| `load-phase-count` | error | document | Arrival count below 1 |  |  |
| `load-phase-duration` | error | document | Phase without a valid duration |  |  |
| `load-phase-kind` | error | document | Phase does not set exactly one arrival kind |  |  |
| `load-phase-ramp` | warning | document | `rampTo` on a phase that is not `arrivalRate` |  |  |
| `load-phase-rate` | error | document | Invalid arrival rate |  |  |
| `load-phase-shape` | error | document | Phase is not a mapping |  |  |
| `load-phase-vus` | error | document | Virtual user count below 1 |  |  |
| `load-scenario-weight` | error | document | Scenario weight is not positive |  |  |
| `load-scenario-workflow` | error | document | Scenario without a workflow id |  |  |
| `load-threshold-shape` | error | document | Threshold is not a single `metric: max` pair |  |  |
| `load-type` | error | document | Value has the wrong type |  |  |
| `load-unknown-workflow` | error | document | Workflow is not in the Arazzo document |  |  |
| `load-unsupported-version` | error | document | Load profile version not supported |  |  |

## lint

| Code | Default | Class | Summary | How to fix | Aliases |
|---|---|---|---|---|---|
| `arazzo-step-operation` | error | document | Every Arazzo step must target exactly one of `operationId` or `operationPath`. |  | alias of `arazzo-step-missing-operation`, `arazzo-step-multiple-targets` |
| `arazzo-workflow-description` | hint | document | Every Arazzo workflow should have a `description`. |  |  |
| `info-contact` | warning | document | Info object should contain a contact object. |  |  |
| `info-description` | info | document | OpenAPI object should have `description`. |  |  |
| `info-license` | warning | document | Info object should contain a license object. |  |  |
| `license-url` | warning | document | License object should include `url`. |  | alias of `oas-license-missing-url` |
| `no-$ref-siblings` | error | document | $ref values must not have siblings other than description/summary (3.0). |  |  |
| `no-ambiguous-paths` | error | document | Path keys must not be ambiguous after normalization (no duplicates). |  |  |
| `no-eval-in-markdown` | error | document | Markdown descriptions must not contain `eval(` or `Function(`. |  |  |
| `no-identical-paths` | warning | document | Path keys must not be identical modulo template variables. |  |  |
| `no-script-tags-in-markdown` | error | document | Markdown descriptions must not contain `<script>` tags. |  |  |
| `oas2-unused-definition` | warning | document | Definition should be referenced by at least one `$ref`. |  |  |
| `oas3-api-contact` | info | document | OpenAPI 3.x API description must have a contact object. |  |  |
| `oas3-api-servers` | warning | document | OpenAPI 3.x servers must be defined. |  |  |
| `oas3-unused-component` | warning | document | Component should be referenced by at least one `$ref`. |  |  |
| `openapi-tags` | warning | document | OpenAPI object should have non-empty `tags` array. |  |  |
| `openapi-tags-alphabetical` | hint | document | Root `tags` should be alphabetically sorted. |  |  |
| `operation-4xx-response` | warning | document | Operation should have at least one 4XX response. |  |  |
| `operation-default-response` | error | document | Operation must have a `default` or 2XX response. |  |  |
| `operation-description` | info | document | Operation should have `description`. |  |  |
| `operation-operationId` | error | document | Operation must have an `operationId`. |  | alias of `oas-operation-missing-operationId` |
| `operation-operationId-unique` | error | document | Every operation must have a unique `operationId`. |  | alias of `oas-duplicate-operation-id` |
| `operation-operationId-valid-in-url` | error | document | Operation `operationId` must only contain URL-safe characters. |  |  |
| `operation-singular-tag` | hint | document | Operation should have exactly one tag. |  |  |
| `operation-success-response` | error | document | Operation must have at least one 2XX response. |  |  |
| `operation-summary` | warning | document | Operation should have `summary`. |  |  |
| `operation-tag-defined` | error | document | Operation tags must be declared in the root `tags` list. |  | alias of `oas-tag-undeclared` |
| `operation-tags` | warning | document | Operation should have non-empty `tags` array. |  |  |
| `overlay-action-description` | hint | document | Every overlay action should have a `description`. |  | alias of `overlay-action-missing-description` |
| `overlay-info-description` | hint | document | Overlay `info` object should have a `description`. |  |  |
| `parameter-description` | info | document | Parameter objects should have `description`. |  |  |
| `parameter-schema-or-content` | error | document | Parameter must define `schema` or `content`. |  |  |
| `path-declarations-must-exist` | error | document | Path keys must begin with a leading slash. |  |  |
| `path-keys-no-trailing-slash` | warning | document | Path keys must not end with a trailing slash. |  | alias of `oas-path-trailing-slash` |
| `path-not-include-query` | error | document | Path keys must not include a query string. |  |  |
| `path-params` | error | document | Path template variables must be declared as path parameters. |  |  |
| `security-defined` | error | document | Security requirement must name a declared scheme. |  |  |
| `security-jwt-bearer-format` | hint | design | Bearer schemes carrying JWTs should declare `bearerFormat: JWT` so clients parse tokens correctly. |  |  |
| `security-no-apikey-in-query` | error | design | API keys in query strings leak into server logs, proxy logs, and browser history — use a header or cookie. |  |  |
| `security-no-basic-auth` | warning | design | HTTP Basic authentication sends credentials on every request without hashing — prefer bearer tokens or OAuth 2.0. |  |  |
| `security-no-credentials-in-url` | error | design | Path and query parameter names must not look like credentials (token, key, secret, password, credential). |  |  |
| `security-no-delete-without-id` | warning | design | DELETE operations must target a specific resource — collection-wide deletes are destructive and unauthenticated by scope. |  |  |
| `security-no-sensitive-params-in-url` | error | design | Sensitive parameters (token, key, secret, password) must not travel in the URL path or query — URLs leak into logs. |  |  |
| `security-oauth-scopes-documented` | warning | design | OAuth 2.0 flows should declare at least one scope — empty scope maps hide the permission model. |  |  |
| `security-operation-auth-or-public` | info | design | Operations without operation-level security inherit the root requirement — mark intentionally public operations with an empty requirement (`security: [{}]`). |  |  |
| `security-rate-limit-documented` | info | design | Operations should document rate limiting — a 429 response or rate-limit headers. |  |  |
| `security-server-https-only` | error | design | Server URLs must use HTTPS — plaintext transport exposes credentials and data. |  |  |
| `server-host-not-example` | warning | document | Server URLs should not use example hosts (localhost, 127.0.0.1, example.com). |  |  |
| `server-trailing-slash` | warning | document | Server URLs should not end in a trailing slash. |  |  |
| `server-variables` | error | document | Templated server URLs must declare every variable they use. |  |  |
| `tag-description` | info | document | Tag objects should have `description`. |  |  |
| `typed-enum` | error | document | Enum member does not match the schema's declared type. |  |  |

## markdown

| Code | Default | Class | Summary | How to fix | Aliases |
|---|---|---|---|---|---|
| `md-bare-url` | hint | document | MD034: no bare URLs |  |  |
| `md-code-fence-language` | hint | document | MD040: fenced code blocks name a language |  |  |
| `md-heading-increment` | hint | document | MD001: heading levels increment by one |  |  |
| `md-multiple-blank-lines` | hint | document | MD012: no multiple consecutive blank lines |  |  |
| `md-single-h1` | hint | document | MD025: one top-level heading per description |  |  |

## extension

| Code | Default | Class | Summary | How to fix | Aliases |
|---|---|---|---|---|---|
| `extension-schema` | error | document | Vendor extension value fails its registered schema |  |  |

## config

| Code | Default | Class | Summary | How to fix | Aliases |
|---|---|---|---|---|---|
| `config-manifest-policy` | warning | document | Policy section in a project manifest (deprecated; move it to .suspect.yaml) |  |  |
| `suspect.config` | warning | document | Unknown key, wrong type or deprecated spelling in a suspect configuration file |  |  |

## rules

| Code | Default | Class | Summary | How to fix | Aliases |
|---|---|---|---|---|---|
| `rules-id-collision` | error | document | TypeScript rule id reuses a builtin code or alias |  |  |
| `rules-runtime-unavailable` | warning | document | TypeScript rules are configured but bun is not on PATH |  |  |

## admission

| Code | Default | Class | Summary | How to fix | Aliases |
|---|---|---|---|---|---|
| `http-body-metadata-invalid` | error | sdk | Request body value is not an object |  |  |
| `http-body-schema-invalid` | error | sdk | Request body schema is not a schema |  |  |
| `http-content-metadata-invalid` | error | sdk | Content value is not an object |  |  |
| `http-method-token` | error | sdk | Unrecognized HTTP method token |  |  |
| `http-method-unsupported` | error | sdk | HTTP method not supported by the profile | Restructure the operation onto GET, POST, PATCH, PUT, or DELETE. |  |
| `http-operation-id` | error | sdk | Operation without a usable operationId | Add a non-empty `operationId` to every operation the profile selects. |  |
| `http-operation-id-duplicate` | error | sdk | Duplicate operationId | Make every selected operationId unique; generated method names depend on it. |  |
| `http-operation-not-found` | error | sdk | Requested operation does not exist | Select operations that exist as outgoing path operations in the document. |  |
| `http-operation-object` | error | sdk | Operation value is not an object |  |  |
| `http-parameter-metadata-invalid` | error | sdk | Parameter value is not an object |  |  |
| `http-parameters-metadata-invalid` | error | sdk | Parameters collection is not an array | Make `parameters` an array of Parameter Objects or references. |  |
| `http-response-metadata-invalid` | error | sdk | Response entry is not an object |  |  |
| `http-responses-metadata-invalid` | error | sdk | Responses value is not an object |  |  |
| `http-schema-metadata-invalid` | error | sdk | Schema value is not an object or boolean | Replace the value with a Schema Object or a boolean schema. |  |
| `http-security-metadata-invalid` | error | sdk | Security scheme value is not an object |  |  |
| `http-server-metadata-invalid` | error | sdk | Server value is not an object |  |  |
| `naming-method-collision` | info | sdk | Operations whose method names collide | Rename one of the operations so the generated method names differ in every convention. |  |
| `naming-model-collision` | info | sdk | Schemas whose model names collide | Rename one of the schemas so the generated model names differ in every convention. |  |

## suppression

| Code | Default | Class | Summary | How to fix | Aliases |
|---|---|---|---|---|---|
| `suppression-unknown-code` | warning | document | Suppression names an unknown or unsuppressible code | Name a code or alias the checks report; `syntax-error` cannot be suppressed. |  |
| `suppression-unused` | hint | document | Suppression matched no finding (off unless `lint.rules` turns it on) | Remove the suppression, or the code it no longer needs. |  |

## Code families

Open-ended codes registered by prefix: a code no table lists takes the metadata of the family with the longest matching prefix.

| Family | Check | Default | Class | Summary |
|---|---|---|---|---|
| `http-*` | admission | error | sdk | The SDK HTTP layer cannot generate this operation or value |
| `DUPLICATE_*` | admission | error | sdk | Duplicate declaration |
| `AMBIGUOUS_*` | admission | error | sdk | Contract semantics the compiler cannot disambiguate |
| `INVALID_*` | admission | error | sdk | Contract semantics the compiler cannot interpret |
| `invalid-*` | admission | error | sdk | Contract semantics the compiler cannot interpret |
| `HTTP_REFERENCE*` | admission | error | sdk | Reference does not resolve in the contract |
| `UNSUPPORTED_*` | admission | error | sdk | Semantics not supported by this compiler version |
| `unsupported-*` | admission | error | sdk | Semantics not supported by this compiler version |
| `UNKNOWN_*` | admission | error | sdk | Contract names something the document does not declare |
| `RECURSIVE_*` | admission | error | sdk | Recursive construct the compiler cannot lower |

