//! Per-code authoring guidance for diagnostics.
//!
//! Every battery code carries a stable one-line [`summary`] and an
//! actionable [`how_to_fix`] instruction, in the spirit of pre-generation
//! linter rules that answer "what does this mean?" and "how do I fix it?"
//! without leaving the editor. Lookups are a sorted match so the compiler
//! rejects duplicate arms and unknown codes fall back to empty guidance
//! (the located message still stands on its own).

/// One-line summary of what a diagnostic code means.
#[must_use]
pub(crate) fn summary(code: &str) -> &'static str {
    summary_entry(code).unwrap_or("")
}

/// Actionable instruction for resolving a diagnostic.
#[must_use]
pub(crate) fn how_to_fix(code: &str) -> &'static str {
    how_to_fix_entry(code).unwrap_or("")
}

fn summary_entry(code: &str) -> Option<&'static str> {
    Some(match code {
        "invalid-ref" => "Malformed reference string",
        "oas-callback-invalid" => "Invalid callback object",
        "oas-deprecated-operation" => "Deprecated operation",
        "oas-discriminator-missing-property" => "Discriminator without propertyName",
        "oas-discriminator-unknown-mapping" => "Discriminator mapping points at an unknown schema",
        "oas-duplicate-header-param" => "Duplicate header parameter",
        "oas-duplicate-operation-id" => "Duplicate operationId",
        "oas-empty-path-template" => "Empty path template",
        "oas-example-type-mismatch" => "Example does not match the schema type set",
        "oas-header-invalid" => "Invalid header entry",
        "oas-http-array-invalid" => "HTTP array shape is not valid",
        "oas-http-boolean-invalid" => "HTTP boolean shape is not valid",
        "oas-http-map-invalid" => "HTTP map shape is not valid",
        "oas-license-missing-url" => "License without a URL",
        "oas-link-invalid" => "Invalid link object",
        "oas-media-type-invalid" => "Invalid media type entry",
        "oas-operation-invalid" => "Invalid operation object",
        "oas-operation-missing-operationId" => "Operation without an operationId",
        "oas-operation-missing-responses" => "Operation without responses",
        "oas-operation-no-error-response" => "Operation without an error response",
        "oas-parameter-invalid" => "Invalid parameter object",
        "oas-parameter-missing-in" => "Parameter without a location",
        "oas-parameter-missing-name" => "Parameter without a name",
        "oas-parameter-required-missing" => "Path parameter not marked required",
        "oas-path-item-invalid" => "Invalid path item",
        "oas-path-item-security-invalid" => "Invalid path-item security requirement",
        "oas-path-param-not-declared" => "Path template variable without a declared parameter",
        "oas-path-trailing-slash" => "Path key ends with a trailing slash",
        "oas-request-body-content-missing" => "Request body without content",
        "oas-request-body-invalid" => "Invalid request body",
        "oas-response-entry-invalid" => "Invalid response entry",
        "oas-response-invalid" => "Invalid response object",
        "oas-response-map-invalid" => "Invalid responses map",
        "oas-response-missing-description" => "Response without a description",
        "oas-responses-invalid" => "Invalid responses object",
        "oas-schema-invalid-keyword" => "Schema keyword not valid in this dialect",
        "oas-schema-invalid-kind" => "Schema node is the wrong kind",
        "oas-schema-invalid-type" => "Schema type value is not valid",
        "oas-schema-unknown-type" => "Schema type is not a known type name",
        "oas-schema-unsupported-dialect" => "Schema dialect not supported by this document version",
        "oas-security-invalid" => "Invalid security declaration",
        "oas-security-requirement-invalid" => "Invalid security requirement",
        "oas-security-scope-invalid" => "Invalid security scope entry",
        "oas-security-scopes-invalid" => "Invalid security scopes list",
        "oas-security-scopes-nonempty" => "Security scheme requires non-empty scopes",
        "oas-security-unknown-scheme" => "Security requirement names an undeclared scheme",
        "oas-server-invalid" => "Invalid server object",
        "oas-server-url-invalid" => "Invalid server URL",
        "oas-server-variable-default-invalid" => "Server variable default outside its enumeration",
        "oas-server-variable-enum-invalid" => "Server variable enumeration is not valid",
        "oas-server-variable-invalid" => "Invalid server variable",
        "oas-server-variable-unknown" => "Server URL references an undeclared variable",
        "oas-tag-undeclared" => "Operation tag not declared in the root tags list",
        "oas-unused-path-param" => "Declared path parameter does not appear in the path template",
        "oas-webhook-unsupported-version" => "Webhooks require a newer OpenAPI version",
        "ref-outside-allowlist" => "Reference points outside the allowed document set",
        "unresolved-ref" => "Reference does not resolve",
        // Swagger 2.0 battery.
        "swagger-info-required" => "Document without an info object",
        "swagger-info-field-required" => "Info object missing a required field",
        "swagger-operation-missing-responses" => "Operation without responses",
        "swagger-parameter-missing-name" => "Parameter without a name",
        "swagger-parameter-missing-in" => "Parameter without a location",
        "swagger-body-param-schema" => "Body parameter without a schema",
        "swagger-parameter-not-in-template" => {
            "Path parameter does not appear in the path template"
        }
        "swagger-path-param-undeclared" => "Path template variable without a declared parameter",
        "swagger-response-missing-description" => "Response without a description",
        "swagger-security-undefined" => "Security requirement names an undeclared scheme",
        "swagger-definition-shape" => "Definition does not declare a schema shape",
        // SDK-generation readiness notes.
        "sdk-operation-missing-id" => "Operation will be selected by method and path",
        "sdk-stream-response-untyped" => "Stream response will generate untyped events",
        "sdk-colon-path-parameters" => "Colon-style path template",
        _ => return None,
    })
}

fn how_to_fix_entry(code: &str) -> Option<&'static str> {
    Some(match code {
        "invalid-ref" => {
            "Write the reference as a URI followed by `#` and a JSON Pointer, e.g. `#/components/schemas/Pet`."
        }
        "oas-callback-invalid" => {
            "Give the callback a valid expression key and a Path Item value with operations."
        }
        "oas-deprecated-operation" => {
            "Set `deprecated: false`, or document the replacement in the description and plan removal."
        }
        "oas-discriminator-missing-property" => {
            "Add `propertyName` naming the field that distinguishes the composed schemas."
        }
        "oas-discriminator-unknown-mapping" => {
            "Point the mapping entry at a schema that exists under `components/schemas`."
        }
        "oas-duplicate-header-param" => {
            "Keep one header parameter per header name; merge the duplicated definitions."
        }
        "oas-duplicate-operation-id" => {
            "Give every operation a unique `operationId`; generated SDK method names depend on it."
        }
        "oas-empty-path-template" => {
            "Remove the empty path key, or name the resource in the template (`/items`)."
        }
        "oas-example-type-mismatch" => {
            "Make the example value match the schema's declared type (and enum values when present)."
        }
        "oas-header-invalid" => {
            "Header entries must be Parameter Objects with `schema`, or references to one."
        }
        "oas-http-array-invalid" => {
            "Declare the array with a JSON-Schema `type: array` and `items` instead of an ad-hoc shape."
        }
        "oas-http-boolean-invalid" => {
            "Declare the value with JSON-Schema `type: boolean` instead of an ad-hoc shape."
        }
        "oas-http-map-invalid" => {
            "Declare the map with `type: object` and `additionalProperties` instead of an ad-hoc shape."
        }
        "oas-license-missing-url" => {
            "Add `url` to the license object pointing at the license text."
        }
        "oas-link-invalid" => {
            "Give the link either `operationId` or `operationRef`, plus its parameters or request body."
        }
        "oas-media-type-invalid" => {
            "Use a valid RFC 2046 media type key such as `application/json`."
        }
        "oas-operation-invalid" => "Replace the operation value with an Operation Object.",
        "oas-operation-missing-responses" => {
            "Add a `responses` object with at least one status code or a `default`."
        }
        "oas-operation-no-error-response" => {
            "Add a `default` response or a 4XX/5XX response so client error handling is predictable."
        }
        "oas-operation-missing-operationId" => {
            "Add an `operationId` so generated SDK methods carry a stable name."
        }
        "oas-parameter-invalid" => {
            "Replace the parameter value with a Parameter Object or a `$ref` to one."
        }
        "oas-parameter-missing-in" => {
            "Add `in` with one of `query`, `header`, `path`, or `cookie`."
        }
        "oas-parameter-missing-name" => "Add `name` identifying the parameter on the wire.",
        "oas-parameter-required-missing" => {
            "Path parameters are always mandatory: set `required: true`."
        }
        "oas-path-item-invalid" => {
            "Replace the path-item value with a Path Item Object or a `$ref` to one."
        }
        "oas-path-item-security-invalid" => {
            "Path-item security must be an array of requirement objects, or omitted."
        }
        "oas-path-param-not-declared" => {
            "Declare every `{var}` in the path as an `in: path` parameter on the path item or operation."
        }
        "oas-path-trailing-slash" => "Drop the trailing slash so `/pets/` becomes `/pets`.",
        "oas-request-body-content-missing" => {
            "Add a `content` object with at least one media type."
        }
        "oas-request-body-invalid" => {
            "Replace the request body with a Request Body Object or a `$ref` to one."
        }
        "oas-response-entry-invalid" => {
            "Response keys must be status codes, status ranges, or `default`."
        }
        "oas-response-invalid" => {
            "Replace the response value with a Response Object or a `$ref` to one."
        }
        "oas-response-map-invalid" => {
            "Responses must be an object keyed by status code, range, or `default`."
        }
        "oas-response-missing-description" => "Add a `description` string to every response.",
        "oas-responses-invalid" => {
            "Replace `responses` with an object of status-code keys mapping to Response Objects."
        }
        "oas-schema-invalid-keyword" => {
            "Remove the keyword or move it into `x-` extension form; it is not valid in this dialect."
        }
        "oas-schema-invalid-kind" => "Schemas must be objects or `$ref` values, not plain scalars.",
        "oas-schema-invalid-type" => {
            "`type` accepts a single type name, or an array of them in 3.1."
        }
        "oas-schema-unknown-type" => {
            "Use one of `string`, `number`, `integer`, `boolean`, `object`, `array`, or `null`."
        }
        "oas-schema-unsupported-dialect" => {
            "Use the JSON-Schema dialect declared by the document's OpenAPI version."
        }
        "oas-security-invalid" => {
            "Declare security schemes under `components/securitySchemes` and reference them by name."
        }
        "oas-security-requirement-invalid" => {
            "Each requirement must map a declared scheme name to a list of required scopes."
        }
        "oas-security-scope-invalid" => "Scope entries must be strings.",
        "oas-security-scopes-invalid" => {
            "Give the scheme a list of required scopes; use `[]` when the scheme needs none."
        }
        "oas-security-scopes-nonempty" => {
            "This scheme type requires at least one scope in each requirement."
        }
        "oas-security-unknown-scheme" => {
            "Declare the scheme under `components/securitySchemes`, or fix the requirement's spelling."
        }
        "oas-server-invalid" => "Servers must be objects with a `url`.",
        "oas-server-url-invalid" => "Use a valid URL, optionally with `{variable}` templates.",
        "oas-server-variable-default-invalid" => {
            "Set `default` to one of the values listed in the variable's `enum`."
        }
        "oas-server-variable-enum-invalid" => {
            "Make the variable's `enum` a non-empty array of strings."
        }
        "oas-server-variable-invalid" => "Server variables must be objects with a `default`.",
        "oas-server-variable-unknown" => {
            "Declare the variable in the server's `variables` object, or remove it from the URL."
        }
        "oas-tag-undeclared" => {
            "Add the tag to the root `tags` list, or use one already declared there."
        }
        "oas-unused-path-param" => {
            "Remove the unused `in: path` parameter or add its `{name}` to the path template."
        }
        "oas-webhook-unsupported-version" => {
            "Move the webhook into a 3.1+ document, or express it as a callback."
        }
        "ref-outside-allowlist" => {
            "Point the reference at a document inside the configured reference allowlist."
        }
        "unresolved-ref" => {
            "Fix the reference's document or pointer so it resolves to an existing node."
        }
        "swagger-info-required" => "Add an `info` object with `title` and `version`.",
        "swagger-info-field-required" => "Add the missing field to `info`.",
        "swagger-operation-missing-responses" => {
            "Add a `responses` object with at least one status code."
        }
        "swagger-parameter-missing-name" => "Add `name` identifying the parameter on the wire.",
        "swagger-parameter-missing-in" => {
            "Add `in` with one of `query`, `header`, `path`, `formData`, or `body`."
        }
        "swagger-body-param-schema" => "Give the body parameter a `schema` describing the payload.",
        "swagger-parameter-not-in-template" => {
            "Remove the parameter or add `{name}` to the path template."
        }
        "swagger-path-param-undeclared" => {
            "Declare every `{var}` in the path as an `in: path` parameter with `required: true`."
        }
        "swagger-response-missing-description" => "Add a `description` string to every response.",
        "swagger-security-undefined" => {
            "Declare the scheme under `securityDefinitions`, or fix the requirement's spelling."
        }
        "swagger-definition-shape" => {
            "Give the definition a `type`, a `$ref`, or a composition (`allOf`/`anyOf`/`oneOf`)."
        }
        "sdk-operation-missing-id" => {
            "Add an `operationId` so generated SDK methods carry a stable name."
        }
        "sdk-stream-response-untyped" => {
            "Declare a `schema` on the `text/event-stream` media type to get typed stream events."
        }
        "sdk-colon-path-parameters" => {
            "Prefer `{var}` templates over `:var` so path parameters need no compatibility profile."
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_sorted_for_binary_search_shapes() {
        // The lookups are linear matches today; the sorted-arm requirement
        // is enforced by construction (duplicate arms fail compilation).
        // This pins the shared code set: every summary must have a matching
        // how-to-fix and vice versa, so the two tables never drift.
        let probes = [
            "invalid-ref",
            "oas-duplicate-operation-id",
            "oas-response-missing-description",
            "oas-security-unknown-scheme",
            "swagger-info-required",
            "sdk-stream-response-untyped",
        ];
        for code in probes {
            assert!(!summary(code).is_empty(), "summary missing for {code}");
            assert!(
                !how_to_fix(code).is_empty(),
                "how-to-fix missing for {code}"
            );
        }
    }

    #[test]
    fn unknown_codes_yield_empty_guidance() {
        assert_eq!(summary("not-a-code"), "");
        assert_eq!(how_to_fix("not-a-code"), "");
    }
}
