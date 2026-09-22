//! Canonical OpenAPI key order, per object context.
//!
//! Ordering philosophy: short at-a-glance keys first, long structural
//! blocks last, related keys grouped (`type`+`format`, numeric
//! `minimum`/`multipleOf`/`maximum`, composition `allOf`/`anyOf`/`oneOf`/
//! `not`), `$ref` always first (a `$ref` object carries at most two other
//! keys). Tables cover Swagger 2.0, OpenAPI 3.0, 3.1, and 3.2; keys absent
//! from a table (unknown or `x-` extensions) sort after the known keys,
//! alphabetically, unless an extension registry entry positions them.

/// Root document keys.
pub const ROOT: &[&str] = &[
    "swagger",
    "openapi",
    "jsonSchemaDialect",
    "info",
    "externalDocs",
    "schemes",
    "host",
    "basePath",
    "consumes",
    "produces",
    "servers",
    "security",
    "tags",
    "paths",
    "webhooks",
    "components",
    "definitions",
    "parameters",
    "responses",
    "securityDefinitions",
];

/// `info` object keys.
pub const INFO: &[&str] = &[
    "title",
    "version",
    "summary",
    "description",
    "termsOfService",
    "contact",
    "license",
];

/// `contact` object keys.
pub const CONTACT: &[&str] = &["name", "email", "url"];

/// `license` object keys.
pub const LICENSE: &[&str] = &["name", "identifier", "url"];

/// `components` object keys: big reusable blocks first, atomic builders last.
pub const COMPONENTS: &[&str] = &[
    "securitySchemes",
    "pathItems",
    "parameters",
    "headers",
    "requestBodies",
    "responses",
    "callbacks",
    "links",
    "schemas",
    "examples",
];

/// Operation object keys.
pub const OPERATION: &[&str] = &[
    "operationId",
    "summary",
    "tags",
    "deprecated",
    "description",
    "externalDocs",
    "security",
    "servers",
    "consumes",
    "produces",
    "parameters",
    "requestBody",
    "responses",
    "callbacks",
    "schemes",
];

/// Parameter object keys.
pub const PARAMETER: &[&str] = &[
    "$ref",
    "name",
    "description",
    "in",
    "required",
    "deprecated",
    "allowEmptyValue",
    "style",
    "explode",
    "allowReserved",
    "schema",
    "content",
    "type",
    "format",
    "items",
    "collectionFormat",
    "default",
    "minimum",
    "exclusiveMinimum",
    "multipleOf",
    "maximum",
    "exclusiveMaximum",
    "pattern",
    "minLength",
    "maxLength",
    "minItems",
    "maxItems",
    "uniqueItems",
    "enum",
    "example",
    "examples",
];

/// Schema object keys (OpenAPI Schema Object superset over JSON Schema
/// 2020-12).
pub const SCHEMA: &[&str] = &[
    "$ref",
    "$id",
    "$schema",
    "$vocabulary",
    "$anchor",
    "$dynamicAnchor",
    "$dynamicRef",
    "$comment",
    "$defs",
    "$recursiveAnchor",
    "$recursiveRef",
    "title",
    "description",
    "externalDocs",
    "deprecated",
    "type",
    "format",
    "contentSchema",
    "contentMediaType",
    "contentEncoding",
    "nullable",
    "const",
    "enum",
    "default",
    "readOnly",
    "writeOnly",
    "example",
    "examples",
    "minimum",
    "exclusiveMinimum",
    "multipleOf",
    "maximum",
    "exclusiveMaximum",
    "pattern",
    "minLength",
    "maxLength",
    "uniqueItems",
    "minItems",
    "maxItems",
    "items",
    "prefixItems",
    "contains",
    "minContains",
    "maxContains",
    "unevaluatedItems",
    "minProperties",
    "maxProperties",
    "patternProperties",
    "additionalProperties",
    "properties",
    "required",
    "unevaluatedProperties",
    "propertyNames",
    "dependentRequired",
    "dependentSchemas",
    "discriminator",
    "xml",
    "allOf",
    "anyOf",
    "oneOf",
    "not",
    "if",
    "then",
    "else",
];

/// Response object keys.
pub const RESPONSE: &[&str] = &[
    "$ref",
    "description",
    "headers",
    "schema",
    "content",
    "examples",
    "links",
];

/// Security scheme object keys.
pub const SECURITY_SCHEME: &[&str] = &[
    "$ref",
    "name",
    "description",
    "type",
    "in",
    "scheme",
    "bearerFormat",
    "openIdConnectUrl",
    "flows",
    "flow",
    "authorizationUrl",
    "tokenUrl",
    "scopes",
];

/// OAuth flow object keys.
pub const OAUTH_FLOW: &[&str] = &["authorizationUrl", "tokenUrl", "refreshUrl", "scopes"];

/// Server object keys.
pub const SERVER: &[&str] = &["name", "description", "url", "variables"];

/// Server variable object keys.
pub const SERVER_VARIABLE: &[&str] = &["description", "default", "enum"];

/// Tag object keys.
pub const TAG: &[&str] = &["name", "description", "externalDocs"];

/// External documentation object keys.
pub const EXTERNAL_DOCS: &[&str] = &["description", "url"];

/// Path item object keys.
pub const PATH_ITEM: &[&str] = &[
    "$ref",
    "summary",
    "description",
    "servers",
    "parameters",
    "get",
    "put",
    "post",
    "patch",
    "delete",
    "options",
    "head",
    "trace",
];

/// Request body object keys.
pub const REQUEST_BODY: &[&str] = &["$ref", "description", "required", "content"];

/// Media type object keys.
pub const MEDIA_TYPE: &[&str] = &["schema", "example", "examples", "encoding"];

/// Encoding object keys.
pub const ENCODING: &[&str] = &[
    "contentType",
    "style",
    "explode",
    "allowReserved",
    "headers",
];

/// Header object keys.
pub const HEADER: &[&str] = &[
    "$ref",
    "description",
    "required",
    "deprecated",
    "schema",
    "content",
    "type",
    "format",
    "style",
    "explode",
    "enum",
    "default",
    "example",
    "examples",
    "items",
    "collectionFormat",
    "maxItems",
    "minItems",
    "uniqueItems",
    "minimum",
    "multipleOf",
    "exclusiveMinimum",
    "maximum",
    "exclusiveMaximum",
    "pattern",
    "minLength",
    "maxLength",
];

/// Link object keys.
pub const LINK: &[&str] = &[
    "$ref",
    "operationId",
    "description",
    "server",
    "operationRef",
    "parameters",
    "requestBody",
];

/// Example object keys.
pub const EXAMPLE: &[&str] = &["summary", "description", "value", "externalValue"];

/// Discriminator object keys.
pub const DISCRIMINATOR: &[&str] = &["propertyName", "mapping"];

/// XML object keys.
pub const XML: &[&str] = &["name", "namespace", "prefix", "attribute", "wrapped"];

/// Callback expression keys (Path Item shape).
pub const CALLBACK: &[&str] = PATH_ITEM;

/// Resolves the key table for a detected context.
#[must_use]
pub fn table_for(context: Context) -> &'static [&'static str] {
    match context {
        Context::Root => ROOT,
        Context::Info => INFO,
        Context::Contact => CONTACT,
        Context::License => LICENSE,
        Context::Components => COMPONENTS,
        Context::Operation => OPERATION,
        Context::Parameter => PARAMETER,
        Context::Schema => SCHEMA,
        Context::Response => RESPONSE,
        Context::SecurityScheme => SECURITY_SCHEME,
        Context::OAuthFlow => OAUTH_FLOW,
        Context::Server => SERVER,
        Context::ServerVariable => SERVER_VARIABLE,
        Context::Tag => TAG,
        Context::ExternalDocs => EXTERNAL_DOCS,
        Context::PathItem => PATH_ITEM,
        Context::RequestBody => REQUEST_BODY,
        Context::MediaType => MEDIA_TYPE,
        Context::Encoding => ENCODING,
        Context::Header => HEADER,
        Context::Link => LINK,
        Context::Example => EXAMPLE,
        Context::Discriminator => DISCRIMINATOR,
        Context::Xml => XML,
        Context::Callback => CALLBACK,
        Context::Unknown => &[],
    }
}

/// The object contexts the formatter recognizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    /// The document root mapping.
    Root,
    /// The `info` object.
    Info,
    /// The `info.contact` object.
    Contact,
    /// The `info.license` object.
    License,
    /// The `components` object.
    Components,
    /// An operation object (`paths./p.get`).
    Operation,
    /// A parameter object.
    Parameter,
    /// A schema object.
    Schema,
    /// A response object.
    Response,
    /// A security scheme object.
    SecurityScheme,
    /// An OAuth flow object.
    OAuthFlow,
    /// A server object.
    Server,
    /// A server variable object.
    ServerVariable,
    /// A tag object.
    Tag,
    /// An external documentation object.
    ExternalDocs,
    /// A path item object.
    PathItem,
    /// A request body object.
    RequestBody,
    /// A media type object.
    MediaType,
    /// An encoding object.
    Encoding,
    /// A header object.
    Header,
    /// A link object.
    Link,
    /// An example object.
    Example,
    /// A discriminator object.
    Discriminator,
    /// An XML object.
    Xml,
    /// A callback expression path item.
    Callback,
    /// No recognized context: keys keep document order.
    Unknown,
}

/// HTTP method keys, canonical order within a path item.
pub const HTTP_METHODS: &[&str] = &[
    "get", "put", "post", "patch", "delete", "options", "head", "trace",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_is_first_in_ref_bearing_tables() {
        for table in [
            PARAMETER,
            SCHEMA,
            RESPONSE,
            SECURITY_SCHEME,
            REQUEST_BODY,
            HEADER,
            LINK,
            PATH_ITEM,
        ] {
            assert_eq!(table[0], "$ref", "{table:?}");
        }
    }

    #[test]
    fn every_table_is_unique() {
        for table in [
            ROOT,
            INFO,
            CONTACT,
            LICENSE,
            COMPONENTS,
            OPERATION,
            PARAMETER,
            SCHEMA,
            RESPONSE,
            SECURITY_SCHEME,
            OAUTH_FLOW,
            SERVER,
            SERVER_VARIABLE,
            TAG,
            EXTERNAL_DOCS,
            PATH_ITEM,
            REQUEST_BODY,
            MEDIA_TYPE,
            ENCODING,
            HEADER,
            LINK,
            EXAMPLE,
            DISCRIMINATOR,
            XML,
        ] {
            let mut seen = std::collections::BTreeSet::new();
            for key in table.iter() {
                assert!(seen.insert(*key), "duplicate key {key} in {table:?}");
            }
        }
    }

    #[test]
    fn tables_cover_their_context_enum() {
        // Every non-unknown context must resolve to a non-empty table.
        for context in [
            Context::Root,
            Context::Info,
            Context::Contact,
            Context::License,
            Context::Components,
            Context::Operation,
            Context::Parameter,
            Context::Schema,
            Context::Response,
            Context::SecurityScheme,
            Context::OAuthFlow,
            Context::Server,
            Context::ServerVariable,
            Context::Tag,
            Context::ExternalDocs,
            Context::PathItem,
            Context::RequestBody,
            Context::MediaType,
            Context::Encoding,
            Context::Header,
            Context::Link,
            Context::Example,
            Context::Discriminator,
            Context::Xml,
            Context::Callback,
        ] {
            assert!(!table_for(context).is_empty(), "{context:?}");
        }
    }
}
