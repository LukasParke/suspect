//! Structural positions in an OpenAPI description. Instance data is never walked.

use serde_json::Value;

use super::SourceId;
use super::method::FIXED_METHODS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Kind {
    Root,
    Components,
    PathItem,
    Operation,
    Parameter,
    Header,
    Body,
    Response,
    Media,
    Encoding,
    Callback,
    SecurityScheme,
    Example,
    Link,
    Schema,
}

impl Kind {
    pub fn reference_allowed(self, version: &str) -> bool {
        matches!(
            self,
            Self::PathItem
                | Self::Parameter
                | Self::Header
                | Self::Body
                | Self::Response
                | Self::Callback
                | Self::SecurityScheme
                | Self::Example
                | Self::Link
                | Self::Schema
        ) || self == Self::Media && version.starts_with("3.2.")
    }
}

/// Containment only: following references is the indexer's responsibility.
/// A malformed declared schema slot is retained so it receives a source-linked
/// error. Maps, arrays, examples, defaults and extensions cannot invent slots.
pub(super) fn children(
    source: &SourceId,
    raw: &Value,
    kind: Kind,
    version: &str,
) -> Vec<(SourceId, Kind)> {
    let mut out = Vec::new();
    if kind.reference_allowed(version)
        && raw.get("$ref").is_some()
        && !(kind == Kind::PathItem || kind == Kind::Schema && !version.starts_with("3.0."))
    {
        return out;
    }
    let modern = !version.starts_with("3.0.");
    let oas32 = version.starts_with("3.2.");
    match kind {
        Kind::Root => {
            field(&mut out, source, raw, "components", Kind::Components);
            map(&mut out, source, raw, "paths", Kind::PathItem, true);
            if modern {
                map(&mut out, source, raw, "webhooks", Kind::PathItem, false);
            }
        }
        Kind::Components => {
            for (key, kind) in [
                ("schemas", Kind::Schema),
                ("parameters", Kind::Parameter),
                ("headers", Kind::Header),
                ("requestBodies", Kind::Body),
                ("responses", Kind::Response),
                ("callbacks", Kind::Callback),
                ("securitySchemes", Kind::SecurityScheme),
                ("examples", Kind::Example),
                ("links", Kind::Link),
            ] {
                map(&mut out, source, raw, key, kind, false);
            }
            if modern {
                map(&mut out, source, raw, "pathItems", Kind::PathItem, false);
            }
            if oas32 {
                map(&mut out, source, raw, "mediaTypes", Kind::Media, false);
            }
        }
        Kind::PathItem => {
            array(&mut out, source, raw, "parameters", Kind::Parameter);
            for (key, _) in FIXED_METHODS {
                if key != "query" || oas32 {
                    field(&mut out, source, raw, key, Kind::Operation);
                }
            }
            if oas32 {
                map(
                    &mut out,
                    source,
                    raw,
                    "additionalOperations",
                    Kind::Operation,
                    false,
                );
            }
        }
        Kind::Operation => {
            array(&mut out, source, raw, "parameters", Kind::Parameter);
            field(&mut out, source, raw, "requestBody", Kind::Body);
            map(&mut out, source, raw, "responses", Kind::Response, true);
            map(&mut out, source, raw, "callbacks", Kind::Callback, false);
        }
        Kind::Parameter | Kind::Header => {
            field(&mut out, source, raw, "schema", Kind::Schema);
            map(&mut out, source, raw, "content", Kind::Media, false);
            map(&mut out, source, raw, "examples", Kind::Example, false);
        }
        Kind::Body => map(&mut out, source, raw, "content", Kind::Media, false),
        Kind::Response => {
            map(&mut out, source, raw, "content", Kind::Media, false);
            map(&mut out, source, raw, "headers", Kind::Header, false);
            map(&mut out, source, raw, "links", Kind::Link, false);
        }
        Kind::Media => {
            field(&mut out, source, raw, "schema", Kind::Schema);
            map(&mut out, source, raw, "encoding", Kind::Encoding, false);
            map(&mut out, source, raw, "examples", Kind::Example, false);
            if oas32 {
                field(&mut out, source, raw, "itemSchema", Kind::Schema);
                array(&mut out, source, raw, "prefixEncoding", Kind::Encoding);
                field(&mut out, source, raw, "itemEncoding", Kind::Encoding);
            }
        }
        Kind::Encoding => {
            map(&mut out, source, raw, "headers", Kind::Header, false);
            if oas32 {
                map(&mut out, source, raw, "encoding", Kind::Encoding, false);
                array(&mut out, source, raw, "prefixEncoding", Kind::Encoding);
                field(&mut out, source, raw, "itemEncoding", Kind::Encoding);
            }
        }
        Kind::Callback => {
            for key in raw.as_object().into_iter().flat_map(|map| map.keys()) {
                if !key.starts_with("x-") {
                    out.push((source.child(key), Kind::PathItem));
                }
            }
        }
        Kind::Schema => {
            for key in [
                "properties",
                "patternProperties",
                "dependentSchemas",
                "$defs",
            ] {
                map(&mut out, source, raw, key, Kind::Schema, false);
            }
            for key in ["allOf", "anyOf", "oneOf", "prefixItems"] {
                array(&mut out, source, raw, key, Kind::Schema);
            }
            for key in [
                "items",
                "contains",
                "additionalProperties",
                "unevaluatedProperties",
                "unevaluatedItems",
                "propertyNames",
                "not",
                "if",
                "then",
                "else",
                "contentSchema",
            ] {
                field(&mut out, source, raw, key, Kind::Schema);
            }
        }
        Kind::SecurityScheme | Kind::Example | Kind::Link => {}
    }
    out
}

fn field(out: &mut Vec<(SourceId, Kind)>, source: &SourceId, raw: &Value, key: &str, kind: Kind) {
    if raw.get(key).is_some() {
        out.push((source.child(key), kind));
    }
}

fn map(
    out: &mut Vec<(SourceId, Kind)>,
    source: &SourceId,
    raw: &Value,
    key: &str,
    kind: Kind,
    extensions: bool,
) {
    if let Some(map) = raw.get(key).and_then(Value::as_object) {
        let source = source.child(key);
        out.extend(
            map.keys()
                .filter(|key| !(extensions && key.starts_with("x-")))
                .map(|key| (source.child(key), kind)),
        );
    }
}

fn array(out: &mut Vec<(SourceId, Kind)>, source: &SourceId, raw: &Value, key: &str, kind: Kind) {
    if let Some(array) = raw.get(key).and_then(Value::as_array) {
        let source = source.child(key);
        out.extend((0..array.len()).map(|index| (source.child(&index.to_string()), kind)));
    }
}
