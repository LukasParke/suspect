//! Presentation hints from source-declared operation roles.
//!
//! A display name is not a schema identity. Every native allocator still owns
//! casing, reserved words, collisions and representation/directional suffixes;
//! references, schema equality and codec bindings continue to use SourceId.

use std::collections::BTreeMap;

use suspect_ir::contract::{Contract, ParameterLocation, SchemaId, SourceId};
use suspect_source::Uri;

pub(crate) struct Hints {
    documents: BTreeMap<Uri, BTreeMap<String, String>>,
}

impl Hints {
    pub(crate) fn new(contract: &Contract) -> Self {
        let mut result = Self {
            documents: BTreeMap::new(),
        };
        // Using all indexed operations keeps a name independent of the caller's
        // selected subset. Component definitions retain their intrinsic names.
        for operation in contract
            .operations()
            .chain(contract.webhooks())
            .chain(contract.callbacks())
        {
            let Some(name) = operation.operation_id().filter(|name| !name.is_empty()) else {
                continue;
            };
            for parameter in operation.parameters() {
                let (Some(wire_name), Some(location)) = (parameter.name(), parameter.location())
                else {
                    continue;
                };
                let location = match location {
                    ParameterLocation::Path => "Path",
                    ParameterLocation::Query => "Query",
                    ParameterLocation::Querystring => "Querystring",
                    ParameterLocation::Header => "Header",
                    ParameterLocation::Cookie => "Cookie",
                };
                let base = format!("{name}_{location}_{wire_name}");
                if let Some(schema) = parameter.schema() {
                    result.insert(operation.source(), schema.id(), base.clone());
                }
                for media in parameter.content() {
                    if let Some(schema) = media.schema() {
                        result.insert(
                            operation.source(),
                            schema.id(),
                            format!("{base}{}", media_suffix(media.name())),
                        );
                    }
                }
            }
            if let Some(body) = operation.request_body() {
                for media in body.content() {
                    if let Some(schema) = media.schema() {
                        result.insert(
                            operation.source(),
                            schema.id(),
                            format!("{name}_Request{}", media_suffix(media.name())),
                        );
                    }
                }
            }
            for response in operation.responses() {
                let base = format!("{name}_Response_{}", response.status_key());
                for media in response.content() {
                    if let Some(schema) = media.schema() {
                        result.insert(
                            operation.source(),
                            schema.id(),
                            format!("{base}{}", media_suffix(media.name())),
                        );
                    }
                }
                for header in response.headers() {
                    if let Some(schema) = header.schema() {
                        result.insert(
                            operation.source(),
                            schema.id(),
                            format!("{base}_Header_{}", header.name()),
                        );
                    }
                }
            }
        }
        result
    }

    fn insert(&mut self, operation: &SourceId, schema: &SchemaId, name: String) {
        // A referenced/shared definition must not acquire the name of whichever
        // operation happens to use it first. This hint is for inline declarations.
        if schema.document() != operation.document()
            || !schema
                .pointer()
                .strip_prefix(operation.pointer())
                .is_some_and(|tail| tail.starts_with('/'))
        {
            return;
        }
        self.documents
            .entry(schema.document().clone())
            .or_default()
            .insert(schema.pointer().to_owned(), name);
    }

    pub(crate) fn get(&self, schema: &SchemaId) -> Option<String> {
        let hints = self.documents.get(schema.document())?;
        let mut pointer = schema.pointer();
        let mut tail: Vec<String> = Vec::new();
        loop {
            if let Some(base) = hints.get(pointer) {
                tail.reverse();
                let mut parts = vec![base.clone()];
                let mut tokens = tail.into_iter();
                while let Some(token) = tokens.next() {
                    match token.as_str() {
                        // Consume the *name* separately: a property literally
                        // called "properties" or "items" must not disappear.
                        "properties" | "$defs" | "definitions" => {
                            if let Some(name) = tokens.next() {
                                parts.push(name);
                            }
                        }
                        "items" => parts.push("Item".into()),
                        "additionalProperties" => parts.push("AdditionalProperty".into()),
                        "propertyNames" => parts.push("PropertyName".into()),
                        "prefixItems" => parts.push("Item".into()),
                        "oneOf" | "anyOf" => parts.push("Variant".into()),
                        "allOf" => parts.push("Intersection".into()),
                        _ => parts.push(token),
                    }
                }
                return Some(parts.join("_"));
            }
            let (parent, token) = pointer.rsplit_once('/')?;
            tail.push(token.replace("~1", "/").replace("~0", "~"));
            pointer = parent;
        }
    }
}

fn media_suffix(media: &str) -> String {
    match media {
        "application/json" => String::new(),
        "text/plain" => "_Text".into(),
        "application/octet-stream" => "_Binary".into(),
        "multipart/form-data" => "_Multipart".into(),
        "application/x-www-form-urlencoded" => "_Form".into(),
        "text/event-stream" => "_EventStream".into(),
        _ => format!("_{media}"),
    }
}
