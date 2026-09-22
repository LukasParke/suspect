//! Keyword documentation for hover: what the keyword means, its value
//! domain, dialect availability, interaction notes, and a mini example.
//!
//! Covers the OpenAPI structural keywords plus the full JSON Schema
//! 2020-12 vocabulary (and 3.0-only keywords). This is the depth layer
//! that answers "what does `minimum:` do?" without leaving the editor.

/// One keyword's documentation entry.
pub struct KeywordDoc {
    /// Human title (`minimum` / `openapi`).
    pub title: &'static str,
    /// Which spec family introduced/maintains it.
    pub family: &'static str,
    /// Availability across dialects.
    pub availability: &'static str,
    /// What values it accepts.
    pub value_domain: &'static str,
    /// What it does, in one or two sentences.
    pub meaning: &'static str,
    /// Interaction notes with sibling keywords.
    pub interactions: &'static str,
    /// A minimal example of usage.
    pub example: &'static str,
}

macro_rules! kw {
    ($title:expr, $family:expr, $availability:expr, $domain:expr, $meaning:expr, $interactions:expr, $example:expr) => {
        KeywordDoc {
            title: $title,
            family: $family,
            availability: $availability,
            value_domain: $domain,
            meaning: $meaning,
            interactions: $interactions,
            example: $example,
        }
    };
}

/// Document- and object-level OpenAPI keywords (hovered outside schemas).
const OPENAPI_KEYWORDS: &[(&str, KeywordDoc)] = &[
    (
        "openapi",
        kw!(
            "openapi",
            "OpenAPI",
            "3.0 · 3.1 · 3.2",
            "Semantic version string",
            "Declares the OpenAPI Specification version the document conforms to. Tooling uses it to pick the right validation rules and codegen dialects.",
            "Root document only. `3.0.x` uses the OAS 3.0 Schema Object dialect; `3.1.x` aligns schemas with JSON Schema 2020-12.",
            "openapi: 3.1.0"
        ),
    ),
    (
        "swagger",
        kw!(
            "swagger",
            "Swagger 2.0",
            "2.0 only",
            "Semantic version string",
            "The Swagger 2.0 predecessor of `openapi`. Documents using it are validated with the 2.0 battery (definitions, securityDefinitions, body/formData parameters).",
            "Mutually exclusive with `openapi`.",
            "swagger: \"2.0\""
        ),
    ),
    (
        "info",
        kw!(
            "info",
            "OpenAPI",
            "2.0 · 3.x",
            "Info Object",
            "Required metadata: title, version, description, contact, license, termsOfService.",
            "Root document only.",
            "info:\n  title: Plex Media Server\n  version: 1.1.1"
        ),
    ),
    (
        "servers",
        kw!(
            "servers",
            "OpenAPI",
            "3.0+",
            "Array of Server Objects",
            "Connectivity targets for the API. Each server URL may carry `{variable}` templates with per-variable enums and defaults.",
            "Root default; an operation or path-item `servers` fully replaces the root list (no merge).",
            "servers:\n  - url: https://{host}:{port}\n    variables:\n      port: {default: '32400'}"
        ),
    ),
    (
        "security",
        kw!(
            "security",
            "OpenAPI",
            "2.0 · 3.x",
            "Array of Security Requirement Objects",
            "Declares which security schemes apply. An empty requirement object (`{}`) marks the operation public.",
            "Names must exist in `components.securitySchemes` (3.x) or `securityDefinitions` (2.0). Multiple requirements in one object are ANDed; multiple objects are ORed.",
            "security:\n  - apiKey: []\n  - {}"
        ),
    ),
    (
        "paths",
        kw!(
            "paths",
            "OpenAPI",
            "2.0 · 3.x",
            "Map of path templates to Path Items",
            "The relative endpoints of the API. Path templates may contain `{variable}` expressions that must be declared as `in: path` parameters.",
            "Root document only. Path-item `servers`/`parameters` cascade to the operations.",
            "paths:\n  /pets/{petId}:\n    get: ..."
        ),
    ),
    (
        "webhooks",
        kw!(
            "webhooks",
            "OpenAPI",
            "3.1+",
            "Map of names to Path Items",
            "Declares outgoing callbacks the API may invoke, addressed by name rather than path.",
            "3.0 documents silently lose this collection under contract compilation — move to 3.1+.",
            "webhooks:\n  petCreated:\n    post: ..."
        ),
    ),
    (
        "components",
        kw!(
            "components",
            "OpenAPI",
            "3.0+",
            "Components Object",
            "Reusable objects — schemas, responses, parameters, securitySchemes and more — referenced via `#/components/<section>/<name>`.",
            "Entries not reachable from `paths`/`webhooks` are legal but are not analyzed by closure-based consumers.",
            "components:\n  schemas:\n    Pet: ..."
        ),
    ),
    (
        "operationId",
        kw!(
            "operationId",
            "OpenAPI",
            "2.0 · 3.x",
            "Unique string (URL-safe recommended)",
            "Names the operation for tooling: generated SDK method names, links (`links.*.operationId`), and Arazzo steps all target it.",
            "Must be unique document-wide. Unnamed operations fall back to `METHOD /path` selection.",
            "get:\n  operationId: listPets"
        ),
    ),
    (
        "deprecated",
        kw!(
            "deprecated",
            "OpenAPI",
            "2.0 · 3.x",
            "Boolean (default false)",
            "Marks the operation/schema/parameter as scheduled for removal so generated clients surface it.",
            "Describe the migration path in `description`; both render as deprecated in editors.",
            "get:\n  deprecated: true"
        ),
    ),
];

/// JSON Schema keyword documentation (hovered inside schemas).
const SCHEMA_KEYWORDS: &[(&str, KeywordDoc)] = &[
    (
        "type",
        kw!(
            "type",
            "JSON Schema",
            "2.0 (string) · 3.0 (string) · 3.1 (string or array)",
            "One of `string`, `number`, `integer`, `boolean`, `object`, `array`, `null`; 3.1 also accepts an array of them",
            "The JSON data type the value must have. An array form accepts any of the listed types.",
            "Pairs with `format`; narrows what `enum`/`const` may contain. `integer` matches any number with zero fractional part.",
            "type: [string, \"null\"]"
        ),
    ),
    (
        "format",
        kw!(
            "format",
            "OpenAPI / JSON Schema",
            "2.0 · 3.x",
            "Known annotations: `int32`, `int64`, `float`, `double`, `date`, `date-time`, `password`, `byte`, `binary`, `email`, `uuid`, `uri`, …",
            "Annotates the intended wire representation of the value. OpenAPI registers a fixed set; OAS 3.1 makes most formats advisory but codegen treats them as binding.",
            "Only meaningful alongside `type` (`format: date-time` with `type: string`). Unknown formats are legal but unvalidated.",
            "type: string\nformat: date-time"
        ),
    ),
    (
        "enum",
        kw!(
            "enum",
            "JSON Schema",
            "2.0 · 3.x · 2020-12",
            "Non-empty array of values",
            "The value must equal one of the listed constants (deep equality).",
            "With `type`, members must also satisfy the type. Generated SDKs model enums from this list; removing a member breaks clients.",
            "enum: [available, pending, sold]"
        ),
    ),
    (
        "const",
        kw!(
            "const",
            "JSON Schema",
            "3.1 · 2020-12 (not in 3.0)",
            "Any JSON value",
            "The value must equal this constant exactly.",
            "Shorthand for a one-element `enum`. Not in the 3.0 vocabulary — moves it into `x-` or an enum of one.",
            "const: 7"
        ),
    ),
    (
        "minimum",
        kw!(
            "minimum",
            "JSON Schema",
            "2.0 · 3.x · 2020-12",
            "Number",
            "Inclusive lower bound: the value must be `>= minimum`.",
            "`exclusiveMinimum: true` (3.0 boolean form) tightens it to exclusive; in 2020-12 `exclusiveMinimum` is the bound itself as a number.",
            "minimum: 1\nmaximum: 65535"
        ),
    ),
    (
        "maximum",
        kw!(
            "maximum",
            "JSON Schema",
            "2.0 · 3.x · 2020-12",
            "Number",
            "Inclusive upper bound: the value must be `<= maximum`.",
            "3.0's `exclusiveMaximum: true` boolean form tightens to exclusive; 2020-12 uses a numeric `exclusiveMaximum`.",
            "minimum: 1\nmaximum: 65535"
        ),
    ),
    (
        "exclusiveMinimum",
        kw!(
            "exclusiveMinimum",
            "JSON Schema",
            "3.0: boolean · 2020-12: number",
            "Boolean (3.0) or number (2020-12)",
            "3.0: modifies `minimum` to be exclusive. 2020-12: the exclusive lower bound itself.",
            "The two dialects are incompatible — moving between them requires rewriting the keyword.",
            "minimum: 0\nexclusiveMinimum: true   # 3.0\n# or, 2020-12:\nexclusiveMinimum: 0"
        ),
    ),
    (
        "multipleOf",
        kw!(
            "multipleOf",
            "JSON Schema",
            "2.0 · 3.x · 2020-12",
            "Positive number",
            "The value must be an integer multiple of this factor.",
            "`multipleOf: 0.01` expresses currency precision. Zero is invalid.",
            "type: number\nmultipleOf: 0.01"
        ),
    ),
    (
        "minLength",
        kw!(
            "minLength",
            "JSON Schema",
            "2.0 · 3.x · 2020-12",
            "Non-negative integer",
            "Minimum string length (in code points).",
            "Applies only when the value is a string; other types pass.",
            "type: string\nminLength: 1"
        ),
    ),
    (
        "maxLength",
        kw!(
            "maxLength",
            "JSON Schema",
            "2.0 · 3.x · 2020-12",
            "Non-negative integer",
            "Maximum string length (in code points).",
            "Applies only to strings. Generated clients may map this to DB column widths.",
            "type: string\nmaxLength: 64"
        ),
    ),
    (
        "pattern",
        kw!(
            "pattern",
            "JSON Schema",
            "2.0 · 3.x · 2020-12",
            "ECMA-262 regular expression",
            "The string (if present) must match this regex — it is not implicitly anchored.",
            "Use `^(...)$` when the whole string must match. Generated SDKs compile this into native validation.",
            "type: string\npattern: \"^\\\\d[x:]\\\\d$\""
        ),
    ),
    (
        "items",
        kw!(
            "items",
            "JSON Schema",
            "2.0 · 3.x · 2020-12",
            "Schema",
            "The schema every element of an array must satisfy.",
            "Requires `type: array` (3.0 enforces this). In 2020-12, `items` alone (without `prefixItems`) applies to ALL elements.",
            "type: array\nitems:\n  $ref: '#/components/schemas/Pet'"
        ),
    ),
    (
        "prefixItems",
        kw!(
            "prefixItems",
            "JSON Schema",
            "3.1 · 2020-12 (not in 3.0)",
            "Array of Schemas",
            "Tuple validation: element *i* must satisfy prefixItems[*i*].",
            "Elements beyond the prefix are validated by `items` (if present). Not in 3.0 — model tuples with min/maxItems plus items in 3.0.",
            "prefixItems:\n  - {type: string}\n  - {type: integer}"
        ),
    ),
    (
        "nullable",
        kw!(
            "nullable",
            "OpenAPI",
            "3.0 only",
            "Boolean (default false)",
            "Widens the declared type to also accept `null`.",
            "3.1 replaces this with `type: [T, \"null\"]`. The two are incompatible spellings; codegen and validation treat them differently.",
            "type: string\nnullable: true"
        ),
    ),
    (
        "required",
        kw!(
            "required",
            "JSON Schema",
            "2.0 · 3.x · 2020-12",
            "Array of property names",
            "These property names must be present (value may still be null unless the property schema forbids it).",
            "Inside a parameter object, `required: true` is the boolean spelling instead. Every `in: path` parameter is implicitly required.",
            "type: object\nrequired: [name]\nproperties:\n  name: {type: string}"
        ),
    ),
    (
        "properties",
        kw!(
            "properties",
            "JSON Schema",
            "2.0 · 3.x · 2020-12",
            "Map of names to Schemas",
            "Object property schemas. Absent properties are unconstrained unless `required` lists them.",
            "Additional keys not listed pass unless `additionalProperties: false`. Works with `patternProperties`.",
            "properties:\n  name: {type: string}\n  age: {type: integer}"
        ),
    ),
    (
        "additionalProperties",
        kw!(
            "additionalProperties",
            "JSON Schema",
            "2.0 · 3.x · 2020-12",
            "Boolean or Schema",
            "`false` forbids unknown keys; a schema validates each unknown key's value.",
            "Switching to `false` in a published spec is breaking (clients sending extras fail). Free-form objects: `{type: object}` with no additionalProperties constraint.",
            "type: object\nadditionalProperties: false"
        ),
    ),
    (
        "allOf",
        kw!(
            "allOf",
            "JSON Schema",
            "2.0 · 3.x · 2020-12",
            "Array of Schemas",
            "The value must satisfy ALL subschemas — logical AND. The canonical OAS inheritance pattern: child allOf parent + own properties.",
            "Produces an intersection in codegen; validators may not merge conflicting constraints loudly.",
            "allOf:\n  - $ref: '#/components/schemas/Base'\n  - type: object\n    properties:\n      extra: {type: string}"
        ),
    ),
    (
        "anyOf",
        kw!(
            "anyOf",
            "JSON Schema",
            "3.0 · 3.1 · 2020-12",
            "Array of Schemas",
            "The value must satisfy at least one subschema — logical OR.",
            "Codegen may widen to a union; prefer `oneOf` when exclusivity matters to generated types.",
            "anyOf:\n  - {type: string}\n  - {type: integer}"
        ),
    ),
    (
        "oneOf",
        kw!(
            "oneOf",
            "JSON Schema",
            "3.0 · 3.1 · 2020-12",
            "Array of Schemas",
            "The value must satisfy EXACTLY ONE subschema — exclusive OR.",
            "Validation fails if two branches both match (use discriminators to disambiguate). Generated SDKs model this as a tagged union when a discriminator exists.",
            "oneOf:\n  - $ref: '#/components/schemas/Cat'\n  - $ref: '#/components/schemas/Dog'\ndiscriminator:\n  propertyName: kind"
        ),
    ),
    (
        "discriminator",
        kw!(
            "discriminator",
            "OpenAPI",
            "3.0+",
            "Discriminator Object",
            "Tells consumers how to pick the correct branch of a `oneOf`/`anyOf`: the payload field to inspect (`propertyName`) and optional explicit `mapping`.",
            "Branch schemas should carry a `const`/`enum` on the property. `mapping` values are schema names or full refs.",
            "oneOf:\n  - $ref: '#/components/schemas/Dog'\ndiscriminator:\n  propertyName: kind\n  mapping:\n    dog: Dog"
        ),
    ),
    (
        "description",
        kw!(
            "description",
            "OpenAPI",
            "everywhere",
            "CommonMark string",
            "Human documentation rendered by editors, docs generators, and SDK doc comments.",
            "On parameters and schemas this is the primary documentation slot; keep markdown clean — it becomes doc comments.",
            "description: >-\n  Rates a media item.\n  Supports thumbs up/down."
        ),
    ),
    (
        "$ref",
        kw!(
            "$ref",
            "JSON Schema",
            "2.0 · 3.x · 2020-12",
            "URI reference (usually a JSON Pointer)",
            "References another schema (or object); the reference is substituted for this position.",
            "In 3.0, `$ref` objects may carry NO sibling keywords — they are ignored. In 3.1 siblings apply alongside. Relative refs resolve against the current document URL.",
            "$ref: '#/components/schemas/Pet'"
        ),
    ),
    // -- OAS discriminator and XML objects ----------------------------------------

    // -- OAS discriminator and XML objects ------------------------------------------
    (
        "propertyName",
        kw!(
            "propertyName",
            "OpenAPI",
            "3.0+",
            "String",
            "The payload field whose value selects the branch of a `oneOf`/`anyOf`. Required on a Discriminator Object.",
            "Branch schemas should constrain the property (`const` or `enum`) so clients can switch on it. Missing here surfaces as `oas-discriminator-missing-property`.",
            "discriminator:\n  propertyName: kind"
        ),
    ),
    (
        "mapping",
        kw!(
            "mapping",
            "OpenAPI",
            "3.0+",
            "Map of values to schema names or refs",
            "Explicit payload-value to schema wiring for a discriminator, overriding name-matching inference.",
            "The LSP completes mapping values with schema names and rewrites them on rename.",
            "mapping:\n  dog: Dog\n  cat: Cat"
        ),
    ),
    (
        "namespace",
        kw!(
            "namespace",
            "OpenAPI",
            "3.0+",
            "URI string",
            "The XML namespace for the element described by the XML Object.",
            "Only applies when the API is serialized as XML (PMS-style specs).",
            "namespace: https://example.com/schema"
        ),
    ),
    (
        "prefix",
        kw!(
            "prefix",
            "OpenAPI",
            "3.0+",
            "String",
            "The namespace prefix for the XML element.",
            "Requires `namespace` to be meaningful in the wire document.",
            "prefix: plex"
        ),
    ),
    (
        "attribute",
        kw!(
            "attribute",
            "OpenAPI",
            "3.0+",
            "Boolean (default false)",
            "Serializes this property as an XML attribute rather than a child element.",
            "Pairs with `wrapped` for arrays. PMS-style XML specs lean on this heavily.",
            "attribute: true"
        ),
    ),
    (
        "wrapped",
        kw!(
            "wrapped",
            "OpenAPI",
            "3.0+",
            "Boolean (default false)",
            "Wraps an array in an outer XML element named after the property.",
            "Only meaningful for array properties with `attribute: false`.",
            "wrapped: true"
        ),
    ),
    // The entries below complete the dictionary: every OpenAPI 3.0/3.1/3.2
    // structural keyword and the full JSON Schema 2020-12 core/applicator/
    // validation/basic/content vocabularies. The completeness test pins this
    // against the canonical keyword list, so a missing keyword fails CI.

    // -- OAS root and metadata ------------------------------------------------
    (
        "jsonSchemaDialect",
        kw!(
            "jsonSchemaDialect",
            "OpenAPI",
            "3.1+",
            "URI string",
            "Declares the default JSON Schema dialect for every schema in the document that does not declare its own `$schema`.",
            "Defaults to the OAS 3.1 Schema dialect. A schema-level `$schema` overrides it per-subtree.",
            "jsonSchemaDialect: https://spec.openapis.org/oas/3.1/dialect/base"
        ),
    ),
    (
        "tags",
        kw!(
            "tags",
            "OpenAPI",
            "2.0 · 3.x",
            "Array of Tag Objects",
            "Declares the tag vocabulary: name, description, external docs. Operation `tags` entries should reference declared names.",
            "An operation tag not declared here surfaces as `oas-tag-undeclared`. Root declaration order is also docs-rendering order.",
            "tags:\n  - name: pets\n    description: Pet routes"
        ),
    ),
    (
        "externalDocs",
        kw!(
            "externalDocs",
            "OpenAPI",
            "2.0 · 3.x",
            "External Documentation Object",
            "A link to external documentation for the API, an operation, a tag, or a schema.",
            "Carries `description` and required `url`. Works at root, operation, tag, and schema level.",
            "externalDocs:\n  url: https://docs.example.com"
        ),
    ),
    (
        "title",
        kw!(
            "title",
            "OpenAPI / JSON Schema",
            "2.0 · 3.x · 2020-12",
            "String",
            "Human-readable name. At root it names the API inside `info`; on a schema it labels the model.",
            "Docs generators render it as a heading; SDK codegen may derive type names when `x-*` name hints are absent.",
            "title: Plex Media Server"
        ),
    ),
    (
        "summary",
        kw!(
            "summary",
            "OpenAPI",
            "3.0+",
            "String",
            "One-line short form, alongside `description`'s long form. Present on operations, links, examples, and (3.1) info.",
            "Keep it a single sentence — it becomes the one-liner in generated method docs and reference lists.",
            "summary: Rates a media item"
        ),
    ),
    (
        "version",
        kw!(
            "version",
            "OpenAPI",
            "2.0 · 3.x",
            "String (semver recommended)",
            "The API document's version inside `info`. This is the document version, not the implementation's.",
            "Required. Keep it semver for changelog tooling.",
            "version: 1.1.1"
        ),
    ),
    (
        "termsOfService",
        kw!(
            "termsOfService",
            "OpenAPI",
            "2.0 · 3.x",
            "URI string",
            "Link to the API's terms of service, inside `info`.",
            "Rendered by docs generators in the API header block.",
            "termsOfService: https://example.com/terms"
        ),
    ),
    (
        "contact",
        kw!(
            "contact",
            "OpenAPI",
            "2.0 · 3.x",
            "Contact Object",
            "Maintainer contact information inside `info`: `name`, `email`, `url`.",
            "Rendered in docs headers; also feeds telemetry attribution in some generators.",
            "contact:\n  name: API Support\n  email: api@example.com"
        ),
    ),
    (
        "license",
        kw!(
            "license",
            "OpenAPI",
            "2.0 · 3.x",
            "License Object",
            "The API's license: `name` plus either `url` (2.0/3.0) or an SPDX `identifier` (3.1).",
            "3.1 prefers SPDX identifiers. A missing `url` surfaces as `oas-license-missing-url`.",
            "license:\n  name: Apache 2.0\n  identifier: Apache-2.0"
        ),
    ),
    (
        "email",
        kw!(
            "email",
            "OpenAPI",
            "2.0 · 3.x",
            "String (email address)",
            "Contact email inside the Contact Object.",
            "The LSP turns this into a `mailto:` document link.",
            "email: api@example.com"
        ),
    ),
    // -- OAS server objects -----------------------------------------------------
    (
        "url",
        kw!(
            "url",
            "OpenAPI",
            "2.0 (as host/basePath) · 3.x",
            "URI reference, optionally with `{variable}` templates",
            "The server's base URL. `{variables}` in the URL must be declared in the server's `variables` map.",
            "Absolute HTTPS URLs are the portable form for codegen; relative URLs resolve against the document location.",
            "url: https://{host}:{port}/v1"
        ),
    ),
    (
        "variables",
        kw!(
            "variables",
            "OpenAPI",
            "3.x",
            "Map of names to Server Variable Objects",
            "Substitution values for `{templates}` in a server URL. Each variable declares an optional `enum`, a required `default`, and a `description`.",
            "An undeclared URL variable surfaces as `oas-server-variable-unknown`; a default outside the enum as `oas-server-variable-default-invalid`.",
            "variables:\n  port:\n    enum: ['32400', '32401']\n    default: '32400'"
        ),
    ),
    // -- OAS operation and parameter objects ------------------------------------
    (
        "in",
        kw!(
            "in",
            "OpenAPI",
            "2.0 · 3.x",
            "One of `query`, `header`, `path`, `cookie` (2.0 adds `formData`, `body`)",
            "The parameter's wire location. `path` parameters are always required and must appear in the path template.",
            "Missing `in` surfaces as `oas-parameter-missing-in`. A `path` parameter not in the template is `oas-parameter-not-in-template`.",
            "in: query"
        ),
    ),
    (
        "allowEmptyValue",
        kw!(
            "allowEmptyValue",
            "OpenAPI",
            "3.0+",
            "Boolean (default false)",
            "Permits a query parameter to be sent with an empty value (`?flag=`).",
            "Deprecated in spirit: the OAS committee recommends modeling it with an empty-string enum instead. Query parameters only.",
            "in: query\nallowEmptyValue: true"
        ),
    ),
    (
        "style",
        kw!(
            "style",
            "OpenAPI",
            "3.0+",
            "Serialization style enum",
            "How arrays/objects serialize into the wire location: `form` (query/cookie), `simple` (path/header), `spaceDelimited`, `pipeDelimited`, `deepObject` (query).",
            "Pairs with `explode`. The default depends on the location — `form` for query/cookie, `simple` for path/header.",
            "in: query\nstyle: deepObject\nexplode: true"
        ),
    ),
    (
        "explode",
        kw!(
            "explode",
            "OpenAPI",
            "3.0+",
            "Boolean",
            "Whether arrays/objects generate a separate parameter per element (`?a=1&a=2`) instead of one joined value (`?a=1,2`).",
            "Defaults to true for `form` style, false for `simple`. Wrong explode settings are a top source of client/server mismatches.",
            "style: form\nexplode: true"
        ),
    ),
    (
        "allowReserved",
        kw!(
            "allowReserved",
            "OpenAPI",
            "3.0+",
            "Boolean (default false)",
            "Permits reserved characters in this parameter's value without percent-encoding.",
            "Query parameters only. Codegen emits a raw-encoding code path when true.",
            "allowReserved: true"
        ),
    ),
    (
        "schema",
        kw!(
            "schema",
            "OpenAPI",
            "3.x",
            "Schema Object or reference",
            "The value's shape for non-body parameters (query, header, path, cookie).",
            "Parameters use `schema` OR `content` — never both. Body payloads live in `requestBody.content` instead.",
            "schema:\n  type: string\n  minLength: 1"
        ),
    ),
    (
        "content",
        kw!(
            "content",
            "OpenAPI",
            "3.x",
            "Map of media types to Media Type Objects",
            "Media-type-keyed payload description. On parameters it is the 3.1 alternative to `schema`; on responses and request bodies it is the only payload slot.",
            "Each key is an RFC 2046 media type (`application/json`, `text/event-stream`, `multipart/form-data`).",
            "content:\n  application/json:\n    schema:\n      $ref: '#/components/schemas/Pet'"
        ),
    ),
    (
        "requestBody",
        kw!(
            "requestBody",
            "OpenAPI",
            "3.0+",
            "Request Body Object or reference",
            "The operation's payload: media-type-keyed content plus `required` and `description`.",
            "GET/DELETE operations carrying one surface as `http-request-body-method` in admission. 2.0 used body/formData parameters instead.",
            "requestBody:\n  required: true\n  content:\n    application/json:\n      schema: ..."
        ),
    ),
    (
        "responses",
        kw!(
            "responses",
            "OpenAPI",
            "2.0 · 3.x",
            "Responses Object (required)",
            "Status-code-keyed responses: exact codes, ranges (`'2XX'`), and `default`. Each needs a `description`.",
            "The one required operation field. No `default`/4XX/5XX surfaces as `oas-operation-no-error-response` — client error handling has nothing to bind to.",
            "responses:\n  '200': {description: ok}\n  default: {description: err}"
        ),
    ),
    (
        "callbacks",
        kw!(
            "callbacks",
            "OpenAPI",
            "3.0+",
            "Map of names to Callback Objects",
            "Out-of-band requests the API may invoke, keyed by name; each value is a Path Item keyed by a runtime expression (`{$request.body#/url}`).",
            "Callback payloads compile into incoming-webhook decoders; broken declarations fail admission like any other diagnostic.",
            "callbacks:\n  petCreated:\n    '{$request.body#/callbackUrl}':\n      post: ..."
        ),
    ),
    (
        "links",
        kw!(
            "links",
            "OpenAPI",
            "3.0+",
            "Map of names to Link Objects",
            "Declares follow-up relationships from a response: the linked operation plus how to derive its parameters from this response.",
            "`operationId` picks the target; runtime expressions (`$response.body#/id`) derive inputs. The LSP links these to the target operation.",
            "links:\n  address:\n    operationId: getAddress\n    parameters:\n      userId: '$response.body#/id'"
        ),
    ),
    (
        "encoding",
        kw!(
            "encoding",
            "OpenAPI",
            "3.0+",
            "Map of property names to Encoding Objects",
            "Per-property serialization for multipart and form-urlencoded bodies: contentType, headers, style, explode.",
            "Property names must exist in the body schema. Encoding of JSON parts defaults to `application/json` regardless of declared type.",
            "encoding:\n  logo:\n    contentType: image/png"
        ),
    ),
    (
        "contentType",
        kw!(
            "contentType",
            "OpenAPI",
            "3.0+",
            "Media type string",
            "Overrides the part's content type inside an Encoding Object.",
            "Ignored when the part's schema is `type: string` with `contentEncoding` — those are raw.",
            "contentType: application/xml"
        ),
    ),
    (
        "headers",
        kw!(
            "headers",
            "OpenAPI",
            "3.x",
            "Map of names to Header Objects",
            "Response or encoding headers. Header Objects are parameters with `name`/`in` omitted and `in: header` implied.",
            "Content-Type is not a header here — it comes from the content map key.",
            "headers:\n  X-Rate-Limit:\n    schema: {type: integer}"
        ),
    ),
    // -- OAS security objects -----------------------------------------------------
    (
        "scheme",
        kw!(
            "scheme",
            "OpenAPI",
            "3.0+",
            "HTTP Authentication Scheme name",
            "The HTTP auth scheme for `type: http` security schemes: `bearer`, `basic`, `digest`, or any RFC 7235 registry name.",
            "Only meaningful with `type: http`. `bearer` pairs with `bearerFormat`.",
            "type: http\nscheme: bearer"
        ),
    ),
    (
        "bearerFormat",
        kw!(
            "bearerFormat",
            "OpenAPI",
            "3.0+",
            "String",
            "A hint about the bearer token's format (`JWT` is the common case).",
            "Advisory only — tooling uses it for docs and token handling hints, not validation.",
            "type: http\nscheme: bearer\nbearerFormat: JWT"
        ),
    ),
    (
        "flows",
        kw!(
            "flows",
            "OpenAPI",
            "3.0+",
            "OAuth Flows Object",
            "The OAuth 2.0 flows this scheme supports: `implicit`, `password`, `clientCredentials`, `authorizationCode` — each with authorization/token/refresh URLs and scopes.",
            "Generated OAuth runtimes bind to these URLs; `scopes` must be declared even when empty.",
            "flows:\n  clientCredentials:\n    tokenUrl: https://auth.example.com/token\n    scopes: {}"
        ),
    ),
    (
        "openIdConnectUrl",
        kw!(
            "openIdConnectUrl",
            "OpenAPI",
            "3.0+",
            "URI reference",
            "An OpenID Connect Discovery URL describing the scheme's configuration.",
            "Required (and only used) when `type: openIdConnect`. Generated clients fetch discovery from here.",
            "type: openIdConnect\nopenIdConnectUrl: https://auth.example.com/.well-known/openid-configuration"
        ),
    ),
    (
        "scopes",
        kw!(
            "scopes",
            "OpenAPI",
            "2.0 · 3.x",
            "Map of scope names to descriptions",
            "The scopes an OAuth flow or requirement references. Inside a security requirement the list is which of them the operation needs.",
            "Empty scopes maps are normal for non-OAuth or unrestricted flows.",
            "scopes:\n  read: Read access\n  write: Write access"
        ),
    ),
    (
        "authorizationUrl",
        kw!(
            "authorizationUrl",
            "OpenAPI",
            "3.0+",
            "URI string",
            "The OAuth authorization endpoint for `implicit` and `authorizationCode` flows.",
            "Must be absolute. Not used by `clientCredentials`/`password` flows.",
            "authorizationUrl: https://auth.example.com/authorize"
        ),
    ),
    (
        "tokenUrl",
        kw!(
            "tokenUrl",
            "OpenAPI",
            "3.0+",
            "URI string",
            "The OAuth token endpoint for `clientCredentials`, `password`, and `authorizationCode` flows.",
            "Generated client-credential runtimes POST here; required for those flows.",
            "tokenUrl: https://auth.example.com/token"
        ),
    ),
    (
        "refreshUrl",
        kw!(
            "refreshUrl",
            "OpenAPI",
            "3.0+",
            "URI string",
            "The endpoint used to refresh tokens, usable across flows.",
            "Optional; when absent clients re-run the original flow to renew.",
            "refreshUrl: https://auth.example.com/refresh"
        ),
    ),
    // -- OAS discriminator, XML, misc ---------------------------------------------
    (
        "propertyName",
        kw!(
            "propertyName",
            "OpenAPI",
            "3.0+",
            "String",
            "The payload field whose value selects the branch of a `oneOf`/`anyOf`. Required on a Discriminator Object.",
            "Branch schemas should constrain the property (`const` or `enum`) so clients can switch on it. Missing here surfaces as `oas-discriminator-missing-property`.",
            "discriminator:\n  propertyName: kind"
        ),
    ),
    (
        "namespace",
        kw!(
            "namespace",
            "OpenAPI",
            "3.0+",
            "URI string",
            "The XML namespace for the element described by the XML Object.",
            "Only applies when the API is serialized as XML (PMS-style specs).",
            "namespace: https://example.com/schema"
        ),
    ),
    (
        "prefix",
        kw!(
            "prefix",
            "OpenAPI",
            "3.0+",
            "String",
            "The namespace prefix for the XML element.",
            "Requires `namespace` to be meaningful in the wire document.",
            "prefix: plex"
        ),
    ),
    (
        "attribute",
        kw!(
            "attribute",
            "OpenAPI",
            "3.0+",
            "Boolean (default false)",
            "Serializes this property as an XML attribute rather than a child element.",
            "Pairs with `wrapped` for arrays. PMS-style XML specs lean on this heavily.",
            "attribute: true"
        ),
    ),
    (
        "wrapped",
        kw!(
            "wrapped",
            "OpenAPI",
            "3.0+",
            "Boolean (default false)",
            "Wraps an array in an outer XML element named after the property.",
            "Only meaningful for array properties with `attribute: false`.",
            "wrapped: true"
        ),
    ),
    // -- JSON Schema 2020-12: core vocabulary --------------------------------------
    (
        "$schema",
        kw!(
            "$schema",
            "JSON Schema",
            "2020-12",
            "URI",
            "Declares the meta-schema this schema conforms to; also anchors dialect interpretation per-subtree.",
            "In OAS documents it overrides `jsonSchemaDialect` for the subtree. Rarely needed inside OAS components.",
            "$schema: https://json-schema.org/draft/2020-12/schema"
        ),
    ),
    (
        "$vocabulary",
        kw!(
            "$vocabulary",
            "JSON Schema",
            "2020-12",
            "Map of vocabulary URIs to booleans",
            "Declares which vocabularies a meta-schema requires (true) or optionally allows (false).",
            "Meta-schema authoring only — instances never carry it.",
            "$vocabulary:\n  https://json-schema.org/draft/2020-12/vocab/core: true"
        ),
    ),
    (
        "$id",
        kw!(
            "$id",
            "JSON Schema",
            "2020-12",
            "URI reference",
            "Sets this schema's canonical base URI: `$ref` targets inside resolve relative to it, and the URI itself names the schema.",
            "Changes relative-reference resolution for the whole subtree — moving a schema with `$id` can silently retarget refs.",
            "$id: https://example.com/schemas/pet.json"
        ),
    ),
    (
        "$anchor",
        kw!(
            "$anchor",
            "JSON Schema",
            "2020-12",
            "Fragment-safe identifier",
            "Names this schema location for plain-name fragment refs (`\"$ref\": \"#name\"`).",
            "Plain-name targeting; the structured alternative is a JSON Pointer ref.",
            "$anchor: PetAddress"
        ),
    ),
    (
        "$dynamicAnchor",
        kw!(
            "$dynamicAnchor",
            "JSON Schema",
            "2020-12",
            "Identifier",
            "A dynamic anchor: `$dynamicRef` walks to the outermost dynamic anchor with this name in scope, enabling recursive-schema extension.",
            "The mechanism behind extensible recursive schemas — JSON Schema's own meta-schemas use it.",
            "$dynamicAnchor: node"
        ),
    ),
    (
        "$dynamicRef",
        kw!(
            "$dynamicRef",
            "JSON Schema",
            "2020-12",
            "URI reference with a dynamic-anchor fragment",
            "Like `$ref` to a dynamic anchor, but resolution continues outward until no more dynamic anchors shadow it.",
            "Pairs only with `$dynamicAnchor`; plain `$ref` never triggers dynamic resolution.",
            "$dynamicRef: '#node'"
        ),
    ),
    (
        "$defs",
        kw!(
            "$defs",
            "JSON Schema",
            "2020-12",
            "Map of names to Schemas",
            "The 2020-12 home for standalone schema definitions, referenced via `$ref: '#/$defs/Name'`.",
            "OAS 3.0 called this `definitions`. Dialect checks flag `definitions` under 3.1 documents.",
            "$defs:\n  Address: {type: object}"
        ),
    ),
    (
        "$comment",
        kw!(
            "$comment",
            "JSON Schema",
            "2020-12",
            "String",
            "A note for schema maintainers. Not for end users — that is `title`/`description` — and never affects validation.",
            "Interpreted by tooling as comments: safe to strip in published artifacts.",
            "$comment: kept in sync with the billing team's enum"
        ),
    ),
    // -- JSON Schema 2020-12: applicators ------------------------------------------
    (
        "not",
        kw!(
            "not",
            "JSON Schema",
            "3.x · 2020-12",
            "Schema",
            "The value must NOT satisfy this subschema — logical negation.",
            "Powerful but hard for codegen: `not` produces no positive shape information, so generated types degrade to unmodeled.",
            "not: {type: string}"
        ),
    ),
    (
        "if",
        kw!(
            "if",
            "JSON Schema",
            "3.1 · 2020-12 (not in 3.0)",
            "Schema",
            "Conditional applicator: if the value satisfies `if`, then `then` applies; otherwise `else` applies. `then`/`else` are optional.",
            "The three keys only work as a set — `if` alone contributes no assertions of its own. Not in 3.0.",
            "if:\n  properties: {kind: {const: dog}}\nthen:\n  required: [barkVolume]"
        ),
    ),
    (
        "then",
        kw!(
            "then",
            "JSON Schema",
            "3.1 · 2020-12",
            "Schema",
            "The schema applied when the sibling `if` succeeds.",
            "Meaningless without an `if` sibling in the same object.",
            "then:\n  required: [barkVolume]"
        ),
    ),
    (
        "else",
        kw!(
            "else",
            "JSON Schema",
            "3.1 · 2020-12",
            "Schema",
            "The schema applied when the sibling `if` fails.",
            "Meaningless without an `if` sibling in the same object.",
            "else:\n  required: [meowVolume]"
        ),
    ),
    (
        "contains",
        kw!(
            "contains",
            "JSON Schema",
            "3.1 · 2020-12",
            "Schema",
            "At least one array element must satisfy this schema.",
            "`minContains`/`maxContains` bound how many elements must match (2020-12). Not in 3.0.",
            "contains:\n  const: plex"
        ),
    ),
    (
        "patternProperties",
        kw!(
            "patternProperties",
            "JSON Schema",
            "3.x · 2020-12",
            "Map of regex to Schema",
            "Property names matching a regex are validated by that regex's schema.",
            "Composes with `properties` (both apply) and `additionalProperties` (which catches the rest).",
            "patternProperties:\n  '^x-': {type: string}"
        ),
    ),
    (
        "propertyNames",
        kw!(
            "propertyNames",
            "JSON Schema",
            "3.1 · 2020-12",
            "Schema",
            "Every property name (as a string) must satisfy this schema — typically `pattern` or `maxLength`.",
            "The canonical map-key constraint.",
            "propertyNames:\n  pattern: '^[a-z][a-zA-Z0-9]*$'"
        ),
    ),
    (
        "unevaluatedItems",
        kw!(
            "unevaluatedItems",
            "JSON Schema",
            "3.1 · 2020-12",
            "Boolean or Schema",
            "Like `additionalProperties` for arrays, but it considers items already validated by ANY sibling (allOf branches included).",
            "The evaluation-aware array constraint; use it instead of `additionalProperties` inside compositions.",
            "unevaluatedItems: false"
        ),
    ),
    (
        "unevaluatedProperties",
        kw!(
            "unevaluatedProperties",
            "JSON Schema",
            "3.1 · 2020-12",
            "Boolean or Schema",
            "Like `additionalProperties`, but evaluation-aware: properties validated anywhere in the composition (allOf parents included) count as evaluated.",
            "The 2020-12 answer to inheritance-plus-closure — allOf a base schema then `unevaluatedProperties: false` without breaking it.",
            "unevaluatedProperties: false"
        ),
    ),
    (
        "dependentSchemas",
        kw!(
            "dependentSchemas",
            "JSON Schema",
            "3.1 · 2020-12",
            "Map of property names to Schemas",
            "When the named property is present, the whole value must also satisfy the mapped schema.",
            "For conditional shape requirements (if `card` present, also require `billingAddress`).",
            "dependentSchemas:\n  card:\n    required: [billingAddress]"
        ),
    ),
    (
        "dependentRequired",
        kw!(
            "dependentRequired",
            "JSON Schema",
            "3.1 · 2020-12",
            "Map of property names to arrays of names",
            "When the named property is present, the listed properties must also be present.",
            "The lightweight form of `dependentSchemas` for pure-required conditions.",
            "dependentRequired:\n  creditCard: [billingAddress]"
        ),
    ),
    // -- JSON Schema 2020-12: validation -------------------------------------------
    (
        "exclusiveMaximum",
        kw!(
            "exclusiveMaximum",
            "JSON Schema",
            "3.0: boolean · 2020-12: number",
            "Boolean (3.0) or number (2020-12)",
            "3.0: modifies `maximum` to be exclusive. 2020-12: the exclusive upper bound itself.",
            "The two dialects are incompatible — moving between them requires rewriting the keyword.",
            "maximum: 100\nexclusiveMaximum: true   # 3.0\n# or, 2020-12:\nexclusiveMaximum: 100"
        ),
    ),
    (
        "maxItems",
        kw!(
            "maxItems",
            "JSON Schema",
            "2.0 · 3.x · 2020-12",
            "Non-negative integer",
            "Maximum number of elements in an array.",
            "Applies only to arrays. Codegen maps this to collection capacity hints.",
            "type: array\nmaxItems: 100"
        ),
    ),
    (
        "minItems",
        kw!(
            "minItems",
            "JSON Schema",
            "2.0 · 3.x · 2020-12",
            "Non-negative integer (default 0)",
            "Minimum number of elements in an array.",
            "An empty array is invalid when minItems >= 1 — a common example-validation false alarm.",
            "type: array\nminItems: 1"
        ),
    ),
    (
        "uniqueItems",
        kw!(
            "uniqueItems",
            "JSON Schema",
            "2.0 · 3.x · 2020-12",
            "Boolean (default false)",
            "All array elements must be distinct (deep equality).",
            "Codegen emits set-like or O(n^2) distinct checks depending on element type.",
            "type: array\nuniqueItems: true"
        ),
    ),
    (
        "maxContains",
        kw!(
            "maxContains",
            "JSON Schema",
            "2020-12",
            "Non-negative integer",
            "Upper bound on how many array elements may match `contains`.",
            "Requires a sibling `contains`; meaningless alone.",
            "contains: {const: plex}\nmaxContains: 2"
        ),
    ),
    (
        "minContains",
        kw!(
            "minContains",
            "JSON Schema",
            "2020-12",
            "Non-negative integer (default 1)",
            "Lower bound on how many array elements must match `contains`. `minContains: 0` makes `contains` optional.",
            "Requires a sibling `contains`.",
            "contains: {const: plex}\nminContains: 2"
        ),
    ),
    (
        "maxProperties",
        kw!(
            "maxProperties",
            "JSON Schema",
            "3.x · 2020-12",
            "Non-negative integer",
            "Maximum number of properties an object may have.",
            "Counts ALL keys (evaluated or not), not just declared ones.",
            "type: object\nmaxProperties: 16"
        ),
    ),
    (
        "minProperties",
        kw!(
            "minProperties",
            "JSON Schema",
            "3.x · 2020-12",
            "Non-negative integer",
            "Minimum number of properties an object must have.",
            "Useful for free-form maps where `required` cannot name keys ahead of time.",
            "type: object\nminProperties: 1"
        ),
    ),
    // -- JSON Schema 2020-12: basic and content ------------------------------------
    (
        "default",
        kw!(
            "default",
            "JSON Schema",
            "2.0 · 3.x · 2020-12",
            "Any JSON value",
            "The value used when none is supplied. An annotation — it never fails validation by itself.",
            "Should satisfy the schema's own assertions; the instance checker flags ones that don't (`oas-schema-instance-invalid`).",
            "type: integer\ndefault: 32400"
        ),
    ),
    (
        "readOnly",
        kw!(
            "readOnly",
            "OpenAPI",
            "3.x",
            "Boolean (default false)",
            "The property is server-computed: clients may read it but sending it is undefined behavior.",
            "In 3.0, readOnly + writeOnly on the same property is an error; in 3.1 the combination just means different visibility per direction.",
            "readOnly: true"
        ),
    ),
    (
        "writeOnly",
        kw!(
            "writeOnly",
            "OpenAPI",
            "3.x",
            "Boolean (default false)",
            "The property is client-settable but never returned in responses (passwords, tokens).",
            "Docs generators omit writeOnly fields from response examples.",
            "writeOnly: true"
        ),
    ),
    (
        "examples",
        kw!(
            "examples",
            "JSON Schema",
            "3.1 · 2020-12",
            "Array of values",
            "Representative values satisfying the schema. In 3.0 this slot was any-value (often a map); in 3.1 it is strictly an array.",
            "Each entry should pass the schema — the instance checker validates them. For named examples use the `examples` component map.",
            "examples:\n  - 32400\n  - 8080"
        ),
    ),
    (
        "contentEncoding",
        kw!(
            "contentEncoding",
            "JSON Schema",
            "3.1 · 2020-12",
            "Encoding name (`base64`, `quoted-printable`, ...)",
            "Declares the string's content encoding — `base64` strings carrying binary payloads.",
            "Replaces 3.0's `format: byte`. Codegen maps base64 to byte-array types.",
            "type: string\ncontentEncoding: base64"
        ),
    ),
    (
        "contentMediaType",
        kw!(
            "contentMediaType",
            "JSON Schema",
            "3.1 · 2020-12",
            "Media type string",
            "Declares the media type of a string's content (`application/xml` inside a string field).",
            "Pairs with `contentSchema` (3.1) to validate the decoded content as JSON.",
            "contentMediaType: application/xml"
        ),
    ),
    (
        "contentSchema",
        kw!(
            "contentSchema",
            "JSON Schema",
            "3.1 · 2020-12",
            "Schema",
            "When the string's media type is JSON, the decoded content must satisfy this schema.",
            "Only applies when `contentMediaType` declares a JSON type. The deepest level of the string-in-json pattern.",
            "contentMediaType: application/json\ncontentSchema:\n  type: object"
        ),
    ),
    // -- components section names -------------------------------------------------

    // -- components section names (hovering the section key itself) -----------------
    (
        "schemas",
        kw!(
            "schemas",
            "OpenAPI",
            "3.0+",
            "Map of names to Schemas",
            "The components section for reusable data models, referenced via `$ref: '#/components/schemas/<name>'`.",
            "Entries unreferenced from paths are legal but skip closure-based analysis. Keys sort alphabetically under the canonical formatter.",
            "schemas:\n  Pet:\n    type: object"
        ),
    ),
    (
        "parameters",
        kw!(
            "parameters",
            "OpenAPI",
            "3.0+",
            "Map of names to Parameter Objects",
            "The components section for reusable parameters — typically pagination, auth headers, and common path parameters.",
            "Referenced via `$ref: '#/components/parameters/<name>'`; operation-level entries override path-item ones by (name, in).",
            "parameters:\n  Limit:\n    name: limit\n    in: query"
        ),
    ),
    (
        "requestBodies",
        kw!(
            "requestBodies",
            "OpenAPI",
            "3.0+",
            "Map of names to Request Body Objects",
            "The components section for reusable request payloads.",
            "Rarely used — request bodies differ per endpoint — but correct for highly uniform APIs.",
            "requestBodies:\n  PetBody:\n    content:\n      application/json: ..."
        ),
    ),
    (
        "responses",
        kw!(
            "responses",
            "OpenAPI",
            "3.0+",
            "Map of names to Response Objects",
            "The components section for reusable responses — usually global error shapes (`400`, `401`, `500`).",
            "Referenced from operation responses; keeps error payloads consistent across endpoints.",
            "responses:\n  NotFound:\n    description: Not found\n    content: ..."
        ),
    ),
    (
        "securitySchemes",
        kw!(
            "securitySchemes",
            "OpenAPI",
            "3.0+",
            "Map of names to Security Scheme Objects",
            "The components section declaring auth mechanisms. Security requirements reference these by name.",
            "An undeclared name in a requirement surfaces as `oas-security-unknown-scheme`. The LSP quick-fix declares missing schemes here.",
            "securitySchemes:\n  ApiKeyAuth:\n    type: http\n    scheme: bearer"
        ),
    ),
    (
        "pathItems",
        kw!(
            "pathItems",
            "OpenAPI",
            "3.1+",
            "Map of names to Path Items",
            "The components section for reusable path-item bundles — shared endpoint groups referenced by `paths` entries or webhooks via `$ref`.",
            "3.1 only. Lets multiple servers or versions mount the same endpoint group.",
            "pathItems:\n  Media:\n    get: ..."
        ),
    ),
    // -- Swagger 2.0-only and OpenAPI 3.2-only keywords -----------------------------

    // -- Swagger 2.0-only keywords ---------------------------------------------------
    (
        "host",
        kw!(
            "host",
            "Swagger 2.0",
            "2.0 only",
            "Host string, optionally with port",
            "The API host, applied to all paths (3.x replaced this with `servers`).",
            "Mutually exclusive with the 3.x `servers` array. No scheme — that comes from `schemes`.",
            "host: plex.example.com\nbasePath: /"
        ),
    ),
    (
        "basePath",
        kw!(
            "basePath",
            "Swagger 2.0",
            "2.0 only",
            "Path prefix string",
            "The root path all paths are relative to (3.x folded this into each server URL).",
            "Concatenated as `{scheme}://{host}{basePath}/{path}`. A trailing slash in basePath duplicates in client URLs.",
            "basePath: /api/v1"
        ),
    ),
    (
        "schemes",
        kw!(
            "schemes",
            "Swagger 2.0",
            "2.0 only (3.0 moved to servers)",
            "Array of `http`, `https`, `ws`, `wss`",
            "The transfer protocols the API supports — 2.0's per-host equivalent of 3.x server URLs.",
            "Repeated at operation level to override. 3.x models protocol in each server `url` instead.",
            "schemes: [https]"
        ),
    ),
    (
        "consumes",
        kw!(
            "consumes",
            "Swagger 2.0",
            "2.0 only",
            "Array of media type strings",
            "Global default for request payload media types — 2.0's counterpart of 3.x requestBody content keys.",
            "Operation-level `consumes` fully replaces the global list. `formData` parameters use it to pick the multipart boundary type.",
            "consumes: [application/json]"
        ),
    ),
    (
        "produces",
        kw!(
            "produces",
            "Swagger 2.0",
            "2.0 only",
            "Array of media type strings",
            "Global default for response payload media types — 2.0's counterpart of 3.x response `content` keys.",
            "Operation-level `produces` fully replaces the global list. Generated clients use it for response decoding.",
            "produces: [application/json, application/xml]"
        ),
    ),
    (
        "definitions",
        kw!(
            "definitions",
            "Swagger 2.0",
            "2.0 only",
            "Map of names to Schemas",
            "2.0's reusable schema section — renamed `components/schemas` in 3.x.",
            "Referenced via `$ref: '#/definitions/<name>'`. Dialect checks flag `definitions` under 3.1 documents.",
            "definitions:\n  Pet:\n    type: object"
        ),
    ),
    (
        "securityDefinitions",
        kw!(
            "securityDefinitions",
            "Swagger 2.0",
            "2.0 only",
            "Map of names to Security Scheme Objects",
            "2.0's `components/securitySchemes`. Security requirements reference these by name.",
            "2.0 scheme types add `basic` and `apiKey` spellings; `flow` (singular) replaces 3.x's `flows` map.",
            "securityDefinitions:\n  ApiKeyAuth:\n    type: apiKey\n    in: header\n    name: X-Api-Key"
        ),
    ),
    (
        "collectionFormat",
        kw!(
            "collectionFormat",
            "Swagger 2.0",
            "2.0 only",
            "One of `csv`, `ssv`, `tsv`, `pipes`, `multi`",
            "How array parameters serialize: `csv` (comma-joined, the default), `ssv`/`tsv`/`pipes` (joined by space/tab/pipe), `multi` (repeated parameter).",
            "3.x replaced this with `style` + `explode` — `csv` maps to `form, explode: false`, `multi` to `form, explode: true`.",
            "type: array\nitems: {type: string}\ncollectionFormat: multi"
        ),
    ),
    (
        "flow",
        kw!(
            "flow",
            "Swagger 2.0",
            "2.0 only",
            "One of `implicit`, `password`, `application`, `accessCode`",
            "2.0's single-OAuth-flow keyword — 3.x replaced it with the `flows` map (`application` renamed `clientCredentials`, `accessCode` renamed `authorizationCode`).",
            "Sits directly on the Security Scheme, with `authorizationUrl`/`tokenUrl`/`scopes` as siblings.",
            "flow: application\ntokenUrl: https://auth.example.com/token"
        ),
    ),
    // -- OpenAPI 3.2-only keywords ---------------------------------------------------
    (
        "itemSchema",
        kw!(
            "itemSchema",
            "OpenAPI",
            "3.2+",
            "Schema",
            "On a Media Type Object: the schema for each item of a map-like or streamed payload, distinct from the envelope schema. The 3.2 counterpart of 2020-12 `items` for wire maps.",
            "The contract planner treats `itemSchema` as the element shape for positional/encoded parts — required there when encodings are positional.",
            "content:\n  application/json:\n    itemSchema:\n      $ref: '#/components/schemas/Pet'"
        ),
    ),
    (
        "itemEncoding",
        kw!(
            "itemEncoding",
            "OpenAPI",
            "3.2+",
            "Encoding Object or array of them",
            "Per-item encoding for streamed or map-like payloads, the item-level counterpart of `encoding`.",
            "Required (or an array schema) when `prefixEncoding` is used positionally — admission flags the combination.",
            "itemEncoding:\n  contentType: application/json"
        ),
    ),
    (
        "prefixEncoding",
        kw!(
            "prefixEncoding",
            "OpenAPI",
            "3.2+",
            "Array of Encoding Objects",
            "Positional encodings for the first N entries of a streamed payload; `itemEncoding` covers the remainder.",
            "Positional entries require `itemSchema` or an explicit array schema so lengths stay checkable.",
            "prefixEncoding:\n  - contentType: application/json"
        ),
    ),
    (
        "parent",
        kw!(
            "parent",
            "OpenAPI",
            "3.2+",
            "Tag name",
            "Declares a tag hierarchy — this tag renders nested under the named parent in docs and explorers.",
            "Forms a tree; cycles or dangling parents are a docs-rendering hazard. Pair with `kind` for visual grouping.",
            "tags:\n  - name: Library Collections\n    parent: Library"
        ),
    ),
    (
        "kind",
        kw!(
            "kind",
            "OpenAPI",
            "3.2+",
            "Tag kind string",
            "Classifies a tag for rendering (e.g. `nav`, `registry`) so docs tools can group or style tag families.",
            "3.2 addition alongside `parent`; tooling that predates 3.2 ignores both.",
            "tags:\n  - name: Internal\n    kind: registry"
        ),
    ),
];

/// Resolves documentation for a keyword in either vocabulary.
#[must_use]
pub fn lookup(keyword: &str) -> Option<&'static KeywordDoc> {
    let lower = keyword.to_ascii_lowercase();
    OPENAPI_KEYWORDS
        .iter()
        .chain(SCHEMA_KEYWORDS.iter())
        .find(|(name, _)| name.eq_ignore_ascii_case(&lower))
        .map(|(_, doc)| doc)
}

/// Renders one keyword's documentation as hover markdown.
#[must_use]
pub fn hover_markdown(doc: &KeywordDoc) -> String {
    let mut md = format!("**`{}`** — {}\n\n", doc.title, doc.family);
    md.push_str(&format!("*Availability:* {}\n\n", doc.availability));
    md.push_str(&format!("**Value:** {}\n\n", doc.value_domain));
    md.push_str(doc.meaning);
    md.push_str("\n\n");
    if !doc.interactions.is_empty() {
        md.push_str(&format!("> {}\n\n", doc.interactions));
    }
    md.push_str("```yaml\n");
    md.push_str(doc.example);
    md.push_str("\n```");
    md
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_and_openapi_keywords_resolve_case_insensitively() {
        assert!(lookup("minimum").is_some());
        assert!(lookup("Minimum").is_some());
        assert!(lookup("openapi").is_some());
        assert!(lookup("$ref").is_some());
        assert!(lookup("not-a-keyword").is_none());
    }

    #[test]
    fn hover_markdown_has_all_sections() {
        let doc = lookup("oneOf").unwrap();
        let md = hover_markdown(doc);
        assert!(md.contains("**`oneOf`**"));
        assert!(md.contains("*Availability:*"));
        assert!(md.contains("**Value:**"));
        assert!(md.contains("```yaml"));
        assert!(md.contains("EXACTLY ONE"));
    }

    /// The canonical keyword set: every OpenAPI 3.0/3.1 structural keyword
    /// plus the full JSON Schema 2020-12 vocabulary. A keyword missing from
    /// the dictionary fails the test — dictionary completeness is pinned,
    /// not aspirational.
    const CANONICAL: &[&str] = &[
        // OAS root/metadata
        "openapi",
        "swagger",
        "info",
        "jsonSchemaDialect",
        "servers",
        "security",
        "tags",
        "paths",
        "webhooks",
        "components",
        "title",
        "summary",
        "version",
        "description",
        "termsOfService",
        "contact",
        "license",
        "email",
        "url",
        "externalDocs",
        // server
        "variables",
        // operations/params
        "operationId",
        "deprecated",
        "in",
        "allowEmptyValue",
        "style",
        "explode",
        "allowReserved",
        "schema",
        "content",
        "requestBody",
        "responses",
        "callbacks",
        "encoding",
        "contentType",
        "headers",
        "links",
        // components sections
        "schemas",
        "responses",
        "parameters",
        "examples",
        "requestBodies",
        "headers",
        "securitySchemes",
        "links",
        "callbacks",
        "pathItems",
        // security
        "scheme",
        "bearerFormat",
        "flows",
        "openIdConnectUrl",
        "scopes",
        "authorizationUrl",
        "tokenUrl",
        "refreshUrl",
        // discriminator/xml
        "discriminator",
        "propertyName",
        "mapping",
        "namespace",
        "prefix",
        "attribute",
        "wrapped",
        // JSON Schema core
        "$ref",
        "$schema",
        "$vocabulary",
        "$id",
        "$anchor",
        "$dynamicAnchor",
        "$dynamicRef",
        "$defs",
        "$comment",
        // applicators
        "allOf",
        "anyOf",
        "oneOf",
        "not",
        "if",
        "then",
        "else",
        "dependentSchemas",
        "prefixItems",
        "items",
        "contains",
        "properties",
        "patternProperties",
        "additionalProperties",
        "propertyNames",
        "unevaluatedItems",
        "unevaluatedProperties",
        // validation
        "type",
        "enum",
        "const",
        "multipleOf",
        "maximum",
        "exclusiveMaximum",
        "minimum",
        "exclusiveMinimum",
        "maxLength",
        "minLength",
        "pattern",
        "maxItems",
        "minItems",
        "uniqueItems",
        "maxContains",
        "minContains",
        "maxProperties",
        "minProperties",
        "required",
        "dependentRequired",
        // basic
        "title",
        "default",
        "deprecated",
        "readOnly",
        "writeOnly",
        "examples",
        // content + OAS-only
        "format",
        "contentEncoding",
        "contentMediaType",
        "contentSchema",
        "nullable",
    ];

    #[test]
    fn dictionary_covers_the_canonical_keyword_set() {
        let mut missing: Vec<&str> = CANONICAL
            .iter()
            .filter(|k| lookup(k).is_none())
            .copied()
            .collect();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "keywords missing from the hover dictionary: {missing:?}"
        );
    }

    #[test]
    fn swagger2_and_openapi32_keywords_are_covered() {
        // The two versions with version-specific vocabularies beyond the
        // shared 3.x core: every version-specific keyword must resolve.
        const SWAGGER2: &[&str] = &[
            "swagger",
            "host",
            "basePath",
            "schemes",
            "consumes",
            "produces",
            "definitions",
            "securityDefinitions",
            "collectionFormat",
            "flow",
        ];
        const OPENAPI32: &[&str] = &[
            "itemSchema",
            "itemEncoding",
            "prefixEncoding",
            "parent",
            "kind",
        ];
        for keyword in SWAGGER2.iter().chain(OPENAPI32.iter()) {
            assert!(
                lookup(keyword).is_some(),
                "{keyword}: version-specific keyword missing from the dictionary"
            );
        }
    }

    #[test]
    fn every_documented_keyword_has_complete_metadata() {
        for (name, doc) in OPENAPI_KEYWORDS.iter().chain(SCHEMA_KEYWORDS.iter()) {
            assert!(!doc.meaning.is_empty(), "{name}: empty meaning");
            assert!(!doc.value_domain.is_empty(), "{name}: empty value domain");
            assert!(!doc.availability.is_empty(), "{name}: empty availability");
            assert!(!doc.example.is_empty(), "{name}: empty example");
        }
    }
}
