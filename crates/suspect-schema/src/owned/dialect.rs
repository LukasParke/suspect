//! Source-aware dialect admission. Contract metadata is never rewritten.

use iri_string::types::{UriReferenceStr, UriStr};
use suspect_ir::contract::SchemaDialect;

use super::compile::{error, invalid};
use super::*;

const OAS: &str = "https://spec.openapis.org/oas/3.1/dialect/base";
const JSON_SCHEMA: &str = "https://json-schema.org/draft/2020-12/schema";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Dialect {
    OpenApi30,
    OpenApi31,
    OpenApi32,
    JsonSchema202012,
}

pub(super) fn reference_only(contract: &Contract, id: &SchemaId) -> bool {
    contract
        .schema(id)
        .is_some_and(|schema| schema.ignores_ref_siblings())
}

/// Follow the actual schema-position ancestry, not arbitrary objects with a
/// `$schema`/`$id` field (property maps and instance annotations are not schemas).
fn ancestors(contract: &Contract, id: &SchemaId) -> Vec<SourceId> {
    let pointer = Pointer::parse(id.pointer()).expect("indexed pointer");
    let mut out = Vec::new();
    for length in 0..=pointer.tokens().len() {
        let path = Pointer::from_tokens(pointer.tokens()[..length].to_vec());
        let at = SourceId::new(id.document().clone(), path.clone());
        if contract.is_schema_position(&at) {
            out.push(at);
        }
    }
    out
}

pub(super) fn admit(
    contract: &Contract,
    id: &SchemaId,
    resources: bool,
) -> Result<Dialect, OwnedCompileError> {
    let schema = contract.schema(id).expect("indexed schema");
    let document = contract.document(id.document()).expect("retained document");
    let document_id = SourceId::new(id.document().clone(), Pointer::root());
    let version = contract.openapi_version_at(id);
    let oas30 = matches!(schema.dialect(), SchemaDialect::OpenApi30);
    let mut declaration = schema.dialect_source().cloned();
    if let Some(at) = schema.default_dialect_source() {
        let value = contract.source(at).expect("retained dialect declaration");
        if oas30 {
            return Err(invalid(
                contract,
                at,
                "OpenAPI 3.0 does not define jsonSchemaDialect",
            ));
        }
        absolute_uri(contract, at, value)?;
    }
    if let Some(at) = &declaration {
        absolute_uri(
            contract,
            at,
            contract
                .source(at)
                .expect("retained effective dialect declaration"),
        )?;
    }
    if document
        .get("openapi")
        .and_then(Value::as_str)
        .is_some_and(|version| version.starts_with("3.2."))
        && let Some(value) = document.get("$self")
    {
        let at = document_id.child("$self");
        uri_reference(contract, &at, value)?;
        if !resources {
            return Err(error(
                contract,
                &at,
                OwnedCompileErrorKind::Unsupported,
                "OpenAPI 3.2 `$self` requires canonical document/base-URI indexing in Contract; retrieval-URI reference edges cannot be assumed equivalent",
            ));
        }
    }
    let ancestry = ancestors(contract, id);
    for (position, ancestor) in ancestry.iter().enumerate() {
        if reference_only(contract, ancestor) {
            continue; // OAS 3.0 Reference Object siblings have no lexical effect.
        }
        let Some(object) = contract.source(ancestor).and_then(Value::as_object) else {
            continue;
        };
        if let Some(value) = object.get("$schema") {
            let at = ancestor.child("$schema");
            if oas30 {
                return Err(invalid(
                    contract,
                    &at,
                    "OpenAPI 3.0 does not permit `$schema` overrides",
                ));
            }
            absolute_uri(contract, &at, value)?;
            // An embedded OpenAPI schema root has no enclosing schema. Nested
            // resources need $id; selecting a subschema cannot create a resource.
            if position != 0 && !object.contains_key("$id") {
                return Err(invalid(
                    contract,
                    &at,
                    "`$schema` is only allowed at a schema resource root; selecting a nested schema does not establish a resource",
                ));
            }
            declaration = Some(at);
        }
        // These also matter when the selected root is below the declaration,
        // including an unindexed root of an external schema document.
        for key in ["$id", "$dynamicAnchor", "$recursiveAnchor", "$vocabulary"] {
            if let Some(value) = object.get(key) {
                let at = ancestor.child(key);
                if oas30 {
                    return Err(invalid(
                        contract,
                        &at,
                        &format!("`{key}` is outside the OpenAPI 3.0 Schema Object vocabulary"),
                    ));
                }
                match key {
                    "$id" => {
                        let uri = uri_reference(contract, &at, value)?;
                        if uri
                            .split_once('#')
                            .is_some_and(|(_, fragment)| !fragment.is_empty())
                        {
                            return Err(invalid(
                                contract,
                                &at,
                                "`$id` must not contain a nonempty fragment",
                            ));
                        }
                    }
                    "$dynamicAnchor" => anchor(contract, &at, value)?,
                    "$recursiveAnchor" if !value.is_boolean() => {
                        return Err(invalid(
                            contract,
                            &at,
                            "`$recursiveAnchor` must be a boolean",
                        ));
                    }
                    "$vocabulary" => vocabulary(contract, &at, value)?,
                    _ => {}
                }
                if resources && matches!(key, "$id" | "$dynamicAnchor") {
                    continue;
                }
                return Err(error(
                    contract,
                    &at,
                    OwnedCompileErrorKind::Unsupported,
                    match key {
                        "$id" => {
                            "`$id` requires canonical schema-resource/base-URI indexing in Contract before static reference lowering"
                        }
                        "$vocabulary" => {
                            "`$vocabulary` declarations are outside the checked SDK profile; custom meta-schema/vocabulary admission is not implemented"
                        }
                        _ => {
                            "dynamic/recursive anchors require dialect-specific resource and dynamic-scope indexing; static ref instructions are insufficient"
                        }
                    },
                ));
            }
        }
    }
    if oas30 {
        return Ok(Dialect::OpenApi30);
    }
    let SchemaDialect::Uri(uri) = schema.dialect() else {
        unreachable!()
    };
    let at = declaration.as_ref().unwrap_or(id);
    // Contract retains the dialect; admission does not guess another dialect
    // or manufacture a new URI from an OpenAPI version number.
    match uri.strip_suffix('#').unwrap_or(uri) {
        OAS if version.starts_with("3.2.") => Ok(Dialect::OpenApi32),
        OAS => Ok(Dialect::OpenApi31),
        JSON_SCHEMA => Ok(Dialect::JsonSchema202012),
        _ => Err(error(
            contract,
            at,
            OwnedCompileErrorKind::Unsupported,
            &format!(
                "schema dialect `{uri}` is not supported; expected the OAS 3.1 base dialect (also used by OAS 3.2) or JSON Schema 2020-12"
            ),
        )),
    }
}

fn absolute_uri<'a>(
    contract: &Contract,
    at: &SourceId,
    value: &'a Value,
) -> Result<&'a str, OwnedCompileError> {
    let uri = value
        .as_str()
        .filter(|uri| UriStr::new(uri).is_ok())
        .ok_or_else(|| {
            invalid(
                contract,
                at,
                "schema dialect declaration must be an absolute RFC 3986 URI string",
            )
        })?;
    Ok(uri)
}

pub(super) fn uri_reference<'a>(
    contract: &Contract,
    at: &SourceId,
    value: &'a Value,
) -> Result<&'a str, OwnedCompileError> {
    value
        .as_str()
        .filter(|uri| UriReferenceStr::new(uri).is_ok())
        .ok_or_else(|| {
            invalid(
                contract,
                at,
                "reference must be a valid RFC 3986 URI-reference string without raw whitespace",
            )
        })
}

pub(super) fn anchor(
    contract: &Contract,
    at: &SourceId,
    value: &Value,
) -> Result<(), OwnedCompileError> {
    let Some(anchor) = value.as_str() else {
        return Err(invalid(contract, at, "anchor must be a string"));
    };
    let bytes = anchor.as_bytes();
    if !bytes
        .first()
        .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
        || !bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
    {
        return Err(invalid(
            contract,
            at,
            "anchor must match [A-Za-z_][A-Za-z0-9_.-]*",
        ));
    }
    Ok(())
}

fn vocabulary(contract: &Contract, at: &SourceId, value: &Value) -> Result<(), OwnedCompileError> {
    let Some(object) = value.as_object() else {
        return Err(invalid(
            contract,
            at,
            "`$vocabulary` must be an object of URI/boolean entries",
        ));
    };
    for (uri, required) in object {
        if UriStr::new(uri).is_err() || !required.is_boolean() {
            return Err(invalid(
                contract,
                &at.child(uri),
                "vocabulary names must be absolute URIs and values must be booleans",
            ));
        }
    }
    Ok(())
}

pub(super) fn keyword_in_30(keyword: &str) -> bool {
    keyword.starts_with("x-")
        || matches!(
            keyword,
            "$ref"
                | "type"
                | "title"
                | "description"
                | "format"
                | "default"
                | "example"
                | "nullable"
                | "readOnly"
                | "writeOnly"
                | "deprecated"
                | "discriminator"
                | "xml"
                | "externalDocs"
                | "multipleOf"
                | "maximum"
                | "exclusiveMaximum"
                | "minimum"
                | "exclusiveMinimum"
                | "maxLength"
                | "minLength"
                | "pattern"
                | "maxItems"
                | "minItems"
                | "uniqueItems"
                | "maxProperties"
                | "minProperties"
                | "required"
                | "enum"
                | "properties"
                | "additionalProperties"
                | "items"
                | "allOf"
                | "anyOf"
                | "oneOf"
                | "not"
        )
}

/// The OAS 3.0 reference is ECMA-262 5.1 (UTF-16 code units), while the shared
/// NFA consumes Unicode scalars. Positive BMP character sets have the same
/// match results over well-formed Unicode. Dot/complement/astral patterns and
/// newer escape/whitespace semantics need a separate pattern profile.
pub(super) fn pattern_30(
    contract: &Contract,
    at: &SourceId,
    text: &str,
    program: &crate::PatternProgram,
) -> Result<(), OwnedCompileError> {
    let mut characters = text.chars().peekable();
    let mut newer_escape = false;
    while let Some(character) = characters.next() {
        if character == '\\' {
            match characters.next() {
                Some('s' | 'S') => newer_escape = true,
                Some('u') if characters.peek() == Some(&'{') => newer_escape = true,
                _ => {}
            }
        }
    }
    let astral = program.states.iter().any(|state| {
        matches!(state,
        crate::PatternState::Char { ranges, .. } if ranges.iter().any(|range| range[1] > 0xffff))
    });
    if newer_escape || astral {
        return Err(error(
            contract,
            at,
            OwnedCompileErrorKind::Unsupported,
            "this OpenAPI 3.0 pattern needs an ECMA-262 5.1 code-unit/escape profile; the existing Unicode-scalar NFA only admits positive BMP character sets with compatible escapes for OAS 3.0",
        ));
    }
    Ok(())
}

/// OAS metadata has declared field types even though it contributes no JSON
/// validation instructions. In particular, 3.2 defaultMapping is a hint, not
/// permission to bypass oneOf; XML nodeType does not turn a JSON value into XML.
pub(super) fn annotation(
    contract: &Contract,
    at: &SourceId,
    keyword: &str,
    value: &Value,
    dialect: Dialect,
) -> Result<(), OwnedCompileError> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid(contract, at, &format!("`{keyword}` must be an object")))?;
    let required = match keyword {
        "discriminator" => Some("propertyName"),
        "externalDocs" => Some("url"),
        _ => None,
    };
    if let Some(required) = required
        && !object.contains_key(required)
    {
        return Err(invalid(
            contract,
            at,
            &format!("`{keyword}` requires a `{required}` string"),
        ));
    }
    for (name, value) in object {
        let source = at.child(name);
        if name.starts_with("x-") {
            continue;
        }
        match (keyword, name.as_str()) {
            ("discriminator", "propertyName")
            | ("externalDocs", "description")
            | ("xml", "name" | "prefix") => {
                if !value.is_string() {
                    return Err(invalid(
                        contract,
                        &source,
                        &format!("`{name}` must be a string"),
                    ));
                }
            }
            ("discriminator", "mapping") => {
                let mappings = value.as_object().ok_or_else(|| {
                    invalid(
                        contract,
                        &source,
                        "discriminator mapping must be an object of strings",
                    )
                })?;
                for (name, value) in mappings {
                    if !value.is_string() {
                        return Err(invalid(
                            contract,
                            &source.child(name),
                            "discriminator mapping values must be strings",
                        ));
                    }
                }
            }
            ("discriminator", "defaultMapping") if dialect == Dialect::OpenApi32 => {
                if !value.is_string() {
                    return Err(invalid(
                        contract,
                        &source,
                        "OpenAPI 3.2 discriminator defaultMapping must be a string",
                    ));
                }
            }
            ("externalDocs", "url") | ("xml", "namespace") => {
                uri_reference(contract, &source, value)?;
            }
            ("xml", "attribute" | "wrapped") => {
                if !value.is_boolean() {
                    return Err(invalid(
                        contract,
                        &source,
                        &format!("`{name}` must be a boolean"),
                    ));
                }
                if dialect == Dialect::OpenApi32 && object.contains_key("nodeType") {
                    return Err(invalid(
                        contract,
                        &source,
                        "OpenAPI 3.2 XML nodeType excludes both attribute and wrapped, even when false",
                    ));
                }
            }
            ("xml", "nodeType") if dialect == Dialect::OpenApi32 => {
                if !matches!(
                    value.as_str(),
                    Some("element" | "attribute" | "text" | "cdata" | "none")
                ) {
                    return Err(invalid(
                        contract,
                        &source,
                        "OpenAPI 3.2 XML nodeType must be element, attribute, text, cdata, or none",
                    ));
                }
            }
            _ => {
                return Err(invalid(
                    contract,
                    &source,
                    &format!(
                        "`{name}` is not a defined field of `{keyword}` for this OpenAPI version; extensions require an x- prefix"
                    ),
                ));
            }
        }
    }
    Ok(())
}
