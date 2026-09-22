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
}
